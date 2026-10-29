//! Bounded temporal consistency of accepted Final hints, never playback evidence.
use super::{
    fuzzy_alignment::{AlignmentScope, FuzzyAlignmentError, FuzzyCueProposal, FuzzyTextOrigin},
    timeline::TimelineBinding,
};
use crate::resolver::contracts::{ResolverCancellation, ResolverError};
use crate::resolver::text_session::WeakExistingTextLease;
use std::{ops::Range, time::Instant};
pub const PREPARED_HISTORY_RETENTION: std::time::Duration = std::time::Duration::from_secs(30);

pub const MAX_TRACK_OBSERVATIONS: usize = 8;
pub const MIN_TRACK_OBSERVATIONS: usize = 3;
/// Require meaningful observed coverage rather than three token-sized samples.
pub const MIN_TRACK_CAPTURE_SPAN_US: u64 = 1_000_000;
/// Engineering endpoint tolerance; neither confidence nor an offset estimate.
pub const MAX_TRACK_ENDPOINT_SPREAD_US: u64 = 250_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FuzzyTrackStatus {
    Collecting(usize),
    Consistent(usize),
    Restarted(usize),
    Rejected(FuzzyAlignmentError),
    Cancelled,
    Stopped,
}
enum Notice {
    None,
    Restarted,
    Rejected(FuzzyAlignmentError),
}
struct Observation {
    binding: TimelineBinding,
    origin: FuzzyTextOrigin,
    cues: Range<usize>,
    primary_cues: Option<Range<usize>>,
    capture: Range<u64>,
    media: Range<u64>,
    expires: Instant,
    agreement: u16,
    margin: u16,
}
impl Observation {
    fn offsets(&self) -> (i128, i128) {
        let start = i128::from(self.media.start) - i128::from(self.capture.start);
        let end = i128::from(self.media.end) - i128::from(self.capture.end);
        (start.min(end), start.max(end))
    }
}

/// Sealed, revocable consistency hint. No text, document, Debug/Clone/Serialize,
/// offset/current-position API or conversion to source-arbiter evidence.
pub struct FuzzyTrackProposal {
    intervals: Vec<(Range<u64>, Range<u64>)>,
    scope: AlignmentScope,
    origin: FuzzyTextOrigin,
    observations: usize,
    capture: Range<u64>,
    media: Range<u64>,
    endpoint_spread_us: u64,
    expires: Instant,
    availability: ResolverCancellation,
    revision: ResolverCancellation,
    publication: Option<WeakExistingTextLease>,
    minimum_agreement: u16,
    minimum_margin: u16,
    last_binding: TimelineBinding,
    last_cues: Range<usize>,
    last_primary_cues: Option<Range<usize>>,
}
impl FuzzyTrackProposal {
    pub(crate) fn intervals(&self) -> impl Iterator<Item = (&Range<u64>, &Range<u64>)> + Clone {
        self.intervals
            .iter()
            .map(|(capture, media)| (capture, media))
    }
    pub(crate) fn expires_at(&self) -> Instant {
        self.expires
    }
    pub(crate) fn verification_quality(&self) -> (u16, u16) {
        (self.minimum_agreement, self.minimum_margin)
    }
    pub(crate) fn last_binding(&self) -> TimelineBinding {
        self.last_binding
    }
    pub(crate) fn last_cues(&self) -> Range<usize> {
        self.last_cues.clone()
    }
    pub(crate) fn last_primary_cues(&self) -> Option<Range<usize>> {
        self.last_primary_cues.clone()
    }
    pub fn scope(&self) -> AlignmentScope {
        self.scope
    }
    pub fn origin(&self) -> FuzzyTextOrigin {
        self.origin
    }
    pub fn observations(&self) -> usize {
        self.observations
    }
    pub fn observed_capture_span_us(&self) -> Range<u64> {
        self.capture.clone()
    }
    pub fn observed_media_span_us(&self) -> Range<u64> {
        self.media.clone()
    }
    pub fn endpoint_spread_us(&self) -> u64 {
        self.endpoint_spread_us
    }
    pub fn validate(&self, scope: AlignmentScope) -> Result<(), FuzzyAlignmentError> {
        self.validate_at(scope, Instant::now())
    }
    fn validate_at(&self, scope: AlignmentScope, now: Instant) -> Result<(), FuzzyAlignmentError> {
        if let Some(publication) = &self.publication {
            publication.validate()?;
        }
        if self.availability.is_cancelled() {
            return Err(ResolverError::Cancelled.into());
        }
        if self.revision.is_cancelled() || scope != self.scope {
            return Err(FuzzyAlignmentError::Obsolete);
        }
        if now >= self.expires {
            return Err(ResolverError::Stale.into());
        }
        Ok(())
    }
}

/// Only the latest-Final owner admits observations while their lexical proposal
/// is valid. Committed metadata follows document lifetime, not the superseded
/// per-job token; revisions/old messages must withdraw their utterance first.
pub(crate) struct FuzzyTrack {
    prepared_history: bool,
    scope: AlignmentScope,
    availability: ResolverCancellation,
    revision: ResolverCancellation,
    points: Vec<Observation>,
    notice: Notice,
    stopped: bool,
    publication: Option<WeakExistingTextLease>,
}
impl FuzzyTrack {
    pub(crate) fn enable_prepared_history(&mut self) {
        self.prepared_history = true;
    }
    pub(crate) fn retains_prepared_history(&self) -> bool {
        self.prepared_history
    }
    pub(crate) fn with_publication(mut self, publication: Option<WeakExistingTextLease>) -> Self {
        self.publication = publication;
        self
    }
    #[cfg(test)]
    pub(crate) fn shorten_oldest_for_test(&mut self, deadline: Instant) {
        if let Some(first) = self.points.first_mut() {
            first.expires = first.expires.min(deadline);
        }
    }
    pub(crate) fn new(scope: AlignmentScope, availability: ResolverCancellation) -> Self {
        Self {
            prepared_history: false,
            scope,
            availability,
            revision: ResolverCancellation::default(),
            points: Vec::with_capacity(MAX_TRACK_OBSERVATIONS),
            notice: Notice::None,
            stopped: false,
            publication: None,
        }
    }
    fn revoke(&mut self) {
        self.revision.cancel();
        self.revision = ResolverCancellation::default();
    }
    pub(crate) fn clear(&mut self) {
        self.revoke();
        self.points.clear();
        self.notice = Notice::None;
    }
    pub(crate) fn reject(&mut self, error: FuzzyAlignmentError) {
        self.clear();
        self.notice = Notice::Rejected(error);
    }
    pub(crate) fn stop(&mut self) {
        self.clear();
        self.stopped = true;
    }
    pub(crate) fn withdraw_utterance(&mut self, utterance: u64) {
        let before = self.points.len();
        self.points
            .retain(|point| point.binding.utterance != utterance);
        if self.points.len() != before {
            self.revoke();
            self.notice = Notice::None;
        }
    }
    pub(crate) fn refresh(&mut self, now: Instant) {
        let before = self.points.len();
        self.points.retain(|point| point.expires > now);
        if self.points.len() != before {
            self.revoke();
            self.notice = Notice::None;
        }
    }
    pub(crate) fn next_expiry(&self) -> Option<Instant> {
        self.points.iter().map(|point| point.expires).min()
    }
    pub(crate) fn admit(
        &mut self,
        proposal: &FuzzyCueProposal,
        now: Instant,
    ) -> Result<bool, FuzzyAlignmentError> {
        if self.stopped || self.availability.is_cancelled() {
            return Err(ResolverError::Cancelled.into());
        }
        proposal.validate(self.scope, proposal.binding())?;
        self.refresh(now);
        let point = Observation {
            binding: proposal.binding(),
            origin: proposal.origin(),
            cues: proposal.cue_range(),
            primary_cues: proposal.paired_primary_cue_range(),
            capture: proposal.capture_range_us(),
            media: proposal.media_range_us(),
            expires: if self.prepared_history {
                proposal
                    .window_stamp()
                    .ended_at
                    .checked_add(PREPARED_HISTORY_RETENTION)
                    .ok_or(ResolverError::Stale)?
            } else {
                proposal.observation_deadline()?
            },
            agreement: proposal.text_agreement_permille(),
            // No runner reached 850. Use that bound rather than inventing a
            // zero-distance runner or a calibrated confidence.
            margin: proposal.text_agreement_permille().saturating_sub(
                proposal
                    .runner_up_agreement_permille()
                    .unwrap_or(super::fuzzy_alignment::MIN_COMPETITOR_AGREEMENT_PERMILLE - 1),
            ),
        };
        if point.expires <= now {
            return Err(ResolverError::Stale.into());
        }
        if let Some(old) = self.points.iter().find(|old| old.binding == point.binding) {
            if old.origin == point.origin
                && old.cues == point.cues
                && old.primary_cues == point.primary_cues
                && old.capture == point.capture
                && old.media == point.media
            {
                return Ok(false);
            }
            self.clear();
            self.notice = Notice::Rejected(ResolverError::InvalidData.into());
            return Err(ResolverError::InvalidData.into());
        }
        self.withdraw_utterance(point.binding.utterance);
        let own = point.offsets();
        if !self.prepared_history && own.1 - own.0 > i128::from(MAX_TRACK_ENDPOINT_SPREAD_US) {
            self.clear();
            self.notice = Notice::Rejected(FuzzyAlignmentError::TimingMismatch);
            return Err(FuzzyAlignmentError::TimingMismatch);
        }
        let ordered = self.points.last().is_none_or(|old| {
            point.binding.sequence > old.binding.sequence
                && point.binding.utterance > old.binding.utterance
                && point.cues.start >= old.cues.end
                && match (&old.primary_cues, &point.primary_cues) {
                    (None, None) => true,
                    (Some(old), Some(new)) => new.start >= old.end,
                    _ => false,
                }
                && point.origin == old.origin
                && point.capture.start >= old.capture.end
                && point.media.start >= old.media.end
                && point.expires >= old.expires
        });
        let (low, high) = self.points.iter().fold(own, |(low, high), old| {
            let bounds = old.offsets();
            (low.min(bounds.0), high.max(bounds.1))
        });
        let consistent = if self.prepared_history {
            super::alignment_verification::AlignmentModel::fit_intervals(
                self.scope,
                self.points
                    .iter()
                    .map(|point| (&point.capture, &point.media))
                    .chain(std::iter::once((&point.capture, &point.media))),
                now,
            )
            .is_ok()
        } else {
            high - low <= i128::from(MAX_TRACK_ENDPOINT_SPREAD_US)
        };
        let restarted = !ordered || !consistent;
        if restarted {
            self.clear();
        }
        self.revoke();
        self.notice = if restarted {
            Notice::Restarted
        } else {
            Notice::None
        };
        if self.points.len() == MAX_TRACK_OBSERVATIONS {
            self.points.remove(0);
        }
        self.points.push(point);
        Ok(true)
    }
    pub(crate) fn status(&self, now: Instant) -> FuzzyTrackStatus {
        if self.stopped {
            return FuzzyTrackStatus::Stopped;
        }
        if let Some(publication) = &self.publication
            && let Err(error) = publication.validate()
        {
            return if error == ResolverError::Cancelled {
                FuzzyTrackStatus::Cancelled
            } else {
                FuzzyTrackStatus::Rejected(error.into())
            };
        }
        if self.availability.is_cancelled() {
            return FuzzyTrackStatus::Cancelled;
        }
        let mut fresh = self.points.iter().filter(|point| point.expires > now);
        let first = fresh.next();
        let mut count = usize::from(first.is_some());
        let mut end = first.map_or(0, |point| point.capture.end);
        for point in fresh {
            count += 1;
            end = point.capture.end;
        }
        if count == self.points.len() {
            match self.notice {
                Notice::Rejected(error) => return FuzzyTrackStatus::Rejected(error),
                Notice::Restarted => return FuzzyTrackStatus::Restarted(count),
                Notice::None => {}
            }
        }
        if count >= MIN_TRACK_OBSERVATIONS
            && first.is_some_and(|point| {
                end.saturating_sub(point.capture.start) >= MIN_TRACK_CAPTURE_SPAN_US
            })
        {
            FuzzyTrackStatus::Consistent(count)
        } else {
            FuzzyTrackStatus::Collecting(count)
        }
    }
    pub(crate) fn proposal(&self, now: Instant) -> Option<FuzzyTrackProposal> {
        let FuzzyTrackStatus::Consistent(observations) = self.status(now) else {
            return None;
        };
        let mut fresh = self.points.iter().filter(|point| point.expires > now);
        let first = fresh.next()?;
        let mut last = first;
        let mut expires = first.expires;
        let (mut low, mut high) = first.offsets();
        let mut minimum_agreement = first.agreement;
        let mut minimum_margin = first.margin;
        for point in fresh {
            let bounds = point.offsets();
            low = low.min(bounds.0);
            high = high.max(bounds.1);
            expires = expires.min(point.expires);
            last = point;
            minimum_agreement = minimum_agreement.min(point.agreement);
            minimum_margin = minimum_margin.min(point.margin);
        }
        Some(FuzzyTrackProposal {
            intervals: self
                .points
                .iter()
                .filter(|point| point.expires > now)
                .map(|point| (point.capture.clone(), point.media.clone()))
                .collect(),
            scope: self.scope,
            origin: first.origin,
            observations,
            capture: first.capture.start..last.capture.end,
            media: first.media.start..last.media.end,
            endpoint_spread_us: u64::try_from(high - low).ok()?,
            expires,
            availability: self.availability.clone(),
            revision: self.revision.clone(),
            publication: self.publication.clone(),
            minimum_agreement,
            minimum_margin,
            last_binding: last.binding,
            last_cues: last.cues.clone(),
            last_primary_cues: last.primary_cues.clone(),
        })
    }
}
impl Drop for FuzzyTrack {
    fn drop(&mut self) {
        self.revision.cancel();
    }
}

#[cfg(test)]
#[path = "fuzzy_track_tests.rs"]
pub(crate) mod tests;
