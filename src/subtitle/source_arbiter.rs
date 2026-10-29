//! Session-only policy boundary. Alignment evidence is supplied, never fabricated.
//! Decisions borrow prepared text; no renderer, clock extrapolation or extra timeline.
use super::{alignment_verification::VerifiedAlignment, fuzzy_worker::AlignedCueText};
use super::{existing_text::LocalizedText, source_translation::TranslatedTextWindow};
use crate::{
    ai::speech::StreamIdentity,
    domain::TranslationQuality,
    resolver::contracts::{
        Confidence, MAX_CANDIDATES, MAX_WINDOW_AGE, ResolverCancellation, ResolverError,
        SubtitleSource, TextLanguage, WindowStamp,
    },
};
use std::sync::Arc;
use std::time::Instant;

/// Conservative engineering gates on supplied scores, not calibrated probabilities.
pub const MIN_CONTENT_CONFIDENCE: u16 = 850;
pub const MIN_ALIGNMENT_CONFIDENCE: u16 = 900;
/// Two existing-text sources, each with the four specification language priorities.
const CANDIDATE_KEYS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArbitrationVersion {
    identity: StreamIdentity,
    epoch: u64,
}
impl ArbitrationVersion {
    pub fn identity(self) -> StreamIdentity {
        self.identity
    }
}
/// A fresh match to one cue, expressed in the actual capture clock. The future
/// alignment component obtains this from playback/ASR evidence, not wall time.
/// cue_index is relative to LocalizedText.original(), including translated windows.
pub struct CueAlignment {
    pub version: ArbitrationVersion,
    pub stamp: WindowStamp,
    pub text_sequence: u64,
    pub content_id: Box<str>,
    pub source: SubtitleSource,
    /// The canonical candidate cue matched by the alignment component.
    pub matched_original: Box<str>,
    pub cue_index: usize,
    pub media_start_us: u64,
    pub capture_start_us: u64,
    pub confidence: Confidence,
}
pub struct CandidateEvidence {
    pub content_confidence: Confidence,
    pub alignment: CueAlignment,
    /// Candidate-scoped token. Withdrawal, retirement and owner Drop cancel it.
    pub availability: ResolverCancellation,
    /// Candidate retirement never cancels this optional lookup publication parent.
    pub lookup_availability: Option<ResolverCancellation>,
    pub preparation_availability: Option<ResolverCancellation>,
}
/// Actual observed capture position. No position is inferred from elapsed wall time.
pub struct PlaybackObservation {
    pub version: ArbitrationVersion,
    pub sequence: u64,
    pub observed_at: Instant,
    pub capture_position_us: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ArbitrationError {
    #[error("{0}")]
    Invalid(#[from] ResolverError),
    #[error("字幕候選的內容或定位信心不足，繼續使用即時翻譯。")]
    WeakEvidence,
    #[error("來源字幕尚未達到所選翻譯品質，繼續使用即時翻譯。")]
    InsufficientQuality,
    #[error("字幕候選定位無效，繼續使用即時翻譯。")]
    InvalidAlignment,
    #[error("字幕來源或播放觀測已失效，請重新取得定位證據。")]
    Obsolete,
    #[error("字幕候選已達容量上限，未加入本次候選。")]
    Overloaded,
    #[error("字幕來源仲裁已停止。")]
    Stopped,
}
/// Private text intentionally has no Debug, Clone or serialization.
pub struct SelectedCue<'a> {
    pub content_id: &'a str,
    /// Index within the prepared candidate, used by CueAlignment.
    pub cue_index: usize,
    /// Index within the caller's original document, including translated ranges.
    pub document_cue_index: usize,
    pub media_start_us: u64,
    pub original: &'a str,
    pub translated: &'a str,
    pub capture_start_us: u64,
    pub capture_end_us: u64,
    pub quality: Option<TranslationQuality>,
    pub(crate) lease: SourceLease,
}
/// Only the arbiter can mint a prepared-text lease. It remains Session RAM only.
#[derive(Clone)]
pub(crate) struct SourceLease {
    pub version: ArbitrationVersion,
    pub text_sequence: u64,
    pub alignment_sequence: u64,
    expires: Instant,
    availability: ResolverCancellation,
    lookup_availability: Option<ResolverCancellation>,
    preparation_availability: Option<ResolverCancellation>,
    alignment: Option<Arc<VerifiedAlignment>>,
}
impl SourceLease {
    pub fn valid(&self, identity: StreamIdentity, now: Instant) -> bool {
        self.version.identity == identity
            && now <= self.expires
            && !self.availability.is_cancelled()
            && self.alignment.as_ref().is_none_or(|alignment| {
                alignment
                    .validate(alignment.scope(), alignment.binding())
                    .is_ok()
            })
            && self
                .lookup_availability
                .as_ref()
                .is_none_or(|cancel| !cancel.is_cancelled())
            && self
                .preparation_availability
                .as_ref()
                .is_none_or(|cancel| !cancel.is_cancelled())
    }
    pub fn same_candidate(&self, other: &Self) -> bool {
        self.version == other.version
            && self.text_sequence == other.text_sequence
            && self.alignment_sequence == other.alignment_sequence
            && self.expires == other.expires
    }
}
pub(crate) struct SourceDecision<'a> {
    pub identity: StreamIdentity,
    pub selection: SourceSelection<'a>,
}
pub enum SourceSelection<'a> {
    LiveAi,
    Existing {
        source: SubtitleSource,
        cue: SelectedCue<'a>,
    },
}
impl SourceSelection<'_> {
    pub fn source(&self) -> SubtitleSource {
        match self {
            Self::LiveAi => SubtitleSource::LiveAi,
            Self::Existing { source, .. } => *source,
        }
    }
}
struct Candidate {
    key: usize,
    text: LocalizedText,
    quality: Option<TranslationQuality>,
    evidence: CandidateEvidence,
    capture_end_us: u64,
    document_cue_index: usize,
}
impl Candidate {
    fn rank(&self) -> (usize, u8) {
        (self.key % 4, self.quality.map_or(3, quality_rank))
    }
    fn covers(&self, position: u64) -> bool {
        self.evidence.alignment.capture_start_us <= position && position < self.capture_end_us
    }
    fn available(&self, identity: StreamIdentity, now: Instant) -> bool {
        !self.evidence.availability.is_cancelled()
            && self
                .evidence
                .preparation_availability
                .as_ref()
                .is_none_or(|cancel| !cancel.is_cancelled())
            && self
                .evidence
                .lookup_availability
                .as_ref()
                .is_none_or(|cancel| !cancel.is_cancelled())
            && self.text.stamp().validate(identity, now).is_ok()
            && self
                .evidence
                .alignment
                .stamp
                .validate(identity, now)
                .is_ok()
    }
}
impl Drop for Candidate {
    fn drop(&mut self) {
        self.evidence.availability.cancel();
    }
}

/// One arbiter per Session, with at most four prepared candidates and no tasks.
pub struct SubtitleSourceArbiter {
    aligned_availability: ResolverCancellation,
    version: ArbitrationVersion,
    quality: TranslationQuality,
    candidates: Vec<Candidate>,
    latest: [Option<WindowStamp>; CANDIDATE_KEYS],
    floor: Option<u64>,
    observation: Option<(u64, Instant, u64)>,
    active: Option<usize>,
    stopped: bool,
}
impl SubtitleSourceArbiter {
    pub fn new(identity: StreamIdentity, quality: TranslationQuality) -> Self {
        Self {
            aligned_availability: ResolverCancellation::default(),
            version: ArbitrationVersion { identity, epoch: 0 },
            quality,
            candidates: Vec::with_capacity(MAX_CANDIDATES),
            latest: [None; CANDIDATE_KEYS],
            floor: None,
            observation: None,
            active: None,
            stopped: false,
        }
    }
    pub fn version(&self) -> ArbitrationVersion {
        self.version
    }
    /// Independent validated-Prepared path. Never accepts a raw fuzzy hint or
    /// converts its text-agreement measure into a content confidence score.
    pub(crate) fn decide_prepared<'a>(
        &mut self,
        text: &'a AlignedCueText,
        alignment: Arc<VerifiedAlignment>,
        now: Instant,
    ) -> Result<SourceDecision<'a>, ArbitrationError> {
        self.check_version(self.version)?;
        alignment
            .validate(alignment.scope(), alignment.binding())
            .map_err(|_| ArbitrationError::Obsolete)?;
        if alignment.scope().capture_identity() != self.version.identity
            || text.media != alignment.media_range()
            || text.cue_index != alignment.cues().start
            || text.original.is_empty()
            || text.translated.is_empty()
            || text.text_sequence == 0
            || now < alignment.observed_at()
            || now > alignment.expires_at()
        {
            return Err(ArbitrationError::InvalidAlignment);
        }
        if text
            .quality
            .is_some_and(|quality| quality_rank(quality) < quality_rank(self.quality))
        {
            return Err(ArbitrationError::InsufficientQuality);
        }
        let capture = alignment.capture_range();
        Ok(SourceDecision {
            identity: self.version.identity,
            selection: SourceSelection::Existing {
                source: SubtitleSource::PreparedTranslation,
                cue: SelectedCue {
                    content_id: &text.content_id,
                    cue_index: text.cue_index,
                    document_cue_index: text.cue_index,
                    media_start_us: text.media.start,
                    original: &text.original,
                    translated: &text.translated,
                    capture_start_us: capture.start,
                    capture_end_us: capture.end,
                    quality: text.quality,
                    lease: SourceLease {
                        version: self.version,
                        text_sequence: text.text_sequence,
                        alignment_sequence: alignment.capture_sequence(),
                        expires: alignment.expires_at(),
                        availability: self.aligned_availability.clone(),
                        lookup_availability: None,
                        preparation_availability: None,
                        alignment: Some(alignment),
                    },
                },
            },
        })
    }
    pub fn candidate_count(&self) -> usize {
        self.candidates.len()
    }
    pub(crate) fn decide(
        &mut self,
        observation: PlaybackObservation,
        now: Instant,
    ) -> Result<SourceDecision<'_>, ArbitrationError> {
        let identity = self.version.identity;
        Ok(SourceDecision {
            identity,
            selection: self.select(observation, now)?,
        })
    }
    pub fn submit_localized(
        &mut self,
        text: LocalizedText,
        evidence: CandidateEvidence,
        now: Instant,
    ) -> Result<(), ArbitrationError> {
        if matches!(text.original().language, TextLanguage::Source(_)) {
            return Err(ArbitrationError::InsufficientQuality);
        }
        self.submit(text, None, 0, evidence, now)
    }
    pub fn submit_translated(
        &mut self,
        window: TranslatedTextWindow,
        evidence: CandidateEvidence,
        now: Instant,
    ) -> Result<(), ArbitrationError> {
        let quality = window.quality();
        let document_start = window.range().start;
        self.submit(
            window.into_localized(),
            Some(quality),
            document_start,
            evidence,
            now,
        )
    }
    fn submit(
        &mut self,
        text: LocalizedText,
        quality: Option<TranslationQuality>,
        document_start: usize,
        evidence: CandidateEvidence,
        now: Instant,
    ) -> Result<(), ArbitrationError> {
        self.check_version(evidence.alignment.version)?;
        self.prune(now);
        let stamp = text.stamp();
        stamp.validate(self.version.identity, now)?;
        let alignment = &evidence.alignment;
        alignment.stamp.validate(self.version.identity, now)?;
        if evidence.availability.is_cancelled()
            || evidence
                .preparation_availability
                .as_ref()
                .is_some_and(ResolverCancellation::is_cancelled)
            || evidence
                .lookup_availability
                .as_ref()
                .is_some_and(ResolverCancellation::is_cancelled)
        {
            return Err(ResolverError::Cancelled.into());
        }
        if evidence.content_confidence.permille() < MIN_CONTENT_CONFIDENCE
            || alignment.confidence.permille() < MIN_ALIGNMENT_CONFIDENCE
        {
            return Err(ArbitrationError::WeakEvidence);
        }
        if quality.is_some_and(|quality| quality_rank(quality) < quality_rank(self.quality)) {
            return Err(ArbitrationError::InsufficientQuality);
        }
        let original = text.original();
        let source_key = match original.source {
            SubtitleSource::ExistingSubtitle => 0,
            SubtitleSource::ExistingLyrics => 1,
            _ => return Err(ResolverError::InvalidData.into()),
        };
        let language_key = match original.language {
            TextLanguage::TaiwanTraditional => 3,
            TextLanguage::OtherTraditional => 2,
            TextLanguage::SimplifiedChinese => 1,
            TextLanguage::Source(_) => 0,
        };
        let key = source_key * 4 + language_key;
        if self.floor.is_some_and(|floor| stamp.sequence <= floor)
            || self.latest[key]
                .is_some_and(|old| stamp.sequence <= old.sequence || stamp.ended_at < old.ended_at)
        {
            return Err(ArbitrationError::Obsolete);
        }
        let cue = original
            .cues
            .get(alignment.cue_index)
            .ok_or(ArbitrationError::InvalidAlignment)?;
        let capture_end_us = cue
            .duration_us
            .and_then(|duration| alignment.capture_start_us.checked_add(duration))
            .ok_or(ArbitrationError::InvalidAlignment)?;
        let capture_window_end = alignment
            .stamp
            .start_us
            .and_then(|start| start.checked_add(alignment.stamp.duration_us))
            .ok_or(ArbitrationError::InvalidAlignment)?;
        if alignment.text_sequence != stamp.sequence
            || alignment.content_id != original.content_id
            || alignment.source != original.source
            || alignment.matched_original != cue.text
            || alignment.stamp.sequence < stamp.sequence
            || alignment.stamp.ended_at < stamp.ended_at
            || cue.start_us != Some(alignment.media_start_us)
            || alignment.capture_start_us < alignment.stamp.start_us.unwrap_or(u64::MAX)
            || capture_end_us > capture_window_end
            || capture_end_us <= alignment.capture_start_us
        {
            return Err(ArbitrationError::InvalidAlignment);
        }
        let replacement = self
            .candidates
            .iter()
            .position(|candidate| candidate.key == key);
        if replacement.is_none() && self.candidates.len() == MAX_CANDIDATES {
            return Err(ArbitrationError::Overloaded);
        }
        let document_cue_index = document_start
            .checked_add(alignment.cue_index)
            .ok_or(ArbitrationError::InvalidAlignment)?;
        let candidate = Candidate {
            key,
            text,
            quality,
            evidence,
            capture_end_us,
            document_cue_index,
        };
        if let Some(index) = replacement {
            self.candidates[index] = candidate;
        } else {
            self.candidates.push(candidate);
        }
        self.latest[key] = Some(stamp);
        Ok(())
    }
    pub fn select(
        &mut self,
        observation: PlaybackObservation,
        now: Instant,
    ) -> Result<SourceSelection<'_>, ArbitrationError> {
        self.check_version(observation.version)?;
        if now
            .checked_duration_since(observation.observed_at)
            .is_none_or(|age| age > MAX_WINDOW_AGE)
            || self.observation.is_some_and(|(sequence, time, position)| {
                observation.sequence <= sequence
                    || observation.observed_at < time
                    || observation.capture_position_us < position
            })
        {
            return Err(ArbitrationError::Obsolete);
        }
        self.observation = Some((
            observation.sequence,
            observation.observed_at,
            observation.capture_position_us,
        ));
        self.prune(now);
        let best = self
            .candidates
            .iter()
            .filter(|candidate| {
                candidate.evidence.alignment.stamp.ended_at <= observation.observed_at
                    && candidate.covers(observation.capture_position_us)
            })
            .max_by_key(|candidate| candidate.rank());
        let Some(best) = best else {
            self.active = None;
            return Ok(SourceSelection::LiveAi);
        };
        // Hold the active source on equal priority; confidence gates already passed.
        let selected = self
            .candidates
            .iter()
            .find(|candidate| {
                Some(candidate.key) == self.active
                    && candidate.rank() == best.rank()
                    && candidate.evidence.alignment.stamp.ended_at <= observation.observed_at
                    && candidate.covers(observation.capture_position_us)
            })
            .unwrap_or(best);
        self.active = Some(selected.key);
        let alignment = &selected.evidence.alignment;
        let original = selected.text.original();
        Ok(SourceSelection::Existing {
            source: original.source,
            cue: SelectedCue {
                content_id: &original.content_id,
                cue_index: alignment.cue_index,
                document_cue_index: selected.document_cue_index,
                media_start_us: alignment.media_start_us,
                original: &original.cues[alignment.cue_index].text,
                translated: &selected.text.translated()[alignment.cue_index],
                capture_start_us: alignment.capture_start_us,
                capture_end_us: selected.capture_end_us,
                quality: selected.quality,
                lease: SourceLease {
                    alignment: None,
                    version: self.version,
                    text_sequence: selected.text.stamp().sequence,
                    alignment_sequence: alignment.stamp.sequence,
                    expires: (selected.text.stamp().ended_at + MAX_WINDOW_AGE)
                        .min(alignment.stamp.ended_at + MAX_WINDOW_AGE),
                    availability: selected.evidence.availability.clone(),
                    lookup_availability: selected.evidence.lookup_availability.clone(),
                    preparation_availability: selected.evidence.preparation_availability.clone(),
                },
            },
        })
    }
    /// Required for source/content transition, audio gaps, seek or provider restart.
    /// New evidence must carry the new version and a newer text query sequence.
    pub fn invalidate(&mut self) -> Result<ArbitrationVersion, ArbitrationError> {
        self.check_version(self.version)?;
        self.aligned_availability.cancel();
        self.aligned_availability = ResolverCancellation::default();
        self.floor = self
            .latest
            .iter()
            .flatten()
            .map(|stamp| stamp.sequence)
            .max()
            .or(self.floor);
        self.candidates.clear();
        self.active = None;
        self.observation = None;
        let Some(epoch) = self.version.epoch.checked_add(1) else {
            self.stopped = true;
            return Err(ArbitrationError::Stopped);
        };
        self.version.epoch = epoch;
        Ok(self.version)
    }
    pub fn stop(&mut self) {
        self.aligned_availability.cancel();
        self.stopped = true;
        self.candidates.clear();
        self.active = None;
        self.observation = None;
    }
    fn check_version(&self, version: ArbitrationVersion) -> Result<(), ArbitrationError> {
        if self.stopped {
            Err(ArbitrationError::Stopped)
        } else if version != self.version {
            Err(ArbitrationError::Obsolete)
        } else {
            Ok(())
        }
    }
    fn prune(&mut self, now: Instant) {
        self.candidates
            .retain(|candidate| candidate.available(self.version.identity, now));
        if !self
            .candidates
            .iter()
            .any(|candidate| Some(candidate.key) == self.active)
        {
            self.active = None;
        }
    }
}
impl Drop for SubtitleSourceArbiter {
    fn drop(&mut self) {
        self.stop();
    }
}
fn quality_rank(quality: TranslationQuality) -> u8 {
    match quality {
        TranslationQuality::Fast => 0,
        TranslationQuality::Standard => 1,
        TranslationQuality::High => 2,
    }
}

#[cfg(test)]
#[path = "source_arbiter_tests.rs"]
mod tests;
