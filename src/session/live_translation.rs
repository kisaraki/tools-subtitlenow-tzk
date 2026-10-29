//! Caller-owned STT-to-translation flow, independent of capture and renderers.
use crate::{
    ai::{
        speech::{StreamIdentity, TranscriptEvent},
        translation::{
            MAX_TRANSLATION_AGE, TranslationCancellation, TranslationCoordinator, TranslationError,
            TranslationJobKey, Translator,
        },
    },
    domain::TranslationQuality,
    subtitle::{
        source_arbiter::{PlaybackObservation, SubtitleSourceArbiter},
        timeline::{MAX_TIMELINE_ITEMS, SourceAdoptionError, TimelineBinding, TranscriptTimeline},
    },
};
use futures_util::{StreamExt, stream::FuturesUnordered};
use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    time::{Duration, Instant},
};
type JobFuture<'a> =
    Pin<Box<dyn Future<Output = (TranslationJobKey, Result<(), TranslationError>)> + Send + 'a>>;
/// Isolated unfinished requests may be skipped; persistent failures stop the Session.
const MAX_CONSECUTIVE_SKIPPED_TRANSLATIONS: u8 = 3;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LiveTranslationNotice {
    pub(crate) sequence: u64,
    pub(crate) skipped: u64,
    pub(crate) consecutive: u8,
    pub(crate) cause: TranslationError,
}
impl LiveTranslationNotice {
    pub(crate) fn exhausted(self) -> bool {
        self.consecutive >= MAX_CONSECUTIVE_SKIPPED_TRANSLATIONS
    }
}
struct DeferredPartial {
    event: TranscriptEvent,
    expires: Instant,
}
pub struct LiveTranslation<'a> {
    core: TranslationCoordinator,
    jobs: FuturesUnordered<JobFuture<'a>>,
    provider: Option<&'a dyn Translator>,
    sources: SubtitleSourceArbiter,
    /// One latest update per active utterance, bounded by the session timeline.
    deferred: Vec<DeferredPartial>,
    notice: Option<LiveTranslationNotice>,
}
impl<'a> LiveTranslation<'a> {
    pub fn new(
        identity: StreamIdentity,
        quality: TranslationQuality,
        provider: &'a dyn Translator,
    ) -> Result<Self, TranslationError> {
        Self::diagnosed(identity, quality, provider, None)
    }
    pub fn diagnosed(
        identity: StreamIdentity,
        quality: TranslationQuality,
        provider: &'a dyn Translator,
        diagnostics: Option<Arc<crate::performance::live::LiveDiagnostics>>,
    ) -> Result<Self, TranslationError> {
        Ok(Self {
            core: TranslationCoordinator::diagnosed(
                identity,
                quality,
                provider.capabilities(),
                diagnostics,
            )?,
            jobs: FuturesUnordered::new(),
            provider: Some(provider),
            sources: SubtitleSourceArbiter::new(identity, quality),
            deferred: Vec::new(),
            notice: None,
        })
    }
    pub fn ingest(&mut self, event: TranscriptEvent) -> Result<bool, TranslationError> {
        if self.provider.is_none() {
            return Err(TranslationError::UnsupportedCapability);
        }
        if event.identity != self.core.timeline().identity() {
            return self.core.ingest(event);
        }
        if self.deferred.iter().any(|pending| {
            pending.event.utterance == event.utterance && event.revision <= pending.event.revision
        }) {
            return Ok(false);
        }
        if self.core.can_coalesce_partial(&event) {
            let pending = DeferredPartial {
                event,
                expires: Instant::now() + MAX_TRANSLATION_AGE,
            };
            if let Some(slot) = self
                .deferred
                .iter_mut()
                .find(|slot| slot.event.utterance == pending.event.utterance)
            {
                *slot = pending;
            } else {
                if self.deferred.len() >= MAX_TIMELINE_ITEMS {
                    return Err(TranslationError::Overloaded);
                }
                self.deferred.push(pending);
            }
            // Accepted by the coalescer, but the shared timeline has not changed.
            return Ok(false);
        }
        let utterance = event.utterance;
        let changed = self.core.ingest(event)?;
        if changed {
            let retired = self.core.timeline().retired_through();
            self.deferred.retain(|pending| {
                pending.event.utterance != utterance
                    && retired.is_none_or(|sequence| pending.event.sequence > sequence)
            });
        }
        Ok(changed)
    }
    pub fn native(
        identity: StreamIdentity,
        quality: TranslationQuality,
        diagnostics: Option<Arc<crate::performance::live::LiveDiagnostics>>,
    ) -> Result<Self, TranslationError> {
        Ok(Self {
            core: TranslationCoordinator::diagnosed(
                identity,
                quality,
                crate::ai::translation::TranslationCapabilities {
                    streaming: true,
                    cancellation: true,
                    supported_languages: [true; 5],
                    max_concurrent: 1,
                },
                diagnostics,
            )?,
            jobs: FuturesUnordered::new(),
            provider: None,
            sources: SubtitleSourceArbiter::new(identity, quality),
            deferred: Vec::new(),
            notice: None,
        })
    }
    pub fn ingest_native(
        &mut self,
        caption: crate::ai::native_caption::NativeCaption,
    ) -> Result<bool, TranslationError> {
        if self.provider.is_some() {
            return Err(TranslationError::UnsupportedCapability);
        }
        self.core.ingest_native(caption)
    }
    fn flush_deferred(&mut self) -> Result<(), TranslationError> {
        let mut index = 0;
        while index < self.deferred.len() {
            if self
                .core
                .active_partial(self.deferred[index].event.utterance)
            {
                index += 1;
                continue;
            }
            let pending = self.deferred.remove(index);
            if pending.expires <= Instant::now() {
                if self.core.skip_live_deferred(&pending.event) {
                    self.record_skip(pending.event.sequence, TranslationError::Stale)?;
                }
                continue;
            }
            self.core.ingest_deadline(pending.event, pending.expires)?;
        }
        Ok(())
    }
    fn record_skip(
        &mut self,
        sequence: u64,
        cause: TranslationError,
    ) -> Result<(), TranslationError> {
        let previous = self.notice;
        let consecutive = previous.map_or(1, |notice| notice.consecutive.saturating_add(1));
        self.notice = Some(LiveTranslationNotice {
            sequence,
            cause,
            skipped: previous.map_or(1, |notice| notice.skipped.saturating_add(1)),
            consecutive,
        });
        tracing::warn!(sequence, skipped=self.notice.map_or(0, |notice| notice.skipped), consecutive, error=%cause,
            "單次 Live 翻譯未完成；未提交定稿，後續新段落依連續失敗上限處理");
        if self.notice.is_some_and(LiveTranslationNotice::exhausted) {
            self.core.cancellation().fail(cause);
            return Err(cause);
        }
        Ok(())
    }
    pub fn notice(&self) -> Option<LiveTranslationNotice> {
        self.notice
    }
    pub fn timeline(&self) -> &TranscriptTimeline {
        self.core.timeline()
    }
    /// Explicit candidate/evidence entry. Default GUI does not supply candidates.
    pub fn sources_mut(&mut self) -> &mut SubtitleSourceArbiter {
        &mut self.sources
    }
    pub fn adopt_prepared(
        &mut self,
        source: &crate::subtitle::fuzzy_worker::AlignmentSource,
        alignment: Arc<crate::subtitle::alignment_verification::VerifiedAlignment>,
    ) -> Result<bool, SourceAdoptionError> {
        let binding = alignment.binding();
        self.core.validate_source_binding(binding)?;
        let text = source
            .display(&alignment)
            .map_err(|_| crate::subtitle::source_arbiter::ArbitrationError::InvalidAlignment)?;
        let now = Instant::now();
        let decision = self.sources.decide_prepared(&text, alignment, now)?;
        self.core.adopt_source(binding, decision, now)
    }
    pub fn arbitrate(
        &mut self,
        binding: TimelineBinding,
        observation: PlaybackObservation,
    ) -> Result<bool, SourceAdoptionError> {
        self.core.validate_source_binding(binding)?;
        let now = Instant::now();
        let decision = self.sources.decide(observation, now)?;
        self.core.adopt_source(binding, decision, now)
    }
    /// Exposed for caller-owned polling when no async wait is running.
    pub fn reconcile_sources(&mut self) -> Option<u64> {
        self.core.reconcile_sources(Instant::now())
    }
    /// Commit only verified text after its current translation has been applied.
    pub fn commit_prefix(&mut self, utterance: u64, bytes: usize) -> Result<(), TranslationError> {
        self.core.commit_prefix(utterance, bytes)
    }
    pub fn glossary_mut(&mut self) -> &mut crate::ai::translation::SessionGlossary {
        self.core.glossary_mut()
    }
    pub fn cancellation(&self) -> TranslationCancellation {
        self.core.cancellation()
    }
    pub fn active_requests(&self) -> usize {
        self.core.active_requests()
    }
    pub fn pending_requests(&self) -> usize {
        self.core.pending_requests() + self.deferred.len()
    }
    pub fn update_queue_depth(&self) -> usize {
        self.core.update_queue_depth()
    }
    /// Cancelling this wait preserves owned jobs. Dropping the flow cancels them.
    pub async fn changed(&mut self) -> Result<Option<u64>, TranslationError> {
        let Some(provider) = self.provider else {
            return self.core.changed().await;
        };
        if let Some(reason) = self.core.cancellation().reason() {
            return Err(reason);
        }
        let previous_notice = self.notice;
        self.flush_deferred()?;
        if self.notice != previous_notice {
            return Ok(None);
        }
        let mut schedule = tokio::time::interval(Duration::from_millis(10));
        schedule.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            if let Some(reason) = self.core.cancellation().reason() {
                return Err(reason);
            }
            if let Some(utterance) = self.reconcile_sources() {
                return Ok(Some(utterance));
            }
            let now = Instant::now();
            let expired = self.core.expire_live_pending(now);
            for sequence in &expired {
                self.record_skip(*sequence, TranslationError::Stale)?;
            }
            if !expired.is_empty() {
                return Ok(None);
            }
            while let Some(job) = self.core.take_ready(now) {
                self.jobs.push(Box::pin(job.run_live(provider)));
            }
            if let Some(reason) = self.core.cancellation().reason() {
                return Err(reason);
            }
            tokio::select! {biased;
                update=self.core.changed()=>{
                    let update = update?;
                    if update.is_some_and(|utterance| self.core.completed_live_revision(utterance))
                        && let Some(notice) = &mut self.notice {
                        notice.consecutive = 0;
                    }
                    return Ok(update);
                },
                result=self.jobs.next(),if !self.jobs.is_empty()=>match result {
                    Some((_, Err(TranslationError::Cancelled)))=>{},
                    Some((key, Err(error))) if error.skippable_live_request()=>{
                        let sequence = if error == TranslationError::Incomplete {
                            self.core.reject_incomplete_live_job(key)
                        } else {
                            self.core.expire_live_job(key)
                        };
                        if let Some(sequence) = sequence {
                            self.record_skip(sequence, error)?;
                            return Ok(None);
                        }
                    },
                    Some((_, Err(error)))=>return Err(error),
                    Some((_, Ok(())))=>{},
                    None=>{},
                },
                _=schedule.tick()=>{},
            }
        }
    }
}
impl Drop for LiveTranslation<'_> {
    fn drop(&mut self) {
        self.core.cancellation().cancel();
        self.sources.stop();
        self.jobs.clear();
        self.deferred.clear();
    }
}

#[cfg(test)]
#[path = "live_source_tests.rs"]
mod source_tests;
