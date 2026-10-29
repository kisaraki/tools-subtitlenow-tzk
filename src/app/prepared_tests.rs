//! Caller-created opaque media/sidecars and explicit mock translation only.
use super::*;
use crate::{
    ai::{
        credentials::CredentialError,
        prepared_glossary::{
            PreparedGlossaryAnalyzer, PreparedGlossaryFuture, PreparedGlossaryProposal,
            PreparedGlossaryRequest, PreparedGlossaryResponse,
        },
        speech::SpeechLanguage,
        translation::{
            TranslationCancellation, TranslationCapabilities, TranslationEmitter, TranslationError,
            TranslationFuture, TranslationPass, TranslationRequest, Translator,
        },
    },
    domain::TranslationQuality,
    media::input::{LocalSubtitleInput, MediaInput},
    resolver::timed_subtitle::SubtitleFormat,
};
use std::{
    fs,
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use crate::media::work::TEST_OWNERS as OWNERS;
struct Fixture {
    directory: tempfile::TempDir,
    media: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let media = directory.path().join("synthetic.media");
        fs::write(&media, b"opaque synthetic bytes, not captured audio").unwrap();
        Self { directory, media }
    }
    fn request(&self, language: TextLanguage) -> PreparedRequest {
        self.with_text(language, "1\n01:02:03,004 --> 01:02:05,006\n软件与服务器\n")
    }
    fn with_text(&self, language: TextLanguage, text: &str) -> PreparedRequest {
        let subtitle = self.directory.path().join("explicit.srt");
        fs::write(&subtitle, text).unwrap();
        PreparedRequest::new(
            MediaInput::local(self.media.clone()).unwrap(),
            vec![LocalSubtitleInput::new(subtitle, SubtitleFormat::Srt, language).unwrap()],
        )
        .unwrap()
    }
    fn chinese(&self) -> PreparedRequest {
        self.request(TextLanguage::SimplifiedChinese)
    }
    fn source(&self) -> PreparedRequest {
        self.request(TextLanguage::Source(SpeechLanguage::English))
    }
    #[cfg(windows)]
    fn embedded(&self, language: TextLanguage) -> PreparedRequest {
        fs::write(
            &self.media,
            crate::media::local_audio_format::mp4_prepared_fixture(language),
        )
        .unwrap();
        PreparedRequest::new(MediaInput::local(self.media.clone()).unwrap(), vec![])
            .unwrap()
            .with_embedded_subtitle(
                crate::media::embedded_mp4::Mp4SubtitleSelection::new(3, language).unwrap(),
            )
            .unwrap()
    }
}

#[cfg(windows)]
#[path = "prepared_embedded_tests.rs"]
mod embedded;
#[path = "prepared_https_guard_tests.rs"]
mod https_guard;
fn options(provider: Option<SpeechProvider>) -> PreparedOptions {
    PreparedOptions {
        provider,
        quality: TranslationQuality::Fast,
        source_language: SourceLanguage::Auto,
    }
}
fn key() -> Option<ApiKey> {
    Some(ApiKey::from_text("SYNTHETIC_PREPARED_TEST_KEY").unwrap())
}
fn terminal(controller: &mut PreparedController) -> PreparedUiState {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let state = controller.poll();
        if !matches!(
            state,
            PreparedUiState::Preparing | PreparedUiState::Stopping
        ) {
            return state;
        }
        assert!(
            Instant::now() < deadline,
            "mock Prepared worker did not stop"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn finished(controller: &PreparedController) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !controller.worker.as_ref().unwrap().thread.is_finished() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
}
type SeenGlossary = (TranslationPass, Vec<(String, String)>);
#[derive(Default)]
struct Stats {
    calls: AtomicUsize,
    analysis_calls: AtomicUsize,
    dropped: AtomicUsize,
    qualities: Mutex<Vec<TranslationQuality>>,
    stages: Mutex<Vec<&'static str>>,
    glossaries: Mutex<Vec<SeenGlossary>>,
}

#[derive(Clone, Copy)]
enum AnalysisMode {
    Empty,
    InvalidSummary,
    Grounded,
    Cropped,
    Fail(TranslationError),
    Pending,
}
enum Behavior {
    Success,
    Fail(PreparedUiError),
    Panic,
    Pending(Mutex<Option<mpsc::SyncSender<TranslationCancellation>>>),
    Gate(Arc<std::sync::Barrier>),
    Analysis(AnalysisMode),
    AnalysisPending(Mutex<Option<mpsc::SyncSender<TranslationCancellation>>>),
}
struct Factory {
    builds: AtomicUsize,
    providers: Mutex<Vec<SpeechProvider>>,
    explicit_keys: AtomicUsize,
    behavior: Behavior,
    stats: Arc<Stats>,
}
impl Factory {
    fn new(behavior: Behavior) -> Arc<Self> {
        Arc::new(Self {
            builds: AtomicUsize::new(0),
            providers: Mutex::new(Vec::new()),
            explicit_keys: AtomicUsize::new(0),
            behavior,
            stats: Arc::new(Stats::default()),
        })
    }
}
impl SelectedPreparedServiceFactory for Factory {
    fn build(
        &self,
        provider: SpeechProvider,
        key: Option<ApiKey>,
        kind: PreparedServiceKind,
    ) -> Result<PreparedServices, PreparedUiError> {
        assert!(kind == PreparedServiceKind::Translation);
        self.builds.fetch_add(1, Ordering::SeqCst);
        self.providers.lock().unwrap().push(provider);
        if let Some(key) = key {
            assert_eq!(key.expose(), "SYNTHETIC_PREPARED_TEST_KEY");
            self.explicit_keys.fetch_add(1, Ordering::SeqCst);
        }
        let (started, analysis, analysis_started) = match &self.behavior {
            Behavior::Success => (None, AnalysisMode::Empty, None),
            Behavior::Fail(error) => return Err(*error),
            Behavior::Panic => panic!("synthetic mock factory panic"),
            Behavior::Pending(started) => {
                (started.lock().unwrap().take(), AnalysisMode::Empty, None)
            }
            Behavior::Gate(gate) => {
                gate.wait();
                gate.wait();
                (None, AnalysisMode::Empty, None)
            }
            Behavior::Analysis(mode) => (None, *mode, None),
            Behavior::AnalysisPending(started) => {
                (None, AnalysisMode::Pending, started.lock().unwrap().take())
            }
        };
        Ok(PreparedServices::Translation(Box::new(MockTranslator {
            stats: self.stats.clone(),
            started: Mutex::new(started),
            analysis,
            analysis_started: Mutex::new(analysis_started),
        })))
    }
}
struct MockTranslator {
    stats: Arc<Stats>,
    started: Mutex<Option<mpsc::SyncSender<TranslationCancellation>>>,
    analysis: AnalysisMode,
    analysis_started: Mutex<Option<mpsc::SyncSender<TranslationCancellation>>>,
}
impl Drop for MockTranslator {
    fn drop(&mut self) {
        self.stats.dropped.fetch_add(1, Ordering::SeqCst);
    }
}
impl Translator for MockTranslator {
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
            if matches!(self.analysis, AnalysisMode::Cropped) {
                assert_eq!(request.source.text.as_ref(), "Aurora Engine two");
                assert_eq!(request.source.start_us, Some(250_000));
                assert_eq!(request.source.duration_us, Some(500_000));
            }
            assert_eq!(
                request
                    .prepared_context
                    .as_ref()
                    .unwrap()
                    .document
                    .as_ref()
                    .unwrap()
                    .summary(),
                "合成全文脈絡"
            );
            self.stats.calls.fetch_add(1, Ordering::SeqCst);
            self.stats.qualities.lock().unwrap().push(request.quality);
            self.stats.stages.lock().unwrap().push("translation");
            self.stats.glossaries.lock().unwrap().push((
                request.pass,
                request
                    .glossary
                    .iter()
                    .map(|entry| {
                        (
                            entry.source.to_string(),
                            entry.taiwan_translation.to_string(),
                        )
                    })
                    .collect(),
            ));
            let started = self.started.lock().unwrap().take();
            if let Some(started) = started {
                started.send(cancel).unwrap();
                futures_util::future::pending::<()>().await;
            }
            Ok(format!("臺灣譯文 {}", request.source.sequence).into_boxed_str())
        })
    }
}

impl PreparedGlossaryAnalyzer for MockTranslator {
    fn analyze<'a>(
        &'a self,
        request: &'a PreparedGlossaryRequest,
        cancel: TranslationCancellation,
    ) -> PreparedGlossaryFuture<'a> {
        Box::pin(async move {
            self.stats.analysis_calls.fetch_add(1, Ordering::SeqCst);
            self.stats.stages.lock().unwrap().push("analysis");
            let proposals = match self.analysis {
                AnalysisMode::Empty | AnalysisMode::InvalidSummary => Vec::new(),
                AnalysisMode::Grounded | AnalysisMode::Cropped => {
                    if matches!(self.analysis, AnalysisMode::Cropped) {
                        assert_eq!(request.cues().len(), 1);
                        assert_eq!(request.cues()[0].text.as_ref(), "Aurora Engine two");
                    }
                    let cue = &request.cues()[0];
                    let start = cue.text.find("Aurora Engine").unwrap();
                    vec![PreparedGlossaryProposal {
                        cue_index: cue.cue_index,
                        source_byte_start: start,
                        source_byte_end: start + "Aurora Engine".len(),
                        taiwan_translation: "极光引擎".into(),
                    }]
                }
                AnalysisMode::Fail(error) => return Err(error),
                AnalysisMode::Pending => {
                    let started = self.analysis_started.lock().unwrap().take().unwrap();
                    started.send(cancel).unwrap();
                    futures_util::future::pending::<()>().await;
                    Vec::new()
                }
            };
            Ok(PreparedGlossaryResponse {
                identity: request.identity(),
                sequence: request.sequence(),
                proposals,
                context_summary: if matches!(self.analysis, AnalysisMode::InvalidSummary) {
                    "".into()
                } else {
                    "合成全文脈絡".into()
                },
            })
        })
    }
}

#[test]
fn chinese_prepares_ram_ready_without_building_a_provider_or_using_a_key() {
    let _serial = OWNERS.lock().unwrap();
    let fixture = Fixture::new();
    let factory = Factory::new(Behavior::Panic);
    let mut controller = PreparedController::with_factory(factory.clone());
    assert_eq!(controller.state(), PreparedUiState::Idle);
    controller
        .start(
            fixture.chinese(),
            options(Some(SpeechProvider::Gemini)),
            key(),
        )
        .unwrap();
    assert_eq!(controller.state(), PreparedUiState::Preparing);
    assert!(controller.ready().is_none());
    assert_eq!(
        terminal(&mut controller),
        PreparedUiState::Ready { cues: 1 }
    );
    let document = controller.ready().unwrap();
    assert!(document.translated()[0].contains("軟體"));
    assert!(document.translated()[0].contains("伺服器"));
    assert_eq!(
        document.original().existing().unwrap().cues[0].start_us,
        Some(3_723_004_000)
    );
    assert_eq!(factory.builds.load(Ordering::SeqCst), 0);
    assert!(controller.worker.is_none());
    assert_eq!(fs::read_dir(fixture.directory.path()).unwrap().count(), 2);
    controller.stop();
    assert_eq!(controller.state(), PreparedUiState::Cancelled);
    assert!(controller.ready().is_none());
}

#[test]
fn source_requires_an_explicit_provider_before_any_worker_or_factory() {
    let _serial = OWNERS.lock().unwrap();
    let fixture = Fixture::new();
    let factory = Factory::new(Behavior::Success);
    let mut controller = PreparedController::with_factory(factory.clone());
    assert_eq!(
        controller.start(fixture.source(), options(None), key()),
        Err(PreparedUiError::ProviderRequired)
    );
    assert_eq!(
        controller.state(),
        PreparedUiState::Failed(PreparedUiError::ProviderRequired)
    );
    assert!(controller.worker.is_none());
    assert!(controller.session.identity().is_none());
    assert_eq!(factory.builds.load(Ordering::SeqCst), 0);
}

#[test]
fn fuzzy_ready_controller_bridge_is_passive_and_revoked_on_stop() {
    use crate::{
        ai::speech::{StreamIdentity, TranscriptEvent, TranscriptKind},
        audio::capture::SessionId,
        resolver::contracts::{ResolverCancellation, ResolverError, WindowStamp},
        subtitle::fuzzy_alignment::FuzzyAlignmentError,
    };
    let _serial = OWNERS.lock().unwrap();
    let fixture = Fixture::new();
    let factory = Factory::new(Behavior::Success);
    let mut controller = PreparedController::with_factory(factory.clone());
    let capture = StreamIdentity {
        session: SessionId(991_117),
        generation: 2,
    };
    assert_eq!(
        controller.fuzzy_locator(capture, 1).err(),
        Some(FuzzyAlignmentError::NotReady)
    );
    assert_eq!(factory.builds.load(Ordering::SeqCst), 0);
    let text = "the synthetic subtitle belongs to this document";
    let request = fixture.with_text(
        TextLanguage::Source(SpeechLanguage::English),
        &format!("1\n00:10:00,000 --> 00:10:02,000\n{text}\n"),
    );
    controller
        .start(request, options(Some(SpeechProvider::OpenAi)), key())
        .unwrap();
    assert_eq!(
        controller.fuzzy_locator(capture, 1).err(),
        Some(FuzzyAlignmentError::NotReady)
    );
    assert_eq!(
        terminal(&mut controller),
        PreparedUiState::Ready { cues: 1 }
    );
    let before = factory.stats.calls.load(Ordering::SeqCst);
    let locator = controller.fuzzy_locator(capture, 1).unwrap();
    let proposal = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(locator.locate(
            TranscriptEvent {
                identity: capture,
                utterance: 2,
                sequence: 7,
                revision: 1,
                start_us: Some(42_000_000),
                duration_us: Some(2_000_000),
                language: Some(SpeechLanguage::English),
                kind: TranscriptKind::Final,
                text: text.into(),
            },
            WindowStamp {
                identity: capture,
                sequence: 9,
                ended_at: Instant::now(),
                start_us: Some(40_000_000),
                duration_us: 12_000_000,
            },
            ResolverCancellation::default(),
        ))
        .unwrap();
    assert_eq!(proposal.media_range_us(), 600_000_000..602_000_000);
    assert_eq!(factory.builds.load(Ordering::SeqCst), 1);
    assert_eq!(factory.stats.calls.load(Ordering::SeqCst), before);
    assert_eq!(controller.state(), PreparedUiState::Ready { cues: 1 });
    controller.stop();
    assert_eq!(
        proposal.validate(locator.scope(), proposal.binding()),
        Err(ResolverError::Cancelled.into())
    );
    assert_eq!(
        controller.fuzzy_locator(capture, 2).err(),
        Some(FuzzyAlignmentError::NotReady)
    );
}

#[test]
fn explicit_selected_source_service_quality_and_real_cue_order_are_preserved() {
    let _serial = OWNERS.lock().unwrap();
    let fixture = Fixture::new();
    let factory = Factory::new(Behavior::Success);
    let mut controller = PreparedController::with_factory(factory.clone());
    let request = fixture.with_text(
        TextLanguage::Source(SpeechLanguage::Korean),
        "1\n00:00:01,000 --> 00:00:02,000\nfirst\n\n2\n00:00:03,000 --> 00:00:04,000\nsecond\n",
    );
    controller
        .start(
            request,
            PreparedOptions {
                provider: Some(SpeechProvider::Gemini),
                quality: TranslationQuality::Standard,
                source_language: SourceLanguage::Auto,
            },
            key(),
        )
        .unwrap();
    assert_eq!(
        terminal(&mut controller),
        PreparedUiState::Ready { cues: 2 }
    );
    assert_eq!(
        controller.ready().unwrap().translated(),
        &[
            Box::<str>::from("臺灣譯文 0"),
            Box::<str>::from("臺灣譯文 1")
        ]
    );
    assert_eq!(
        factory.providers.lock().unwrap().as_slice(),
        &[SpeechProvider::Gemini]
    );
    assert_eq!(factory.explicit_keys.load(Ordering::SeqCst), 1);
    assert_eq!(factory.stats.calls.load(Ordering::SeqCst), 4);
    assert!(
        factory
            .stats
            .qualities
            .lock()
            .unwrap()
            .iter()
            .all(|quality| *quality == TranslationQuality::Standard)
    );
    assert_eq!(factory.stats.dropped.load(Ordering::SeqCst), 1);
}

#[test]
fn factory_errors_do_not_fallback_and_the_next_local_run_recovers() {
    let _serial = OWNERS.lock().unwrap();
    let fixture = Fixture::new();
    for error in [
        PreparedUiError::Credential(CredentialError::Missing),
        PreparedUiError::Translation(TranslationError::Network),
    ] {
        let factory = Factory::new(Behavior::Fail(error));
        let mut controller = PreparedController::with_factory(factory.clone());
        controller
            .start(
                fixture.source(),
                options(Some(SpeechProvider::OpenAi)),
                key(),
            )
            .unwrap();
        assert_eq!(terminal(&mut controller), PreparedUiState::Failed(error));
        assert!(controller.ready().is_none());
        assert_eq!(
            factory.providers.lock().unwrap().as_slice(),
            &[SpeechProvider::OpenAi]
        );
        assert!(!error.to_string().contains("SYNTHETIC_PREPARED_TEST_KEY"));
        assert!(
            !error
                .to_string()
                .contains(&fixture.media.to_string_lossy().to_string())
        );
        controller
            .start(fixture.chinese(), options(None), None)
            .unwrap();
        assert_eq!(
            terminal(&mut controller),
            PreparedUiState::Ready { cues: 1 }
        );
        assert_eq!(factory.builds.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn factory_panic_becomes_typed_failure_and_releases_the_worker_slot() {
    let _serial = OWNERS.lock().unwrap();
    let fixture = Fixture::new();
    let factory = Factory::new(Behavior::Panic);
    let mut controller = PreparedController::with_factory(factory);
    controller
        .start(
            fixture.source(),
            options(Some(SpeechProvider::OpenAi)),
            key(),
        )
        .unwrap();
    assert_eq!(
        terminal(&mut controller),
        PreparedUiState::Failed(PreparedUiError::WorkerFailed)
    );
    assert!(controller.worker.is_none());
    assert!(controller.session.identity().is_none());
    controller
        .start(fixture.chinese(), options(None), None)
        .unwrap();
    assert_eq!(
        terminal(&mut controller),
        PreparedUiState::Ready { cues: 1 }
    );
}

#[test]
fn stop_cancels_pending_requests_but_blocks_restart_until_the_thread_is_reaped() {
    let _serial = OWNERS.lock().unwrap();
    let fixture = Fixture::new();
    let (started, received) = mpsc::sync_channel(1);
    let factory = Factory::new(Behavior::Pending(Mutex::new(Some(started))));
    let mut controller = PreparedController::with_factory(factory.clone());
    controller
        .start(
            fixture.source(),
            options(Some(SpeechProvider::OpenAi)),
            key(),
        )
        .unwrap();
    let token = received.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(
        controller.start(fixture.chinese(), options(None), None),
        Err(PreparedUiError::Busy)
    );
    controller.stop();
    assert_eq!(controller.state(), PreparedUiState::Stopping);
    assert!(controller.ready().is_none());
    assert_eq!(
        controller.start(fixture.chinese(), options(None), None),
        Err(PreparedUiError::Busy)
    );
    assert_eq!(terminal(&mut controller), PreparedUiState::Cancelled);
    assert_eq!(token.reason(), Some(TranslationError::Cancelled));
    assert_eq!(factory.stats.dropped.load(Ordering::SeqCst), 1);
    controller
        .start(fixture.chinese(), options(None), None)
        .unwrap();
    assert_eq!(
        terminal(&mut controller),
        PreparedUiState::Ready { cues: 1 }
    );
}

#[test]
fn buffered_old_completion_after_stop_cannot_publish_ready_in_a_new_session() {
    let _serial = OWNERS.lock().unwrap();
    let fixture = Fixture::new();
    let factory = Factory::new(Behavior::Success);
    let mut controller = PreparedController::with_factory(factory);
    controller
        .start(fixture.chinese(), options(None), None)
        .unwrap();
    let previous = controller.session.identity().unwrap();
    finished(&controller);
    assert_eq!(controller.state(), PreparedUiState::Preparing);
    assert_eq!(
        controller.start(fixture.chinese(), options(None), None),
        Err(PreparedUiError::Busy)
    );
    controller.stop();
    assert_eq!(terminal(&mut controller), PreparedUiState::Cancelled);
    assert!(controller.ready().is_none());
    controller
        .start(fixture.chinese(), options(None), None)
        .unwrap();
    assert_ne!(
        controller.session.identity().unwrap().session,
        previous.session
    );
    assert_eq!(
        terminal(&mut controller),
        PreparedUiState::Ready { cues: 1 }
    );
    assert_ne!(
        controller.ready().unwrap().identity().session,
        previous.session
    );
}

#[test]
fn drop_cancels_and_joins_the_owned_pending_translator() {
    let _serial = OWNERS.lock().unwrap();
    let fixture = Fixture::new();
    let (started, received) = mpsc::sync_channel(1);
    let factory = Factory::new(Behavior::Pending(Mutex::new(Some(started))));
    let mut controller = PreparedController::with_factory(factory.clone());
    controller
        .start(
            fixture.source(),
            options(Some(SpeechProvider::Gemini)),
            key(),
        )
        .unwrap();
    let token = received.recv_timeout(Duration::from_secs(5)).unwrap();
    drop(controller);
    assert_eq!(token.reason(), Some(TranslationError::Cancelled));
    assert_eq!(factory.stats.dropped.load(Ordering::SeqCst), 1);
    assert_eq!(fs::read_dir(fixture.directory.path()).unwrap().count(), 2);
}

#[test]
fn stop_during_factory_setup_keeps_stopping_until_owned_setup_exits() {
    let _serial = OWNERS.lock().unwrap();
    let fixture = Fixture::new();
    let gate = Arc::new(std::sync::Barrier::new(2));
    let factory = Factory::new(Behavior::Gate(gate.clone()));
    let mut controller = PreparedController::with_factory(factory.clone());
    controller
        .start(
            fixture.source(),
            options(Some(SpeechProvider::OpenAi)),
            key(),
        )
        .unwrap();
    gate.wait();
    controller.stop();
    assert_eq!(controller.poll(), PreparedUiState::Stopping);
    assert_eq!(
        controller.start(fixture.chinese(), options(None), None),
        Err(PreparedUiError::Busy)
    );
    gate.wait();
    assert_eq!(terminal(&mut controller), PreparedUiState::Cancelled);
    assert_eq!(factory.stats.calls.load(Ordering::SeqCst), 0);
    assert_eq!(factory.stats.dropped.load(Ordering::SeqCst), 1);
}

#[test]
fn asr_auto_rejects_local_and_remote_inputs_before_any_worker_or_cloud_service() {
    let _serial = OWNERS.lock().unwrap();
    let fixture = Fixture::new();
    let factory = Factory::new(Behavior::Panic);
    let mut controller = PreparedController::with_factory(factory.clone());
    for media in [
        MediaInput::remote("https://example.invalid/synthetic.media").unwrap(),
        MediaInput::local(fixture.media.clone()).unwrap(),
    ] {
        assert_eq!(
            controller.start(
                PreparedRequest::new(media, vec![]).unwrap(),
                options(Some(SpeechProvider::Gemini)),
                key(),
            ),
            Err(PreparedUiError::Speech(
                PreparedSpeechError::UnsupportedLanguage
            ))
        );
        assert_eq!(
            controller.state(),
            PreparedUiState::Failed(PreparedUiError::Speech(
                PreparedSpeechError::UnsupportedLanguage
            ))
        );
        assert!(controller.ready().is_none());
        assert!(controller.worker.is_none());
        assert!(controller.session.identity().is_none());
    }
    assert_eq!(factory.builds.load(Ordering::SeqCst), 0);
}

#[test]
fn invalid_subtitle_errors_keep_private_input_text_and_paths_out_of_status() {
    let _serial = OWNERS.lock().unwrap();
    let fixture = Fixture::new();
    let factory = Factory::new(Behavior::Panic);
    let mut controller = PreparedController::with_factory(factory.clone());
    let request = fixture.with_text(
        TextLanguage::TaiwanTraditional,
        "PRIVATE_SYNTHETIC_TEST_TEXT",
    );
    controller.start(request, options(None), None).unwrap();
    let state = terminal(&mut controller);
    assert_eq!(
        state,
        PreparedUiState::Failed(PreparedUiError::Media(MediaError::InvalidSubtitle))
    );
    let status = state.label();
    assert!(!status.contains("PRIVATE_SYNTHETIC_TEST_TEXT"));
    assert!(!status.contains(&fixture.media.to_string_lossy().to_string()));
    assert_eq!(factory.builds.load(Ordering::SeqCst), 0);
}

struct SyntheticCredentials {
    reads: AtomicUsize,
    providers: Mutex<Vec<SpeechProvider>>,
    missing: bool,
}
impl CredentialStore for SyntheticCredentials {
    fn read(&self, provider: SpeechProvider) -> Result<ApiKey, CredentialError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.providers.lock().unwrap().push(provider);
        if self.missing {
            Err(CredentialError::Missing)
        } else {
            Ok(key().unwrap())
        }
    }
}
#[test]
fn real_http_service_construction_resolves_one_selected_synthetic_key_for_both_clients() {
    for provider in [SpeechProvider::OpenAi, SpeechProvider::Gemini] {
        let store = SyntheticCredentials {
            reads: AtomicUsize::new(0),
            providers: Mutex::new(Vec::new()),
            missing: false,
        };
        let services = HttpPreparedServiceFactory::build_with_store(
            provider,
            None,
            PreparedServiceKind::Recognition,
            &store,
        )
        .unwrap();
        let PreparedServices::Recognition {
            recognizer,
            translator,
        } = services
        else {
            panic!("ASR construction returned translation-only services");
        };
        assert_eq!(store.reads.load(Ordering::SeqCst), 1);
        assert_eq!(store.providers.lock().unwrap().as_slice(), &[provider]);
        assert_eq!(
            recognizer
                .capabilities()
                .validate(SourceLanguage::Auto, PreparedSampleRate::Hz16000),
            Err(PreparedSpeechError::UnsupportedLanguage)
        );
        assert!(
            recognizer
                .capabilities()
                .supported_languages
                .into_iter()
                .all(|supported| supported)
        );
        assert!(
            translator
                .capabilities()
                .supported_languages
                .into_iter()
                .all(|supported| supported)
        );
        // Client construction has no async request, media input or real store.
    }
}

#[test]
fn explicit_synthetic_key_bypasses_credential_store_for_each_service_route() {
    for provider in [SpeechProvider::OpenAi, SpeechProvider::Gemini] {
        for kind in [
            PreparedServiceKind::Translation,
            PreparedServiceKind::Recognition,
        ] {
            let store = SyntheticCredentials {
                reads: AtomicUsize::new(0),
                providers: Mutex::new(Vec::new()),
                missing: true,
            };
            let services =
                HttpPreparedServiceFactory::build_with_store(provider, key(), kind, &store)
                    .unwrap();
            assert!(matches!(
                (services, kind),
                (
                    PreparedServices::Translation(_),
                    PreparedServiceKind::Translation
                ) | (
                    PreparedServices::Recognition { .. },
                    PreparedServiceKind::Recognition
                )
            ));
            assert_eq!(store.reads.load(Ordering::SeqCst), 0);
        }
    }
}

#[test]
fn missing_synthetic_credential_is_typed_and_never_retried_at_construction() {
    let store = SyntheticCredentials {
        reads: AtomicUsize::new(0),
        providers: Mutex::new(Vec::new()),
        missing: true,
    };
    assert!(matches!(
        HttpPreparedServiceFactory::build_with_store(
            SpeechProvider::Gemini,
            None,
            PreparedServiceKind::Recognition,
            &store,
        ),
        Err(PreparedUiError::Credential(CredentialError::Missing))
    ));
    assert_eq!(store.reads.load(Ordering::SeqCst), 1);
}

#[test]
fn controller_analyzes_source_before_draft_and_shares_localized_terms_with_review() {
    let _serial = OWNERS.lock().unwrap();
    let fixture = Fixture::new();
    for provider in [SpeechProvider::OpenAi, SpeechProvider::Gemini] {
        let factory = Factory::new(Behavior::Analysis(AnalysisMode::Grounded));
        let mut controller = PreparedController::with_factory(factory.clone());
        let mut selected = options(Some(provider));
        selected.quality = TranslationQuality::Standard;
        controller.start(fixture.with_text(TextLanguage::Source(SpeechLanguage::English),
            "1\n00:00:01,000 --> 00:00:02,000\nAurora Engine starts.\n\n2\n00:00:03,000 --> 00:00:04,000\nThe engine runs.\n"), selected, key()).unwrap();
        assert_eq!(
            terminal(&mut controller),
            PreparedUiState::Ready { cues: 2 }
        );
        assert_eq!(factory.stats.analysis_calls.load(Ordering::SeqCst), 1);
        assert_eq!(factory.stats.calls.load(Ordering::SeqCst), 4);
        assert_eq!(factory.stats.stages.lock().unwrap()[0], "analysis");
        let seen = factory.stats.glossaries.lock().unwrap();
        assert_eq!(
            seen.iter()
                .filter(|row| row.0 == TranslationPass::Draft)
                .count(),
            2
        );
        assert_eq!(
            seen.iter()
                .filter(|row| row.0 == TranslationPass::Review)
                .count(),
            2
        );
        assert!(
            seen.iter()
                .all(|row| row.1 == vec![("Aurora Engine".into(), "極光引擎".into())])
        );
        assert_eq!(
            controller
                .ready()
                .unwrap()
                .original()
                .existing()
                .unwrap()
                .cues[1]
                .start_us,
            Some(3_000_000)
        );
    }
}

#[test]
fn analysis_failure_never_translates_or_publishes_ready() {
    let _serial = OWNERS.lock().unwrap();
    let fixture = Fixture::new();
    let factory = Factory::new(Behavior::Analysis(AnalysisMode::Fail(
        TranslationError::Credential,
    )));
    let mut controller = PreparedController::with_factory(factory.clone());
    controller
        .start(
            fixture.source(),
            options(Some(SpeechProvider::Gemini)),
            key(),
        )
        .unwrap();
    assert_eq!(
        terminal(&mut controller),
        PreparedUiState::Failed(PreparedUiError::Translation(TranslationError::Credential))
    );
    assert_eq!(factory.stats.analysis_calls.load(Ordering::SeqCst), 1);
    assert_eq!(factory.stats.calls.load(Ordering::SeqCst), 0);
    assert!(controller.ready().is_none());
    assert!(controller.worker.is_none());
    assert!(
        std::fs::OpenOptions::new()
            .write(true)
            .open(&fixture.media)
            .is_ok()
    );
}

#[test]
fn stop_during_analysis_cancels_request_and_releases_pin_before_restart() {
    let _serial = OWNERS.lock().unwrap();
    let fixture = Fixture::new();
    let (started, received) = mpsc::sync_channel(1);
    let factory = Factory::new(Behavior::AnalysisPending(Mutex::new(Some(started))));
    let mut controller = PreparedController::with_factory(factory.clone());
    controller
        .start(
            fixture.source(),
            options(Some(SpeechProvider::OpenAi)),
            key(),
        )
        .unwrap();
    let token = received.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(
        std::fs::OpenOptions::new()
            .write(true)
            .open(&fixture.media)
            .is_err()
    );
    controller.stop();
    assert_eq!(terminal(&mut controller), PreparedUiState::Cancelled);
    assert_eq!(token.reason(), Some(TranslationError::Cancelled));
    assert_eq!(factory.stats.calls.load(Ordering::SeqCst), 0);
    assert!(controller.ready().is_none());
    assert!(
        std::fs::OpenOptions::new()
            .write(true)
            .open(&fixture.media)
            .is_ok()
    );
    controller
        .start(fixture.chinese(), options(None), None)
        .unwrap();
    assert_eq!(
        terminal(&mut controller),
        PreparedUiState::Ready { cues: 1 }
    );
    assert_eq!(factory.stats.analysis_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn phase11_cancel_error_and_ready_clear_leave_only_owned_fixture_inputs_then_release_all_pins() {
    let _serial = OWNERS.lock().unwrap();
    for mode in 0..3 {
        let fixture = Fixture::new();
        let root = fixture.directory.path().canonicalize().unwrap();
        let temp_root = std::env::temp_dir().canonicalize().unwrap();
        assert!(root.starts_with(&temp_root) && root != temp_root);
        let (entered, received) = mpsc::sync_channel(1);
        let behavior = match mode {
            0 => Behavior::Pending(Mutex::new(Some(entered))),
            1 => Behavior::Fail(PreparedUiError::Credential(CredentialError::Missing)),
            _ => Behavior::Success,
        };
        let factory = Factory::new(behavior);
        let mut controller = PreparedController::with_factory(factory);
        controller
            .start(
                fixture.source(),
                options(Some(SpeechProvider::OpenAi)),
                key(),
            )
            .unwrap();
        if mode == 0 {
            let token = received.recv_timeout(Duration::from_secs(5)).unwrap();
            controller.stop();
            assert_eq!(terminal(&mut controller), PreparedUiState::Cancelled);
            assert_eq!(token.reason(), Some(TranslationError::Cancelled));
        } else {
            let state = terminal(&mut controller);
            if mode == 1 {
                assert_eq!(
                    state,
                    PreparedUiState::Failed(PreparedUiError::Credential(CredentialError::Missing))
                );
            } else {
                assert_eq!(state, PreparedUiState::Ready { cues: 1 });
                controller.stop();
            }
        }
        drop(controller);
        let mut names: Vec<_> = fs::read_dir(&root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                std::ffi::OsString::from("explicit.srt"),
                std::ffi::OsString::from("synthetic.media")
            ]
        );
        // TempDir owns this exact unique fixture only. Successful removal on
        // Windows proves that cancellation/error/Ready clear released file pins.
        drop(fixture);
        assert!(!root.exists());
    }
}
