//! Explicit synthetic pull decoders; no files, provider, keys or retained audio.
use super::*;
use crate::{
    audio::capture::SessionId,
    media::{
        decode_worker::decode_with_consumer,
        decoded::{DecodedAudioFormat, MediaDecoder},
    },
};
use std::{
    collections::VecDeque,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread::{self, ThreadId},
};
use tokio::sync::Semaphore;

fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(520),
        generation: 1,
    }
}
fn format(rate: u32) -> DecodedAudioFormat {
    DecodedAudioFormat::new(rate, 1).unwrap()
}
fn block(rate: u32, sequence: u64, timestamp: u64, frames: usize) -> DecodedAudioBlock {
    DecodedAudioBlock::new(
        identity(),
        sequence,
        timestamp,
        format(rate),
        vec![1000; frames],
    )
    .unwrap()
}
fn lane() -> Arc<Semaphore> {
    Arc::new(Semaphore::new(1))
}
struct SyntheticDecoder {
    format: DecodedAudioFormat,
    blocks: VecDeque<DecodedAudioBlock>,
    eof_error: Option<MediaError>,
    pulls: Arc<AtomicUsize>,
    owner: ThreadId,
    _affine: Rc<()>,
}
impl MediaDecoder for SyntheticDecoder {
    fn format(&self) -> DecodedAudioFormat {
        self.format
    }
    fn next(
        &mut self,
        cancel: &ResolverCancellation,
    ) -> Result<Option<DecodedAudioBlock>, MediaError> {
        assert_eq!(thread::current().id(), self.owner);
        if cancel.is_cancelled() {
            return Err(MediaError::Cancelled);
        }
        self.pulls.fetch_add(1, Ordering::SeqCst);
        if let Some(block) = self.blocks.pop_front() {
            Ok(Some(block))
        } else if let Some(error) = self.eof_error {
            Err(error)
        } else {
            Ok(None)
        }
    }
}
impl Drop for SyntheticDecoder {
    fn drop(&mut self) {
        assert_eq!(thread::current().id(), self.owner);
    }
}
fn factory(
    rate: u32,
    blocks: Vec<DecodedAudioBlock>,
    eof_error: Option<MediaError>,
    pulls: Arc<AtomicUsize>,
) -> impl FnOnce(&ResolverCancellation) -> Result<Box<dyn MediaDecoder>, MediaError> + Send {
    move |_| {
        Ok(Box::new(SyntheticDecoder {
            format: format(rate),
            blocks: blocks.into(),
            eof_error,
            pulls,
            owner: thread::current().id(),
            _affine: Rc::new(()),
        }))
    }
}

#[tokio::test]
async fn complete_worker_flushes_short_tail_only_after_real_eof() {
    let pulls = Arc::new(AtomicUsize::new(0));
    let observed_pulls = pulls.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed_calls = calls.clone();
    let main_thread = thread::current().id();
    let summary = decode_with_consumer(
        identity(),
        ResolverCancellation::default(),
        PreparedConsumer::new(
            identity(),
            PreparedSampleRate::Hz16000,
            move |block: &PreparedPcmBlock, _: &ResolverCancellation| {
                assert_ne!(thread::current().id(), main_thread);
                assert_eq!(observed_pulls.load(Ordering::SeqCst), 2);
                assert_eq!(block.identity(), identity());
                assert_eq!(block.sample_rate(), 16_000);
                assert_eq!(block.samples(), &[1000; 47]);
                assert_eq!(block.timestamp_us(), 1_000_000);
                assert!(!block.discontinuity());
                observed_calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        ),
        factory(16_000, vec![block(16_000, 1, 1_000_000, 47)], None, pulls),
        lane(),
    )
    .await
    .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(summary.identity(), identity());
    assert_eq!(summary.frames(), 47);
    assert_eq!(summary.blocks(), 1);
    assert_eq!(summary.last_end_us(), 1_002_937);
}

#[tokio::test]
async fn fractional_conversion_preserves_nonzero_media_anchor_and_all_tail_frames() {
    let frames = Arc::new(AtomicUsize::new(0));
    let observed = frames.clone();
    let summary = decode_with_consumer(
        identity(),
        ResolverCancellation::default(),
        PreparedConsumer::new(
            identity(),
            PreparedSampleRate::Hz24000,
            move |block: &PreparedPcmBlock, _: &ResolverCancellation| {
                assert!(block.samples().iter().all(|value| *value == 1000));
                assert_eq!(
                    block.timestamp_us(),
                    2_000_000 + observed.load(Ordering::SeqCst) as u64 * 1_000_000 / 24_000
                );
                assert!(block.frames() <= 480);
                observed.fetch_add(block.frames(), Ordering::SeqCst);
                Ok(())
            },
        ),
        factory(
            44_100,
            vec![
                block(44_100, 1, 2_000_000, 137),
                block(44_100, 2, 2_003_106, 1864),
            ],
            None,
            Arc::new(AtomicUsize::new(0)),
        ),
        lane(),
    )
    .await
    .unwrap();
    assert_eq!(summary.frames(), 2_001 * 24_000 / 44_100);
    assert_eq!(frames.load(Ordering::SeqCst) as u64, summary.frames());
    assert!(summary.last_end_us() <= 2_000_000 + 2_001 * 1_000_000 / 44_100);
}

#[tokio::test]
async fn native_time_gap_keeps_two_runs_separate_without_filling_audio() {
    let observed = Arc::new(Mutex::new(Vec::new()));
    let sink = observed.clone();
    let summary = decode_with_consumer(
        identity(),
        ResolverCancellation::default(),
        PreparedConsumer::new(
            identity(),
            PreparedSampleRate::Hz16000,
            move |block: &PreparedPcmBlock, _: &ResolverCancellation| {
                sink.lock().unwrap().push((
                    block.sequence(),
                    block.timestamp_us(),
                    block.frames(),
                    block.discontinuity(),
                ));
                Ok(())
            },
        ),
        factory(
            16_000,
            vec![
                block(16_000, 1, 1_000_000, 160),
                block(16_000, 2, 3_000_000, 160),
            ],
            None,
            Arc::new(AtomicUsize::new(0)),
        ),
        lane(),
    )
    .await
    .unwrap();
    assert_eq!(
        *observed.lock().unwrap(),
        vec![(1, 1_000_000, 160, false), (2, 3_000_000, 160, true)]
    );
    assert_eq!(summary.gaps(), 1);
    assert_eq!(summary.frames(), 320);
}

#[tokio::test]
async fn decoder_failure_never_flushes_held_short_pcm_as_success() {
    let calls = Arc::new(AtomicUsize::new(0));
    let sink = calls.clone();
    let cancel = ResolverCancellation::default();
    let workers = lane();
    let result = decode_with_consumer(
        identity(),
        cancel.clone(),
        PreparedConsumer::new(
            identity(),
            PreparedSampleRate::Hz16000,
            move |_: &PreparedPcmBlock, _: &ResolverCancellation| {
                sink.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        ),
        factory(
            16_000,
            vec![block(16_000, 1, 0, 47)],
            Some(MediaError::Unavailable),
            Arc::new(AtomicUsize::new(0)),
        ),
        workers.clone(),
    )
    .await;
    assert_eq!(result.err(), Some(MediaError::Unavailable));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(cancel.is_cancelled());
    assert_eq!(workers.available_permits(), 1);
}

#[tokio::test]
async fn eof_sink_error_is_typed_and_never_commits_partial_summary() {
    let cancel = ResolverCancellation::default();
    let workers = lane();
    let result = decode_with_consumer(
        identity(),
        cancel.clone(),
        PreparedConsumer::new(
            identity(),
            PreparedSampleRate::Hz16000,
            |_: &PreparedPcmBlock, _: &ResolverCancellation| Err(MediaError::Unavailable),
        ),
        factory(
            16_000,
            vec![block(16_000, 1, 0, 47)],
            None,
            Arc::new(AtomicUsize::new(0)),
        ),
        workers.clone(),
    )
    .await;
    assert_eq!(result.err(), Some(MediaError::Unavailable));
    assert!(cancel.is_cancelled());
    assert_eq!(workers.available_permits(), 1);
}

#[tokio::test]
async fn cancellation_during_final_tail_delivery_rejects_even_successful_sink() {
    let cancel = ResolverCancellation::default();
    let result = decode_with_consumer(
        identity(),
        cancel.clone(),
        PreparedConsumer::new(
            identity(),
            PreparedSampleRate::Hz16000,
            |_: &PreparedPcmBlock, cancel: &ResolverCancellation| {
                cancel.cancel();
                Ok(())
            },
        ),
        factory(
            16_000,
            vec![block(16_000, 1, 0, 47)],
            None,
            Arc::new(AtomicUsize::new(0)),
        ),
        lane(),
    )
    .await;
    assert_eq!(result.err(), Some(MediaError::Cancelled));
    assert!(cancel.is_cancelled());
}

#[tokio::test]
async fn valid_source_shorter_than_one_target_frame_reports_insufficient_audio() {
    let result = decode_with_consumer(
        identity(),
        ResolverCancellation::default(),
        PreparedConsumer::new(
            identity(),
            PreparedSampleRate::Hz16000,
            |_: &PreparedPcmBlock, _: &ResolverCancellation| {
                panic!("no target frame may be manufactured")
            },
        ),
        factory(
            192_000,
            vec![block(192_000, 1, 0, 1)],
            None,
            Arc::new(AtomicUsize::new(0)),
        ),
        lane(),
    )
    .await;
    assert_eq!(result.err(), Some(MediaError::InsufficientAudio));
}

#[test]
fn unpolled_public_prepare_future_cancels_without_opening_missing_source() {
    let fixture = tempfile::tempdir().unwrap();
    let input = MediaInput::local(fixture.path().join("missing-synthetic.wav")).unwrap();
    let cancel = ResolverCancellation::default();
    let future = prepare_local_audio(
        input,
        identity(),
        PreparedSampleRate::Hz24000,
        cancel.clone(),
        |_, _| panic!("an unpolled future never emits PCM"),
    );
    drop(future);
    assert!(cancel.is_cancelled());
    assert_eq!(std::fs::read_dir(fixture.path()).unwrap().count(), 0);
}
