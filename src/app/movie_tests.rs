//! Production composition exercised with original synthetic dialogue/metadata.
//! No media capture, cloud service, credentials or persistent content is used.
use super::*;
use crate::{
    ai::speech::{SpeechLanguage, StreamIdentity, TranscriptEvent, TranscriptKind},
    app::content::{ContentConfiguration, ContentServiceFactory},
    audio::capture::SessionId,
    resolver::{
        contracts::{
            AudioWindow, Confidence, ContentIdentifier, ContentType, MAX_WINDOW_AGE, MovieSignal,
            ResolverCancellation, ResolverError, WindowStamp,
        },
        movie::{
            MovieCatalog, MovieCatalogCapabilities, MovieCatalogFuture, MovieCatalogReply,
            MovieQuery, MovieRecord,
        },
        movie_hints::MovieHintBuffer,
        window::WINDOW_FRAMES,
    },
    ui::content::{ContentDisclosure, ContentTransport},
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

const DIALOGUE: &str = "The synthetic lantern remains beside this empty doorway";
const MEDIA_RUNTIME_US: u64 = 100_000_000;
fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(1201),
        generation: 1,
    }
}
fn window(sequence: u64) -> AudioWindow {
    AudioWindow {
        stamp: WindowStamp {
            identity: identity(),
            sequence,
            ended_at: Instant::now(),
            start_us: Some(10_000_000),
            duration_us: 12_000_000,
        },
        pcm: vec![0_i16; WINDOW_FRAMES].into(),
    }
}
fn event(sequence: u64) -> TranscriptEvent {
    TranscriptEvent {
        identity: identity(),
        utterance: sequence,
        revision: 1,
        sequence,
        start_us: Some(12_000_000),
        duration_us: Some(2_000_000),
        language: Some(SpeechLanguage::English),
        kind: TranscriptKind::Final,
        text: DIALOGUE.into(),
    }
}
#[derive(Clone, Copy)]
enum MetadataChange {
    None,
    Foreign,
    Sequence,
    EndedAt,
    Start,
    Span,
    Filename,
    Runtime,
    Year,
}
struct Metadata {
    calls: AtomicUsize,
    change: MetadataChange,
}
impl Metadata {
    fn new(change: MetadataChange) -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            change,
        })
    }
}
impl MovieMetadataSource for Metadata {
    fn metadata(&self, stamp: WindowStamp) -> Result<MovieHintMetadata, ResolverError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let mut metadata = MovieHintMetadata {
            stamp,
            window_title: Some("Synthetic Lantern - Fixture Player".into()),
            filename: None,
            year: Some(2020),
            duration_us: Some(MEDIA_RUNTIME_US),
            validity: None,
        };
        match self.change {
            MetadataChange::None => {}
            MetadataChange::Foreign => metadata.stamp.identity.generation += 1,
            MetadataChange::Sequence => metadata.stamp.sequence += 1,
            MetadataChange::EndedAt => metadata.stamp.ended_at -= Duration::from_millis(1),
            MetadataChange::Start => metadata.stamp.start_us = Some(1),
            MetadataChange::Span => metadata.stamp.duration_us += 1,
            MetadataChange::Filename => metadata.filename = Some("private/path/movie.mkv".into()),
            MetadataChange::Runtime => metadata.duration_us = Some(0),
            MetadataChange::Year => metadata.year = Some(0),
        }
        Ok(metadata)
    }
}
struct Catalog {
    calls: AtomicUsize,
    active: Arc<AtomicUsize>,
    entered: tokio::sync::mpsc::Sender<ResolverCancellation>,
    delay: Duration,
    failure: Option<ResolverError>,
}
struct ActiveRequest(Arc<AtomicUsize>);
impl Drop for ActiveRequest {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
impl Catalog {
    fn new(
        delay: Duration,
        failure: Option<ResolverError>,
    ) -> (Arc<Self>, tokio::sync::mpsc::Receiver<ResolverCancellation>) {
        let (entered, receiver) = tokio::sync::mpsc::channel(4);
        (
            Arc::new(Self {
                calls: AtomicUsize::new(0),
                active: Arc::new(AtomicUsize::new(0)),
                entered,
                delay,
                failure,
            }),
            receiver,
        )
    }
}
impl MovieCatalog for Catalog {
    fn capabilities(&self) -> MovieCatalogCapabilities {
        MovieCatalogCapabilities {
            cancellation: true,
            dialogue_references: true,
        }
    }
    fn lookup<'a>(
        &'a self,
        query: &'a MovieQuery,
        cancel: ResolverCancellation,
    ) -> MovieCatalogFuture<'a> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.active.fetch_add(1, Ordering::SeqCst);
            let _active = ActiveRequest(Arc::clone(&self.active));
            let _ = self.entered.try_send(cancel);
            assert_eq!(query.dialogue.as_deref(), Some(DIALOGUE));
            assert_eq!(query.duration_us, Some(MEDIA_RUNTIME_US));
            assert_ne!(query.duration_us, Some(query.stamp.duration_us));
            tokio::time::sleep(self.delay).await;
            if let Some(error) = self.failure {
                return Err(error);
            }
            Ok(MovieCatalogReply {
                stamp: query.stamp,
                records: vec![MovieRecord {
                    id: "synthetic:lantern".into(),
                    title: "Synthetic Lantern".into(),
                    aliases: Vec::new(),
                    year: Some(2020),
                    language: Some(SpeechLanguage::English),
                    duration_us: Some(MEDIA_RUNTIME_US),
                    confidence: Confidence::new(876).unwrap(),
                    // Actual synthetic candidate evidence, not echoed query text.
                    dialogue_references: vec![
                        "Earlier fixture words. The synthetic lantern remains beside this empty doorway. Later fixture words."
                            .into(),
                    ],
                }],
            })
        })
    }
}
fn identifier(
    catalog: &Arc<Catalog>,
    hints: &Arc<MovieHintBuffer>,
    metadata: &Arc<Metadata>,
) -> MovieIdentifier {
    MovieIdentifier::new(identity(), catalog.clone(), hints.clone(), metadata.clone()).unwrap()
}
async fn entered(
    receiver: &mut tokio::sync::mpsc::Receiver<ResolverCancellation>,
) -> ResolverCancellation {
    tokio::time::timeout(Duration::from_secs(1), receiver.recv())
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn absent_partial_short_untimed_and_outside_dialogue_do_not_request_metadata_or_catalog() {
    for case in 0..7 {
        let hints = Arc::new(MovieHintBuffer::new(identity()));
        let metadata = Metadata::new(MetadataChange::None);
        let (catalog, _receiver) = Catalog::new(Duration::ZERO, None);
        if case != 0 {
            let mut final_event = event(1);
            match case {
                1 => final_event.kind = TranscriptKind::Partial,
                2 => final_event.text = "Too short".into(),
                3 => final_event.start_us = None,
                4 => final_event.duration_us = None,
                5 => final_event.start_us = Some(9_999_999),
                _ => final_event.start_us = Some(21_000_001),
            }
            let _ = hints.observe(&final_event);
        }
        let provider = identifier(&catalog, &hints, &metadata);
        let evidence = provider
            .identify(&window(1), ResolverCancellation::default())
            .await
            .unwrap();
        assert_eq!(evidence.kind, ContentType::Unknown);
        assert!(evidence.movies.is_empty());
        assert_eq!(metadata.calls.load(Ordering::SeqCst), 0);
        assert_eq!(catalog.calls.load(Ordering::SeqCst), 0);
    }
}
#[tokio::test]
async fn actual_final_dialogue_and_declared_metadata_preserve_catalog_confidence_and_signals() {
    let hints = Arc::new(MovieHintBuffer::new(identity()));
    assert!(hints.observe(&event(1)).unwrap());
    let metadata = Metadata::new(MetadataChange::None);
    let (catalog, _receiver) = Catalog::new(Duration::ZERO, None);
    let provider = identifier(&catalog, &hints, &metadata);
    let evidence = provider
        .identify(&window(1), ResolverCancellation::default())
        .await
        .unwrap();
    assert_eq!(evidence.kind, ContentType::MovieOrShow);
    assert_eq!(evidence.confidence.permille(), 876);
    assert!(evidence.songs.is_empty());
    assert_eq!(evidence.movies.len(), 1);
    assert_eq!(evidence.movies[0].id.as_ref(), "synthetic:lantern");
    assert_eq!(evidence.movies[0].confidence.permille(), 876);
    for signal in [
        MovieSignal::Dialogue,
        MovieSignal::WindowTitle,
        MovieSignal::RemoteMetadata,
        MovieSignal::Language,
        MovieSignal::Duration,
    ] {
        assert!(evidence.movies[0].signals.contains(&signal));
    }
    assert_eq!(metadata.calls.load(Ordering::SeqCst), 1);
    assert_eq!(catalog.calls.load(Ordering::SeqCst), 1);
    assert_eq!(catalog.active.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn metadata_must_match_every_query_stamp_component_before_catalog_receives_it() {
    for change in [
        MetadataChange::Foreign,
        MetadataChange::Sequence,
        MetadataChange::EndedAt,
        MetadataChange::Start,
        MetadataChange::Span,
    ] {
        let hints = Arc::new(MovieHintBuffer::new(identity()));
        hints.observe(&event(1)).unwrap();
        let metadata = Metadata::new(change);
        let (catalog, _receiver) = Catalog::new(Duration::ZERO, None);
        let provider = identifier(&catalog, &hints, &metadata);
        assert!(matches!(
            provider
                .identify(&window(1), ResolverCancellation::default())
                .await,
            Err(ResolverError::InvalidData)
        ));
        assert_eq!(metadata.calls.load(Ordering::SeqCst), 1);
        assert_eq!(catalog.calls.load(Ordering::SeqCst), 0);
    }
}
#[tokio::test]
async fn private_directory_paths_invalid_year_and_non_runtime_are_rejected_before_catalog() {
    for change in [
        MetadataChange::Filename,
        MetadataChange::Runtime,
        MetadataChange::Year,
    ] {
        let hints = Arc::new(MovieHintBuffer::new(identity()));
        hints.observe(&event(1)).unwrap();
        let metadata = Metadata::new(change);
        let (catalog, _receiver) = Catalog::new(Duration::ZERO, None);
        let provider = identifier(&catalog, &hints, &metadata);
        assert!(matches!(
            provider
                .identify(&window(1), ResolverCancellation::default())
                .await,
            Err(ResolverError::InvalidData)
        ));
        assert_eq!(catalog.calls.load(Ordering::SeqCst), 0);
    }
}
#[tokio::test]
async fn missing_window_timing_and_expired_or_foreign_windows_never_request_metadata() {
    for case in 0..3 {
        let hints = Arc::new(MovieHintBuffer::new(identity()));
        hints.observe(&event(1)).unwrap();
        let metadata = Metadata::new(MetadataChange::None);
        let (catalog, _receiver) = Catalog::new(Duration::ZERO, None);
        let provider = identifier(&catalog, &hints, &metadata);
        let mut audio = window(1);
        match case {
            0 => audio.stamp.start_us = None,
            1 => audio.stamp.ended_at -= MAX_WINDOW_AGE + Duration::from_secs(1),
            _ => audio.stamp.identity.generation += 1,
        }
        let result = provider
            .identify(&audio, ResolverCancellation::default())
            .await;
        match case {
            0 => assert_eq!(result.unwrap().kind, ContentType::Unknown),
            1 => assert!(matches!(result, Err(ResolverError::Stale))),
            _ => assert!(matches!(result, Err(ResolverError::InvalidData))),
        }
        assert_eq!(metadata.calls.load(Ordering::SeqCst), 0);
        assert_eq!(catalog.calls.load(Ordering::SeqCst), 0);
    }
}
#[tokio::test]
async fn newer_final_audio_gap_and_stop_cancel_inflight_catalog_without_orphans() {
    for action in 0..3 {
        let hints = Arc::new(MovieHintBuffer::new(identity()));
        hints.observe(&event(1)).unwrap();
        let metadata = Metadata::new(MetadataChange::None);
        let (catalog, mut receiver) = Catalog::new(Duration::from_secs(30), None);
        let provider = identifier(&catalog, &hints, &metadata);
        let audio = window(1);
        let caller = ResolverCancellation::default();
        let mut request = Box::pin(provider.identify(&audio, caller.clone()));
        let child = tokio::select! {
            _ = &mut request => panic!("lookup completed before cancellation"),
            child = entered(&mut receiver) => child,
        };
        match action {
            0 => {
                assert!(hints.observe(&event(2)).unwrap());
            }
            1 => hints.clear(),
            _ => hints.stop(),
        }
        let result = tokio::time::timeout(Duration::from_secs(1), request)
            .await
            .unwrap();
        assert!(matches!(
            result,
            Err(ResolverError::Stale | ResolverError::Cancelled)
        ));
        assert!(child.is_cancelled());
        assert!(!caller.is_cancelled());
        assert_eq!(catalog.active.load(Ordering::SeqCst), 0);
        assert_eq!(catalog.calls.load(Ordering::SeqCst), 1);
    }
}
#[tokio::test]
async fn dropping_identifier_future_releases_catalog_and_cancels_its_child_only() {
    let hints = Arc::new(MovieHintBuffer::new(identity()));
    hints.observe(&event(1)).unwrap();
    let metadata = Metadata::new(MetadataChange::None);
    let (catalog, mut receiver) = Catalog::new(Duration::from_secs(30), None);
    let provider = identifier(&catalog, &hints, &metadata);
    let audio = window(1);
    let caller = ResolverCancellation::default();
    let mut request = Box::pin(provider.identify(&audio, caller.clone()));
    let child = tokio::select! {
        _ = &mut request => panic!("lookup completed before Drop"),
        child = entered(&mut receiver) => child,
    };
    drop(request);
    assert!(child.is_cancelled());
    assert!(!caller.is_cancelled());
    assert_eq!(catalog.active.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn caller_cancellation_skips_admission_and_cancels_an_already_active_catalog() {
    let hints = Arc::new(MovieHintBuffer::new(identity()));
    hints.observe(&event(1)).unwrap();
    let metadata = Metadata::new(MetadataChange::None);
    let (catalog, mut receiver) = Catalog::new(Duration::from_secs(30), None);
    let provider = identifier(&catalog, &hints, &metadata);
    let cancelled = ResolverCancellation::default();
    cancelled.cancel();
    assert!(matches!(
        provider.identify(&window(1), cancelled).await,
        Err(ResolverError::Cancelled)
    ));
    assert_eq!(metadata.calls.load(Ordering::SeqCst), 0);
    assert_eq!(catalog.calls.load(Ordering::SeqCst), 0);
    let audio = window(2);
    let caller = ResolverCancellation::default();
    let mut request = Box::pin(provider.identify(&audio, caller.clone()));
    let child = tokio::select! {
        _ = &mut request => panic!("lookup completed before caller cancellation"),
        child = entered(&mut receiver) => child,
    };
    caller.cancel();
    assert!(matches!(request.await, Err(ResolverError::Cancelled)));
    assert!(child.is_cancelled());
    assert_eq!(catalog.active.load(Ordering::SeqCst), 0);
    assert!(hints.query(audio.stamp).unwrap().is_some());
}
#[tokio::test]
async fn original_capture_deadline_is_not_refreshed_by_hint_or_metadata_callbacks() {
    let hints = Arc::new(MovieHintBuffer::new(identity()));
    hints.observe(&event(1)).unwrap();
    let metadata = Metadata::new(MetadataChange::None);
    let (catalog, mut receiver) = Catalog::new(Duration::from_secs(30), None);
    let provider = identifier(&catalog, &hints, &metadata);
    let mut audio = window(1);
    audio.stamp.ended_at -= MAX_WINDOW_AGE - Duration::from_millis(200);
    assert!(matches!(
        tokio::time::timeout(
            Duration::from_secs(1),
            provider.identify(&audio, ResolverCancellation::default())
        )
        .await
        .unwrap(),
        Err(ResolverError::Stale)
    ));
    assert!(entered(&mut receiver).await.is_cancelled());
    assert_eq!(metadata.calls.load(Ordering::SeqCst), 1);
    assert_eq!(catalog.active.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn movie_lookup_timeout_cancels_remote_work_and_performs_no_retry() {
    let hints = Arc::new(MovieHintBuffer::new(identity()));
    hints.observe(&event(1)).unwrap();
    let metadata = Metadata::new(MetadataChange::None);
    let (catalog, mut receiver) = Catalog::new(Duration::from_secs(30), None);
    let provider = identifier(&catalog, &hints, &metadata);
    assert!(matches!(
        provider
            .identify(&window(1), ResolverCancellation::default())
            .await,
        Err(ResolverError::Timeout)
    ));
    assert!(entered(&mut receiver).await.is_cancelled());
    assert_eq!(catalog.calls.load(Ordering::SeqCst), 1);
    assert_eq!(catalog.active.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn catalog_errors_remain_typed_without_leaking_dialogue_or_triggering_fallback() {
    for error in [
        ResolverError::Credential,
        ResolverError::Network,
        ResolverError::RateLimited,
    ] {
        let hints = Arc::new(MovieHintBuffer::new(identity()));
        hints.observe(&event(1)).unwrap();
        let metadata = Metadata::new(MetadataChange::None);
        let (catalog, _receiver) = Catalog::new(Duration::ZERO, Some(error));
        let provider = identifier(&catalog, &hints, &metadata);
        assert!(matches!(
            provider.identify(&window(1), ResolverCancellation::default()).await,
            Err(actual) if actual == error
        ));
        assert_eq!(catalog.calls.load(Ordering::SeqCst), 1);
        assert_eq!(catalog.active.load(Ordering::SeqCst), 0);
        assert!(!error.to_string().contains(DIALOGUE));
    }
}
struct CatalogFactory {
    sessions: Mutex<Vec<StreamIdentity>>,
}
impl MovieCatalogFactory for CatalogFactory {
    fn create(&self, identity: StreamIdentity) -> Result<Arc<dyn MovieCatalog>, ResolverError> {
        self.sessions.lock().unwrap().push(identity);
        let (catalog, _receiver) = Catalog::new(Duration::ZERO, None);
        Ok(catalog)
    }
}
fn disclosure() -> ContentDisclosure {
    ContentDisclosure {
        service: "明確選用的合成電影目錄",
        transport: ContentTransport::Local,
    }
}
#[test]
fn movie_composition_enables_final_dialogue_but_generic_content_configuration_stays_opted_out() {
    let catalog = Arc::new(CatalogFactory {
        sessions: Mutex::new(Vec::new()),
    });
    let metadata = Metadata::new(MetadataChange::None);
    let configured = configuration(disclosure(), catalog.clone(), metadata.clone()).unwrap();
    assert!(configured.movie_dialogue_enabled());
    assert_eq!(configured.disclosure().service, disclosure().service);
    let factory = Arc::new(MovieFactory { catalog, metadata });
    let generic = ContentConfiguration::new(disclosure(), factory).unwrap();
    assert!(!generic.movie_dialogue_enabled());
    assert!(generic.with_movie_dialogue().movie_dialogue_enabled());
}
#[test]
fn movie_catalog_and_hint_owners_are_new_for_each_session() {
    let catalog = Arc::new(CatalogFactory {
        sessions: Mutex::new(Vec::new()),
    });
    let metadata = Metadata::new(MetadataChange::None);
    let factory = MovieFactory {
        catalog: catalog.clone(),
        metadata,
    };
    let first_id = identity();
    let second_id = StreamIdentity {
        generation: 2,
        ..first_id
    };
    let first = factory
        .create_with_movie_hints(first_id, Arc::new(MovieHintBuffer::new(first_id)))
        .unwrap();
    let second = factory
        .create_with_movie_hints(second_id, Arc::new(MovieHintBuffer::new(second_id)))
        .unwrap();
    assert!(!Arc::ptr_eq(&first, &second));
    assert!(first.capabilities().movies);
    assert!(!first.capabilities().songs);
    assert_eq!(*catalog.sessions.lock().unwrap(), vec![first_id, second_id]);
}

// Exercise the real source publication owner through MovieIdentifier. The
// catalog remains an explicit synthetic fixture and receives no captured PCM.
use crate::app::source_metadata::{MAX_SOURCE_METADATA_AGE, SourceMetadataBuffer};

const SOURCE_TITLE: &str = "Synthetic Lantern - Fixture Player";
struct SourceCatalog {
    calls: AtomicUsize,
    active: Arc<AtomicUsize>,
    entered: tokio::sync::mpsc::Sender<ResolverCancellation>,
    delay: Duration,
    before_reply: Option<Arc<dyn Fn() + Send + Sync>>,
}
impl SourceCatalog {
    fn new(delay: Duration) -> (Arc<Self>, tokio::sync::mpsc::Receiver<ResolverCancellation>) {
        let (entered, receiver) = tokio::sync::mpsc::channel(4);
        (
            Arc::new(Self {
                calls: AtomicUsize::new(0),
                active: Arc::new(AtomicUsize::new(0)),
                entered,
                delay,
                before_reply: None,
            }),
            receiver,
        )
    }
}
impl MovieCatalog for SourceCatalog {
    fn capabilities(&self) -> MovieCatalogCapabilities {
        MovieCatalogCapabilities {
            cancellation: true,
            dialogue_references: true,
        }
    }
    fn lookup<'a>(
        &'a self,
        query: &'a MovieQuery,
        cancel: ResolverCancellation,
    ) -> MovieCatalogFuture<'a> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.active.fetch_add(1, Ordering::SeqCst);
            let _active = ActiveRequest(Arc::clone(&self.active));
            let _ = self.entered.try_send(cancel);
            assert_eq!(query.window_title.as_deref(), Some(SOURCE_TITLE));
            assert_eq!(query.dialogue.as_deref(), Some(DIALOGUE));
            // A verified window caption proves neither a basename, production
            // year, nor whole-media runtime. None must remain None.
            assert!(query.filename.is_none());
            assert!(query.year.is_none());
            assert!(query.duration_us.is_none());
            tokio::time::sleep(self.delay).await;
            if let Some(callback) = &self.before_reply {
                callback();
            }
            Ok(MovieCatalogReply {
                stamp: query.stamp,
                records: vec![MovieRecord {
                    id: "synthetic:lantern".into(),
                    title: "Synthetic Lantern".into(),
                    aliases: Vec::new(),
                    year: Some(2020),
                    language: Some(SpeechLanguage::English),
                    duration_us: Some(MEDIA_RUNTIME_US),
                    confidence: Confidence::new(876).unwrap(),
                    dialogue_references: vec![
                        "Earlier fixture words. The synthetic lantern remains beside this empty doorway. Later fixture words."
                            .into(),
                    ],
                }],
            })
        })
    }
}
fn source_identifier(
    catalog: &Arc<SourceCatalog>,
    hints: &Arc<MovieHintBuffer>,
    source: &Arc<SourceMetadataBuffer>,
) -> MovieIdentifier {
    MovieIdentifier::new(identity(), catalog.clone(), hints.clone(), source.clone()).unwrap()
}
fn published_source() -> Arc<SourceMetadataBuffer> {
    let source = Arc::new(SourceMetadataBuffer::new(identity()));
    source
        .publish(1, Instant::now(), Ok(SOURCE_TITLE.into()))
        .unwrap();
    source
}

#[tokio::test]
async fn verified_source_title_and_current_final_preserve_actual_catalog_evidence() {
    let source = published_source();
    let hints = Arc::new(MovieHintBuffer::new(identity()));
    hints.observe(&event(1)).unwrap();
    let (catalog, _receiver) = SourceCatalog::new(Duration::ZERO);
    let provider = source_identifier(&catalog, &hints, &source);
    let evidence = provider
        .identify(&window(1), ResolverCancellation::default())
        .await
        .unwrap();
    assert_eq!(evidence.kind, ContentType::MovieOrShow);
    assert_eq!(evidence.confidence.permille(), 876);
    assert_eq!(evidence.movies.len(), 1);
    assert_eq!(evidence.movies[0].id.as_ref(), "synthetic:lantern");
    for signal in [
        MovieSignal::Dialogue,
        MovieSignal::WindowTitle,
        MovieSignal::RemoteMetadata,
        MovieSignal::Language,
    ] {
        assert!(evidence.movies[0].signals.contains(&signal));
    }
    assert!(!evidence.movies[0].signals.contains(&MovieSignal::Duration));
    assert!(!evidence.movies[0].signals.contains(&MovieSignal::Filename));
    assert_eq!(catalog.calls.load(Ordering::SeqCst), 1);
    assert_eq!(catalog.active.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn title_change_scan_failure_and_source_stop_cancel_catalog_child_without_orphans() {
    for action in 0..3 {
        let source = published_source();
        let hints = Arc::new(MovieHintBuffer::new(identity()));
        hints.observe(&event(1)).unwrap();
        let (catalog, mut receiver) = SourceCatalog::new(Duration::from_secs(30));
        let provider = source_identifier(&catalog, &hints, &source);
        let audio = window(1);
        let caller = ResolverCancellation::default();
        let mut request = Box::pin(provider.identify(&audio, caller.clone()));
        let child = tokio::select! {
            _ = &mut request => panic!("lookup completed before source invalidation"),
            child = entered(&mut receiver) => child,
        };
        match action {
            0 => source
                .publish(2, Instant::now(), Ok("Different Synthetic Feature".into()))
                .unwrap(),
            1 => {
                let _ = source.publish(
                    2,
                    Instant::now(),
                    Err(ResolverError::SourceMetadataUnavailable),
                );
            }
            _ => source.stop(),
        }
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(1), request)
                .await
                .unwrap(),
            Err(ResolverError::Stale | ResolverError::Cancelled)
        ));
        assert!(child.is_cancelled());
        assert!(!caller.is_cancelled());
        assert_eq!(catalog.calls.load(Ordering::SeqCst), 1);
        assert_eq!(catalog.active.load(Ordering::SeqCst), 0);
        // Invalidating source metadata does not cancel independent ASR hints.
        assert!(hints.query(audio.stamp).unwrap().is_some());
    }
}

#[tokio::test]
async fn unchanged_caption_refresh_cannot_extend_an_active_query_source_deadline() {
    let source = Arc::new(SourceMetadataBuffer::new(identity()));
    source
        .publish(
            1,
            Instant::now() - (MAX_SOURCE_METADATA_AGE - Duration::from_millis(150)),
            Ok(SOURCE_TITLE.into()),
        )
        .unwrap();
    let hints = Arc::new(MovieHintBuffer::new(identity()));
    hints.observe(&event(1)).unwrap();
    let (catalog, mut receiver) = SourceCatalog::new(Duration::from_secs(30));
    let provider = source_identifier(&catalog, &hints, &source);
    let audio = window(1);
    let caller = ResolverCancellation::default();
    let mut request = Box::pin(provider.identify(&audio, caller.clone()));
    let child = tokio::select! {
        _ = &mut request => panic!("lookup completed before identical scan refresh"),
        child = entered(&mut receiver) => child,
    };
    source
        .publish(2, Instant::now(), Ok(SOURCE_TITLE.into()))
        .unwrap();
    assert!(!child.is_cancelled());
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(1), request)
            .await
            .unwrap(),
        Err(ResolverError::Stale)
    ));
    assert!(child.is_cancelled());
    assert!(!caller.is_cancelled());
    assert_eq!(catalog.calls.load(Ordering::SeqCst), 1);
    assert_eq!(catalog.active.load(Ordering::SeqCst), 0);
    // The refreshed source can still serve a later independent query.
    assert!(source.metadata(window(2).stamp).is_ok());
}

#[tokio::test]
async fn expired_source_and_missing_final_hint_never_reach_movie_catalog() {
    for with_dialogue in [true, false] {
        let source = Arc::new(SourceMetadataBuffer::new(identity()));
        let observed = if with_dialogue {
            Instant::now() - MAX_SOURCE_METADATA_AGE - Duration::from_secs(1)
        } else {
            Instant::now()
        };
        let _ = source.publish(1, observed, Ok(SOURCE_TITLE.into()));
        let hints = Arc::new(MovieHintBuffer::new(identity()));
        if with_dialogue {
            hints.observe(&event(1)).unwrap();
        }
        let (catalog, _receiver) = SourceCatalog::new(Duration::ZERO);
        let provider = source_identifier(&catalog, &hints, &source);
        let result = provider
            .identify(&window(1), ResolverCancellation::default())
            .await;
        if with_dialogue {
            assert!(matches!(
                result,
                Err(ResolverError::Stale | ResolverError::SourceMetadataUnavailable)
            ));
        } else {
            assert_eq!(result.unwrap().kind, ContentType::Unknown);
        }
        assert_eq!(catalog.calls.load(Ordering::SeqCst), 0);
        assert_eq!(catalog.active.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn source_publication_from_previous_session_cannot_enter_new_catalog() {
    let source = published_source();
    let next_identity = StreamIdentity {
        generation: identity().generation + 1,
        ..identity()
    };
    let hints = Arc::new(MovieHintBuffer::new(next_identity));
    let mut next_event = event(1);
    next_event.identity = next_identity;
    hints.observe(&next_event).unwrap();
    let (catalog, _receiver) = SourceCatalog::new(Duration::ZERO);
    let provider = MovieIdentifier::new(next_identity, catalog.clone(), hints, source).unwrap();
    let mut audio = window(1);
    audio.stamp.identity = next_identity;
    assert!(matches!(
        provider
            .identify(&audio, ResolverCancellation::default())
            .await,
        Err(ResolverError::InvalidData)
    ));
    assert_eq!(catalog.calls.load(Ordering::SeqCst), 0);
    assert_eq!(catalog.active.load(Ordering::SeqCst), 0);
}

struct InvalidateMetadata {
    source: Arc<SourceMetadataBuffer>,
}
impl MovieMetadataSource for InvalidateMetadata {
    fn metadata(&self, stamp: WindowStamp) -> Result<MovieHintMetadata, ResolverError> {
        let metadata = self.source.metadata(stamp)?;
        metadata.validity.as_ref().unwrap().cancellation.cancel();
        Ok(metadata)
    }
}
#[tokio::test]
async fn metadata_invalidated_during_callback_is_rechecked_before_catalog_admission() {
    let source = published_source();
    let hints = Arc::new(MovieHintBuffer::new(identity()));
    hints.observe(&event(1)).unwrap();
    let (catalog, _receiver) = SourceCatalog::new(Duration::ZERO);
    let metadata = Arc::new(InvalidateMetadata { source });
    let provider = MovieIdentifier::new(identity(), catalog.clone(), hints, metadata).unwrap();
    assert!(matches!(
        provider
            .identify(&window(1), ResolverCancellation::default())
            .await,
        Err(ResolverError::Stale)
    ));
    assert_eq!(catalog.calls.load(Ordering::SeqCst), 0);
    assert_eq!(catalog.active.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn source_invalidated_inside_catalog_reply_cannot_publish_validated_evidence() {
    let source = published_source();
    let hints = Arc::new(MovieHintBuffer::new(identity()));
    hints.observe(&event(1)).unwrap();
    let (mut catalog, mut receiver) = SourceCatalog::new(Duration::ZERO);
    let retired = Arc::clone(&source);
    Arc::get_mut(&mut catalog).unwrap().before_reply = Some(Arc::new(move || retired.stop()));
    let provider = source_identifier(&catalog, &hints, &source);
    let caller = ResolverCancellation::default();
    assert!(matches!(
        provider.identify(&window(1), caller.clone()).await,
        Err(ResolverError::Stale)
    ));
    assert!(entered(&mut receiver).await.is_cancelled());
    assert!(!caller.is_cancelled());
    assert_eq!(catalog.calls.load(Ordering::SeqCst), 1);
    assert_eq!(catalog.active.load(Ordering::SeqCst), 0);
}
