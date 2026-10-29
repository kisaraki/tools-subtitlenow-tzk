//! Explicit alignment lifecycle only: never operate a player or start capture.
use super::{
    fuzzy_proposals::{FuzzyProposalSession, ProposalState},
    prepared::PreparedSession,
};
use crate::{
    ai::speech::{StreamIdentity, TranscriptEvent},
    resolver::{
        contracts::{ResolverError, WindowStamp},
        text_session::ExistingTextLease,
    },
    subtitle::{
        fuzzy_alignment::{AlignmentScope, FuzzyAlignmentError, FuzzyCueProposal},
        fuzzy_track::{FuzzyTrackProposal, FuzzyTrackStatus},
        fuzzy_worker::{FuzzyCueLocator, FuzzyLocatorBinding},
    },
};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::{runtime::Handle, task::JoinHandle};

// Shared by this controller's publications/runs, never serialized or reset.
static NEXT_EPOCH: AtomicU64 = AtomicU64::new(1);
fn allocate_epoch(counter: &AtomicU64) -> Result<u64, AlignmentControlError> {
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |old| {
            old.checked_add(1)
        })
        .map_err(|_| AlignmentControlError::EpochExhausted)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AlignmentControlError {
    #[error("{0}")]
    Alignment(#[from] FuzzyAlignmentError),
    #[error("對齊已暫停，請明確恢復後再送入辨識結果。")]
    Paused,
    #[error("對齊控制器已停止，請為新的來源建立控制器。")]
    Stopped,
    #[error("對齊版本序號已用盡，未重用舊版本。")]
    EpochExhausted,
    #[error("對齊背景工作異常結束；已撤回舊結果。")]
    WorkerFailed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlignmentControlState {
    Paused,
    Running,
    Unavailable(FuzzyAlignmentError),
    Stopped,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Paused,
    Running,
    Stopped,
}
struct Active {
    owner: FuzzyProposalSession,
    worker: JoinHandle<Result<(), FuzzyAlignmentError>>,
    // Cache immediately on join: dropping a pause while waiting for CPU cleanup
    // must never cause a completed JoinHandle to be polled a second time.
    joined: Option<Result<(), AlignmentControlError>>,
}
/// Explicit, RAM-only alignment owner. Construction is passive; resume starts
/// one driver on the caller's timer-enabled Tokio runtime. Keep that runtime
/// alive through stop().await to confirm cleanup of admitted blocking work.
/// Lifecycle waits are cancellation-safe: a dropped wait keeps its old driver
/// and join result here; the next operation must finish cleanup before resuming.
/// Paused/Stopped signal input withdrawal, not completion of that cleanup.
pub struct AlignmentController {
    prepared_history: bool,
    source: Option<FuzzyLocatorBinding>,
    capture: StreamIdentity,
    active: Option<Active>,
    phase: Phase,
}
impl AlignmentController {
    pub fn from_source(
        source: &crate::subtitle::fuzzy_worker::AlignmentSource,
        capture: StreamIdentity,
    ) -> Result<Self, AlignmentControlError> {
        let mut owner = Self::from_fresh(
            source.locator(capture, allocate_epoch(&NEXT_EPOCH)?)?,
            capture,
        )?;
        owner.prepared_history = true;
        Ok(owner)
    }
    /// Passive, paused construction. No runtime, task, capture or service I/O.
    pub fn from_prepared(
        source: &PreparedSession,
        capture: StreamIdentity,
    ) -> Result<Self, AlignmentControlError> {
        let locator = source.fuzzy_locator(capture, allocate_epoch(&NEXT_EPOCH)?)?;
        Self::from_fresh(locator, capture)
    }
    pub fn from_lookup(
        source: &ExistingTextLease,
        capture: StreamIdentity,
    ) -> Result<Self, AlignmentControlError> {
        let scope = AlignmentScope::new(
            source.query().stamp.identity,
            capture,
            allocate_epoch(&NEXT_EPOCH)?,
        )?;
        Self::from_fresh(FuzzyCueLocator::from_lookup(scope, source)?, capture)
    }
    fn from_fresh(
        locator: FuzzyCueLocator,
        capture: StreamIdentity,
    ) -> Result<Self, AlignmentControlError> {
        Ok(Self {
            prepared_history: false,
            source: Some(locator.into_binding()?),
            capture,
            active: None,
            phase: Phase::Paused,
        })
    }
    pub fn state(&self) -> AlignmentControlState {
        if self.phase == Phase::Stopped {
            return AlignmentControlState::Stopped;
        }
        let Some(source) = &self.source else {
            return AlignmentControlState::Unavailable(FuzzyAlignmentError::WorkerFailed);
        };
        if let Err(error) = source.validate() {
            return AlignmentControlState::Unavailable(error);
        }
        if self.phase == Phase::Paused {
            return AlignmentControlState::Paused;
        }
        match &self.active {
            Some(active) if !active.worker.is_finished() => AlignmentControlState::Running,
            _ => AlignmentControlState::Unavailable(FuzzyAlignmentError::WorkerFailed),
        }
    }
    pub fn scope(&self) -> Option<AlignmentScope> {
        (self.state() == AlignmentControlState::Running)
            .then(|| self.active.as_ref().map(|active| active.owner.scope()))
            .flatten()
    }
    /// Explicitly start one caller-runtime-owned driver. Resuming an already
    /// healthy run is idempotent; resuming after pause always gets a fresh epoch.
    pub async fn resume(&mut self) -> Result<AlignmentScope, AlignmentControlError> {
        if self.phase == Phase::Stopped {
            return Err(AlignmentControlError::Stopped);
        }
        if self.state() == AlignmentControlState::Running {
            return self.scope().ok_or(AlignmentControlError::WorkerFailed);
        }
        self.quiesce(Phase::Paused).await?;
        let runtime = Handle::try_current().map_err(|_| {
            AlignmentControlError::Alignment(FuzzyAlignmentError::RuntimeUnavailable)
        })?;
        let source = self.source.as_ref().ok_or(AlignmentControlError::Stopped)?;
        source.validate()?;
        let locator = source.locator(self.capture, allocate_epoch(&NEXT_EPOCH)?)?;
        let (owner, task) = if self.prepared_history {
            FuzzyProposalSession::new_prepared_history(locator)?
        } else {
            FuzzyProposalSession::new(locator)?
        };
        let scope = owner.scope();
        self.active = Some(Active {
            owner,
            worker: runtime.spawn(task.run()),
            joined: None,
        });
        self.phase = Phase::Running;
        Ok(scope)
    }
    pub async fn pause(&mut self) -> Result<(), AlignmentControlError> {
        if self.phase == Phase::Stopped {
            return Err(AlignmentControlError::Stopped);
        }
        self.quiesce(Phase::Paused).await
    }
    /// Caller has observed a seek/discontinuity or changed capture binding.
    /// This resets alignment, not the media player, and invents no position.
    pub async fn reset(
        &mut self,
        capture: StreamIdentity,
    ) -> Result<AlignmentScope, AlignmentControlError> {
        if self.phase == Phase::Stopped {
            return Err(AlignmentControlError::Stopped);
        }
        if capture.session.0 == 0 || capture.generation == 0 {
            return Err(FuzzyAlignmentError::Invalid(ResolverError::InvalidData).into());
        }
        self.quiesce(Phase::Paused).await?;
        self.capture = capture;
        self.resume().await
    }
    /// Terminal and repeatable. If this wait is dropped, call stop again to
    /// finish cleanup; no subsequent resume can start a new driver.
    pub async fn stop(&mut self) -> Result<(), AlignmentControlError> {
        let result = self.quiesce(Phase::Stopped).await;
        self.source = None;
        result
    }
    async fn quiesce(&mut self, phase: Phase) -> Result<(), AlignmentControlError> {
        self.phase = phase;
        let Some(active) = self.active.as_mut() else {
            return Ok(());
        };
        active.owner.stop();
        if active.joined.is_none() {
            active.joined = Some(match (&mut active.worker).await {
                Ok(Ok(()))
                | Ok(Err(FuzzyAlignmentError::Invalid(
                    ResolverError::Cancelled | ResolverError::Stale,
                ))) => Ok(()),
                Ok(Err(error)) => Err(error.into()),
                Err(_) => Err(AlignmentControlError::WorkerFailed),
            });
        }
        if let Some(source) = &self.source {
            source.wait_idle().await?;
        }
        self.active
            .take()
            .and_then(|active| active.joined)
            .unwrap_or(Err(AlignmentControlError::WorkerFailed))
    }
    fn running(&mut self) -> Result<&mut FuzzyProposalSession, AlignmentControlError> {
        match self.phase {
            Phase::Paused => Err(AlignmentControlError::Paused),
            Phase::Stopped => Err(AlignmentControlError::Stopped),
            Phase::Running => self
                .active
                .as_mut()
                .map(|active| &mut active.owner)
                .ok_or(AlignmentControlError::WorkerFailed),
        }
    }
    pub(crate) fn await_new_input(&mut self) -> Result<(), AlignmentControlError> {
        if !self.prepared_history {
            return Err(FuzzyAlignmentError::UnsupportedEvidence.into());
        }
        self.running()?.await_new_input();
        Ok(())
    }
    pub fn offer(
        &mut self,
        event: TranscriptEvent,
        stamp: WindowStamp,
    ) -> Result<bool, AlignmentControlError> {
        Ok(self.running()?.offer(event, stamp)?)
    }
    pub fn poll(&mut self) -> Result<ProposalState, AlignmentControlError> {
        Ok(self.running()?.poll())
    }
    pub async fn changed(&mut self) -> Result<ProposalState, AlignmentControlError> {
        Ok(self.running()?.changed().await)
    }
    pub fn proposal(&self) -> Option<&FuzzyCueProposal> {
        (self.state() == AlignmentControlState::Running)
            .then(|| {
                self.active
                    .as_ref()
                    .and_then(|active| active.owner.proposal())
            })
            .flatten()
    }
    pub fn track_proposal(&self) -> Option<FuzzyTrackProposal> {
        (self.state() == AlignmentControlState::Running)
            .then(|| {
                self.active
                    .as_ref()
                    .and_then(|active| active.owner.track_proposal())
            })
            .flatten()
    }
    pub fn track_status(&self) -> Option<FuzzyTrackStatus> {
        (self.phase == Phase::Running)
            .then(|| {
                self.active
                    .as_ref()
                    .map(|active| active.owner.track_status())
            })
            .flatten()
    }
}

impl Drop for AlignmentController {
    fn drop(&mut self) {
        if let Some(active) = &mut self.active {
            active.owner.stop();
            active.worker.abort();
        }
    }
}

#[cfg(test)]
#[path = "alignment_control_tests.rs"]
mod tests;
