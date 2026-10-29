//! Explicit synthetic WAVE/sidecar inputs and mock providers. No real media,
//! credentials, provider network, content cache, playback or extra timeline.
#![cfg(windows)]
#[path = "prepared_alignment_asr_tests.rs"]
mod alignment;
use super::*;
use crate::{
    ai::{
        prepared_speech::{
            PreparedSpeechCapabilities, PreparedSpeechCue, PreparedSpeechFuture,
            PreparedSpeechRequest, PreparedSpeechResponse,
        },
        speech::SpeechLanguage,
        translation::{
            TranslationCancellation, TranslationCapabilities, TranslationEmitter,
            TranslationFuture, TranslationPass, TranslationRequest,
        },
    },
    media::input::{LocalSubtitleInput, MediaInput},
    resolver::{contracts::SubtitleSource, timed_subtitle::SubtitleFormat},
};
use std::{
    fs::{self, OpenOptions},
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::Notify;

const TEST_TIMEOUT: Duration = Duration::from_secs(5);
const SYNTHETIC_WAVE_FRAMES: usize = 1600;
type SeenTerms = Vec<(Box<str>, Box<str>)>;

struct Fixture {
    directory: tempfile::TempDir,
    media: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let media = directory.path().join("synthetic.wav");
        let data_bytes = (SYNTHETIC_WAVE_FRAMES * 2) as u32;
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
        for _ in 0..SYNTHETIC_WAVE_FRAMES {
            bytes.extend_from_slice(&901i16.to_le_bytes());
        }
        fs::write(&media, bytes).unwrap();
        Self { directory, media }
    }
    fn request(&self, subtitles: Vec<LocalSubtitleInput>) -> PreparedRequest {
        PreparedRequest::new(MediaInput::local(self.media.clone()).unwrap(), subtitles).unwrap()
    }
    fn subtitle(&self, name: &str, text: &str, language: TextLanguage) -> LocalSubtitleInput {
        let path = self.directory.path().join(name);
        fs::write(&path, text).unwrap();
        LocalSubtitleInput::new(path, SubtitleFormat::Srt, language).unwrap()
    }
    fn chinese(&self) -> PreparedRequest {
        self.request(vec![self.subtitle(
            "zh.srt",
            "1\n00:00:00,004 --> 00:00:00,016\n软件与服务器\n",
            TextLanguage::SimplifiedChinese,
        )])
    }
    fn writable(&self) -> bool {
        OpenOptions::new().write(true).open(&self.media).is_ok()
    }
}
fn session() -> PreparedSession {
    let mut session = PreparedSession::new();
    session.workers = Arc::new(Semaphore::new(1));
    session
}
fn with_fixture_lane<T>(future: impl Future<Output = T>) -> T {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}
async fn bounded<F: Future>(future: F) -> F::Output {
    tokio::time::timeout(TEST_TIMEOUT, future)
        .await
        .expect("synthetic preparation exceeded its bounded timeout")
}
async fn entered<F: Future<Output = PreparedCompletion>>(work: &mut Pin<Box<F>>, signal: &Notify) {
    tokio::select! {
        completion = work.as_mut() => panic!("gated synthetic preparation finished early: {:?}", completion.result.err()),
        () = bounded(signal.notified()) => {},
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Immediate,
    Forbidden,
    Gated,
    Fail,
}
struct Recognizer {
    mode: Mode,
    language: SpeechLanguage,
    unsupported: bool,
    capabilities: AtomicUsize,
    calls: AtomicUsize,
    entered: Notify,
    release: Notify,
    token: Mutex<Option<ResolverCancellation>>,
}
impl Recognizer {
    fn new(mode: Mode, language: SpeechLanguage) -> Self {
        Self {
            mode,
            language,
            unsupported: false,
            capabilities: AtomicUsize::new(0),
            calls: AtomicUsize::new(0),
            entered: Notify::new(),
            release: Notify::new(),
            token: Mutex::new(None),
        }
    }
}
impl PreparedSpeechRecognizer for Recognizer {
    fn capabilities(&self) -> PreparedSpeechCapabilities {
        self.capabilities.fetch_add(1, Ordering::SeqCst);
        assert!(
            self.mode != Mode::Forbidden,
            "subtitle preference must not query ASR capabilities"
        );
        PreparedSpeechCapabilities {
            cancellation: !self.unsupported,
            timestamped_finals: true,
            supported_languages: [true; 5],
            constrained_auto_detection: true,
            sample_rates: [true; 2],
            max_concurrent: 2,
        }
    }
    fn recognize<'a>(
        &'a self,
        request: &'a PreparedSpeechRequest,
        cancel: ResolverCancellation,
    ) -> PreparedSpeechFuture<'a> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            *self.token.lock().unwrap() = Some(cancel);
            self.entered.notify_one();
            if self.mode == Mode::Gated {
                self.release.notified().await;
            }
            if self.mode == Mode::Fail {
                return Err(PreparedSpeechError::Network);
            }
            let stamp = request.chunk().stamp();
            Ok(PreparedSpeechResponse {
                identity: stamp.identity,
                sequence: stamp.sequence,
                run_id: stamp.run_id,
                language: Some(self.language),
                cues: vec![PreparedSpeechCue {
                    start_us: 4000,
                    duration_us: 12_000,
                    text: "synthetic recognized dialogue".into(),
                }],
            })
        })
    }
}
struct TranslatorMock {
    mode: Mode,
    calls: AtomicUsize,
    entered: Notify,
    release: Notify,
    token: Mutex<Option<TranslationCancellation>>,
    glossary: Mutex<Vec<SeenTerms>>,
}
impl TranslatorMock {
    fn new(mode: Mode) -> Self {
        Self {
            mode,
            calls: AtomicUsize::new(0),
            entered: Notify::new(),
            release: Notify::new(),
            token: Mutex::new(None),
            glossary: Mutex::new(Vec::new()),
        }
    }
}
impl Translator for TranslatorMock {
    fn capabilities(&self) -> TranslationCapabilities {
        assert!(
            self.mode != Mode::Forbidden,
            "Chinese preference must not query translator capabilities"
        );
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
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.glossary.lock().unwrap().push(
                request
                    .glossary
                    .iter()
                    .map(|entry| (entry.source.clone(), entry.taiwan_translation.clone()))
                    .collect(),
            );
            *self.token.lock().unwrap() = Some(cancel);
            self.entered.notify_one();
            if self.mode == Mode::Gated {
                self.release.notified().await;
            }
            if self.mode == Mode::Fail {
                return Err(TranslationError::RateLimited);
            }
            Ok(format!(
                "软件与服务器 {}",
                if request.pass == TranslationPass::Review {
                    "校訂"
                } else {
                    "草稿"
                }
            )
            .into())
        })
    }
}

#[test]
fn native_asr_translation_publishes_ready_with_genuine_recognized_provenance() {
    with_fixture_lane(async {
        let fixture = Fixture::new();
        let before = fs::read(&fixture.media).unwrap();
        let cpu = crate::media::work::workers();
        let native = crate::media::work::native_owners();
        let cpu_before = cpu.available_permits();
        let native_before = native.available_permits();
        let mut session = session();
        let task = session.start(fixture.request(Vec::new())).unwrap();
        let identity = task.identity;
        let recognizer = Recognizer::new(Mode::Immediate, SpeechLanguage::Japanese);
        let translator = TranslatorMock::new(Mode::Immediate);
        let completion = bounded(task.run_with_recognizer(
            &recognizer,
            &translator,
            SourceLanguage::Auto,
            PreparedSampleRate::Hz24000,
            TranslationQuality::Standard,
            None,
        ))
        .await;
        assert!(session.ready().is_none());
        assert!(!fixture.writable());
        assert_eq!(cpu.available_permits(), cpu_before);
        assert_eq!(native.available_permits(), native_before);
        assert!(session.apply(completion).unwrap());
        assert_eq!(session.state(), PreparedState::Ready);
        assert_eq!(session.app_state(), AppState::PreparedReady);
        let document = session.ready().unwrap();
        assert_eq!(document.identity(), identity);
        assert_eq!(document.sequence(), 1);
        assert_eq!(document.cue_count(), 1);
        assert_eq!(document.quality(), Some(TranslationQuality::Standard));
        assert_eq!(document.target_language(), "zh-TW");
        assert!(document.original().existing().is_none());
        let original = document.original().recognized().unwrap();
        assert_eq!(original.identity(), identity);
        assert_eq!(original.language(), SpeechLanguage::Japanese);
        assert_eq!(original.cues()[0].start_us, 4000);
        assert_eq!(original.cues()[0].duration_us, 12_000);
        assert_eq!(
            original.cues()[0].text.as_ref(),
            "synthetic recognized dialogue"
        );
        assert_eq!(document.translated()[0].as_ref(), "軟體與伺服器 校訂");
        assert_eq!(recognizer.calls.load(Ordering::SeqCst), 1);
        assert_eq!(translator.calls.load(Ordering::SeqCst), 2);
        assert_eq!(document.media_byte_len(), before.len() as u64);
        assert!(!fixture.writable());
        assert_eq!(fs::read(&fixture.media).unwrap(), before);
        assert_eq!(fs::read_dir(fixture.directory.path()).unwrap().count(), 1);
        let capture = StreamIdentity {
            session: SessionId(991_117),
            generation: 1,
        };
        let mut locator = session.fuzzy_locator(capture, 1).unwrap();
        locator.isolate_workers(Arc::new(Semaphore::new(1)));
        let proposal = locator
            .locate(
                crate::ai::speech::TranscriptEvent {
                    identity: capture,
                    utterance: 1,
                    sequence: 7,
                    revision: 1,
                    start_us: Some(42_000_000),
                    duration_us: Some(12_000),
                    language: Some(SpeechLanguage::Japanese),
                    kind: crate::ai::speech::TranscriptKind::Final,
                    text: "synthetic recognized dialogue".into(),
                },
                crate::resolver::contracts::WindowStamp {
                    identity: capture,
                    sequence: 9,
                    ended_at: Instant::now(),
                    start_us: Some(40_000_000),
                    duration_us: 12_000_000,
                },
                ResolverCancellation::default(),
            )
            .await
            .unwrap();
        assert_eq!(
            proposal.origin(),
            crate::subtitle::fuzzy_alignment::FuzzyTextOrigin::PreparedRecognition
        );
        assert_eq!(proposal.media_range_us(), 4000..16_000);
        assert!(session.ready().unwrap().original().existing().is_none());
        session.stop();
        assert_eq!(
            proposal.validate(locator.scope(), proposal.binding()),
            Err(ResolverError::Cancelled.into())
        );
        assert!(session.ready().is_none());
        assert!(fixture.writable());
    });
}

#[test]
fn reliable_chinese_preference_does_not_query_either_provider_or_stale_glossary() {
    with_fixture_lane(async {
        let fixture = Fixture::new();
        let mut session = session();
        let lower = fixture.subtitle(
            "source.srt",
            "invalid lower priority data",
            TextLanguage::Source(SpeechLanguage::English),
        );
        let chinese = fixture.subtitle(
            "zh.srt",
            "1\n00:00:00,004 --> 00:00:00,016\n软件\n",
            TextLanguage::TaiwanTraditional,
        );
        let task = session
            .start(fixture.request(vec![lower, chinese]))
            .unwrap();
        let old = SessionGlossary::new(StreamIdentity {
            session: task.identity.session,
            generation: 2,
        });
        let recognizer = Recognizer::new(Mode::Forbidden, SpeechLanguage::English);
        let translator = TranslatorMock::new(Mode::Forbidden);
        let completion = bounded(task.run_with_recognizer(
            &recognizer,
            &translator,
            SourceLanguage::English,
            PreparedSampleRate::Hz16000,
            TranslationQuality::High,
            Some(&old),
        ))
        .await;
        assert!(session.apply(completion).unwrap());
        let document = session.ready().unwrap();
        assert!(document.original().recognized().is_none());
        assert_eq!(
            document.original().existing().unwrap().source,
            SubtitleSource::ExistingSubtitle
        );
        assert_eq!(document.translated()[0].as_ref(), "軟體");
        assert_eq!(recognizer.capabilities.load(Ordering::SeqCst), 0);
        assert_eq!(recognizer.calls.load(Ordering::SeqCst), 0);
        assert_eq!(translator.calls.load(Ordering::SeqCst), 0);
        assert_eq!(document.quality(), None);
        session.stop();
    });
}

#[test]
fn source_subtitle_preference_translates_without_querying_recognizer() {
    with_fixture_lane(async {
        let fixture = Fixture::new();
        let subtitle = fixture.subtitle(
            "source.srt",
            "1\n00:00:00,004 --> 00:00:00,016\nsynthetic sidecar source\n",
            TextLanguage::Source(SpeechLanguage::German),
        );
        let mut session = session();
        let task = session.start(fixture.request(vec![subtitle])).unwrap();
        let recognizer = Recognizer::new(Mode::Forbidden, SpeechLanguage::English);
        let translator = TranslatorMock::new(Mode::Immediate);
        let completion = bounded(task.run_with_recognizer(
            &recognizer,
            &translator,
            SourceLanguage::Auto,
            PreparedSampleRate::Hz24000,
            TranslationQuality::Fast,
            None,
        ))
        .await;
        session.apply(completion).unwrap();
        let document = session.ready().unwrap();
        assert!(document.original().recognized().is_none());
        let original = document.original().existing().unwrap();
        assert!(original.language == TextLanguage::Source(SpeechLanguage::German));
        assert_eq!(original.cues[0].text.as_ref(), "synthetic sidecar source");
        assert_eq!(recognizer.capabilities.load(Ordering::SeqCst), 0);
        assert_eq!(recognizer.calls.load(Ordering::SeqCst), 0);
        assert_eq!(translator.calls.load(Ordering::SeqCst), 1);
        session.stop();
    });
}

#[test]
fn invalid_or_ambiguous_preferred_subtitle_never_falls_back_to_asr() {
    with_fixture_lane(async {
        for ambiguous in [false, true] {
            let fixture = Fixture::new();
            let mut session = session();
            let first = fixture.subtitle(
                "one.srt",
                "invalid selected subtitle",
                TextLanguage::TaiwanTraditional,
            );
            let second = fixture.subtitle(
                "two.srt",
                "1\n00:00:00,004 --> 00:00:00,016\nsynthetic source\n",
                if ambiguous {
                    TextLanguage::TaiwanTraditional
                } else {
                    TextLanguage::Source(SpeechLanguage::English)
                },
            );
            let task = session.start(fixture.request(vec![first, second])).unwrap();
            let recognizer = Recognizer::new(Mode::Forbidden, SpeechLanguage::English);
            let translator = TranslatorMock::new(Mode::Forbidden);
            let completion = bounded(task.run_with_recognizer(
                &recognizer,
                &translator,
                SourceLanguage::English,
                PreparedSampleRate::Hz16000,
                TranslationQuality::Fast,
                None,
            ))
            .await;
            let error = if ambiguous {
                MediaError::AmbiguousSubtitle
            } else {
                MediaError::InvalidSubtitle
            };
            assert_eq!(session.apply(completion), Err(error));
            assert_eq!(session.state(), PreparedState::Failed(error));
            assert!(session.ready().is_none());
            assert_eq!(recognizer.capabilities.load(Ordering::SeqCst), 0);
            assert_eq!(recognizer.calls.load(Ordering::SeqCst), 0);
            assert_eq!(translator.calls.load(Ordering::SeqCst), 0);
            assert!(fixture.writable());
        }
    });
}

#[test]
fn stale_session_or_generation_glossary_is_rejected_before_asr_capabilities() {
    with_fixture_lane(async {
        for stale_generation in [false, true] {
            let fixture = Fixture::new();
            let mut session = session();
            let task = session.start(fixture.request(Vec::new())).unwrap();
            let current = task.identity;
            let stale = if stale_generation {
                StreamIdentity {
                    generation: current.generation + 1,
                    ..current
                }
            } else {
                StreamIdentity {
                    session: SessionId(current.session.0.checked_add(1).unwrap()),
                    ..current
                }
            };
            let glossary = SessionGlossary::new(stale);
            let recognizer = Recognizer::new(Mode::Forbidden, SpeechLanguage::English);
            let translator = TranslatorMock::new(Mode::Forbidden);
            let completion = bounded(task.run_with_recognizer(
                &recognizer,
                &translator,
                SourceLanguage::English,
                PreparedSampleRate::Hz16000,
                TranslationQuality::Fast,
                Some(&glossary),
            ))
            .await;
            assert!(!session.cancellation.is_cancelled());
            let error = MediaError::Translation(TranslationError::InvalidData);
            assert_eq!(session.apply(completion), Err(error));
            assert_eq!(session.state(), PreparedState::Failed(error));
            assert!(session.ready().is_none());
            assert_eq!(recognizer.capabilities.load(Ordering::SeqCst), 0);
            assert_eq!(translator.calls.load(Ordering::SeqCst), 0);
            assert!(fixture.writable());
        }
    });
}

#[test]
fn same_session_glossary_reaches_the_selected_translator() {
    with_fixture_lane(async {
        let fixture = Fixture::new();
        let mut session = session();
        let task = session.start(fixture.request(Vec::new())).unwrap();
        let mut glossary = SessionGlossary::new(task.identity);
        glossary.set(task.identity, "synthetic", "合成").unwrap();
        let recognizer = Recognizer::new(Mode::Immediate, SpeechLanguage::English);
        let translator = TranslatorMock::new(Mode::Immediate);
        let completion = bounded(task.run_with_recognizer(
            &recognizer,
            &translator,
            SourceLanguage::English,
            PreparedSampleRate::Hz16000,
            TranslationQuality::High,
            Some(&glossary),
        ))
        .await;
        session.apply(completion).unwrap();
        assert_eq!(
            session.ready().unwrap().quality(),
            Some(TranslationQuality::High)
        );
        let observed = translator.glossary.lock().unwrap();
        assert_eq!(observed.len(), 2);
        assert!(observed.iter().all(|entries| entries.len() == 1
            && entries[0].0.as_ref() == "synthetic"
            && entries[0].1.as_ref() == "合成"));
        session.stop();
    });
}

#[test]
fn input_pin_survives_final_asr_and_translation_wait_then_unapplied_completion() {
    with_fixture_lane(async {
        let fixture = Fixture::new();
        let cpu = crate::media::work::workers();
        let native = crate::media::work::native_owners();
        let cpu_before = cpu.available_permits();
        let native_before = native.available_permits();
        let mut session = session();
        let task = session.start(fixture.request(Vec::new())).unwrap();
        let recognizer = Recognizer::new(Mode::Gated, SpeechLanguage::English);
        let translator = TranslatorMock::new(Mode::Gated);
        let mut work = Box::pin(task.run_with_recognizer(
            &recognizer,
            &translator,
            SourceLanguage::English,
            PreparedSampleRate::Hz16000,
            TranslationQuality::Fast,
            None,
        ));
        entered(&mut work, &recognizer.entered).await;
        assert!(!fixture.writable());
        assert_eq!(cpu.available_permits(), cpu_before);
        assert_eq!(native.available_permits(), native_before);
        recognizer.release.notify_one();
        entered(&mut work, &translator.entered).await;
        assert!(!fixture.writable());
        assert_eq!(cpu.available_permits(), cpu_before);
        assert_eq!(native.available_permits(), native_before);
        translator.release.notify_one();
        let completion = bounded(work).await;
        assert!(completion.result.is_ok());
        assert!(session.ready().is_none());
        assert!(!fixture.writable());
        drop(completion);
        assert!(fixture.writable());
        assert_eq!(cpu.available_permits(), cpu_before);
        assert_eq!(native.available_permits(), native_before);
        session.stop();
    });
}

#[test]
fn typed_speech_or_translation_failure_keeps_failure_kind_and_releases_pin() {
    with_fixture_lane(async {
        for speech_failure in [false, true] {
            let fixture = Fixture::new();
            let mut session = session();
            let task = session.start(fixture.request(Vec::new())).unwrap();
            let recognizer = Recognizer::new(
                if speech_failure {
                    Mode::Fail
                } else {
                    Mode::Immediate
                },
                SpeechLanguage::English,
            );
            let translator = TranslatorMock::new(if speech_failure {
                Mode::Forbidden
            } else {
                Mode::Fail
            });
            let completion = bounded(task.run_with_recognizer(
                &recognizer,
                &translator,
                SourceLanguage::English,
                PreparedSampleRate::Hz16000,
                TranslationQuality::Fast,
                None,
            ))
            .await;
            // Lower failure cancels only its child scope; apply can retain the
            // real typed failure instead of mistaking it for a user stop.
            assert!(!session.cancellation.is_cancelled());
            let error = if speech_failure {
                MediaError::Speech(PreparedSpeechError::Network)
            } else {
                MediaError::Translation(TranslationError::RateLimited)
            };
            assert_eq!(session.apply(completion), Err(error));
            assert_eq!(session.state(), PreparedState::Failed(error));
            assert!(session.ready().is_none());
            assert!(fixture.writable());
            if speech_failure {
                assert_eq!(translator.calls.load(Ordering::SeqCst), 0);
            }
        }
    });
}

#[test]
fn unsupported_recognizer_or_media_returns_error_without_fake_ready() {
    with_fixture_lane(async {
        for unsupported_recognizer in [false, true] {
            let fixture = Fixture::new();
            if !unsupported_recognizer {
                fs::write(&fixture.media, b"explicit opaque unsupported input").unwrap();
            }
            let mut session = session();
            let task = session.start(fixture.request(Vec::new())).unwrap();
            let mut recognizer = Recognizer::new(Mode::Immediate, SpeechLanguage::English);
            recognizer.unsupported = unsupported_recognizer;
            let translator = TranslatorMock::new(Mode::Forbidden);
            let completion = bounded(task.run_with_recognizer(
                &recognizer,
                &translator,
                SourceLanguage::English,
                PreparedSampleRate::Hz16000,
                TranslationQuality::Fast,
                None,
            ))
            .await;
            let error = if unsupported_recognizer {
                MediaError::Speech(PreparedSpeechError::UnsupportedCapability)
            } else {
                MediaError::UnsupportedMedia
            };
            assert_eq!(session.apply(completion), Err(error));
            assert_eq!(session.state(), PreparedState::Failed(error));
            assert!(session.ready().is_none());
            assert_eq!(recognizer.calls.load(Ordering::SeqCst), 0);
            assert_eq!(translator.calls.load(Ordering::SeqCst), 0);
            assert!(fixture.writable());
        }
    });
}

#[test]
fn unpolled_task_drop_cancels_without_io_or_provider_work() {
    with_fixture_lane(async {
        let fixture = Fixture::new();
        let mut session = session();
        let task = session.start(fixture.request(Vec::new())).unwrap();
        let recognizer = Recognizer::new(Mode::Forbidden, SpeechLanguage::English);
        let translator = TranslatorMock::new(Mode::Forbidden);
        drop(task.run_with_recognizer(
            &recognizer,
            &translator,
            SourceLanguage::English,
            PreparedSampleRate::Hz16000,
            TranslationQuality::Fast,
            None,
        ));
        assert_eq!(session.state(), PreparedState::Cancelled);
        assert!(session.ready().is_none());
        assert_eq!(recognizer.capabilities.load(Ordering::SeqCst), 0);
        assert_eq!(translator.calls.load(Ordering::SeqCst), 0);
        assert!(fixture.writable());
    });
}

#[test]
fn stop_during_final_asr_wait_cancels_provider_and_releases_pin() {
    with_fixture_lane(async {
        let fixture = Fixture::new();
        let mut session = session();
        let task = session.start(fixture.request(Vec::new())).unwrap();
        let recognizer = Recognizer::new(Mode::Gated, SpeechLanguage::English);
        let translator = TranslatorMock::new(Mode::Forbidden);
        let mut work = Box::pin(task.run_with_recognizer(
            &recognizer,
            &translator,
            SourceLanguage::English,
            PreparedSampleRate::Hz16000,
            TranslationQuality::Fast,
            None,
        ));
        entered(&mut work, &recognizer.entered).await;
        assert!(!fixture.writable());
        session.stop();
        let completion = bounded(work).await;
        assert_eq!(completion.result.err(), Some(MediaError::Cancelled));
        assert!(
            recognizer
                .token
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .is_cancelled()
        );
        assert_eq!(session.state(), PreparedState::Cancelled);
        assert!(session.ready().is_none());
        assert!(fixture.writable());
    });
}

#[test]
fn future_drop_during_translation_cancels_request_and_releases_retained_pin() {
    with_fixture_lane(async {
        let fixture = Fixture::new();
        let mut session = session();
        let task = session.start(fixture.request(Vec::new())).unwrap();
        let recognizer = Recognizer::new(Mode::Immediate, SpeechLanguage::English);
        let translator = TranslatorMock::new(Mode::Gated);
        let mut work = Box::pin(task.run_with_recognizer(
            &recognizer,
            &translator,
            SourceLanguage::English,
            PreparedSampleRate::Hz16000,
            TranslationQuality::Fast,
            None,
        ));
        entered(&mut work, &translator.entered).await;
        assert!(!fixture.writable());
        drop(work);
        assert_eq!(session.state(), PreparedState::Cancelled);
        assert!(session.ready().is_none());
        assert_eq!(
            translator.token.lock().unwrap().as_ref().unwrap().reason(),
            Some(TranslationError::Cancelled)
        );
        assert!(fixture.writable());
    });
}

#[test]
fn new_session_rejects_pending_old_completion_without_touching_new_ready() {
    with_fixture_lane(async {
        let fixture = Fixture::new();
        let mut session = session();
        let old = session.start(fixture.request(Vec::new())).unwrap();
        let old_identity = old.identity;
        let recognizer = Recognizer::new(Mode::Gated, SpeechLanguage::English);
        let translator = TranslatorMock::new(Mode::Forbidden);
        let mut work = Box::pin(old.run_with_recognizer(
            &recognizer,
            &translator,
            SourceLanguage::English,
            PreparedSampleRate::Hz16000,
            TranslationQuality::Fast,
            None,
        ));
        entered(&mut work, &recognizer.entered).await;
        let next = session.start(fixture.chinese()).unwrap();
        let next_identity = next.identity;
        assert_ne!(old_identity, next_identity);
        let old_completion = bounded(work).await;
        session.apply(bounded(next.run()).await).unwrap();
        assert_eq!(session.apply(old_completion), Err(MediaError::Stale));
        assert_eq!(session.state(), PreparedState::Ready);
        assert_eq!(session.ready().unwrap().identity(), next_identity);
        assert!(session.ready().unwrap().original().recognized().is_none());
        session.stop();
        assert!(fixture.writable());
    });
}

#[test]
fn late_successful_completion_releases_its_pin_and_preserves_current_ready() {
    with_fixture_lane(async {
        let old_fixture = Fixture::new();
        let new_fixture = Fixture::new();
        let mut session = session();
        let old = session.start(old_fixture.request(Vec::new())).unwrap();
        let recognizer = Recognizer::new(Mode::Immediate, SpeechLanguage::English);
        let translator = TranslatorMock::new(Mode::Immediate);
        let old_completion = bounded(old.run_with_recognizer(
            &recognizer,
            &translator,
            SourceLanguage::English,
            PreparedSampleRate::Hz16000,
            TranslationQuality::Fast,
            None,
        ))
        .await;
        assert!(old_completion.result.is_ok());
        assert!(!old_fixture.writable());
        let next = session.start(new_fixture.chinese()).unwrap();
        let identity = next.identity;
        session.apply(bounded(next.run()).await).unwrap();
        assert_eq!(session.apply(old_completion), Err(MediaError::Stale));
        assert!(old_fixture.writable());
        assert!(!new_fixture.writable());
        assert_eq!(session.ready().unwrap().identity(), identity);
        assert_eq!(session.state(), PreparedState::Ready);
        session.stop();
        assert!(new_fixture.writable());
    });
}
