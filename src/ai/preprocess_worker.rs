//! One bounded CPU worker per active recognizer. No per-block tasks or network I/O.
use super::input_level::{InputLevelPolicy, boost_soft_input};
use super::speech::{
    AUDIO_QUEUE_BLOCKS, AudioIssue, AudioStage, Cancellation, SpeechError, SpeechQueue,
    StreamIdentity,
};
use crate::audio::{
    capture::{MAX_AUDIO_AGE, SAMPLE_RATE},
    preprocess::NormalizedAudio,
    resample::{ResampledAudio, StreamingResampler},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use tokio::sync::mpsc as async_mpsc;

/// Energy endpointing, not a claim of semantic speech classification. Quiet blocks
/// end a turn after 400 ms; continuous audio is capped at 4 seconds per turn.
pub const QUIET_RMS: f64 = 0.003;
pub const END_SILENCE_FRAMES: u64 = SAMPLE_RATE as u64 * 400 / 1000;
pub const MAX_TURN_FRAMES: u64 = SAMPLE_RATE as u64 * 4;
pub const MIN_TURN_FRAMES: u64 = SAMPLE_RATE as u64 / 10;
// In the last existing 400 ms, an observed 100 ms low-energy gap may end
// a turn before the hard cap. This never waits beyond the existing cap and
// does not claim the gap is a semantic word/sentence boundary.
const PREFERRED_PAUSE_START_FRAMES: u64 = MAX_TURN_FRAMES - END_SILENCE_FRAMES;
const PREFERRED_PAUSE_FRAMES: u64 = MIN_TURN_FRAMES;
const WORKER_POLL: Duration = Duration::from_millis(10);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EndpointPolicy {
    FixedLimit,
    PreferPauseNearLimit,
}

pub struct PreparedBlock {
    pub audio: ResampledAudio,
    pub active: bool,
    pub begin: bool,
    pub end: bool,
    pub boosted: bool,
}
struct Endpointing {
    policy: EndpointPolicy,
    active: bool,
    total: u64,
    quiet: u64,
}
impl Endpointing {
    fn new(policy: EndpointPolicy) -> Self {
        Self {
            policy,
            active: false,
            total: 0,
            quiet: 0,
        }
    }
    fn process(&mut self, input: &NormalizedAudio) -> (bool, bool, bool) {
        let rms = (input.samples[..input.frames]
            .iter()
            .map(|sample| f64::from(*sample).powi(2))
            .sum::<f64>()
            / input.frames as f64)
            .sqrt();
        let begin = !self.active && rms >= QUIET_RMS;
        self.active |= begin;
        let active = self.active;
        if active {
            self.total += input.frames as u64;
            self.quiet = if rms < QUIET_RMS {
                self.quiet + input.frames as u64
            } else {
                0
            };
        }
        let preferred_pause = self.policy == EndpointPolicy::PreferPauseNearLimit
            && self.total >= PREFERRED_PAUSE_START_FRAMES
            && self.quiet >= PREFERRED_PAUSE_FRAMES;
        let end = active
            && self.total >= MIN_TURN_FRAMES
            && (self.quiet >= END_SILENCE_FRAMES
                || self.total >= MAX_TURN_FRAMES
                || preferred_pause);
        if end {
            self.active = false;
            self.total = 0;
            self.quiet = 0;
        }
        (active, begin, end)
    }
}
pub struct PreprocessWorker {
    input: Option<mpsc::SyncSender<NormalizedAudio>>,
    pub output: async_mpsc::Receiver<Result<PreparedBlock, SpeechError>>,
    worker: Option<thread::JoinHandle<()>>,
    stopped: Arc<AtomicBool>,
}
impl PreprocessWorker {
    pub fn start(
        identity: StreamIdentity,
        rate: u32,
        cancel: Cancellation,
        level: InputLevelPolicy,
        endpoints: EndpointPolicy,
    ) -> Result<Self, SpeechError> {
        let mut resampler = StreamingResampler::new(identity.session, rate)
            .map_err(|_| SpeechError::InvalidData)?;
        let (sender, receiver) = mpsc::sync_channel::<NormalizedAudio>(AUDIO_QUEUE_BLOCKS);
        let (output, consumer) = async_mpsc::channel(AUDIO_QUEUE_BLOCKS);
        let worker_cancel = cancel.clone();
        let stopped = Arc::new(AtomicBool::new(false));
        let worker_stopped = Arc::clone(&stopped);
        let worker = thread::Builder::new()
            .name("stt-preprocess".into())
            .spawn(move || {
                let mut vad = Endpointing::new(endpoints);
                while worker_cancel.reason().is_none() && !worker_stopped.load(Ordering::Acquire) {
                    let mut input = match receiver.recv_timeout(WORKER_POLL) {
                        Ok(input) => input,
                        Err(mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    };
                    let result = if Instant::now()
                        .checked_duration_since(input.stamp.received)
                        .is_none_or(|age| age > MAX_AUDIO_AGE)
                    {
                        Err(SpeechError::audio_failure(
                            AudioStage::Preprocess,
                            AudioIssue::Age,
                        ))
                    } else {
                        (match level {
                            InputLevelPolicy::Original => Ok(false),
                            InputLevelPolicy::BoundedGain => boost_soft_input(&mut input),
                        })
                        .and_then(|boosted| {
                            resampler
                                .process(&input)
                                .map_err(|_| {
                                    SpeechError::audio_failure(
                                        AudioStage::Preprocess,
                                        AudioIssue::ResamplerRejected,
                                    )
                                })
                                .map(|audio| {
                                    let (active, begin, end) = vad.process(&input);
                                    PreparedBlock {
                                        audio,
                                        active,
                                        begin,
                                        end,
                                        boosted,
                                    }
                                })
                        })
                    };
                    let failed = result.is_err();
                    if let Err(error) = &result {
                        worker_cancel.fail(error.clone());
                    }
                    if let Err(error) = output.try_send(result) {
                        worker_cancel.fail(match error {
                            async_mpsc::error::TrySendError::Full(_) => SpeechError::queue_full(
                                SpeechQueue::PreprocessOutput,
                                AUDIO_QUEUE_BLOCKS,
                            ),
                            async_mpsc::error::TrySendError::Closed(_) => SpeechError::Cancelled,
                        });
                        break;
                    }
                    if failed {
                        break;
                    }
                }
            })
            .map_err(|_| SpeechError::WorkerStart)?;
        Ok(Self {
            input: Some(sender),
            output: consumer,
            worker: Some(worker),
            stopped,
        })
    }
    pub fn push(&self, audio: NormalizedAudio) -> Result<(), SpeechError> {
        self.input
            .as_ref()
            .ok_or(SpeechError::Cancelled)?
            .try_send(audio)
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => {
                    SpeechError::queue_full(SpeechQueue::PreprocessInput, AUDIO_QUEUE_BLOCKS)
                }
                mpsc::TrySendError::Disconnected(_) => SpeechError::audio_failure(
                    AudioStage::Preprocess,
                    AudioIssue::WorkerDisconnected,
                ),
            })
    }
}
impl Drop for PreprocessWorker {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        self.input.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::capture::{AudioStamp, BLOCK_FRAMES, SessionId};
    fn block(level: f32) -> NormalizedAudio {
        NormalizedAudio {
            stamp: AudioStamp {
                session: SessionId(1),
                sequence: 1,
                frame_position: Some(0),
                qpc_100ns: None,
                received: Instant::now(),
                discontinuity: false,
            },
            frames: BLOCK_FRAMES,
            samples: [level; BLOCK_FRAMES],
        }
    }
    #[test]
    fn phase11_pause_boundary_uses_existing_quiet_gap_before_cap() {
        let mut vad = Endpointing::new(EndpointPolicy::PreferPauseNearLimit);
        for index in 0..180 {
            assert_eq!(vad.process(&block(0.1)), (true, index == 0, false));
        }
        for index in 0..5 {
            assert_eq!(vad.process(&block(0.0)), (true, false, index == 4));
        }
        assert_eq!(vad.process(&block(0.1)), (true, true, false));
    }

    #[test]
    fn silence_is_not_a_turn_and_energy_has_bounded_endpoints() {
        let mut vad = Endpointing::new(EndpointPolicy::FixedLimit);
        assert_eq!(vad.process(&block(0.0)), (false, false, false));
        assert_eq!(vad.process(&block(0.1)), (true, true, false));
        for _ in 0..19 {
            assert_eq!(vad.process(&block(0.0)), (true, false, false));
        }
        assert_eq!(vad.process(&block(0.0)), (true, false, true));
        assert_eq!(vad.process(&block(0.0)), (false, false, false));
        for index in 0..200 {
            let (_, begin, end) = vad.process(&block(0.1));
            assert_eq!(begin, index == 0);
            assert_eq!(end, index == 199);
        }
    }

    #[test]
    fn phase11_pause_boundary_keeps_quiet_threshold_and_hard_cap_without_waiting() {
        for policy in [
            EndpointPolicy::FixedLimit,
            EndpointPolicy::PreferPauseNearLimit,
        ] {
            let mut vad = Endpointing::new(policy);
            for _ in 0..250 {
                assert_eq!(vad.process(&block(0.0)), (false, false, false));
            }
            for index in 0..100 {
                assert_eq!(vad.process(&block(0.1)), (true, index == 0, false));
            }
            for index in 0..20 {
                assert_eq!(vad.process(&block(0.0)), (true, false, index == 19));
            }
            // Short energy dips near the cap must not qualify as the preferred gap.
            for index in 0..200 {
                let level = if (190..192).contains(&index) {
                    0.0
                } else {
                    0.1
                };
                assert_eq!(vad.process(&block(level)), (true, index == 0, index == 199));
            }
            // A hard endpoint resets its counters even if the next block is voiced.
            for index in 0..200 {
                assert_eq!(vad.process(&block(0.1)), (true, index == 0, index == 199));
            }
        }
    }

    #[tokio::test]
    async fn phase11_pause_boundary_worker_preserves_every_sample_and_capture_stamp() {
        let identity = StreamIdentity {
            session: SessionId(1),
            generation: 1,
        };
        for policy in [
            EndpointPolicy::FixedLimit,
            EndpointPolicy::PreferPauseNearLimit,
        ] {
            let (session, _task) = super::super::speech::SpeechSession::new(
                identity,
                crate::domain::SourceLanguage::English,
                super::super::speech::SpeechCapabilities {
                    streaming: true,
                    partials: true,
                    cancellation: true,
                    supported_languages: [true; 5],
                    constrained_auto_detection: true,
                },
            )
            .unwrap();
            let cancel = session.cancellation();
            let mut worker = PreprocessWorker::start(
                identity,
                24000,
                cancel.clone(),
                InputLevelPolicy::Original,
                policy,
            )
            .unwrap();
            let mut expected_resampler = StreamingResampler::new(identity.session, 24000).unwrap();
            let mut endpoints = Vec::new();
            for sequence in 0..220 {
                let level = if (180..185).contains(&sequence) || sequence >= 200 {
                    0.0
                } else {
                    0.1
                };
                let mut input = block(level);
                input.stamp.sequence = sequence;
                input.stamp.frame_position = Some(sequence * BLOCK_FRAMES as u64);
                let received = input.stamp.received;
                let expected = expected_resampler.process(&input).unwrap();
                worker.push(input).unwrap();
                let actual = tokio::time::timeout(Duration::from_secs(2), worker.output.recv())
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap();
                assert_eq!(actual.audio.stamp.sequence, sequence);
                assert_eq!(
                    actual.audio.stamp.frame_position,
                    Some(sequence * BLOCK_FRAMES as u64)
                );
                assert_eq!(actual.audio.stamp.received, received);
                assert_eq!(actual.audio.frames, expected.frames);
                assert_eq!(
                    actual.audio.samples[..actual.audio.frames],
                    expected.samples[..expected.frames]
                );
                if actual.end {
                    endpoints.push(sequence);
                }
            }
            assert_eq!(
                endpoints,
                if policy == EndpointPolicy::PreferPauseNearLimit {
                    vec![184, 219]
                } else {
                    vec![199]
                }
            );
            cancel.cancel();
            drop(worker);
        }
    }

    #[tokio::test]
    async fn phase11_soft_input_worker_amplifies_before_endpointing_at_both_rates() {
        let identity = StreamIdentity {
            session: SessionId(1),
            generation: 1,
        };
        for (rate, level) in [16000, 24000].into_iter().flat_map(|rate| {
            [InputLevelPolicy::Original, InputLevelPolicy::BoundedGain].map(|level| (rate, level))
        }) {
            let (session, _task) = super::super::speech::SpeechSession::new(
                identity,
                crate::domain::SourceLanguage::English,
                super::super::speech::SpeechCapabilities {
                    streaming: true,
                    partials: true,
                    cancellation: true,
                    supported_languages: [true; 5],
                    constrained_auto_detection: true,
                },
            )
            .unwrap();
            let cancel = session.cancellation();
            let mut worker = PreprocessWorker::start(
                identity,
                rate,
                cancel.clone(),
                level,
                EndpointPolicy::FixedLimit,
            )
            .unwrap();
            let mut input = block(0.0);
            input.stamp.sequence = 0;
            for (index, sample) in input.samples.iter_mut().enumerate() {
                *sample = 0.0005
                    * (index as f32 * std::f32::consts::TAU * 500.0 / SAMPLE_RATE as f32).sin();
            }
            let expected_original = StreamingResampler::new(identity.session, rate)
                .unwrap()
                .process(&input)
                .unwrap();
            worker.push(input).unwrap();
            let output = tokio::time::timeout(Duration::from_secs(2), worker.output.recv())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let boosted = matches!(level, InputLevelPolicy::BoundedGain);
            assert_eq!(
                (output.active, output.begin, output.boosted),
                (boosted, boosted, boosted)
            );
            assert_eq!(output.audio.sample_rate, rate);
            if boosted {
                assert!(
                    output.audio.samples[..output.audio.frames]
                        .iter()
                        .any(|sample| sample.unsigned_abs() > 160)
                );
            } else {
                assert_eq!(output.audio.frames, expected_original.frames);
                assert_eq!(
                    output.audio.samples[..output.audio.frames],
                    expected_original.samples[..expected_original.frames]
                );
            }
            assert_eq!(output.audio.stamp.sequence, 0);
            assert_eq!(output.audio.stamp.frame_position, Some(0));
            cancel.cancel();
            drop(worker);
        }
    }

    #[tokio::test]
    async fn phase11_preprocess_reports_age_and_resampler_rejection_without_losing_cause() {
        let identity = StreamIdentity {
            session: SessionId(1),
            generation: 1,
        };
        for (issue, level) in [AudioIssue::Age, AudioIssue::ResamplerRejected]
            .into_iter()
            .flat_map(|issue| {
                [InputLevelPolicy::Original, InputLevelPolicy::BoundedGain]
                    .map(|level| (issue, level))
            })
        {
            let (session, _task) = super::super::speech::SpeechSession::new(
                identity,
                crate::domain::SourceLanguage::English,
                super::super::speech::SpeechCapabilities {
                    streaming: true,
                    partials: true,
                    cancellation: true,
                    supported_languages: [true; 5],
                    constrained_auto_detection: true,
                },
            )
            .unwrap();
            let cancel = session.cancellation();
            let mut worker = PreprocessWorker::start(
                identity,
                16000,
                cancel.clone(),
                level,
                EndpointPolicy::FixedLimit,
            )
            .unwrap();
            let mut input = block(0.1);
            if issue == AudioIssue::Age {
                input.stamp.received -= MAX_AUDIO_AGE + Duration::from_millis(1);
            } else {
                input.stamp.session = SessionId(2);
            }
            worker.push(input).unwrap();
            let error = tokio::time::timeout(Duration::from_secs(2), worker.output.recv())
                .await
                .unwrap()
                .unwrap()
                .err()
                .unwrap();
            let expected = SpeechError::audio_failure(AudioStage::Preprocess, issue);
            assert_eq!(error, expected);
            assert_eq!(cancel.reason(), Some(expected));
            assert_eq!(error.root_cause(), &SpeechError::AudioDiscontinuity);
            drop(worker); // joins the same worker; no detached task survives.
        }
    }
}
