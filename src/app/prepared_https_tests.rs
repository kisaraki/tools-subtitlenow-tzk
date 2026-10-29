//! Disclosed controller entry, literal loopback subtitles and explicit services.
use super::*;
use crate::media::{
    embedded_mp4::Mp4SubtitleSelection,
    remote_subtitle::{PreparedSubtitleUrl, RemoteSubtitleFailure},
};
use std::sync::atomic::AtomicU16;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::oneshot,
};

type ResponsePause = (oneshot::Sender<()>, oneshot::Receiver<()>);
struct TextSource {
    address: std::net::SocketAddr,
    rows: Arc<AtomicUsize>,
    status: Arc<AtomicU16>,
    body: Arc<Mutex<Vec<u8>>>,
    pause: Arc<Mutex<Option<ResponsePause>>>,
    stop: oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}
impl TextSource {
    async fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let rows = Arc::new(AtomicUsize::new(0));
        let status = Arc::new(AtomicU16::new(200));
        let body = Arc::new(Mutex::new(Vec::new()));
        let pause = Arc::new(Mutex::new(
            None::<(oneshot::Sender<()>, oneshot::Receiver<()>)>,
        ));
        let (stop, mut stopped) = oneshot::channel();
        let seen = rows.clone();
        let mode = status.clone();
        let content = body.clone();
        let paused = pause.clone();
        let task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = tokio::select! {biased;
                    _ = &mut stopped => break,
                    result = listener.accept() => result.unwrap(),
                };
                let mut header = Vec::new();
                while !header.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    assert_eq!(socket.read(&mut byte).await.unwrap(), 1);
                    header.push(byte[0]);
                    assert!(header.len() <= 8192);
                }
                let header = String::from_utf8(header).unwrap().to_ascii_lowercase();
                assert!(
                    header
                        .starts_with("get /private-subtitle?ticket=synthetic_private http/1.1\r\n")
                );
                for forbidden in ["authorization:", "cookie:", "range:"] {
                    assert!(!header.contains(forbidden));
                }
                seen.fetch_add(1, Ordering::SeqCst);
                let bytes = content.lock().unwrap().clone();
                socket.write_all(format!("HTTP/1.1 {} Synthetic\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", mode.load(Ordering::Acquire), bytes.len()).as_bytes()).await.unwrap();
                let wait = paused.lock().unwrap().take();
                if let Some((entered, resume)) = wait {
                    let _ = entered.send(());
                    let mut byte = [0];
                    tokio::select! {biased;
                        _ = &mut stopped => break,
                        result = socket.read(&mut byte) => { assert!(matches!(result, Ok(0) | Err(_))); continue; },
                        _ = resume => {},
                    }
                }
                let _ = socket.write_all(&bytes).await;
            }
        });
        Self {
            address,
            rows,
            status,
            body,
            pause,
            stop,
            task,
        }
    }
    fn input(&self, format: SubtitleFormat, language: TextLanguage) -> PreparedSubtitleUrl {
        PreparedSubtitleUrl::local(
            &format!(
                "http://{}/private-subtitle?ticket=SYNTHETIC_PRIVATE",
                self.address
            ),
            format,
            language,
        )
        .unwrap()
    }
    fn document(&self, format: SubtitleFormat, text: &str) {
        *self.body.lock().unwrap() = match format {
            SubtitleFormat::Srt => format!("1\n00:00:00,004 --> 00:00:00,016\n{text}\n"),
            SubtitleFormat::WebVtt => format!("WEBVTT\n\n00:00:00.004 --> 00:00:00.016\n{text}\n"),
        }
        .into_bytes();
    }
    fn request(
        &self,
        fixture: &Fixture,
        format: SubtitleFormat,
        language: TextLanguage,
    ) -> PreparedRequest {
        fixture
            .request(vec![])
            .with_remote_subtitle(self.input(format, language))
            .unwrap()
    }
    fn pause(&self) -> (oneshot::Receiver<()>, oneshot::Sender<()>) {
        let (entered, ready) = oneshot::channel();
        let (release, resume) = oneshot::channel();
        *self.pause.lock().unwrap() = Some((entered, resume));
        (ready, release)
    }
    async fn finish(self) {
        let _ = self.stop.send(());
        self.task.await.unwrap();
    }
}
fn with_text_source(test: impl FnOnce(&TextSource) + Send + 'static) {
    let _serial = work::TEST_OWNERS.lock().unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let source = TextSource::new().await;
        let source = tokio::task::spawn_blocking(move || {
            test(&source);
            source
        })
        .await
        .unwrap();
        source.finish().await;
    });
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Success,
    AnalysisPending,
    TranslationPending,
    AnalysisFailure,
    TranslationFailure,
    WrongKind,
}
#[derive(Default)]
struct HttpsStats {
    builds: AtomicUsize,
    keys: AtomicUsize,
    analyses: AtomicUsize,
    translations: AtomicUsize,
    active: AtomicUsize,
    selected: Mutex<Vec<(SpeechProvider, PreparedServiceKind)>>,
    qualities: Mutex<Vec<TranslationQuality>>,
    passes: Mutex<Vec<TranslationPass>>,
}
struct HttpsFactory {
    mode: Mode,
    stats: Arc<HttpsStats>,
    entered: Mutex<Option<mpsc::SyncSender<TranslationCancellation>>>,
}
impl HttpsFactory {
    fn new(mode: Mode) -> Arc<Self> {
        Arc::new(Self {
            mode,
            stats: Arc::new(HttpsStats::default()),
            entered: Mutex::new(None),
        })
    }
    fn pending(mode: Mode) -> (Arc<Self>, mpsc::Receiver<TranslationCancellation>) {
        let (entered, received) = mpsc::sync_channel(1);
        let mut factory = Self::new(mode);
        *Arc::get_mut(&mut factory)
            .unwrap()
            .entered
            .get_mut()
            .unwrap() = Some(entered);
        (factory, received)
    }
}
impl SelectedPreparedServiceFactory for HttpsFactory {
    fn build(
        &self,
        provider: SpeechProvider,
        key: Option<ApiKey>,
        kind: PreparedServiceKind,
    ) -> Result<PreparedServices, PreparedUiError> {
        assert!(kind == PreparedServiceKind::Translation);
        assert_eq!(
            key.expect("explicit factory must not read secure storage")
                .expose(),
            "SYNTHETIC_PREPARED_ASR_KEY"
        );
        self.stats.builds.fetch_add(1, Ordering::SeqCst);
        self.stats.keys.fetch_add(1, Ordering::SeqCst);
        self.stats.selected.lock().unwrap().push((provider, kind));
        let translator = Box::new(HttpsTranslator {
            mode: self.mode,
            stats: self.stats.clone(),
            entered: Mutex::new(self.entered.lock().unwrap().take()),
        });
        if self.mode == Mode::WrongKind {
            Ok(PreparedServices::Recognition {
                recognizer: Box::new(UnexpectedRecognizer),
                translator,
            })
        } else {
            Ok(PreparedServices::Translation(translator))
        }
    }
}
struct UnexpectedRecognizer;
impl PreparedSpeechRecognizer for UnexpectedRecognizer {
    fn capabilities(&self) -> PreparedSpeechCapabilities {
        panic!("unexpected ASR capability request")
    }
    fn recognize<'a>(
        &'a self,
        _: &'a PreparedSpeechRequest,
        _: ResolverCancellation,
    ) -> PreparedSpeechFuture<'a> {
        Box::pin(async { panic!("unexpected ASR request") })
    }
}
struct HttpsTranslator {
    mode: Mode,
    stats: Arc<HttpsStats>,
    entered: Mutex<Option<mpsc::SyncSender<TranslationCancellation>>>,
}
struct HttpsActive(Arc<HttpsStats>);
impl Drop for HttpsActive {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
    }
}
fn private_metadata_absent(text: &str) {
    for private in [
        "127.0.0.1",
        "http://",
        "https://",
        "private-subtitle",
        "ticket=",
        "SYNTHETIC_PRIVATE",
        "synthetic.wav",
    ] {
        assert!(
            !text.contains(private),
            "private destination crossed AI boundary"
        );
    }
}
impl PreparedGlossaryAnalyzer for HttpsTranslator {
    fn analyze<'a>(
        &'a self,
        request: &'a PreparedGlossaryRequest,
        cancel: TranslationCancellation,
    ) -> PreparedGlossaryFuture<'a> {
        Box::pin(async move {
            self.stats.analyses.fetch_add(1, Ordering::SeqCst);
            self.stats.active.fetch_add(1, Ordering::SeqCst);
            let _active = HttpsActive(self.stats.clone());
            for cue in request.cues() {
                private_metadata_absent(&cue.text);
            }
            for term in request.known_terms() {
                private_metadata_absent(&term.source);
                private_metadata_absent(&term.taiwan_translation);
            }
            if self.mode == Mode::AnalysisPending {
                assert!(
                    self.entered
                        .lock()
                        .unwrap()
                        .take()
                        .unwrap()
                        .send(cancel)
                        .is_ok()
                );
                futures_util::future::pending::<()>().await;
            }
            if self.mode == Mode::AnalysisFailure {
                return Err(TranslationError::Refused);
            }
            Ok(PreparedGlossaryResponse {
                identity: request.identity(),
                sequence: request.sequence(),
                proposals: vec![],
                context_summary: "合成全文摘要".into(),
            })
        })
    }
}
impl Translator for HttpsTranslator {
    fn capabilities(&self) -> TranslationCapabilities {
        TranslationCapabilities {
            streaming: true,
            cancellation: true,
            supported_languages: [true; 5],
            max_concurrent: 2,
        }
    }
    fn translate<'a>(
        &'a self,
        request: &'a TranslationRequest,
        _: &'a mut TranslationEmitter,
        cancel: TranslationCancellation,
    ) -> TranslationFuture<'a> {
        Box::pin(async move {
            self.stats.translations.fetch_add(1, Ordering::SeqCst);
            self.stats.active.fetch_add(1, Ordering::SeqCst);
            let _active = HttpsActive(self.stats.clone());
            self.stats.qualities.lock().unwrap().push(request.quality);
            self.stats.passes.lock().unwrap().push(request.pass);
            private_metadata_absent(&request.source.text);
            private_metadata_absent(&request.committed_prefix);
            if let Some(draft) = &request.draft {
                private_metadata_absent(draft);
            }
            for stable in &request.previous_stable {
                private_metadata_absent(&stable.source);
                private_metadata_absent(&stable.translation);
            }
            for term in &request.glossary {
                private_metadata_absent(&term.source);
                private_metadata_absent(&term.taiwan_translation);
            }
            let context = request.prepared_context.as_ref().unwrap();
            assert_eq!(context.document.as_ref().unwrap().summary(), "合成全文摘要");
            for source in context
                .previous_source
                .iter()
                .chain(&context.following_source)
            {
                private_metadata_absent(source);
            }
            for draft in &context.neighboring_drafts {
                private_metadata_absent(&draft.source);
                private_metadata_absent(&draft.translation);
            }
            if self.mode == Mode::TranslationPending {
                assert!(
                    self.entered
                        .lock()
                        .unwrap()
                        .take()
                        .unwrap()
                        .send(cancel)
                        .is_ok()
                );
                futures_util::future::pending::<()>().await;
            }
            if self.mode == Mode::TranslationFailure {
                return Err(TranslationError::RateLimited);
            }
            Ok("软件与服务器".into())
        })
    }
}
fn assert_ready(controller: &mut PreparedController, language: TextLanguage) {
    assert_eq!(terminal(controller), PreparedUiState::Ready { cues: 1 });
    let document = controller.ready().unwrap();
    let original = document.original().existing().unwrap();
    assert!(original.language == language);
    assert_eq!(original.identity, document.identity());
    assert_eq!(original.cues[0].start_us, Some(4_000));
    assert_eq!(original.cues[0].duration_us, Some(12_000));
    assert_eq!(document.translated()[0].as_ref(), "軟體與伺服器");
}

#[test]
fn https_controller_chinese_skips_factory_keys_and_all_cloud_work() {
    with_text_source(|source| {
        let fixture = Fixture::new();
        for language in [
            TextLanguage::TaiwanTraditional,
            TextLanguage::OtherTraditional,
            TextLanguage::SimplifiedChinese,
        ] {
            for format in [SubtitleFormat::Srt, SubtitleFormat::WebVtt] {
                source.document(format, "软件与服务器");
                let factory = HttpsFactory::new(Mode::WrongKind);
                let mut controller = PreparedController::with_factory(factory.clone());
                controller
                    .start_with_https_subtitles(
                        source.request(&fixture, format, language),
                        options(None, SourceLanguage::Auto),
                        key(),
                    )
                    .unwrap();
                assert_ready(&mut controller, language);
                assert_eq!(factory.stats.builds.load(Ordering::SeqCst), 0);
                assert_eq!(factory.stats.keys.load(Ordering::SeqCst), 0);
                let requests = source.rows.load(Ordering::SeqCst);
                assert_eq!(controller.poll(), PreparedUiState::Ready { cues: 1 });
                assert_eq!(source.rows.load(Ordering::SeqCst), requests);
                controller.stop();
                assert!(controller.ready().is_none());
                assert!(fixture.writable());
            }
        }
    });
}

#[test]
fn https_controller_both_services_five_languages_and_qualities_use_one_translation_key() {
    with_text_source(|source| {
        let fixture = Fixture::new();
        for provider in [SpeechProvider::OpenAi, SpeechProvider::Gemini] {
            for speech in [
                SpeechLanguage::English,
                SpeechLanguage::Japanese,
                SpeechLanguage::French,
                SpeechLanguage::German,
                SpeechLanguage::Korean,
            ] {
                for quality in [
                    TranslationQuality::Fast,
                    TranslationQuality::Standard,
                    TranslationQuality::High,
                ] {
                    let format = if speech == SpeechLanguage::Japanese {
                        SubtitleFormat::WebVtt
                    } else {
                        SubtitleFormat::Srt
                    };
                    source.document(format, "synthetic source subtitle");
                    let language = TextLanguage::Source(speech);
                    let factory = HttpsFactory::new(Mode::Success);
                    let mut controller = PreparedController::with_factory(factory.clone());
                    let mut selected = options(Some(provider), SourceLanguage::Auto);
                    selected.quality = quality;
                    controller
                        .start_with_https_subtitles(
                            source.request(&fixture, format, language),
                            selected,
                            key(),
                        )
                        .unwrap();
                    assert_ready(&mut controller, language);
                    assert_eq!(factory.stats.builds.load(Ordering::SeqCst), 1);
                    assert_eq!(factory.stats.keys.load(Ordering::SeqCst), 1);
                    assert!(
                        factory.stats.selected.lock().unwrap().as_slice()
                            == [(provider, PreparedServiceKind::Translation)]
                    );
                    let passes: &[TranslationPass] = if quality == TranslationQuality::Fast {
                        &[TranslationPass::Draft]
                    } else {
                        &[TranslationPass::Draft, TranslationPass::Review]
                    };
                    assert_eq!(factory.stats.passes.lock().unwrap().as_slice(), passes);
                    assert!(
                        factory
                            .stats
                            .qualities
                            .lock()
                            .unwrap()
                            .iter()
                            .all(|actual| *actual == quality)
                    );
                    assert_eq!(factory.stats.analyses.load(Ordering::SeqCst), 1);
                    controller.stop();
                }
            }
        }
    });
}

#[test]
fn https_controller_winner_language_skips_factory_and_only_fetches_selected_remote_text() {
    with_text_source(|source| {
        let fixture = Fixture::new();
        let factory = HttpsFactory::new(Mode::WrongKind);
        let mut controller = PreparedController::with_factory(factory.clone());
        for local_wins in [true, false] {
            let local = if local_wins {
                fixture.subtitle("preferred-chinese.srt", TextLanguage::TaiwanTraditional)
            } else {
                LocalSubtitleInput::new(
                    fixture.directory.path().join("missing-lower-source.srt"),
                    SubtitleFormat::Srt,
                    TextLanguage::Source(SpeechLanguage::English),
                )
                .unwrap()
            };
            let remote_language = if local_wins {
                TextLanguage::Source(SpeechLanguage::English)
            } else {
                TextLanguage::SimplifiedChinese
            };
            let request = fixture
                .request(vec![local])
                .with_remote_subtitle(source.input(SubtitleFormat::Srt, remote_language))
                .unwrap();
            source.document(SubtitleFormat::Srt, "软件与服务器");
            let before = source.rows.load(Ordering::SeqCst);
            controller
                .start_with_https_subtitles(
                    request,
                    options(Some(SpeechProvider::Gemini), SourceLanguage::Auto),
                    key(),
                )
                .unwrap();
            assert_ready(
                &mut controller,
                if local_wins {
                    TextLanguage::TaiwanTraditional
                } else {
                    TextLanguage::SimplifiedChinese
                },
            );
            assert_eq!(
                source.rows.load(Ordering::SeqCst) - before,
                usize::from(!local_wins)
            );
            assert_eq!(factory.stats.builds.load(Ordering::SeqCst), 0);
            assert_eq!(factory.stats.keys.load(Ordering::SeqCst), 0);
            controller.stop();
        }
    });
}

#[test]
fn https_controller_metadata_guards_precede_credentials_factory_and_worker() {
    with_text_source(|source| {
        let fixture = Fixture::new();
        let factory = HttpsFactory::new(Mode::WrongKind);
        let mut controller = PreparedController::with_factory(factory.clone());
        for scenario in 0..4 {
            let mut request = if scenario == 3 {
                fixture.request(vec![])
            } else {
                source.request(
                    &fixture,
                    SubtitleFormat::Srt,
                    TextLanguage::Source(SpeechLanguage::English),
                )
            };
            if scenario == 0 {
                request = request
                    .with_remote_subtitle(source.input(
                        SubtitleFormat::WebVtt,
                        TextLanguage::Source(SpeechLanguage::Japanese),
                    ))
                    .unwrap();
            }
            if scenario == 1 {
                request.media =
                    MediaInput::remote("https://synthetic.invalid/never-fetch").unwrap();
                request = request
                    .with_embedded_subtitle(
                        Mp4SubtitleSelection::new(3, TextLanguage::SimplifiedChinese).unwrap(),
                    )
                    .unwrap();
            }
            let expected = match scenario {
                0 => PreparedUiError::Media(MediaError::AmbiguousSubtitle),
                1 => PreparedUiError::Media(MediaError::UnsupportedUrl),
                2 => PreparedUiError::ProviderRequired,
                3 => PreparedUiError::Media(MediaError::InvalidInput),
                _ => unreachable!(),
            };
            assert_eq!(
                controller.start_with_https_subtitles(
                    request,
                    options(None, SourceLanguage::Auto),
                    key()
                ),
                Err(expected)
            );
            assert!(controller.worker.is_none());
            assert!(controller.session.identity().is_none());
        }
        assert_eq!(factory.stats.builds.load(Ordering::SeqCst), 0);
        assert_eq!(source.rows.load(Ordering::SeqCst), 0);
    });
}

#[test]
fn https_controller_transport_and_provider_errors_are_typed_without_audio_fallback() {
    with_text_source(|source| {
        let fixture = Fixture::new();
        for mode in [
            Mode::Success,
            Mode::AnalysisFailure,
            Mode::TranslationFailure,
            Mode::WrongKind,
        ] {
            source.status.store(
                if mode == Mode::Success { 404 } else { 200 },
                Ordering::Release,
            );
            source.document(SubtitleFormat::Srt, "synthetic source subtitle");
            let factory = HttpsFactory::new(mode);
            let mut controller = PreparedController::with_factory(factory.clone());
            let before = source.rows.load(Ordering::SeqCst);
            controller
                .start_with_https_subtitles(
                    source.request(
                        &fixture,
                        SubtitleFormat::Srt,
                        TextLanguage::Source(SpeechLanguage::English),
                    ),
                    options(Some(SpeechProvider::Gemini), SourceLanguage::Auto),
                    key(),
                )
                .unwrap();
            let error = match mode {
                Mode::Success => PreparedUiError::Media(MediaError::RemoteSubtitle(
                    RemoteSubtitleFailure::NotFound,
                )),
                Mode::AnalysisFailure => PreparedUiError::Translation(TranslationError::Refused),
                Mode::TranslationFailure => {
                    PreparedUiError::Translation(TranslationError::RateLimited)
                }
                Mode::WrongKind => PreparedUiError::WorkerFailed,
                _ => unreachable!(),
            };
            assert_eq!(terminal(&mut controller), PreparedUiState::Failed(error));
            assert!(controller.ready().is_none());
            assert_eq!(factory.stats.active.load(Ordering::SeqCst), 0);
            assert_eq!(
                source.rows.load(Ordering::SeqCst) - before,
                usize::from(mode != Mode::WrongKind)
            );
            private_metadata_absent(&error.to_string());
            assert!(fixture.writable());
        }
    });
}

#[test]
fn https_controller_stop_during_http_analysis_or_translation_reaps_owned_work() {
    with_text_source(|source| {
        let fixture = Fixture::new();
        source.document(SubtitleFormat::Srt, "synthetic source subtitle");
        for mode in [
            Mode::Success,
            Mode::AnalysisPending,
            Mode::TranslationPending,
        ] {
            let (factory, entered) = if mode == Mode::Success {
                (HttpsFactory::new(mode), None)
            } else {
                let (factory, received) = HttpsFactory::pending(mode);
                (factory, Some(received))
            };
            let pause = (mode == Mode::Success).then(|| source.pause());
            let mut controller = PreparedController::with_factory(factory.clone());
            controller
                .start_with_https_subtitles(
                    source.request(
                        &fixture,
                        SubtitleFormat::Srt,
                        TextLanguage::Source(SpeechLanguage::English),
                    ),
                    options(Some(SpeechProvider::OpenAi), SourceLanguage::Auto),
                    key(),
                )
                .unwrap();
            let token = entered.map(|received| received.recv_timeout(TEST_TIMEOUT).unwrap());
            let mut held_pause = None;
            if let Some((mut ready, release)) = pause {
                held_pause = Some(release);
                let deadline = Instant::now() + TEST_TIMEOUT;
                loop {
                    match ready.try_recv() {
                        Ok(()) => break,
                        Err(oneshot::error::TryRecvError::Empty) => {}
                        Err(error) => panic!("synthetic HTTP did not enter: {error}"),
                    }
                    assert!(Instant::now() < deadline);
                    std::thread::yield_now();
                }
            }
            controller.stop();
            assert_eq!(controller.state(), PreparedUiState::Stopping);
            assert_eq!(
                controller.start_with_https_subtitles(
                    source.request(
                        &fixture,
                        SubtitleFormat::Srt,
                        TextLanguage::TaiwanTraditional
                    ),
                    options(None, SourceLanguage::Auto),
                    None
                ),
                Err(PreparedUiError::Busy)
            );
            assert_eq!(terminal(&mut controller), PreparedUiState::Cancelled);
            drop(held_pause);
            if let Some(token) = token {
                assert_eq!(token.reason(), Some(TranslationError::Cancelled));
            }
            assert!(controller.worker.is_none());
            assert!(controller.ready().is_none());
            assert_eq!(factory.stats.active.load(Ordering::SeqCst), 0);
            assert!(fixture.writable());
        }
    });
}

#[test]
fn https_controller_stopped_buffered_completion_cannot_replace_new_session_ready() {
    with_text_source(|source| {
        let fixture = Fixture::new();
        source.document(SubtitleFormat::Srt, "synthetic source subtitle");
        let factory = HttpsFactory::new(Mode::Success);
        let mut controller = PreparedController::with_factory(factory.clone());
        controller
            .start_with_https_subtitles(
                source.request(
                    &fixture,
                    SubtitleFormat::Srt,
                    TextLanguage::Source(SpeechLanguage::French),
                ),
                options(Some(SpeechProvider::OpenAi), SourceLanguage::Auto),
                key(),
            )
            .unwrap();
        let old = controller.session.identity().unwrap();
        finished(&controller);
        assert!(controller.ready().is_none());
        controller.stop();
        assert_eq!(terminal(&mut controller), PreparedUiState::Cancelled);
        source.document(SubtitleFormat::Srt, "软件与服务器");
        controller
            .start_with_https_subtitles(
                source.request(
                    &fixture,
                    SubtitleFormat::Srt,
                    TextLanguage::SimplifiedChinese,
                ),
                options(None, SourceLanguage::Auto),
                key(),
            )
            .unwrap();
        assert_ready(&mut controller, TextLanguage::SimplifiedChinese);
        assert_ne!(controller.ready().unwrap().identity().session, old.session);
        assert_eq!(factory.stats.builds.load(Ordering::SeqCst), 1);
        controller.stop();
        assert!(fixture.writable());
    });
}
