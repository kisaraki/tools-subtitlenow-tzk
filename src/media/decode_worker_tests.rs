//! Synthetic pull decoders only; no media files, recorded audio, or providers.
use super::*;
use crate::audio::capture::SessionId;
use std::{
    collections::VecDeque,
    rc::Rc,
    sync::{
        Barrier, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread::ThreadId,
    time::Duration,
};

fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(51),
        generation: 1,
    }
}
fn format() -> DecodedAudioFormat {
    DecodedAudioFormat::new(16_000, 1).unwrap()
}
fn block(sequence: u64, timestamp_us: u64) -> DecodedAudioBlock {
    DecodedAudioBlock::new(identity(), sequence, timestamp_us, format(), vec![0; 160]).unwrap()
}
fn lane() -> Arc<Semaphore> {
    Arc::new(Semaphore::new(1))
}
fn discard(_: &DecodedAudioBlock, _: &ResolverCancellation) -> Result<(), MediaError> {
    Ok(())
}

struct SyntheticDecoder {
    blocks: VecDeque<DecodedAudioBlock>,
    pulls: Arc<AtomicUsize>,
    // Native decoder objects are permitted to be thread-affine and non-Send.
    _thread_affine: Rc<()>,
}
impl MediaDecoder for SyntheticDecoder {
    fn format(&self) -> DecodedAudioFormat {
        format()
    }
    fn next(
        &mut self,
        cancel: &ResolverCancellation,
    ) -> Result<Option<DecodedAudioBlock>, MediaError> {
        check(cancel)?;
        self.pulls.fetch_add(1, Ordering::SeqCst);
        Ok(self.blocks.pop_front())
    }
}
fn synthetic(
    blocks: Vec<DecodedAudioBlock>,
    pulls: Arc<AtomicUsize>,
) -> impl FnOnce(&ResolverCancellation) -> Result<Box<dyn MediaDecoder>, MediaError> + Send {
    move |_| {
        Ok(Box::new(SyntheticDecoder {
            blocks: blocks.into(),
            pulls,
            _thread_affine: Rc::new(()),
        }))
    }
}

#[tokio::test]
async fn complete_pull_delivers_ordered_borrows_and_returns_only_metadata() {
    let cancel = ResolverCancellation::default();
    let workers = lane();
    let pulls = Arc::new(AtomicUsize::new(0));
    let observations = Arc::new(Mutex::new(Vec::new()));
    let observed = observations.clone();
    let summary = decode_with(
        identity(),
        cancel.clone(),
        move |audio, _| {
            observed
                .lock()
                .unwrap()
                .push((audio.sequence(), audio.timestamp_us(), audio.frames()));
            Ok(())
        },
        synthetic(
            vec![block(1, 500), block(2, 10_500), block(3, 50_000)],
            pulls.clone(),
        ),
        workers.clone(),
    )
    .await
    .unwrap();
    assert_eq!(summary.format(), format());
    assert_eq!(summary.blocks(), 3);
    assert_eq!(summary.frames(), 480);
    assert_eq!(summary.first_timestamp_us(), 500);
    assert_eq!(summary.last_end_us(), 60_000);
    assert_eq!(summary.gaps(), 1);
    assert_eq!(
        *observations.lock().unwrap(),
        [(1, 500, 160), (2, 10_500, 160), (3, 50_000, 160)]
    );
    assert_eq!(pulls.load(Ordering::SeqCst), 4);
    assert!(!cancel.is_cancelled());
    assert_eq!(workers.available_permits(), 1);
}

#[tokio::test]
async fn empty_eof_is_invalid_and_never_becomes_successful_summary() {
    let cancel = ResolverCancellation::default();
    let result = decode_with(
        identity(),
        cancel.clone(),
        discard,
        synthetic(Vec::new(), Arc::new(AtomicUsize::new(0))),
        lane(),
    )
    .await;
    assert_eq!(result.err(), Some(MediaError::InvalidAudio));
    assert!(cancel.is_cancelled());
}

#[tokio::test]
async fn callback_error_stops_pull_without_committing_partial_summary() {
    let cancel = ResolverCancellation::default();
    let pulls = Arc::new(AtomicUsize::new(0));
    let workers = lane();
    let result = decode_with(
        identity(),
        cancel.clone(),
        |_, _| Err(MediaError::Unavailable),
        synthetic(vec![block(1, 0), block(2, 10_000)], pulls.clone()),
        workers.clone(),
    )
    .await;
    assert_eq!(result.err(), Some(MediaError::Unavailable));
    assert_eq!(pulls.load(Ordering::SeqCst), 1);
    assert!(cancel.is_cancelled());
    assert_eq!(workers.available_permits(), 1);
}

#[tokio::test]
async fn duplicate_or_skipped_sequence_is_rejected_before_next_callback() {
    for next_sequence in [1, 3] {
        let consumed = Arc::new(AtomicUsize::new(0));
        let observed = consumed.clone();
        let result = decode_with(
            identity(),
            ResolverCancellation::default(),
            move |_, _| {
                observed.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
            synthetic(
                vec![block(1, 0), block(next_sequence, 10_000)],
                Arc::new(AtomicUsize::new(0)),
            ),
            lane(),
        )
        .await;
        assert_eq!(result.err(), Some(MediaError::InvalidAudio));
        assert_eq!(consumed.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn old_identity_and_changed_format_never_reach_consumer() {
    let old = StreamIdentity {
        session: SessionId(50),
        generation: 1,
    };
    let old_block = DecodedAudioBlock::new(old, 1, 0, format(), vec![0; 160]).unwrap();
    let changed_block = DecodedAudioBlock::new(
        identity(),
        1,
        0,
        DecodedAudioFormat::new(48_000, 2).unwrap(),
        vec![0; 960],
    )
    .unwrap();
    for (audio, expected) in [
        (old_block, MediaError::Stale),
        (changed_block, MediaError::InvalidAudio),
    ] {
        let result = decode_with(
            identity(),
            ResolverCancellation::default(),
            |_, _| panic!("invalid audio must never reach the consumer"),
            synthetic(vec![audio], Arc::new(AtomicUsize::new(0))),
            lane(),
        )
        .await;
        assert_eq!(result.err(), Some(expected));
    }
}

#[tokio::test]
async fn duration_limit_rejects_audio_before_callback_and_accepts_exact_boundary() {
    let exact = block(1, MAX_DECODED_MEDIA_DURATION_US - 10_000);
    let summary = decode_with(
        identity(),
        ResolverCancellation::default(),
        discard,
        synthetic(vec![exact], Arc::new(AtomicUsize::new(0))),
        lane(),
    )
    .await
    .unwrap();
    assert_eq!(summary.last_end_us(), MAX_DECODED_MEDIA_DURATION_US);
    let too_long = block(1, MAX_DECODED_MEDIA_DURATION_US - 9_999);
    let result = decode_with(
        identity(),
        ResolverCancellation::default(),
        |_, _| panic!("out-of-budget audio must not reach the consumer"),
        synthetic(vec![too_long], Arc::new(AtomicUsize::new(0))),
        lane(),
    )
    .await;
    assert_eq!(result.err(), Some(MediaError::TooLarge));
}

#[tokio::test]
async fn pre_cancelled_scope_does_not_construct_decoder_or_take_capacity() {
    let cancel = ResolverCancellation::default();
    cancel.cancel();
    let workers = lane();
    let result = decode_with(
        identity(),
        cancel,
        discard,
        |_| panic!("a cancelled request must not construct its decoder"),
        workers.clone(),
    )
    .await;
    assert_eq!(result.err(), Some(MediaError::Cancelled));
    assert_eq!(workers.available_permits(), 1);
}

#[test]
fn dropping_unpolled_future_cancels_without_creating_worker() {
    let cancel = ResolverCancellation::default();
    let workers = lane();
    let future = decode_with(
        identity(),
        cancel.clone(),
        discard,
        |_| panic!("an unpolled request must not construct its decoder"),
        workers.clone(),
    );
    assert!(!cancel.is_cancelled());
    drop(future);
    assert!(cancel.is_cancelled());
    assert_eq!(workers.available_permits(), 1);
}

#[tokio::test]
async fn saturated_lane_returns_overloaded_without_queuing_or_factory_call() {
    let workers = lane();
    let permit = workers.clone().try_acquire_owned().unwrap();
    let cancel = ResolverCancellation::default();
    let result = decode_with(
        identity(),
        cancel.clone(),
        discard,
        |_| panic!("an overloaded request must not construct its decoder"),
        workers.clone(),
    )
    .await;
    assert_eq!(result.err(), Some(MediaError::Overloaded));
    assert!(cancel.is_cancelled());
    assert_eq!(workers.available_permits(), 0);
    drop(permit);
    assert_eq!(workers.available_permits(), 1);
}

#[tokio::test]
async fn dropping_started_future_keeps_permit_until_actual_worker_exit() {
    let workers = lane();
    let cancel = ResolverCancellation::default();
    let release = Arc::new(Barrier::new(2));
    let worker_release = release.clone();
    let (started, ready) = tokio::sync::oneshot::channel();
    let mut future = Box::pin(decode_with(
        identity(),
        cancel.clone(),
        discard,
        move |cancel| {
            started.send(()).unwrap();
            worker_release.wait();
            check(cancel)?;
            panic!("dropped worker must observe cancellation after release")
        },
        workers.clone(),
    ));
    tokio::select! {
        result = &mut future => panic!("worker finished before release: {:?}", result.err()),
        signal = ready => signal.unwrap(),
    }
    drop(future);
    assert!(cancel.is_cancelled());
    assert_eq!(workers.available_permits(), 0);
    let second = decode_with(
        identity(),
        ResolverCancellation::default(),
        discard,
        |_| panic!("a dropped but running worker still owns its capacity"),
        workers.clone(),
    )
    .await;
    assert_eq!(second.err(), Some(MediaError::Overloaded));
    release.wait();
    let permit = tokio::time::timeout(Duration::from_secs(5), workers.acquire())
        .await
        .unwrap()
        .unwrap();
    drop(permit);
    let summary = decode_with(
        identity(),
        ResolverCancellation::default(),
        discard,
        synthetic(vec![block(1, 0)], Arc::new(AtomicUsize::new(0))),
        workers,
    )
    .await
    .unwrap();
    assert_eq!(summary.blocks(), 1);
}

#[tokio::test]
async fn external_cancel_returns_while_started_worker_remains_bounded() {
    let workers = lane();
    let cancel = ResolverCancellation::default();
    let release = Arc::new(Barrier::new(2));
    let worker_release = release.clone();
    let (started, ready) = tokio::sync::oneshot::channel();
    let mut future = Box::pin(decode_with(
        identity(),
        cancel.clone(),
        discard,
        move |cancel| {
            started.send(()).unwrap();
            worker_release.wait();
            check(cancel)?;
            panic!("cancelled worker must stop after release")
        },
        workers.clone(),
    ));
    tokio::select! {
        result = &mut future => panic!("worker finished before release: {:?}", result.err()),
        signal = ready => signal.unwrap(),
    }
    cancel.cancel();
    assert_eq!(future.await.err(), Some(MediaError::Cancelled));
    assert_eq!(workers.available_permits(), 0);
    release.wait();
    let permit = tokio::time::timeout(Duration::from_secs(5), workers.acquire())
        .await
        .unwrap()
        .unwrap();
    drop(permit);
}

#[tokio::test]
async fn callback_cancel_never_commits_even_its_last_successful_block() {
    let cancel = ResolverCancellation::default();
    let pulls = Arc::new(AtomicUsize::new(0));
    let result = decode_with(
        identity(),
        cancel.clone(),
        |_, cancel| {
            cancel.cancel();
            Ok(())
        },
        synthetic(vec![block(1, 0), block(2, 10_000)], pulls.clone()),
        lane(),
    )
    .await;
    assert_eq!(result.err(), Some(MediaError::Cancelled));
    assert_eq!(pulls.load(Ordering::SeqCst), 1);
    assert!(cancel.is_cancelled());
}

#[tokio::test]
async fn cancellation_from_eof_is_checked_before_summary() {
    struct CancellingEof;
    impl MediaDecoder for CancellingEof {
        fn format(&self) -> DecodedAudioFormat {
            format()
        }
        fn next(
            &mut self,
            cancel: &ResolverCancellation,
        ) -> Result<Option<DecodedAudioBlock>, MediaError> {
            cancel.cancel();
            Ok(None)
        }
    }
    let result = decode_with(
        identity(),
        ResolverCancellation::default(),
        discard,
        |_| Ok(Box::new(CancellingEof)),
        lane(),
    )
    .await;
    assert_eq!(result.err(), Some(MediaError::Cancelled));
}

#[tokio::test]
async fn invalid_request_identity_is_rejected_before_factory_io() {
    for invalid in [
        StreamIdentity {
            session: SessionId(0),
            generation: 1,
        },
        StreamIdentity {
            session: SessionId(51),
            generation: 0,
        },
    ] {
        let result = decode_with(
            invalid,
            ResolverCancellation::default(),
            discard,
            |_| panic!("an invalid session must not construct a decoder"),
            lane(),
        )
        .await;
        assert_eq!(result.err(), Some(MediaError::InvalidAudio));
    }
}

#[tokio::test]
async fn worker_panic_is_typed_and_capacity_recovers() {
    let cancel = ResolverCancellation::default();
    let workers = lane();
    let result = decode_with(
        identity(),
        cancel.clone(),
        discard,
        |_| panic!("explicit synthetic decode worker panic"),
        workers.clone(),
    )
    .await;
    assert_eq!(result.err(), Some(MediaError::WorkerFailed));
    assert!(cancel.is_cancelled());
    assert_eq!(workers.available_permits(), 1);
    let summary = decode_with(
        identity(),
        ResolverCancellation::default(),
        discard,
        synthetic(vec![block(1, 0)], Arc::new(AtomicUsize::new(0))),
        workers,
    )
    .await
    .unwrap();
    assert_eq!(summary.frames(), 160);
}

#[tokio::test]
async fn decoder_creation_and_destruction_stay_on_same_non_owner_thread() {
    #[derive(Default)]
    struct Threads {
        created: Option<ThreadId>,
        dropped: Option<ThreadId>,
    }
    struct AffineDecoder {
        block: Option<DecodedAudioBlock>,
        observations: Arc<Mutex<Threads>>,
        _non_send: Rc<()>,
    }
    impl MediaDecoder for AffineDecoder {
        fn format(&self) -> DecodedAudioFormat {
            format()
        }
        fn next(
            &mut self,
            cancel: &ResolverCancellation,
        ) -> Result<Option<DecodedAudioBlock>, MediaError> {
            check(cancel)?;
            assert_eq!(
                self.observations.lock().unwrap().created,
                Some(std::thread::current().id())
            );
            Ok(self.block.take())
        }
    }
    impl Drop for AffineDecoder {
        fn drop(&mut self) {
            self.observations.lock().unwrap().dropped = Some(std::thread::current().id());
        }
    }
    let owner = std::thread::current().id();
    let observations = Arc::new(Mutex::new(Threads::default()));
    let observed = observations.clone();
    decode_with(
        identity(),
        ResolverCancellation::default(),
        discard,
        move |_| {
            observed.lock().unwrap().created = Some(std::thread::current().id());
            Ok(Box::new(AffineDecoder {
                block: Some(block(1, 0)),
                observations: observed,
                _non_send: Rc::new(()),
            }))
        },
        lane(),
    )
    .await
    .unwrap();
    let observations = observations.lock().unwrap();
    assert_ne!(observations.created, Some(owner));
    assert_eq!(observations.created, observations.dropped);
}

#[tokio::test]
async fn factory_failure_preserves_typed_error_and_cancels_scope() {
    let cancel = ResolverCancellation::default();
    let workers = lane();
    let result = decode_with(
        identity(),
        cancel.clone(),
        discard,
        |_| Err(MediaError::Unavailable),
        workers.clone(),
    )
    .await;
    assert_eq!(result.err(), Some(MediaError::Unavailable));
    assert!(cancel.is_cancelled());
    assert_eq!(workers.available_permits(), 1);
}

// The EOF-aware seam retains metadata only. These finite synthetic consumers
// never invoke native decoding, a cloud provider, or the shared production lane.
struct MetadataOnlyOutput {
    identity: StreamIdentity,
    blocks: u64,
    frames: u64,
}

#[derive(Default)]
struct EofObservations {
    pushes: AtomicUsize,
    finishes: AtomicUsize,
}

enum FinishOutcome {
    Success,
    Cancel,
    Error,
    Panic,
    // Test-only blocking gate with a finite wait, so a failed assertion or a
    // dropped sender cannot strand a blocking worker or runtime shutdown.
    Wait {
        started: tokio::sync::oneshot::Sender<()>,
        release: std::sync::mpsc::Receiver<()>,
    },
}

struct EofConsumer {
    observations: Arc<EofObservations>,
    outcome: FinishOutcome,
}
impl DecodeConsumer for EofConsumer {
    type Output = MetadataOnlyOutput;

    fn push(
        &mut self,
        _: &DecodedAudioBlock,
        cancel: &ResolverCancellation,
    ) -> Result<(), MediaError> {
        check(cancel)?;
        self.observations.pushes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn finish(
        self,
        summary: DecodedAudioSummary,
        cancel: &ResolverCancellation,
    ) -> Result<Self::Output, MediaError> {
        check(cancel)?;
        self.observations.finishes.fetch_add(1, Ordering::SeqCst);
        assert_eq!(summary.identity(), identity());
        assert!(summary.blocks() > 0);
        match self.outcome {
            FinishOutcome::Success => {}
            FinishOutcome::Cancel => cancel.cancel(),
            FinishOutcome::Error => return Err(MediaError::Unavailable),
            FinishOutcome::Panic => panic!("explicit synthetic EOF consumer panic"),
            FinishOutcome::Wait { started, release } => {
                started.send(()).unwrap();
                release.recv_timeout(Duration::from_secs(5)).unwrap();
                check(cancel)?;
            }
        }
        Ok(MetadataOnlyOutput {
            identity: summary.identity(),
            blocks: summary.blocks(),
            frames: summary.frames(),
        })
    }
}

fn eof_consumer(observations: Arc<EofObservations>, outcome: FinishOutcome) -> EofConsumer {
    EofConsumer {
        observations,
        outcome,
    }
}

#[tokio::test]
async fn validated_nonempty_eof_finishes_once_after_all_successful_pushes() {
    let observations = Arc::new(EofObservations::default());
    let pulls = Arc::new(AtomicUsize::new(0));
    let cancel = ResolverCancellation::default();
    let workers = lane();
    let output = decode_with_consumer(
        identity(),
        cancel.clone(),
        eof_consumer(observations.clone(), FinishOutcome::Success),
        synthetic(vec![block(1, 0), block(2, 10_000)], pulls.clone()),
        workers.clone(),
    )
    .await
    .unwrap();
    assert_eq!(output.identity, identity());
    assert_eq!(output.blocks, 2);
    assert_eq!(output.frames, 320);
    assert_eq!(observations.pushes.load(Ordering::SeqCst), 2);
    assert_eq!(observations.finishes.load(Ordering::SeqCst), 1);
    assert_eq!(pulls.load(Ordering::SeqCst), 3);
    assert!(!cancel.is_cancelled());
    assert_eq!(workers.available_permits(), 1);
}

#[tokio::test]
async fn empty_eof_never_calls_private_finish() {
    let observations = Arc::new(EofObservations::default());
    let workers = lane();
    let result = decode_with_consumer(
        identity(),
        ResolverCancellation::default(),
        eof_consumer(observations.clone(), FinishOutcome::Success),
        synthetic(Vec::new(), Arc::new(AtomicUsize::new(0))),
        workers.clone(),
    )
    .await;
    assert_eq!(result.err(), Some(MediaError::InvalidAudio));
    assert_eq!(observations.pushes.load(Ordering::SeqCst), 0);
    assert_eq!(observations.finishes.load(Ordering::SeqCst), 0);
    assert_eq!(workers.available_permits(), 1);
}

#[tokio::test]
async fn late_invalid_block_never_reaches_push_or_private_finish() {
    let observations = Arc::new(EofObservations::default());
    let pulls = Arc::new(AtomicUsize::new(0));
    let workers = lane();
    let result = decode_with_consumer(
        identity(),
        ResolverCancellation::default(),
        eof_consumer(observations.clone(), FinishOutcome::Success),
        synthetic(vec![block(1, 0), block(3, 10_000)], pulls.clone()),
        workers.clone(),
    )
    .await;
    assert_eq!(result.err(), Some(MediaError::InvalidAudio));
    assert_eq!(observations.pushes.load(Ordering::SeqCst), 1);
    assert_eq!(observations.finishes.load(Ordering::SeqCst), 0);
    assert_eq!(pulls.load(Ordering::SeqCst), 2);
    assert_eq!(workers.available_permits(), 1);
}

#[tokio::test]
async fn decoder_error_after_valid_input_never_calls_private_finish() {
    struct FailingTailDecoder(Option<DecodedAudioBlock>);
    impl MediaDecoder for FailingTailDecoder {
        fn format(&self) -> DecodedAudioFormat {
            format()
        }
        fn next(
            &mut self,
            cancel: &ResolverCancellation,
        ) -> Result<Option<DecodedAudioBlock>, MediaError> {
            check(cancel)?;
            self.0.take().map(Some).ok_or(MediaError::UnsupportedMedia)
        }
    }
    let observations = Arc::new(EofObservations::default());
    let workers = lane();
    let result = decode_with_consumer(
        identity(),
        ResolverCancellation::default(),
        eof_consumer(observations.clone(), FinishOutcome::Success),
        |_| Ok(Box::new(FailingTailDecoder(Some(block(1, 0))))),
        workers.clone(),
    )
    .await;
    assert_eq!(result.err(), Some(MediaError::UnsupportedMedia));
    assert_eq!(observations.pushes.load(Ordering::SeqCst), 1);
    assert_eq!(observations.finishes.load(Ordering::SeqCst), 0);
    assert_eq!(workers.available_permits(), 1);
}

async fn assert_failed_finish(outcome: FinishOutcome, expected: MediaError) {
    let observations = Arc::new(EofObservations::default());
    let workers = lane();
    let cancel = ResolverCancellation::default();
    let result = decode_with_consumer(
        identity(),
        cancel.clone(),
        eof_consumer(observations.clone(), outcome),
        synthetic(vec![block(1, 0)], Arc::new(AtomicUsize::new(0))),
        workers.clone(),
    )
    .await;
    assert_eq!(result.err(), Some(expected));
    assert_eq!(observations.pushes.load(Ordering::SeqCst), 1);
    assert_eq!(observations.finishes.load(Ordering::SeqCst), 1);
    assert!(cancel.is_cancelled());
    // Cancellation can win select before the already-started worker returns.
    // Wait for actual exit, then prove the recovered lane can execute again.
    let permit = tokio::time::timeout(Duration::from_secs(5), workers.acquire())
        .await
        .unwrap()
        .unwrap();
    drop(permit);
    let recovered = decode_with(
        identity(),
        ResolverCancellation::default(),
        discard,
        synthetic(vec![block(1, 0)], Arc::new(AtomicUsize::new(0))),
        workers,
    )
    .await
    .unwrap();
    assert_eq!(recovered.blocks(), 1);
}

#[tokio::test]
async fn private_finish_cancellation_rejects_even_successful_output() {
    assert_failed_finish(FinishOutcome::Cancel, MediaError::Cancelled).await;
}

#[tokio::test]
async fn private_finish_typed_failure_rejects_output_and_recovers_capacity() {
    assert_failed_finish(FinishOutcome::Error, MediaError::Unavailable).await;
}

#[tokio::test]
async fn private_finish_panic_rejects_output_and_recovers_capacity() {
    assert_failed_finish(FinishOutcome::Panic, MediaError::WorkerFailed).await;
}

#[tokio::test]
async fn dropping_owner_during_private_finish_keeps_permit_until_actual_exit() {
    let observations = Arc::new(EofObservations::default());
    let workers = lane();
    let cancel = ResolverCancellation::default();
    let (started, ready) = tokio::sync::oneshot::channel();
    let (release, blocked) = std::sync::mpsc::channel();
    let mut future = Box::pin(decode_with_consumer(
        identity(),
        cancel.clone(),
        eof_consumer(
            observations.clone(),
            FinishOutcome::Wait {
                started,
                release: blocked,
            },
        ),
        synthetic(vec![block(1, 0)], Arc::new(AtomicUsize::new(0))),
        workers.clone(),
    ));
    tokio::select! {
        result = &mut future => panic!("finish completed before release: {:?}", result.err()),
        signal = tokio::time::timeout(Duration::from_secs(5), ready) => signal.unwrap().unwrap(),
    }
    assert_eq!(observations.pushes.load(Ordering::SeqCst), 1);
    assert_eq!(observations.finishes.load(Ordering::SeqCst), 1);
    drop(future);
    assert!(cancel.is_cancelled());
    assert_eq!(workers.available_permits(), 0);
    let second = decode_with_consumer(
        identity(),
        ResolverCancellation::default(),
        eof_consumer(Arc::new(EofObservations::default()), FinishOutcome::Success),
        |_| panic!("a started finish still owns its worker capacity"),
        workers.clone(),
    )
    .await;
    assert_eq!(second.err(), Some(MediaError::Overloaded));
    release.send(()).unwrap();
    let permit = tokio::time::timeout(Duration::from_secs(5), workers.acquire())
        .await
        .unwrap()
        .unwrap();
    drop(permit);
    assert_eq!(workers.available_permits(), 1);
    assert_eq!(observations.finishes.load(Ordering::SeqCst), 1);
}

#[derive(Default)]
struct NativeOwnerObservations {
    created: Option<ThreadId>,
    dropped: Option<ThreadId>,
    owner_permits_at_drop: Option<usize>,
    owner_permits_after_drop_gate: Option<usize>,
    format_calls: usize,
    pulls: usize,
}

type NativeDropGate = (
    tokio::sync::oneshot::Sender<()>,
    std::sync::mpsc::Receiver<()>,
);

struct OwnerObservedDecoder {
    block: Option<DecodedAudioBlock>,
    owners: Arc<Semaphore>,
    observations: Arc<Mutex<NativeOwnerObservations>>,
    drop_gate: Option<NativeDropGate>,
    _thread_affine: Rc<()>,
}
impl MediaDecoder for OwnerObservedDecoder {
    fn format(&self) -> DecodedAudioFormat {
        let mut observed = self.observations.lock().unwrap();
        assert_eq!(observed.created, Some(std::thread::current().id()));
        observed.format_calls += 1;
        format()
    }
    fn next(
        &mut self,
        cancel: &ResolverCancellation,
    ) -> Result<Option<DecodedAudioBlock>, MediaError> {
        check(cancel)?;
        let mut observed = self.observations.lock().unwrap();
        assert_eq!(observed.created, Some(std::thread::current().id()));
        observed.pulls += 1;
        Ok(self.block.take())
    }
}
impl Drop for OwnerObservedDecoder {
    fn drop(&mut self) {
        {
            let mut observed = self.observations.lock().unwrap();
            observed.dropped = Some(std::thread::current().id());
            observed.owner_permits_at_drop = Some(self.owners.available_permits());
        }
        if let Some((started, release)) = self.drop_gate.take() {
            let _ = started.send(());
            // A finite wait and sender-Drop recovery prevent a failing test
            // from leaving a started blocking worker alive indefinitely.
            let _ = release.recv_timeout(Duration::from_secs(5));
        }
        self.observations
            .lock()
            .unwrap()
            .owner_permits_after_drop_gate = Some(self.owners.available_permits());
    }
}
fn owned_fixture(
    owners: Arc<Semaphore>,
    observations: Arc<Mutex<NativeOwnerObservations>>,
    drop_gate: Option<NativeDropGate>,
) -> Result<Box<dyn MediaDecoder>, MediaError> {
    // Only this test's independent lane is used; injected decode factories
    // never consume process-wide production native-owner capacity.
    let owner = owners
        .clone()
        .try_acquire_owned()
        .map_err(|_| MediaError::Overloaded)?;
    observations.lock().unwrap().created = Some(std::thread::current().id());
    Ok(Box::new(OwnedDecoder {
        inner: Box::new(OwnerObservedDecoder {
            block: Some(block(1, 0)),
            owners,
            observations,
            drop_gate,
            _thread_affine: Rc::new(()),
        }),
        _owner: owner,
    }))
}

#[tokio::test]
async fn native_owner_wrapper_delegates_and_releases_capacity_after_same_thread_inner_drop() {
    for fail_consumer in [false, true] {
        let owners = lane();
        let workers = lane();
        let observations = Arc::new(Mutex::new(NativeOwnerObservations::default()));
        let inner_owners = owners.clone();
        let inner_observations = observations.clone();
        let caller_thread = std::thread::current().id();
        let result = decode_with(
            identity(),
            ResolverCancellation::default(),
            move |_, _| {
                if fail_consumer {
                    Err(MediaError::Unavailable)
                } else {
                    Ok(())
                }
            },
            move |_| owned_fixture(inner_owners, inner_observations, None),
            workers.clone(),
        )
        .await;
        if fail_consumer {
            assert_eq!(result.err(), Some(MediaError::Unavailable));
        } else {
            assert_eq!(result.unwrap().frames(), 160);
        }
        let observed = observations.lock().unwrap();
        assert_ne!(observed.created, Some(caller_thread));
        assert_eq!(observed.created, observed.dropped);
        assert_eq!(observed.owner_permits_at_drop, Some(0));
        assert_eq!(observed.owner_permits_after_drop_gate, Some(0));
        assert_eq!(observed.format_calls, 1);
        assert_eq!(observed.pulls, if fail_consumer { 1 } else { 2 });
        assert_eq!(owners.available_permits(), 1);
        assert_eq!(workers.available_permits(), 1);
    }
}

#[tokio::test]
async fn dropped_async_owner_retains_both_lanes_until_inner_native_destruction_finishes() {
    let owners = lane();
    let workers = lane();
    let cancel = ResolverCancellation::default();
    let observations = Arc::new(Mutex::new(NativeOwnerObservations::default()));
    let inner_owners = owners.clone();
    let inner_observations = observations.clone();
    let (started, ready) = tokio::sync::oneshot::channel();
    let (release, blocked) = std::sync::mpsc::channel();
    let mut future = Box::pin(decode_with(
        identity(),
        cancel.clone(),
        discard,
        move |_| owned_fixture(inner_owners, inner_observations, Some((started, blocked))),
        workers.clone(),
    ));
    tokio::select! {
        result = &mut future => panic!("native destructor returned before release: {:?}", result.err()),
        signal = tokio::time::timeout(Duration::from_secs(5), ready) => signal.unwrap().unwrap(),
    }
    drop(future);
    assert!(cancel.is_cancelled());
    assert_eq!(owners.available_permits(), 0);
    assert_eq!(workers.available_permits(), 0);
    assert_eq!(observations.lock().unwrap().owner_permits_at_drop, Some(0));
    release.send(()).unwrap();
    let owner = tokio::time::timeout(Duration::from_secs(5), owners.acquire())
        .await
        .unwrap()
        .unwrap();
    let worker = tokio::time::timeout(Duration::from_secs(5), workers.acquire())
        .await
        .unwrap()
        .unwrap();
    let observed = observations.lock().unwrap();
    assert_eq!(observed.created, observed.dropped);
    assert_ne!(observed.created, Some(std::thread::current().id()));
    assert_eq!(observed.owner_permits_after_drop_gate, Some(0));
    drop(observed);
    drop(owner);
    drop(worker);
    assert_eq!(owners.available_permits(), 1);
    assert_eq!(workers.available_permits(), 1);
}
