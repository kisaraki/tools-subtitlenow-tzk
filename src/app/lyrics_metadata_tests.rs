//! Synthetic Session metadata lifecycle, with no network or real credentials.
use super::*;
use crate::{audio::capture::SessionId, ui::content::ContentTransport};
use std::{
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

struct Facts {
    revision: Option<u64>,
    available: bool,
    guard: ResolverCancellation,
    revision_panic: bool,
    guard_panic: bool,
}
struct Control {
    facts: Mutex<Facts>,
    factories: AtomicUsize,
    bindings: AtomicUsize,
    lookups: AtomicUsize,
}
struct Factory(Arc<Control>);
struct Provider {
    control: Arc<Control>,
    guard: ResolverCancellation,
    guard_panic: bool,
}
impl LyricsServiceFactory for Factory {
    fn create(&self, _: StreamIdentity) -> Result<Arc<dyn LyricsSongBinder>, ResolverError> {
        self.0.factories.fetch_add(1, Ordering::SeqCst);
        Ok(Arc::new(Factory(self.0.clone())))
    }
}
impl LyricsSongBinder for Factory {
    fn revision(&self, _: &Resolution) -> Option<u64> {
        let facts = self.0.facts.lock().unwrap();
        let (revision, panic) = (facts.revision, facts.revision_panic);
        drop(facts);
        assert!(!panic, "synthetic revision panic");
        revision
    }
    fn bind(&self, _: &Resolution) -> Result<Arc<dyn ExistingTextProvider>, ResolverError> {
        self.0.bindings.fetch_add(1, Ordering::SeqCst);
        let facts = self.0.facts.lock().unwrap();
        if !facts.available {
            return Err(ResolverError::UnknownTextLanguage);
        }
        Ok(Arc::new(Provider {
            control: self.0.clone(),
            guard: facts.guard.clone(),
            guard_panic: facts.guard_panic,
        }))
    }
}
impl ExistingTextProvider for Provider {
    fn supports(&self, source: SubtitleSource) -> bool {
        source == SubtitleSource::ExistingLyrics
    }
    fn publication_guard(&self) -> Option<ResolverCancellation> {
        assert!(!self.guard_panic, "synthetic publication guard panic");
        Some(self.guard.clone())
    }
    fn lookup<'a>(&'a self, query: &'a TextQuery, _: ResolverCancellation) -> TextFuture<'a> {
        Box::pin(async move {
            self.control.lookups.fetch_add(1, Ordering::SeqCst);
            Ok(Some(ExistingText {
                identity: query.stamp.identity,
                content_id: query.content_id.clone(),
                source: query.source,
                language: TextLanguage::TaiwanTraditional,
                cues: vec![TextCue {
                    start_us: Some(0),
                    duration_us: Some(1_000_000),
                    text: "原創 metadata 生命週期測試".into(),
                }],
            }))
        })
    }
}
fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(1401),
        generation: 1,
    }
}
fn stamp(sequence: u64) -> WindowStamp {
    WindowStamp {
        identity: identity(),
        sequence,
        ended_at: Instant::now(),
        start_us: None,
        duration_us: 12_000_000,
    }
}
fn song() -> Resolution {
    Resolution {
        kind: ContentType::Music,
        song_state: SongState::Confirmed,
        song: Some(Arc::new(SongCandidate {
            id: "synthetic metadata song".into(),
            title: "synthetic title".into(),
            artist: None,
            confidence: Confidence::new(900).unwrap(),
            match_offset_us: None,
        })),
        movies: vec![],
    }
}
fn setup(available: bool) -> (LiveLyrics, Arc<Control>) {
    let control = Arc::new(Control {
        facts: Mutex::new(Facts {
            revision: available.then_some(1),
            available,
            guard: ResolverCancellation::default(),
            revision_panic: false,
            guard_panic: false,
        }),
        factories: AtomicUsize::new(0),
        bindings: AtomicUsize::new(0),
        lookups: AtomicUsize::new(0),
    });
    let config = LyricsConfiguration::new(
        ContentDisclosure {
            service: "合成 metadata 測試",
            transport: ContentTransport::Local,
        },
        Arc::new(Factory(control.clone())),
    )
    .unwrap();
    (LiveLyrics::new(identity(), Some(&config)), control)
}
async fn available(owner: &mut LiveLyrics) -> ExistingTextLease {
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), owner.changed())
            .await
            .unwrap(),
        UiLyricsState::Available
    );
    owner.ready_text().unwrap()
}
#[tokio::test]
async fn metadata_registration_retries_missing_binding_once_then_renews() {
    let (mut owner, control) = setup(false);
    owner.select(stamp(1), &song()).unwrap();
    assert_eq!(
        owner.status().state,
        UiLyricsState::Failed(ResolverError::UnknownTextLanguage)
    );
    owner.select(stamp(2), &song()).unwrap();
    assert_eq!(control.bindings.load(Ordering::SeqCst), 1);
    {
        let mut facts = control.facts.lock().unwrap();
        facts.revision = Some(3);
        facts.available = true;
    }
    assert!(owner.select(stamp(3), &song()).unwrap());
    available(&mut owner).await;
    assert!(!owner.select(stamp(4), &song()).unwrap());
    assert_eq!(control.bindings.load(Ordering::SeqCst), 2);
    assert_eq!(control.lookups.load(Ordering::SeqCst), 1);
    assert_eq!(control.factories.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn changed_metadata_revision_retires_old_lease_without_restarting_factory() {
    let (mut owner, control) = setup(true);
    owner.select(stamp(1), &song()).unwrap();
    let old = available(&mut owner).await;
    let old_guard = control.facts.lock().unwrap().guard.clone();
    {
        let mut facts = control.facts.lock().unwrap();
        facts.revision = Some(2);
        facts.guard = ResolverCancellation::default();
    }
    assert!(owner.select(stamp(2), &song()).unwrap());
    assert_eq!(old.validate(identity()), Err(ResolverError::Cancelled));
    assert!(
        !old_guard.is_cancelled(),
        "publication retirement must not cancel its metadata registration"
    );
    let current = available(&mut owner).await;
    assert!(current.validate(identity()).is_ok());
    assert_eq!(control.bindings.load(Ordering::SeqCst), 2);
    assert_eq!(control.lookups.load(Ordering::SeqCst), 2);
    assert_eq!(control.factories.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn cancelled_metadata_guard_rebinds_even_when_revision_is_unchanged() {
    let (mut owner, control) = setup(true);
    owner.select(stamp(1), &song()).unwrap();
    let old = available(&mut owner).await;
    {
        let mut facts = control.facts.lock().unwrap();
        facts.guard.cancel();
        facts.guard = ResolverCancellation::default();
    }
    assert_eq!(old.validate(identity()), Err(ResolverError::Cancelled));
    assert!(owner.ready_text().is_none());
    assert!(owner.select(stamp(2), &song()).unwrap());
    available(&mut owner).await;
    assert_eq!(control.bindings.load(Ordering::SeqCst), 2);
    assert_eq!(control.lookups.load(Ordering::SeqCst), 2);
}
#[tokio::test]
async fn metadata_removal_retires_text_and_does_not_retry_each_confirmation() {
    let (mut owner, control) = setup(true);
    owner.select(stamp(1), &song()).unwrap();
    let old = available(&mut owner).await;
    {
        let mut facts = control.facts.lock().unwrap();
        facts.guard.cancel();
        facts.revision = None;
        facts.available = false;
    }
    owner.select(stamp(2), &song()).unwrap();
    owner.select(stamp(3), &song()).unwrap();
    assert_eq!(old.validate(identity()), Err(ResolverError::Cancelled));
    assert!(owner.ready_text().is_none());
    assert_eq!(
        owner.status().state,
        UiLyricsState::Failed(ResolverError::UnknownTextLanguage)
    );
    assert_eq!(control.bindings.load(Ordering::SeqCst), 2);
    assert_eq!(control.lookups.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn retiring_one_publication_preserves_sibling_metadata_publication() {
    let (mut owner, control) = setup(true);
    let config = LyricsConfiguration::new(
        ContentDisclosure {
            service: "合成 metadata 測試",
            transport: ContentTransport::Local,
        },
        Arc::new(Factory(control.clone())),
    )
    .unwrap();
    let mut sibling = LiveLyrics::new(identity(), Some(&config));
    owner.select(stamp(1), &song()).unwrap();
    sibling.select(stamp(1), &song()).unwrap();
    let retired = available(&mut owner).await;
    let valid = available(&mut sibling).await;
    owner.clear(None);
    assert_eq!(retired.validate(identity()), Err(ResolverError::Cancelled));
    assert!(valid.validate(identity()).is_ok());
    assert!(!control.facts.lock().unwrap().guard.is_cancelled());
}
#[tokio::test]
async fn revision_panic_is_typed_and_clears_existing_text() {
    let (mut owner, control) = setup(true);
    owner.select(stamp(1), &song()).unwrap();
    let old = available(&mut owner).await;
    control.facts.lock().unwrap().revision_panic = true;
    owner.select(stamp(2), &song()).unwrap();
    assert_eq!(
        owner.status().state,
        UiLyricsState::Failed(ResolverError::WorkerPanicked)
    );
    assert_eq!(old.validate(identity()), Err(ResolverError::Cancelled));
    assert!(owner.ready_text().is_none());
    assert_eq!(control.bindings.load(Ordering::SeqCst), 1);
}
#[test]
fn publication_guard_panic_is_cached_without_escaping_or_lookup() {
    let (mut owner, control) = setup(true);
    control.facts.lock().unwrap().guard_panic = true;
    owner.select(stamp(1), &song()).unwrap();
    owner.select(stamp(2), &song()).unwrap();
    assert_eq!(
        owner.status().state,
        UiLyricsState::Failed(ResolverError::WorkerPanicked)
    );
    assert_eq!(control.bindings.load(Ordering::SeqCst), 1);
    assert_eq!(control.lookups.load(Ordering::SeqCst), 0);
}
#[test]
fn stale_confirmations_do_not_call_panicking_metadata_reader() {
    let (mut owner, control) = setup(true);
    owner.select(stamp(2), &song()).unwrap();
    control.facts.lock().unwrap().revision_panic = true;
    assert_eq!(owner.select(stamp(1), &song()), Err(ResolverError::Stale));
    assert_eq!(owner.status().state, UiLyricsState::LookingUp);
    assert_eq!(control.bindings.load(Ordering::SeqCst), 1);
}
