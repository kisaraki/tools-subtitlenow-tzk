//! Synthetic Session metadata replacement; no network, media files or credentials.
use super::*;
use crate::{
    audio::capture::SessionId, resolver::candidates::SongState, ui::content::ContentTransport,
};
use std::{
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

struct Metadata {
    revision: Option<u64>,
    guard: ResolverCancellation,
    text: &'static str,
    missing: bool,
}
struct Control {
    metadata: Mutex<Metadata>,
    revision_panics: AtomicBool,
    guard_panics: AtomicBool,
    guard_calls: AtomicUsize,
    guard_panic_at: AtomicUsize,
    bindings: AtomicUsize,
    lookups: AtomicUsize,
}
struct Factory(Arc<Control>);
struct Binder(Arc<Control>);
struct Provider {
    control: Arc<Control>,
    guard: ResolverCancellation,
    text: &'static str,
}
impl SubtitleServiceFactory for Factory {
    fn create(&self, _: StreamIdentity) -> Result<Arc<dyn SubtitleMovieBinder>, ResolverError> {
        Ok(Arc::new(Binder(self.0.clone())))
    }
}
impl SubtitleMovieBinder for Binder {
    fn revision(&self, _: &MovieCandidate) -> Option<u64> {
        assert!(
            !self.0.revision_panics.load(Ordering::SeqCst),
            "synthetic revision panic"
        );
        self.0.metadata.lock().unwrap().revision
    }
    fn bind(&self, _: &MovieCandidate) -> Result<Arc<dyn ExistingTextProvider>, ResolverError> {
        self.0.bindings.fetch_add(1, Ordering::SeqCst);
        let metadata = self.0.metadata.lock().unwrap();
        if metadata.missing {
            return Err(ResolverError::NotConfigured);
        }
        Ok(Arc::new(Provider {
            control: self.0.clone(),
            guard: metadata.guard.clone(),
            text: metadata.text,
        }))
    }
}
impl ExistingTextProvider for Provider {
    fn supports(&self, source: SubtitleSource) -> bool {
        source == SubtitleSource::ExistingSubtitle
    }
    fn publication_guard(&self) -> Option<ResolverCancellation> {
        let call = self.control.guard_calls.fetch_add(1, Ordering::SeqCst) + 1;
        assert!(
            !self.control.guard_panics.load(Ordering::SeqCst),
            "synthetic publication guard panic"
        );
        assert_ne!(
            call,
            self.control.guard_panic_at.load(Ordering::SeqCst),
            "synthetic selection guard panic"
        );
        Some(self.guard.clone())
    }
    fn lookup<'a>(&'a self, query: &'a TextQuery, _: ResolverCancellation) -> TextFuture<'a> {
        Box::pin(async move {
            self.control.lookups.fetch_add(1, Ordering::SeqCst);
            Ok(Some(ExistingText {
                identity: query.stamp.identity,
                source: query.source,
                content_id: query.content_id.clone(),
                language: TextLanguage::TaiwanTraditional,
                cues: vec![TextCue {
                    start_us: Some(5_000_000),
                    duration_us: Some(1_000_000),
                    text: self.text.into(),
                }],
            }))
        })
    }
}
fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(1048),
        generation: 1,
    }
}
fn stamp(sequence: u64) -> WindowStamp {
    WindowStamp {
        identity: identity(),
        sequence,
        ended_at: Instant::now(),
        start_us: Some(40_000_000),
        duration_us: 12_000_000,
    }
}
fn resolution() -> Resolution {
    Resolution {
        kind: ContentType::MovieOrShow,
        song_state: SongState::Unknown,
        song: None,
        movies: vec![Arc::new(MovieCandidate {
            id: "synthetic scope movie".into(),
            title: "synthetic scope title".into(),
            year: Some(2026),
            confidence: Confidence::new(900).unwrap(),
            signals: vec![
                MovieSignal::Dialogue,
                MovieSignal::RemoteMetadata,
                MovieSignal::WindowTitle,
            ],
        })],
    }
}
fn setup(missing: bool) -> (LiveSubtitles, Arc<Control>) {
    let control = Arc::new(Control {
        metadata: Mutex::new(Metadata {
            revision: if missing { None } else { Some(1) },
            guard: ResolverCancellation::default(),
            text: "原創合成第一版字幕",
            missing,
        }),
        revision_panics: AtomicBool::new(false),
        guard_panics: AtomicBool::new(false),
        guard_calls: AtomicUsize::new(0),
        guard_panic_at: AtomicUsize::new(0),
        bindings: AtomicUsize::new(0),
        lookups: AtomicUsize::new(0),
    });
    let config = SubtitleConfiguration::new(
        ContentDisclosure {
            service: "合成 Session metadata 字幕測試",
            transport: ContentTransport::Local,
        },
        Arc::new(Factory(control.clone())),
    )
    .unwrap();
    (LiveSubtitles::new(identity(), Some(&config)), control)
}
async fn available(owner: &mut LiveSubtitles) -> ExistingTextLease {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let state = owner.changed().await;
            if state == UiSubtitleState::Available {
                return owner.ready_text().unwrap();
            }
            assert!(matches!(
                state,
                UiSubtitleState::LookingUp | UiSubtitleState::Waiting
            ));
        }
    })
    .await
    .unwrap()
}
fn replace(control: &Control, revision: u64) {
    let mut metadata = control.metadata.lock().unwrap();
    metadata.guard.cancel();
    metadata.revision = Some(revision);
    metadata.guard = ResolverCancellation::default();
    metadata.text = "原創合成第二版字幕";
    metadata.missing = false;
}

#[tokio::test]
async fn unchanged_metadata_keeps_provider_and_publication_without_redownload() {
    let (mut owner, control) = setup(false);
    assert!(owner.select(stamp(1), &resolution()).unwrap());
    let old = available(&mut owner).await;
    assert!(!owner.select(stamp(2), &resolution()).unwrap());
    assert!(!owner.select(stamp(3), &resolution()).unwrap());
    assert_eq!(
        owner.ready_text().unwrap().text().cues[0].text.as_ref(),
        "原創合成第一版字幕"
    );
    assert!(old.validate(identity()).is_ok());
    assert_eq!(control.bindings.load(Ordering::SeqCst), 1);
    assert_eq!(control.lookups.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn changed_metadata_retires_old_publication_before_loading_same_movie_again() {
    let (mut owner, control) = setup(false);
    owner.select(stamp(1), &resolution()).unwrap();
    let mut old = available(&mut owner).await;
    replace(&control, 2);
    assert!(owner.ready_text().is_none());
    assert!(matches!(
        old.validate(identity()),
        Err(ResolverError::Cancelled)
    ));
    assert!(owner.select(stamp(2), &resolution()).unwrap());
    assert!(owner.ready_text().is_none());
    assert!(matches!(
        old.renew(stamp(3), Confidence::new(900).unwrap()),
        Err(ResolverError::Cancelled)
    ));
    let current = available(&mut owner).await;
    assert_eq!(current.text().cues[0].text.as_ref(), "原創合成第二版字幕");
    assert!(matches!(
        old.validate(identity()),
        Err(ResolverError::Cancelled)
    ));
    assert_eq!(control.bindings.load(Ordering::SeqCst), 2);
    assert_eq!(control.lookups.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn newly_available_metadata_retries_a_cached_missing_binding_once() {
    let (mut owner, control) = setup(true);
    owner.select(stamp(1), &resolution()).unwrap();
    assert_eq!(
        owner.status().state,
        UiSubtitleState::Failed(ResolverError::NotConfigured)
    );
    owner.select(stamp(2), &resolution()).unwrap();
    assert_eq!(control.bindings.load(Ordering::SeqCst), 1);
    replace(&control, 1);
    assert!(owner.select(stamp(3), &resolution()).unwrap());
    available(&mut owner).await;
    owner.select(stamp(4), &resolution()).unwrap();
    assert_eq!(control.bindings.load(Ordering::SeqCst), 2);
    assert_eq!(control.lookups.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn cancelled_guard_forces_rebind_even_if_metadata_revision_is_unchanged() {
    let (mut owner, control) = setup(false);
    owner.select(stamp(1), &resolution()).unwrap();
    let old = available(&mut owner).await;
    replace(&control, 1);
    assert!(owner.select(stamp(2), &resolution()).unwrap());
    let current = available(&mut owner).await;
    assert!(matches!(
        old.validate(identity()),
        Err(ResolverError::Cancelled)
    ));
    assert_eq!(current.text().cues[0].text.as_ref(), "原創合成第二版字幕");
    assert_eq!(control.bindings.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn revision_callback_panic_retires_text_and_remains_recoverable() {
    let (mut owner, control) = setup(false);
    owner.select(stamp(1), &resolution()).unwrap();
    let old = available(&mut owner).await;
    control.revision_panics.store(true, Ordering::SeqCst);
    owner.select(stamp(2), &resolution()).unwrap();
    assert_eq!(
        owner.status().state,
        UiSubtitleState::Failed(ResolverError::WorkerPanicked)
    );
    assert!(owner.ready_text().is_none());
    assert!(matches!(
        old.validate(identity()),
        Err(ResolverError::Cancelled)
    ));
    control.revision_panics.store(false, Ordering::SeqCst);
    assert!(owner.select(stamp(3), &resolution()).unwrap());
    available(&mut owner).await;
    assert_eq!(control.bindings.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn publication_guard_callback_panic_is_cached_until_metadata_changes() {
    let (mut owner, control) = setup(false);
    control.guard_panics.store(true, Ordering::SeqCst);
    owner.select(stamp(1), &resolution()).unwrap();
    assert_eq!(
        owner.status().state,
        UiSubtitleState::Failed(ResolverError::WorkerPanicked)
    );
    owner.select(stamp(2), &resolution()).unwrap();
    assert_eq!(control.bindings.load(Ordering::SeqCst), 1);
    assert_eq!(control.lookups.load(Ordering::SeqCst), 0);
    control.guard_panics.store(false, Ordering::SeqCst);
    replace(&control, 2);
    assert!(owner.select(stamp(3), &resolution()).unwrap());
    available(&mut owner).await;
    assert_eq!(control.bindings.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn selection_guard_callback_panic_is_contained_after_the_binding_snapshot() {
    let (mut owner, control) = setup(false);
    control.guard_panic_at.store(2, Ordering::SeqCst);
    owner.select(stamp(1), &resolution()).unwrap();
    assert_eq!(
        owner.status().state,
        UiSubtitleState::Failed(ResolverError::WorkerPanicked)
    );
    assert_eq!(control.guard_calls.load(Ordering::SeqCst), 2);
    assert_eq!(control.lookups.load(Ordering::SeqCst), 0);
    owner.select(stamp(2), &resolution()).unwrap();
    assert_eq!(control.bindings.load(Ordering::SeqCst), 1);
    control.guard_panic_at.store(0, Ordering::SeqCst);
    replace(&control, 2);
    assert!(owner.select(stamp(3), &resolution()).unwrap());
    available(&mut owner).await;
}

#[tokio::test]
async fn changed_revision_alone_retires_the_previous_publication() {
    let (mut owner, control) = setup(false);
    owner.select(stamp(1), &resolution()).unwrap();
    let old = available(&mut owner).await;
    {
        let mut metadata = control.metadata.lock().unwrap();
        metadata.revision = Some(2);
        metadata.guard = ResolverCancellation::default();
        metadata.text = "原創合成第二版字幕";
    }
    assert!(old.validate(identity()).is_ok());
    owner.select(stamp(2), &resolution()).unwrap();
    assert!(matches!(
        old.validate(identity()),
        Err(ResolverError::Cancelled)
    ));
    available(&mut owner).await;
    assert_eq!(control.bindings.load(Ordering::SeqCst), 2);
    assert_eq!(control.lookups.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn already_cancelled_metadata_never_starts_lookup() {
    let (mut owner, control) = setup(false);
    control.metadata.lock().unwrap().guard.cancel();
    owner.select(stamp(1), &resolution()).unwrap();
    assert_eq!(
        owner.status().state,
        UiSubtitleState::Failed(ResolverError::Cancelled)
    );
    assert_eq!(control.lookups.load(Ordering::SeqCst), 0);
    assert!(owner.ready_text().is_none());
    replace(&control, 1);
    assert!(owner.select(stamp(2), &resolution()).unwrap());
    available(&mut owner).await;
    assert_eq!(control.bindings.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn metadata_disappearance_retires_text_without_repeated_missing_retry() {
    let (mut owner, control) = setup(false);
    owner.select(stamp(1), &resolution()).unwrap();
    let old = available(&mut owner).await;
    {
        let mut metadata = control.metadata.lock().unwrap();
        metadata.guard.cancel();
        metadata.revision = None;
        metadata.missing = true;
    }
    owner.select(stamp(2), &resolution()).unwrap();
    owner.select(stamp(3), &resolution()).unwrap();
    assert_eq!(
        owner.status().state,
        UiSubtitleState::Failed(ResolverError::NotConfigured)
    );
    assert!(matches!(
        old.validate(identity()),
        Err(ResolverError::Cancelled)
    ));
    assert_eq!(control.bindings.load(Ordering::SeqCst), 2);
    assert_eq!(control.lookups.load(Ordering::SeqCst), 1);
}
