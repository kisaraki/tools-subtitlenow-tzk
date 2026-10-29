//! Original bounded fixtures; no capture, real services, credentials or media files.
use super::*;
use crate::{
    ai::speech::{SpeechLanguage, TranscriptEvent, TranscriptKind},
    audio::capture::SessionId,
    domain::TranslationQuality,
    resolver::candidates::SongState,
    ui::content::ContentTransport,
};
use std::{
    sync::{
        Mutex, Weak,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::mpsc;

#[derive(Clone, Copy)]
enum Mode {
    Text,
    Paired,
    Pending,
    Missing,
    Error,
    FactoryError,
    BindError,
    FactoryPanic,
    BindPanic,
    SupportsPanic,
    Unsupported,
    LookupPanic,
    FuturePanic,
    ForeignText,
}
struct Control {
    mode: Mode,
    factories: AtomicUsize,
    bindings: AtomicUsize,
    supports: AtomicUsize,
    calls: AtomicUsize,
    active: AtomicUsize,
    last_bound: Mutex<Option<Box<str>>>,
    last_identity: Mutex<Option<StreamIdentity>>,
    provider_marker: Mutex<Option<Weak<()>>>,
    started: mpsc::Sender<ResolverCancellation>,
}
struct Mock(Arc<Control>);
struct MockProvider {
    control: Arc<Control>,
    _marker: Arc<()>,
}
struct Active(Arc<Control>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
    }
}
impl SubtitleServiceFactory for Mock {
    fn create(
        &self,
        identity: StreamIdentity,
    ) -> Result<Arc<dyn SubtitleMovieBinder>, ResolverError> {
        self.0.factories.fetch_add(1, Ordering::SeqCst);
        *self.0.last_identity.lock().unwrap() = Some(identity);
        match self.0.mode {
            Mode::FactoryError => Err(ResolverError::Network),
            Mode::FactoryPanic => panic!("synthetic factory panic"),
            _ => Ok(Arc::new(Self(self.0.clone()))),
        }
    }
}
impl SubtitleMovieBinder for Mock {
    fn bind(&self, movie: &MovieCandidate) -> Result<Arc<dyn ExistingTextProvider>, ResolverError> {
        self.0.bindings.fetch_add(1, Ordering::SeqCst);
        *self.0.last_bound.lock().unwrap() = Some(movie.id.clone());
        match self.0.mode {
            Mode::BindError => Err(ResolverError::Network),
            Mode::BindPanic => panic!("synthetic binder panic"),
            _ => {
                let marker = Arc::new(());
                *self.0.provider_marker.lock().unwrap() = Some(Arc::downgrade(&marker));
                Ok(Arc::new(MockProvider {
                    control: self.0.clone(),
                    _marker: marker,
                }))
            }
        }
    }
}
impl MockProvider {
    fn text(query: &TextQuery, language: TextLanguage, body: &str) -> ExistingText {
        ExistingText {
            identity: query.stamp.identity,
            content_id: query.content_id.clone(),
            source: query.source,
            language,
            cues: vec![TextCue {
                start_us: Some(5_000_000),
                duration_us: Some(1_000_000),
                text: body.into(),
            }],
        }
    }
    fn bundle<'a>(
        &'a self,
        query: &'a TextQuery,
        cancel: ResolverCancellation,
    ) -> TextBundleFuture<'a> {
        if matches!(self.control.mode, Mode::LookupPanic) {
            panic!("synthetic synchronous lookup panic");
        }
        Box::pin(async move {
            self.control.calls.fetch_add(1, Ordering::SeqCst);
            self.control.active.fetch_add(1, Ordering::SeqCst);
            let _active = Active(self.control.clone());
            let _ = self.control.started.try_send(cancel);
            match self.control.mode {
                Mode::Pending => std::future::pending().await,
                Mode::FuturePanic => panic!("synthetic asynchronous lookup panic"),
                Mode::Missing => Ok(None),
                Mode::Error => Err(ResolverError::Network),
                Mode::Paired => Ok(Some(ExistingTextBundle {
                    text: Self::text(query, TextLanguage::SimplifiedChinese, "软件与服务器字幕"),
                    source_reference: Some(SourceTextReference {
                        text: Self::text(
                            query,
                            TextLanguage::Source(SpeechLanguage::English),
                            "synthetic source one",
                        ),
                        primary_edition_id: "synthetic subtitle edition 1".into(),
                        reference_edition_id: "synthetic subtitle edition 1".into(),
                    }),
                })),
                _ => {
                    let mut text =
                        Self::text(query, TextLanguage::TaiwanTraditional, "原創合成電影字幕");
                    if matches!(self.control.mode, Mode::ForeignText) {
                        text.identity.generation += 1;
                    }
                    Ok(Some(ExistingTextBundle {
                        text,
                        source_reference: None,
                    }))
                }
            }
        })
    }
}
impl ExistingTextProvider for MockProvider {
    fn supports(&self, source: SubtitleSource) -> bool {
        self.control.supports.fetch_add(1, Ordering::SeqCst);
        if matches!(self.control.mode, Mode::SupportsPanic) {
            panic!("synthetic capability panic");
        }
        !matches!(self.control.mode, Mode::Unsupported)
            && source == SubtitleSource::ExistingSubtitle
    }
    fn lookup<'a>(&'a self, query: &'a TextQuery, cancel: ResolverCancellation) -> TextFuture<'a> {
        Box::pin(async move {
            self.bundle(query, cancel)
                .await
                .map(|item| item.map(|item| item.text))
        })
    }
    fn lookup_bundle<'a>(
        &'a self,
        query: &'a TextQuery,
        cancel: ResolverCancellation,
    ) -> TextBundleFuture<'a> {
        self.bundle(query, cancel)
    }
}
fn id() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(861),
        generation: 1,
    }
}
fn stamp(sequence: u64) -> WindowStamp {
    WindowStamp {
        identity: id(),
        sequence,
        ended_at: Instant::now(),
        start_us: Some(40_000_000),
        duration_us: 12_000_000,
    }
}
fn movie(name: &str, confidence: u16) -> Arc<MovieCandidate> {
    Arc::new(MovieCandidate {
        id: name.into(),
        title: name.into(),
        year: Some(2026),
        confidence: Confidence::new(confidence).unwrap(),
        signals: vec![
            MovieSignal::Dialogue,
            MovieSignal::RemoteMetadata,
            MovieSignal::Filename,
        ],
    })
}
fn resolution(movies: Vec<Arc<MovieCandidate>>) -> Resolution {
    Resolution {
        kind: ContentType::MovieOrShow,
        song_state: SongState::Unknown,
        song: None,
        movies,
    }
}
fn one(name: &str, confidence: u16) -> Resolution {
    resolution(vec![movie(name, confidence)])
}
fn setup(
    mode: Mode,
) -> (
    SubtitleConfiguration,
    Arc<Control>,
    mpsc::Receiver<ResolverCancellation>,
) {
    let (started, receiver) = mpsc::channel(16);
    let control = Arc::new(Control {
        mode,
        factories: AtomicUsize::new(0),
        bindings: AtomicUsize::new(0),
        supports: AtomicUsize::new(0),
        calls: AtomicUsize::new(0),
        active: AtomicUsize::new(0),
        last_bound: Mutex::new(None),
        last_identity: Mutex::new(None),
        provider_marker: Mutex::new(None),
        started,
    });
    let config = SubtitleConfiguration::new(
        ContentDisclosure {
            service: "合成字幕測試服務",
            transport: ContentTransport::Local,
        },
        Arc::new(Mock(control.clone())),
    )
    .unwrap();
    (config, control, receiver)
}
async fn changed(owner: &mut LiveSubtitles) -> UiSubtitleState {
    tokio::time::timeout(Duration::from_secs(5), owner.changed())
        .await
        .unwrap()
}
async fn enter(
    owner: &mut LiveSubtitles,
    receiver: &mut mpsc::Receiver<ResolverCancellation>,
) -> ResolverCancellation {
    tokio::time::timeout(Duration::from_secs(2), async {
        tokio::select! {
            token = receiver.recv() => token.unwrap(),
            state = owner.changed() => panic!("pending lookup unexpectedly completed: {state:?}"),
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn configuration_requires_disclosure_and_disabled_owner_never_claims_lookup() {
    let (_, control, _) = setup(Mode::Text);
    for service in ["", "   ", "invalid\nservice"] {
        assert!(matches!(
            SubtitleConfiguration::new(
                ContentDisclosure {
                    service,
                    transport: ContentTransport::Remote,
                },
                Arc::new(Mock(control.clone())),
            ),
            Err(ResolverError::InvalidData)
        ));
    }
    let mut owner = LiveSubtitles::new(id(), None);
    assert!(!owner.select(stamp(1), &one("a", 900)).unwrap());
    assert_eq!(owner.status().state, UiSubtitleState::Disabled);
    assert!(owner.ready_text().is_none());
    assert!(
        tokio::time::timeout(Duration::from_millis(20), owner.changed())
            .await
            .is_err()
    );
    owner.stop();
    assert_eq!(owner.status().state, UiSubtitleState::Disabled);
    assert_eq!(control.factories.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn selection_uses_actual_top_score_and_boundary_margin_independent_of_order() {
    for (scores, chosen) in [
        (vec![849], None),
        (vec![850], Some("a")),
        (vec![800, 900], Some("b")),
        (vec![801, 900], None),
        (vec![900, 900], None),
        (vec![750, 850, 600], Some("b")),
    ] {
        let (config, control, _) = setup(Mode::Text);
        let mut owner = LiveSubtitles::new(id(), Some(&config));
        let items = scores
            .into_iter()
            .enumerate()
            .map(|(index, confidence)| movie(["a", "b", "c"][index], confidence))
            .collect();
        owner.select(stamp(1), &resolution(items)).unwrap();
        if let Some(chosen) = chosen {
            assert_eq!(changed(&mut owner).await, UiSubtitleState::Available);
            assert_eq!(control.last_bound.lock().unwrap().as_deref(), Some(chosen));
            assert_eq!(
                owner.ready_text().unwrap().text().content_id.as_ref(),
                chosen
            );
        } else {
            assert_eq!(owner.status().state, UiSubtitleState::Waiting);
            assert_eq!(control.bindings.load(Ordering::SeqCst), 0);
        }
    }
}

#[test]
fn eligibility_requires_dialogue_remote_metadata_and_a_title_or_filename() {
    for signals in [
        vec![MovieSignal::Dialogue, MovieSignal::Filename],
        vec![MovieSignal::RemoteMetadata, MovieSignal::Filename],
        vec![MovieSignal::Dialogue, MovieSignal::RemoteMetadata],
        vec![
            MovieSignal::Process,
            MovieSignal::Language,
            MovieSignal::Duration,
        ],
    ] {
        let (config, control, _) = setup(Mode::Text);
        let mut owner = LiveSubtitles::new(id(), Some(&config));
        let mut candidate = movie("a", 950);
        Arc::make_mut(&mut candidate).signals = signals;
        owner
            .select(stamp(1), &resolution(vec![candidate]))
            .unwrap();
        assert_eq!(owner.status().state, UiSubtitleState::Waiting);
        assert_eq!(control.bindings.load(Ordering::SeqCst), 0);
    }
    let (config, control, _) = setup(Mode::Text);
    let mut owner = LiveSubtitles::new(id(), Some(&config));
    let mut candidate = movie("a", 900);
    Arc::make_mut(&mut candidate).signals[2] = MovieSignal::WindowTitle;
    assert!(
        owner
            .select(stamp(1), &resolution(vec![candidate]))
            .unwrap()
    );
    assert_eq!(control.bindings.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn malformed_candidates_retire_text_even_when_the_bad_entry_is_not_top() {
    let mut cases = vec![];
    for alteration in 0..8 {
        let mut candidate = movie("bad", 100);
        let bad = Arc::make_mut(&mut candidate);
        match alteration {
            0 => bad.id = " ".into(),
            1 => bad.title = "bad\ntitle".into(),
            2 => bad.id = "a".repeat(MAX_METADATA_BYTES + 1).into(),
            3 => bad.year = Some(0),
            4 => bad.year = Some(10_000),
            5 => bad.signals.push(MovieSignal::Dialogue),
            6 => bad.signals = vec![MovieSignal::Process; MAX_MOVIE_SIGNALS + 1],
            7 => bad.title = "".into(),
            _ => unreachable!(),
        }
        cases.push(resolution(vec![movie("good", 950), candidate]));
    }
    cases.push(resolution(vec![movie("same", 950), movie("same", 100)]));
    cases.push(resolution(
        (0..=MAX_CANDIDATES)
            .map(|index| movie(&format!("candidate {index}"), 950 - index as u16 * 10))
            .collect(),
    ));
    for malformed in cases {
        let (config, control, _) = setup(Mode::Text);
        let mut owner = LiveSubtitles::new(id(), Some(&config));
        owner.select(stamp(1), &one("incumbent", 900)).unwrap();
        changed(&mut owner).await;
        let old = owner.ready_text().unwrap();
        owner.select(stamp(2), &malformed).unwrap();
        assert_eq!(
            owner.status().state,
            UiSubtitleState::Failed(ResolverError::InvalidData)
        );
        assert!(owner.ready_text().is_none());
        assert_eq!(old.validate(id()), Err(ResolverError::Cancelled));
        assert_eq!(control.bindings.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn non_movie_weak_and_ambiguous_updates_retire_an_incumbent_to_waiting() {
    let mut cases = vec![Resolution::default(), one("weak", 849)];
    for kind in [ContentType::Speech, ContentType::Music, ContentType::Mixed] {
        let mut other = one("other", 950);
        other.kind = kind;
        if kind != ContentType::Mixed {
            other.movies.clear();
        }
        cases.push(other);
    }
    cases.push(resolution(vec![movie("a", 950), movie("b", 900)]));
    cases.push(resolution(vec![]));
    for replacement in cases {
        let (config, control, _) = setup(Mode::Text);
        let mut owner = LiveSubtitles::new(id(), Some(&config));
        owner.select(stamp(1), &one("incumbent", 900)).unwrap();
        changed(&mut owner).await;
        let old = owner.ready_text().unwrap();
        owner.select(stamp(2), &replacement).unwrap();
        assert_eq!(owner.status().state, UiSubtitleState::Waiting);
        assert_eq!(old.validate(id()), Err(ResolverError::Cancelled));
        assert!(owner.ready_text().is_none());
        assert_eq!(control.bindings.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn invalid_foreign_and_nonadvancing_stamps_preserve_a_valid_incumbent() {
    let (config, control, _) = setup(Mode::Text);
    let mut owner = LiveSubtitles::new(id(), Some(&config));
    let initial = stamp(20);
    owner.select(initial, &one("incumbent", 900)).unwrap();
    changed(&mut owner).await;
    let old = owner.ready_text().unwrap();
    for bad in [
        stamp(19),
        stamp(20),
        WindowStamp {
            identity: StreamIdentity {
                generation: 2,
                ..id()
            },
            ..stamp(21)
        },
        WindowStamp {
            identity: StreamIdentity {
                session: SessionId(862),
                ..id()
            },
            ..stamp(21)
        },
        WindowStamp {
            ended_at: Instant::now() + Duration::from_secs(1),
            ..stamp(21)
        },
        WindowStamp {
            ended_at: Instant::now() - Duration::from_secs(6),
            ..stamp(21)
        },
        WindowStamp {
            ended_at: initial.ended_at - Duration::from_micros(1),
            ..stamp(21)
        },
        WindowStamp {
            duration_us: 0,
            ..stamp(21)
        },
        WindowStamp {
            start_us: Some(u64::MAX),
            ..stamp(21)
        },
    ] {
        assert!(owner.select(bad, &Resolution::default()).is_err());
        assert_eq!(owner.status().state, UiSubtitleState::Available);
        assert!(old.validate(id()).is_ok());
        assert_eq!(owner.ready_text().unwrap().query().stamp.sequence, 20);
    }
    assert_eq!(control.bindings.load(Ordering::SeqCst), 1);
    assert!(!owner.select(stamp(21), &one("incumbent", 900)).unwrap());
}

#[tokio::test]
async fn unchanged_metadata_renews_the_lease_and_title_or_year_changes_rebind() {
    let (config, control, _) = setup(Mode::Text);
    let mut owner = LiveSubtitles::new(id(), Some(&config));
    owner.select(stamp(1), &one("a", 900)).unwrap();
    changed(&mut owner).await;
    let old = owner.ready_text().unwrap();
    let mut updated = one("a", 950);
    Arc::make_mut(&mut updated.movies[0])
        .signals
        .push(MovieSignal::Process);
    assert!(!owner.select(stamp(2), &updated).unwrap());
    let fresh = owner.ready_text().unwrap();
    assert_eq!(fresh.query().stamp.sequence, 2);
    assert_eq!(fresh.content_confidence().unwrap().permille(), 950);
    assert_eq!(old.query().stamp.sequence, 1);
    assert_eq!(old.content_confidence().unwrap().permille(), 900);
    assert!(old.validate(id()).is_ok());
    assert_eq!(control.bindings.load(Ordering::SeqCst), 1);
    assert_eq!(control.calls.load(Ordering::SeqCst), 1);
    Arc::make_mut(&mut updated.movies[0]).title = "different title".into();
    assert!(owner.select(stamp(3), &updated).unwrap());
    changed(&mut owner).await;
    assert_eq!(old.validate(id()), Err(ResolverError::Cancelled));
    Arc::make_mut(&mut updated.movies[0]).year = None;
    assert!(owner.select(stamp(4), &updated).unwrap());
    changed(&mut owner).await;
    assert_eq!(control.bindings.load(Ordering::SeqCst), 3);
    assert_eq!(control.calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn lower_confirmed_confidence_retires_old_evidence_without_refetch() {
    let (config, control, _) = setup(Mode::Paired);
    let mut owner = LiveSubtitles::new(id(), Some(&config));
    owner.select(stamp(1), &one("a", 950)).unwrap();
    changed(&mut owner).await;
    let old = owner.ready_text().unwrap();
    assert!(old.source_reference().is_some());
    assert!(!owner.select(stamp(2), &one("a", 850)).unwrap());
    assert_eq!(old.validate(id()), Err(ResolverError::Cancelled));
    let fresh = owner.ready_text().unwrap();
    assert!(fresh.validate(id()).is_ok());
    assert_eq!(fresh.content_confidence().unwrap().permille(), 850);
    assert!(fresh.source_reference().is_some());
    assert_eq!(control.calls.load(Ordering::SeqCst), 1);
    assert_eq!(control.bindings.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn clear_before_polling_removes_the_bound_provider_and_prevents_late_lookup() {
    let (config, control, _) = setup(Mode::Text);
    let mut owner = LiveSubtitles::new(id(), Some(&config));
    owner.select(stamp(1), &one("a", 900)).unwrap();
    let marker = control.provider_marker.lock().unwrap().clone().unwrap();
    assert!(marker.upgrade().is_some());
    assert!(owner.clear(None));
    assert!(marker.upgrade().is_none());
    assert!(owner.ready_text().is_none());
    assert_eq!(control.calls.load(Ordering::SeqCst), 0);
    assert!(
        tokio::time::timeout(Duration::from_millis(20), owner.changed())
            .await
            .is_err()
    );
    assert!(owner.select(stamp(2), &one("b", 900)).unwrap());
    assert_eq!(changed(&mut owner).await, UiSubtitleState::Available);
    assert_eq!(owner.ready_text().unwrap().text().content_id.as_ref(), "b");
    assert_eq!(control.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn latest_unpolled_selection_never_starts_superseded_lookup() {
    let (config, control, _) = setup(Mode::Text);
    let mut owner = LiveSubtitles::new(id(), Some(&config));
    owner.select(stamp(1), &one("old", 900)).unwrap();
    let old_marker = control.provider_marker.lock().unwrap().clone().unwrap();
    owner.select(stamp(2), &one("new", 900)).unwrap();
    assert!(old_marker.upgrade().is_none());
    assert_eq!(changed(&mut owner).await, UiSubtitleState::Available);
    assert_eq!(
        owner.ready_text().unwrap().text().content_id.as_ref(),
        "new"
    );
    assert_eq!(control.bindings.load(Ordering::SeqCst), 2);
    assert_eq!(control.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn source_change_and_stop_cancel_pending_requests_without_orphans() {
    let (config, control, mut receiver) = setup(Mode::Pending);
    let mut owner = LiveSubtitles::new(id(), Some(&config));
    owner.select(stamp(1), &one("a", 900)).unwrap();
    let old = enter(&mut owner, &mut receiver).await;
    owner.select(stamp(2), &one("b", 900)).unwrap();
    assert!(old.is_cancelled());
    let fresh = enter(&mut owner, &mut receiver).await;
    assert_eq!(control.active.load(Ordering::SeqCst), 1);
    owner.stop();
    assert!(fresh.is_cancelled());
    assert_eq!(control.active.load(Ordering::SeqCst), 0);
    assert_eq!(owner.status().state, UiSubtitleState::Stopped);
    assert!(!owner.select(stamp(3), &one("b", 900)).unwrap());
}

#[tokio::test]
async fn dropping_a_pending_owner_cancels_work_and_new_generation_gets_a_fresh_factory() {
    let (config, control, mut receiver) = setup(Mode::Pending);
    let mut owner = LiveSubtitles::new(id(), Some(&config));
    owner.select(stamp(1), &one("a", 900)).unwrap();
    let old = enter(&mut owner, &mut receiver).await;
    drop(owner);
    assert!(old.is_cancelled());
    assert_eq!(control.active.load(Ordering::SeqCst), 0);
    let next_id = StreamIdentity {
        generation: 2,
        ..id()
    };
    let mut next = LiveSubtitles::new(next_id, Some(&config));
    assert_eq!(*control.last_identity.lock().unwrap(), Some(next_id));
    assert_eq!(control.factories.load(Ordering::SeqCst), 2);
    let mut next_stamp = stamp(1);
    next_stamp.identity = next_id;
    next.select(next_stamp, &one("a", 900)).unwrap();
    let fresh = enter(&mut next, &mut receiver).await;
    assert!(!fresh.is_cancelled());
    assert!(next.select(stamp(2), &Resolution::default()).is_err());
    assert!(!fresh.is_cancelled());
    next.stop();
    assert!(fresh.is_cancelled());
}

#[tokio::test]
async fn unavailable_errors_and_foreign_responses_never_retry_unchanged_metadata() {
    for (mode, expected) in [
        (Mode::Missing, UiSubtitleState::Unavailable),
        (Mode::Error, UiSubtitleState::Failed(ResolverError::Network)),
        (
            Mode::ForeignText,
            UiSubtitleState::Failed(ResolverError::InvalidData),
        ),
    ] {
        let (config, control, _) = setup(mode);
        let mut owner = LiveSubtitles::new(id(), Some(&config));
        owner.select(stamp(1), &one("a", 900)).unwrap();
        assert_eq!(changed(&mut owner).await, expected);
        assert!(!owner.select(stamp(2), &one("a", 950)).unwrap());
        assert_eq!(owner.status().state, expected);
        assert_eq!(control.calls.load(Ordering::SeqCst), 1);
        assert_eq!(control.bindings.load(Ordering::SeqCst), 1);
        assert!(owner.ready_text().is_none());
        assert_eq!(control.active.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn factory_binding_and_capability_failures_are_typed_and_not_implicitly_retried() {
    for (mode, expected) in [
        (Mode::FactoryError, ResolverError::Network),
        (Mode::FactoryPanic, ResolverError::WorkerPanicked),
        (Mode::BindError, ResolverError::Network),
        (Mode::BindPanic, ResolverError::WorkerPanicked),
        (Mode::SupportsPanic, ResolverError::WorkerPanicked),
        (Mode::Unsupported, ResolverError::UnsupportedCapability),
    ] {
        let (config, control, _) = setup(mode);
        let mut owner = LiveSubtitles::new(id(), Some(&config));
        owner.select(stamp(1), &one("a", 900)).unwrap();
        assert_eq!(owner.status().state, UiSubtitleState::Failed(expected));
        owner.select(stamp(2), &one("a", 900)).unwrap();
        assert_eq!(owner.status().state, UiSubtitleState::Failed(expected));
        assert!(control.bindings.load(Ordering::SeqCst) <= 1);
        assert!(control.supports.load(Ordering::SeqCst) <= 1);
        assert_eq!(control.calls.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn synchronous_and_asynchronous_lookup_panics_drop_the_task_and_active_future() {
    for mode in [Mode::LookupPanic, Mode::FuturePanic] {
        let (config, control, _) = setup(mode);
        let mut owner = LiveSubtitles::new(id(), Some(&config));
        owner.select(stamp(1), &one("a", 900)).unwrap();
        assert_eq!(
            changed(&mut owner).await,
            UiSubtitleState::Failed(ResolverError::WorkerPanicked)
        );
        assert_eq!(control.active.load(Ordering::SeqCst), 0);
        assert!(!owner.select(stamp(2), &one("b", 900)).unwrap());
        assert_eq!(control.bindings.load(Ordering::SeqCst), 1);
        assert!(owner.ready_text().is_none());
    }
}

#[tokio::test]
async fn timeout_cancels_pending_work_without_retrying_each_window() {
    let (config, control, mut receiver) = setup(Mode::Pending);
    let mut owner = LiveSubtitles::new(id(), Some(&config));
    owner.select(stamp(1), &one("a", 900)).unwrap();
    let token = enter(&mut owner, &mut receiver).await;
    assert_eq!(
        changed(&mut owner).await,
        UiSubtitleState::Failed(ResolverError::Timeout)
    );
    assert!(token.is_cancelled());
    assert_eq!(control.active.load(Ordering::SeqCst), 0);
    assert!(!owner.select(stamp(2), &one("a", 900)).unwrap());
    assert_eq!(control.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn paired_existing_subtitle_uses_the_shared_timeline_and_clear_withdraws_adoption() {
    use crate::{
        ai::translation::{
            TranslationCancellation, TranslationCapabilities, TranslationEmitter,
            TranslationFuture, TranslationRequest, Translator,
        },
        session::{live_text::LiveTextPipeline, live_translation::LiveTranslation},
        subtitle::source_arbiter::PlaybackObservation,
    };
    struct NeverTranslate(AtomicUsize);
    impl Translator for NeverTranslate {
        fn capabilities(&self) -> TranslationCapabilities {
            TranslationCapabilities {
                streaming: true,
                cancellation: true,
                supported_languages: [true; 5],
                max_concurrent: 1,
            }
        }
        fn translate<'a>(
            &'a self,
            _: &'a TranslationRequest,
            _: &'a mut TranslationEmitter,
            _: TranslationCancellation,
        ) -> TranslationFuture<'a> {
            Box::pin(async move {
                self.0.fetch_add(1, Ordering::SeqCst);
                panic!("Chinese paired subtitle must not invoke cloud translation")
            })
        }
    }
    let (config, control, _) = setup(Mode::Paired);
    let mut subtitles = LiveSubtitles::new(id(), Some(&config));
    subtitles
        .select(stamp(1), &one("paired movie", 900))
        .unwrap();
    assert_eq!(changed(&mut subtitles).await, UiSubtitleState::Available);
    let input = subtitles.ready_text().unwrap();
    assert_eq!(input.text().source, SubtitleSource::ExistingSubtitle);
    let translator = NeverTranslate(AtomicUsize::new(0));
    let mut flow = LiveTranslation::new(id(), TranslationQuality::Standard, &translator).unwrap();
    let event = TranscriptEvent {
        identity: id(),
        sequence: 1,
        utterance: 1,
        revision: 1,
        kind: TranscriptKind::Final,
        language: Some(SpeechLanguage::English),
        start_us: Some(42_000_000),
        duration_us: Some(950_000),
        text: "synthetic source one".into(),
    };
    flow.ingest(event.clone()).unwrap();
    let (mut pipeline, task) = LiveTextPipeline::new(id(), TranslationQuality::Standard);
    assert!(
        pipeline
            .offer(
                Some(input.clone()),
                Some(&event),
                flow.sources_mut().version()
            )
            .unwrap()
    );
    let worker = task.isolated().run(&translator);
    tokio::pin!(worker);
    let done = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            tokio::select! {
                result = &mut worker => panic!("subtitle preparation unexpectedly ended: {result:?}"),
                done = pipeline.changed() => if let Some(done) = done { break done; },
            }
        }
    })
    .await
    .unwrap();
    let observation = PlaybackObservation {
        version: flow.sources_mut().version(),
        sequence: 100,
        observed_at: Instant::now(),
        capture_position_us: 42_500_000,
    };
    assert!(pipeline.apply(done, &mut flow, Some(observation)).unwrap());
    assert_eq!(flow.timeline().items().len(), 1);
    assert_eq!(
        flow.timeline().items()[0].display_origin(),
        SubtitleSource::ExistingSubtitle
    );
    assert_eq!(
        flow.timeline().items()[0].display_translation(),
        "軟體與伺服器字幕"
    );
    assert!(subtitles.clear(None));
    assert!(subtitles.ready_text().is_none());
    assert_eq!(input.validate(id()), Err(ResolverError::Cancelled));
    assert_eq!(
        flow.timeline().items()[0].display_origin(),
        SubtitleSource::LiveAi
    );
    assert_eq!(flow.reconcile_sources(), Some(event.utterance));
    assert_eq!(translator.0.load(Ordering::SeqCst), 0);
    assert_eq!(control.calls.load(Ordering::SeqCst), 1);
    pipeline.stop();
    worker.await.unwrap();
}
