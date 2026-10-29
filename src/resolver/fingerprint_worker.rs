//! One bounded CPU lane for one active Live session, with latest-only pending work.
//! Caller Drop cancels and joins the worker; no per-window thread or detached blocking task.
use super::{
    contracts::*,
    window::{WINDOW_FRAMES, WINDOW_SECONDS},
};
use crate::ai::speech::StreamIdentity;
use crossbeam_queue::ArrayQueue;
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle, Thread},
    time::{Duration, Instant},
};
use tokio::sync::oneshot;

const MAX_ACTIVE_FINGERPRINTS: usize = 1; // single active Live session / resolver request
const PENDING_WINDOWS: usize = 1;
const WORKER_POLL: Duration = Duration::from_millis(10);
fn worker_budget(logical_cpus: usize) -> usize {
    logical_cpus.clamp(1, MAX_ACTIVE_FINGERPRINTS)
}
type Reply = Result<(), ResolverError>;
pub struct FingerprintResult {
    pub stamp: WindowStamp,
    pub fingerprint: Fingerprint,
}
struct Latest {
    stamp: Option<WindowStamp>,
    completed: Option<Fingerprint>, // one result slot, even if callers never poll replies
}
struct QueuedJob {
    input: Arc<AudioWindow>,
    cancel: ResolverCancellation,
    reply: Option<oneshot::Sender<Reply>>,
}
impl Drop for QueuedJob {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
struct Inner {
    identity: StreamIdentity,
    queue: ArrayQueue<QueuedJob>,
    latest: Mutex<Latest>,
    active: Mutex<Option<ResolverCancellation>>,
    stopped: AtomicBool,
    wake: OnceLock<Thread>,
}
impl Inner {
    fn cancel_active(&self) {
        // Small private token state; no lock is held while extracting or awaiting.
        // Poison recovery is safe because this field contains only an immutable token.
        if let Some(token) = self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
        {
            token.cancel();
        }
    }
    fn drain(&self) {
        while let Some(job) = self.queue.pop() {
            drop(job);
        }
    }
}
#[derive(Clone)]
pub struct FingerprintClient(Arc<Inner>);
pub struct FingerprintJob {
    reply: Option<oneshot::Receiver<Reply>>,
    cancel: ResolverCancellation,
    state: Arc<Inner>,
    stamp: WindowStamp,
}
impl FingerprintJob {
    pub async fn result(mut self) -> Result<FingerprintResult, ResolverError> {
        self.reply
            .take()
            .ok_or(ResolverError::InvalidData)?
            .await
            .unwrap_or(Err(ResolverError::Cancelled))?;
        let mut latest = self
            .state
            .latest
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.state.stopped.load(Ordering::Acquire) {
            return Err(ResolverError::Cancelled);
        }
        if latest
            .stamp
            .is_none_or(|stamp| stamp.sequence != self.stamp.sequence)
        {
            return Err(ResolverError::Stale);
        }
        self.stamp.validate(self.state.identity, Instant::now())?;
        let fingerprint = latest.completed.take().ok_or(ResolverError::InvalidData)?;
        Ok(FingerprintResult {
            stamp: self.stamp,
            fingerprint,
        })
    }
}
impl Drop for FingerprintJob {
    fn drop(&mut self) {
        self.cancel.cancel();
        let mut latest = self
            .state
            .latest
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if latest
            .stamp
            .is_some_and(|stamp| stamp.sequence == self.stamp.sequence)
        {
            latest.completed.take();
        }
    }
}
impl FingerprintClient {
    /// Called by orchestration, never capture/UI callbacks. Tiny metadata locks only;
    /// the CPU lane cannot make the caller wait for extraction, queue space or a reply.
    pub fn request(&self, input: Arc<AudioWindow>) -> Result<FingerprintJob, ResolverError> {
        input.stamp.validate(self.0.identity, Instant::now())?;
        if input.samples().len() != WINDOW_FRAMES
            || input.stamp.duration_us != WINDOW_SECONDS as u64 * 1_000_000
        {
            return Err(ResolverError::InvalidData);
        }
        let mut latest = self
            .0
            .latest
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.0.stopped.load(Ordering::Acquire) {
            return Err(ResolverError::Cancelled);
        }
        if latest.stamp.is_some_and(|previous| {
            input.stamp.sequence <= previous.sequence || input.stamp.ended_at < previous.ended_at
        }) {
            return Err(ResolverError::Stale);
        }
        let stamp = input.stamp;
        latest.stamp = Some(stamp);
        latest.completed.take();
        let (reply, receiver) = oneshot::channel();
        let cancel = ResolverCancellation::default();
        self.0.cancel_active();
        drop(self.0.queue.force_push(QueuedJob {
            input,
            cancel: cancel.clone(),
            reply: Some(reply),
        }));
        if let Some(thread) = self.0.wake.get() {
            thread.unpark();
        }
        Ok(FingerprintJob {
            reply: Some(receiver),
            cancel,
            state: self.0.clone(),
            stamp,
        })
    }
}
pub struct FingerprintWorker {
    inner: Arc<Inner>,
    worker: Option<JoinHandle<()>>,
    worker_count: usize,
}
impl FingerprintWorker {
    pub fn start(
        identity: StreamIdentity,
        extractor: Arc<dyn FingerprintExtractor>,
    ) -> Result<Self, ResolverError> {
        let worker_count =
            worker_budget(thread::available_parallelism().map_or(1, |count| count.get()));
        let inner = Arc::new(Inner {
            identity,
            queue: ArrayQueue::new(PENDING_WINDOWS),
            latest: Mutex::new(Latest {
                stamp: None,
                completed: None,
            }),
            active: Mutex::new(None),
            stopped: AtomicBool::new(false),
            wake: OnceLock::new(),
        });
        let state = inner.clone();
        let worker = thread::Builder::new()
            .name("subtitle-fingerprint".into())
            .spawn(move || {
                while !state.stopped.load(Ordering::Acquire) {
                    let Some(mut job) = state.queue.pop() else {
                        thread::park_timeout(WORKER_POLL);
                        continue;
                    };
                    {
                        let latest = state
                            .latest
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        if state.stopped.load(Ordering::Acquire)
                            || job.cancel.is_cancelled()
                            || latest
                                .stamp
                                .is_none_or(|stamp| stamp.sequence != job.input.stamp.sequence)
                        {
                            continue;
                        }
                        *state
                            .active
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner) =
                            Some(job.cancel.clone());
                    }
                    let result = job
                        .input
                        .stamp
                        .validate(state.identity, Instant::now())
                        .and_then(|()| {
                            catch_unwind(AssertUnwindSafe(|| {
                                extractor.extract(&job.input, &job.cancel)
                            }))
                            .unwrap_or(Err(ResolverError::WorkerPanicked))
                        });
                    // Serialize publication against the next submission: a superseded
                    // extraction can never publish success after a newer window is queued.
                    let mut latest = state
                        .latest
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let result = if state.stopped.load(Ordering::Acquire)
                        || job.cancel.is_cancelled()
                        || latest
                            .stamp
                            .is_none_or(|stamp| stamp.sequence != job.input.stamp.sequence)
                    {
                        Err(ResolverError::Cancelled)
                    } else {
                        result.and_then(|fingerprint| {
                            job.input.stamp.validate(state.identity, Instant::now())?;
                            latest.completed = Some(fingerprint);
                            Ok(())
                        })
                    };
                    state
                        .active
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .take();
                    if let Some(reply) = job.reply.take()
                        && reply.send(result).is_err()
                    {
                        latest.completed.take();
                    }
                    drop(latest);
                }
                state.drain();
            })
            .map_err(|_| ResolverError::WorkerStart)?;
        // Only this constructor sets the wake handle. Failure still owns and joins the thread.
        if inner.wake.set(worker.thread().clone()).is_err() {
            inner.stopped.store(true, Ordering::Release);
            let _ = worker.join();
            return Err(ResolverError::WorkerStart);
        }
        Ok(Self {
            inner,
            worker: Some(worker),
            worker_count,
        })
    }
    pub fn client(&self) -> FingerprintClient {
        FingerprintClient(self.inner.clone())
    }
    pub fn worker_count(&self) -> usize {
        self.worker_count
    }
    pub fn stop(&mut self) -> Result<(), ResolverError> {
        let mut latest = self
            .inner
            .latest
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.inner.stopped.store(true, Ordering::Release);
        self.inner.cancel_active();
        self.inner.drain();
        latest.completed.take();
        drop(latest);
        if let Some(thread) = self.inner.wake.get() {
            thread.unpark();
        }
        match self.worker.take() {
            Some(worker) => worker.join().map_err(|_| ResolverError::WorkerPanicked),
            None => Ok(()),
        }
    }
}
impl Drop for FingerprintWorker {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[cfg(test)]
#[path = "fingerprint_tests.rs"]
mod tests;
