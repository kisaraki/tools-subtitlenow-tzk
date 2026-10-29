//! Independent multi-observation gates. Raw fuzzy hints never mint evidence.
//! Position is the end of observed Final speech, never extrapolated capture.
use super::{
    fuzzy_alignment::{AlignmentScope, FuzzyAlignmentError, FuzzyCueProposal},
    fuzzy_track::{FuzzyTrackProposal, MIN_TRACK_OBSERVATIONS},
    timeline::TimelineBinding,
};
use crate::{resolver::contracts::ResolverError, session::playback_clock::CaptureCursor};
use std::{ops::Range, time::Instant};

pub const MIN_VERIFIED_AGREEMENT: u16 = 950;
pub const MIN_VERIFIED_MARGIN: u16 = 100;
pub const MAX_VERIFIED_RESIDUAL_US: u64 = 50_000;
pub const MAX_RATE_DEVIATION_PPM: u64 = 50_000;
const PPM: u128 = 1_000_000;
/// Recognition may lag actual capture, but stale speech is not current position.
pub const MAX_POSITION_LAG_US: u64 = 2_000_000;
/// Small corrections are measured anew, never integrated using wall-clock time.
pub const MAX_DRIFT_STEP_US: i128 = 100_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AlignmentVerificationError {
    #[error("{0}")]
    Invalid(#[from] FuzzyAlignmentError),
    #[error("對齊的獨立觀測、文字差距或時間一致性不足，繼續即時翻譯。")]
    InsufficientEvidence,
    #[error("擷取觀測與辨識時間不符或已過期，等待重新定位。")]
    InvalidCapture,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalibrationChange {
    Acquired,
    Refreshed,
    DriftCorrected { delta_us: i128 },
    SeekForward,
    SeekBackward,
}

/// This value reports a past observed speech endpoint. Continued capture or
/// silence cannot advance it. Only a newly verified independent Final can.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObservedMediaPosition {
    pub capture_us: u64,
    pub media_us: u64,
}
/// Bounded diagnostic fit. Projection is private and is used only to compare
/// new, already matched speech endpoints; never to advance a displayed clock.
#[derive(Clone, Copy)]
pub(crate) struct AlignmentModel {
    scope: AlignmentScope,
    capture_base: u64,
    media_base: u64,
    numerator: u64,
    denominator: u64,
    last_capture: u64,
    observed_at: Instant,
}
impl AlignmentModel {
    fn fit(
        track: &FuzzyTrackProposal,
        observed_at: Instant,
    ) -> Result<Self, AlignmentVerificationError> {
        Self::fit_intervals(track.scope(), track.intervals(), observed_at)
    }
    pub(crate) fn fit_intervals<'a, I>(
        scope: AlignmentScope,
        intervals: I,
        observed_at: Instant,
    ) -> Result<Self, AlignmentVerificationError>
    where
        I: Clone + Iterator<Item = (&'a Range<u64>, &'a Range<u64>)>,
    {
        let (first_capture, first_media) = intervals
            .clone()
            .next()
            .ok_or(AlignmentVerificationError::InsufficientEvidence)?;
        let (last_capture, last_media) = intervals
            .clone()
            .last()
            .ok_or(AlignmentVerificationError::InsufficientEvidence)?;
        let capture = first_capture.start..last_capture.end;
        let media = first_media.start..last_media.end;
        let denominator = capture
            .end
            .checked_sub(capture.start)
            .filter(|value| *value > 0)
            .ok_or(AlignmentVerificationError::InsufficientEvidence)?;
        let numerator = media
            .end
            .checked_sub(media.start)
            .filter(|value| *value > 0)
            .ok_or(AlignmentVerificationError::InsufficientEvidence)?;
        if u128::from(numerator.abs_diff(denominator)) * PPM
            > u128::from(denominator) * u128::from(MAX_RATE_DEVIATION_PPM)
        {
            return Err(AlignmentVerificationError::InsufficientEvidence);
        }
        let model = Self {
            scope,
            capture_base: capture.start,
            media_base: media.start,
            numerator,
            denominator,
            last_capture: capture.end,
            observed_at,
        };
        for (capture, media) in intervals {
            for (at, expected) in [(capture.start, media.start), (capture.end, media.end)] {
                if model
                    .project(at)
                    .is_none_or(|value| value.abs_diff(expected) > MAX_VERIFIED_RESIDUAL_US)
                {
                    return Err(AlignmentVerificationError::InsufficientEvidence);
                }
            }
        }
        Ok(model)
    }
    fn project(self, capture: u64) -> Option<u64> {
        let delta = capture.checked_sub(self.capture_base)?;
        let offset = u128::from(delta)
            .checked_mul(u128::from(self.numerator))?
            .checked_div(u128::from(self.denominator))?;
        u64::try_from(u128::from(self.media_base).checked_add(offset)?).ok()
    }
    pub(crate) fn fits_next(self, next: &FuzzyCueProposal) -> bool {
        let capture = next.capture_range_us();
        let media = next.media_range_us();
        let max_gap =
            u64::try_from(crate::resolver::contracts::MAX_WINDOW_AGE.as_micros()).unwrap_or(0);
        next.scope() == self.scope
            && next
                .window_stamp()
                .ended_at
                .checked_duration_since(self.observed_at)
                .is_some_and(|age| age <= crate::resolver::contracts::MAX_WINDOW_AGE)
            && capture.end >= self.last_capture
            && capture.end - self.last_capture <= max_gap
            && self
                .project(capture.start)
                .is_some_and(|value| value.abs_diff(media.start) <= MAX_VERIFIED_RESIDUAL_US)
            && self
                .project(capture.end)
                .is_some_and(|value| value.abs_diff(media.end) <= MAX_VERIFIED_RESIDUAL_US)
    }
}
pub struct VerifiedAlignment {
    expires: Instant,
    model: AlignmentModel,
    track: FuzzyTrackProposal,
    binding: TimelineBinding,
    cues: Range<usize>,
    primary_cues: Option<Range<usize>>,
    capture: Range<u64>,
    media: Range<u64>,
    observed_at: Instant,
    capture_sequence: u64,
    change: CalibrationChange,
    capture_cursor: CaptureCursor,
    withdrawn: crate::resolver::contracts::ResolverCancellation,
}
impl VerifiedAlignment {
    pub(crate) fn model(&self) -> AlignmentModel {
        self.model
    }
    pub fn rate_ppm(&self) -> Result<u32, AlignmentVerificationError> {
        self.validate(self.scope(), self.binding)?;
        u32::try_from(u128::from(self.model.numerator) * PPM / u128::from(self.model.denominator))
            .map_err(|_| AlignmentVerificationError::InsufficientEvidence)
    }
    pub(crate) fn revoke(&self) {
        self.withdrawn.cancel();
    }
    pub fn scope(&self) -> AlignmentScope {
        self.track.scope()
    }
    pub fn binding(&self) -> TimelineBinding {
        self.binding
    }
    pub fn validate(
        &self,
        scope: AlignmentScope,
        binding: TimelineBinding,
    ) -> Result<(), AlignmentVerificationError> {
        self.track.validate(scope)?;
        if Instant::now() >= self.expires {
            return Err(FuzzyAlignmentError::Invalid(ResolverError::Stale).into());
        }
        if !self.capture_cursor.continuity_valid() || self.withdrawn.is_cancelled() {
            return Err(AlignmentVerificationError::InvalidCapture);
        }
        if binding != self.binding {
            return Err(FuzzyAlignmentError::Obsolete.into());
        }
        Ok(())
    }
    pub fn position(&self) -> Result<ObservedMediaPosition, AlignmentVerificationError> {
        self.validate(self.scope(), self.binding)?;
        Ok(ObservedMediaPosition {
            capture_us: self.capture.end,
            media_us: self.media.end,
        })
    }
    pub fn offset_us(&self) -> Result<i128, AlignmentVerificationError> {
        self.position()
            .map(|position| i128::from(position.media_us) - i128::from(position.capture_us))
    }
    pub fn change(&self) -> CalibrationChange {
        self.change
    }
    pub(crate) fn cues(&self) -> Range<usize> {
        self.primary_cues
            .clone()
            .unwrap_or_else(|| self.cues.clone())
    }
    pub(crate) fn capture_range(&self) -> Range<u64> {
        self.capture.clone()
    }
    pub(crate) fn reference_cues(&self) -> Range<usize> {
        self.cues.clone()
    }
    pub(crate) fn expires_at(&self) -> Instant {
        self.expires
    }
    pub(crate) fn media_range(&self) -> Range<u64> {
        self.media.clone()
    }
    pub(crate) fn observed_at(&self) -> Instant {
        self.observed_at
    }
    pub(crate) fn capture_sequence(&self) -> u64 {
        self.capture_sequence
    }
}
#[derive(Default)]
pub struct AlignmentVerifier {
    last_model: Option<AlignmentModel>,
    expected: Option<AlignmentScope>,
    last: Option<(
        AlignmentScope,
        TimelineBinding,
        ObservedMediaPosition,
        Instant,
    )>,
}
impl AlignmentVerifier {
    pub fn reset(&mut self, scope: AlignmentScope) {
        self.last_model = None;
        self.expected = Some(scope);
        self.last = None;
    }
    pub fn verify(
        &mut self,
        track: FuzzyTrackProposal,
        latest: &FuzzyCueProposal,
        capture: CaptureCursor,
    ) -> Result<VerifiedAlignment, AlignmentVerificationError> {
        let scope = track.scope();
        if self.expected.is_some_and(|expected| expected != scope) {
            return Err(FuzzyAlignmentError::Obsolete.into());
        }
        track.validate(scope)?;
        latest.validate(scope, track.last_binding())?;
        if track.last_cues() != latest.cue_range()
            || track.last_primary_cues() != latest.paired_primary_cue_range()
            || track.origin() != latest.origin()
        {
            return Err(FuzzyAlignmentError::Obsolete.into());
        }
        let (agreement, margin) = track.verification_quality();
        if track.observations() < MIN_TRACK_OBSERVATIONS
            || agreement < MIN_VERIFIED_AGREEMENT
            || margin < MIN_VERIFIED_MARGIN
        {
            return Err(AlignmentVerificationError::InsufficientEvidence);
        }
        let model = AlignmentModel::fit(&track, capture.observed_at())?;
        let span = latest.capture_range_us();
        if !capture.is_fresh()
            || !capture.covers(track.observed_capture_span_us())
            || capture.identity() != scope.capture_identity()
            || capture.observed_at() < latest.window_stamp().ended_at
            || capture.end_us() < span.end
            || capture.end_us() - span.end > MAX_POSITION_LAG_US
        {
            return Err(AlignmentVerificationError::InvalidCapture);
        }
        let media = latest.media_range_us();
        let position = ObservedMediaPosition {
            capture_us: span.end,
            media_us: media.end,
        };
        let offset = i128::from(position.media_us) - i128::from(position.capture_us);
        let change = match self.last {
            Some((previous_scope, binding, previous, at))
                if previous_scope == scope
                    && capture
                        .observed_at()
                        .checked_duration_since(at)
                        .is_some_and(|age| age <= crate::resolver::contracts::MAX_WINDOW_AGE) =>
            {
                if latest.binding().sequence <= binding.sequence
                    || latest.binding().utterance <= binding.utterance
                {
                    return Err(FuzzyAlignmentError::Obsolete.into());
                }
                let delta =
                    offset - (i128::from(previous.media_us) - i128::from(previous.capture_us));
                if delta == 0 {
                    CalibrationChange::Refreshed
                } else if delta.abs() <= MAX_DRIFT_STEP_US
                    || self.last_model.is_some_and(|model| model.fits_next(latest))
                {
                    CalibrationChange::DriftCorrected { delta_us: delta }
                } else if delta > 0 {
                    CalibrationChange::SeekForward
                } else {
                    CalibrationChange::SeekBackward
                }
            }
            _ => CalibrationChange::Acquired,
        };
        if span.start >= span.end || media.start >= media.end {
            return Err(FuzzyAlignmentError::Invalid(ResolverError::InvalidData).into());
        }
        self.last = Some((scope, latest.binding(), position, capture.observed_at()));
        self.last_model = Some(model);
        self.expected = Some(scope);
        Ok(VerifiedAlignment {
            expires: track.expires_at().min(latest.observation_deadline()?),
            model,
            binding: latest.binding(),
            cues: latest.cue_range(),
            primary_cues: latest.paired_primary_cue_range(),
            capture: span,
            media,
            observed_at: capture.observed_at(),
            capture_sequence: capture.sequence(),
            change,
            capture_cursor: capture,
            withdrawn: crate::resolver::contracts::ResolverCancellation::default(),
            track,
        })
    }
}

#[cfg(test)]
#[path = "alignment_verification_tests.rs"]
mod tests;
