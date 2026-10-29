//! Explicit Prepared-to-Live alignment. No player operations, guessed capture
//! stamps, wall-clock advancement, extra STT/translation or renderer access.
use super::{
    alignment_control::{AlignmentControlError, AlignmentController},
    fuzzy_proposals::ProposalState,
    playback_clock::CaptureCursor,
};
use crate::{
    ai::speech::{StreamIdentity, TranscriptEvent, TranscriptKind},
    resolver::contracts::WindowStamp,
    subtitle::{
        alignment_verification::{
            AlignmentVerificationError, AlignmentVerifier, CalibrationChange,
            ObservedMediaPosition, VerifiedAlignment,
        },
        fuzzy_alignment::FuzzyAlignmentError,
        fuzzy_track::FuzzyTrackStatus,
        fuzzy_worker::AlignmentSource,
    },
};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RealignmentReason {
    Drift,
    AudioGap,
    SeekForward,
    SeekBackward,
    Discontinuity,
    ExplicitReset,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlignedPlaybackState {
    AdoptionFailed(crate::subtitle::timeline::SourceAdoptionError),
    Paused,
    Collecting(usize),
    Reacquiring(RealignmentReason),
    Positioned {
        position: ObservedMediaPosition,
        offset_us: i128,
        change: CalibrationChange,
    },
    Unmatched,
    Failed(AlignmentVerificationError),
    Stopped,
}
pub struct AlignedPlayback {
    input_error: Option<AlignmentVerificationError>,
    previous_model: Option<crate::subtitle::alignment_verification::AlignmentModel>,
    drift_restart: Option<crate::subtitle::timeline::TimelineBinding>,
    delivered: Option<crate::subtitle::timeline::TimelineBinding>,
    rejected: Option<crate::subtitle::timeline::TimelineBinding>,
    control: AlignmentController,
    verifier: AlignmentVerifier,
    verified: Option<Arc<VerifiedAlignment>>,
    state: AlignedPlaybackState,
    latest: Option<(TranscriptEvent, WindowStamp)>,
    previous_position: Option<ObservedMediaPosition>,
}
impl AlignedPlayback {
    pub(crate) fn missing_capture(&mut self) {
        self.reject_input(AlignmentVerificationError::InvalidCapture);
    }
    pub(crate) fn reject_input(&mut self, error: AlignmentVerificationError) {
        self.withdraw();
        self.input_error = Some(error);
        self.state = AlignedPlaybackState::Failed(error);
    }
    fn withdraw(&mut self) {
        self.delivered = None;
        if let Some(previous) = self.verified.take() {
            previous.revoke();
        }
    }
    pub fn running(&self) -> bool {
        !matches!(
            self.state,
            AlignedPlaybackState::Paused | AlignedPlaybackState::Stopped
        )
    }
    pub fn new(
        source: &AlignmentSource,
        capture: StreamIdentity,
    ) -> Result<Self, AlignmentControlError> {
        Ok(Self {
            input_error: None,
            previous_model: None,
            drift_restart: None,
            delivered: None,
            rejected: None,
            control: AlignmentController::from_source(source, capture)?,
            verifier: AlignmentVerifier::default(),
            verified: None,
            state: AlignedPlaybackState::Paused,
            latest: None,
            previous_position: None,
        })
    }
    pub async fn resume(&mut self) -> Result<(), AlignmentControlError> {
        let old = self.control.scope();
        let scope = self.control.resume().await?;
        if old != Some(scope) {
            self.verifier.reset(scope);
            self.withdraw();
            self.latest = None;
            self.state = AlignedPlaybackState::Collecting(0);
        }
        Ok(())
    }
    pub async fn pause(&mut self) -> Result<(), AlignmentControlError> {
        self.input_error = None;
        self.withdraw();
        self.latest = None;
        self.state = AlignedPlaybackState::Paused;
        self.control.pause().await
    }
    pub async fn reset(
        &mut self,
        capture: StreamIdentity,
        reason: RealignmentReason,
    ) -> Result<(), AlignmentControlError> {
        if capture.session.0 == 0 || capture.generation == 0 {
            return Err(FuzzyAlignmentError::Invalid(
                crate::resolver::contracts::ResolverError::InvalidData,
            )
            .into());
        }
        self.withdraw();
        self.previous_model = None;
        self.input_error = None;
        self.drift_restart = None;
        self.latest = None;
        self.state = AlignedPlaybackState::Reacquiring(reason);
        let scope = self.control.reset(capture).await?;
        self.verifier.reset(scope);
        Ok(())
    }
    pub async fn stop(&mut self) -> Result<(), AlignmentControlError> {
        self.input_error = None;
        self.withdraw();
        self.latest = None;
        self.previous_position = None;
        self.state = AlignedPlaybackState::Stopped;
        self.control.stop().await
    }
    pub fn state(&self) -> AlignedPlaybackState {
        if matches!(self.state, AlignedPlaybackState::Positioned { .. })
            && self.verified().is_none()
        {
            AlignedPlaybackState::Collecting(0)
        } else {
            self.state
        }
    }
    pub fn verified(&self) -> Option<Arc<VerifiedAlignment>> {
        self.verified
            .as_ref()
            .filter(|alignment| {
                alignment
                    .validate(alignment.scope(), alignment.binding())
                    .is_ok()
            })
            .cloned()
    }
    pub fn take_verified(&mut self) -> Option<Arc<VerifiedAlignment>> {
        let verified = self.verified()?;
        if self.delivered == Some(verified.binding()) {
            return None;
        }
        self.delivered = Some(verified.binding());
        Some(verified)
    }
    pub fn reject_adoption(&mut self, error: crate::subtitle::timeline::SourceAdoptionError) {
        self.rejected = self.verified.as_ref().map(|alignment| alignment.binding());
        self.withdraw();
        self.state = AlignedPlaybackState::AdoptionFailed(error);
    }
    pub fn offer(
        &mut self,
        event: TranscriptEvent,
        stamp: WindowStamp,
    ) -> Result<bool, AlignmentControlError> {
        if event.kind != TranscriptKind::Final {
            return Ok(false);
        }
        let changed = self.control.offer(event.clone(), stamp)?;
        if changed {
            self.input_error = None;
            self.rejected = None;
            self.latest = Some((event, stamp));
        }
        Ok(changed)
    }
    pub async fn changed(&mut self) -> Result<ProposalState, AlignmentControlError> {
        self.control.changed().await
    }
    /// Consume metadata on the orchestration task, never the audio callback.
    pub async fn reconcile(
        &mut self,
        capture: Option<CaptureCursor>,
    ) -> Result<bool, AlignmentControlError> {
        if !self.running() {
            return Ok(false);
        }
        let before = self.state();
        let proposal_state = self.control.poll()?;
        if proposal_state == ProposalState::Waiting
            && let Some(error) = self.input_error
        {
            self.state = AlignedPlaybackState::Failed(error);
            return Ok(before != self.state());
        }
        if proposal_state == ProposalState::Unmatched {
            if let Some(scope) = self.control.scope() {
                self.reset(scope.capture_identity(), RealignmentReason::Discontinuity)
                    .await?;
            }
            self.state = AlignedPlaybackState::Unmatched;
            return Ok(before != self.state());
        }
        if proposal_state
            == ProposalState::Failed(crate::resolver::contracts::ResolverError::Stale.into())
        {
            self.withdraw();
            self.latest = None;
            self.rejected = None;
            self.control.await_new_input()?;
            self.state = match self.control.track_status() {
                Some(FuzzyTrackStatus::Collecting(count) | FuzzyTrackStatus::Consistent(count)) => {
                    AlignedPlaybackState::Collecting(count)
                }
                _ => AlignedPlaybackState::Collecting(0),
            };
            return Ok(before != self.state());
        }
        if self
            .control
            .proposal()
            .is_some_and(|latest| self.rejected == Some(latest.binding()))
        {
            return Ok(before != self.state());
        }
        if matches!(
            self.control.track_status(),
            Some(FuzzyTrackStatus::Restarted(_))
        ) {
            if let Some(next) = self.control.proposal()
                && self
                    .previous_model
                    .is_some_and(|model| model.fits_next(next))
            {
                let binding = next.binding();
                if self.drift_restart != Some(binding) {
                    self.drift_restart = Some(binding);
                    self.withdraw();
                    self.state = AlignedPlaybackState::Reacquiring(RealignmentReason::Drift);
                    return Ok(true);
                }
                return Ok(before != self.state());
            }
            let latest = self.latest.clone();
            let reason = match (self.previous_position, self.control.proposal()) {
                (Some(previous), Some(next)) if next.media_range_us().start < previous.media_us => {
                    RealignmentReason::SeekBackward
                }
                (Some(previous), Some(next))
                    if i128::from(next.media_range_us().end)
                        - i128::from(next.capture_range_us().end)
                        > i128::from(previous.media_us) - i128::from(previous.capture_us) =>
                {
                    RealignmentReason::SeekForward
                }
                _ => RealignmentReason::Discontinuity,
            };
            if let Some((event, stamp)) = latest {
                self.reset(event.identity, reason).await?;
                self.offer(event, stamp)?;
            }
            return Ok(true);
        }
        if capture.as_ref().is_some_and(|cursor| cursor.is_fresh())
            && self.control.proposal().is_some_and(|latest| {
                self.verified.as_ref().is_some_and(|old| {
                    old.binding() == latest.binding()
                        && old.validate(old.scope(), old.binding()).is_ok()
                })
            })
        {
            return Ok(before != self.state());
        }
        if let (Some(track), Some(latest), Some(cursor)) = (
            self.control.track_proposal(),
            self.control.proposal(),
            capture,
        ) {
            match self.verifier.verify(track, latest, cursor) {
                Ok(verified) => {
                    let position = verified
                        .position()
                        .map_err(|_| FuzzyAlignmentError::Obsolete)?;
                    let offset_us = verified
                        .offset_us()
                        .map_err(|_| FuzzyAlignmentError::Obsolete)?;
                    self.previous_position = Some(position);
                    self.previous_model = Some(verified.model());
                    self.drift_restart = None;
                    self.state = AlignedPlaybackState::Positioned {
                        position,
                        offset_us,
                        change: verified.change(),
                    };
                    self.withdraw();
                    self.verified = Some(Arc::new(verified));
                }
                Err(error) => {
                    self.withdraw();
                    self.state = AlignedPlaybackState::Failed(error);
                }
            }
        } else {
            self.withdraw();
            self.state = match proposal_state {
                ProposalState::Unmatched => AlignedPlaybackState::Unmatched,
                ProposalState::Failed(error) => AlignedPlaybackState::Failed(error.into()),
                ProposalState::Cancelled | ProposalState::Stopped => AlignedPlaybackState::Stopped,
                _ => match self.control.track_status() {
                    Some(
                        FuzzyTrackStatus::Collecting(count) | FuzzyTrackStatus::Consistent(count),
                    ) => AlignedPlaybackState::Collecting(count),
                    _ => self.state,
                },
            };
        }
        Ok(before != self.state())
    }
}

#[cfg(test)]
#[path = "aligned_playback_tests.rs"]
pub(crate) mod tests;
