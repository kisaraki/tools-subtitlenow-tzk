//! Real loopback WAVE ranges, explicit service mocks, no keys or user media.
use super::*;
use crate::{
    ai::{
        prepared_glossary::{
            PreparedGlossaryFuture, PreparedGlossaryProposal, PreparedGlossaryRequest,
            PreparedGlossaryResponse,
        },
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
    media::{
        input::{LocalSubtitleInput, MediaInput},
        remote_wave::test_source::{Source, samples, wave},
    },
    resolver::timed_subtitle::SubtitleFormat,
};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::Notify;

const TEST_TIMEOUT: Duration = Duration::from_secs(8);
#[derive(Clone, Copy, PartialEq)]
enum Stage {
    None,
    Asr,
    Analysis,
    Translation,
}
struct Services {
    gate: Stage,
    fail: Stage,
    unsupported: bool,
    stale: bool,
    invalid_summary: bool,
    capabilities: AtomicUsize,
    asr: AtomicUsize,
    analysis: AtomicUsize,
    translations: AtomicUsize,
    active: AtomicUsize,
    entered: Notify,
    release: Notify,
    text_rows: Mutex<Vec<(TranslationPass, usize)>>,
    changed: Option<Arc<crate::media::remote_wave::test_source::Control>>,
}
impl Services {
    fn new() -> Self {
        Self {
            gate: Stage::None,
            fail: Stage::None,
            unsupported: false,
            stale: false,
            invalid_summary: false,
            capabilities: AtomicUsize::new(0),
            asr: AtomicUsize::new(0),
            analysis: AtomicUsize::new(0),
            translations: AtomicUsize::new(0),
            active: AtomicUsize::new(0),
            entered: Notify::new(),
            release: Notify::new(),
            text_rows: Mutex::new(Vec::new()),
            changed: None,
        }
    }
    async fn wait(&self, stage: Stage) {
        if self.gate == stage {
            self.entered.notify_one();
            self.release.notified().await;
        }
    }
    fn counts(&self) -> (usize, usize, usize) {
        (
            self.asr.load(Ordering::SeqCst),
            self.analysis.load(Ordering::SeqCst),
            self.translations.load(Ordering::SeqCst),
        )
    }
}
struct Active<'a>(&'a AtomicUsize);
impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
impl PreparedSpeechRecognizer for Services {
    fn capabilities(&self) -> PreparedSpeechCapabilities {
        self.capabilities.fetch_add(1, Ordering::SeqCst);
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
        _: ResolverCancellation,
    ) -> PreparedSpeechFuture<'a> {
        Box::pin(async move {
            self.asr.fetch_add(1, Ordering::SeqCst);
            self.active.fetch_add(1, Ordering::SeqCst);
            let _active = Active(&self.active);
            self.wait(Stage::Asr).await;
            if self.fail == Stage::Asr {
                return Err(PreparedSpeechError::Credential);
            }
            if let Some(changed) = &self.changed {
                changed.changed.store(true, Ordering::Release);
            }
            let stamp = request.chunk().stamp();
            let mut identity = stamp.identity;
            if self.stale {
                identity.generation += 1;
            }
            Ok(PreparedSpeechResponse {
                identity,
                sequence: stamp.sequence,
                run_id: stamp.run_id,
                language: Some(SpeechLanguage::English),
                cues: vec![PreparedSpeechCue {
                    start_us: stamp.body_start_us - stamp.audio_start_us,
                    duration_us: stamp.body_end_us - stamp.body_start_us,
                    text: "synthetic recognized dialogue".into(),
                }],
            })
        })
    }
}
impl PreparedGlossaryAnalyzer for Services {
    fn analyze<'a>(
        &'a self,
        request: &'a PreparedGlossaryRequest,
        _: TranslationCancellation,
    ) -> PreparedGlossaryFuture<'a> {
        Box::pin(async move {
            assert_eq!(self.translations.load(Ordering::SeqCst), 0);
            self.analysis.fetch_add(1, Ordering::SeqCst);
            self.active.fetch_add(1, Ordering::SeqCst);
            let _active = Active(&self.active);
            self.wait(Stage::Analysis).await;
            if self.fail == Stage::Analysis {
                return Err(TranslationError::Refused);
            }
            Ok(PreparedGlossaryResponse {
                identity: request.identity(),
                sequence: request.sequence(),
                proposals: vec![PreparedGlossaryProposal {
                    cue_index: request.cues()[0].cue_index,
                    source_byte_start: 0,
                    source_byte_end: 9,
                    taiwan_translation: "合成内容".into(),
                }],
                context_summary: if self.invalid_summary {
                    "".into()
                } else {
                    "本次合成全文摘要".into()
                },
            })
        })
    }
}
impl Translator for Services {
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
        _: TranslationCancellation,
    ) -> TranslationFuture<'a> {
        Box::pin(async move {
            assert!(self.analysis.load(Ordering::SeqCst) > 0);
            let context = request
                .prepared_context
                .as_ref()
                .unwrap()
                .document
                .as_ref()
                .unwrap();
            assert_eq!(context.summary(), "本次合成全文摘要");
            assert!(request.glossary.iter().any(
                |term| &*term.source == "synthetic" && &*term.taiwan_translation == "合成內容"
            ));
            self.text_rows
                .lock()
                .unwrap()
                .push((request.pass, Arc::as_ptr(context) as usize));
            self.translations.fetch_add(1, Ordering::SeqCst);
            self.active.fetch_add(1, Ordering::SeqCst);
            let _active = Active(&self.active);
            self.wait(Stage::Translation).await;
            if self.fail == Stage::Translation {
                return Err(TranslationError::RateLimited);
            }
            Ok(if request.pass == TranslationPass::Review {
                "软件与服务器 校訂".into()
            } else {
                "软件与服务器 草稿".into()
            })
        })
    }
}
fn lane<T>(work: impl Future<Output = T>) -> T {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(work)
}
fn request(source: &Source) -> PreparedRequest {
    PreparedRequest::new(source.media_input(), vec![]).unwrap()
}
async fn run(
    task: PreparedTask,
    services: &Services,
    rate: PreparedSampleRate,
    quality: TranslationQuality,
) -> PreparedCompletion {
    tokio::time::timeout(
        TEST_TIMEOUT,
        task.run_with_remote_wave_recognizer(
            services,
            services,
            SourceLanguage::English,
            rate,
            quality,
            None,
        ),
    )
    .await
    .unwrap()
}
async fn entered<F: Future<Output = PreparedCompletion>>(
    work: &mut Pin<Box<F>>,
    services: &Services,
) {
    tokio::select! { result=work.as_mut()=>panic!("early {:?}",result.result.err()), result=tokio::time::timeout(TEST_TIMEOUT,services.entered.notified())=>result.unwrap() }
}

#[test]
fn remote_ready_both_rates_all_qualities_share_analysis_context_taiwan_text_and_media_time() {
    lane(async {
        for rate in [PreparedSampleRate::Hz16000, PreparedSampleRate::Hz24000] {
            for quality in [
                TranslationQuality::Fast,
                TranslationQuality::Standard,
                TranslationQuality::High,
            ] {
                let bytes = wave(44100, 2, &samples(11025, 2));
                let length = bytes.len() as u64;
                let source = Source::new(bytes).await;
                let mut owner = PreparedSession::new();
                let task = owner.start(request(&source)).unwrap();
                let identity = task.identity;
                let services = Services::new();
                let completion = run(task, &services, rate, quality).await;
                assert!(owner.ready().is_none());
                assert!(completion.result.is_ok(), "{:?}", completion.result.err());
                assert!(owner.apply(completion).unwrap());
                assert_eq!(owner.state(), PreparedState::Ready);
                assert_eq!(owner.app_state(), AppState::PreparedReady);
                let document = owner.ready().unwrap();
                assert_eq!(document.identity(), identity);
                assert_eq!(document.media_byte_len(), length);
                assert_eq!(document.sequence(), 1);
                assert_eq!(document.cue_count(), 1);
                assert_eq!(document.quality(), Some(quality));
                assert_eq!(document.target_language(), "zh-TW");
                assert!(document.original().existing().is_none());
                let original = document.original().recognized().unwrap();
                assert_eq!(original.cues()[0].start_us, 0);
                assert_eq!(original.cues()[0].duration_us, 250000);
                assert_eq!(
                    document.translated()[0].as_ref(),
                    if quality == TranslationQuality::Fast {
                        "軟體與伺服器 草稿"
                    } else {
                        "軟體與伺服器 校訂"
                    }
                );
                assert!(matches!(document._media, PreparedMediaOwner::Remote(_)));
                {
                    let rows = services.text_rows.lock().unwrap();
                    assert_eq!(
                        rows.len(),
                        if quality == TranslationQuality::Fast {
                            1
                        } else {
                            2
                        }
                    );
                    assert!(rows.iter().all(|row| row.1 == rows[0].1));
                }
                let reads = source.rows.lock().unwrap().len();
                owner.stop();
                assert!(owner.ready().is_none());
                assert_eq!(source.rows.lock().unwrap().len(), reads);
                source.finish().await;
            }
        }
    });
}

#[test]
fn remote_ready_retains_http_owner_during_analysis_translation_completion_and_ready() {
    lane(async {
        for stage in [Stage::Analysis, Stage::Translation] {
            let source = Source::new(wave(16000, 1, &samples(1600, 1))).await;
            let mut owner = PreparedSession::new();
            let task = owner.start(request(&source)).unwrap();
            let identity = task.identity;
            let mut services = Services::new();
            services.gate = stage;
            let mut work = Box::pin(run(
                task,
                &services,
                PreparedSampleRate::Hz16000,
                TranslationQuality::Fast,
            ));
            entered(&mut work, &services).await;
            let reads = source.rows.lock().unwrap().len();
            let cpu = crate::media::work::workers();
            assert_eq!(cpu.available_permits(), crate::media::work::worker_limit());
            let spare = RemoteHttpMedia::open(
                source.media_input(),
                identity,
                ResolverCancellation::default(),
            )
            .await
            .unwrap();
            assert!(matches!(
                RemoteHttpMedia::open(
                    source.media_input(),
                    identity,
                    ResolverCancellation::default()
                )
                .await,
                Err(RemoteMediaError::Overloaded)
            ));
            services.release.notify_one();
            let completion = work.await;
            assert!(completion.result.is_ok());
            assert!(matches!(
                RemoteHttpMedia::open(
                    source.media_input(),
                    identity,
                    ResolverCancellation::default()
                )
                .await,
                Err(RemoteMediaError::Overloaded)
            ));
            assert!(owner.apply(completion).unwrap());
            assert!(matches!(
                RemoteHttpMedia::open(
                    source.media_input(),
                    identity,
                    ResolverCancellation::default()
                )
                .await,
                Err(RemoteMediaError::Overloaded)
            ));
            assert_eq!(source.rows.lock().unwrap().len(), reads + 1); // Only spare's one-byte probe.
            owner.stop();
            let replacement = RemoteHttpMedia::open(
                source.media_input(),
                identity,
                ResolverCancellation::default(),
            )
            .await
            .unwrap();
            drop(replacement);
            drop(spare);
            source.finish().await;
        }
    });
}

#[test]
fn remote_ready_failures_never_publish_partial_text_and_keep_typed_causes() {
    lane(async {
        for stage in [Stage::Asr, Stage::Analysis, Stage::Translation] {
            let source = Source::new(wave(16000, 1, &samples(1600, 1))).await;
            let mut owner = PreparedSession::new();
            let task = owner.start(request(&source)).unwrap();
            let mut services = Services::new();
            services.fail = stage;
            let completion = run(
                task,
                &services,
                PreparedSampleRate::Hz16000,
                TranslationQuality::Standard,
            )
            .await;
            let expected = match stage {
                Stage::Asr => MediaError::Speech(PreparedSpeechError::Credential),
                Stage::Analysis => MediaError::Translation(TranslationError::Refused),
                _ => MediaError::Translation(TranslationError::RateLimited),
            };
            assert_eq!(owner.apply(completion), Err(expected));
            assert_eq!(owner.state(), PreparedState::Failed(expected));
            assert!(owner.ready().is_none());
            if stage != Stage::Translation {
                assert_eq!(services.translations.load(Ordering::SeqCst), 0);
            }
            if stage == Stage::Asr {
                assert_eq!(services.analysis.load(Ordering::SeqCst), 0);
            }
            assert_eq!(services.active.load(Ordering::SeqCst), 0);
            source.finish().await;
        }
    });
}

#[test]
fn remote_ready_invalid_summary_or_old_recognition_cannot_reach_translation() {
    lane(async {
        for stale in [false, true] {
            let source = Source::new(wave(16000, 1, &samples(1600, 1))).await;
            let mut owner = PreparedSession::new();
            let task = owner.start(request(&source)).unwrap();
            let mut services = Services::new();
            services.stale = stale;
            services.invalid_summary = !stale;
            let completion = run(
                task,
                &services,
                PreparedSampleRate::Hz16000,
                TranslationQuality::Standard,
            )
            .await;
            assert!(owner.apply(completion).is_err());
            assert!(owner.ready().is_none());
            assert_eq!(services.translations.load(Ordering::SeqCst), 0);
            if stale {
                assert_eq!(services.analysis.load(Ordering::SeqCst), 0);
            }
            source.finish().await;
        }
    });
}

#[test]
fn remote_ready_late_source_change_preserves_transport_error_without_analysis() {
    lane(async {
        let source = Source::new(wave(16000, 1, &samples(16000 * 42, 1))).await;
        let mut owner = PreparedSession::new();
        let task = owner.start(request(&source)).unwrap();
        let mut services = Services::new();
        services.changed = Some(source.control.clone());
        let completion = run(
            task,
            &services,
            PreparedSampleRate::Hz16000,
            TranslationQuality::Fast,
        )
        .await;
        let error = MediaError::Remote(RemoteMediaError::SourceChanged);
        assert_eq!(owner.apply(completion), Err(error));
        assert_eq!(owner.state(), PreparedState::Failed(error));
        assert_eq!(services.analysis.load(Ordering::SeqCst), 0);
        assert_eq!(services.translations.load(Ordering::SeqCst), 0);
        source.finish().await;
    });
}

#[path = "prepared_remote_lifecycle_tests.rs"]
mod lifecycle;

#[path = "prepared_remote_subtitle_tests.rs"]
mod subtitle;

#[path = "prepared_https_tests.rs"]
mod https;
