//! Synthetic caller-owned WAVE/MP3 files and explicitly injected providers only.
//! No real credentials, private media, provider network or content persistence.
#![cfg(windows)]
#[path = "prepared_https_tests.rs"]
mod https;
use super::*;
use crate::{
    ai::{
        prepared_glossary::{
            PreparedGlossaryAnalyzer, PreparedGlossaryFuture, PreparedGlossaryProposal,
            PreparedGlossaryRequest, PreparedGlossaryResponse,
        },
        prepared_speech::{
            PreparedSpeechCapabilities, PreparedSpeechCue, PreparedSpeechFuture,
            PreparedSpeechRequest, PreparedSpeechResponse,
        },
        speech::{SpeechLanguage, StreamIdentity},
        translation::{
            TranslationCancellation, TranslationCapabilities, TranslationEmitter, TranslationError,
            TranslationFuture, TranslationPass, TranslationRequest, Translator,
        },
    },
    audio::capture::SessionId,
    domain::TranslationQuality,
    media::{
        decoded::MediaDecoder,
        input::{LocalSubtitleInput, MediaInput},
        work,
    },
    resolver::{contracts::ResolverCancellation, timed_subtitle::SubtitleFormat},
};
use std::{
    fs::{self, OpenOptions},
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::Notify;

const TEST_TIMEOUT: Duration = Duration::from_secs(5);
const MP3_TEST_TIMEOUT: Duration = Duration::from_secs(30);
// This native codec fixture can decode a twelve-second ASR window before the
// injected provider failure/start signal. Match the existing long-codec budget;
// keep cancellation and Chinese subtitle-only processing at TEST_TIMEOUT.
const NATIVE_CODEC_START_TIMEOUT: Duration = MP3_TEST_TIMEOUT;
const MAX_ORACLE_ROWS: usize = 1024;
const SYNTHETIC_FRAMES: usize = 1600;
type SpeechStarted = Arc<Mutex<Option<mpsc::SyncSender<ResolverCancellation>>>>;
type TranslationStarted = Arc<Mutex<Option<mpsc::SyncSender<TranslationCancellation>>>>;

struct Fixture {
    directory: tempfile::TempDir,
    media: PathBuf,
}
impl Fixture {
    fn mp3() -> Self {
        Self::mp3_frames(44_100, 1, 44_100)
    }
    fn mp3_frames(rate: u32, channels: u16, frames: usize) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let media = directory.path().join("synthetic.mp3");
        let bytes = crate::win32::media_mp3_fixture::synthetic_mp3(rate, channels, frames).unwrap();
        fs::write(&media, bytes).unwrap();
        Self { directory, media }
    }
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let media = directory.path().join("synthetic.wav");
        let data_bytes = (SYNTHETIC_FRAMES * 2) as u32;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_bytes).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&16_000u32.to_le_bytes());
        bytes.extend_from_slice(&32_000u32.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_bytes.to_le_bytes());
        for _ in 0..SYNTHETIC_FRAMES {
            bytes.extend_from_slice(&901i16.to_le_bytes());
        }
        fs::write(&media, bytes).unwrap();
        Self { directory, media }
    }
    fn request(&self, subtitles: Vec<LocalSubtitleInput>) -> PreparedRequest {
        PreparedRequest::new(MediaInput::local(self.media.clone()).unwrap(), subtitles).unwrap()
    }
    fn subtitle(&self, name: &str, language: TextLanguage) -> LocalSubtitleInput {
        let path = self.directory.path().join(name);
        fs::write(&path, "1\n00:00:00,004 --> 00:00:00,016\n软件与服务器\n").unwrap();
        LocalSubtitleInput::new(path, SubtitleFormat::Srt, language).unwrap()
    }
    fn writable(&self) -> bool {
        OpenOptions::new().write(true).open(&self.media).is_ok()
    }
}
fn options(provider: Option<SpeechProvider>, language: SourceLanguage) -> PreparedOptions {
    PreparedOptions {
        provider,
        quality: TranslationQuality::Standard,
        source_language: language,
    }
}
fn key() -> Option<ApiKey> {
    Some(ApiKey::from_text("SYNTHETIC_PREPARED_ASR_KEY").unwrap())
}
fn terminal(controller: &mut PreparedController) -> PreparedUiState {
    terminal_with_timeout(controller, TEST_TIMEOUT)
}
fn terminal_with_timeout(
    controller: &mut PreparedController,
    timeout: Duration,
) -> PreparedUiState {
    let deadline = Instant::now() + timeout;
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
            "synthetic Prepared worker did not exit"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn finished(controller: &PreparedController) {
    let deadline = Instant::now() + TEST_TIMEOUT;
    while !controller.worker.as_ref().unwrap().thread.is_finished() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[derive(Clone)]
enum SpeechMode {
    Success,
    BodySuccess,
    Unsupported,
    Fail(PreparedSpeechError),
    Pending(SpeechStarted),
    Late(SpeechStarted, Arc<Notify>),
}
#[derive(Clone)]
enum TextMode {
    Success,
    AnalysisTerms,
    AnalysisFailure,
    Fail(TranslationError),
    Pending(TranslationStarted),
}
#[derive(Default)]
struct Stats {
    forbid_file_metadata: AtomicBool,
    builds: AtomicUsize,
    keys: AtomicUsize,
    selected: Mutex<Vec<(SpeechProvider, PreparedServiceKind)>>,
    speech_capabilities: AtomicUsize,
    speech_calls: AtomicUsize,
    speech_dropped: AtomicUsize,
    speech_futures_dropped: AtomicUsize,
    speech_selection: Mutex<Vec<(SourceLanguage, PreparedSampleRate)>>,
    speech_intervals: Mutex<Vec<RecordedInterval>>,
    translation_calls: AtomicUsize,
    analysis_calls: AtomicUsize,
    translation_dropped: AtomicUsize,
    passes: Mutex<Vec<TranslationPass>>,
}
fn assert_no_file_metadata(text: &str) {
    for marker in [
        "臺灣合成標題",
        "synthetic artist",
        "synthetic album",
        "127.0.0.1",
        "synthetic-never-fetch",
    ] {
        assert!(
            !text.contains(marker),
            "file metadata crossed a provider boundary"
        );
    }
}
#[derive(Clone, Copy)]
struct RecordedInterval {
    sequence: u64,
    source_run: Option<u64>,
    start_us: u64,
    end_us: u64,
}
struct Factory {
    speech: SpeechMode,
    text: TextMode,
    setup_error: Option<PreparedUiError>,
    wrong_kind: bool,
    stats: Arc<Stats>,
}
impl Factory {
    fn new(speech: SpeechMode, text: TextMode) -> Arc<Self> {
        Arc::new(Self {
            speech,
            text,
            setup_error: None,
            wrong_kind: false,
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
        self.stats.builds.fetch_add(1, Ordering::SeqCst);
        self.stats.selected.lock().unwrap().push((provider, kind));
        let key = key.expect("explicit mock must never read a real credential");
        assert_eq!(key.expose(), "SYNTHETIC_PREPARED_ASR_KEY");
        self.stats.keys.fetch_add(1, Ordering::SeqCst);
        if let Some(error) = self.setup_error {
            return Err(error);
        }
        let translator = Box::new(TranslatorMock {
            mode: self.text.clone(),
            stats: self.stats.clone(),
        });
        if kind == PreparedServiceKind::Translation || self.wrong_kind {
            return Ok(PreparedServices::Translation(translator));
        }
        Ok(PreparedServices::Recognition {
            recognizer: Box::new(Recognizer {
                mode: self.speech.clone(),
                stats: self.stats.clone(),
            }),
            translator,
        })
    }
}
struct Recognizer {
    mode: SpeechMode,
    stats: Arc<Stats>,
}
impl Drop for Recognizer {
    fn drop(&mut self) {
        self.stats.speech_dropped.fetch_add(1, Ordering::SeqCst);
    }
}
struct SpeechFutureGuard(Arc<Stats>);
impl Drop for SpeechFutureGuard {
    fn drop(&mut self) {
        self.0.speech_futures_dropped.fetch_add(1, Ordering::SeqCst);
    }
}
impl PreparedSpeechRecognizer for Recognizer {
    fn capabilities(&self) -> PreparedSpeechCapabilities {
        self.stats
            .speech_capabilities
            .fetch_add(1, Ordering::SeqCst);
        PreparedSpeechCapabilities {
            cancellation: !matches!(self.mode, SpeechMode::Unsupported),
            timestamped_finals: true,
            supported_languages: [true; 5],
            constrained_auto_detection: false,
            sample_rates: [true; 2],
            max_concurrent: 2,
        }
    }
    fn recognize<'a>(
        &'a self,
        request: &'a PreparedSpeechRequest,
        cancel: ResolverCancellation,
    ) -> PreparedSpeechFuture<'a> {
        let guard = SpeechFutureGuard(self.stats.clone());
        Box::pin(async move {
            let _guard = guard;
            self.stats.speech_calls.fetch_add(1, Ordering::SeqCst);
            self.stats
                .speech_selection
                .lock()
                .unwrap()
                .push((request.language(), request.chunk().stamp().rate));
            match &self.mode {
                SpeechMode::Fail(error) => return Err(*error),
                SpeechMode::Pending(started) => {
                    if let Some(started) = started.lock().unwrap().take() {
                        started.send(cancel).unwrap();
                    }
                    futures_util::future::pending::<()>().await;
                }
                SpeechMode::Late(started, release) => {
                    if let Some(started) = started.lock().unwrap().take() {
                        started.send(cancel).unwrap();
                    }
                    // Deliberately ignores cancellation to test the caller's
                    // cancellation and late-result boundary with an explicit mock.
                    release.notified().await;
                }
                _ => {}
            }
            let stamp = request.chunk().stamp();
            let (start_us, duration_us) = if matches!(self.mode, SpeechMode::BodySuccess) {
                (stamp.body_start_us, stamp.body_end_us - stamp.body_start_us)
            } else {
                (stamp.body_start_us + 4000, 12_000)
            };
            self.stats
                .speech_intervals
                .lock()
                .unwrap()
                .push(RecordedInterval {
                    sequence: stamp.sequence,
                    source_run: request
                        .chunk()
                        .source_clock()
                        .map(|clock| clock.source_run_id()),
                    start_us,
                    end_us: start_us + duration_us,
                });
            Ok(PreparedSpeechResponse {
                identity: stamp.identity,
                sequence: stamp.sequence,
                run_id: stamp.run_id,
                language: SpeechLanguage::selected(request.language()),
                cues: vec![PreparedSpeechCue {
                    start_us: start_us - stamp.audio_start_us,
                    duration_us,
                    text: "synthetic recognized dialogue".into(),
                }],
            })
        })
    }
}
struct TranslatorMock {
    mode: TextMode,
    stats: Arc<Stats>,
}
impl PreparedGlossaryAnalyzer for TranslatorMock {
    fn analyze<'a>(
        &'a self,
        request: &'a PreparedGlossaryRequest,
        _: TranslationCancellation,
    ) -> PreparedGlossaryFuture<'a> {
        Box::pin(async move {
            self.stats.analysis_calls.fetch_add(1, Ordering::SeqCst);
            if self.stats.forbid_file_metadata.load(Ordering::SeqCst) {
                for cue in request.cues() {
                    assert_no_file_metadata(&cue.text);
                }
                for term in request.known_terms() {
                    assert_no_file_metadata(&term.source);
                    assert_no_file_metadata(&term.taiwan_translation);
                }
            }
            if matches!(
                self.mode,
                TextMode::AnalysisTerms | TextMode::AnalysisFailure
            ) {
                assert!(self.stats.speech_calls.load(Ordering::SeqCst) > 0);
                assert_eq!(self.stats.translation_calls.load(Ordering::SeqCst), 0);
            }
            if matches!(self.mode, TextMode::AnalysisFailure) {
                return Err(TranslationError::Refused);
            }
            let proposals = if matches!(self.mode, TextMode::AnalysisTerms) {
                assert!(request.cues()[0].text.starts_with("synthetic"));
                vec![PreparedGlossaryProposal {
                    cue_index: request.cues()[0].cue_index,
                    source_byte_start: 0,
                    source_byte_end: 9,
                    taiwan_translation: "合成内容".into(),
                }]
            } else {
                vec![]
            };
            Ok(PreparedGlossaryResponse {
                identity: request.identity(),
                sequence: request.sequence(),
                proposals,
                context_summary: "合成全文脈絡".into(),
            })
        })
    }
}
impl Drop for TranslatorMock {
    fn drop(&mut self) {
        self.stats
            .translation_dropped
            .fetch_add(1, Ordering::SeqCst);
    }
}
impl Translator for TranslatorMock {
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
            let document = &request.prepared_context.as_ref().unwrap().document;
            if self.stats.analysis_calls.load(Ordering::SeqCst) > 0 {
                assert_eq!(document.as_ref().unwrap().summary(), "合成全文脈絡");
            } else {
                // Explicit manual library runs skip automatic analysis. The
                // same mock validates that they do not acquire a fake summary.
                assert!(document.is_none());
            }
            self.stats.translation_calls.fetch_add(1, Ordering::SeqCst);
            if self.stats.forbid_file_metadata.load(Ordering::SeqCst) {
                assert_no_file_metadata(&request.source.text);
                assert_no_file_metadata(&request.committed_prefix);
                if let Some(draft) = &request.draft {
                    assert_no_file_metadata(draft);
                }
                for stable in &request.previous_stable {
                    assert_no_file_metadata(&stable.source);
                    assert_no_file_metadata(&stable.translation);
                }
                for term in &request.glossary {
                    assert_no_file_metadata(&term.source);
                    assert_no_file_metadata(&term.taiwan_translation);
                }
                if let Some(context) = &request.prepared_context {
                    for source in context
                        .previous_source
                        .iter()
                        .chain(&context.following_source)
                    {
                        assert_no_file_metadata(source);
                    }
                    for draft in &context.neighboring_drafts {
                        assert_no_file_metadata(&draft.source);
                        assert_no_file_metadata(&draft.translation);
                    }
                }
            }
            self.stats.passes.lock().unwrap().push(request.pass);
            match &self.mode {
                TextMode::Fail(error) => return Err(*error),
                TextMode::Pending(started) => {
                    if let Some(started) = started.lock().unwrap().take() {
                        started.send(cancel).unwrap();
                    }
                    futures_util::future::pending::<()>().await;
                }
                TextMode::Success | TextMode::AnalysisFailure => {}
                TextMode::AnalysisTerms => {
                    assert_eq!(request.glossary.len(), 1);
                    assert_eq!(request.glossary[0].source.as_ref(), "synthetic");
                    assert_eq!(request.glossary[0].taiwan_translation.as_ref(), "合成內容");
                }
            }
            Ok("软件与服务器".into())
        })
    }
}

#[test]
fn each_selected_provider_and_five_languages_reach_genuine_asr_ready_with_one_key() {
    let _serial = work::TEST_OWNERS.lock().unwrap();
    let fixture = Fixture::new();
    for provider in [SpeechProvider::OpenAi, SpeechProvider::Gemini] {
        for language in [
            SourceLanguage::English,
            SourceLanguage::Japanese,
            SourceLanguage::French,
            SourceLanguage::German,
            SourceLanguage::Korean,
        ] {
            let factory = Factory::new(SpeechMode::Success, TextMode::Success);
            let mut controller = PreparedController::with_factory(factory.clone());
            controller
                .start(
                    fixture.request(vec![]),
                    options(Some(provider), language),
                    key(),
                )
                .unwrap();
            assert_eq!(
                terminal(&mut controller),
                PreparedUiState::Ready { cues: 1 }
            );
            let ready = controller.ready().unwrap();
            assert!(ready.original().existing().is_none());
            let original = ready.original().recognized().unwrap();
            assert_eq!(original.identity(), ready.identity());
            assert_eq!(
                original.language(),
                SpeechLanguage::selected(language).unwrap()
            );
            assert_eq!(original.cues()[0].start_us, 4000);
            assert_eq!(original.cues()[0].duration_us, 12_000);
            assert_eq!(ready.quality(), Some(TranslationQuality::Standard));
            assert!(ready.translated()[0].contains("軟體"));
            assert!(ready.translated()[0].contains("伺服器"));
            assert_eq!(factory.stats.builds.load(Ordering::SeqCst), 1);
            assert_eq!(factory.stats.keys.load(Ordering::SeqCst), 1);
            assert!(
                factory.stats.selected.lock().unwrap().as_slice()
                    == [(provider, PreparedServiceKind::Recognition)]
            );
            assert_eq!(
                factory.stats.speech_selection.lock().unwrap().as_slice(),
                &[(language, PreparedSampleRate::Hz16000)]
            );
            assert_eq!(factory.stats.translation_calls.load(Ordering::SeqCst), 2);
            assert_eq!(
                factory.stats.passes.lock().unwrap().as_slice(),
                &[TranslationPass::Draft, TranslationPass::Review]
            );
            assert!(!fixture.writable());
            assert_eq!(factory.stats.speech_dropped.load(Ordering::SeqCst), 1);
            assert_eq!(factory.stats.translation_dropped.load(Ordering::SeqCst), 1);
            controller.stop();
            assert!(fixture.writable());
        }
    }
    assert_eq!(fs::read_dir(fixture.directory.path()).unwrap().count(), 1);
}

#[test]
fn recognized_source_is_analyzed_before_translation_and_failure_releases_original_pin() {
    let _serial = work::TEST_OWNERS.lock().unwrap();
    let fixture = Fixture::new();
    for provider in [SpeechProvider::OpenAi, SpeechProvider::Gemini] {
        for mode in [TextMode::AnalysisTerms, TextMode::AnalysisFailure] {
            let failed = matches!(mode, TextMode::AnalysisFailure);
            let factory = Factory::new(SpeechMode::Success, mode);
            let mut controller = PreparedController::with_factory(factory.clone());
            controller
                .start(
                    fixture.request(vec![]),
                    options(Some(provider), SourceLanguage::English),
                    key(),
                )
                .unwrap();
            let state = terminal(&mut controller);
            assert_eq!(factory.stats.analysis_calls.load(Ordering::SeqCst), 1);
            if failed {
                assert_eq!(
                    state,
                    PreparedUiState::Failed(PreparedUiError::Translation(
                        TranslationError::Refused
                    ))
                );
                assert_eq!(factory.stats.translation_calls.load(Ordering::SeqCst), 0);
                assert!(controller.ready().is_none());
            } else {
                assert!(matches!(state, PreparedUiState::Ready { .. }));
                assert!(
                    controller
                        .ready()
                        .unwrap()
                        .original()
                        .recognized()
                        .is_some()
                );
                assert!(factory.stats.translation_calls.load(Ordering::SeqCst) > 0);
                controller.stop();
            }
            assert!(OpenOptions::new().write(true).open(&fixture.media).is_ok());
            assert!(controller.worker.is_none());
        }
    }
}

#[test]
fn sidecar_priority_avoids_recognition_and_chinese_avoids_all_factory_work() {
    let _serial = work::TEST_OWNERS.lock().unwrap();
    let fixture = Fixture::new();
    let factory = Factory::new(
        SpeechMode::Fail(PreparedSpeechError::Network),
        TextMode::Success,
    );
    let mut controller = PreparedController::with_factory(factory.clone());
    let source = fixture.subtitle("source.srt", TextLanguage::Source(SpeechLanguage::Japanese));
    let chinese = fixture.subtitle("zh.srt", TextLanguage::SimplifiedChinese);
    controller
        .start(
            fixture.request(vec![source, chinese]),
            options(Some(SpeechProvider::Gemini), SourceLanguage::Auto),
            None,
        )
        .unwrap();
    assert_eq!(
        terminal(&mut controller),
        PreparedUiState::Ready { cues: 1 }
    );
    assert!(controller.ready().unwrap().original().existing().is_some());
    assert_eq!(factory.stats.builds.load(Ordering::SeqCst), 0);
    controller.stop();
    let source = fixture.subtitle("source.srt", TextLanguage::Source(SpeechLanguage::Japanese));
    controller
        .start(
            fixture.request(vec![source]),
            options(Some(SpeechProvider::Gemini), SourceLanguage::Auto),
            key(),
        )
        .unwrap();
    assert_eq!(
        terminal(&mut controller),
        PreparedUiState::Ready { cues: 1 }
    );
    assert!(controller.ready().unwrap().original().existing().is_some());
    assert!(
        factory.stats.selected.lock().unwrap().as_slice()
            == [(SpeechProvider::Gemini, PreparedServiceKind::Translation)]
    );
    assert_eq!(factory.stats.speech_capabilities.load(Ordering::SeqCst), 0);
    assert_eq!(factory.stats.speech_calls.load(Ordering::SeqCst), 0);
    assert_eq!(factory.stats.speech_dropped.load(Ordering::SeqCst), 0);
}

#[test]
fn asr_missing_provider_and_auto_fail_before_worker_factory_or_media_open() {
    let _serial = work::TEST_OWNERS.lock().unwrap();
    let fixture = Fixture::new();
    let factory = Factory::new(SpeechMode::Success, TextMode::Success);
    let mut controller = PreparedController::with_factory(factory.clone());
    for (provider, language, expected) in [
        (
            None,
            SourceLanguage::English,
            PreparedUiError::ProviderRequired,
        ),
        (
            Some(SpeechProvider::OpenAi),
            SourceLanguage::Auto,
            PreparedUiError::Speech(PreparedSpeechError::UnsupportedLanguage),
        ),
    ] {
        let missing = MediaInput::local(fixture.directory.path().join("missing.wav")).unwrap();
        assert_eq!(
            controller.start(
                PreparedRequest::new(missing, vec![]).unwrap(),
                options(provider, language),
                None
            ),
            Err(expected)
        );
        assert!(controller.worker.is_none());
        assert!(controller.session.identity().is_none());
        assert!(controller.ready().is_none());
    }
    assert_eq!(factory.stats.builds.load(Ordering::SeqCst), 0);
    assert!(fixture.writable());
}

#[test]
fn recognizer_capability_failure_precedes_missing_file_and_releases_services() {
    let _serial = work::TEST_OWNERS.lock().unwrap();
    let fixture = Fixture::new();
    let factory = Factory::new(SpeechMode::Unsupported, TextMode::Success);
    let mut controller = PreparedController::with_factory(factory.clone());
    let missing = MediaInput::local(fixture.directory.path().join("missing.wav")).unwrap();
    controller
        .start(
            PreparedRequest::new(missing, vec![]).unwrap(),
            options(Some(SpeechProvider::OpenAi), SourceLanguage::English),
            key(),
        )
        .unwrap();
    assert_eq!(
        terminal(&mut controller),
        PreparedUiState::Failed(PreparedUiError::Speech(
            PreparedSpeechError::UnsupportedCapability
        ))
    );
    assert_eq!(factory.stats.speech_calls.load(Ordering::SeqCst), 0);
    assert_eq!(factory.stats.translation_calls.load(Ordering::SeqCst), 0);
    assert_eq!(factory.stats.speech_dropped.load(Ordering::SeqCst), 1);
    assert_eq!(factory.stats.translation_dropped.load(Ordering::SeqCst), 1);
    assert!(controller.ready().is_none());
}

#[test]
fn pending_final_asr_stop_reaps_worker_and_pin_before_restart() {
    let _serial = work::TEST_OWNERS.lock().unwrap();
    let fixture = Fixture::new();
    let cpu = work::workers();
    let native = work::native_owners();
    let before = (cpu.available_permits(), native.available_permits());
    let (started, received) = mpsc::sync_channel(1);
    let factory = Factory::new(
        SpeechMode::Pending(Arc::new(Mutex::new(Some(started)))),
        TextMode::Success,
    );
    let mut controller = PreparedController::with_factory(factory.clone());
    controller
        .start(
            fixture.request(vec![]),
            options(Some(SpeechProvider::OpenAi), SourceLanguage::English),
            key(),
        )
        .unwrap();
    let token = received.recv_timeout(TEST_TIMEOUT).unwrap();
    assert!(!fixture.writable());
    assert!(fs::remove_file(&fixture.media).is_err());
    assert_eq!(
        (cpu.available_permits(), native.available_permits()),
        before
    );
    controller.stop();
    assert_eq!(controller.state(), PreparedUiState::Stopping);
    assert_eq!(
        controller.start(
            fixture.request(vec![]),
            options(Some(SpeechProvider::Gemini), SourceLanguage::English),
            key()
        ),
        Err(PreparedUiError::Busy)
    );
    assert_eq!(terminal(&mut controller), PreparedUiState::Cancelled);
    assert!(token.is_cancelled());
    assert!(fixture.writable());
    assert!(controller.ready().is_none());
    assert_eq!(
        factory.stats.speech_futures_dropped.load(Ordering::SeqCst),
        1
    );
    assert_eq!(factory.stats.speech_dropped.load(Ordering::SeqCst), 1);
    assert_eq!(
        (cpu.available_permits(), native.available_permits()),
        before
    );
}

#[test]
fn controller_drop_cancels_pending_asr_and_joins_all_owned_work() {
    let _serial = work::TEST_OWNERS.lock().unwrap();
    let fixture = Fixture::new();
    let (started, received) = mpsc::sync_channel(1);
    let factory = Factory::new(
        SpeechMode::Pending(Arc::new(Mutex::new(Some(started)))),
        TextMode::Success,
    );
    let mut controller = PreparedController::with_factory(factory.clone());
    controller
        .start(
            fixture.request(vec![]),
            options(Some(SpeechProvider::Gemini), SourceLanguage::Korean),
            key(),
        )
        .unwrap();
    let token = received.recv_timeout(TEST_TIMEOUT).unwrap();
    assert!(!fixture.writable());
    drop(controller);
    assert!(token.is_cancelled());
    assert!(fixture.writable());
    assert_eq!(factory.stats.speech_dropped.load(Ordering::SeqCst), 1);
    assert_eq!(factory.stats.translation_dropped.load(Ordering::SeqCst), 1);
    assert_eq!(factory.stats.translation_calls.load(Ordering::SeqCst), 0);
    assert_eq!(fs::read_dir(fixture.directory.path()).unwrap().count(), 1);
}

#[test]
fn pending_asr_translation_retains_original_pin_then_stop_releases_it() {
    let _serial = work::TEST_OWNERS.lock().unwrap();
    let fixture = Fixture::new();
    let (started, received) = mpsc::sync_channel(1);
    let factory = Factory::new(
        SpeechMode::Success,
        TextMode::Pending(Arc::new(Mutex::new(Some(started)))),
    );
    let mut controller = PreparedController::with_factory(factory.clone());
    controller
        .start(
            fixture.request(vec![]),
            options(Some(SpeechProvider::OpenAi), SourceLanguage::French),
            key(),
        )
        .unwrap();
    let token = received.recv_timeout(TEST_TIMEOUT).unwrap();
    assert!(!fixture.writable());
    assert!(fs::remove_file(&fixture.media).is_err());
    assert!(controller.ready().is_none());
    controller.stop();
    assert_eq!(terminal(&mut controller), PreparedUiState::Cancelled);
    assert_eq!(token.reason(), Some(TranslationError::Cancelled));
    assert!(fixture.writable());
    assert_eq!(factory.stats.speech_dropped.load(Ordering::SeqCst), 1);
    assert_eq!(factory.stats.translation_dropped.load(Ordering::SeqCst), 1);
}

#[test]
fn stopped_noncooperative_asr_result_never_publishes_or_enters_translation() {
    let _serial = work::TEST_OWNERS.lock().unwrap();
    let fixture = Fixture::new();
    let (started, received) = mpsc::sync_channel(1);
    let release = Arc::new(Notify::new());
    let factory = Factory::new(
        SpeechMode::Late(Arc::new(Mutex::new(Some(started))), release.clone()),
        TextMode::Success,
    );
    let mut controller = PreparedController::with_factory(factory.clone());
    controller
        .start(
            fixture.request(vec![]),
            options(Some(SpeechProvider::Gemini), SourceLanguage::German),
            key(),
        )
        .unwrap();
    let token = received.recv_timeout(TEST_TIMEOUT).unwrap();
    controller.stop();
    release.notify_one();
    assert_eq!(terminal(&mut controller), PreparedUiState::Cancelled);
    assert!(token.is_cancelled());
    assert_eq!(factory.stats.translation_calls.load(Ordering::SeqCst), 0);
    assert!(controller.ready().is_none());
    assert!(fixture.writable());
}

#[test]
fn buffered_asr_completion_after_stop_is_dropped_before_new_session_ready() {
    let _serial = work::TEST_OWNERS.lock().unwrap();
    let fixture = Fixture::new();
    let factory = Factory::new(SpeechMode::Success, TextMode::Success);
    let mut controller = PreparedController::with_factory(factory);
    controller
        .start(
            fixture.request(vec![]),
            options(Some(SpeechProvider::OpenAi), SourceLanguage::English),
            key(),
        )
        .unwrap();
    let old = controller.session.identity().unwrap();
    finished(&controller);
    assert!(controller.ready().is_none());
    assert!(!fixture.writable());
    controller.stop();
    assert_eq!(terminal(&mut controller), PreparedUiState::Cancelled);
    assert!(fixture.writable());
    controller
        .start(
            fixture.request(vec![]),
            options(Some(SpeechProvider::Gemini), SourceLanguage::Japanese),
            key(),
        )
        .unwrap();
    assert_eq!(
        terminal(&mut controller),
        PreparedUiState::Ready { cues: 1 }
    );
    assert_ne!(controller.ready().unwrap().identity().session, old.session);
    assert!(
        controller
            .ready()
            .unwrap()
            .original()
            .recognized()
            .is_some()
    );
}

#[test]
fn typed_asr_and_translation_failures_release_pin_and_never_fallback() {
    let _serial = work::TEST_OWNERS.lock().unwrap();
    let fixture = Fixture::new();
    for (speech, text, expected) in [
        (
            SpeechMode::Fail(PreparedSpeechError::Network),
            TextMode::Success,
            PreparedUiError::Speech(PreparedSpeechError::Network),
        ),
        (
            SpeechMode::Fail(PreparedSpeechError::Credential),
            TextMode::Success,
            PreparedUiError::Speech(PreparedSpeechError::Credential),
        ),
        (
            SpeechMode::Success,
            TextMode::Fail(TranslationError::Network),
            PreparedUiError::Translation(TranslationError::Network),
        ),
    ] {
        let factory = Factory::new(speech, text);
        let mut controller = PreparedController::with_factory(factory.clone());
        controller
            .start(
                fixture.request(vec![]),
                options(Some(SpeechProvider::Gemini), SourceLanguage::English),
                key(),
            )
            .unwrap();
        assert_eq!(terminal(&mut controller), PreparedUiState::Failed(expected));
        assert!(controller.ready().is_none());
        assert!(fixture.writable());
        assert_eq!(factory.stats.builds.load(Ordering::SeqCst), 1);
        assert!(
            factory.stats.selected.lock().unwrap().as_slice()
                == [(SpeechProvider::Gemini, PreparedServiceKind::Recognition)]
        );
        assert_eq!(factory.stats.speech_dropped.load(Ordering::SeqCst), 1);
        assert_eq!(factory.stats.translation_dropped.load(Ordering::SeqCst), 1);
        assert!(!expected.to_string().contains("SYNTHETIC_PREPARED_ASR_KEY"));
        assert!(
            !expected
                .to_string()
                .contains(&fixture.media.to_string_lossy().to_string())
        );
    }
}

#[test]
fn asr_setup_failure_and_wrong_service_kind_fail_closed_before_any_media_work() {
    let _serial = work::TEST_OWNERS.lock().unwrap();
    let fixture = Fixture::new();
    for wrong_kind in [false, true] {
        let mut factory = Factory::new(SpeechMode::Success, TextMode::Success);
        let factory_mut = Arc::get_mut(&mut factory).unwrap();
        factory_mut.wrong_kind = wrong_kind;
        if !wrong_kind {
            factory_mut.setup_error =
                Some(PreparedUiError::Speech(PreparedSpeechError::NotConfigured));
        }
        let expected = if wrong_kind {
            PreparedUiError::WorkerFailed
        } else {
            PreparedUiError::Speech(PreparedSpeechError::NotConfigured)
        };
        let mut controller = PreparedController::with_factory(factory.clone());
        let missing = MediaInput::local(fixture.directory.path().join("missing.wav")).unwrap();
        controller
            .start(
                PreparedRequest::new(missing, vec![]).unwrap(),
                options(Some(SpeechProvider::OpenAi), SourceLanguage::English),
                key(),
            )
            .unwrap();
        assert_eq!(terminal(&mut controller), PreparedUiState::Failed(expected));
        assert_eq!(factory.stats.speech_calls.load(Ordering::SeqCst), 0);
        assert_eq!(factory.stats.translation_calls.load(Ordering::SeqCst), 0);
        assert!(controller.ready().is_none());
    }
}

#[path = "prepared_remote_tests.rs"]
mod remote;

/// Independent endpoint arithmetic over the actual decoder's native packet
/// observations. It uses neither the production normalizer nor any clock map.
struct NativeOracleRow {
    run: u64,
    sample_start_us: u64,
    sample_end_us: u64,
    native_start_us: u64,
    native_end_us: u64,
}
struct NativeOracle(Vec<NativeOracleRow>);
impl NativeOracle {
    fn read(fixture: &Fixture) -> Self {
        let cancel = ResolverCancellation::default();
        let identity = StreamIdentity {
            session: SessionId::allocate().unwrap(),
            generation: 1,
        };
        let mut decoder = crate::win32::media_decode::open(
            MediaInput::local(fixture.media.clone()).unwrap(),
            identity,
            cancel.clone(),
        )
        .unwrap();
        let rate = u64::from(decoder.format().sample_rate());
        let mut rows = Vec::<NativeOracleRow>::new();
        let mut origin = 0;
        let mut frames = 0;
        let mut run = 0;
        while let Some(block) = decoder.next(&cancel).unwrap() {
            assert!(rows.len() < MAX_ORACLE_ROWS);
            let gap = rows.last().map_or(0, |previous| {
                block.timestamp_us().saturating_sub(previous.native_end_us)
            });
            if rows.is_empty() || gap > 1 {
                origin = rows.last().map_or(block.timestamp_us(), |previous| {
                    previous.sample_end_us + gap
                });
                frames = 0;
                run += 1;
            }
            let sample_start_us = origin + frames * 1_000_000 / rate;
            frames += block.frames() as u64;
            rows.push(NativeOracleRow {
                run,
                sample_start_us,
                sample_end_us: origin + frames * 1_000_000 / rate,
                native_start_us: block.timestamp_us(),
                native_end_us: block.end_us(),
            });
        }
        drop(decoder);
        assert!(fixture.writable());
        assert!(rows.last().unwrap().native_end_us > 12_000_000);
        Self(rows)
    }
    fn endpoint(row: &NativeOracleRow, sample_us: u64) -> u64 {
        if sample_us == row.sample_end_us {
            row.native_end_us
        } else {
            row.native_start_us
                + (sample_us - row.sample_start_us) * (row.native_end_us - row.native_start_us)
                    / (row.sample_end_us - row.sample_start_us)
        }
    }
    fn interval(&self, input: RecordedInterval) -> (u64, u64) {
        let first = self
            .0
            .iter()
            .find(|row| row.sample_start_us <= input.start_us && input.start_us < row.sample_end_us)
            .unwrap();
        let last = self
            .0
            .iter()
            .find(|row| row.sample_start_us < input.end_us && input.end_us <= row.sample_end_us)
            .unwrap();
        assert_eq!(input.source_run, Some(first.run));
        assert_eq!(first.run, last.run);
        (
            Self::endpoint(first, input.start_us),
            Self::endpoint(last, input.end_us),
        )
    }
}

#[test]
fn synthesized_adts_dense_packets_reach_ready_with_native_times_and_glossary_for_each_provider() {
    let _serial = work::TEST_OWNERS.lock().unwrap();
    assert_adts_ready(2);
}
#[test]
fn synthesized_adts_mono_reaches_ready_with_native_times_for_each_provider_and_rate() {
    let _serial = work::TEST_OWNERS.lock().unwrap();
    assert_adts_ready(1);
}
fn assert_adts_ready(channels: u16) {
    let bytes =
        crate::win32::media_mp3_fixture::synthetic_adts(48_000, channels, 13 * 48_000).unwrap();
    // The file has dense AAC packets; MF may coalesce several into one decoded
    // sample. Never assume source packet count equals native observation count.
    let mut position = 0;
    let mut packets = 0;
    while position < bytes.len() {
        let length = (usize::from(bytes[position + 3] & 3) << 11)
            | (usize::from(bytes[position + 4]) << 3)
            | usize::from(bytes[position + 5] >> 5);
        assert!(length > 7);
        position += length;
        packets += 1;
    }
    assert_eq!(position, bytes.len());
    assert!(packets > 512);
    assert_aac_ready(bytes);
}
#[test]
fn synthesized_mp4_reaches_ready_with_native_endpoints_for_both_channels_providers_and_rates() {
    let _serial = work::TEST_OWNERS.lock().unwrap();
    for channels in [1, 2] {
        assert_aac_ready(
            crate::win32::media_mp3_fixture::synthetic_mp4(48_000, channels, 13 * 48_000).unwrap(),
        );
    }
}
#[test]
fn synthesized_mp4_44100_reaches_ready_with_native_endpoints_for_both_channels_providers_and_rates()
{
    let _serial = work::TEST_OWNERS.lock().unwrap();
    for channels in [1, 2] {
        assert_aac_ready(
            crate::win32::media_mp3_fixture::synthetic_mp4(44_100, channels, 13 * 44_100 + 311)
                .unwrap(),
        );
    }
}
fn assert_aac_ready(bytes: Vec<u8>) {
    assert_aac_ready_with_metadata_guard(bytes, false);
}
fn assert_aac_ready_with_metadata_guard(bytes: Vec<u8>, forbid_metadata: bool) {
    let fixture = Fixture::new();
    fs::write(&fixture.media, &bytes).unwrap();
    let oracle = NativeOracle::read(&fixture);
    let permits = (
        work::workers().available_permits(),
        work::native_owners().available_permits(),
    );
    for provider in [SpeechProvider::OpenAi, SpeechProvider::Gemini] {
        for rate in [PreparedSampleRate::Hz16000, PreparedSampleRate::Hz24000] {
            let factory = Factory::new(SpeechMode::BodySuccess, TextMode::Success);
            factory
                .stats
                .forbid_file_metadata
                .store(forbid_metadata, Ordering::SeqCst);
            let mut controller = PreparedController::with_factory(factory.clone());
            if rate == PreparedSampleRate::Hz16000 {
                controller
                    .start(
                        fixture.request(vec![]),
                        options(Some(provider), SourceLanguage::English),
                        key(),
                    )
                    .unwrap();
                let state = terminal_with_timeout(&mut controller, MP3_TEST_TIMEOUT);
                assert!(matches!(state, PreparedUiState::Ready { .. }), "{state:?}");
            } else {
                let task = controller.session.start(fixture.request(vec![])).unwrap();
                let PreparedServices::Recognition {
                    recognizer,
                    translator,
                } = factory
                    .build(provider, key(), PreparedServiceKind::Recognition)
                    .unwrap()
                else {
                    panic!("explicit ASR mock returned wrong services");
                };
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                let completion = runtime.block_on(task.run_with_analyzed_recognizer(
                    recognizer.as_ref(),
                    translator.as_ref(),
                    SourceLanguage::English,
                    rate,
                    TranslationQuality::Standard,
                    None,
                ));
                assert!(controller.session.apply(completion).unwrap());
                controller.state = PreparedUiState::Ready {
                    cues: controller.session.ready().unwrap().cue_count(),
                };
            }
            let ready = controller.ready().unwrap();
            let original = ready.original().recognized().unwrap();
            let mut recorded = factory.stats.speech_intervals.lock().unwrap().clone();
            recorded.sort_by_key(|row| row.sequence);
            assert!(recorded.len() >= 2);
            assert_eq!(original.cues().len(), recorded.len());
            for (cue, input) in original.cues().iter().zip(&recorded) {
                let (start, end) = oracle.interval(*input);
                assert_eq!(cue.start_us, start);
                assert_eq!(cue.duration_us, end - start);
            }
            assert_eq!(factory.stats.analysis_calls.load(Ordering::SeqCst), 1);
            assert_eq!(
                factory.stats.translation_calls.load(Ordering::SeqCst),
                recorded.len() * 2
            );
            assert!(
                ready
                    .translated()
                    .iter()
                    .all(|text| text.contains("軟體") && text.contains("伺服器"))
            );
            assert!(!fixture.writable());
            assert!(fs::remove_file(&fixture.media).is_err());
            controller.stop();
            assert!(controller.ready().is_none());
            assert!(fixture.writable());
            assert_eq!(
                (
                    work::workers().available_permits(),
                    work::native_owners().available_permits(),
                ),
                permits
            );
            assert_eq!(fs::read(&fixture.media).unwrap(), bytes);
            assert_eq!(fs::read_dir(fixture.directory.path()).unwrap().count(), 1);
        }
    }
}

#[test]
fn tagged_mp4_reaches_ready_with_native_times_without_transmitting_metadata() {
    let _serial = work::TEST_OWNERS.lock().unwrap();
    let bytes =
        crate::win32::media_mp3_fixture::synthetic_mp4(44_100, 2, 13 * 44_100 + 311).unwrap();
    assert_aac_ready_with_metadata_guard(crate::win32::media_mp3_fixture::tagged_mp4(&bytes), true);
}

#[test]
fn tagged_mp4_stop_error_and_chinese_sidecar_preference_release_resources() {
    let _serial = work::TEST_OWNERS.lock().unwrap();
    let bytes = crate::win32::media_mp3_fixture::synthetic_mp4(48_000, 1, 48_000).unwrap();
    assert_aac_stop_error_sidecar(crate::win32::media_mp3_fixture::tagged_mp4(&bytes));
}

#[test]
fn video_mp4_reaches_ready_for_both_rates_channels_and_selected_services() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    for rate in [44_100u32, 48_000] {
        for channels in [1, 2] {
            let bytes = crate::win32::media_mp3_fixture::synthetic_mp4_video(
                rate,
                channels,
                rate as usize * 13,
            )
            .unwrap();
            if rate == 44_100 && channels == 2 {
                assert_aac_ready_with_metadata_guard(
                    crate::win32::media_mp3_fixture::tagged_mp4(&bytes),
                    true,
                );
            } else {
                assert_aac_ready(bytes);
            }
        }
    }
}

#[test]
fn video_mp4_stop_error_and_chinese_sidecar_preference_release_resources() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    for rate in [44_100u32, 48_000] {
        assert_aac_stop_error_sidecar(
            crate::win32::media_mp3_fixture::synthetic_mp4_video(rate, 2, rate as usize * 13)
                .unwrap(),
        );
    }
}

#[test]
fn main_high_mp4_reach_ready_with_both_selected_services_and_dsp_rates() {
    use crate::win32::media_mp3_fixture::{H264Profile, synthetic_mp4_video_profile};
    let _serial = work::TEST_OWNERS.lock().unwrap();
    for (profile, rate, channels) in [
        (H264Profile::Main, 44_100, 1),
        (H264Profile::High, 48_000, 2),
    ] {
        let bytes =
            synthetic_mp4_video_profile(rate, channels, 13 * rate as usize + 311, profile).unwrap();
        assert_aac_ready_with_metadata_guard(
            crate::win32::media_mp3_fixture::tagged_mp4(&bytes),
            true,
        );
    }
}

#[test]
fn main_high_mp4_stop_error_and_chinese_sidecar_release_resources() {
    use crate::win32::media_mp3_fixture::{H264Profile, synthetic_mp4_video_profile};
    let _serial = work::TEST_OWNERS.lock().unwrap();
    for (profile, rate, channels) in [
        (H264Profile::Main, 48_000, 1),
        (H264Profile::High, 44_100, 2),
    ] {
        assert_aac_stop_error_sidecar(
            synthetic_mp4_video_profile(rate, channels, rate as usize, profile).unwrap(),
        );
    }
}

#[test]
fn b_frames_mp4_reaches_ready_with_selected_services_native_times_and_no_metadata() {
    use crate::win32::media_mp3_fixture::{H264Profile, synthetic_mp4_b_frames};
    let _serial = work::TEST_OWNERS.lock().unwrap();
    for (profile, rate, channels) in [
        (H264Profile::Main, 44_100, 1),
        (H264Profile::High, 48_000, 2),
    ] {
        let bytes =
            synthetic_mp4_b_frames(rate, channels, 13 * rate as usize + 311, profile).unwrap();
        assert_aac_ready_with_metadata_guard(
            crate::win32::media_mp3_fixture::tagged_mp4(&bytes),
            true,
        );
    }
}

#[test]
fn b_frames_mp4_stop_error_and_chinese_sidecar_release_resources() {
    use crate::win32::media_mp3_fixture::{H264Profile, synthetic_mp4_b_frames};
    let _serial = work::TEST_OWNERS.lock().unwrap();
    for (profile, rate, channels) in [
        (H264Profile::Main, 48_000, 1),
        (H264Profile::High, 44_100, 2),
    ] {
        assert_aac_stop_error_sidecar(
            synthetic_mp4_b_frames(rate, channels, 6 * rate as usize, profile).unwrap(),
        );
    }
}

#[test]
fn b_frames_mp4_invalid_video_composition_and_reference_do_not_issue_requests() {
    use crate::win32::media_mp3_fixture::{H264Profile, synthetic_mp4_b_frames};
    let _serial = work::TEST_OWNERS.lock().unwrap();
    let fixture = Fixture::new();
    for profile in [H264Profile::Main, H264Profile::High] {
        let original = synthetic_mp4_b_frames(48_000, 2, 6 * 48_000, profile).unwrap();
        for (kind, offset, value) in [(b"ctts", 0, 2), (b"ctts", 11, 0), (b"url ", 3, 0)] {
            let mut bytes = original.clone();
            let at = bytes.windows(4).position(|b| b == kind).unwrap() + 4;
            bytes[at + offset] = value;
            fs::write(&fixture.media, &bytes).unwrap();
            let factory = Factory::new(SpeechMode::Success, TextMode::Success);
            let mut controller = PreparedController::with_factory(factory.clone());
            controller
                .start(
                    fixture.request(vec![]),
                    options(Some(SpeechProvider::OpenAi), SourceLanguage::English),
                    key(),
                )
                .unwrap();
            assert_eq!(
                terminal(&mut controller),
                PreparedUiState::Failed(PreparedUiError::Media(MediaError::UnsupportedMedia))
            );
            assert_eq!(factory.stats.speech_calls.load(Ordering::SeqCst), 0);
            assert_eq!(factory.stats.analysis_calls.load(Ordering::SeqCst), 0);
            assert_eq!(factory.stats.translation_calls.load(Ordering::SeqCst), 0);
            assert!(controller.ready().is_none() && fixture.writable());
            assert_eq!(fs::read(&fixture.media).unwrap(), bytes);
        }
    }
}

#[test]
fn main_high_mp4_invalid_video_track_does_not_issue_cloud_requests() {
    use crate::win32::media_mp3_fixture::{H264Profile, synthetic_mp4_video_profile};
    let _serial = work::TEST_OWNERS.lock().unwrap();
    let fixture = Fixture::new();
    for profile in [H264Profile::Main, H264Profile::High] {
        let original = synthetic_mp4_video_profile(48_000, 2, 48_000, profile).unwrap();
        for (kind, offset, value) in [(b"url ", 3, 0), (b"avcC", 1, 110), (b"stco", 8, 255)] {
            let mut bytes = original.clone();
            let at = bytes.windows(4).position(|b| b == kind).unwrap() + 4;
            bytes[at + offset] = value;
            fs::write(&fixture.media, &bytes).unwrap();
            let factory = Factory::new(SpeechMode::Success, TextMode::Success);
            let mut controller = PreparedController::with_factory(factory.clone());
            controller
                .start(
                    fixture.request(vec![]),
                    options(Some(SpeechProvider::Gemini), SourceLanguage::English),
                    key(),
                )
                .unwrap();
            assert_eq!(
                terminal(&mut controller),
                PreparedUiState::Failed(PreparedUiError::Media(MediaError::UnsupportedMedia))
            );
            assert_eq!(factory.stats.speech_calls.load(Ordering::SeqCst), 0);
            assert_eq!(factory.stats.analysis_calls.load(Ordering::SeqCst), 0);
            assert_eq!(factory.stats.translation_calls.load(Ordering::SeqCst), 0);
            assert!(controller.ready().is_none() && fixture.writable());
            assert_eq!(fs::read(&fixture.media).unwrap(), bytes);
        }
    }
}

#[test]
fn synthesized_adts_stop_error_and_chinese_sidecar_preference_release_resources() {
    let _serial = work::TEST_OWNERS.lock().unwrap();
    for channels in [1, 2] {
        assert_adts_stop_error_sidecar(channels);
    }
}
fn assert_adts_stop_error_sidecar(channels: u16) {
    assert_aac_stop_error_sidecar(
        crate::win32::media_mp3_fixture::synthetic_adts(48_000, channels, 48_000).unwrap(),
    );
}
#[test]
fn synthesized_mp4_stop_error_and_chinese_sidecar_preference_release_resources() {
    let _serial = work::TEST_OWNERS.lock().unwrap();
    for rate in [44_100u32, 48_000] {
        for channels in [1, 2] {
            assert_aac_stop_error_sidecar(
                crate::win32::media_mp3_fixture::synthetic_mp4(rate, channels, rate as usize)
                    .unwrap(),
            );
        }
    }
}
fn assert_aac_stop_error_sidecar(bytes: Vec<u8>) {
    let fixture = Fixture::new();
    fs::write(&fixture.media, &bytes).unwrap();
    let before = fs::read(&fixture.media).unwrap();
    let factory = Factory::new(
        SpeechMode::Fail(PreparedSpeechError::Network),
        TextMode::Success,
    );
    let mut controller = PreparedController::with_factory(factory.clone());
    controller
        .start(
            fixture.request(vec![]),
            options(Some(SpeechProvider::OpenAi), SourceLanguage::English),
            key(),
        )
        .unwrap();
    let failed_start = Instant::now();
    let failure = terminal_with_timeout(&mut controller, NATIVE_CODEC_START_TIMEOUT);
    eprintln!(
        "synthetic codec failure startup: {:?}",
        failed_start.elapsed()
    );
    assert_eq!(
        failure,
        PreparedUiState::Failed(PreparedUiError::Speech(PreparedSpeechError::Network))
    );
    assert_eq!(factory.stats.analysis_calls.load(Ordering::SeqCst), 0);
    assert!(fixture.writable());
    let (started, received) = mpsc::sync_channel(1);
    let factory = Factory::new(
        SpeechMode::Pending(Arc::new(Mutex::new(Some(started)))),
        TextMode::Success,
    );
    let mut controller = PreparedController::with_factory(factory.clone());
    controller
        .start(
            fixture.request(vec![]),
            options(Some(SpeechProvider::Gemini), SourceLanguage::English),
            key(),
        )
        .unwrap();
    let pending_start = Instant::now();
    let token = received.recv_timeout(NATIVE_CODEC_START_TIMEOUT).unwrap();
    eprintln!(
        "synthetic codec pending startup: {:?}",
        pending_start.elapsed()
    );
    controller.stop();
    let stopped = Instant::now();
    assert_eq!(terminal(&mut controller), PreparedUiState::Cancelled);
    eprintln!("synthetic codec cancellation: {:?}", stopped.elapsed());
    assert!(token.is_cancelled());
    assert!(fixture.writable());
    let subtitle = fixture.subtitle("chinese.srt", TextLanguage::SimplifiedChinese);
    controller
        .start(
            fixture.request(vec![subtitle]),
            options(None, SourceLanguage::Auto),
            None,
        )
        .unwrap();
    assert!(matches!(
        terminal(&mut controller),
        PreparedUiState::Ready { .. }
    ));
    assert_eq!(factory.stats.builds.load(Ordering::SeqCst), 1);
    assert_eq!(factory.stats.analysis_calls.load(Ordering::SeqCst), 0);
    controller.stop();
    assert!(controller.ready().is_none());
    assert!(fixture.writable());
    assert_eq!(fs::read(&fixture.media).unwrap(), before);
}

#[test]
fn mp4_external_reference_fails_before_cloud_requests_at_each_rate() {
    let _serial = work::TEST_OWNERS.lock().unwrap();
    let fixture = Fixture::new();
    for (rate, channels) in [(44_100u32, 1), (44_100, 2), (48_000, 1), (48_000, 2)] {
        let mut bytes =
            crate::win32::media_mp3_fixture::synthetic_mp4(rate, channels, rate as usize).unwrap();
        let at = bytes.windows(4).position(|w| w == b"url ").unwrap();
        bytes[at + 7] = 0; // Disable the self-contained data-reference flag.
        fs::write(&fixture.media, &bytes).unwrap();
        let factory = Factory::new(SpeechMode::Success, TextMode::Success);
        let mut controller = PreparedController::with_factory(factory.clone());
        controller
            .start(
                fixture.request(vec![]),
                options(Some(SpeechProvider::OpenAi), SourceLanguage::English),
                key(),
            )
            .unwrap();
        assert_eq!(
            terminal(&mut controller),
            PreparedUiState::Failed(PreparedUiError::Media(MediaError::UnsupportedMedia))
        );
        assert_eq!(factory.stats.speech_calls.load(Ordering::SeqCst), 0);
        assert_eq!(factory.stats.analysis_calls.load(Ordering::SeqCst), 0);
        assert_eq!(factory.stats.translation_calls.load(Ordering::SeqCst), 0);
        assert!(controller.ready().is_none());
        assert!(fixture.writable());
    }
}

#[test]
fn video_mp4_invalid_video_track_fails_without_any_cloud_requests() {
    let _serial = work::TEST_OWNERS.lock().unwrap();
    let fixture = Fixture::new();
    let original = crate::win32::media_mp3_fixture::synthetic_mp4_video(48_000, 2, 48_000).unwrap();
    for (kind, offset, value) in [(b"url ", 3, 0), (b"avcC", 1, 77), (b"stco", 8, 255)] {
        let mut bytes = original.clone();
        // Native sink writes video first. These failures must be detected even
        // though the product selects only the audio stream for decoding.
        let at = bytes.windows(4).position(|w| w == kind).unwrap() + 4;
        bytes[at + offset] = value;
        fs::write(&fixture.media, &bytes).unwrap();
        let factory = Factory::new(SpeechMode::Success, TextMode::Success);
        let mut controller = PreparedController::with_factory(factory.clone());
        controller
            .start(
                fixture.request(vec![]),
                options(Some(SpeechProvider::OpenAi), SourceLanguage::English),
                key(),
            )
            .unwrap();
        assert_eq!(
            terminal(&mut controller),
            PreparedUiState::Failed(PreparedUiError::Media(MediaError::UnsupportedMedia))
        );
        assert_eq!(factory.stats.speech_calls.load(Ordering::SeqCst), 0);
        assert_eq!(factory.stats.analysis_calls.load(Ordering::SeqCst), 0);
        assert_eq!(factory.stats.translation_calls.load(Ordering::SeqCst), 0);
        assert!(controller.ready().is_none() && fixture.writable());
        assert_eq!(fs::read(&fixture.media).unwrap(), bytes);
    }
}

#[test]
fn recognized_adts_unverified_clock_fails_without_speech_or_glossary_requests() {
    let _serial = work::TEST_OWNERS.lock().unwrap();
    let fixture = Fixture::new();
    for (rate, channels) in [(44_100, 1), (44_100, 2)] {
        fs::write(
            &fixture.media,
            crate::win32::media_mp3_fixture::synthetic_adts(rate, channels, rate as usize).unwrap(),
        )
        .unwrap();
        let factory = Factory::new(SpeechMode::Success, TextMode::Success);
        let mut controller = PreparedController::with_factory(factory.clone());
        controller
            .start(
                fixture.request(vec![]),
                options(Some(SpeechProvider::Gemini), SourceLanguage::English),
                key(),
            )
            .unwrap();
        assert_eq!(
            terminal(&mut controller),
            PreparedUiState::Failed(PreparedUiError::Media(MediaError::UnsupportedAudioClock))
        );
        assert_eq!(factory.stats.speech_calls.load(Ordering::SeqCst), 0);
        assert_eq!(factory.stats.analysis_calls.load(Ordering::SeqCst), 0);
        assert_eq!(factory.stats.translation_calls.load(Ordering::SeqCst), 0);
        assert!(controller.ready().is_none());
        assert!(fixture.writable());
    }
}

fn assert_mp3_ready(
    ready: &PreparedDocument,
    factory: &Factory,
    oracle: &NativeOracle,
    provider: SpeechProvider,
    rate: PreparedSampleRate,
    media_bytes: usize,
) -> usize {
    assert!(ready.original().existing().is_none());
    let original = ready.original().recognized().unwrap();
    assert_eq!(original.identity(), ready.identity());
    assert_eq!(original.language(), SpeechLanguage::English);
    let mut recorded = factory.stats.speech_intervals.lock().unwrap().clone();
    recorded.sort_by_key(|row| row.sequence);
    // Decoder delay creates an initial short run; the long run owns one full
    // ten-second body and a real EOF tail. No source run is silently joined.
    assert!(recorded.len() >= 3);
    assert_eq!(original.cues().len(), recorded.len());
    let mut changed_clock = false;
    for (index, (cue, input)) in original.cues().iter().zip(&recorded).enumerate() {
        assert_eq!(input.sequence, index as u64 + 1);
        let (start, end) = oracle.interval(*input);
        assert_eq!(cue.start_us, start);
        assert_eq!(cue.duration_us, end - start);
        assert_eq!(&*cue.text, "synthetic recognized dialogue");
        changed_clock |= end.abs_diff(input.end_us) > 1;
    }
    assert!(
        changed_clock,
        "packet quantization must be mapped back to native time"
    );
    assert!(
        original
            .cues()
            .windows(2)
            .all(|pair| { pair[0].start_us + pair[0].duration_us <= pair[1].start_us })
    );
    assert_eq!(ready.media_byte_len(), media_bytes as u64);
    assert_eq!(ready.quality(), Some(TranslationQuality::Standard));
    assert_eq!(ready.target_language(), "zh-TW");
    assert_eq!(ready.translated().len(), recorded.len());
    assert!(
        ready
            .translated()
            .iter()
            .all(|text| { text.contains("軟體") && text.contains("伺服器") })
    );
    assert_eq!(factory.stats.builds.load(Ordering::SeqCst), 1);
    assert_eq!(factory.stats.keys.load(Ordering::SeqCst), 1);
    assert!(
        factory.stats.selected.lock().unwrap().as_slice()
            == [(provider, PreparedServiceKind::Recognition)]
    );
    assert!(
        factory
            .stats
            .speech_selection
            .lock()
            .unwrap()
            .iter()
            .all(|selected| { *selected == (SourceLanguage::English, rate) })
    );
    assert_eq!(
        factory.stats.speech_calls.load(Ordering::SeqCst),
        recorded.len()
    );
    assert_eq!(
        factory.stats.translation_calls.load(Ordering::SeqCst),
        recorded.len() * 2
    );
    let passes = factory.stats.passes.lock().unwrap();
    assert_eq!(
        passes
            .iter()
            .filter(|pass| **pass == TranslationPass::Draft)
            .count(),
        recorded.len()
    );
    assert_eq!(
        passes
            .iter()
            .filter(|pass| **pass == TranslationPass::Review)
            .count(),
        recorded.len()
    );
    recorded.len()
}

#[test]
fn synthesized_mp3_reaches_pinned_ready_with_native_endpoints_for_each_provider_and_rate() {
    let _serial = work::TEST_OWNERS.lock().unwrap();
    let fixture = Fixture::mp3_frames(44_100, 2, 13 * 44_100);
    let before = fs::read(&fixture.media).unwrap();
    let oracle = NativeOracle::read(&fixture);
    let permits = (
        work::workers().available_permits(),
        work::native_owners().available_permits(),
    );
    for provider in [SpeechProvider::OpenAi, SpeechProvider::Gemini] {
        for rate in [PreparedSampleRate::Hz16000, PreparedSampleRate::Hz24000] {
            let factory = Factory::new(SpeechMode::BodySuccess, TextMode::Success);
            if rate == PreparedSampleRate::Hz16000 {
                // The actual GUI backend deliberately uses the configured16k
                // path;24k is exercised through the same genuine Session API.
                let mut controller = PreparedController::with_factory(factory.clone());
                controller
                    .start(
                        fixture.request(vec![]),
                        options(Some(provider), SourceLanguage::English),
                        key(),
                    )
                    .unwrap();
                let state = terminal_with_timeout(&mut controller, MP3_TEST_TIMEOUT);
                assert!(matches!(state, PreparedUiState::Ready { .. }), "{state:?}");
                let cues = assert_mp3_ready(
                    controller.ready().unwrap(),
                    &factory,
                    &oracle,
                    provider,
                    rate,
                    before.len(),
                );
                assert_eq!(state, PreparedUiState::Ready { cues });
                assert!(!fixture.writable());
                assert!(fs::remove_file(&fixture.media).is_err());
                controller.stop();
                assert!(controller.ready().is_none());
            } else {
                let PreparedServices::Recognition {
                    recognizer,
                    translator,
                } = factory
                    .build(provider, key(), PreparedServiceKind::Recognition)
                    .unwrap()
                else {
                    panic!("explicit selected-provider mock returned wrong services")
                };
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                let mut session = PreparedSession::new();
                let completion = runtime.block_on(
                    session
                        .start(fixture.request(vec![]))
                        .unwrap()
                        .run_with_recognizer(
                            recognizer.as_ref(),
                            translator.as_ref(),
                            SourceLanguage::English,
                            rate,
                            TranslationQuality::Standard,
                            None,
                        ),
                );
                assert!(session.apply(completion).unwrap());
                assert_mp3_ready(
                    session.ready().unwrap(),
                    &factory,
                    &oracle,
                    provider,
                    rate,
                    before.len(),
                );
                assert!(!fixture.writable());
                assert!(fs::remove_file(&fixture.media).is_err());
                session.stop();
                assert!(session.ready().is_none());
                drop(runtime);
            }
            assert!(fixture.writable());
            assert_eq!(
                (
                    work::workers().available_permits(),
                    work::native_owners().available_permits()
                ),
                permits
            );
            assert_eq!(factory.stats.speech_dropped.load(Ordering::SeqCst), 1);
            assert_eq!(factory.stats.translation_dropped.load(Ordering::SeqCst), 1);
            assert_eq!(fs::read(&fixture.media).unwrap(), before);
            assert_eq!(fs::read_dir(fixture.directory.path()).unwrap().count(), 1);
        }
    }
}

#[test]
fn synthesized_mp3_service_failures_preserve_typed_error_and_release_pin_without_ready() {
    let _serial = work::TEST_OWNERS.lock().unwrap();
    let fixture = Fixture::mp3();
    let before = fs::read(&fixture.media).unwrap();
    let permits = (
        work::workers().available_permits(),
        work::native_owners().available_permits(),
    );
    for provider in [SpeechProvider::OpenAi, SpeechProvider::Gemini] {
        for (speech, text, error) in [
            (
                SpeechMode::Fail(PreparedSpeechError::Network),
                TextMode::Success,
                PreparedUiError::Speech(PreparedSpeechError::Network),
            ),
            (
                SpeechMode::Fail(PreparedSpeechError::Credential),
                TextMode::Success,
                PreparedUiError::Speech(PreparedSpeechError::Credential),
            ),
            (
                SpeechMode::BodySuccess,
                TextMode::Fail(TranslationError::Network),
                PreparedUiError::Translation(TranslationError::Network),
            ),
        ] {
            let factory = Factory::new(speech, text);
            let mut controller = PreparedController::with_factory(factory.clone());
            controller
                .start(
                    fixture.request(vec![]),
                    options(Some(provider), SourceLanguage::English),
                    key(),
                )
                .unwrap();
            assert_eq!(
                terminal_with_timeout(&mut controller, MP3_TEST_TIMEOUT),
                PreparedUiState::Failed(error)
            );
            assert!(controller.ready().is_none());
            assert!(fixture.writable());
            assert_eq!(factory.stats.builds.load(Ordering::SeqCst), 1);
            assert_eq!(factory.stats.keys.load(Ordering::SeqCst), 1);
            assert!(
                factory.stats.selected.lock().unwrap().as_slice()
                    == [(provider, PreparedServiceKind::Recognition)]
            );
            assert_eq!(
                (
                    work::workers().available_permits(),
                    work::native_owners().available_permits()
                ),
                permits
            );
            assert_eq!(factory.stats.speech_dropped.load(Ordering::SeqCst), 1);
            assert_eq!(factory.stats.translation_dropped.load(Ordering::SeqCst), 1);
        }
    }
    assert_eq!(fs::read(&fixture.media).unwrap(), before);
    assert_eq!(fs::read_dir(fixture.directory.path()).unwrap().count(), 1);
}

#[test]
fn synthesized_mp3_pending_final_asr_stop_and_drop_join_before_releasing_pin() {
    let _serial = work::TEST_OWNERS.lock().unwrap();
    let fixture = Fixture::mp3();
    let before = fs::read(&fixture.media).unwrap();
    let permits = (
        work::workers().available_permits(),
        work::native_owners().available_permits(),
    );
    for provider in [SpeechProvider::OpenAi, SpeechProvider::Gemini] {
        for owner_drop in [false, true] {
            let (started, received) = mpsc::sync_channel(1);
            let factory = Factory::new(
                SpeechMode::Pending(Arc::new(Mutex::new(Some(started)))),
                TextMode::Success,
            );
            let mut controller = PreparedController::with_factory(factory.clone());
            controller
                .start(
                    fixture.request(vec![]),
                    options(Some(provider), SourceLanguage::English),
                    key(),
                )
                .unwrap();
            let token = received.recv_timeout(MP3_TEST_TIMEOUT).unwrap();
            assert!(!fixture.writable());
            assert!(fs::remove_file(&fixture.media).is_err());
            assert!(controller.ready().is_none());
            // Native EOF destruction/join already restored both lanes. The
            // same original media pin survives the final provider wait.
            assert_eq!(
                (
                    work::workers().available_permits(),
                    work::native_owners().available_permits()
                ),
                permits
            );
            if owner_drop {
                drop(controller);
            } else {
                controller.stop();
                assert_eq!(
                    terminal_with_timeout(&mut controller, MP3_TEST_TIMEOUT),
                    PreparedUiState::Cancelled
                );
                assert!(controller.ready().is_none());
            }
            assert!(token.is_cancelled());
            assert!(fixture.writable());
            assert_eq!(
                (
                    work::workers().available_permits(),
                    work::native_owners().available_permits()
                ),
                permits
            );
            assert_eq!(factory.stats.translation_calls.load(Ordering::SeqCst), 0);
            assert_eq!(factory.stats.speech_dropped.load(Ordering::SeqCst), 1);
            assert_eq!(factory.stats.translation_dropped.load(Ordering::SeqCst), 1);
            assert!(factory.stats.speech_futures_dropped.load(Ordering::SeqCst) > 0);
        }
    }
    assert_eq!(fs::read(&fixture.media).unwrap(), before);
    assert_eq!(fs::read_dir(fixture.directory.path()).unwrap().count(), 1);
}

#[test]
fn compressed_media_sidecar_priority_does_not_require_audio_clock_or_cloud() {
    let _serial = work::TEST_OWNERS.lock().unwrap();
    let fixture = Fixture::mp3();
    let factory = Factory::new(
        SpeechMode::Fail(PreparedSpeechError::Network),
        TextMode::Fail(TranslationError::Network),
    );
    let mut controller = PreparedController::with_factory(factory.clone());
    let sidecar = fixture.subtitle("explicit-zh.srt", TextLanguage::SimplifiedChinese);
    controller
        .start(
            fixture.request(vec![sidecar]),
            options(None, SourceLanguage::Auto),
            None,
        )
        .unwrap();
    assert_eq!(
        terminal(&mut controller),
        PreparedUiState::Ready { cues: 1 }
    );
    assert!(controller.ready().unwrap().original().existing().is_some());
    assert_eq!(factory.stats.builds.load(Ordering::SeqCst), 0);
    assert!(!fixture.writable());
    controller.stop();
    assert!(fixture.writable());
}
