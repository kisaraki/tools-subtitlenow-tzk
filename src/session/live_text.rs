//! Optional Live existing-text branch: latest input, one worker, one result.
//! No capture callback, provider creation, detached task or extra timeline.
use crate::{
    ai::{
        speech::{StreamIdentity, TranscriptEvent, TranscriptKind},
        translation::Translator,
    },
    domain::TranslationQuality,
    resolver::{
        contracts::{ResolverCancellation, ResolverError, SourceTextReference, TextLanguage},
        text_session::ExistingTextLease,
    },
    session::{
        live_translation::LiveTranslation,
        text_preparation::{PreparedTextCandidate, TextPreparationError, TextPreparationSession},
    },
    subtitle::{
        exact_alignment::{ExactAlignmentError, ExactCueMatch, ExactCueVerifier},
        source_arbiter::{ArbitrationVersion, PlaybackObservation},
        timeline::{SourceAdoptionError, TimelineBinding},
    },
};
use std::sync::Arc;
use tokio::sync::{mpsc, watch};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextPipelineStatus {
    #[default]
    Waiting,
    Verifying,
    Localizing,
    Preparing,
    Ready,
    Unmatched,
    Failed(TextPipelineError),
    Stopped,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TextPipelineError {
    #[error("{0}")]
    Preparation(#[from] TextPreparationError),
    #[error("{0}")]
    Adoption(#[from] SourceAdoptionError),
}
struct Request {
    ticket: u64,
    version: ArbitrationVersion,
    input: ExistingTextLease,
    event: TranscriptEvent,
    cancel: ResolverCancellation,
    reset_context: bool,
}
pub struct TextCompletion {
    ticket: u64,
    result: Result<(PreparedTextCandidate, ExactCueMatch), TextPreparationError>,
}
struct PreparedKey {
    version: ArbitrationVersion,
    binding: TimelineBinding,
    source: crate::resolver::contracts::SubtitleSource,
    language: crate::resolver::contracts::TextLanguage,
    content_id: Box<str>,
    reference: Option<ReferenceKey>,
}
struct ReferenceKey {
    primary_edition_id: Box<str>,
    reference_edition_id: Box<str>,
    language: TextLanguage,
}
impl ReferenceKey {
    fn from_reference(reference: &SourceTextReference) -> Self {
        Self {
            primary_edition_id: reference.primary_edition_id.clone(),
            reference_edition_id: reference.reference_edition_id.clone(),
            language: reference.text.language,
        }
    }
    fn identity(&self) -> (&str, &str, TextLanguage) {
        (
            &self.primary_edition_id,
            &self.reference_edition_id,
            self.language,
        )
    }
}
fn reference_identity(
    reference: Option<&SourceTextReference>,
) -> Option<(&str, &str, TextLanguage)> {
    reference.map(|reference| {
        (
            reference.primary_edition_id.as_ref(),
            reference.reference_edition_id.as_ref(),
            reference.text.language,
        )
    })
}
/// One active request plus one latest replacement; mpsc result capacity is one.
pub struct LiveTextPipeline {
    identity: StreamIdentity,
    input: watch::Sender<Option<Arc<Request>>>,
    output: mpsc::Receiver<TextCompletion>,
    progress: watch::Receiver<(u64, TextPipelineStatus)>,
    cancel: ResolverCancellation,
    active: Option<Arc<Request>>,
    ticket: u64,
    status: TextPipelineStatus,
    busy: bool,
    prepared: Option<PreparedKey>,
    reset_context: bool,
    stopped: bool,
}
pub struct LiveTextTask {
    identity: StreamIdentity,
    quality: TranslationQuality,
    input: watch::Receiver<Option<Arc<Request>>>,
    output: mpsc::Sender<TextCompletion>,
    progress: watch::Sender<(u64, TextPipelineStatus)>,
    cancel: ResolverCancellation,
    #[cfg(test)]
    isolated_verifier: bool,
}
impl LiveTextPipeline {
    pub fn new(identity: StreamIdentity, quality: TranslationQuality) -> (Self, LiveTextTask) {
        let (input, receiver) = watch::channel(None);
        let (output, results) = mpsc::channel(1);
        let (progress, state) = watch::channel((0, TextPipelineStatus::Waiting));
        let cancel = ResolverCancellation::default();
        (
            Self {
                identity,
                input,
                output: results,
                progress: state,
                cancel: cancel.clone(),
                active: None,
                ticket: 0,
                status: TextPipelineStatus::Waiting,
                busy: false,
                prepared: None,
                reset_context: true,
                stopped: false,
            },
            LiveTextTask {
                identity,
                quality,
                input: receiver,
                output,
                progress,
                cancel,
                #[cfg(test)]
                isolated_verifier: false,
            },
        )
    }
    pub fn status(&self) -> TextPipelineStatus {
        let (ticket, progress) = *self.progress.borrow();
        if self.busy && ticket == self.ticket {
            progress
        } else {
            self.status
        }
    }
    pub fn binding(&self) -> Option<TimelineBinding> {
        self.active
            .as_ref()
            .map(|active| TimelineBinding::from_event(&active.event))
    }
    pub fn source(&self) -> Option<crate::resolver::contracts::SubtitleSource> {
        self.active
            .as_ref()
            .map(|active| active.input.text().source)
    }
    pub fn clear(&mut self) {
        if let Some(active) = self.active.take() {
            active.cancel.cancel();
        }
        if self.input.borrow().is_some() {
            self.input.send_replace(None);
        }
        while self.output.try_recv().is_ok() {}
        self.busy = false;
        self.reset_context = true;
        if !self.stopped {
            self.status = TextPipelineStatus::Waiting;
        }
    }
    pub fn stop(&mut self) {
        self.stopped = true;
        self.cancel.cancel();
        self.clear();
        self.prepared = None;
        self.status = TextPipelineStatus::Stopped;
    }
    pub fn fail(&mut self, error: TextPreparationError) {
        self.stop();
        self.status = TextPipelineStatus::Failed(error.into());
    }
    pub fn reject(&mut self, error: TextPreparationError) {
        // The Live scheduler reports obsolete offers here too. Preserve the
        // current request and progress; an old input is not its failure.
        if error == TextPreparationError::Lookup(ResolverError::Stale) {
            return;
        }
        self.clear();
        self.status = TextPipelineStatus::Failed(error.into());
    }
    /// Called from the orchestration loop, never a raw audio callback. Fresh
    /// confirmations do not restart a useful in-flight request for the same cue.
    pub fn offer(
        &mut self,
        input: Option<ExistingTextLease>,
        event: Option<&TranscriptEvent>,
        version: ArbitrationVersion,
    ) -> Result<bool, TextPreparationError> {
        if self.stopped {
            return Ok(false);
        }
        let Some(input) = input else {
            let changed = self.active.is_some();
            self.clear();
            return Ok(changed);
        };
        input.validate(self.identity)?;
        if version.identity() != self.identity {
            return Err(ResolverError::InvalidData.into());
        }
        let Some(event) = event.filter(|event| event.kind == TranscriptKind::Final) else {
            return Ok(false);
        };
        if event.identity != self.identity
            || event.text.len() > crate::ai::speech::MAX_TRANSCRIPT_BYTES
        {
            return Err(ResolverError::InvalidData.into());
        }
        if self.active.as_ref().is_some_and(|active| {
            active.version == version
                && (event.sequence < active.event.sequence
                    || (event.sequence == active.event.sequence
                        && event.revision < active.event.revision)
                    || input.query().stamp.sequence < active.input.query().stamp.sequence
                    || input.query().stamp.ended_at < active.input.query().stamp.ended_at)
        }) {
            // An older publication cannot cancel newer work by changing edition.
            return Err(ResolverError::Stale.into());
        }
        if self.prepared.as_ref().is_some_and(|prepared| {
            prepared.version == version
                && prepared.binding == TimelineBinding::from_event(event)
                && prepared.source == input.text().source
                && prepared.language == input.text().language
                && prepared.content_id == input.text().content_id
                && prepared.reference.as_ref().map(ReferenceKey::identity)
                    == reference_identity(input.source_reference())
        }) {
            return Ok(false);
        }
        if let Some(active) = &self.active {
            let same_binding =
                TimelineBinding::from_event(event) == TimelineBinding::from_event(&active.event);
            let same_source = input.text().content_id == active.input.text().content_id
                && input.text().source == active.input.text().source
                && input.text().language == active.input.text().language
                && reference_identity(input.source_reference())
                    == reference_identity(active.input.source_reference());
            if active.version == version
                && same_source
                && active.input.validate(self.identity).is_ok()
            {
                if same_binding
                    && (self.busy
                        || input.query().stamp.sequence == active.input.query().stamp.sequence)
                {
                    return Ok(false);
                }
                if input.query().stamp.sequence <= active.input.query().stamp.sequence {
                    // A new final supersedes old work, but cannot reuse a processed query.
                    if !same_binding && !active.cancel.is_cancelled() {
                        active.cancel.cancel();
                        self.input.send_replace(None);
                        self.busy = false;
                        self.status = TextPipelineStatus::Waiting;
                    }
                    return Ok(false);
                }
            }
        }
        let ticket = self
            .ticket
            .checked_add(1)
            .ok_or(ResolverError::InvalidData)?;
        let reset_context = self.reset_context
            || self.active.as_ref().is_none_or(|active| {
                active.version != version
                    || active.input.validate(self.identity).is_err()
                    || input.text().content_id != active.input.text().content_id
                    || input.text().source != active.input.text().source
                    || input.text().language != active.input.text().language
                    || reference_identity(input.source_reference())
                        != reference_identity(active.input.source_reference())
            });
        self.clear();
        self.ticket = ticket;
        self.reset_context = false;
        let request = Arc::new(Request {
            ticket,
            version,
            input,
            event: event.clone(),
            cancel: ResolverCancellation::default(),
            reset_context,
        });
        self.active = Some(request.clone());
        self.busy = true;
        self.status = TextPipelineStatus::Verifying;
        self.progress.borrow_and_update();
        self.input.send_replace(Some(request));
        Ok(true)
    }
    /// Progress wakes return None once; disabled/closed workers never busy-loop.
    pub async fn changed(&mut self) -> Option<TextCompletion> {
        if self.stopped {
            return std::future::pending().await;
        }
        tokio::select! { biased;
            _ = self.cancel.cancelled() => std::future::pending().await,
            result = self.output.recv() => match result {
                Some(completion) => Some(completion),
                None => std::future::pending().await,
            },
            changed = self.progress.changed() => match changed {
                Ok(()) => { self.progress.borrow_and_update(); None },
                Err(_) => std::future::pending().await,
            },
        }
    }
    pub fn apply(
        &mut self,
        completion: TextCompletion,
        flow: &mut LiveTranslation<'_>,
        observation: Option<PlaybackObservation>,
    ) -> Result<bool, TextPipelineError> {
        let Some(active) = &self.active else {
            return Ok(false);
        };
        if self.stopped || completion.ticket != self.ticket || active.cancel.is_cancelled() {
            return Ok(false);
        }
        self.busy = false;
        let result = (|| {
            active
                .input
                .validate(self.identity)
                .map_err(TextPreparationError::from)?;
            if active.version != flow.sources_mut().version() {
                return Err(TextPreparationError::Alignment(ExactAlignmentError::Obsolete).into());
            }
            let (candidate, matched) = completion.result?;
            // Retain one bounded metadata watermark even after lookup expiry;
            // a renewed query must not retransmit the same finalized ASR.
            self.prepared = Some(PreparedKey {
                version: active.version,
                binding: TimelineBinding::from_event(&active.event),
                source: active.input.text().source,
                language: active.input.text().language,
                content_id: active.input.text().content_id.clone(),
                reference: active
                    .input
                    .source_reference()
                    .map(ReferenceKey::from_reference),
            });
            let Some(observation) = observation.filter(|observation| {
                matched.covers_capture_position(observation.capture_position_us)
            }) else {
                self.status = TextPipelineStatus::Unmatched;
                return Ok(false);
            };
            let binding = candidate.submit_matched(flow, matched)?;
            let changed = flow.arbitrate(binding, observation)?;
            self.status = if changed {
                TextPipelineStatus::Ready
            } else {
                TextPipelineStatus::Unmatched
            };
            Ok(changed)
        })();
        if let Err(error) = result {
            self.status = match error {
                TextPipelineError::Preparation(TextPreparationError::Alignment(
                    ExactAlignmentError::NoUniqueMatch
                    | ExactAlignmentError::UnsupportedEvidence
                    | ExactAlignmentError::TimingMismatch,
                )) => TextPipelineStatus::Unmatched,
                _ => TextPipelineStatus::Failed(error),
            };
        }
        result
    }
}
impl Drop for LiveTextPipeline {
    fn drop(&mut self) {
        self.stop();
    }
}

impl LiveTextTask {
    #[cfg(test)]
    pub(crate) fn isolated(mut self) -> Self {
        self.isolated_verifier = true;
        self
    }
    fn verifier(&self, version: ArbitrationVersion) -> ExactCueVerifier {
        #[cfg(test)]
        if self.isolated_verifier {
            return ExactCueVerifier::isolated(version);
        }
        ExactCueVerifier::new(version)
    }
    /// The main select loop polls this persistent future together with Live AI.
    /// One preparation lane reuses the explicitly selected client's rate limits.
    pub async fn run(mut self, translator: &dyn Translator) -> Result<(), TextPreparationError> {
        let mut processor = TextPreparationSession::new(self.identity, self.quality, translator)?;
        let mut verifier = None;
        let mut version = None;
        loop {
            if self.cancel.is_cancelled() {
                return Ok(());
            }
            let request = self.input.borrow_and_update().clone();
            let Some(request) = request else {
                processor.clear_context();
                verifier.take();
                version = None;
                tokio::select! { biased;
                    _ = self.cancel.cancelled() => return Ok(()),
                    changed = self.input.changed() => if changed.is_err() { return Ok(()); },
                }
                continue;
            };
            if version != Some(request.version) {
                if version.is_some() {
                    processor.stop();
                    processor =
                        TextPreparationSession::new(self.identity, self.quality, translator)?;
                }
                verifier = Some(self.verifier(request.version));
                version = Some(request.version);
            }
            let Some(verifier) = &mut verifier else {
                return Err(ResolverError::InvalidData.into());
            };
            if request.reset_context {
                processor.clear_context();
            }
            self.progress
                .send_replace((request.ticket, TextPipelineStatus::Verifying));
            let work = async {
                let matched = verifier
                    .verify(
                        request.input.clone(),
                        request.event.clone(),
                        request.input.query().stamp,
                        request.cancel.clone(),
                    )
                    .await?;
                self.progress.send_replace((
                    request.ticket,
                    if matches!(
                        request.input.text().language,
                        crate::resolver::contracts::TextLanguage::Source(_)
                    ) {
                        TextPipelineStatus::Preparing
                    } else {
                        TextPipelineStatus::Localizing
                    },
                ));
                processor
                    .prepare_matched(matched, request.cancel.clone())
                    .await
            };
            let result = tokio::select! { biased;
                _ = self.cancel.cancelled() => return Ok(()),
                _ = request.cancel.cancelled() => continue,
                changed = self.input.changed() => { if changed.is_err() { return Ok(()); } continue; },
                result = work => result,
            };
            // At most one unread result; a new input cancels blocked publication.
            let completion = TextCompletion {
                ticket: request.ticket,
                result,
            };
            tokio::select! { biased;
                _ = self.cancel.cancelled() => return Ok(()),
                _ = request.cancel.cancelled() => {},
                changed = self.input.changed() => { if changed.is_err() { return Ok(()); } },
                sent = self.output.send(completion) => {
                    if sent.is_err() { return Ok(()); }
                    tokio::select! { biased;
                        _ = self.cancel.cancelled() => return Ok(()),
                        changed = self.input.changed() => if changed.is_err() { return Ok(()); },
                    }
                },
            }
        }
    }
}

#[cfg(test)]
#[path = "paired_live_text_tests.rs"]
mod paired_tests;
#[cfg(test)]
#[path = "live_text_tests.rs"]
mod tests;
