//! Explicit synthetic sources exercising the actual Live composition and UI gate.
use super::*;
use crate::{
    ai::speech::SpeechLanguage,
    app::{
        lyrics::{LyricsServiceFactory, LyricsSongBinder},
        subtitles::{SubtitleMovieBinder, SubtitleServiceFactory},
    },
    audio::capture::SessionId,
    resolver::{
        candidates::{Resolution, SongState},
        contracts::*,
    },
    ui::{content::ContentTransport, subtitles::UiSubtitleState},
};
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

#[derive(Default)]
struct Control {
    subtitle_factories: AtomicUsize,
    lyrics_factories: AtomicUsize,
    lookups: AtomicUsize,
    failed_content: bool,
}
struct Services(Arc<Control>);
struct PendingIdentifier;
impl ContentIdentifier for PendingIdentifier {
    fn capabilities(&self) -> ResolverCapabilities {
        ResolverCapabilities {
            classification: true,
            songs: true,
            movies: true,
            cancellation: true,
        }
    }
    fn identify<'a>(&'a self, _: &'a AudioWindow, _: ResolverCancellation) -> IdentifyFuture<'a> {
        Box::pin(std::future::pending())
    }
}
impl ContentServiceFactory for Services {
    fn create(&self, _: StreamIdentity) -> Result<Arc<dyn ContentIdentifier>, ResolverError> {
        if self.0.failed_content {
            return Err(ResolverError::Network);
        }
        Ok(Arc::new(PendingIdentifier))
    }
}
impl SubtitleServiceFactory for Services {
    fn create(&self, _: StreamIdentity) -> Result<Arc<dyn SubtitleMovieBinder>, ResolverError> {
        self.0.subtitle_factories.fetch_add(1, Ordering::SeqCst);
        Ok(Arc::new(Self(self.0.clone())))
    }
}
impl SubtitleMovieBinder for Services {
    fn bind(&self, _: &MovieCandidate) -> Result<Arc<dyn ExistingTextProvider>, ResolverError> {
        Ok(Arc::new(Provider {
            control: self.0.clone(),
            source: SubtitleSource::ExistingSubtitle,
        }))
    }
}
impl LyricsServiceFactory for Services {
    fn create(&self, _: StreamIdentity) -> Result<Arc<dyn LyricsSongBinder>, ResolverError> {
        self.0.lyrics_factories.fetch_add(1, Ordering::SeqCst);
        Ok(Arc::new(Self(self.0.clone())))
    }
}
impl LyricsSongBinder for Services {
    fn bind(&self, _: &Resolution) -> Result<Arc<dyn ExistingTextProvider>, ResolverError> {
        Ok(Arc::new(Provider {
            control: self.0.clone(),
            source: SubtitleSource::ExistingLyrics,
        }))
    }
}
struct Provider {
    control: Arc<Control>,
    source: SubtitleSource,
}
impl Provider {
    fn document(&self, query: &TextQuery, language: TextLanguage, text: &str) -> ExistingText {
        ExistingText {
            identity: query.stamp.identity,
            source: query.source,
            content_id: query.content_id.clone(),
            language,
            cues: vec![TextCue {
                start_us: Some(5_000_000),
                duration_us: Some(1_000_000),
                text: text.into(),
            }],
        }
    }
}
impl ExistingTextProvider for Provider {
    fn supports(&self, source: SubtitleSource) -> bool {
        self.source == source
    }
    fn lookup<'a>(&'a self, query: &'a TextQuery, _: ResolverCancellation) -> TextFuture<'a> {
        Box::pin(async move {
            self.control.lookups.fetch_add(1, Ordering::SeqCst);
            Ok(Some(self.document(
                query,
                TextLanguage::Source(SpeechLanguage::English),
                "synthetic source sentence",
            )))
        })
    }
    fn lookup_bundle<'a>(
        &'a self,
        query: &'a TextQuery,
        cancel: ResolverCancellation,
    ) -> TextBundleFuture<'a> {
        Box::pin(async move {
            if self.source == SubtitleSource::ExistingLyrics {
                return Ok(self
                    .lookup(query, cancel)
                    .await?
                    .map(|text| ExistingTextBundle {
                        text,
                        source_reference: None,
                    }));
            }
            self.control.lookups.fetch_add(1, Ordering::SeqCst);
            Ok(Some(ExistingTextBundle {
                text: self.document(query, TextLanguage::SimplifiedChinese, "软件与服务器字幕"),
                source_reference: Some(SourceTextReference {
                    text: self.document(
                        query,
                        TextLanguage::Source(SpeechLanguage::English),
                        "synthetic source sentence",
                    ),
                    primary_edition_id: "synthetic edition".into(),
                    reference_edition_id: "synthetic edition".into(),
                }),
            }))
        })
    }
}
fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(995),
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
fn movie() -> Resolution {
    Resolution {
        kind: ContentType::MovieOrShow,
        song_state: SongState::Unknown,
        song: None,
        movies: vec![Arc::new(MovieCandidate {
            id: "synthetic movie".into(),
            title: "synthetic title".into(),
            year: Some(2020),
            confidence: Confidence::new(900).unwrap(),
            signals: vec![
                MovieSignal::WindowTitle,
                MovieSignal::Dialogue,
                MovieSignal::RemoteMetadata,
            ],
        })],
    }
}
fn song() -> Resolution {
    Resolution {
        kind: ContentType::Music,
        song_state: SongState::Confirmed,
        movies: vec![],
        song: Some(Arc::new(SongCandidate {
            id: "synthetic song".into(),
            title: "synthetic song title".into(),
            artist: None,
            confidence: Confidence::new(900).unwrap(),
            match_offset_us: None,
        })),
    }
}
fn disclosure(service: &'static str) -> ContentDisclosure {
    ContentDisclosure {
        service,
        transport: ContentTransport::Local,
    }
}
fn configuration(control: Arc<Control>) -> ContentConfiguration {
    ContentConfiguration::new(
        disclosure("合成內容目錄"),
        Arc::new(Services(control.clone())),
    )
    .unwrap()
    .with_lyrics(
        LyricsConfiguration::new(
            disclosure("合成歌詞來源"),
            Arc::new(Services(control.clone())),
        )
        .unwrap(),
    )
    .with_subtitles(
        SubtitleConfiguration::new(disclosure("合成字幕來源"), Arc::new(Services(control)))
            .unwrap(),
    )
}
fn observe(owner: &mut LiveContent, sequence: u64, status: ResolverStatus, resolution: Resolution) {
    let state = owner
        .observe(&ResolverUpdate::fixture(
            stamp(sequence),
            status,
            resolution,
        ))
        .unwrap();
    owner.set_state(state);
}
async fn ready(owner: &mut LiveContent, source: SubtitleSource) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while owner
            .ready_text()
            .is_none_or(|text| text.text().source != source)
        {
            owner.changed().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn settled_movie_and_song_switch_one_selected_text_source_without_refetching_confirmations() {
    let control = Arc::new(Control::default());
    let config = configuration(control.clone());
    let mut owner = LiveContent::new(identity(), Some(&config));
    observe(&mut owner, 1, ResolverStatus::Ready, movie());
    ready(&mut owner, SubtitleSource::ExistingSubtitle).await;
    let old = owner.ready_subtitles().unwrap();
    observe(
        &mut owner,
        2,
        ResolverStatus::Resolving,
        Resolution::default(),
    );
    assert!(owner.ready_subtitles().is_some());
    observe(&mut owner, 2, ResolverStatus::Ready, movie());
    assert_eq!(control.lookups.load(Ordering::SeqCst), 1);
    assert_eq!(old.query().stamp.sequence, 1);
    assert_eq!(owner.ready_text().unwrap().query().stamp.sequence, 2);
    observe(&mut owner, 3, ResolverStatus::Ready, song());
    assert_eq!(old.validate(identity()), Err(ResolverError::Cancelled));
    assert!(owner.ready_subtitles().is_none());
    ready(&mut owner, SubtitleSource::ExistingLyrics).await;
    let lyrics = owner.ready_lyrics().unwrap();
    let mut ambiguous = movie();
    let mut other = (*ambiguous.movies[0]).clone();
    other.id = "other synthetic movie".into();
    other.confidence = Confidence::new(850).unwrap();
    ambiguous.movies.push(Arc::new(other));
    observe(&mut owner, 4, ResolverStatus::Ready, ambiguous);
    assert_eq!(lyrics.validate(identity()), Err(ResolverError::Cancelled));
    assert!(owner.ready_text().is_none());
    assert_eq!(owner.subtitles_status().state, UiSubtitleState::Waiting);
    assert_eq!(control.lookups.load(Ordering::SeqCst), 2);
    owner.stop();
}

#[tokio::test]
async fn gaps_errors_expiry_stop_and_drop_retire_subtitle_publications_synchronously() {
    for action in 0..5 {
        let config = configuration(Arc::new(Control::default()));
        let mut owner = LiveContent::new(identity(), Some(&config));
        observe(&mut owner, 1, ResolverStatus::Ready, movie());
        ready(&mut owner, SubtitleSource::ExistingSubtitle).await;
        let lease = owner.ready_subtitles().unwrap();
        match action {
            0 => {
                assert!(owner.reset_text_for_audio_gap());
            }
            1 => observe(
                &mut owner,
                2,
                ResolverStatus::Failed(ResolverError::Network),
                Resolution::default(),
            ),
            2 => {
                owner.expires = Some(Instant::now());
                assert_eq!(
                    owner.changed().await,
                    UiContentState::Failed(ResolverError::Stale)
                );
            }
            3 => owner.stop(),
            _ => {
                drop(owner);
                assert_eq!(lease.validate(identity()), Err(ResolverError::Cancelled));
                continue;
            }
        }
        assert_eq!(lease.validate(identity()), Err(ResolverError::Cancelled));
        assert!(owner.ready_text().is_none());
        owner.stop();
    }
}

#[tokio::test]
async fn old_and_duplicate_confirmations_do_not_clear_current_subtitle_selection() {
    let config = configuration(Arc::new(Control::default()));
    let mut owner = LiveContent::new(identity(), Some(&config));
    observe(&mut owner, 3, ResolverStatus::Ready, movie());
    ready(&mut owner, SubtitleSource::ExistingSubtitle).await;
    let lease = owner.ready_subtitles().unwrap();
    assert_eq!(
        owner.observe(&ResolverUpdate::fixture(
            stamp(1),
            ResolverStatus::Ready,
            song()
        )),
        Err(ResolverError::Stale)
    );
    let duplicate = lease.query().stamp;
    owner
        .observe(&ResolverUpdate::fixture(
            duplicate,
            ResolverStatus::Ready,
            movie(),
        ))
        .unwrap();
    assert!(lease.validate(identity()).is_ok());
    assert_eq!(
        owner.ready_text().unwrap().text().source,
        SubtitleSource::ExistingSubtitle
    );
    owner.stop();
}

#[tokio::test]
async fn unconfigured_and_failed_content_never_start_subtitle_requests_or_spin() {
    let mut disabled = LiveContent::new(identity(), None);
    assert!(disabled.ready_text().is_none());
    assert!(
        tokio::time::timeout(Duration::from_millis(20), disabled.changed())
            .await
            .is_err()
    );
    let control = Arc::new(Control {
        failed_content: true,
        ..Control::default()
    });
    let config = configuration(control.clone());
    let mut failed = LiveContent::new(identity(), Some(&config));
    assert_eq!(
        failed.subtitles_status().state,
        UiSubtitleState::Failed(ResolverError::Network)
    );
    assert_eq!(control.lookups.load(Ordering::SeqCst), 0);
    assert!(
        tokio::time::timeout(Duration::from_millis(20), failed.changed())
            .await
            .is_err()
    );
    failed.stop();
}

#[cfg(windows)]
#[tokio::test]
async fn actual_live_scheduler_and_publisher_route_subtitles_through_one_timeline_and_source_status()
 {
    use crate::{
        ai::{
            provider::SpeechProvider,
            speech::{TranscriptEvent, TranscriptKind},
            translation::*,
        },
        domain::TranslationQuality,
        session::{live_text::LiveTextPipeline, live_translation::LiveTranslation},
        subtitle::source_arbiter::PlaybackObservation,
        ui::bridge::{UiSource, channel},
    };
    struct NoTranslation;
    impl Translator for NoTranslation {
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
            Box::pin(async { panic!("Chinese subtitle preparation must remain local") })
        }
    }
    let config = configuration(Arc::new(Control::default()));
    let mut owner = LiveContent::new(identity(), Some(&config));
    observe(&mut owner, 1, ResolverStatus::Ready, movie());
    ready(&mut owner, SubtitleSource::ExistingSubtitle).await;
    let translator = NoTranslation;
    let mut flow = LiveTranslation::new(identity(), TranslationQuality::High, &translator).unwrap();
    flow.ingest(TranscriptEvent {
        identity: identity(),
        sequence: 1,
        utterance: 1,
        revision: 1,
        kind: TranscriptKind::Final,
        language: Some(SpeechLanguage::English),
        start_us: Some(42_000_000),
        duration_us: Some(950_000),
        text: "synthetic source sentence".into(),
    })
    .unwrap();
    let (mut pipeline, task) = LiveTextPipeline::new(identity(), TranslationQuality::High);
    crate::app::live::schedule_text(&owner, Some(&mut pipeline), Some(&mut flow));
    assert_eq!(pipeline.source(), Some(SubtitleSource::ExistingSubtitle));
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
    publisher
        .prepare_subtitles(config.subtitles_disclosure())
        .unwrap();
    publisher.begin(identity(), source).unwrap();
    crate::app::live::publish_content_status(
        &mut publisher,
        identity(),
        &owner,
        Some(&pipeline),
        Some(&flow),
    )
    .unwrap();
    let frame = inbox.poll().unwrap();
    assert_eq!(frame.subtitles.state, UiSubtitleState::Verifying);
    assert_eq!(
        frame.lyrics.state,
        crate::ui::lyrics::UiLyricsState::Waiting
    );
    let worker = task.isolated().run(&translator);
    tokio::pin!(worker);
    let done = tokio::time::timeout(Duration::from_secs(2), async {
        loop { tokio::select! { done = pipeline.changed() => if let Some(done) = done { break done; }, result = &mut worker => panic!("worker ended: {result:?}") } }
    }).await.unwrap();
    let observation = PlaybackObservation {
        version: flow.sources_mut().version(),
        sequence: 100,
        observed_at: Instant::now(),
        capture_position_us: 42_500_000,
    };
    assert!(pipeline.apply(done, &mut flow, Some(observation)).unwrap());
    publisher.publish(flow.timeline(), true).unwrap();
    crate::app::live::publish_content_status(
        &mut publisher,
        identity(),
        &owner,
        Some(&pipeline),
        Some(&flow),
    )
    .unwrap();
    let frame = inbox.poll().unwrap();
    assert_eq!(frame.subtitles.state, UiSubtitleState::UsingExisting);
    assert_eq!(
        frame.lyrics.state,
        crate::ui::lyrics::UiLyricsState::Waiting
    );
    assert_eq!(
        frame.snapshot.as_ref().unwrap().latest_origin(),
        Some(SubtitleSource::ExistingSubtitle)
    );
    assert_eq!(flow.timeline().items().len(), 1);
    assert_eq!(
        flow.timeline().items()[0].display_translation(),
        "軟體與伺服器字幕"
    );
    assert!(owner.reset_text_for_audio_gap());
    crate::app::live::schedule_text(&owner, Some(&mut pipeline), Some(&mut flow));
    assert!(pipeline.source().is_none());
    assert_eq!(
        flow.timeline().items()[0].display_origin(),
        SubtitleSource::LiveAi
    );
    pipeline.stop();
    worker.await.unwrap();
    owner.stop();
}
