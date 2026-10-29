//! Local algorithm and owned-worker regressions. All PCM is synthetic and RAM-only.
use super::*;
use crate::{audio::capture::SessionId, resolver::fingerprint::ChromaprintExtractor};
use rusty_chromaprint::{Configuration, FingerprintCompressor, Fingerprinter};
use std::sync::{Barrier, atomic::AtomicUsize};

fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(12000),
        generation: 2,
    }
}
fn input(sequence: u64, variant: bool) -> Arc<AudioWindow> {
    let notes = if variant {
        [659.25, 440.0, 783.99, 523.25]
    } else {
        [220.0, 261.63, 329.63, 392.0]
    };
    let samples = (0..WINDOW_FRAMES)
        .map(|index| {
            let time = index as f64 / f64::from(AudioWindow::SAMPLE_RATE);
            let frequency = notes[(index / 4000) % notes.len()];
            let envelope = 0.25 + 0.15 * (time * 5.0).sin();
            let wave = (std::f64::consts::TAU * frequency * time).sin()
                + 0.3 * (std::f64::consts::TAU * frequency * 2.0 * time).sin();
            (wave * envelope * 16000.0) as i16
        })
        .collect::<Vec<_>>();
    Arc::new(AudioWindow {
        stamp: WindowStamp {
            identity: identity(),
            sequence,
            ended_at: Instant::now(),
            start_us: None,
            duration_us: WINDOW_SECONDS as u64 * 1_000_000,
        },
        pcm: samples.into(),
    })
}
fn blank_input(sequence: u64) -> Arc<AudioWindow> {
    Arc::new(AudioWindow {
        stamp: WindowStamp {
            identity: identity(),
            sequence,
            ended_at: Instant::now(),
            start_us: None,
            duration_us: WINDOW_SECONDS as u64 * 1_000_000,
        },
        pcm: vec![0; WINDOW_FRAMES].into(),
    })
}
#[test]
fn actual_chromaprint_is_deterministic_and_chunking_matches_upstream_compression() {
    let input = input(1, false);
    let cancel = ResolverCancellation::default();
    let first = ChromaprintExtractor.extract(&input, &cancel).unwrap();
    let second = ChromaprintExtractor.extract(&input, &cancel).unwrap();
    let config = Configuration::preset_test2();
    let mut reference = Fingerprinter::new(&config);
    reference.start(16000, 1).unwrap();
    reference.consume(input.samples());
    reference.finish();
    let expected = FingerprintCompressor::from(&config).compress(reference.fingerprint());
    assert!(!reference.fingerprint().is_empty());
    assert_eq!(
        first.format(),
        FingerprintFormat::ChromaprintTest2Compressed
    );
    assert_eq!(first.bytes(), expected);
    assert_eq!(first.bytes(), second.bytes());
    assert_eq!(first.bytes()[0], config.id());
    assert!(first.bytes().len() <= MAX_FINGERPRINT_BYTES);
    let different = ChromaprintExtractor
        .extract(&self::input(2, true), &cancel)
        .unwrap();
    assert_ne!(first.bytes(), different.bytes());
}
#[test]
fn actual_extractor_rejects_silence_stale_dimensions_and_precancel() {
    let mut silence = input(1, false);
    Arc::get_mut(&mut silence).unwrap().pcm = vec![0; WINDOW_FRAMES].into();
    let token = ResolverCancellation::default();
    assert!(matches!(
        ChromaprintExtractor.extract(&silence, &token),
        Err(ResolverError::InsufficientSignal)
    ));
    let mut stale = input(2, false);
    Arc::get_mut(&mut stale).unwrap().stamp.ended_at =
        Instant::now() - MAX_WINDOW_AGE - Duration::from_secs(1);
    assert!(matches!(
        ChromaprintExtractor.extract(&stale, &token),
        Err(ResolverError::Stale)
    ));
    let mut malformed = input(3, false);
    Arc::get_mut(&mut malformed).unwrap().pcm = vec![1; 1].into();
    assert!(matches!(
        ChromaprintExtractor.extract(&malformed, &token),
        Err(ResolverError::InvalidData)
    ));
    token.cancel();
    assert!(matches!(
        ChromaprintExtractor.extract(&input(4, false), &token),
        Err(ResolverError::Cancelled)
    ));
}
#[tokio::test]
async fn real_fingerprint_runs_on_owned_cpu_worker_and_keeps_identity() {
    let mut worker = FingerprintWorker::start(identity(), Arc::new(ChromaprintExtractor)).unwrap();
    assert_eq!(worker.worker_count(), 1);
    let client = worker.client();
    let audio = input(1, false);
    let expected = audio.stamp;
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        client.request(audio).unwrap().result(),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result.stamp.identity, expected.identity);
    assert_eq!(result.stamp.sequence, expected.sequence);
    assert!(result.stamp.start_us.is_none());
    assert_eq!(
        result.fingerprint.format(),
        FingerprintFormat::ChromaprintTest2Compressed
    );
    worker.stop().unwrap();
    worker.stop().unwrap();
    assert!(matches!(
        client.request(input(2, false)),
        Err(ResolverError::Cancelled)
    ));
}

struct SpyExtractor {
    calls: tokio::sync::mpsc::Sender<(u64, thread::ThreadId)>,
    active: Arc<AtomicUsize>,
    release: Option<Arc<AtomicBool>>,
    wait_cancel: bool,
    panic_first: bool,
}
struct ActiveGuard(Arc<AtomicUsize>);
impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
impl FingerprintExtractor for SpyExtractor {
    fn extract(
        &self,
        input: &AudioWindow,
        cancel: &ResolverCancellation,
    ) -> Result<Fingerprint, ResolverError> {
        assert_eq!(self.active.fetch_add(1, Ordering::SeqCst), 0);
        let _active = ActiveGuard(self.active.clone());
        let _ = self
            .calls
            .try_send((input.stamp.sequence, thread::current().id()));
        if self.panic_first && input.stamp.sequence == 1 {
            panic!("synthetic extractor panic");
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        if let Some(release) = &self.release {
            // Simulate one finite library call which checks cancellation only after
            // returning; this proves stale success is discarded at publication.
            while !release.load(Ordering::Acquire) && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(1));
            }
        }
        if self.wait_cancel {
            while !cancel.is_cancelled() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(1));
            }
            return Err(ResolverError::Cancelled);
        }
        Fingerprint::new(vec![input.stamp.sequence as u8].into_boxed_slice())
    }
}
struct ReleaseGuard(Arc<AtomicBool>);
impl Drop for ReleaseGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
fn spy(
    release: Option<Arc<AtomicBool>>,
    wait_cancel: bool,
    panic_first: bool,
) -> (
    Arc<SpyExtractor>,
    tokio::sync::mpsc::Receiver<(u64, thread::ThreadId)>,
) {
    let (calls, events) = tokio::sync::mpsc::channel(16);
    (
        Arc::new(SpyExtractor {
            calls,
            active: Arc::new(AtomicUsize::new(0)),
            release,
            wait_cancel,
            panic_first,
        }),
        events,
    )
}
async fn next_call(
    events: &mut tokio::sync::mpsc::Receiver<(u64, thread::ThreadId)>,
) -> (u64, thread::ThreadId) {
    tokio::time::timeout(Duration::from_secs(1), events.recv())
        .await
        .unwrap()
        .unwrap()
}
async fn wait_until(predicate: impl Fn() -> bool) {
    tokio::time::timeout(Duration::from_secs(1), async {
        while !predicate() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn saturation_releases_evicted_pcm_and_never_publishes_superseded_success() {
    let release = Arc::new(AtomicBool::new(false));
    let (extractor, mut events) = spy(Some(release.clone()), false, false);
    let mut worker = FingerprintWorker::start(identity(), extractor.clone()).unwrap();
    let _release_guard = ReleaseGuard(release.clone()); // unwind releases before joining worker
    let client = worker.client();
    let audio = blank_input(1);
    let first_weak = Arc::downgrade(&audio);
    let first = client.request(audio).unwrap();
    let (seq, cpu_thread) = next_call(&mut events).await;
    assert_eq!(seq, 1);
    assert_ne!(cpu_thread, thread::current().id());
    let mut jobs = Vec::new();
    let mut weak = Vec::new();
    for seq in 2..=50 {
        let audio = blank_input(seq);
        weak.push(Arc::downgrade(&audio));
        jobs.push(client.request(audio).unwrap());
    }
    assert!(first_weak.upgrade().is_some());
    assert_eq!(
        weak.iter().filter(|item| item.upgrade().is_some()).count(),
        1
    );
    assert_eq!(extractor.active.load(Ordering::SeqCst), 1);
    release.store(true, Ordering::Release);
    assert!(matches!(
        first.result().await,
        Err(ResolverError::Cancelled)
    ));
    let latest = jobs.pop().unwrap().result().await.unwrap();
    assert_eq!(latest.stamp.sequence, 50);
    assert_eq!(latest.fingerprint.bytes(), &[50]);
    for job in jobs {
        assert!(matches!(job.result().await, Err(ResolverError::Cancelled)));
    }
    worker.stop().unwrap();
    assert!(first_weak.upgrade().is_none());
    assert!(weak.iter().all(|item| item.upgrade().is_none()));
}
#[tokio::test]
async fn abandoned_future_and_owner_drop_cancel_and_join_all_cpu_work() {
    let (extractor, mut events) = spy(None, true, false);
    let worker = FingerprintWorker::start(identity(), extractor.clone()).unwrap();
    let client = worker.client();
    let audio = blank_input(1);
    let weak = Arc::downgrade(&audio);
    let job = client.request(audio).unwrap();
    next_call(&mut events).await;
    let mut future = Box::pin(job.result());
    assert!(futures_util::poll!(future.as_mut()).is_pending());
    drop(future);
    wait_until(|| weak.upgrade().is_none()).await;
    assert_eq!(extractor.active.load(Ordering::SeqCst), 0);
    let pending = client.request(blank_input(2)).unwrap();
    next_call(&mut events).await;
    drop(worker);
    assert!(matches!(
        pending.result().await,
        Err(ResolverError::Cancelled)
    ));
    assert_eq!(extractor.active.load(Ordering::SeqCst), 0);
    assert!(client.0.queue.is_empty());
    assert!(client.0.latest.lock().unwrap().completed.is_none());
}
#[tokio::test]
async fn completed_reply_backlog_retains_only_one_fingerprint_and_old_reply_is_stale() {
    let (extractor, _events) = spy(None, false, false);
    let mut worker = FingerprintWorker::start(identity(), extractor).unwrap();
    let client = worker.client();
    let mut jobs = Vec::new();
    for seq in 1..=10 {
        jobs.push(client.request(blank_input(seq)).unwrap());
        wait_until(|| client.0.latest.lock().unwrap().completed.is_some()).await;
        assert_eq!(
            client
                .0
                .latest
                .lock()
                .unwrap()
                .completed
                .as_ref()
                .unwrap()
                .bytes(),
            &[seq as u8]
        );
    }
    let latest = jobs.pop().unwrap().result().await.unwrap();
    assert_eq!(latest.stamp.sequence, 10);
    for job in jobs {
        assert!(matches!(job.result().await, Err(ResolverError::Stale)));
    }
    assert!(client.0.latest.lock().unwrap().completed.is_none());
    worker.stop().unwrap();
}
#[tokio::test]
async fn extractor_panic_is_typed_and_cpu_lane_remains_reclaimable() {
    let (extractor, _events) = spy(None, false, true);
    let mut worker = FingerprintWorker::start(identity(), extractor.clone()).unwrap();
    let client = worker.client();
    assert!(matches!(
        client.request(blank_input(1)).unwrap().result().await,
        Err(ResolverError::WorkerPanicked)
    ));
    let next = client
        .request(blank_input(2))
        .unwrap()
        .result()
        .await
        .unwrap();
    assert_eq!(next.stamp.sequence, 2);
    assert_eq!(extractor.active.load(Ordering::SeqCst), 0);
    worker.stop().unwrap();
}
#[tokio::test]
async fn stop_racing_submission_leaves_no_pcm_or_completed_output_after_join() {
    for _ in 0..20 {
        let (extractor, _events) = spy(None, false, false);
        let mut worker = FingerprintWorker::start(identity(), extractor).unwrap();
        let client = worker.client();
        let other = client.clone();
        let audio = blank_input(1);
        let weak = Arc::downgrade(&audio);
        let barrier = Arc::new(Barrier::new(2));
        let peer = barrier.clone();
        let submit = thread::spawn(move || {
            peer.wait();
            other.request(audio)
        });
        barrier.wait();
        worker.stop().unwrap();
        match submit.join().unwrap() {
            Ok(job) => assert!(matches!(job.result().await, Err(ResolverError::Cancelled))),
            Err(error) => assert_eq!(error, ResolverError::Cancelled),
        }
        assert!(weak.upgrade().is_none());
        assert!(client.0.queue.is_empty());
        assert!(client.0.latest.lock().unwrap().completed.is_none());
    }
}
#[test]
fn worker_budget_and_invalid_input_never_change_source_or_enqueue_bad_audio() {
    for cpus in [0, 1, 2, 64] {
        assert_eq!(worker_budget(cpus), 1);
    }
    let mut worker = FingerprintWorker::start(identity(), Arc::new(ChromaprintExtractor)).unwrap();
    let client = worker.client();
    let mut foreign = input(1, false);
    Arc::get_mut(&mut foreign).unwrap().stamp.identity.session = SessionId(42);
    assert!(matches!(
        client.request(foreign),
        Err(ResolverError::InvalidData)
    ));
    let mut bad = input(1, false);
    Arc::get_mut(&mut bad).unwrap().pcm = vec![0; 1].into();
    assert!(matches!(
        client.request(bad),
        Err(ResolverError::InvalidData)
    ));
    assert!(client.0.queue.is_empty());
    worker.stop().unwrap();
}

#[tokio::test]
async fn real_fingerprint_catalog_adapter_feeds_three_votes_through_resolver_core() {
    use crate::resolver::{candidates::SongState, catalog::*, session::*};
    struct MockCatalog(AtomicUsize);
    impl FingerprintCatalog for MockCatalog {
        fn capabilities(&self) -> CatalogCapabilities {
            CatalogCapabilities {
                format: FingerprintFormat::ChromaprintTest2Compressed,
                cancellation: true,
            }
        }
        fn lookup<'a>(
            &'a self,
            fingerprint: &'a Fingerprint,
            stamp: WindowStamp,
            _: ResolverCancellation,
        ) -> IdentifyFuture<'a> {
            Box::pin(async move {
                assert_eq!(
                    fingerprint.format(),
                    FingerprintFormat::ChromaprintTest2Compressed
                );
                assert_eq!(fingerprint.bytes()[0], 1);
                assert_eq!(stamp.identity, identity());
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(ContentEvidence {
                    kind: ContentType::Music,
                    confidence: Confidence::new(900).unwrap(),
                    songs: vec![SongCandidate {
                        id: "synthetic catalog ID".into(),
                        title: "synthetic test title".into(),
                        artist: None,
                        confidence: Confidence::new(900).unwrap(),
                        match_offset_us: None,
                    }],
                    movies: Vec::new(),
                })
            })
        }
    }
    let catalog = Arc::new(MockCatalog(AtomicUsize::new(0)));
    let identifier = Arc::new(CatalogIdentifier::new(identity(), catalog.clone()).unwrap());
    assert_eq!(identifier.worker_count(), 1);
    let (mut session, task) = ResolverSession::new(identity(), identifier.capabilities()).unwrap();
    let provider = identifier.clone();
    let run = tokio::spawn(async move { task.run(provider.as_ref()).await });
    for seq in 1..=3 {
        session.try_submit(input(seq, false)).unwrap();
        let update = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Some(update) = session.poll()
                    && update.stamp.sequence == seq
                    && update.status == ResolverStatus::Ready
                {
                    return update;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            update.resolution.song_state,
            if seq == 3 {
                SongState::Confirmed
            } else {
                SongState::Candidate
            }
        );
    }
    assert_eq!(catalog.0.load(Ordering::SeqCst), 3);
    session.cancellation().cancel();
    run.await.unwrap().unwrap();
    drop(identifier);
}

#[tokio::test]
async fn catalog_cancellation_drops_lookup_and_incompatible_format_is_rejected() {
    use crate::resolver::catalog::*;
    struct PendingCatalog {
        format: FingerprintFormat,
        entered: tokio::sync::mpsc::Sender<ResolverCancellation>,
        active: Arc<AtomicUsize>,
    }
    impl FingerprintCatalog for PendingCatalog {
        fn capabilities(&self) -> CatalogCapabilities {
            CatalogCapabilities {
                format: self.format,
                cancellation: true,
            }
        }
        fn lookup<'a>(
            &'a self,
            _: &'a Fingerprint,
            _: WindowStamp,
            cancel: ResolverCancellation,
        ) -> IdentifyFuture<'a> {
            Box::pin(async move {
                self.active.fetch_add(1, Ordering::SeqCst);
                let _active = ActiveGuard(self.active.clone());
                self.entered.try_send(cancel).unwrap();
                std::future::pending().await
            })
        }
    }
    let (sender, mut entered) = tokio::sync::mpsc::channel(1);
    let active = Arc::new(AtomicUsize::new(0));
    let bad = Arc::new(PendingCatalog {
        format: FingerprintFormat::Opaque,
        entered: sender.clone(),
        active: active.clone(),
    });
    assert!(matches!(
        CatalogIdentifier::new(identity(), bad),
        Err(ResolverError::UnsupportedCapability)
    ));
    let catalog = Arc::new(PendingCatalog {
        format: FingerprintFormat::ChromaprintTest2Compressed,
        entered: sender,
        active: active.clone(),
    });
    let identifier = Arc::new(CatalogIdentifier::new(identity(), catalog).unwrap());
    let cancel = ResolverCancellation::default();
    let token = cancel.clone();
    let provider = identifier.clone();
    let audio = input(1, false);
    let run = tokio::spawn(async move { provider.identify(&audio, token).await });
    let request = tokio::time::timeout(Duration::from_secs(2), entered.recv())
        .await
        .unwrap()
        .unwrap();
    cancel.cancel();
    assert!(matches!(run.await.unwrap(), Err(ResolverError::Cancelled)));
    assert!(request.is_cancelled());
    assert_eq!(active.load(Ordering::SeqCst), 0);
    drop(identifier);
}
