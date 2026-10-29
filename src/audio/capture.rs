//! Bounded, session-owned capture pipeline. The producer never waits on consumers.

use super::preprocess::{NormalizedAudio, normalize};
use crossbeam_queue::ArrayQueue;
use std::{
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tokio::sync::watch;

pub const SAMPLE_RATE: u32 = 48_000;
pub const CHANNELS: usize = 2;
/// At most 20 ms per block, 240 ms per ring; stale blocks are rejected at both consumers.
pub const BLOCK_FRAMES: usize = 960;
pub const RING_BLOCKS: usize = 12;
pub const MAX_AUDIO_AGE: Duration = Duration::from_millis(250);
const PREPROCESS_POLL: Duration = Duration::from_millis(5);
static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CaptureError {
    #[error("請選擇有效的獨立程序音訊來源；無法擷取系統音效、跨程序來源或本程式。")]
    InvalidSource,
    #[error("選取的程序已結束，音訊擷取已停止。")]
    SourceExited,
    #[error("音訊裝置或服務已中斷，擷取已停止；請重新選取來源。")]
    Disconnected,
    #[error("Windows 音訊擷取失敗（{operation}，HRESULT {code:#010x}）。")]
    Native { operation: &'static str, code: i32 },
    #[error("音訊啟動逾時；請稍後重試。")]
    ActivationTimeout,
    #[error("上一個 Windows 音訊啟動要求尚未完成；請稍後重試。")]
    ActivationPending,
    #[error("無法建立音訊工作執行緒。")]
    WorkerStart,
    #[error("無法啟動音訊背景程序。")]
    WorkerProcessStart,
    #[error("音訊背景程序通訊資料無效，擷取已停止。")]
    WorkerProtocol,
    #[error("音訊背景程序已結束，擷取已停止。")]
    WorkerExited,
    #[error("音訊工作執行緒異常結束。")]
    WorkerPanicked,
    #[error("音訊資料或順序無效（{0}）。")]
    InvalidData(&'static str),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptureState {
    Starting,
    Running,
    Stopped(Result<(), CaptureError>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionId(pub u64);
impl SessionId {
    /// Live and Prepared runs share one checked namespace; no second counter can
    /// alias captured work with a separately prepared document.
    pub(crate) fn allocate() -> Result<Self, CaptureError> {
        NEXT_SESSION
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map(Self)
            .map_err(|_| CaptureError::InvalidData("session_id"))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct AudioStamp {
    pub session: SessionId,
    pub sequence: u64,
    /// Stream-relative frame position; absent when the native timestamp is unreliable.
    pub frame_position: Option<u64>,
    /// WASAPI QPC timestamp in 100 ns units, absent when Windows flags a timestamp error.
    pub qpc_100ns: Option<u64>,
    /// Local monotonic capture estimate, accounting for native buffer age when available.
    pub received: Instant,
    pub discontinuity: bool,
}

pub(crate) struct PcmBlock {
    pub stamp: AudioStamp,
    pub frames: usize,
    pub samples: [i16; BLOCK_FRAMES * CHANNELS],
}

#[derive(Default)]
struct Counters {
    captured: AtomicU64,
    dropped: AtomicU64,
    stale: AtomicU64,
    raw_high_water: AtomicUsize,
    normalized_high_water: AtomicUsize,
    timings: Option<Box<CaptureTimings>>,
}

#[derive(Default)]
struct CaptureTimings {
    sink_call: crate::performance::DurationHistogram,
    delivered_age: crate::performance::DurationHistogram,
}

#[derive(Clone, Copy, Debug)]
pub struct CaptureTimingMetrics {
    /// Sink entry to return, including validation/copy/enqueue. Excludes native
    /// engine/IPC/normalization upstream and the final histogram recording cost.
    pub sink_call: crate::performance::DurationDistribution,
    /// Native capture estimate to successful consumer delivery; excludes stale
    /// or invalid discarded blocks. This is not STT or end-to-end UI latency.
    pub delivered_age: crate::performance::DurationDistribution,
}

#[derive(Clone, Copy, Debug)]
pub struct CaptureMetrics {
    pub workers_running: usize,
    pub captured_frames: u64,
    pub dropped_frames: u64,
    pub stale_frames: u64,
    pub raw_queue_depth: usize,
    pub normalized_queue_depth: usize,
    /// Producer-observed lower bounds on true peaks. Concurrent dequeue can
    /// hide brief peaks; eviction always records the actual ring capacity.
    pub raw_queue_observed_high_water: usize,
    pub normalized_queue_observed_high_water: usize,
    /// Enabled at Session creation by DEBUG/TRACE diagnostics; None is unmeasured.
    pub timings: Option<CaptureTimingMetrics>,
}

fn observe_queue_peak(counter: &AtomicUsize, depth: usize, evicted: bool) {
    counter.fetch_max(
        if evicted { RING_BLOCKS } else { depth.max(1) },
        Ordering::Relaxed,
    );
}

struct Shared {
    resource_probe: OnceLock<Arc<dyn crate::performance::resources::ProcessProbe>>,
    raw: ArrayQueue<PcmBlock>,
    normalized: ArrayQueue<NormalizedAudio>,
    cancelled: AtomicBool,
    finished: AtomicBool,
    counters: Counters,
}

/// Terminal errors are sticky: a concurrently finishing worker must not erase a failure.
fn publish_stopped(state: &watch::Sender<CaptureState>, result: Result<(), CaptureError>) {
    state.send_modify(|current| {
        if !matches!(current, CaptureState::Stopped(Err(_))) {
            *current = CaptureState::Stopped(result);
        }
    });
}

struct WorkerExit {
    shared: Arc<Shared>,
    state: watch::Sender<CaptureState>,
    capture: bool,
}

impl Drop for WorkerExit {
    fn drop(&mut self) {
        if thread::panicking() {
            self.shared.cancelled.store(true, Ordering::Release);
            publish_stopped(&self.state, Err(CaptureError::WorkerPanicked));
        }
        if self.capture {
            self.shared.finished.store(true, Ordering::Release);
        }
    }
}

/// Driver boundary: start once, copy PCM packets, check cancellation frequently, then return.
/// No allocation, output, async work, or mutex is needed by `push_pcm`.
pub struct CaptureSink {
    shared: Arc<Shared>,
    state: watch::Sender<CaptureState>,
    session: SessionId,
    sequence: u64,
}

impl CaptureSink {
    pub(crate) fn measurements_enabled(&self) -> bool {
        self.shared.counters.timings.is_some()
    }
    pub(crate) fn attach_resource_probe(
        &self,
        probe: Arc<dyn crate::performance::resources::ProcessProbe>,
    ) {
        let _ = self.shared.resource_probe.set(probe);
    }
    pub(crate) fn session_id(&self) -> SessionId {
        self.session
    }

    pub(crate) fn push_normalized(&mut self, block: NormalizedAudio) -> Result<(), CaptureError> {
        let _timing = self
            .shared
            .counters
            .timings
            .as_ref()
            .map(|timings| timings.sink_call.start());
        if block.frames == 0
            || block.frames > BLOCK_FRAMES
            || block.stamp.session != self.session
            || block.stamp.sequence <= self.sequence
            || block.samples[..block.frames]
                .iter()
                .any(|sample| !sample.is_finite() || sample.abs() > 1.0)
        {
            return Err(CaptureError::WorkerProtocol);
        }
        self.sequence = block.stamp.sequence;
        self.shared
            .counters
            .captured
            .fetch_add(block.frames as u64, Ordering::Relaxed);
        if block.stamp.received.elapsed() > MAX_AUDIO_AGE {
            self.shared
                .counters
                .stale
                .fetch_add(block.frames as u64, Ordering::Relaxed);
        } else if !self.cancelled() {
            let old = self.shared.normalized.force_push(block);
            observe_queue_peak(
                &self.shared.counters.normalized_high_water,
                self.shared.normalized.len(),
                old.is_some(),
            );
            if let Some(old) = old {
                self.shared
                    .counters
                    .dropped
                    .fetch_add(old.frames as u64, Ordering::Relaxed);
            }
        }
        Ok(())
    }
    pub fn cancelled(&self) -> bool {
        self.shared.cancelled.load(Ordering::Acquire)
    }

    pub fn started(&self) {
        self.state.send_if_modified(|state| {
            if !self.cancelled() && *state == CaptureState::Starting {
                *state = CaptureState::Running;
                true
            } else {
                false
            }
        });
    }

    /// PCM is interleaved stereo signed 16-bit at SAMPLE_RATE. None denotes silence.
    /// Packet work is bounded by the driver's native buffer size; slices never escape this call.
    pub fn push_pcm(
        &mut self,
        pcm: Option<&[i16]>,
        frames: usize,
        frame_position: u64,
        qpc_100ns: Option<u64>,
        discontinuity: bool,
    ) -> Result<(), CaptureError> {
        self.push_pcm_received(
            pcm,
            frames,
            frame_position,
            qpc_100ns,
            discontinuity,
            Instant::now(),
        )
    }

    /// Native drivers may account for time already spent in the engine's buffer.
    pub fn push_pcm_received(
        &mut self,
        pcm: Option<&[i16]>,
        frames: usize,
        frame_position: u64,
        qpc_100ns: Option<u64>,
        discontinuity: bool,
        received: Instant,
    ) -> Result<(), CaptureError> {
        let _timing = self
            .shared
            .counters
            .timings
            .as_ref()
            .map(|timings| timings.sink_call.start());
        if pcm.is_some_and(|pcm| frames.checked_mul(CHANNELS) != Some(pcm.len())) {
            return Err(CaptureError::InvalidData("pcm_length"));
        }
        if received.elapsed() > MAX_AUDIO_AGE {
            self.shared
                .counters
                .captured
                .fetch_add(frames as u64, Ordering::Relaxed);
            self.shared
                .counters
                .stale
                .fetch_add(frames as u64, Ordering::Relaxed);
            // Preserve a sequence gap for the next accepted block.
            self.sequence = self
                .sequence
                .checked_add(1)
                .ok_or(CaptureError::InvalidData("sequence"))?;
            return Ok(());
        }
        for offset in (0..frames).step_by(BLOCK_FRAMES) {
            if self.cancelled() {
                break;
            }
            self.sequence = self
                .sequence
                .checked_add(1)
                .ok_or(CaptureError::InvalidData("sequence"))?;
            let count = (frames - offset).min(BLOCK_FRAMES);
            let mut block = PcmBlock {
                stamp: AudioStamp {
                    session: self.session,
                    sequence: self.sequence,
                    frame_position: if qpc_100ns.is_some() {
                        Some(
                            frame_position
                                .checked_add(offset as u64)
                                .ok_or(CaptureError::InvalidData("position"))?,
                        )
                    } else {
                        None
                    },
                    qpc_100ns: qpc_100ns.and_then(|qpc| {
                        qpc.checked_add(offset as u64 * 10_000_000 / u64::from(SAMPLE_RATE))
                    }),
                    received,
                    discontinuity: (discontinuity && offset == 0) || qpc_100ns.is_none(),
                },
                frames: count,
                samples: [0; BLOCK_FRAMES * CHANNELS],
            };
            if let Some(pcm) = pcm {
                block.samples[..count * CHANNELS]
                    .copy_from_slice(&pcm[offset * CHANNELS..(offset + count) * CHANNELS]);
            }
            self.shared
                .counters
                .captured
                .fetch_add(count as u64, Ordering::Relaxed);
            let old = self.shared.raw.force_push(block);
            observe_queue_peak(
                &self.shared.counters.raw_high_water,
                self.shared.raw.len(),
                old.is_some(),
            );
            if let Some(old) = old {
                self.shared
                    .counters
                    .dropped
                    .fetch_add(old.frames as u64, Ordering::Relaxed);
            }
        }
        Ok(())
    }
}

pub struct CaptureSession {
    id: SessionId,
    shared: Arc<Shared>,
    state: watch::Receiver<CaptureState>,
    capture: Option<JoinHandle<Result<(), CaptureError>>>,
    preprocess: Option<JoinHandle<()>>,
    last_sequence: u64,
}

impl CaptureSession {
    pub fn resource_probe(&self) -> Option<Arc<dyn crate::performance::resources::ProcessProbe>> {
        self.shared.resource_probe.get().cloned()
    }
    /// Exactly two workers with separate roles, independent of CPU count. No per-block tasks.
    pub fn start(
        driver: impl FnOnce(CaptureSink) -> Result<(), CaptureError> + Send + 'static,
    ) -> Result<Self, CaptureError> {
        Self::start_with_preprocessor(driver, preprocess_loop)
    }

    fn start_with_preprocessor(
        driver: impl FnOnce(CaptureSink) -> Result<(), CaptureError> + Send + 'static,
        processor: impl FnOnce(&Shared) + Send + 'static,
    ) -> Result<Self, CaptureError> {
        Self::start_pipeline(driver, Some(processor))
    }

    /// A normalized external source has one receiver worker; normalization runs at its source.
    pub(crate) fn start_normalized(
        driver: impl FnOnce(CaptureSink) -> Result<(), CaptureError> + Send + 'static,
    ) -> Result<Self, CaptureError> {
        Self::start_pipeline(driver, None::<fn(&Shared)>)
    }

    fn start_pipeline(
        driver: impl FnOnce(CaptureSink) -> Result<(), CaptureError> + Send + 'static,
        processor: Option<impl FnOnce(&Shared) + Send + 'static>,
    ) -> Result<Self, CaptureError> {
        Self::start_pipeline_measured(driver, processor, tracing::enabled!(tracing::Level::DEBUG))
    }

    fn start_pipeline_measured(
        driver: impl FnOnce(CaptureSink) -> Result<(), CaptureError> + Send + 'static,
        processor: Option<impl FnOnce(&Shared) + Send + 'static>,
        measure: bool,
    ) -> Result<Self, CaptureError> {
        let id = SessionId::allocate()?;
        let shared = Arc::new(Shared {
            resource_probe: OnceLock::new(),
            raw: ArrayQueue::new(RING_BLOCKS),
            normalized: ArrayQueue::new(RING_BLOCKS),
            cancelled: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            counters: Counters {
                timings: measure.then(|| Box::new(CaptureTimings::default())),
                ..Counters::default()
            },
        });
        let (state_sender, state) = watch::channel(CaptureState::Starting);
        let preprocess = if let Some(processor) = processor {
            let preprocess_shared = Arc::clone(&shared);
            let preprocess_exit = WorkerExit {
                shared: Arc::clone(&shared),
                state: state_sender.clone(),
                capture: false,
            };
            Some(
                thread::Builder::new()
                    .name("audio-preprocess".into())
                    .spawn(move || {
                        let _exit = preprocess_exit;
                        processor(&preprocess_shared);
                    })
                    .map_err(|_| CaptureError::WorkerStart)?,
            )
        } else {
            None
        };
        let worker_shared = Arc::clone(&shared);
        let capture = thread::Builder::new()
            .name("process-capture".into())
            .spawn(move || {
                let _exit = WorkerExit {
                    shared: Arc::clone(&worker_shared),
                    state: state_sender.clone(),
                    capture: true,
                };
                let result = driver(CaptureSink {
                    shared: worker_shared,
                    state: state_sender.clone(),
                    session: id,
                    sequence: 0,
                });
                publish_stopped(&state_sender, result.clone());
                result
            });
        let capture = match capture {
            Ok(worker) => worker,
            Err(_) => {
                shared.cancelled.store(true, Ordering::Release);
                if let Some(worker) = preprocess {
                    let _ = worker.join();
                }
                return Err(CaptureError::WorkerStart);
            }
        };
        Ok(Self {
            id,
            shared,
            state,
            capture: Some(capture),
            preprocess,
            last_sequence: 0,
        })
    }

    pub fn id(&self) -> SessionId {
        self.id
    }
    pub fn state(&self) -> CaptureState {
        self.state.borrow().clone()
    }
    pub async fn changed(&mut self) -> Result<CaptureState, CaptureError> {
        self.state
            .changed()
            .await
            .map_err(|_| CaptureError::WorkerPanicked)?;
        Ok(self.state.borrow_and_update().clone())
    }

    /// A stopped run cannot deliver queued audio. A caller must retain/check SessionId downstream.
    pub fn try_next(&mut self) -> Option<NormalizedAudio> {
        if self.shared.cancelled.load(Ordering::Acquire)
            || self.shared.finished.load(Ordering::Acquire)
        {
            return None;
        }
        while let Some(mut block) = self.shared.normalized.pop() {
            if block.stamp.received.elapsed() > MAX_AUDIO_AGE {
                self.shared
                    .counters
                    .stale
                    .fetch_add(block.frames as u64, Ordering::Relaxed);
                continue;
            }
            if block.stamp.session != self.id || block.stamp.sequence <= self.last_sequence {
                continue;
            }
            block.stamp.discontinuity |= block.stamp.sequence != self.last_sequence + 1;
            self.last_sequence = block.stamp.sequence;
            if let Some(timings) = &self.shared.counters.timings {
                timings.delivered_age.record_since(block.stamp.received);
            }
            return Some(block);
        }
        None
    }

    pub fn metrics(&self) -> CaptureMetrics {
        CaptureMetrics {
            workers_running: usize::from(
                self.capture
                    .as_ref()
                    .is_some_and(|worker| !worker.is_finished()),
            ) + usize::from(
                self.preprocess
                    .as_ref()
                    .is_some_and(|worker| !worker.is_finished()),
            ),
            captured_frames: self.shared.counters.captured.load(Ordering::Relaxed),
            dropped_frames: self.shared.counters.dropped.load(Ordering::Relaxed),
            stale_frames: self.shared.counters.stale.load(Ordering::Relaxed),
            raw_queue_depth: self.shared.raw.len(),
            normalized_queue_depth: self.shared.normalized.len(),
            raw_queue_observed_high_water: self
                .shared
                .counters
                .raw_high_water
                .load(Ordering::Relaxed),
            normalized_queue_observed_high_water: self
                .shared
                .counters
                .normalized_high_water
                .load(Ordering::Relaxed),
            timings: self
                .shared
                .counters
                .timings
                .as_ref()
                .map(|timings| CaptureTimingMetrics {
                    sink_call: timings.sink_call.snapshot(),
                    delivered_age: timings.delivered_age.snapshot(),
                }),
        }
    }

    pub fn stop(&mut self) -> Result<(), CaptureError> {
        self.shared.cancelled.store(true, Ordering::Release);
        // Always join both workers, even if one panics or reports an error.
        let captured = self
            .capture
            .take()
            .map(|worker| worker.join().unwrap_or(Err(CaptureError::WorkerPanicked)))
            .unwrap_or(Ok(()));
        let processed = self
            .preprocess
            .take()
            .map(|worker| worker.join().map_err(|_| CaptureError::WorkerPanicked))
            .unwrap_or(Ok(()));
        while self.shared.raw.pop().is_some() {}
        while self.shared.normalized.pop().is_some() {}
        // Match the error already observed by consumers, even when both workers failed.
        // Repeated stop calls preserve that error instead of reporting a false success.
        match self.state() {
            CaptureState::Stopped(Err(error)) => Err(error),
            _ => captured.and(processed),
        }
    }
}

impl Drop for CaptureSession {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn preprocess_loop(shared: &Shared) {
    let mut previous: Option<(u64, Option<u64>)> = None;
    while !shared.cancelled.load(Ordering::Acquire) && !shared.finished.load(Ordering::Acquire) {
        let Some(mut raw) = shared.raw.pop() else {
            thread::park_timeout(PREPROCESS_POLL);
            continue;
        };
        if raw.stamp.received.elapsed() > MAX_AUDIO_AGE {
            shared
                .counters
                .stale
                .fetch_add(raw.frames as u64, Ordering::Relaxed);
            continue;
        }
        raw.stamp.discontinuity |= previous.is_none_or(|(sequence, end)| {
            raw.stamp.sequence != sequence + 1
                || raw.stamp.frame_position.is_none()
                || raw.stamp.frame_position != end
        });
        previous = Some((
            raw.stamp.sequence,
            raw.stamp
                .frame_position
                .and_then(|position| position.checked_add(raw.frames as u64)),
        ));
        let block = normalize(raw);
        let old = shared.normalized.force_push(block);
        observe_queue_peak(
            &shared.counters.normalized_high_water,
            shared.normalized.len(),
            old.is_some(),
        );
        if let Some(old) = old {
            shared
                .counters
                .dropped
                .fetch_add(old.frames as u64, Ordering::Relaxed);
        }
    }
    while shared.raw.pop().is_some() {}
    while shared.normalized.pop().is_some() {}
}

#[cfg(test)]
#[path = "capture_metrics_tests.rs"]
mod metrics_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn wait_for(mut condition: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !condition() {
            assert!(Instant::now() < deadline, "capture test timed out");
            thread::sleep(Duration::from_millis(2));
        }
    }

    fn sink() -> CaptureSink {
        let (state, _) = watch::channel(CaptureState::Starting);
        CaptureSink {
            shared: Arc::new(Shared {
                resource_probe: OnceLock::new(),
                raw: ArrayQueue::new(RING_BLOCKS),
                normalized: ArrayQueue::new(RING_BLOCKS),
                cancelled: AtomicBool::new(false),
                finished: AtomicBool::new(false),
                counters: Counters::default(),
            }),
            state,
            session: SessionId(100),
            sequence: 0,
        }
    }

    fn normalized_packet(session: SessionId, sequence: u64) -> NormalizedAudio {
        NormalizedAudio {
            stamp: AudioStamp {
                session,
                sequence,
                frame_position: Some(0),
                qpc_100ns: Some(0),
                received: Instant::now(),
                discontinuity: false,
            },
            frames: 1,
            samples: [0.25; BLOCK_FRAMES],
        }
    }

    #[test]
    fn normalized_input_rejects_foreign_old_and_malformed_results() {
        let mut sink = sink();
        sink.push_normalized(normalized_packet(sink.session, 10))
            .unwrap();
        assert_eq!(
            sink.push_normalized(normalized_packet(SessionId(101), 11)),
            Err(CaptureError::WorkerProtocol)
        );
        assert_eq!(
            sink.push_normalized(normalized_packet(sink.session, 9)),
            Err(CaptureError::WorkerProtocol)
        );
        let mut packet = normalized_packet(sink.session, 11);
        packet.frames = BLOCK_FRAMES + 1;
        assert_eq!(
            sink.push_normalized(packet),
            Err(CaptureError::WorkerProtocol)
        );
        let mut packet = normalized_packet(sink.session, 11);
        packet.samples[0] = f32::NAN;
        assert_eq!(
            sink.push_normalized(packet),
            Err(CaptureError::WorkerProtocol)
        );
        assert_eq!(sink.shared.normalized.len(), 1);
    }

    #[test]
    fn normalized_input_stays_bounded_and_discards_expired_audio() {
        let mut sink = sink();
        let mut packet = normalized_packet(sink.session, 1);
        packet.stamp.received = Instant::now() - MAX_AUDIO_AGE - Duration::from_millis(1);
        sink.push_normalized(packet).unwrap();
        assert!(sink.shared.normalized.is_empty());
        assert_eq!(sink.shared.counters.stale.load(Ordering::Relaxed), 1);
        for sequence in 2..=RING_BLOCKS as u64 + 4 {
            sink.push_normalized(normalized_packet(sink.session, sequence))
                .unwrap();
        }
        assert_eq!(sink.shared.normalized.len(), RING_BLOCKS);
        assert_eq!(sink.shared.counters.dropped.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn normalized_driver_uses_one_worker_and_stop_clears_output() {
        let mut session = CaptureSession::start_normalized(|mut sink| {
            sink.started();
            sink.push_normalized(normalized_packet(sink.session_id(), 1))?;
            while !sink.cancelled() {
                thread::park_timeout(Duration::from_millis(1));
            }
            Ok(())
        })
        .unwrap();
        assert!(session.preprocess.is_none());
        wait_for(|| session.metrics().normalized_queue_depth == 1);
        assert_eq!(session.try_next().unwrap().samples[0], 0.25);
        session.stop().unwrap();
        assert!(session.try_next().is_none());
        assert_eq!(session.metrics().normalized_queue_depth, 0);
    }

    #[test]
    fn pcm_is_split_normalized_and_timestamped_without_clipping() {
        let mut sink = sink();
        let frames = BLOCK_FRAMES * 2 + 1;
        let mut pcm = vec![i16::MAX; frames * CHANNELS];
        pcm[0] = i16::MIN;
        pcm[1] = i16::MIN;
        pcm[2] = i16::MAX;
        pcm[3] = i16::MIN;
        sink.push_pcm(Some(&pcm), frames, 48_000, Some(100), true)
            .unwrap();
        let first = normalize(sink.shared.raw.pop().unwrap());
        assert_eq!(first.samples[0], -1.0);
        assert_eq!(first.samples[1], -1.0 / 65536.0);
        assert_eq!(first.samples[2], 32767.0 / 32768.0);
        assert_eq!(first.stamp.sequence, 1);
        assert_eq!(first.start_us(), Some(1_000_000));
        assert_eq!(first.duration_us(), 20_000);
        assert!(first.stamp.discontinuity);
        let second = sink.shared.raw.pop().unwrap();
        assert_eq!(second.stamp.frame_position, Some(48_960));
        assert_eq!(second.stamp.qpc_100ns, Some(200_100));
        assert!(!second.stamp.discontinuity);
        assert_eq!(sink.shared.raw.pop().unwrap().frames, 1);
        assert!(sink.shared.raw.is_empty());
    }

    #[test]
    fn silent_packets_and_unknown_timestamps_are_preserved() {
        let mut sink = sink();
        sink.push_pcm(None, 480, 0, None, false).unwrap();
        let normalized = normalize(sink.shared.raw.pop().unwrap());
        assert!(normalized.samples.iter().all(|sample| *sample == 0.0));
        assert_eq!(normalized.stamp.qpc_100ns, None);
        assert_eq!(normalized.stamp.frame_position, None);
        assert!(normalized.stamp.discontinuity);
        assert!(sink.push_pcm(Some(&[1]), 1, 0, None, false).is_err());
    }

    #[test]
    fn saturated_raw_ring_evicts_oldest_without_growing() {
        let mut sink = sink();
        for position in 0..RING_BLOCKS + 5 {
            sink.push_pcm(None, 1, position as u64, None, false)
                .unwrap();
        }
        assert_eq!(sink.shared.raw.len(), RING_BLOCKS);
        assert_eq!(sink.shared.counters.dropped.load(Ordering::Relaxed), 5);
        assert_eq!(sink.shared.raw.pop().unwrap().stamp.sequence, 6);
    }

    #[test]
    fn stale_raw_audio_is_discarded_before_normalization() {
        let mut sink = sink();
        sink.push_pcm(None, 10, 0, None, false).unwrap();
        let mut old = sink.shared.raw.pop().unwrap();
        old.stamp.received = Instant::now() - MAX_AUDIO_AGE - Duration::from_millis(1);
        assert!(sink.shared.raw.push(old).is_ok());
        let shared = Arc::clone(&sink.shared);
        let worker = thread::spawn(move || preprocess_loop(&shared));
        wait_for(|| sink.shared.counters.stale.load(Ordering::Relaxed) == 10);
        sink.shared.cancelled.store(true, Ordering::Release);
        worker.join().unwrap();
        assert!(sink.shared.normalized.is_empty());
    }

    #[test]
    fn stale_native_packets_never_enter_the_ring() {
        let mut sink = sink();
        sink.push_pcm_received(
            None,
            480,
            0,
            Some(0),
            false,
            Instant::now() - MAX_AUDIO_AGE - Duration::from_millis(1),
        )
        .unwrap();
        assert!(sink.shared.raw.is_empty());
        assert_eq!(sink.shared.counters.stale.load(Ordering::Relaxed), 480);
        sink.push_pcm(None, 480, 480, Some(100_000), false).unwrap();
        assert_eq!(sink.shared.raw.pop().unwrap().stamp.sequence, 2);
    }

    #[test]
    fn slow_consumer_gets_bounded_recent_audio_and_gap_marker() {
        let (ready, received) = mpsc::sync_channel(1);
        let mut session = CaptureSession::start(move |mut sink| {
            sink.started();
            for position in 0..200 {
                sink.push_pcm(None, 1, position, None, false)?;
                thread::sleep(Duration::from_millis(1));
            }
            let _ = ready.try_send(());
            while !sink.cancelled() {
                thread::park_timeout(Duration::from_millis(2));
            }
            Ok(())
        })
        .unwrap();
        received.recv_timeout(Duration::from_secs(5)).unwrap();
        wait_for(|| session.shared.raw.is_empty());
        assert!(session.metrics().dropped_frames > 0);
        assert!(session.metrics().normalized_queue_depth <= RING_BLOCKS);
        let latest = session.try_next().unwrap();
        assert!(latest.stamp.sequence > 1);
        assert!(latest.stamp.discontinuity);
        assert_eq!(latest.stamp.session, session.id());
        // Reject aging data even when the consumer stopped polling altogether.
        let mut old = latest;
        old.stamp.sequence = 500;
        old.stamp.received = Instant::now() - MAX_AUDIO_AGE - Duration::from_millis(1);
        while session.try_next().is_some() {}
        assert!(session.shared.normalized.push(old).is_ok());
        assert!(session.try_next().is_none());
        session.stop().unwrap();
        assert_eq!(session.metrics().normalized_queue_depth, 0);
    }

    #[test]
    fn stop_drop_and_restart_isolate_sessions_and_join_workers() {
        let stopped = Arc::new(AtomicBool::new(false));
        let guard = Arc::clone(&stopped);
        let first = CaptureSession::start(move |mut sink| {
            sink.started();
            sink.push_pcm(None, 1, 0, None, false)?;
            while !sink.cancelled() {
                thread::park_timeout(Duration::from_millis(2));
            }
            guard.store(true, Ordering::Release);
            Ok(())
        })
        .unwrap();
        let id = first.id();
        drop(first);
        assert!(stopped.load(Ordering::Acquire));
        let mut second = CaptureSession::start(|mut sink| {
            sink.started();
            sink.push_pcm(None, 1, 0, None, false)?;
            while !sink.cancelled() {
                thread::park_timeout(Duration::from_millis(2));
            }
            Ok(())
        })
        .unwrap();
        assert_ne!(id, second.id());
        wait_for(|| second.metrics().normalized_queue_depth == 1);
        let block = second.try_next().unwrap();
        assert_eq!(block.stamp.session, second.id());
        assert_eq!(block.stamp.sequence, 1);
        let before = Instant::now();
        second.stop().unwrap();
        assert!(before.elapsed() < Duration::from_secs(1));
        second.stop().unwrap();
        assert!(second.try_next().is_none());
    }

    #[test]
    fn disconnect_and_panic_stop_pipeline_and_release_pending_audio() {
        for panic in [false, true] {
            let mut session = CaptureSession::start(move |mut sink| {
                sink.started();
                sink.push_pcm(None, 1, 0, None, false)?;
                if panic {
                    panic!("explicit mock capture panic");
                }
                Err(CaptureError::Disconnected)
            })
            .unwrap();
            wait_for(|| session.shared.finished.load(Ordering::Acquire));
            assert!(session.try_next().is_none());
            assert_eq!(
                session.stop(),
                Err(if panic {
                    CaptureError::WorkerPanicked
                } else {
                    CaptureError::Disconnected
                })
            );
            assert_eq!(session.metrics().raw_queue_depth, 0);
            assert_eq!(session.metrics().normalized_queue_depth, 0);
        }
    }

    #[test]
    fn saturated_producer_observes_stop_without_waiting_for_a_consumer() {
        let mut session = CaptureSession::start(|mut sink| {
            sink.started();
            let mut position = 0;
            while !sink.cancelled() {
                sink.push_pcm(None, BLOCK_FRAMES, position, Some(position), false)?;
                position += BLOCK_FRAMES as u64;
            }
            Ok(())
        })
        .unwrap();
        wait_for(|| session.metrics().dropped_frames > 0);
        let before = Instant::now();
        session.stop().unwrap();
        assert!(before.elapsed() < Duration::from_secs(1));
        assert!(session.try_next().is_none());
        assert_eq!(session.metrics().raw_queue_depth, 0);
        assert_eq!(session.metrics().normalized_queue_depth, 0);
    }

    #[test]
    fn frame_gaps_are_not_compressed_into_continuous_audio() {
        let mut session = CaptureSession::start(|mut sink| {
            sink.started();
            sink.push_pcm(None, 10, 0, Some(0), false)?;
            sink.push_pcm(None, 10, 500, Some(104_167), false)?;
            while !sink.cancelled() {
                thread::park_timeout(Duration::from_millis(2));
            }
            Ok(())
        })
        .unwrap();
        wait_for(|| session.metrics().normalized_queue_depth == 2);
        let first = session.try_next().unwrap();
        let second = session.try_next().unwrap();
        assert!(first.stamp.discontinuity);
        assert!(second.stamp.discontinuity);
        assert_eq!(second.stamp.frame_position, Some(500));
        session.stop().unwrap();
    }

    #[test]
    fn preprocessor_panic_cancels_capture_and_late_start_cannot_erase_error() {
        let driver_stopped = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&driver_stopped);
        let mut session = CaptureSession::start_with_preprocessor(
            move |sink| {
                while !sink.cancelled() {
                    thread::park_timeout(Duration::from_millis(2));
                }
                // Model a native activation completing after another worker has already failed.
                sink.started();
                stopped.store(true, Ordering::Release);
                Ok(())
            },
            |_| panic!("explicit mock preprocessing panic"),
        )
        .unwrap();
        wait_for(|| driver_stopped.load(Ordering::Acquire));
        assert_eq!(session.stop(), Err(CaptureError::WorkerPanicked));
        assert_eq!(
            session.state(),
            CaptureState::Stopped(Err(CaptureError::WorkerPanicked))
        );
        assert!(session.try_next().is_none());
        assert_eq!(session.metrics().raw_queue_depth, 0);
        assert_eq!(session.metrics().normalized_queue_depth, 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn capture_panic_publishes_terminal_state_to_async_consumer() {
        let mut session = CaptureSession::start(|sink| {
            sink.started();
            panic!("explicit mock capture panic");
        })
        .unwrap();
        let state = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let state = session.changed().await.unwrap();
                if matches!(state, CaptureState::Stopped(_)) {
                    break state;
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(
            state,
            CaptureState::Stopped(Err(CaptureError::WorkerPanicked))
        );
        assert_eq!(session.state(), state);
        assert_eq!(session.stop(), Err(CaptureError::WorkerPanicked));
    }

    #[test]
    fn stop_returns_the_published_error_when_a_second_worker_also_fails() {
        let mut session = CaptureSession::start_with_preprocessor(
            |sink| {
                wait_for(|| matches!(*sink.state.borrow(), CaptureState::Stopped(Err(_))));
                Err(CaptureError::Disconnected)
            },
            |_| panic!("explicit first worker failure"),
        )
        .unwrap();
        wait_for(|| session.shared.finished.load(Ordering::Acquire));
        assert_eq!(
            session.state(),
            CaptureState::Stopped(Err(CaptureError::WorkerPanicked))
        );
        assert_eq!(session.stop(), Err(CaptureError::WorkerPanicked));
        assert_eq!(session.stop(), Err(CaptureError::WorkerPanicked));
    }

    #[test]
    fn late_previous_session_and_out_of_order_blocks_cannot_enter_new_run() {
        let mut previous = CaptureSession::start(|mut sink| {
            sink.started();
            sink.push_pcm(None, 1, 0, Some(0), false)?;
            while !sink.cancelled() {
                thread::park_timeout(Duration::from_millis(2));
            }
            Ok(())
        })
        .unwrap();
        wait_for(|| previous.metrics().normalized_queue_depth == 1);
        let mut late = previous.try_next().unwrap();
        previous.stop().unwrap();
        // A downstream consumer still owns this result after the old run has been joined.
        late.stamp.sequence = 100;
        let mut session = CaptureSession::start(|sink| {
            sink.started();
            while !sink.cancelled() {
                thread::park_timeout(Duration::from_millis(2));
            }
            Ok(())
        })
        .unwrap();
        let mut producer = sink();
        // Distinct old identity with a much later sequence must not advance the new run.
        assert_ne!(late.stamp.session, session.id());
        assert!(session.shared.normalized.push(late).is_ok());
        producer.session = session.id();
        for sequence in [2, 1, 3] {
            producer.sequence = sequence - 1;
            producer
                .push_pcm(None, 1, sequence - 1, Some(sequence), false)
                .unwrap();
            assert!(
                session
                    .shared
                    .normalized
                    .push(normalize(producer.shared.raw.pop().unwrap()))
                    .is_ok()
            );
        }
        let first = session.try_next().unwrap();
        assert_eq!(first.stamp.session, session.id());
        assert_eq!(first.stamp.sequence, 2);
        assert!(first.stamp.discontinuity);
        assert_eq!(session.try_next().unwrap().stamp.sequence, 3);
        assert!(session.try_next().is_none());
        session.stop().unwrap();
    }
}
