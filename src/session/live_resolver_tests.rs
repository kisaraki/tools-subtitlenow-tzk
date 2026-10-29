//! Explicit offline adapters; no catalog service, real speech, keys or native audio.
use super::*;
use crate::{
    ai::speech::*,
    audio::capture::{AudioStamp, BLOCK_FRAMES, SessionId},
    domain::SourceLanguage,
    resolver::{contracts::*, session::ResolverStatus},
};
use std::{
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(91),
        generation: 3,
    }
}
fn block(seq: u64) -> NormalizedAudio {
    NormalizedAudio {
        stamp: AudioStamp {
            session: identity().session,
            sequence: seq,
            frame_position: None,
            qpc_100ns: None,
            received: Instant::now(),
            discontinuity: false,
        },
        frames: BLOCK_FRAMES,
        samples: [0.2; BLOCK_FRAMES],
    }
}
struct SlowMock {
    calls: AtomicUsize,
    active: Arc<AtomicUsize>,
    tokens: Mutex<Vec<ResolverCancellation>>,
}
struct Active(Arc<AtomicUsize>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
impl ContentIdentifier for SlowMock {
    fn capabilities(&self) -> ResolverCapabilities {
        ResolverCapabilities {
            classification: true,
            songs: false,
            movies: false,
            cancellation: true,
        }
    }
    fn identify<'a>(
        &'a self,
        audio: &'a AudioWindow,
        cancel: ResolverCancellation,
    ) -> IdentifyFuture<'a> {
        Box::pin(async move {
            assert_eq!(audio.stamp.identity, identity());
            assert_eq!(audio.samples().len(), 192000);
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.active.fetch_add(1, Ordering::SeqCst);
            let _active = Active(Arc::clone(&self.active));
            self.tokens.lock().unwrap().push(cancel.clone());
            cancel.cancelled().await;
            Err(ResolverError::Cancelled)
        })
    }
}
fn provider() -> Arc<SlowMock> {
    Arc::new(SlowMock {
        calls: AtomicUsize::new(0),
        active: Arc::new(AtomicUsize::new(0)),
        tokens: Mutex::new(Vec::new()),
    })
}
struct MockSpeech;
impl SpeechRecognizer for MockSpeech {
    fn capabilities(&self) -> SpeechCapabilities {
        SpeechCapabilities {
            streaming: true,
            partials: true,
            cancellation: true,
            supported_languages: [true; 5],
            constrained_auto_detection: true,
        }
    }
    fn run<'a>(
        &'a self,
        _: SourceLanguage,
        mut audio: SpeechAudio,
        output: TranscriptOutput,
        _: Cancellation,
    ) -> SpeechFuture<'a> {
        Box::pin(async move {
            loop {
                let audio = audio.recv().await?;
                for (revision, kind) in [(1, TranscriptKind::Partial), (2, TranscriptKind::Final)] {
                    output.emit(TranscriptEvent {
                        identity: identity(),
                        sequence: audio.stamp.sequence,
                        utterance: audio.stamp.sequence + 1,
                        revision,
                        start_us: None,
                        duration_us: None,
                        language: Some(SpeechLanguage::English),
                        kind,
                        text: "synthetic STT".into(),
                    })?;
                }
            }
        })
    }
}
async fn pump(resolver: &mut LiveResolver) {
    let result = futures_util::poll!(Box::pin(resolver.changed()));
    if let std::task::Poll::Ready(result) = result {
        result.unwrap();
    }
}

#[tokio::test]
async fn normalized_fanout_keeps_stt_running_during_slow_lookup_and_drop_reclaims_resolver_work() {
    let adapter = provider();
    let mut resolver = LiveResolver::new(identity(), adapter.clone()).unwrap();
    let input = resolver.audio.input();
    let (mut speech, task) = SpeechSession::new(
        identity(),
        SourceLanguage::English,
        MockSpeech.capabilities(),
    )
    .unwrap();
    let mut speech_run = Box::pin(task.run(&MockSpeech));
    for seq in 0..=600 {
        let count = resolver.metrics().processed_blocks;
        assert_eq!(
            submit_live_audio(&speech.input, Some(&resolver), block(seq)).unwrap(),
            Some(Ok(()))
        );
        assert!(futures_util::poll!(speech_run.as_mut()).is_pending());
        for kind in [TranscriptKind::Partial, TranscriptKind::Final] {
            let event = speech.next().await.unwrap();
            assert_eq!(event.sequence, seq);
            assert_eq!(event.kind, kind);
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while resolver.metrics().processed_blocks == count {
            pump(&mut resolver).await;
            assert!(Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        pump(&mut resolver).await;
    }
    assert_eq!(adapter.calls.load(Ordering::SeqCst), 1);
    assert_eq!(adapter.active.load(Ordering::SeqCst), 1);
    assert!(speech.cancellation().reason().is_none());
    let assembly = resolver.audio.current();
    let weak = Arc::downgrade(assembly.window.as_ref().unwrap());
    drop(assembly);
    drop(resolver);
    assert_eq!(adapter.active.load(Ordering::SeqCst), 0);
    assert!(adapter.tokens.lock().unwrap()[0].is_cancelled());
    assert!(weak.upgrade().is_none());
    assert_eq!(input.try_push(block(601)), Err(ResolverError::Cancelled));
    assert!(speech.cancellation().reason().is_none());
    speech.cancellation().cancel();
    assert_eq!(speech_run.await, Err(SpeechError::Cancelled));
}

#[tokio::test]
async fn admission_rejection_never_reaches_resolver_and_resolver_failure_never_cancels_stt() {
    let mut resolver = LiveResolver::new(identity(), provider()).unwrap();
    let (speech, _task) = SpeechSession::new(
        identity(),
        SourceLanguage::English,
        MockSpeech.capabilities(),
    )
    .unwrap();
    for seq in 0..12 {
        assert_eq!(
            submit_live_audio(&speech.input, None, block(seq)).unwrap(),
            None
        );
    }
    assert_eq!(
        submit_live_audio(&speech.input, Some(&resolver), block(12)),
        Err(SpeechError::queue_full(
            crate::ai::speech::SpeechQueue::AudioInput,
            crate::ai::speech::AUDIO_QUEUE_BLOCKS
        ))
    );
    assert_eq!(resolver.metrics().processed_blocks, 0);
    assert_eq!(resolver.metrics().queue_depth, 0);
    let (speech, mut task) = SpeechSession::new(
        identity(),
        SourceLanguage::English,
        MockSpeech.capabilities(),
    )
    .unwrap();
    let mut receiver = task.take_audio_for_test();
    resolver.stop().unwrap();
    assert_eq!(
        submit_live_audio(&speech.input, Some(&resolver), block(0)).unwrap(),
        Some(Err(ResolverError::Cancelled))
    );
    assert!(speech.cancellation().reason().is_none());
    assert_eq!(receiver.recv().await.unwrap().stamp.sequence, 0);
    assert_eq!(resolver.metrics().worker_count, 0);
}

#[tokio::test]
async fn facade_processes_audio_reset_and_cancels_lookup_without_new_window() {
    let adapter = provider();
    let mut resolver = LiveResolver::new(identity(), adapter.clone()).unwrap();
    for seq in 0..600 {
        let count = resolver.metrics().processed_blocks;
        resolver.try_push(block(seq)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while resolver.metrics().processed_blocks == count {
            pump(&mut resolver).await;
            assert!(Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        pump(&mut resolver).await;
    }
    assert_eq!(adapter.active.load(Ordering::SeqCst), 1);
    // Local resolver branch loses a packet; no new 12-second window is available.
    resolver.try_push(block(601)).unwrap();
    let reset = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let update = resolver.changed().await.unwrap();
            if matches!(update.status, ResolverStatus::Failed(_)) {
                break update;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(
        reset.status,
        ResolverStatus::Failed(ResolverError::AudioDiscontinuity)
    );
    assert!(reset.resolution.song.is_none());
    assert_eq!(adapter.active.load(Ordering::SeqCst), 0);
    assert!(adapter.tokens.lock().unwrap()[0].is_cancelled());
    assert_eq!(adapter.calls.load(Ordering::SeqCst), 1);
    resolver.stop().unwrap();
    assert!(matches!(
        resolver.changed().await,
        Err(ResolverError::Cancelled)
    ));
}

#[tokio::test]
async fn unread_ready_evidence_is_filtered_when_audio_resets_before_adoption() {
    struct ImmediateMock;
    impl ContentIdentifier for ImmediateMock {
        fn capabilities(&self) -> ResolverCapabilities {
            provider().capabilities()
        }
        fn identify<'a>(
            &'a self,
            _: &'a AudioWindow,
            _: ResolverCancellation,
        ) -> IdentifyFuture<'a> {
            Box::pin(async {
                Ok(ContentEvidence {
                    kind: ContentType::Speech,
                    confidence: Confidence::new(900).unwrap(),
                    songs: Vec::new(),
                    movies: Vec::new(),
                })
            })
        }
    }
    let mut resolver = LiveResolver::new(identity(), Arc::new(ImmediateMock)).unwrap();
    for seq in 0..600 {
        let count = resolver.metrics().processed_blocks;
        resolver.try_push(block(seq)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while resolver.metrics().processed_blocks == count {
            assert!(Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }
    let assembly = resolver.audio.latest();
    resolver.accept(assembly).unwrap();
    assert!(futures_util::poll!(resolver.run.as_mut().unwrap().as_mut()).is_pending());
    // Ready exists in the output mailbox, but has not been consumed by the owner.
    resolver.try_push(block(601)).unwrap();
    let update = tokio::time::timeout(Duration::from_secs(1), resolver.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        update.status,
        ResolverStatus::Failed(ResolverError::AudioDiscontinuity)
    );
    assert_eq!(update.resolution.kind, ContentType::Unknown);
    assert!(update.resolution.song.is_none());
}
