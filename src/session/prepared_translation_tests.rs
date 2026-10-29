//! Synthetic sidecars and explicit mock providers; no credentials or network.
use super::*;
use crate::{
    ai::{
        speech::SpeechLanguage,
        translation::{
            TranslationCancellation, TranslationCapabilities, TranslationEmitter,
            TranslationFuture, TranslationPass, TranslationRequest,
        },
    },
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
};

struct Fixture {
    directory: tempfile::TempDir,
    media: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let media = directory.path().join("synthetic.media");
        fs::write(&media, b"synthetic opaque bytes, not captured media").unwrap();
        Self { directory, media }
    }
    fn subtitle(&self, name: &str, text: &str, language: TextLanguage) -> LocalSubtitleInput {
        let path = self.directory.path().join(name);
        fs::write(&path, text).unwrap();
        LocalSubtitleInput::new(path, SubtitleFormat::Srt, language).unwrap()
    }
    fn request(&self, language: TextLanguage) -> PreparedRequest {
        self.with_subtitles(vec![self.subtitle(
            "explicit.srt",
            "1\n01:02:03,004 --> 01:02:05,006\nsource one\n\n2\n01:02:06,008 --> 01:02:08,010\nsource two\n",
            language,
        )])
    }
    fn with_subtitles(&self, subtitles: Vec<LocalSubtitleInput>) -> PreparedRequest {
        PreparedRequest::new(MediaInput::local(self.media.clone()).unwrap(), subtitles).unwrap()
    }
}
fn owner() -> PreparedSession {
    let mut session = PreparedSession::new();
    session.workers = Arc::new(Semaphore::new(1));
    session
}
fn capabilities() -> TranslationCapabilities {
    TranslationCapabilities {
        streaming: true,
        cancellation: true,
        supported_languages: [true; 5],
        max_concurrent: 2,
    }
}
struct MockTranslator {
    calls: AtomicUsize,
    error: Option<TranslationError>,
}
impl MockTranslator {
    fn success() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            error: None,
        }
    }
}
impl Translator for MockTranslator {
    fn capabilities(&self) -> TranslationCapabilities {
        capabilities()
    }
    fn translate<'a>(
        &'a self,
        request: &'a TranslationRequest,
        _: &'a mut TranslationEmitter,
        _: TranslationCancellation,
    ) -> TranslationFuture<'a> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            if let Some(error) = self.error {
                return Err(error);
            }
            assert_eq!(request.source.identity.generation, 1);
            assert!(request.prepared_context.is_some());
            Ok(format!(
                "软件与服务器 {} {}",
                request.source.sequence,
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
struct PendingTranslator {
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
    token: Mutex<Option<TranslationCancellation>>,
}
impl PendingTranslator {
    fn new() -> Self {
        Self {
            entered: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
            token: Mutex::new(None),
        }
    }
    fn token(&self) -> TranslationCancellation {
        self.token.lock().unwrap().as_ref().unwrap().clone()
    }
}
impl Translator for PendingTranslator {
    fn capabilities(&self) -> TranslationCapabilities {
        TranslationCapabilities {
            max_concurrent: 1,
            ..capabilities()
        }
    }
    fn translate<'a>(
        &'a self,
        _: &'a TranslationRequest,
        _: &'a mut TranslationEmitter,
        cancel: TranslationCancellation,
    ) -> TranslationFuture<'a> {
        Box::pin(async move {
            *self.token.lock().unwrap() = Some(cancel);
            self.entered.notify_one();
            // Deliberately noncooperative: the owner must drop this future.
            self.release.notified().await;
            Ok("軟體".into())
        })
    }
}

#[tokio::test]
async fn all_five_source_languages_translate_preserve_times_and_localize_before_ready() {
    let fixture = Fixture::new();
    for language in SpeechLanguage::ALL {
        let mut owner = owner();
        let provider = MockTranslator::success();
        let task = owner
            .start(fixture.request(TextLanguage::Source(language)))
            .unwrap();
        owner
            .apply(
                task.run_with_translator(&provider, TranslationQuality::Standard, None)
                    .await,
            )
            .unwrap();
        let document = owner.ready().unwrap();
        assert_eq!(document.cue_count(), 2);
        assert_eq!(document.translated().len(), 2);
        assert_eq!(
            document.original().existing().unwrap().cues[0]
                .text
                .as_ref(),
            "source one"
        );
        assert_eq!(
            document.original().existing().unwrap().cues[0].start_us,
            Some(3_723_004_000)
        );
        assert_eq!(
            document.original().existing().unwrap().cues[1].duration_us,
            Some(2_002_000)
        );
        assert!(document.translated()[0].contains("軟體與伺服器 0 校訂"));
        assert!(document.translated()[1].contains("軟體與伺服器 1 校訂"));
        assert_eq!(provider.calls.load(Ordering::SeqCst), 4);
        assert_eq!(owner.app_state(), AppState::PreparedReady);
        owner.stop();
        assert!(owner.ready().is_none());
    }
    assert_eq!(fs::read_dir(fixture.directory.path()).unwrap().count(), 2);
}

#[tokio::test]
async fn native_chinese_priority_does_not_touch_unnecessary_provider_or_glossary() {
    struct Forbidden;
    impl Translator for Forbidden {
        fn capabilities(&self) -> TranslationCapabilities {
            panic!("Chinese must not need provider")
        }
        fn translate<'a>(
            &'a self,
            _: &'a TranslationRequest,
            _: &'a mut TranslationEmitter,
            _: TranslationCancellation,
        ) -> TranslationFuture<'a> {
            panic!("no cloud for Chinese")
        }
    }
    let fixture = Fixture::new();
    let request = fixture.with_subtitles(vec![
        fixture.subtitle(
            "source.srt",
            "invalid source",
            TextLanguage::Source(SpeechLanguage::English),
        ),
        fixture.subtitle(
            "zh.srt",
            "1\n00:00:01,000 --> 00:00:02,000\n軟件與服務器\n",
            TextLanguage::TaiwanTraditional,
        ),
    ]);
    let old_glossary = SessionGlossary::new(StreamIdentity {
        session: SessionId(1),
        generation: 1,
    });
    let mut owner = owner();
    let task = owner.start(request).unwrap();
    owner
        .apply(
            task.run_with_translator(&Forbidden, TranslationQuality::High, Some(&old_glossary))
                .await,
        )
        .unwrap();
    assert_eq!(
        owner.ready().unwrap().translated()[0].as_ref(),
        "軟體與伺服器"
    );
}

#[tokio::test]
async fn wrong_session_glossary_is_rejected_before_provider_requests() {
    let fixture = Fixture::new();
    let mut owner = owner();
    let task = owner
        .start(fixture.request(TextLanguage::Source(SpeechLanguage::English)))
        .unwrap();
    let glossary = SessionGlossary::new(StreamIdentity {
        session: SessionId(0),
        generation: 1,
    });
    let provider = MockTranslator::success();
    let completion = task
        .run_with_translator(&provider, TranslationQuality::High, Some(&glossary))
        .await;
    assert_eq!(
        owner.apply(completion).err(),
        Some(MediaError::Translation(TranslationError::InvalidData))
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    assert!(owner.ready().is_none());
}

#[tokio::test]
async fn typed_provider_failure_has_no_partial_ready_and_next_run_recovers() {
    let fixture = Fixture::new();
    let mut owner = owner();
    let provider = MockTranslator {
        calls: AtomicUsize::new(0),
        error: Some(TranslationError::Credential),
    };
    let task = owner
        .start(fixture.request(TextLanguage::Source(SpeechLanguage::English)))
        .unwrap();
    let completion = task
        .run_with_translator(&provider, TranslationQuality::Fast, None)
        .await;
    assert_eq!(
        owner.apply(completion).err(),
        Some(MediaError::Translation(TranslationError::Credential))
    );
    assert_eq!(owner.app_state(), AppState::Error);
    assert!(owner.ready().is_none());
    let provider = MockTranslator::success();
    let task = owner
        .start(fixture.request(TextLanguage::Source(SpeechLanguage::Japanese)))
        .unwrap();
    owner
        .apply(
            task.run_with_translator(&provider, TranslationQuality::Fast, None)
                .await,
        )
        .unwrap();
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    assert!(owner.ready().unwrap().translated()[0].contains("草稿"));
}

#[tokio::test]
async fn dropping_pending_translation_cancels_provider_and_releases_media_and_cpu_lane() {
    let fixture = Fixture::new();
    let mut owner = owner();
    let provider = PendingTranslator::new();
    let task = owner
        .start(fixture.request(TextLanguage::Source(SpeechLanguage::English)))
        .unwrap();
    let mut future = Box::pin(task.run_with_translator(&provider, TranslationQuality::Fast, None));
    tokio::select! { biased;
        _ = provider.entered.notified() => (),
        _ = &mut future => panic!("pending fixture completed"),
    }
    assert_eq!(owner.workers.available_permits(), 1);
    #[cfg(windows)]
    assert!(
        fs::OpenOptions::new()
            .write(true)
            .open(&fixture.media)
            .is_err()
    );
    let token = provider.token();
    drop(future);
    assert_eq!(token.reason(), Some(TranslationError::Cancelled));
    assert_eq!(owner.state(), PreparedState::Cancelled);
    assert!(owner.ready().is_none());
    assert!(
        fs::OpenOptions::new()
            .write(true)
            .open(&fixture.media)
            .is_ok()
    );
}

#[tokio::test]
async fn replacing_pending_source_work_cancels_it_and_keeps_new_chinese_ready() {
    let fixture = Fixture::new();
    let mut owner = owner();
    let provider = PendingTranslator::new();
    let task = owner
        .start(fixture.request(TextLanguage::Source(SpeechLanguage::English)))
        .unwrap();
    let mut future = Box::pin(task.run_with_translator(&provider, TranslationQuality::High, None));
    tokio::select! { biased;
        _ = provider.entered.notified() => (),
        _ = &mut future => panic!("pending fixture completed"),
    }
    let current = owner
        .start(fixture.request(TextLanguage::TaiwanTraditional))
        .unwrap();
    let old = future.await;
    assert_eq!(old.result.err(), Some(MediaError::Cancelled));
    assert_eq!(provider.token().reason(), Some(TranslationError::Cancelled));
    owner.apply(current.run().await).unwrap();
    assert_eq!(owner.state(), PreparedState::Ready);
}

#[tokio::test]
async fn late_completed_source_document_cannot_replace_new_ready_document() {
    let fixture = Fixture::new();
    let mut owner = owner();
    let provider = MockTranslator::success();
    let task = owner
        .start(fixture.request(TextLanguage::Source(SpeechLanguage::French)))
        .unwrap();
    let old = task
        .run_with_translator(&provider, TranslationQuality::High, None)
        .await;
    let current = owner
        .start(fixture.request(TextLanguage::TaiwanTraditional))
        .unwrap();
    owner.apply(current.run().await).unwrap();
    let identity = owner.ready().unwrap().identity();
    assert_eq!(owner.apply(old).err(), Some(MediaError::Stale));
    assert_eq!(owner.ready().unwrap().identity(), identity);
}

#[tokio::test]
async fn unpolled_provider_future_drop_cancels_without_loading_inputs() {
    let fixture = Fixture::new();
    let mut owner = owner();
    let provider = MockTranslator::success();
    let task = owner
        .start(fixture.request(TextLanguage::Source(SpeechLanguage::Korean)))
        .unwrap();
    let future = task.run_with_translator(&provider, TranslationQuality::Standard, None);
    drop(future);
    assert_eq!(owner.state(), PreparedState::Cancelled);
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn final_merge_rejects_wrong_cardinality_controls_and_invalid_localized_size() {
    let fixture = Fixture::new();
    let identity = StreamIdentity {
        session: SessionId(200),
        generation: 1,
    };
    let cancel = ResolverCancellation::default();
    for translated in [
        vec![],
        vec!["x".into(), "\0bad".into()],
        vec!["x".into(), "x".repeat(MAX_CUE_BYTES + 1).into()],
    ] {
        let loaded = load(
            fixture.request(TextLanguage::Source(SpeechLanguage::German)),
            identity,
            1,
            &cancel,
            true,
        )
        .unwrap();
        assert_eq!(
            finish(loaded, translated, identity, 1, &cancel).err(),
            Some(MediaError::InvalidSubtitle)
        );
    }
    // Taiwan phrase conversion can expand an otherwise valid provider result.
    let loaded = load(
        fixture.request(TextLanguage::Source(SpeechLanguage::German)),
        identity,
        1,
        &cancel,
        true,
    )
    .unwrap();
    let growing = "内存".repeat(MAX_CUE_BYTES / "内存".len());
    assert!(growing.len() <= MAX_CUE_BYTES);
    assert_eq!(
        finish(
            loaded,
            vec![growing.into(), "x".into()],
            identity,
            1,
            &cancel
        )
        .err(),
        Some(MediaError::InvalidSubtitle)
    );
}
