//! Bounded, session-owned 48 kHz -> 16 kHz -> sliding-window CPU lane.
//! Only orchestration calls this API; native capture never waits on this worker.
use super::{contracts::*, window::SlidingAudioWindow};
use crate::{
    ai::speech::StreamIdentity,
    audio::{
        capture::{BLOCK_FRAMES, MAX_AUDIO_AGE, RING_BLOCKS},
        preprocess::NormalizedAudio,
        resample::{ResampleError, StreamingResampler},
    },
};
use crossbeam_queue::ArrayQueue;
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Mutex, MutexGuard, OnceLock,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    thread::{self, JoinHandle, Thread},
    time::Instant,
};
use tokio::sync::watch;

/// Same 240 ms budget as normalized capture, with no unbounded intermediate PCM.
pub const AUDIO_QUEUE_BLOCKS: usize = RING_BLOCKS;
const MAX_AUDIO_WORKERS: usize = 1;

pub struct AudioAssembly {
    pub epoch: u64,
    pub window: Option<Arc<AudioWindow>>,
    pub error: Option<ResolverError>,
}
#[derive(Clone, Copy, Debug)]
pub struct AudioWorkerMetrics {
    pub queue_depth: usize,
    pub processed_blocks: u64,
    pub dropped_blocks: u64,
    pub resets: u64,
    pub worker_count: usize,
}
struct QueuedBlock {
    epoch: u64,
    audio: NormalizedAudio,
}
struct State {
    epoch: u64,
    stopped: bool,
    previous: Option<(u64, Option<u64>, usize, Instant)>,
}
struct Inner {
    identity: StreamIdentity,
    state: Mutex<State>,
    queue: ArrayQueue<QueuedBlock>,
    output: watch::Sender<Arc<AudioAssembly>>,
    wake: OnceLock<Thread>,
    processed: AtomicU64,
    dropped: AtomicU64,
    workers: AtomicUsize,
}
fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(|error| error.into_inner())
}
impl Inner {
    fn clear_queue(&self) {
        while self.queue.pop().is_some() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
    /// Called under state lock: submissions, reset publication and completion share
    /// one ordering gate. No FIR, allocation of PCM windows, network or await here.
    fn reset(&self, state: &mut State, error: ResolverError) -> Result<(), ResolverError> {
        state.epoch = state
            .epoch
            .checked_add(1)
            .ok_or(ResolverError::InvalidData)?;
        self.clear_queue();
        self.output.send_replace(Arc::new(AudioAssembly {
            epoch: state.epoch,
            window: None,
            error: Some(error),
        }));
        if let Some(worker) = self.wake.get() {
            worker.unpark();
        }
        Ok(())
    }
    fn fail(&self, epoch: u64, error: ResolverError) {
        let mut state = lock(&self.state);
        if !state.stopped && state.epoch == epoch && self.reset(&mut state, error).is_err() {
            self.finish(&mut state, ResolverError::InvalidData);
        }
    }
    fn finish(&self, state: &mut State, error: ResolverError) {
        state.stopped = true;
        self.clear_queue();
        self.output.send_replace(Arc::new(AudioAssembly {
            epoch: state.epoch,
            window: None,
            error: Some(error),
        }));
    }
}

#[derive(Clone)]
pub struct ResolverAudioInput(Arc<Inner>);
impl ResolverAudioInput {
    /// Non-waiting queue submission, with a short metadata lock on the caller's
    /// orchestration thread. Full queue discards backlog, resets evidence, and keeps
    /// the newest block. Resolver overload never cancels the speech pipeline.
    pub fn try_push(&self, audio: NormalizedAudio) -> Result<(), ResolverError> {
        let inner = &self.0;
        if audio.stamp.session != inner.identity.session {
            return Err(ResolverError::InvalidData);
        }
        let mut state = lock(&inner.state);
        if state.stopped {
            return Err(ResolverError::Cancelled);
        }
        if state
            .previous
            .is_some_and(|(seq, _, _, at)| audio.stamp.sequence <= seq || audio.stamp.received < at)
        {
            return Err(ResolverError::Stale);
        }
        let valid = audio.frames > 0
            && audio.frames <= BLOCK_FRAMES
            && audio.samples[..audio.frames]
                .iter()
                .all(|sample| sample.is_finite() && (-1.0..=1.0).contains(sample));
        if !valid {
            inner.reset(&mut state, ResolverError::InvalidData)?;
            return Err(ResolverError::InvalidData);
        }
        if Instant::now()
            .checked_duration_since(audio.stamp.received)
            .is_none_or(|age| age > MAX_AUDIO_AGE)
        {
            inner.reset(&mut state, ResolverError::Stale)?;
            return Err(ResolverError::Stale);
        }
        let gap =
            audio.stamp.discontinuity
                || state.previous.is_some_and(|(seq, position, frames, _)| {
                    seq.checked_add(1) != Some(audio.stamp.sequence)
                        || position.zip(audio.stamp.frame_position).is_some_and(
                            |(before, after)| before.checked_add(frames as u64) != Some(after),
                        )
                });
        if gap || inner.queue.is_full() {
            inner.reset(&mut state, ResolverError::AudioDiscontinuity)?;
        }
        state.previous = Some((
            audio.stamp.sequence,
            audio.stamp.frame_position,
            audio.frames,
            audio.stamp.received,
        ));
        let block = QueuedBlock {
            epoch: state.epoch,
            audio,
        };
        // Consumer also pops under state lock; capacity cannot change to full here.
        inner
            .queue
            .push(block)
            .map_err(|_| ResolverError::InvalidData)?;
        drop(state);
        if let Some(worker) = inner.wake.get() {
            worker.unpark();
        }
        Ok(())
    }
}

pub struct ResolverAudioWorker {
    inner: Arc<Inner>,
    output: watch::Receiver<Arc<AudioAssembly>>,
    worker: Option<JoinHandle<()>>,
}
impl ResolverAudioWorker {
    pub fn new(identity: StreamIdentity) -> Result<Self, ResolverError> {
        Self::start(identity, || {})
    }
    fn start(
        identity: StreamIdentity,
        before_block: impl Fn() + Send + 'static,
    ) -> Result<Self, ResolverError> {
        let budget = thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1)
            .clamp(1, MAX_AUDIO_WORKERS);
        let (output, receiver) = watch::channel(Arc::new(AudioAssembly {
            epoch: 0,
            window: None,
            error: None,
        }));
        let inner = Arc::new(Inner {
            identity,
            state: Mutex::new(State {
                epoch: 0,
                stopped: false,
                previous: None,
            }),
            queue: ArrayQueue::new(AUDIO_QUEUE_BLOCKS),
            output,
            wake: OnceLock::new(),
            processed: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            workers: AtomicUsize::new(budget),
        });
        let worker_inner = Arc::clone(&inner);
        let worker = thread::Builder::new()
            .name("resolver-audio".into())
            .spawn(move || {
                let result = catch_unwind(AssertUnwindSafe(|| run(&worker_inner, before_block)));
                let mut state = lock(&worker_inner.state);
                let error = if result.is_err() {
                    ResolverError::WorkerPanicked
                } else {
                    ResolverError::Cancelled
                };
                worker_inner.finish(&mut state, error);
                worker_inner.workers.store(0, Ordering::Release);
            })
            .map_err(|_| ResolverError::WorkerStart)?;
        if inner.wake.set(worker.thread().clone()).is_err() {
            lock(&inner.state).stopped = true;
            worker.thread().unpark();
            let _ = worker.join();
            return Err(ResolverError::WorkerStart);
        }
        Ok(Self {
            inner,
            output: receiver,
            worker: Some(worker),
        })
    }
    pub fn input(&self) -> ResolverAudioInput {
        ResolverAudioInput(Arc::clone(&self.inner))
    }
    /// Mark this latest-only mailbox observed; callers propagate its epoch before
    /// submitting its window. The error remains attached across coalesced updates.
    pub fn latest(&mut self) -> Arc<AudioAssembly> {
        self.output.borrow_and_update().clone()
    }
    pub fn current(&self) -> Arc<AudioAssembly> {
        self.output.borrow().clone()
    }
    pub async fn changed(&mut self) -> Arc<AudioAssembly> {
        let _ = self.output.changed().await;
        self.latest()
    }
    pub fn metrics(&self) -> AudioWorkerMetrics {
        let state = lock(&self.inner.state);
        AudioWorkerMetrics {
            queue_depth: self.inner.queue.len(),
            processed_blocks: self.inner.processed.load(Ordering::Acquire),
            dropped_blocks: self.inner.dropped.load(Ordering::Relaxed),
            resets: state.epoch,
            worker_count: self.inner.workers.load(Ordering::Acquire),
        }
    }
    pub fn stop(&mut self) -> Result<(), ResolverError> {
        {
            let mut state = lock(&self.inner.state);
            // Retain a panic diagnosis rather than replacing it on repeated stop.
            if !state.stopped {
                self.inner.finish(&mut state, ResolverError::Cancelled);
            }
        }
        if let Some(worker) = self.inner.wake.get() {
            worker.unpark();
        }
        if let Some(worker) = self.worker.take() {
            worker.join().map_err(|_| ResolverError::WorkerPanicked)?;
        }
        Ok(())
    }
}
impl Drop for ResolverAudioWorker {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn run(inner: &Inner, before_block: impl Fn()) {
    let mut epoch = None;
    let mut resampler = None;
    let mut windows = SlidingAudioWindow::new(inner.identity);
    loop {
        let (current_epoch, job) = {
            let state = lock(&inner.state);
            if state.stopped {
                return;
            }
            (state.epoch, inner.queue.pop())
        };
        // Reset even without another packet, so an idle gap does not retain PCM.
        if epoch != Some(current_epoch) {
            windows.reset();
            resampler = None;
            epoch = None;
        }
        let Some(job) = job else {
            thread::park();
            continue;
        };
        before_block(); // Empty in production; deterministic scheduling hook for tests.
        if epoch != Some(job.epoch) {
            epoch = Some(job.epoch);
            windows.reset();
            match StreamingResampler::new(inner.identity.session, AudioWindow::SAMPLE_RATE) {
                Ok(filter) => resampler = Some(filter),
                Err(_) => {
                    inner.fail(job.epoch, ResolverError::InvalidData);
                    continue;
                }
            }
        }
        if Instant::now()
            .checked_duration_since(job.audio.stamp.received)
            .is_none_or(|age| age > MAX_AUDIO_AGE)
        {
            inner.fail(job.epoch, ResolverError::Stale);
            inner.processed.fetch_add(1, Ordering::Release);
            continue;
        }
        let result = resampler
            .as_mut()
            .ok_or(ResolverError::InvalidData)
            .and_then(|filter| {
                filter.process(&job.audio).map_err(|error| match error {
                    ResampleError::Discontinuity => ResolverError::AudioDiscontinuity,
                    _ => ResolverError::InvalidData,
                })
            })
            .and_then(|block| {
                // Tiny input can produce no decimated frame; its FIR state is retained.
                if block.frames == 0 {
                    return Ok(None);
                }
                let resets = windows.reset_count();
                let buffered = windows.buffered_frames();
                let result = windows.push(&block, Instant::now());
                if buffered > 0 && resets != windows.reset_count() {
                    return Err(ResolverError::AudioDiscontinuity);
                }
                result
            });
        match result {
            Ok(window) => {
                let state = lock(&inner.state);
                if state.epoch != job.epoch || state.stopped {
                    inner.dropped.fetch_add(1, Ordering::Relaxed);
                } else if let Some(window) = window {
                    let error = inner.output.borrow().error;
                    inner.output.send_replace(Arc::new(AudioAssembly {
                        epoch: job.epoch,
                        window: Some(window),
                        error,
                    }));
                }
            }
            Err(error) => inner.fail(job.epoch, error),
        }
        inner.processed.fetch_add(1, Ordering::Release);
    }
}

#[cfg(test)]
#[path = "audio_worker_tests.rs"]
mod tests;
