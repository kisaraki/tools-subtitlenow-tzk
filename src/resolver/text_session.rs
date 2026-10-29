//! One active text selection, one lookup, one latest publication. No text is
//! adopted/localized/rendered here, and no provider is selected automatically.
use super::{
    candidates::{CONFIRMED_FRESHNESS, Resolution, SONG_CONFIDENCE, SongState},
    contracts::*,
};
use crate::ai::speech::StreamIdentity;
use std::{
    sync::{Arc, Weak},
    time::{Duration, Instant},
};
use tokio::sync::watch;

pub const TEXT_LOOKUP_TIMEOUT: Duration = Duration::from_secs(3);
/// Stricter live-session budgets keep validation work small on the async owner.
/// Larger provider-neutral contracts require a separate bounded CPU lane later.
pub const MAX_SESSION_TEXT_CUES: usize = 1024;
pub const MAX_SESSION_TEXT_BYTES: usize = 128 * 1024;
/// Renewed only by a fresh explicit selection, not a lookup response or UI poll.
pub const TEXT_SELECTION_LEASE: Duration = CONFIRMED_FRESHNESS;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextLookupStatus {
    LookingUp,
    Available,
    Unavailable,
    Cleared,
    Failed(ResolverError),
    Stopped,
}
pub struct TextLookupUpdate {
    pub identity: StreamIdentity,
    pub stamp: Option<WindowStamp>,
    pub emitted_at: Instant,
    pub status: TextLookupStatus,
    pub query: Option<Arc<TextQuery>>,
    pub text: Option<Arc<ExistingText>>,
    pub source_reference: Option<Arc<SourceTextReference>>,
    provider_guard: Option<ResolverCancellation>,
    epoch: u64,
}
/// Immutable Session lookup; clones share RAM and retirement invalidates consumers.
#[derive(Clone)]
pub struct ExistingTextLease {
    query: Arc<TextQuery>,
    text: Arc<ExistingText>,
    source_reference: Option<Arc<SourceTextReference>>,
    availability: ResolverCancellation,
    provider_guard: Option<ResolverCancellation>,
    content_confidence: Option<Confidence>,
}
/// Immutable weak publication snapshot for bounded CPU consumers. It never
/// renews the query or keeps either text track alive while idle/unpolled.
#[derive(Clone)]
pub(crate) struct WeakExistingTextLease {
    query: Arc<TextQuery>,
    text: Weak<ExistingText>,
    source_reference: Option<Weak<SourceTextReference>>,
    availability: ResolverCancellation,
}
pub(crate) struct BorrowedLookupText {
    pub(crate) text: Arc<ExistingText>,
    pub(crate) source_reference: Option<Arc<SourceTextReference>>,
}
impl WeakExistingTextLease {
    pub(crate) fn validate(&self) -> Result<(), ResolverError> {
        if self.availability.is_cancelled() {
            return Err(ResolverError::Cancelled);
        }
        self.query.validate(Instant::now())?;
        if Instant::now() >= self.deadline()? {
            return Err(ResolverError::Stale);
        }
        if self.text.strong_count() == 0
            || self
                .source_reference
                .as_ref()
                .is_some_and(|reference| reference.strong_count() == 0)
        {
            return Err(ResolverError::Cancelled);
        }
        Ok(())
    }
    pub(crate) fn deadline(&self) -> Result<Instant, ResolverError> {
        self.query
            .stamp
            .ended_at
            .checked_add(MAX_WINDOW_AGE)
            .ok_or(ResolverError::Stale)
    }
    pub(crate) fn availability(&self) -> ResolverCancellation {
        self.availability.clone()
    }
    pub(crate) fn acquire(&self) -> Result<BorrowedLookupText, ResolverError> {
        self.validate()?;
        let text = self.text.upgrade().ok_or(ResolverError::Cancelled)?;
        let source_reference = self
            .source_reference
            .as_ref()
            .map(|reference| reference.upgrade().ok_or(ResolverError::Cancelled))
            .transpose()?;
        self.validate()?;
        Ok(BorrowedLookupText {
            text,
            source_reference,
        })
    }
}
impl ExistingTextLease {
    pub(crate) fn downgrade(&self) -> WeakExistingTextLease {
        WeakExistingTextLease {
            query: self.query.clone(),
            text: Arc::downgrade(&self.text),
            source_reference: self.source_reference.as_ref().map(Arc::downgrade),
            availability: self.availability.clone(),
        }
    }
    pub(crate) fn from_update(update: &TextLookupUpdate) -> Result<Self, ResolverError> {
        if update.status != TextLookupStatus::Available {
            return Err(ResolverError::InvalidData);
        }
        let availability = match &update.provider_guard {
            Some(parent) => {
                if parent.is_cancelled() {
                    return Err(ResolverError::Cancelled);
                }
                ResolverCancellation::linked(parent.clone())?
            }
            None => ResolverCancellation::default(),
        };
        let query = update.query.clone().ok_or(ResolverError::InvalidData)?;
        let text = update.text.clone().ok_or(ResolverError::InvalidData)?;
        if update.identity != query.stamp.identity
            || update.stamp.is_none_or(|stamp| {
                stamp.identity != update.identity || stamp.sequence != query.stamp.sequence
            })
            || Instant::now()
                .checked_duration_since(update.emitted_at)
                .is_none()
        {
            return Err(ResolverError::InvalidData);
        }
        query.validate(Instant::now())?;
        validate_bundle_budget(&text, update.source_reference.as_deref())?;
        text.validate(&query)?;
        if let Some(reference) = &update.source_reference {
            reference.validate(&text, &query)?;
        }
        Ok(Self {
            query,
            text,
            source_reference: update.source_reference.clone(),
            availability,
            provider_guard: update.provider_guard.clone(),
            content_confidence: None,
        })
    }
    pub fn query(&self) -> &TextQuery {
        &self.query
    }
    pub fn text(&self) -> &ExistingText {
        &self.text
    }
    /// Borrowed from the same publication: no independent lifetime or renewal.
    pub fn source_reference(&self) -> Option<&SourceTextReference> {
        self.source_reference.as_deref()
    }
    pub fn validate(&self, identity: StreamIdentity) -> Result<(), ResolverError> {
        if self.availability.is_cancelled() {
            return Err(ResolverError::Cancelled);
        }
        self.query.stamp.validate(identity, Instant::now())
    }
    pub(crate) fn availability(&self) -> ResolverCancellation {
        self.availability.clone()
    }
    pub(crate) fn retire(&self) {
        self.availability.cancel();
    }
    pub fn content_confidence(&self) -> Option<Confidence> {
        self.content_confidence
    }
    /// Only a fresh, explicit confirmed selection may renew a lookup publication.
    /// Existing clones retain their original query/time; no poll or response renews it.
    pub(crate) fn renew(
        &mut self,
        stamp: WindowStamp,
        confidence: Confidence,
    ) -> Result<(), ResolverError> {
        if self.availability.is_cancelled() {
            return Err(ResolverError::Cancelled);
        }
        stamp.validate(self.query.stamp.identity, Instant::now())?;
        if stamp.sequence < self.query.stamp.sequence || stamp.ended_at < self.query.stamp.ended_at
        {
            return Err(ResolverError::Stale);
        }
        if self.content_confidence.is_some_and(|old| confidence < old) {
            let availability = match &self.provider_guard {
                Some(parent) => ResolverCancellation::linked(parent.clone())?,
                None => ResolverCancellation::default(),
            };
            self.retire();
            self.availability = availability;
        }
        self.query = Arc::new(TextQuery {
            stamp,
            source: self.query.source,
            content_id: self.query.content_id.clone(),
        });
        self.content_confidence = Some(confidence);
        Ok(())
    }
}
pub(crate) fn validate_text_budget(text: &ExistingText) -> Result<(), ResolverError> {
    validate_bundle_budget(text, None)
}
pub(crate) fn validate_bundle_budget(
    text: &ExistingText,
    reference: Option<&SourceTextReference>,
) -> Result<(), ResolverError> {
    let count = text
        .cues
        .len()
        .checked_add(reference.map_or(0, |reference| reference.text.cues.len()));
    let bytes = text
        .cues
        .iter()
        .chain(
            reference
                .into_iter()
                .flat_map(|reference| &reference.text.cues),
        )
        .try_fold(0usize, |bytes, cue| bytes.checked_add(cue.text.len()));
    if count.is_none_or(|count| count > MAX_SESSION_TEXT_CUES)
        || bytes.is_none_or(|bytes| bytes > MAX_SESSION_TEXT_BYTES)
    {
        Err(ResolverError::InvalidData)
    } else {
        Ok(())
    }
}
struct Selection {
    query: Arc<TextQuery>,
    provider: Arc<dyn ExistingTextProvider>,
    cancel: ResolverCancellation,
    provider_guard: Option<ResolverCancellation>,
}
#[derive(Clone, Default)]
struct InputFrame {
    epoch: u64,
    stamp: Option<WindowStamp>,
    selection: Option<Arc<Selection>>,
}
struct RequestGuard(ResolverCancellation);
impl Drop for RequestGuard {
    fn drop(&mut self) {
        self.0.cancel();
    }
}
pub struct ExistingTextSession {
    identity: StreamIdentity,
    input: watch::Sender<InputFrame>,
    output: watch::Receiver<Option<Arc<TextLookupUpdate>>>,
    output_owner: watch::Sender<Option<Arc<TextLookupUpdate>>>,
    cancel: ResolverCancellation,
    previous: Option<WindowStamp>,
    observed: Option<Weak<TextLookupUpdate>>,
    epoch: u64,
}
pub struct ExistingTextTask {
    identity: StreamIdentity,
    input: watch::Receiver<InputFrame>,
    input_owner: watch::Sender<InputFrame>,
    output: watch::Sender<Option<Arc<TextLookupUpdate>>>,
    cancel: ResolverCancellation,
}
impl ExistingTextSession {
    pub fn new(identity: StreamIdentity) -> (Self, ExistingTextTask) {
        let (input, receiver) = watch::channel(InputFrame::default());
        let (output, results) = watch::channel(None);
        let cancel = ResolverCancellation::default();
        (
            Self {
                identity,
                input: input.clone(),
                output: results,
                output_owner: output.clone(),
                cancel: cancel.clone(),
                previous: None,
                observed: None,
                epoch: 0,
            },
            ExistingTextTask {
                identity,
                input: receiver,
                input_owner: input,
                output,
                cancel,
            },
        )
    }
    fn validate_stamp(&self, stamp: WindowStamp) -> Result<(), ResolverError> {
        if self.cancel.is_cancelled() {
            return Err(ResolverError::Cancelled);
        }
        stamp.validate(self.identity, Instant::now())?;
        if self
            .previous
            .is_some_and(|old| stamp.sequence <= old.sequence || stamp.ended_at < old.ended_at)
        {
            return Err(ResolverError::Stale);
        }
        Ok(())
    }
    fn cancel_selection(&self) {
        let selection = self.input.borrow().selection.clone();
        if let Some(selection) = selection {
            selection.cancel.cancel();
        }
    }
    fn reset(&mut self, status: TextLookupStatus) -> Result<(), ResolverError> {
        self.cancel_selection();
        let Some(epoch) = self.epoch.checked_add(1) else {
            // No version wrap can resurrect old text. Fail closed and release data.
            self.cancel.cancel();
            self.input.send_replace(InputFrame {
                epoch: self.epoch,
                stamp: self.previous,
                selection: None,
            });
            self.output_owner.send_replace(None);
            return Err(ResolverError::InvalidData);
        };
        self.epoch = epoch;
        self.input.send_replace(InputFrame {
            epoch,
            stamp: self.previous,
            selection: None,
        });
        self.output_owner.send_replace(Some(update(
            self.identity,
            &InputFrame {
                epoch,
                stamp: self.previous,
                selection: None,
            },
            status,
            None,
        )));
        Ok(())
    }
    /// New provider/source/content means a new lookup. Identical binding renews
    /// the selection lease without cancelling a useful request or fetching again.
    pub fn select(
        &mut self,
        query: TextQuery,
        provider: Arc<dyn ExistingTextProvider>,
    ) -> Result<bool, ResolverError> {
        self.select_inner(query, provider, false)
    }
    /// Explicit refresh only; errors/unavailable responses never retry by themselves.
    pub fn refresh(
        &mut self,
        query: TextQuery,
        provider: Arc<dyn ExistingTextProvider>,
    ) -> Result<(), ResolverError> {
        self.select_inner(query, provider, true).map(|_| ())
    }
    fn select_inner(
        &mut self,
        query: TextQuery,
        provider: Arc<dyn ExistingTextProvider>,
        force: bool,
    ) -> Result<bool, ResolverError> {
        self.validate_stamp(query.stamp)?;
        query.validate(Instant::now())?;
        // Provider callbacks may perform adapter work. Never hold the input
        // watch gate while asking for metadata lifetime or capabilities.
        let provider_guard = provider.publication_guard();
        if provider_guard
            .as_ref()
            .is_some_and(ResolverCancellation::is_cancelled)
        {
            return Err(ResolverError::Cancelled);
        }
        if !provider.supports(query.source) {
            return Err(ResolverError::UnsupportedCapability);
        }
        let unchanged = !force
            && self.input.borrow().selection.as_ref().is_some_and(|old| {
                old.query.source == query.source
                    && old.query.content_id == query.content_id
                    && Arc::ptr_eq(&old.provider, &provider)
                    && match (&old.provider_guard, &provider_guard) {
                        (None, None) => true,
                        (Some(old), Some(new)) => old.same_scope(new),
                        _ => false,
                    }
            });
        self.previous = Some(query.stamp);
        if unchanged {
            return Ok(false);
        }
        self.reset(TextLookupStatus::Cleared)?; // Hide unread old text immediately.
        let cancel = match &provider_guard {
            Some(parent) => ResolverCancellation::linked(parent.clone())?,
            None => ResolverCancellation::default(),
        };
        let selection = Arc::new(Selection {
            query: Arc::new(query),
            provider,
            cancel,
            provider_guard,
        });
        self.input.send_replace(InputFrame {
            epoch: self.epoch,
            stamp: self.previous,
            selection: Some(selection),
        });
        Ok(true)
    }
    /// Feed only settled resolver evidence (Ready, or a failure's Unknown), never
    /// its interim Resolving notification. Transition/weak/unknown evidence clears
    /// old lyrics. The supplied provider must already be bound to this song.
    pub fn select_confirmed_lyrics(
        &mut self,
        stamp: WindowStamp,
        resolution: &Resolution,
        provider: Option<Arc<dyn ExistingTextProvider>>,
    ) -> Result<bool, ResolverError> {
        self.validate_stamp(stamp)?;
        if resolution.kind != ContentType::Music || resolution.song_state != SongState::Confirmed {
            self.previous = Some(stamp);
            self.clear()?;
            return Ok(false);
        }
        let Some(song) = &resolution.song else {
            self.previous = Some(stamp);
            self.clear()?;
            return Err(ResolverError::InvalidData);
        };
        if song.confidence.permille() < SONG_CONFIDENCE || !metadata(&song.id) {
            self.previous = Some(stamp);
            self.clear()?;
            return Err(ResolverError::InvalidData);
        }
        let Some(provider) = provider else {
            self.previous = Some(stamp);
            self.clear()?;
            return Err(ResolverError::NotConfigured);
        };
        let result = self.select(
            TextQuery {
                stamp,
                source: SubtitleSource::ExistingLyrics,
                content_id: song.id.clone(),
            },
            provider,
        );
        if result.is_err() {
            self.clear()?;
        }
        result
    }
    /// Source loss, source arbitration reset or caller cancellation. No old text
    /// remains in the publication slot while waiting for the task to be polled.
    pub fn clear(&mut self) -> Result<(), ResolverError> {
        if self.cancel.is_cancelled() {
            return Err(ResolverError::Cancelled);
        }
        self.reset(TextLookupStatus::Cleared)
    }
    pub fn stop(&mut self) -> Result<(), ResolverError> {
        if self.cancel.is_cancelled() {
            return Ok(());
        }
        self.cancel.cancel();
        self.reset(TextLookupStatus::Stopped)
    }
    pub fn poll(&mut self) -> Option<Arc<TextLookupUpdate>> {
        if !self.cancel.is_cancelled() {
            let invalidation = self
                .input
                .borrow()
                .selection
                .as_ref()
                .and_then(|selection| {
                    if selection
                        .provider_guard
                        .as_ref()
                        .is_some_and(ResolverCancellation::is_cancelled)
                    {
                        Some(ResolverError::Cancelled)
                    } else if self.previous.is_some_and(|stamp| {
                        Instant::now()
                            .checked_duration_since(stamp.ended_at)
                            .is_none_or(|age| age > TEXT_SELECTION_LEASE)
                    }) {
                        Some(ResolverError::Stale)
                    } else {
                        None
                    }
                });
            if let Some(error) = invalidation {
                // reset removes the selection, so a cancelled metadata token
                // cannot repeatedly publish or wake an idle consumer.
                let _ = self.reset(TextLookupStatus::Failed(error));
            }
        }
        let result = self.output.borrow_and_update().clone()?;
        if result.epoch != self.epoch
            || (self.cancel.is_cancelled() && result.status != TextLookupStatus::Stopped)
        {
            return None;
        }
        if self
            .observed
            .as_ref()
            .and_then(Weak::upgrade)
            .is_some_and(|old| Arc::ptr_eq(&old, &result))
        {
            return None;
        }
        self.observed = Some(Arc::downgrade(&result));
        Some(result)
    }
    pub async fn next(&mut self) -> Option<Arc<TextLookupUpdate>> {
        loop {
            if let Some(result) = self.poll() {
                return Some(result);
            }
            if self.cancel.is_cancelled() {
                return None;
            }
            let deadline = self
                .input
                .borrow()
                .selection
                .as_ref()
                .and(self.previous)
                .and_then(|stamp| stamp.ended_at.checked_add(TEXT_SELECTION_LEASE));
            let provider_guard = self
                .input
                .borrow()
                .selection
                .as_ref()
                .and_then(|selection| selection.provider_guard.clone());
            tokio::select! { biased;
                changed = self.output.changed() => { if changed.is_err() { return self.poll(); } },
                _ = async {
                    match provider_guard {
                        Some(guard) => guard.cancelled().await,
                        None => std::future::pending().await,
                    }
                } => {},
                _ = async {
                    match deadline {
                        Some(deadline) => tokio::time::sleep_until(deadline.into()).await,
                        None => std::future::pending().await,
                    }
                } => {},
            }
        }
    }
}
impl Drop for ExistingTextSession {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
impl ExistingTextTask {
    fn publish(
        &self,
        frame: &InputFrame,
        status: TextLookupStatus,
        text: Option<ExistingTextBundle>,
    ) {
        // Hold the small input gate through publication. Otherwise a clear can
        // publish its empty snapshot between this check and an old result write,
        // leaving hidden stale text in the output slot even though poll rejects it.
        let publication = update(self.identity, frame, status, text);
        let input = self.input.borrow();
        if self.cancel.is_cancelled()
            || input.epoch != frame.epoch
            || frame.selection.as_ref().is_some_and(|selection| {
                selection
                    .provider_guard
                    .as_ref()
                    .is_some_and(ResolverCancellation::is_cancelled)
            })
        {
            return;
        }
        let previous = self.output.send_replace(Some(publication));
        drop(input);
        drop(previous); // Release text buffers outside the short publication gate.
    }
    pub async fn run(mut self) -> Result<(), ResolverError> {
        loop {
            if self.cancel.is_cancelled() {
                return Ok(());
            }
            tokio::select! { biased;
                _ = self.cancel.cancelled() => return Ok(()),
                changed = self.input.changed() => { if changed.is_err() { return Ok(()); } },
            }
            let frame = self.input.borrow_and_update().clone();
            let Some(selection) = &frame.selection else {
                continue;
            };
            let _request = RequestGuard(selection.cancel.clone());
            if selection.cancel.is_cancelled() {
                continue;
            }
            let checked = selection
                .query
                .stamp
                .validate(self.identity, Instant::now())
                .and_then(|_| selection.query.validate(Instant::now()));
            if let Err(error) = checked {
                self.publish(&frame, TextLookupStatus::Failed(error), None);
                continue;
            }
            if !selection.provider.supports(selection.query.source) {
                self.publish(
                    &frame,
                    TextLookupStatus::Failed(ResolverError::UnsupportedCapability),
                    None,
                );
                continue;
            }
            self.publish(&frame, TextLookupStatus::LookingUp, None);
            let result = tokio::select! { biased;
                _ = self.cancel.cancelled() => return Ok(()),
                _ = selection.cancel.cancelled() => continue,
                changed = self.input.changed() => {
                    if changed.is_err() { return Ok(()); }
                    // Keep the newest notification unread for the next iteration.
                    self.input.mark_changed(); continue;
                },
                result = tokio::time::timeout(TEXT_LOOKUP_TIMEOUT, selection.provider.lookup_bundle(&selection.query, selection.cancel.clone())) => result.unwrap_or(Err(ResolverError::Timeout)),
            };
            let result = result.and_then(|text| {
                selection
                    .query
                    .stamp
                    .validate(self.identity, Instant::now())?;
                selection.query.validate(Instant::now())?;
                if let Some(bundle) = &text {
                    validate_bundle_budget(&bundle.text, bundle.source_reference.as_ref())?;
                    bundle.validate(&selection.query)?;
                }
                selection.query.validate(Instant::now())?;
                if selection.cancel.is_cancelled() {
                    return Err(ResolverError::Cancelled);
                }
                Ok(text)
            });
            match result {
                Ok(Some(text)) => self.publish(&frame, TextLookupStatus::Available, Some(text)),
                Ok(None) => self.publish(&frame, TextLookupStatus::Unavailable, None),
                Err(error) => self.publish(&frame, TextLookupStatus::Failed(error), None),
            }
        }
    }
}
impl Drop for ExistingTextTask {
    fn drop(&mut self) {
        self.cancel.cancel();
        let mut frame = self.input_owner.borrow().clone();
        if let Some(selection) = frame.selection.take() {
            selection.cancel.cancel();
        }
        self.input_owner.send_replace(frame.clone());
        self.output.send_replace(Some(update(
            self.identity,
            &frame,
            TextLookupStatus::Stopped,
            None,
        )));
    }
}
fn update(
    identity: StreamIdentity,
    frame: &InputFrame,
    status: TextLookupStatus,
    bundle: Option<ExistingTextBundle>,
) -> Arc<TextLookupUpdate> {
    let (text, source_reference) = match bundle {
        Some(bundle) => (
            Some(Arc::new(bundle.text)),
            bundle.source_reference.map(Arc::new),
        ),
        None => (None, None),
    };
    Arc::new(TextLookupUpdate {
        identity,
        stamp: frame.stamp,
        emitted_at: Instant::now(),
        status,
        query: frame.selection.as_ref().map(|v| Arc::clone(&v.query)),
        text,
        source_reference,
        provider_guard: frame
            .selection
            .as_ref()
            .and_then(|selection| selection.provider_guard.clone()),
        epoch: frame.epoch,
    })
}

#[cfg(test)]
#[path = "text_session_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "text_session_guard_tests.rs"]
mod guard_tests;
