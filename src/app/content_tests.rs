//! Explicit synthetic providers; no native capture, cloud, keys or real metadata.
use super::*;
use crate::{
    ai::{
        provider::SpeechProvider,
        speech::*,
        translation::{
            TranslationCancellation, TranslationCapabilities, TranslationEmitter,
            TranslationFuture, TranslationRequest, Translator,
        },
    },
    audio::capture::{AudioStamp, BLOCK_FRAMES, SessionId},
    domain::{SourceLanguage, TranslationQuality},
    resolver::{candidates::SongState, contracts::*},
    session::live_translation::LiveTranslation,
    ui::{
        bridge::{UiSource, UiStatus, channel},
        content::ContentTransport,
    },
};
use std::{
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

fn identity(id: u64) -> StreamIdentity {
    StreamIdentity {
        session: SessionId(id),
        generation: 1,
    }
}
fn block(id: StreamIdentity, sequence: u64) -> NormalizedAudio {
    NormalizedAudio {
        stamp: AudioStamp {
            session: id.session,
            sequence,
            frame_position: None,
            qpc_100ns: None,
            received: Instant::now(),
            discontinuity: false,
        },
        frames: BLOCK_FRAMES,
        samples: [0.2; BLOCK_FRAMES],
    }
}
#[derive(Default)]
struct Control {
    released: AtomicBool,
    panic: AtomicBool,
    active: AtomicUsize,
    calls: AtomicUsize,
    token: Mutex<Option<ResolverCancellation>>,
    notify: tokio::sync::Notify,
    hints: Mutex<Option<Arc<MovieHintBuffer>>>,
}
struct Active(Arc<Control>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
    }
}
struct MockIdentifier {
    id: StreamIdentity,
    control: Arc<Control>,
}
impl ContentIdentifier for MockIdentifier {
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
        window: &'a AudioWindow,
        cancel: ResolverCancellation,
    ) -> IdentifyFuture<'a> {
        Box::pin(async move {
            assert_eq!(window.stamp.identity, self.id);
            self.control.calls.fetch_add(1, Ordering::SeqCst);
            self.control.active.fetch_add(1, Ordering::SeqCst);
            let _active = Active(self.control.clone());
            *self.control.token.lock().unwrap() = Some(cancel.clone());
            while !self.control.released.load(Ordering::SeqCst) {
                tokio::select! {biased;_=cancel.cancelled()=>return Err(ResolverError::Cancelled),_=self.control.notify.notified()=>{}}
            }
            assert!(
                !self.control.panic.load(Ordering::SeqCst),
                "synthetic identifier panic"
            );
            Ok(ContentEvidence {
                kind: ContentType::Speech,
                confidence: Confidence::new(900).unwrap(),
                songs: vec![],
                movies: vec![],
            })
        })
    }
}
struct Factory {
    control: Arc<Control>,
    calls: AtomicUsize,
    fail: Option<ResolverError>,
    panic: bool,
}
impl ContentServiceFactory for Factory {
    fn create(&self, id: StreamIdentity) -> Result<Arc<dyn ContentIdentifier>, ResolverError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert!(!self.panic, "synthetic factory panic");
        if let Some(error) = self.fail {
            return Err(error);
        }
        Ok(Arc::new(MockIdentifier {
            id,
            control: self.control.clone(),
        }))
    }
    fn create_with_movie_hints(
        &self,
        id: StreamIdentity,
        hints: Arc<MovieHintBuffer>,
    ) -> Result<Arc<dyn ContentIdentifier>, ResolverError> {
        let provider = self.create(id)?;
        *self.control.hints.lock().unwrap() = Some(hints);
        Ok(provider)
    }
}
fn config(factory: Arc<Factory>) -> ContentConfiguration {
    ContentConfiguration::new(
        ContentDisclosure {
            service: "明確測試目錄",
            transport: ContentTransport::Local,
        },
        factory,
    )
    .unwrap()
}
fn factory() -> Arc<Factory> {
    Arc::new(Factory {
        control: Arc::new(Control::default()),
        calls: AtomicUsize::new(0),
        fail: None,
        panic: false,
    })
}
fn movie_event(id: StreamIdentity, sequence: u64) -> TranscriptEvent {
    TranscriptEvent {
        identity: id,
        utterance: sequence,
        revision: 1,
        sequence,
        start_us: Some(sequence * 1_000_000),
        duration_us: Some(1_000_000),
        language: Some(SpeechLanguage::English),
        kind: TranscriptKind::Final,
        text: "The current movie dialogue stays in session memory".into(),
    }
}
fn movie_stamp(id: StreamIdentity) -> WindowStamp {
    WindowStamp {
        identity: id,
        sequence: 10,
        ended_at: Instant::now(),
        start_us: Some(0),
        duration_us: 12_000_000,
    }
}
#[test]
fn movie_dialogue_is_explicit_and_rejected_events_preserve_current_hint() {
    let id = identity(820);
    let unconfigured = LiveContent::new(id, None);
    assert!(!unconfigured.movie_dialogue_enabled());
    assert!(unconfigured.movie_hints.is_none());
    let configured = config(factory());
    let mut ordinary = LiveContent::new(id, Some(&configured));
    assert!(!ordinary.observe_transcript(&movie_event(id, 1)));
    assert!(ordinary.movie_hints.is_none());
    let configured = configured.with_movie_dialogue();
    let mut owner = LiveContent::new(id, Some(&configured));
    assert!(owner.status().movie_dialogue);
    assert!(owner.observe_transcript(&movie_event(id, 2)));
    let current = owner
        .movie_hints
        .as_ref()
        .unwrap()
        .query(movie_stamp(id))
        .unwrap()
        .unwrap();
    let mut partial = movie_event(id, 3);
    partial.kind = TranscriptKind::Partial;
    assert!(!owner.observe_transcript(&partial));
    assert!(!owner.observe_transcript(&movie_event(identity(821), 3)));
    assert!(!owner.observe_transcript(&movie_event(id, 1)));
    assert!(!current.cancellation.is_cancelled());
    assert_eq!(owner.status().state, UiContentState::Collecting);
}
#[test]
fn an_audio_only_factory_cannot_silently_accept_movie_dialogue_configuration() {
    struct AudioOnly(Arc<Factory>);
    impl ContentServiceFactory for AudioOnly {
        fn create(&self, id: StreamIdentity) -> Result<Arc<dyn ContentIdentifier>, ResolverError> {
            self.0.create(id)
        }
    }
    let original = factory();
    let configured = ContentConfiguration::new(
        ContentDisclosure {
            service: "音訊測試服務",
            transport: ContentTransport::Local,
        },
        Arc::new(AudioOnly(original.clone())),
    )
    .unwrap()
    .with_movie_dialogue();
    let owner = LiveContent::new(identity(824), Some(&configured));
    assert_eq!(
        owner.status().state,
        UiContentState::Failed(ResolverError::UnsupportedCapability)
    );
    assert!(!owner.movie_dialogue_enabled());
    assert_eq!(original.calls.load(Ordering::SeqCst), 0);
}
#[test]
fn movie_dialogue_gap_stop_drop_and_reconnect_cancel_only_the_session_hint() {
    let id = identity(822);
    let configured = config(factory()).with_movie_dialogue();
    let mut owner = LiveContent::new(id, Some(&configured));
    assert!(owner.observe_transcript(&movie_event(id, 1)));
    let hints = owner.movie_hints.as_ref().unwrap().clone();
    let prior = hints.query(movie_stamp(id)).unwrap().unwrap();
    owner.reset_text_for_audio_gap();
    assert!(prior.cancellation.is_cancelled());
    assert!(hints.query(movie_stamp(id)).unwrap().is_none());
    assert!(!owner.observe_transcript(&movie_event(id, 1)));
    assert!(owner.observe_transcript(&movie_event(id, 2)));
    let second = hints.query(movie_stamp(id)).unwrap().unwrap();
    owner.stop();
    assert!(second.cancellation.is_cancelled());
    assert!(!owner.observe_transcript(&movie_event(id, 3)));
    assert!(matches!(
        hints.query(movie_stamp(id)),
        Err(ResolverError::Cancelled)
    ));
    let new_id = identity(823);
    let mut next = LiveContent::new(new_id, Some(&configured));
    assert!(!next.observe_transcript(&movie_event(id, 3)));
    assert!(next.observe_transcript(&movie_event(new_id, 1)));
    let snapshot = next
        .movie_hints
        .as_ref()
        .unwrap()
        .query(movie_stamp(new_id))
        .unwrap()
        .unwrap();
    drop(next);
    assert!(snapshot.cancellation.is_cancelled());
}
struct SourceBinding {
    delegate: Arc<Factory>,
    selected: Arc<Mutex<Vec<SelectedProcess>>>,
    failure: Option<ResolverError>,
    panic: bool,
}
impl ContentServiceFactory for SourceBinding {
    fn create(&self, id: StreamIdentity) -> Result<Arc<dyn ContentIdentifier>, ResolverError> {
        self.delegate.create(id)
    }
    fn bind_source(
        &self,
        selected: SelectedProcess,
    ) -> Result<Option<Arc<dyn ContentServiceFactory>>, ResolverError> {
        assert!(!self.panic, "synthetic binding panic");
        self.selected.lock().unwrap().push(selected);
        if let Some(error) = self.failure {
            return Err(error);
        }
        Ok(Some(self.delegate.clone()))
    }
}
#[test]
fn binding_preserves_configuration_and_uses_the_exact_selected_creation_identity() {
    let delegate = factory();
    let selected = Arc::new(Mutex::new(Vec::new()));
    let original = ContentConfiguration::new(
        ContentDisclosure {
            service: "明確來源測試",
            transport: ContentTransport::Local,
        },
        Arc::new(SourceBinding {
            delegate: delegate.clone(),
            selected: selected.clone(),
            failure: None,
            panic: false,
        }),
    )
    .unwrap()
    .with_movie_dialogue()
    .with_lyrics(lyrics_config());
    let source = SelectedProcess {
        pid: 42,
        created: Some(111),
    };
    let bound = original.for_selected_source(source);
    assert_eq!(selected.lock().unwrap().len(), 1);
    assert!(selected.lock().unwrap()[0] == source);
    assert_eq!(delegate.calls.load(Ordering::SeqCst), 0);
    assert!(bound.disclosure() == original.disclosure());
    assert!(bound.lyrics_disclosure() == original.lyrics_disclosure());
    assert!(bound.movie_dialogue_enabled());
    let owner = LiveContent::new(identity(825), Some(&bound));
    assert!(owner.movie_dialogue_enabled());
    assert_eq!(delegate.calls.load(Ordering::SeqCst), 1);
}
#[test]
fn unavailable_or_panicking_source_binding_disables_only_content_without_speech_failure() {
    for panic in [false, true] {
        let delegate = factory();
        let original = ContentConfiguration::new(
            ContentDisclosure {
                service: "來源失敗測試",
                transport: ContentTransport::Local,
            },
            Arc::new(SourceBinding {
                delegate: delegate.clone(),
                selected: Arc::new(Mutex::new(Vec::new())),
                failure: Some(ResolverError::SourceMetadataUnavailable),
                panic,
            }),
        )
        .unwrap()
        .with_movie_dialogue();
        let bound = original.for_selected_source(SelectedProcess {
            pid: 42,
            created: Some(111),
        });
        let id = identity(826);
        let owner = LiveContent::new(id, Some(&bound));
        let expected = if panic {
            ResolverError::WorkerPanicked
        } else {
            ResolverError::SourceMetadataUnavailable
        };
        assert_eq!(owner.status().state, UiContentState::Failed(expected));
        assert!(owner.status().disclosure == Some(original.disclosure()));
        assert!(owner.status().movie_dialogue);
        assert!(!owner.movie_dialogue_enabled());
        assert_eq!(delegate.calls.load(Ordering::SeqCst), 0);
        let (speech, _task) =
            SpeechSession::new(id, SourceLanguage::English, MockSpeech(id).capabilities()).unwrap();
        speech.input.try_push(block(id, 1)).unwrap();
        assert!(speech.cancellation().reason().is_none());
    }
}
struct MockSpeech(StreamIdentity);
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
                let block = audio.recv().await?;
                for (revision, kind) in [(1, TranscriptKind::Partial), (2, TranscriptKind::Final)] {
                    output.emit(TranscriptEvent {
                        identity: self.0,
                        sequence: block.stamp.sequence,
                        utterance: block.stamp.sequence + 1,
                        revision,
                        start_us: None,
                        duration_us: None,
                        language: Some(SpeechLanguage::English),
                        kind,
                        text: "original synthetic speech".into(),
                    })?;
                }
            }
        })
    }
}
struct MockTranslator;
impl Translator for MockTranslator {
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
        output: &'a mut TranslationEmitter,
        _: TranslationCancellation,
    ) -> TranslationFuture<'a> {
        Box::pin(async move {
            output.emit("測試暫定譯文")?;
            Ok("測試完成譯文".into())
        })
    }
}
async fn pump(owner: &mut LiveContent) {
    let _ = futures_util::poll!(Box::pin(owner.changed()));
}

struct MockLyrics;
impl crate::app::lyrics::LyricsServiceFactory for MockLyrics {
    fn create(
        &self,
        _: StreamIdentity,
    ) -> Result<Arc<dyn crate::app::lyrics::LyricsSongBinder>, ResolverError> {
        Ok(Arc::new(Self))
    }
}
impl crate::app::lyrics::LyricsSongBinder for MockLyrics {
    fn bind(
        &self,
        _: &crate::resolver::candidates::Resolution,
    ) -> Result<Arc<dyn ExistingTextProvider>, ResolverError> {
        Ok(Arc::new(Self))
    }
}
impl ExistingTextProvider for MockLyrics {
    fn supports(&self, source: SubtitleSource) -> bool {
        source == SubtitleSource::ExistingLyrics
    }
    fn lookup<'a>(&'a self, query: &'a TextQuery, _: ResolverCancellation) -> TextFuture<'a> {
        Box::pin(async move {
            Ok(Some(ExistingText {
                identity: query.stamp.identity,
                source: query.source,
                content_id: query.content_id.clone(),
                language: TextLanguage::TaiwanTraditional,
                cues: vec![TextCue {
                    start_us: Some(0),
                    duration_us: Some(1_000_000),
                    text: "原創合成候選歌詞".into(),
                }],
            }))
        })
    }
}
fn lyrics_config() -> LyricsConfiguration {
    LyricsConfiguration::new(
        ContentDisclosure {
            service: "測試歌詞",
            transport: ContentTransport::Local,
        },
        Arc::new(MockLyrics),
    )
    .unwrap()
}
fn confirmed_update(id: StreamIdentity) -> ResolverUpdate {
    ResolverUpdate::fixture(
        WindowStamp {
            identity: id,
            sequence: 1,
            ended_at: Instant::now(),
            start_us: None,
            duration_us: 12_000_000,
        },
        ResolverStatus::Ready,
        crate::resolver::candidates::Resolution {
            kind: ContentType::Music,
            song_state: SongState::Confirmed,
            movies: vec![],
            song: Some(Arc::new(SongCandidate {
                id: "synthetic".into(),
                title: "synthetic title".into(),
                artist: None,
                confidence: Confidence::new(900).unwrap(),
                match_offset_us: None,
            })),
        },
    )
}
#[tokio::test]
async fn live_owner_polls_lyrics_and_preserves_candidate_during_interim_identification() {
    use crate::ui::lyrics::UiLyricsState;
    let config = config(factory()).with_lyrics(lyrics_config());
    let mut owner = LiveContent::new(identity(871), Some(&config));
    let mut update = confirmed_update(identity(871));
    let state = owner.observe(&update).unwrap();
    owner.set_state(state);
    assert_eq!(owner.lyrics_status().state, UiLyricsState::LookingUp);
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), owner.changed())
            .await
            .unwrap(),
        state
    );
    assert_eq!(owner.lyrics_status().state, UiLyricsState::Available);
    let text_lease = owner.ready_lyrics().unwrap();
    update.stamp.sequence = 2;
    update.status = ResolverStatus::Resolving;
    update.resolution = Default::default();
    owner.observe(&update).unwrap();
    assert_eq!(owner.lyrics_status().state, UiLyricsState::Available);
    text_lease.validate(identity(871)).unwrap();
    update.status = ResolverStatus::Ready;
    owner.observe(&update).unwrap();
    assert_eq!(owner.lyrics_status().state, UiLyricsState::Waiting);
    assert!(owner.ready_lyrics().is_none());
    assert_eq!(
        text_lease.validate(identity(871)),
        Err(ResolverError::Cancelled)
    );
    owner.stop();
    assert_eq!(owner.lyrics_status().state, UiLyricsState::Stopped);
}
#[tokio::test]
async fn capture_clock_gap_retires_publication_before_any_new_resolver_reply() {
    use crate::ui::lyrics::UiLyricsState;
    let id = identity(880);
    let configuration = config(factory()).with_lyrics(lyrics_config());
    let mut owner = LiveContent::new(id, Some(&configuration));
    let update = confirmed_update(id);
    let state = owner.observe(&update).unwrap();
    owner.set_state(state);
    tokio::time::timeout(Duration::from_secs(2), owner.changed())
        .await
        .unwrap();
    let lease = owner.ready_lyrics().unwrap();
    assert!(owner.reset_text_for_audio_gap());
    assert!(owner.ready_lyrics().is_none());
    assert_eq!(lease.validate(id), Err(ResolverError::Cancelled));
    assert_eq!(owner.lyrics_status().state, UiLyricsState::Waiting);
    assert!(!owner.reset_text_for_audio_gap());
    owner.stop();
}
#[tokio::test]
async fn live_owner_gap_and_failure_clear_lyrics_without_changing_shared_timeline() {
    use crate::ui::lyrics::UiLyricsState;
    let id = identity(872);
    let config = config(factory()).with_lyrics(lyrics_config());
    let mut owner = LiveContent::new(id, Some(&config));
    let (mut publisher, mut inbox) = channel();
    let source = UiSource {
        pid: 42,
        provider: SpeechProvider::OpenAi,
    };
    publisher.prepare_source(source).unwrap();
    publisher
        .prepare_content(Some(config.disclosure()))
        .unwrap();
    publisher
        .prepare_lyrics(config.lyrics_disclosure())
        .unwrap();
    publisher.begin(id, source).unwrap();
    let timeline = crate::subtitle::timeline::TranscriptTimeline::new(id);
    publisher.publish(&timeline, true).unwrap();
    let snapshot = inbox.poll().unwrap().snapshot.as_ref().unwrap().clone();
    let mut update = confirmed_update(id);
    let state = owner.observe(&update).unwrap();
    owner.set_state(state);
    tokio::time::timeout(Duration::from_secs(2), owner.changed())
        .await
        .unwrap();
    publisher.publish_lyrics(id, owner.lyrics_status()).unwrap();
    assert!(Arc::ptr_eq(
        inbox.poll().unwrap().snapshot.as_ref().unwrap(),
        &snapshot
    ));
    update.status = ResolverStatus::Failed(ResolverError::AudioDiscontinuity);
    owner.observe(&update).unwrap();
    assert_eq!(owner.lyrics_status().state, UiLyricsState::Waiting);
    update.status = ResolverStatus::Failed(ResolverError::Network);
    owner.observe(&update).unwrap();
    assert_eq!(
        owner.lyrics_status().state,
        UiLyricsState::Failed(ResolverError::Network)
    );
    assert!(owner.metrics().is_some());
}

#[tokio::test]
async fn unconfigured_owner_has_no_cpu_work_and_never_produces_ready_notifications() {
    let mut owner = LiveContent::new(identity(1), None);
    let (speech, _task) = SpeechSession::new(
        identity(1),
        SourceLanguage::English,
        MockSpeech(identity(1)).capabilities(),
    )
    .unwrap();
    assert_eq!(owner.status().state, UiContentState::Disabled);
    assert!(owner.metrics().is_none());
    assert!(
        !owner
            .submit_audio(&speech.input, block(identity(1), 0))
            .unwrap()
    );
    assert_eq!(speech.input.queue_depth(), 1);
    assert!(
        tokio::time::timeout(Duration::from_millis(5), owner.changed())
            .await
            .is_err()
    );
    owner.stop();
    assert_eq!(owner.status().state, UiContentState::Disabled);
}
#[tokio::test]
async fn factory_failure_and_panic_do_not_cancel_speech_or_create_retry_loops() {
    for panic in [false, true] {
        let factory = Arc::new(Factory {
            control: Arc::new(Control::default()),
            calls: AtomicUsize::new(0),
            fail: Some(ResolverError::Credential),
            panic,
        });
        let config = config(factory.clone());
        let mut owner = LiveContent::new(identity(1), Some(&config));
        assert_eq!(
            owner.status().state,
            UiContentState::Failed(if panic {
                ResolverError::WorkerPanicked
            } else {
                ResolverError::Credential
            })
        );
        let (speech, _task) = SpeechSession::new(
            identity(1),
            SourceLanguage::English,
            MockSpeech(identity(1)).capabilities(),
        )
        .unwrap();
        assert!(
            !owner
                .submit_audio(&speech.input, block(identity(1), 0))
                .unwrap()
        );
        assert!(speech.cancellation().reason().is_none());
        assert!(owner.metrics().is_none());
        assert!(
            tokio::time::timeout(Duration::from_millis(5), owner.changed())
                .await
                .is_err()
        );
        assert_eq!(factory.calls.load(Ordering::SeqCst), 1);
    }
}
#[test]
fn each_reconnect_creates_and_reclaims_a_fresh_session_worker() {
    let factory = factory();
    let config = config(factory.clone());
    for id in 1..=20 {
        let mut owner = LiveContent::new(identity(id), Some(&config));
        assert_eq!(owner.status().state, UiContentState::Collecting);
        assert!(owner.metrics().is_some());
        owner.stop();
        owner.stop();
        assert_eq!(owner.status().state, UiContentState::Stopped);
        assert!(owner.metrics().is_none());
    }
    assert_eq!(factory.calls.load(Ordering::SeqCst), 20);
}
#[tokio::test]
async fn speech_rejection_never_enters_content_and_content_stop_does_not_cancel_speech() {
    let config = config(factory());
    let mut owner = LiveContent::new(identity(1), Some(&config));
    let (speech, _task) = SpeechSession::new(
        identity(1),
        SourceLanguage::English,
        MockSpeech(identity(1)).capabilities(),
    )
    .unwrap();
    for seq in 0..12 {
        speech.input.try_push(block(identity(1), seq)).unwrap();
    }
    assert_eq!(
        owner.submit_audio(&speech.input, block(identity(1), 12)),
        Err(SpeechError::queue_full(
            crate::ai::speech::SpeechQueue::AudioInput,
            crate::ai::speech::AUDIO_QUEUE_BLOCKS
        ))
    );
    assert_eq!(owner.metrics().unwrap().processed_blocks, 0);
    assert_eq!(owner.metrics().unwrap().queue_depth, 0);
    assert_eq!(owner.status().state, UiContentState::Collecting);
    let (speech, _task) = SpeechSession::new(
        identity(1),
        SourceLanguage::English,
        MockSpeech(identity(1)).capabilities(),
    )
    .unwrap();
    owner.resolver.as_mut().unwrap().stop().unwrap();
    assert!(
        owner
            .submit_audio(&speech.input, block(identity(1), 0))
            .unwrap()
    );
    assert_eq!(
        owner.status().state,
        UiContentState::Failed(ResolverError::Cancelled)
    );
    assert!(speech.cancellation().reason().is_none());
    assert!(owner.metrics().is_none());
}
#[tokio::test]
async fn expired_summary_clears_without_a_new_audio_window() {
    let config = config(factory());
    let mut owner = LiveContent::new(identity(1), Some(&config));
    owner.set_state(UiContentState::Ready(
        ContentType::Music,
        SongState::Confirmed,
    ));
    owner.expires = Some(Instant::now() - Duration::from_millis(1));
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), owner.changed())
            .await
            .unwrap(),
        UiContentState::Failed(ResolverError::Stale)
    );
    assert!(owner.expires.is_none());
    assert!(
        tokio::time::timeout(Duration::from_millis(5), owner.changed())
            .await
            .is_err()
    );
}
#[test]
fn invalid_service_disclosures_are_rejected_before_factory_creation() {
    for service in ["", " ", "cloud\nservice", "\0"] {
        let factory = factory();
        assert!(
            ContentConfiguration::new(
                ContentDisclosure {
                    service,
                    transport: ContentTransport::Remote
                },
                factory.clone()
            )
            .is_err()
        );
        assert_eq!(factory.calls.load(Ordering::SeqCst), 0);
    }
}
#[tokio::test]
async fn real_audio_branch_and_mock_speech_translation_share_ui_without_source_adoption() {
    verify_audio_branch(false).await;
}
#[tokio::test]
async fn provider_panic_reclaims_request_and_worker_while_speech_remains_running() {
    verify_audio_branch(true).await;
}
async fn verify_audio_branch(panic: bool) {
    let id = identity(1);
    let factory = factory();
    let config = config(factory.clone());
    let mut owner = LiveContent::new(id, Some(&config));
    let recognizer = MockSpeech(id);
    let (mut speech, task) =
        SpeechSession::new(id, SourceLanguage::English, recognizer.capabilities()).unwrap();
    let mut run = Box::pin(task.run(&recognizer));
    let mut flow = LiveTranslation::new(id, TranslationQuality::Standard, &MockTranslator).unwrap();
    let (mut publisher, mut inbox) = channel();
    publisher
        .prepare_source(UiSource {
            pid: 42,
            provider: SpeechProvider::OpenAi,
        })
        .unwrap();
    publisher
        .prepare_content(Some(config.disclosure()))
        .unwrap();
    publisher
        .begin(
            id,
            UiSource {
                pid: 42,
                provider: SpeechProvider::OpenAi,
            },
        )
        .unwrap();
    // Use the same submit/poll methods as the formal Live loop, with one speech run.
    for seq in 0..=600 {
        let before = owner.metrics().unwrap().processed_blocks;
        owner.submit_audio(&speech.input, block(id, seq)).unwrap();
        assert!(futures_util::poll!(run.as_mut()).is_pending());
        for kind in [TranscriptKind::Partial, TranscriptKind::Final] {
            let event = speech.next().await.unwrap();
            assert_eq!(event.kind, kind);
            if seq == 600 {
                flow.ingest(event).unwrap();
            }
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while owner.metrics().unwrap().processed_blocks == before {
            pump(&mut owner).await;
            assert!(Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        pump(&mut owner).await;
    }
    assert_eq!(factory.control.calls.load(Ordering::SeqCst), 1);
    assert_eq!(factory.control.active.load(Ordering::SeqCst), 1);
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            flow.changed().await.unwrap();
            if flow.timeline().items()[0].stability == crate::subtitle::timeline::Stability::Final {
                break;
            }
        }
    })
    .await
    .unwrap();
    publisher.publish(flow.timeline(), true).unwrap();
    let snapshot = inbox.poll().unwrap().snapshot.as_ref().unwrap().clone();
    factory.control.released.store(true, Ordering::SeqCst);
    factory.control.notify.notify_one();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), owner.changed())
            .await
            .unwrap(),
        UiContentState::Ready(ContentType::Speech, SongState::Unknown)
    );
    publisher.publish_content(id, owner.status()).unwrap();
    let frame = inbox.poll().unwrap();
    assert!(Arc::ptr_eq(frame.snapshot.as_ref().unwrap(), &snapshot));
    assert!(frame.status == UiStatus::Translating);
    assert!(frame.status_text().contains("語音"));
    assert!(speech.cancellation().reason().is_none());
    factory.control.released.store(false, Ordering::SeqCst);
    for seq in 601..=750 {
        let before = owner.metrics().unwrap().processed_blocks;
        owner.submit_audio(&speech.input, block(id, seq)).unwrap();
        assert!(futures_util::poll!(run.as_mut()).is_pending());
        speech.next().await.unwrap();
        speech.next().await.unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while owner.metrics().unwrap().processed_blocks == before {
            pump(&mut owner).await;
            assert!(Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        pump(&mut owner).await;
    }
    assert_eq!(factory.control.calls.load(Ordering::SeqCst), 2);
    assert_eq!(factory.control.active.load(Ordering::SeqCst), 1);
    if panic {
        factory.control.panic.store(true, Ordering::SeqCst);
        factory.control.released.store(true, Ordering::SeqCst);
        factory.control.notify.notify_one();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), owner.changed())
                .await
                .unwrap(),
            UiContentState::Failed(ResolverError::WorkerPanicked)
        );
        assert!(owner.metrics().is_none());
        assert!(speech.cancellation().reason().is_none());
        assert!(
            tokio::time::timeout(Duration::from_millis(5), owner.changed())
                .await
                .is_err()
        );
    }
    owner.stop();
    assert!(
        factory
            .control
            .token
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .is_cancelled()
    );
    assert_eq!(factory.control.active.load(Ordering::SeqCst), 0);
    publisher.clear(UiStatus::Reconnecting).unwrap();
    assert_eq!(inbox.poll().unwrap().content.state, UiContentState::Stopped);
    speech.cancellation().cancel();
    assert_eq!(run.await, Err(SpeechError::Cancelled));
}

#[tokio::test]
async fn formal_ui_worker_waits_for_disclosure_before_creating_content_service() {
    let factory = factory();
    let config = config(factory.clone());
    let (publisher, mut inbox) = channel();
    let mut worker = crate::app::ui_owner::UiWorker::start(publisher, move |publisher| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                publisher.prepare_source(UiSource {
                    pid: 42,
                    provider: SpeechProvider::OpenAi,
                })?;
                publisher.prepare_content(Some(config.disclosure()))?;
                publisher.wait_until_visible().await?;
                let _content = LiveContent::new(identity(1), Some(&config));
                publisher.cancellation().cancelled().await;
                Ok(())
            })
    })
    .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Some(frame) = inbox.poll()
                && frame.content.disclosure.is_some()
            {
                assert_eq!(factory.calls.load(Ordering::SeqCst), 0);
                inbox.acknowledge_visible(frame.version);
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        while factory.calls.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    worker.stop_join().unwrap();
    assert_eq!(factory.calls.load(Ordering::SeqCst), 1);
    assert_eq!(inbox.poll().unwrap().content.state, UiContentState::Stopped);
}
