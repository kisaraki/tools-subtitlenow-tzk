//! Original synthetic fixtures; no credentials, capture or remote traffic.
use super::*;
use crate::{audio::capture::SessionId, ui::content::ContentTransport};
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
use tokio::sync::mpsc;

#[derive(Clone, Copy)]
enum Mode {
    Text,
    Pending,
    Missing,
    Error,
    FactoryError,
    BindError,
    FactoryPanic,
    BindPanic,
    SupportsPanic,
    LookupPanic,
    FuturePanic,
}
struct Control {
    mode: Mode,
    factories: AtomicUsize,
    bindings: AtomicUsize,
    calls: AtomicUsize,
    active: AtomicUsize,
    started: mpsc::Sender<ResolverCancellation>,
}
struct Mock(Arc<Control>);
struct Active(Arc<Control>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
    }
}
impl LyricsServiceFactory for Mock {
    fn create(&self, _: StreamIdentity) -> Result<Arc<dyn LyricsSongBinder>, ResolverError> {
        self.0.factories.fetch_add(1, Ordering::SeqCst);
        match self.0.mode {
            Mode::FactoryError => Err(ResolverError::Network),
            Mode::FactoryPanic => panic!("synthetic factory panic"),
            _ => Ok(Arc::new(Mock(self.0.clone()))),
        }
    }
}
impl LyricsSongBinder for Mock {
    fn bind(&self, _: &Resolution) -> Result<Arc<dyn ExistingTextProvider>, ResolverError> {
        self.0.bindings.fetch_add(1, Ordering::SeqCst);
        match self.0.mode {
            Mode::BindError => Err(ResolverError::UnknownTextLanguage),
            Mode::BindPanic => panic!("synthetic binder panic"),
            _ => Ok(Arc::new(Mock(self.0.clone()))),
        }
    }
}
impl ExistingTextProvider for Mock {
    fn supports(&self, source: SubtitleSource) -> bool {
        if matches!(self.0.mode, Mode::SupportsPanic) {
            panic!("synthetic supports panic");
        }
        source == SubtitleSource::ExistingLyrics
    }
    fn lookup<'a>(&'a self, query: &'a TextQuery, cancel: ResolverCancellation) -> TextFuture<'a> {
        if matches!(self.0.mode, Mode::LookupPanic) {
            panic!("synthetic lookup panic");
        }
        Box::pin(async move {
            self.0.calls.fetch_add(1, Ordering::SeqCst);
            self.0.active.fetch_add(1, Ordering::SeqCst);
            let _active = Active(self.0.clone());
            let _ = self.0.started.try_send(cancel);
            match self.0.mode {
                Mode::Pending => std::future::pending().await,
                Mode::FuturePanic => panic!("synthetic future panic"),
                Mode::Missing => Ok(None),
                Mode::Error => Err(ResolverError::Network),
                _ => Ok(Some(ExistingText {
                    identity: query.stamp.identity,
                    content_id: query.content_id.clone(),
                    source: query.source,
                    language: TextLanguage::TaiwanTraditional,
                    cues: vec![TextCue {
                        start_us: Some(0),
                        duration_us: Some(1_000_000),
                        text: "原創合成歌詞測試".into(),
                    }],
                })),
            }
        })
    }
}
fn id() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(851),
        generation: 1,
    }
}
fn stamp(sequence: u64) -> WindowStamp {
    WindowStamp {
        identity: id(),
        sequence,
        ended_at: Instant::now(),
        start_us: None,
        duration_us: 12_000_000,
    }
}
fn song(name: &str) -> Resolution {
    Resolution {
        kind: ContentType::Music,
        song_state: SongState::Confirmed,
        song: Some(Arc::new(SongCandidate {
            id: name.into(),
            title: name.into(),
            artist: Some("synthetic artist".into()),
            confidence: Confidence::new(900).unwrap(),
            match_offset_us: None,
        })),
        movies: vec![],
    }
}
fn setup(
    mode: Mode,
) -> (
    LyricsConfiguration,
    Arc<Control>,
    mpsc::Receiver<ResolverCancellation>,
) {
    let (started, receiver) = mpsc::channel(16);
    let control = Arc::new(Control {
        mode,
        factories: AtomicUsize::new(0),
        bindings: AtomicUsize::new(0),
        calls: AtomicUsize::new(0),
        active: AtomicUsize::new(0),
        started,
    });
    let config = LyricsConfiguration::new(
        ContentDisclosure {
            service: "合成測試服務",
            transport: ContentTransport::Local,
        },
        Arc::new(Mock(control.clone())),
    )
    .unwrap();
    (config, control, receiver)
}
async fn changed(owner: &mut LiveLyrics) -> UiLyricsState {
    tokio::time::timeout(Duration::from_secs(5), owner.changed())
        .await
        .unwrap()
}
async fn enter(
    owner: &mut LiveLyrics,
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
async fn disabled_owner_stays_pending_and_never_claims_lookup() {
    let mut owner = LiveLyrics::new(id(), None);
    assert!(!owner.select(stamp(1), &song("a")).unwrap());
    assert_eq!(owner.status().state, UiLyricsState::Disabled);
    assert!(
        tokio::time::timeout(Duration::from_millis(20), owner.changed())
            .await
            .is_err()
    );
    owner.stop();
    assert_eq!(owner.status().state, UiLyricsState::Disabled);
}
#[tokio::test]
async fn same_binding_renews_without_refetch_and_changed_metadata_rebinds() {
    let (config, control, _) = setup(Mode::Text);
    let mut owner = LiveLyrics::new(id(), Some(&config));
    owner.select(stamp(1), &song("a")).unwrap();
    assert_eq!(changed(&mut owner).await, UiLyricsState::Available);
    // A changed confidence/offset is still the same metadata binding.
    let mut renewal = song("a");
    Arc::make_mut(renewal.song.as_mut().unwrap()).match_offset_us = Some(9_000_000);
    assert!(!owner.select(stamp(2), &renewal).unwrap());
    assert_eq!(control.calls.load(Ordering::SeqCst), 1);
    Arc::make_mut(renewal.song.as_mut().unwrap()).title = "different metadata".into();
    assert!(owner.select(stamp(3), &renewal).unwrap());
    assert_eq!(changed(&mut owner).await, UiLyricsState::Available);
    assert_eq!(control.bindings.load(Ordering::SeqCst), 2);
    assert_eq!(control.calls.load(Ordering::SeqCst), 2);
}
#[tokio::test]
async fn transition_releases_unread_candidate_without_rendering_it() {
    let (config, _, _) = setup(Mode::Text);
    let mut owner = LiveLyrics::new(id(), Some(&config));
    owner.select(stamp(1), &song("a")).unwrap();
    assert_eq!(changed(&mut owner).await, UiLyricsState::Available);
    let weak = Arc::downgrade(&owner.binding.as_ref().unwrap().song);
    let mut transition = song("a");
    transition.song_state = SongState::Transition;
    owner.select(stamp(2), &transition).unwrap();
    assert_eq!(owner.status().state, UiLyricsState::Waiting);
    assert!(weak.upgrade().is_none());
    assert!(
        owner
            .session
            .as_mut()
            .unwrap()
            .poll()
            .unwrap()
            .text
            .is_none()
    );
}
#[tokio::test]
async fn foreign_old_and_future_stamps_do_not_clear_a_valid_selection() {
    let (config, control, _) = setup(Mode::Text);
    let mut owner = LiveLyrics::new(id(), Some(&config));
    owner.select(stamp(2), &song("a")).unwrap();
    changed(&mut owner).await;
    for bad in [
        stamp(1),
        WindowStamp {
            identity: StreamIdentity {
                generation: 2,
                ..id()
            },
            ..stamp(3)
        },
        WindowStamp {
            ended_at: Instant::now() + Duration::from_secs(1),
            ..stamp(3)
        },
    ] {
        assert!(owner.select(bad, &Resolution::default()).is_err());
        assert_eq!(owner.status().state, UiLyricsState::Available);
    }
    assert_eq!(control.bindings.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn source_change_cancels_old_request_and_stop_drops_pending_future() {
    let (config, control, mut receiver) = setup(Mode::Pending);
    let mut owner = LiveLyrics::new(id(), Some(&config));
    owner.select(stamp(1), &song("a")).unwrap();
    let a = enter(&mut owner, &mut receiver).await;
    owner.select(stamp(2), &song("b")).unwrap();
    assert!(a.is_cancelled());
    let b = enter(&mut owner, &mut receiver).await;
    assert_eq!(control.active.load(Ordering::SeqCst), 1);
    owner.stop();
    assert!(b.is_cancelled());
    assert_eq!(control.active.load(Ordering::SeqCst), 0);
    assert_eq!(owner.status().state, UiLyricsState::Stopped);
}
#[tokio::test]
async fn drop_releases_pending_request_and_fresh_session_uses_new_factory() {
    let (config, control, mut receiver) = setup(Mode::Pending);
    let mut owner = LiveLyrics::new(id(), Some(&config));
    owner.select(stamp(1), &song("a")).unwrap();
    let token = enter(&mut owner, &mut receiver).await;
    drop(owner);
    assert!(token.is_cancelled());
    assert_eq!(control.active.load(Ordering::SeqCst), 0);
    let _next = LiveLyrics::new(
        StreamIdentity {
            generation: 2,
            ..id()
        },
        Some(&config),
    );
    assert_eq!(control.factories.load(Ordering::SeqCst), 2);
}
#[tokio::test]
async fn unavailable_and_network_failure_do_not_retry_each_window() {
    for (mode, expected) in [
        (Mode::Missing, UiLyricsState::Unavailable),
        (Mode::Error, UiLyricsState::Failed(ResolverError::Network)),
    ] {
        let (config, control, _) = setup(mode);
        let mut owner = LiveLyrics::new(id(), Some(&config));
        owner.select(stamp(1), &song("a")).unwrap();
        assert_eq!(changed(&mut owner).await, expected);
        assert!(!owner.select(stamp(2), &song("a")).unwrap());
        assert_eq!(control.calls.load(Ordering::SeqCst), 1);
        assert_eq!(owner.status().state, expected);
    }
}
#[test]
fn factory_and_binder_errors_are_typed_and_no_implicit_binding_retry() {
    for (mode, expected) in [
        (Mode::FactoryError, ResolverError::Network),
        (Mode::FactoryPanic, ResolverError::WorkerPanicked),
        (Mode::BindError, ResolverError::UnknownTextLanguage),
        (Mode::BindPanic, ResolverError::WorkerPanicked),
        (Mode::SupportsPanic, ResolverError::WorkerPanicked),
    ] {
        let (config, control, _) = setup(mode);
        let mut owner = LiveLyrics::new(id(), Some(&config));
        owner.select(stamp(1), &song("a")).unwrap();
        assert_eq!(owner.status().state, UiLyricsState::Failed(expected));
        if matches!(mode, Mode::BindError | Mode::BindPanic) {
            owner.select(stamp(2), &song("a")).unwrap();
            assert_eq!(control.bindings.load(Ordering::SeqCst), 1);
        }
        assert_eq!(control.calls.load(Ordering::SeqCst), 0);
    }
}
#[tokio::test]
async fn sync_and_async_provider_panics_release_owner_and_request() {
    for mode in [Mode::LookupPanic, Mode::FuturePanic] {
        let (config, control, _) = setup(mode);
        let mut owner = LiveLyrics::new(id(), Some(&config));
        owner.select(stamp(1), &song("a")).unwrap();
        assert_eq!(
            changed(&mut owner).await,
            UiLyricsState::Failed(ResolverError::WorkerPanicked)
        );
        assert_eq!(control.active.load(Ordering::SeqCst), 0);
        assert!(owner.session.is_none());
    }
}
#[tokio::test]
async fn timeout_releases_request_and_does_not_cancel_new_session() {
    let (config, control, mut receiver) = setup(Mode::Pending);
    let mut owner = LiveLyrics::new(id(), Some(&config));
    owner.select(stamp(1), &song("a")).unwrap();
    let token = enter(&mut owner, &mut receiver).await;
    assert_eq!(
        changed(&mut owner).await,
        UiLyricsState::Failed(ResolverError::Timeout)
    );
    assert!(token.is_cancelled());
    assert_eq!(control.active.load(Ordering::SeqCst), 0);
    assert!(!owner.select(stamp(2), &song("a")).unwrap());
}
#[test]
fn weak_confirmation_clears_selection_and_never_binds() {
    let (config, control, _) = setup(Mode::Text);
    let mut owner = LiveLyrics::new(id(), Some(&config));
    let mut weak = song("a");
    Arc::make_mut(weak.song.as_mut().unwrap()).confidence = Confidence::new(500).unwrap();
    owner.select(stamp(1), &weak).unwrap();
    assert_eq!(
        owner.status().state,
        UiLyricsState::Failed(ResolverError::InvalidData)
    );
    assert_eq!(control.bindings.load(Ordering::SeqCst), 0);
}
