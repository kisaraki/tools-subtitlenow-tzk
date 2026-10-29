//! Caller-polled resolver task: one request, one latest input, one latest result.
//! No task is spawned/detached here, and resolver failure never cancels Live STT.
use super::{
    candidates::{CandidateTracker, Resolution},
    contracts::*,
    window::{WINDOW_FRAMES, WINDOW_SECONDS},
};
use crate::ai::speech::StreamIdentity;
use std::{
    sync::{Arc, Weak},
    time::{Duration, Instant},
};
use tokio::sync::watch;

pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
struct RequestGuard(ResolverCancellation);
impl Drop for RequestGuard {
    fn drop(&mut self) {
        self.0.cancel();
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolverStatus {
    Resolving,
    Ready,
    Failed(ResolverError),
    Stopped,
}
pub struct ResolverUpdate {
    pub stamp: WindowStamp,
    pub emitted_at: Instant,
    pub status: ResolverStatus,
    pub resolution: Resolution,
    epoch: u64,
}
#[cfg(test)]
impl ResolverUpdate {
    pub(crate) fn fixture(
        stamp: WindowStamp,
        status: ResolverStatus,
        resolution: Resolution,
    ) -> Self {
        Self {
            stamp,
            emitted_at: Instant::now(),
            status,
            resolution,
            epoch: 0,
        }
    }
}
#[derive(Clone, Default)]
struct InputFrame {
    epoch: u64,
    window: Option<Arc<AudioWindow>>,
    cancel: ResolverCancellation,
}
pub struct ResolverSession {
    identity: StreamIdentity,
    input: watch::Sender<InputFrame>,
    output: watch::Receiver<Option<Arc<ResolverUpdate>>>,
    output_owner: watch::Sender<Option<Arc<ResolverUpdate>>>,
    cancel: ResolverCancellation,
    previous: Option<WindowStamp>,
    observed: Option<Weak<ResolverUpdate>>,
    epoch: u64,
}
pub struct ResolverTask {
    identity: StreamIdentity,
    capabilities: ResolverCapabilities,
    input: watch::Receiver<InputFrame>,
    input_owner: watch::Sender<InputFrame>,
    output: watch::Sender<Option<Arc<ResolverUpdate>>>,
    cancel: ResolverCancellation,
    current: Option<WindowStamp>,
    epoch: u64,
}
impl ResolverSession {
    pub fn new(
        identity: StreamIdentity,
        capabilities: ResolverCapabilities,
    ) -> Result<(Self, ResolverTask), ResolverError> {
        capabilities.validate()?;
        let (input, receiver) = watch::channel(InputFrame::default());
        let (output, results) = watch::channel(None);
        let cancel = ResolverCancellation::default();
        Ok((
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
            ResolverTask {
                identity,
                capabilities,
                input: receiver,
                input_owner: input,
                output,
                cancel,
                current: None,
                epoch: 0,
            },
        ))
    }
    pub fn cancellation(&self) -> ResolverCancellation {
        self.cancel.clone()
    }
    /// Non-waiting latest-window replacement. The caller is an orchestration/CPU worker,
    /// never the native capture callback. Old queued windows are immediately released.
    pub fn try_submit(&mut self, window: Arc<AudioWindow>) -> Result<(), ResolverError> {
        if self.cancel.is_cancelled() {
            return Err(ResolverError::Cancelled);
        }
        window.stamp.validate(self.identity, Instant::now())?;
        if window.samples().len() != WINDOW_FRAMES
            || window.stamp.duration_us != WINDOW_SECONDS as u64 * 1_000_000
        {
            return Err(ResolverError::InvalidData);
        }
        if self.previous.is_some_and(|old| {
            window.stamp.sequence <= old.sequence || window.stamp.ended_at < old.ended_at
        }) {
            return Err(ResolverError::Stale);
        }
        self.cancel_request();
        let stamp = window.stamp;
        let replaced = self.replace_input(
            InputFrame {
                epoch: self.epoch,
                window: Some(window),
                cancel: ResolverCancellation::default(),
            },
            None,
        );
        if !replaced {
            return Err(ResolverError::Cancelled);
        }
        self.previous = Some(stamp);
        Ok(())
    }
    fn cancel_request(&self) {
        let cancel = self.input.borrow().cancel.clone();
        cancel.cancel();
    }
    fn empty_update(&self, status: ResolverStatus) -> Option<Arc<ResolverUpdate>> {
        self.previous.map(|stamp| {
            Arc::new(ResolverUpdate {
                stamp,
                emitted_at: Instant::now(),
                status,
                resolution: Resolution::default(),
                epoch: self.epoch,
            })
        })
    }
    fn replace_input(&self, frame: InputFrame, publication: Option<Arc<ResolverUpdate>>) -> bool {
        // Clear the output under the same write gate as input replacement. A valid
        // new-window result must not be erased after the worker observes that input.
        let mut old_frame = None;
        let mut old_output = None;
        let replaced = self.input.send_if_modified(|current| {
            if frame.window.is_some() && self.cancel.is_cancelled() {
                return false;
            }
            old_frame = Some(std::mem::replace(current, frame));
            old_output = self.output_owner.send_replace(publication);
            true
        });
        drop(old_frame);
        drop(old_output); // Audio/metadata destruction stays outside the gate.
        replaced
    }
    fn reset(&mut self, error: ResolverError) -> Result<(), ResolverError> {
        self.cancel_request();
        let Some(epoch) = self.epoch.checked_add(1) else {
            self.stop();
            return Err(ResolverError::InvalidData);
        };
        self.epoch = epoch;
        self.replace_input(
            InputFrame {
                epoch,
                window: None,
                cancel: ResolverCancellation::default(),
            },
            self.empty_update(ResolverStatus::Failed(error)),
        );
        Ok(())
    }
    /// Clears shared evidence and signals the active request immediately. The epoch
    /// travels with later windows so a coalesced notification cannot erase a reset.
    pub fn invalidate(&mut self) -> Result<(), ResolverError> {
        if self.cancel.is_cancelled() {
            return Err(ResolverError::Cancelled);
        }
        self.reset(ResolverError::AudioDiscontinuity)
    }
    /// Remove shared evidence and pending audio synchronously, and signal the active
    /// request. Poll/drop the task to release its future and private voting state.
    /// Repeated stop does not republish the terminal snapshot.
    pub fn stop(&mut self) {
        self.cancel.cancel();
        self.cancel_request();
        let stopped = self.output.borrow().as_ref().is_some_and(|update| {
            update.epoch == self.epoch && update.status == ResolverStatus::Stopped
        });
        if !stopped {
            self.replace_input(
                InputFrame {
                    epoch: self.epoch,
                    window: None,
                    cancel: ResolverCancellation::default(),
                },
                self.empty_update(ResolverStatus::Stopped),
            );
        }
    }
    pub fn poll(&mut self) -> Option<Arc<ResolverUpdate>> {
        if self.cancel.is_cancelled() {
            self.stop();
        }
        let mut result = self.output.borrow_and_update().clone()?;
        if result.epoch != self.epoch {
            return None;
        }
        if matches!(
            result.status,
            ResolverStatus::Ready | ResolverStatus::Resolving
        ) && let Err(error) = result.stamp.validate(self.identity, Instant::now())
        {
            let _ = self.reset(error); // Overflow stops instead of wrapping the epoch.
            result = self.output.borrow_and_update().clone()?;
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
    pub async fn next(&mut self) -> Option<Arc<ResolverUpdate>> {
        loop {
            if let Some(result) = self.poll() {
                return Some(result);
            }
            if self.cancel.is_cancelled() {
                return None;
            }
            let deadline = {
                let output = self.output.borrow();
                output
                    .as_ref()
                    .filter(|update| {
                        matches!(
                            update.status,
                            ResolverStatus::Ready | ResolverStatus::Resolving
                        )
                    })
                    .and_then(|update| update.stamp.ended_at.checked_add(MAX_WINDOW_AGE))
            };
            tokio::select! { biased;
                _ = self.cancel.cancelled() => {},
                changed = self.output.changed() => { if changed.is_err() { return self.poll(); } },
                _ = async { match deadline { Some(at) => tokio::time::sleep_until(at.into()).await, None => std::future::pending().await } } => {},
            }
        }
    }
}
impl Drop for ResolverSession {
    fn drop(&mut self) {
        self.stop();
    }
}
impl ResolverTask {
    fn publish(&self, stamp: WindowStamp, status: ResolverStatus, resolution: Resolution) {
        let publication = Arc::new(ResolverUpdate {
            stamp,
            emitted_at: Instant::now(),
            status,
            resolution,
            epoch: self.epoch,
        });
        // Serialize epoch/latest-window checks with owner replacement. Otherwise
        // a late result can repopulate a cleared slot even when poll rejects it.
        let input = self.input.borrow();
        if input.epoch != self.epoch
            || (status != ResolverStatus::Stopped
                && (self.cancel.is_cancelled()
                    || input.cancel.is_cancelled()
                    || input
                        .window
                        .as_ref()
                        .is_none_or(|window| window.stamp.sequence != stamp.sequence)))
        {
            return;
        }
        if status == ResolverStatus::Stopped
            && self
                .output
                .borrow()
                .as_ref()
                .is_some_and(|old| old.epoch == self.epoch && old.status == ResolverStatus::Stopped)
        {
            return; // Preserve the owner's newer stamp and one terminal notification.
        }
        let previous = self.output.send_replace(Some(publication));
        drop(input);
        drop(previous); // Release metadata outside the short input gate.
    }
    pub async fn run(mut self, provider: &dyn ContentIdentifier) -> Result<(), ResolverError> {
        let actual = provider.capabilities();
        actual.validate()?;
        if actual.classification != self.capabilities.classification
            || actual.songs != self.capabilities.songs
            || actual.movies != self.capabilities.movies
            || actual.cancellation != self.capabilities.cancellation
        {
            return Err(ResolverError::UnsupportedCapability);
        }
        let mut tracker = CandidateTracker::new(self.identity);
        let mut next = None;
        loop {
            if self.cancel.is_cancelled() {
                return Ok(());
            }
            let frame = if let Some(frame) = next.take() {
                frame
            } else {
                tokio::select! { biased;
                    _ = self.cancel.cancelled() => return Ok(()),
                    changed = self.input.changed() => { if changed.is_err() { return Ok(()); } }
                }
                self.input.borrow_and_update().clone()
            };
            if frame.epoch != self.epoch {
                self.epoch = frame.epoch;
                tracker.clear(); // The owner already published the reset atomically.
            }
            let Some(window) = frame.window else {
                continue;
            };
            if frame.cancel.is_cancelled() {
                continue;
            }
            self.current = Some(window.stamp);
            if let Err(error) = window.stamp.validate(self.identity, Instant::now()) {
                tracker.clear();
                self.publish(
                    window.stamp,
                    ResolverStatus::Failed(error),
                    Resolution::default(),
                );
                continue;
            }
            self.publish(
                window.stamp,
                ResolverStatus::Resolving,
                Resolution::default(),
            );
            let request_cancel = frame.cancel;
            let _request = RequestGuard(request_cancel.clone());
            let result = tokio::select! { biased;
                _ = self.cancel.cancelled() => return Ok(()),
                _ = request_cancel.cancelled() => {
                    self.input.mark_changed();
                    continue;
                },
                changed = self.input.changed() => {
                    if changed.is_err() { return Ok(()); }
                    next = Some(self.input.borrow_and_update().clone());
                    continue;
                },
                result = tokio::time::timeout(REQUEST_TIMEOUT, provider.identify(&window, request_cancel.clone())) => result.unwrap_or(Err(ResolverError::Timeout)),
            };
            let result = result.and_then(|evidence| {
                if !actual.accepts(&evidence) {
                    return Err(ResolverError::UnsupportedCapability);
                }
                tracker.observe(window.stamp, evidence, Instant::now())
            });
            match result {
                Ok(resolution) => self.publish(window.stamp, ResolverStatus::Ready, resolution),
                Err(error) => {
                    tracker.clear();
                    self.publish(
                        window.stamp,
                        ResolverStatus::Failed(error),
                        Resolution::default(),
                    );
                }
            }
        }
    }
}
impl Drop for ResolverTask {
    fn drop(&mut self) {
        self.cancel.cancel();
        let (stamp, epoch, request) = {
            let input = self.input_owner.borrow();
            (
                input
                    .window
                    .as_ref()
                    .map(|window| window.stamp)
                    .or(self.current),
                input.epoch,
                input.cancel.clone(),
            )
        };
        request.cancel();
        self.input_owner.send_modify(|frame| frame.window = None);
        self.epoch = epoch;
        if let Some(stamp) = stamp {
            self.publish(stamp, ResolverStatus::Stopped, Resolution::default());
        }
    }
}

#[cfg(test)]
#[path = "session_lifecycle_tests.rs"]
mod lifecycle_tests;
