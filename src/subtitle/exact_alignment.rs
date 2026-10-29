//! Conservative full-cue verification, not fuzzy matching or a playback clock.
//! Final ASR must match the source document or its provider-declared same-edition
//! reference; translated Live output is never treated as original-text evidence.
use crate::{
    ai::speech::{MAX_TRANSCRIPT_BYTES, StreamIdentity, TranscriptEvent, TranscriptKind},
    resolver::{
        contracts::{
            Confidence, MAX_WINDOW_AGE, ResolverCancellation, ResolverError, TextLanguage,
            WindowStamp,
        },
        text_session::ExistingTextLease,
    },
    subtitle::{
        existing_text::LocalizedText,
        source_arbiter::{
            ArbitrationVersion, CandidateEvidence, CueAlignment, MIN_ALIGNMENT_CONFIDENCE,
        },
        timeline::{TimelineBinding, TranscriptTimeline},
    },
};
use std::{
    ops::Range,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;

/// Reject short/common phrases; count letters/numbers, not bytes or punctuation.
const MIN_MATCH_CHARACTERS: usize = 8;
const MATCH_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ExactAlignmentError {
    #[error("{0}")]
    Invalid(#[from] ResolverError),
    #[error("定位需要語言一致、時間完整的最終辨識；中文候選另需同版本來源參考軌。")]
    UnsupportedEvidence,
    #[error("完整字幕未唯一命中，繼續使用即時翻譯。")]
    NoUniqueMatch,
    #[error("辨識與字幕的時間長度不符，繼續使用即時翻譯。")]
    TimingMismatch,
    #[error("字幕定位工作已達容量上限，未排入本次工作。")]
    Overloaded,
    #[error("字幕定位工作異常結束。")]
    WorkerFailed,
    #[error("字幕定位證據與目前字幕或辨識修訂不符。")]
    Obsolete,
}

/// Sealed evidence keeps the original publication and ASR revision. RAM only.
pub struct ExactCueMatch {
    version: ArbitrationVersion,
    input: ExistingTextLease,
    event: TranscriptEvent,
    stamp: WindowStamp,
    cue_index: usize,
    reference_index: Option<usize>,
    confidence: Confidence,
    owner: ResolverCancellation,
}
impl ExactCueMatch {
    pub fn validate(&self, identity: StreamIdentity) -> Result<(), ExactAlignmentError> {
        self.input.validate(identity)?;
        self.stamp.validate(identity, Instant::now())?;
        if self.owner.is_cancelled() {
            return Err(ResolverError::Cancelled.into());
        }
        Ok(())
    }
    pub(crate) fn cancellation(&self) -> ResolverCancellation {
        self.owner.clone()
    }
    pub fn input(&self) -> ExistingTextLease {
        self.input.clone()
    }
    /// Translate only the uniquely matched cue; no cloud work before verification.
    pub fn source_range(&self) -> Range<usize> {
        self.cue_index..self.cue_index + 1
    }
    /// Reference index is distinct from the displayed Chinese document index.
    pub fn reference_cue_index(&self) -> Option<usize> {
        self.reference_index
    }
    pub fn timing_agreement(&self) -> Confidence {
        self.confidence
    }
    pub fn covers_capture_position(&self, position_us: u64) -> bool {
        self.event
            .start_us
            .zip(self.input.text().cues[self.cue_index].duration_us)
            .is_some_and(|(start, duration)| {
                start <= position_us
                    && start
                        .checked_add(duration)
                        .is_some_and(|end| position_us < end)
            })
    }
    pub(crate) fn evidence(
        self,
        prepared: &LocalizedText,
        range: Range<usize>,
        timeline: &TranscriptTimeline,
        version: ArbitrationVersion,
        publication: &ResolverCancellation,
    ) -> Result<(TimelineBinding, CandidateEvidence), ExactAlignmentError> {
        self.validate(timeline.identity())?;
        if !self.input.availability().same_scope(publication) {
            return Err(ExactAlignmentError::Obsolete);
        }
        let binding = TimelineBinding::from_event(&self.event);
        let index = timeline
            .validate_binding(binding)
            .map_err(|_| ExactAlignmentError::Obsolete)?;
        let current = &timeline.items()[index].source;
        if current.text != self.event.text
            || version != self.version
            || current.language != self.event.language
            || current.start_us != self.event.start_us
            || current.duration_us != self.event.duration_us
            || !range.contains(&self.cue_index)
            || prepared.stamp().sequence != self.input.query().stamp.sequence
            || prepared.stamp().ended_at != self.input.query().stamp.ended_at
            || prepared.stamp().start_us != self.input.query().stamp.start_us
            || prepared.stamp().duration_us != self.input.query().stamp.duration_us
            || prepared.original().content_id != self.input.text().content_id
            || prepared.original().source != self.input.text().source
            || prepared.original().language != self.input.text().language
        {
            return Err(ExactAlignmentError::Obsolete);
        }
        let relative = self.cue_index - range.start;
        let original = &self.input.text().cues[self.cue_index];
        let cue = prepared
            .original()
            .cues
            .get(relative)
            .ok_or(ExactAlignmentError::Obsolete)?;
        if cue.text != original.text
            || cue.start_us != original.start_us
            || cue.duration_us != original.duration_us
        {
            return Err(ExactAlignmentError::Obsolete);
        }
        if let Some(reference_index) = self.reference_index {
            let reference = self
                .input
                .source_reference()
                .ok_or(ExactAlignmentError::Obsolete)?;
            let reference_cue = reference
                .text
                .cues
                .get(reference_index)
                .ok_or(ExactAlignmentError::Obsolete)?;
            // Published tracks and provenance are immutable. Recheck the exact
            // correspondence before minting evidence for the Chinese original.
            if reference.primary_edition_id != reference.reference_edition_id
                || reference_cue.start_us != cue.start_us
                || reference_cue.duration_us != cue.duration_us
                || !matches!(reference.text.language, TextLanguage::Source(language) if Some(language) == self.event.language)
            {
                return Err(ExactAlignmentError::Obsolete);
            }
        }
        Ok((
            binding,
            CandidateEvidence {
                content_confidence: self
                    .input
                    .content_confidence()
                    .ok_or(ExactAlignmentError::UnsupportedEvidence)?,
                alignment: CueAlignment {
                    version,
                    stamp: self.stamp,
                    text_sequence: prepared.stamp().sequence,
                    content_id: prepared.original().content_id.clone(),
                    source: prepared.original().source,
                    matched_original: cue.text.clone(),
                    cue_index: relative,
                    media_start_us: cue.start_us.ok_or(ExactAlignmentError::TimingMismatch)?,
                    capture_start_us: self
                        .event
                        .start_us
                        .ok_or(ExactAlignmentError::TimingMismatch)?,
                    confidence: self.confidence,
                },
                availability: ResolverCancellation::default(),
                lookup_availability: None,
                preparation_availability: None,
            },
        ))
    }
}

struct CancelOnDrop(ResolverCancellation);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}
struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);
impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// No queue: saturation returns Overloaded. CPU jobs hold a global permit even
/// after future Drop, check cancellation per cue, and retain only bounded RAM.
pub struct ExactCueVerifier {
    identity: StreamIdentity,
    version: ArbitrationVersion,
    cancel: ResolverCancellation,
    workers: Arc<Semaphore>,
}
impl ExactCueVerifier {
    #[cfg(test)]
    pub(crate) fn isolated(version: ArbitrationVersion) -> Self {
        Self {
            identity: version.identity(),
            version,
            cancel: ResolverCancellation::default(),
            workers: Arc::new(Semaphore::new(1)),
        }
    }
    pub fn new(version: ArbitrationVersion) -> Self {
        let workers = super::alignment_work::workers();
        Self {
            identity: version.identity(),
            version,
            cancel: ResolverCancellation::default(),
            workers,
        }
    }
    pub fn stop(&mut self) {
        self.cancel.cancel();
    }
    pub async fn verify(
        &mut self,
        input: ExistingTextLease,
        event: TranscriptEvent,
        stamp: WindowStamp,
        cancel: ResolverCancellation,
    ) -> Result<ExactCueMatch, ExactAlignmentError> {
        input.validate(self.identity)?;
        stamp.validate(self.identity, Instant::now())?;
        if self.cancel.is_cancelled() || cancel.is_cancelled() {
            return Err(ResolverError::Cancelled.into());
        }
        validate_evidence(&input, &event, stamp)?;
        let permit = self
            .workers
            .clone()
            .try_acquire_owned()
            .map_err(|_| ExactAlignmentError::Overloaded)?;
        let deadline = input
            .query()
            .stamp
            .ended_at
            .checked_add(MAX_WINDOW_AGE)
            .and_then(|limit| {
                Instant::now()
                    .checked_add(MATCH_TIMEOUT)
                    .map(|own| limit.min(own))
            })
            .ok_or(ResolverError::Stale)?;
        let parent = input.availability();
        let owner = self.cancel.clone();
        let job = ResolverCancellation::default();
        let _guard = CancelOnDrop(job.clone());
        let external = cancel.clone();
        let lookup = parent.clone();
        let stopped = owner.clone();
        let version = self.version;
        let mut worker = AbortOnDrop(tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let check = || {
                if job.is_cancelled()
                    || external.is_cancelled()
                    || lookup.is_cancelled()
                    || stopped.is_cancelled()
                {
                    Err(ResolverError::Cancelled.into())
                } else if Instant::now() > deadline {
                    Err(ResolverError::Stale.into())
                } else {
                    Ok(())
                }
            };
            check()?;
            let (matched_index, confidence) = match_cue(&input, &event, stamp, check)?;
            let reference_index = input.source_reference().map(|_| matched_index);
            let cue_index = if let Some(index) = reference_index {
                corresponding_cue(&input, index, &check)?
            } else {
                matched_index
            };
            check()?;
            Ok::<_, ExactAlignmentError>(ExactCueMatch {
                version,
                input,
                event,
                stamp,
                cue_index,
                reference_index,
                confidence,
                owner,
            })
        }));
        let matched = tokio::select! { biased;
            _ = parent.cancelled() => return Err(ResolverError::Cancelled.into()),
            _ = self.cancel.cancelled() => return Err(ResolverError::Cancelled.into()),
            _ = cancel.cancelled() => return Err(ResolverError::Cancelled.into()),
            _ = tokio::time::sleep_until(deadline.into()) => return Err(ResolverError::Stale.into()),
            result = &mut worker.0 => result.map_err(|_| ExactAlignmentError::WorkerFailed)??,
        };
        matched.input.validate(self.identity)?;
        matched.stamp.validate(self.identity, Instant::now())?;
        if parent.is_cancelled() || self.cancel.is_cancelled() || cancel.is_cancelled() {
            return Err(ResolverError::Cancelled.into());
        }
        Ok(matched)
    }
}
impl Drop for ExactCueVerifier {
    fn drop(&mut self) {
        self.stop();
    }
}

fn validate_evidence(
    input: &ExistingTextLease,
    event: &TranscriptEvent,
    stamp: WindowStamp,
) -> Result<(), ExactAlignmentError> {
    if event.identity != input.query().stamp.identity
        || event.kind != TranscriptKind::Final
        || !matches!(matching_document(input).language, TextLanguage::Source(language) if Some(language) == event.language)
        || event.text.is_empty()
        || event.text.len() > MAX_TRANSCRIPT_BYTES
        || event
            .text
            .chars()
            .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
        || event.text.chars().filter(|ch| ch.is_alphanumeric()).count() < MIN_MATCH_CHARACTERS
        || stamp.sequence < input.query().stamp.sequence
        || stamp.ended_at < input.query().stamp.ended_at
    {
        return Err(ExactAlignmentError::UnsupportedEvidence);
    }
    let start = event
        .start_us
        .ok_or(ExactAlignmentError::UnsupportedEvidence)?;
    let duration = event
        .duration_us
        .filter(|value| *value > 0)
        .ok_or(ExactAlignmentError::UnsupportedEvidence)?;
    let end = start
        .checked_add(duration)
        .ok_or(ExactAlignmentError::TimingMismatch)?;
    let window_start = stamp
        .start_us
        .ok_or(ExactAlignmentError::UnsupportedEvidence)?;
    let window_end = window_start
        .checked_add(stamp.duration_us)
        .ok_or(ExactAlignmentError::TimingMismatch)?;
    if start < window_start || end > window_end {
        return Err(ExactAlignmentError::TimingMismatch);
    }
    if input
        .content_confidence()
        .is_none_or(|value| value.permille() < super::source_arbiter::MIN_CONTENT_CONFIDENCE)
    {
        return Err(ExactAlignmentError::UnsupportedEvidence);
    }
    Ok(())
}

/// Preserve punctuation and word boundaries; only ASCII case and whitespace differ.
fn canonical(text: &str) -> String {
    text.split_whitespace()
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}
fn match_cue(
    input: &ExistingTextLease,
    event: &TranscriptEvent,
    stamp: WindowStamp,
    check: impl Fn() -> Result<(), ExactAlignmentError>,
) -> Result<(usize, Confidence), ExactAlignmentError> {
    let expected = canonical(&event.text);
    let mut found = None;
    // Include untimed and out-of-window repeats in ambiguity detection.
    for (index, cue) in matching_document(input).cues.iter().enumerate() {
        check()?;
        if canonical(&cue.text) == expected {
            if found.is_some() {
                return Err(ExactAlignmentError::NoUniqueMatch);
            }
            found = Some(index);
        }
    }
    let index = found.ok_or(ExactAlignmentError::NoUniqueMatch)?;
    let cue = &matching_document(input).cues[index];
    let duration = cue
        .duration_us
        .filter(|value| *value > 0)
        .ok_or(ExactAlignmentError::TimingMismatch)?;
    let media = cue.start_us.ok_or(ExactAlignmentError::TimingMismatch)?;
    let capture = event.start_us.ok_or(ExactAlignmentError::TimingMismatch)?;
    let actual = event
        .duration_us
        .ok_or(ExactAlignmentError::TimingMismatch)?;
    let window_end = stamp
        .start_us
        .and_then(|start| start.checked_add(stamp.duration_us))
        .ok_or(ExactAlignmentError::TimingMismatch)?;
    if media.checked_add(duration).is_none()
        || capture
            .checked_add(duration)
            .is_none_or(|end| end > window_end)
        || actual > duration
    {
        return Err(ExactAlignmentError::TimingMismatch);
    }
    // Measured duration agreement, not a match probability or identifier score.
    let agreement = (u128::from(actual) * 1000 / u128::from(duration)) as u16;
    if agreement < MIN_ALIGNMENT_CONFIDENCE {
        return Err(ExactAlignmentError::TimingMismatch);
    }
    Ok((index, Confidence::new(agreement)?))
}
fn matching_document(input: &ExistingTextLease) -> &crate::resolver::contracts::ExistingText {
    input
        .source_reference()
        .map_or(input.text(), |reference| &reference.text)
}
fn corresponding_cue(
    input: &ExistingTextLease,
    reference_index: usize,
    check: &impl Fn() -> Result<(), ExactAlignmentError>,
) -> Result<usize, ExactAlignmentError> {
    let reference = input
        .source_reference()
        .ok_or(ExactAlignmentError::UnsupportedEvidence)?;
    let cue = reference
        .text
        .cues
        .get(reference_index)
        .ok_or(ExactAlignmentError::TimingMismatch)?;
    let mut found = None;
    for (index, target) in input.text().cues.iter().enumerate() {
        check()?;
        if target.start_us == cue.start_us && target.duration_us == cue.duration_us {
            if found.is_some() {
                return Err(ExactAlignmentError::NoUniqueMatch);
            }
            found = Some(index);
        }
    }
    found.ok_or(ExactAlignmentError::TimingMismatch)
}

#[cfg(test)]
#[path = "paired_alignment_tests.rs"]
mod paired_tests;
#[cfg(test)]
#[path = "exact_alignment_tests.rs"]
mod tests;
