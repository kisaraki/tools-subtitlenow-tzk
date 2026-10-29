//! Original synthetic metadata/dialogue and explicit mock catalogs only. No real
//! media, catalog, credentials, audio capture, network or content files.
use super::*;
use crate::{audio::capture::SessionId, resolver::candidates::CandidateTracker};
use std::sync::atomic::{AtomicUsize, Ordering};

const DIALOGUE: &str = "The synthetic lantern will remain beside this empty doorway";
fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(810),
        generation: 3,
    }
}
fn query(sequence: u64) -> MovieQuery {
    MovieQuery {
        stamp: WindowStamp {
            identity: identity(),
            sequence,
            ended_at: Instant::now(),
            start_us: None,
            duration_us: 12_000_000,
        },
        window_title: Some("Synthetic Lantern - Test Player".into()),
        filename: None,
        dialogue: Some(DIALOGUE.into()),
        language: Some(SpeechLanguage::English),
        year: Some(2020),
        duration_us: Some(100_000_000),
    }
}
fn record() -> MovieRecord {
    MovieRecord {
        id: "synthetic:movie:1".into(),
        title: "Synthetic Lantern".into(),
        aliases: Vec::new(),
        year: Some(2020),
        language: Some(SpeechLanguage::English),
        duration_us: Some(100_000_000),
        confidence: Confidence::new(850).unwrap(),
        dialogue_references: vec![
            format!("Earlier fixture words. {DIALOGUE}. Later fixture words.").into(),
        ],
    }
}
fn reply(query: &MovieQuery, records: Vec<MovieRecord>) -> MovieCatalogReply {
    MovieCatalogReply {
        stamp: query.stamp,
        records,
    }
}
fn verify(query: &MovieQuery, records: Vec<MovieRecord>) -> ContentEvidence {
    verify_reply(query, reply(query, records)).unwrap()
}
struct MockCatalog {
    capabilities: MovieCatalogCapabilities,
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
impl MovieCatalog for MockCatalog {
    fn capabilities(&self) -> MovieCatalogCapabilities {
        self.capabilities
    }
    fn lookup<'a>(
        &'a self,
        query: &'a MovieQuery,
        cancel: ResolverCancellation,
    ) -> MovieCatalogFuture<'a> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.active.fetch_add(1, Ordering::SeqCst);
            let _guard = ActiveRequest(Arc::clone(&self.active));
            self.entered.try_send(cancel).unwrap();
            tokio::time::sleep(self.delay).await;
            if let Some(error) = self.failure {
                return Err(error);
            }
            Ok(reply(query, vec![record()]))
        })
    }
}
fn mock(
    delay: Duration,
    failure: Option<ResolverError>,
) -> (
    Arc<MockCatalog>,
    tokio::sync::mpsc::Receiver<ResolverCancellation>,
) {
    let (entered, receiver) = tokio::sync::mpsc::channel(8);
    (
        Arc::new(MockCatalog {
            capabilities: MovieCatalogCapabilities {
                cancellation: true,
                dialogue_references: true,
            },
            calls: AtomicUsize::new(0),
            active: Arc::new(AtomicUsize::new(0)),
            entered,
            delay,
            failure,
        }),
        receiver,
    )
}
fn resolver(catalog: &Arc<MockCatalog>) -> MovieMetadataResolver {
    MovieMetadataResolver::new(identity(), catalog.clone()).unwrap()
}
async fn entered(
    receiver: &mut tokio::sync::mpsc::Receiver<ResolverCancellation>,
) -> ResolverCancellation {
    tokio::time::timeout(Duration::from_secs(5), receiver.recv())
        .await
        .unwrap()
        .unwrap()
}

#[test]
fn catalog_requires_cancellation_and_real_dialogue_references() {
    for (cancellation, dialogue_references) in [(false, true), (true, false), (false, false)] {
        let (mut catalog, _) = mock(Duration::ZERO, None);
        Arc::get_mut(&mut catalog).unwrap().capabilities = MovieCatalogCapabilities {
            cancellation,
            dialogue_references,
        };
        assert!(matches!(
            MovieMetadataResolver::new(identity(), catalog),
            Err(ResolverError::UnsupportedCapability)
        ));
    }
}

#[test]
fn query_bounds_reject_paths_controls_and_do_not_substitute_window_runtime() {
    for filename in [
        "C:\\private\\film.mkv",
        "dir/film.mkv",
        "https://private.test/movie",
        "..",
        ".",
        "",
        "bad\0name",
    ] {
        let mut q = query(1);
        q.filename = Some(filename.into());
        assert_eq!(
            q.validate(identity(), Instant::now()),
            Err(ResolverError::InvalidData)
        );
    }
    for field in 0..7 {
        let mut q = query(1);
        match field {
            0 => q.window_title = Some("x".repeat(MAX_METADATA_BYTES + 1).into()),
            1 => q.dialogue = Some("x".repeat(MAX_DIALOGUE_BYTES + 1).into()),
            2 => q.dialogue = Some("private\0text".into()),
            3 => q.year = Some(0),
            4 => q.year = Some(10000),
            5 => q.duration_us = Some(0),
            _ => q.duration_us = Some(MAX_MEDIA_DURATION_US + 1),
        }
        assert_eq!(
            q.validate(identity(), Instant::now()),
            Err(ResolverError::InvalidData)
        );
    }
    let mut q = query(1);
    q.filename = Some("Synthetic Lantern (2020).mkv".into());
    q.validate(identity(), Instant::now()).unwrap();
    q.duration_us = Some(q.stamp.duration_us); // A valid duration is still not silently substituted by the resolver.
    assert_eq!(verify(&q, vec![record()]).kind, ContentType::Unknown);
}

#[test]
fn independent_title_and_dialogue_preserve_supplied_confidence() {
    let q = query(1);
    let evidence = verify(&q, vec![record()]);
    assert_eq!(evidence.kind, ContentType::MovieOrShow);
    assert_eq!(evidence.confidence.permille(), 850);
    assert_eq!(evidence.movies[0].confidence.permille(), 850);
    assert!(evidence.songs.is_empty());
    assert_eq!(
        evidence.movies[0].signals,
        vec![
            MovieSignal::Dialogue,
            MovieSignal::RemoteMetadata,
            MovieSignal::WindowTitle,
            MovieSignal::Language,
            MovieSignal::Duration
        ]
    );
}

#[test]
fn filename_alias_case_punctuation_and_supported_languages() {
    for language in SpeechLanguage::ALL {
        let mut q = query(1);
        q.window_title = None;
        q.filename = Some("ALTERNATE_LANTERN.(2020).mkv".into());
        q.language = Some(language);
        let mut r = record();
        r.aliases = vec!["Alternate Lantern".into()];
        r.language = Some(language);
        let evidence = verify(&q, vec![r]);
        assert_eq!(evidence.kind, ContentType::MovieOrShow);
        assert!(evidence.movies[0].signals.contains(&MovieSignal::Filename));
        assert!(
            !evidence.movies[0]
                .signals
                .contains(&MovieSignal::WindowTitle)
        );
    }
    for text in [
        "架空電影的測試對白只用於驗證文字比對不代表實際辨識任何作品",
        "架空の作品の長いテスト台詞は実際の作品から引用されたものではありません",
        "가상작품의테스트대사는실제작품의인식결과가아닌검증용자료입니다",
        "ÉCLAIRAGE fictif près de la fenêtre synthétique",
        "Das künstliche Licht bleibt vor dieser leeren Tür",
    ] {
        let mut q = query(1);
        q.dialogue = Some(text.into());
        let mut r = record();
        r.dialogue_references = vec![text.to_lowercase().into()];
        assert!(q.eligible());
        assert_eq!(verify(&q, vec![r]).kind, ContentType::MovieOrShow);
    }
}

#[tokio::test]
async fn absent_or_short_independent_hints_do_not_query_catalog() {
    let (catalog, _) = mock(Duration::ZERO, None);
    let resolver = resolver(&catalog);
    for sequence in 1..=4 {
        let mut q = query(sequence);
        match sequence {
            1 => q.dialogue = None,
            2 => q.dialogue = Some("hello there".into()),
            3 => {
                q.window_title = None;
                q.filename = None;
            }
            _ => q.dialogue = Some("!".repeat(MIN_DIALOGUE_CHARACTERS).into()),
        }
        let evidence = resolver
            .resolve(&q, ResolverCancellation::default())
            .await
            .unwrap();
        assert_eq!(evidence.kind, ContentType::Unknown);
        assert_eq!(evidence.confidence.permille(), 0);
    }
    assert_eq!(catalog.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn repeated_title_metadata_and_substrings_never_replace_dialogue() {
    let mut q = query(1);
    q.filename = Some("Synthetic Lantern.mkv".into());
    let mut r = record();
    r.dialogue_references.clear();
    assert_eq!(verify(&q, vec![r]).kind, ContentType::Unknown);
    let mut r = record();
    r.dialogue_references =
        vec!["Unrelated synthetic dialogue about a different subject entirely".into()];
    assert_eq!(verify(&q, vec![r]).kind, ContentType::Unknown);
    q.window_title = Some("Synthetic LanternTwo - Test Player".into());
    q.filename = None;
    assert_eq!(verify(&q, vec![record()]).kind, ContentType::Unknown);
    q = query(1);
    let mut r = record();
    r.dialogue_references = vec![format!("x{DIALOGUE}x").into()];
    assert_eq!(verify(&q, vec![r]).kind, ContentType::Unknown);
    assert!(!phrase_contains("another title", ""));
}

#[test]
fn known_year_language_and_runtime_must_agree() {
    let q = query(1);
    for mismatch in 0..6 {
        let mut r = record();
        match mismatch {
            0 => r.year = Some(2021),
            1 => r.year = None,
            2 => r.language = Some(SpeechLanguage::Japanese),
            3 => r.language = None,
            4 => r.duration_us = Some(100_000_000 + MEDIA_DURATION_TOLERANCE_US + 1),
            _ => r.duration_us = None,
        }
        assert_eq!(verify(&q, vec![r]).kind, ContentType::Unknown);
    }
    let mut r = record();
    r.duration_us = Some(100_000_000 + MEDIA_DURATION_TOLERANCE_US);
    assert_eq!(verify(&q, vec![r]).kind, ContentType::MovieOrShow);
    let mut q = query(1);
    q.year = None;
    q.language = None;
    q.duration_us = None;
    let mut r = record();
    r.year = None;
    r.language = None;
    r.duration_us = None;
    let evidence = verify(&q, vec![r]);
    assert_eq!(evidence.movies[0].signals.len(), 3);
}

#[test]
fn bounded_ambiguous_candidates_keep_original_scores_and_deterministic_order() {
    let q = query(1);
    let mut records = Vec::new();
    for (id, score) in [("B", 850), ("weak", 699), ("A", 850), ("top", 900)] {
        let mut r = record();
        r.id = id.into();
        r.confidence = Confidence::new(score).unwrap();
        records.push(r);
    }
    let evidence = verify(&q, records);
    assert_eq!(evidence.confidence.permille(), 900);
    assert_eq!(
        evidence
            .movies
            .iter()
            .map(|r| r.id.as_ref())
            .collect::<Vec<_>>(),
        vec!["top", "A", "B"]
    );
    let mut r = record();
    r.confidence = Confidence::new(699).unwrap();
    assert_eq!(verify(&q, vec![r]).kind, ContentType::Unknown);
    // Ambiguity stays explicit: no single winner/subtitle timeline is invented.
    assert_eq!(evidence.movies.len(), 3);
}

#[test]
fn reply_stamp_duplicates_and_all_record_bounds_are_checked() {
    let q = query(1);
    for field in 0..6 {
        let mut response = reply(&q, vec![record()]);
        match field {
            0 => response.stamp.identity.session = SessionId(999),
            1 => response.stamp.identity.generation += 1,
            2 => response.stamp.sequence += 1,
            3 => response.stamp.ended_at -= Duration::from_millis(1),
            4 => response.stamp.start_us = Some(1),
            _ => response.stamp.duration_us += 1,
        }
        assert!(matches!(
            verify_reply(&q, response),
            Err(ResolverError::InvalidData)
        ));
    }
    assert!(matches!(
        verify_reply(&q, reply(&q, vec![record(), record()])),
        Err(ResolverError::InvalidData)
    ));
    assert!(matches!(
        verify_reply(&q, reply(&q, vec![record(); MAX_CANDIDATES + 1])),
        Err(ResolverError::InvalidData)
    ));
    for field in 0..10 {
        let mut r = record();
        match field {
            0 => r.id = "".into(),
            1 => r.title = "bad\0title".into(),
            2 => r.aliases = vec!["alias".into(); MAX_MOVIE_ALIASES + 1],
            3 => r.aliases = vec!["x".repeat(MAX_METADATA_BYTES + 1).into()],
            4 => r.dialogue_references = vec![DIALOGUE.into(); MAX_DIALOGUE_REFERENCES + 1],
            5 => r.dialogue_references = vec!["x".repeat(MAX_DIALOGUE_BYTES + 1).into()],
            6 => r.dialogue_references = vec!["bad\0reference".into()],
            7 => r.year = Some(0),
            8 => r.duration_us = Some(MAX_MEDIA_DURATION_US + 1),
            _ => r.duration_us = Some(0),
        }
        assert!(matches!(
            verify_reply(&q, reply(&q, vec![r])),
            Err(ResolverError::InvalidData)
        ));
    }
    assert_eq!(verify(&q, Vec::new()).kind, ContentType::Unknown);
}

#[tokio::test]
async fn old_sequence_foreign_session_and_pre_cancel_are_rejected_without_calls() {
    let (catalog, mut receiver) = mock(Duration::ZERO, None);
    let resolver = resolver(&catalog);
    let initial = query(10);
    resolver
        .resolve(&initial, ResolverCancellation::default())
        .await
        .unwrap();
    assert!(entered(&mut receiver).await.is_cancelled());
    for field in 0..8 {
        let mut q = query(11);
        let token = ResolverCancellation::default();
        let error = match field {
            0 => {
                q.stamp.sequence = 9;
                ResolverError::Stale
            }
            1 => {
                q.stamp.sequence = 10;
                ResolverError::Stale
            }
            2 => {
                q.stamp.identity.generation += 1;
                ResolverError::InvalidData
            }
            3 => {
                q.stamp.identity.session = SessionId(999);
                ResolverError::InvalidData
            }
            4 => {
                q.stamp.ended_at -= MAX_WINDOW_AGE + Duration::from_millis(1);
                ResolverError::Stale
            }
            5 => {
                q.stamp.ended_at += Duration::from_secs(1);
                ResolverError::Stale
            }
            6 => {
                q.stamp.ended_at = initial.stamp.ended_at - Duration::from_millis(1);
                ResolverError::Stale
            }
            _ => {
                token.cancel();
                ResolverError::Cancelled
            }
        };
        assert!(matches!(resolver.resolve(&q, token).await, Err(actual) if actual == error));
    }
    assert_eq!(catalog.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn concurrent_requests_reject_without_backlog_and_cancel_reclaims_permit() {
    let (catalog, mut receiver) = mock(Duration::from_secs(30), None);
    let resolver = resolver(&catalog);
    let q = query(1);
    let token = ResolverCancellation::default();
    let job = resolver.resolve(&q, token.clone());
    tokio::pin!(job);
    tokio::select! { _ = &mut job => panic!("unexpected completion"), child = entered(&mut receiver) => {
        assert!(!child.is_cancelled());
        assert!(matches!(resolver.resolve(&query(2), ResolverCancellation::default()).await, Err(ResolverError::RateLimited)));
        token.cancel();
        assert!(matches!(job.await, Err(ResolverError::Cancelled)));
        assert!(child.is_cancelled());
    }}
    assert_eq!(catalog.active.load(Ordering::SeqCst), 0);
    assert_eq!(catalog.calls.load(Ordering::SeqCst), 1);
    assert_eq!(resolver.requests.available_permits(), 1);
}

#[tokio::test]
async fn dropping_future_cancels_request_only_and_releases_borrowed_work() {
    let (catalog, mut receiver) = mock(Duration::from_secs(30), None);
    let resolver = resolver(&catalog);
    let q = query(1);
    let caller = ResolverCancellation::default();
    let mut job = Box::pin(resolver.resolve(&q, caller.clone()));
    let child = tokio::select! { _ = &mut job => panic!("unexpected completion"), child = entered(&mut receiver) => child };
    drop(job);
    assert!(child.is_cancelled());
    assert!(!caller.is_cancelled());
    assert_eq!(catalog.active.load(Ordering::SeqCst), 0);
    assert_eq!(resolver.requests.available_permits(), 1);
}

#[tokio::test]
async fn timeout_cancels_catalog_work_without_implicit_retry() {
    let (catalog, mut receiver) = mock(Duration::from_secs(30), None);
    let resolver = resolver(&catalog);
    let q = query(1);
    assert!(matches!(
        resolver.resolve(&q, ResolverCancellation::default()).await,
        Err(ResolverError::Timeout)
    ));
    assert!(entered(&mut receiver).await.is_cancelled());
    assert_eq!(catalog.active.load(Ordering::SeqCst), 0);
    assert_eq!(catalog.calls.load(Ordering::SeqCst), 1);
    assert_eq!(resolver.requests.available_permits(), 1);
}

#[tokio::test]
async fn result_that_ages_out_during_lookup_is_never_accepted() {
    let (catalog, mut receiver) = mock(Duration::from_millis(100), None);
    let resolver = resolver(&catalog);
    let mut q = query(1);
    q.stamp.ended_at -= MAX_WINDOW_AGE - Duration::from_millis(50);
    assert!(matches!(
        resolver.resolve(&q, ResolverCancellation::default()).await,
        Err(ResolverError::Stale)
    ));
    assert!(entered(&mut receiver).await.is_cancelled());
    assert_eq!(resolver.requests.available_permits(), 1);
}

#[tokio::test]
async fn failures_are_typed_and_next_window_retries_only_when_explicitly_called() {
    for error in [
        ResolverError::Network,
        ResolverError::Credential,
        ResolverError::InvalidData,
        ResolverError::RateLimited,
    ] {
        let (catalog, mut receiver) = mock(Duration::ZERO, Some(error));
        let resolver = resolver(&catalog);
        for sequence in 1..=2 {
            assert!(
                matches!(resolver.resolve(&query(sequence), ResolverCancellation::default()).await, Err(actual) if actual == error)
            );
            assert!(entered(&mut receiver).await.is_cancelled());
        }
        assert_eq!(catalog.calls.load(Ordering::SeqCst), 2);
        assert_eq!(catalog.active.load(Ordering::SeqCst), 0);
        assert!(!error.to_string().contains(DIALOGUE));
    }
}

#[tokio::test]
async fn verified_metadata_flows_through_existing_candidate_tracker_without_text_adoption() {
    let (catalog, _receiver) = mock(Duration::ZERO, None);
    let resolver = resolver(&catalog);
    let q = query(1);
    let evidence = resolver
        .resolve(&q, ResolverCancellation::default())
        .await
        .unwrap();
    let mut tracker = CandidateTracker::new(identity());
    let resolution = tracker.observe(q.stamp, evidence, Instant::now()).unwrap();
    assert_eq!(resolution.kind, ContentType::MovieOrShow);
    assert_eq!(resolution.movies[0].id.as_ref(), "synthetic:movie:1");
    assert!(resolution.song.is_none());
    let newer = StreamIdentity {
        session: SessionId(811),
        generation: 1,
    };
    let next = MovieMetadataResolver::new(newer, catalog.clone()).unwrap();
    assert!(matches!(
        next.resolve(&q, ResolverCancellation::default()).await,
        Err(ResolverError::InvalidData)
    ));
    assert_eq!(catalog.calls.load(Ordering::SeqCst), 1);
}
