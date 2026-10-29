//! Real loopback media and explicit injected services; no real keys or cloud.
use super::*;
use crate::media::{
    embedded_mp4::Mp4SubtitleSelection,
    remote_http::RemoteMediaError,
    remote_wave::test_source::{Source, samples, wave},
};

/// Keep the real TCP source running while the controller's UI-style polling
/// deliberately stays synchronous. Production retains its original runtime.
fn with_remote_source(test: impl FnOnce(&Source) + Send + 'static) {
    let _serial = work::TEST_OWNERS.lock().unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let source = Source::new(wave(16_000, 1, &samples(SYNTHETIC_FRAMES, 1))).await;
        let source = tokio::task::spawn_blocking(move || {
            test(&source);
            source
        })
        .await
        .unwrap();
        source.finish().await;
    });
}
fn request(source: &Source) -> PreparedRequest {
    PreparedRequest::new(source.media_input(), vec![]).unwrap()
}
fn assert_no_provider_work(factory: &Factory) {
    assert_eq!(factory.stats.speech_calls.load(Ordering::SeqCst), 0);
    assert_eq!(factory.stats.analysis_calls.load(Ordering::SeqCst), 0);
    assert_eq!(factory.stats.translation_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn remote_controller_selected_providers_and_qualities_reach_ram_ready_with_one_key() {
    with_remote_source(|source| {
        for provider in [SpeechProvider::OpenAi, SpeechProvider::Gemini] {
            for quality in [
                TranslationQuality::Fast,
                TranslationQuality::Standard,
                TranslationQuality::High,
            ] {
                let factory = Factory::new(SpeechMode::Success, TextMode::Success);
                factory
                    .stats
                    .forbid_file_metadata
                    .store(true, Ordering::SeqCst);
                let mut controller = PreparedController::with_factory(factory.clone());
                let mut selected = options(Some(provider), SourceLanguage::Japanese);
                selected.quality = quality;
                controller.start(request(source), selected, key()).unwrap();
                assert_eq!(
                    terminal(&mut controller),
                    PreparedUiState::Ready { cues: 1 }
                );
                let ready = controller.ready().unwrap();
                assert_eq!(ready.media_byte_len(), 44 + SYNTHETIC_FRAMES as u64 * 2);
                assert_eq!(ready.quality(), Some(quality));
                let original = ready.original().recognized().unwrap();
                assert_eq!(original.identity(), ready.identity());
                assert_eq!(original.language(), SpeechLanguage::Japanese);
                assert_eq!(original.cues()[0].start_us, 4_000);
                assert_eq!(original.cues()[0].duration_us, 12_000);
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
                    &[(SourceLanguage::Japanese, PreparedSampleRate::Hz16000)]
                );
                assert_eq!(factory.stats.analysis_calls.load(Ordering::SeqCst), 1);
                let expected_passes: &[TranslationPass] = if quality == TranslationQuality::Fast {
                    &[TranslationPass::Draft]
                } else {
                    &[TranslationPass::Draft, TranslationPass::Review]
                };
                assert_eq!(
                    factory.stats.passes.lock().unwrap().as_slice(),
                    expected_passes
                );
                let requests = source.rows.lock().unwrap().len();
                assert!(requests > 0);
                assert_eq!(controller.poll(), PreparedUiState::Ready { cues: 1 });
                assert_eq!(source.rows.lock().unwrap().len(), requests);
                controller.stop();
                assert!(controller.ready().is_none());
                assert_eq!(controller.state(), PreparedUiState::Cancelled);
                assert_eq!(factory.stats.speech_dropped.load(Ordering::SeqCst), 1);
                assert_eq!(factory.stats.translation_dropped.load(Ordering::SeqCst), 1);
            }
        }
    });
}

#[test]
fn remote_controller_any_embedded_choice_rejects_before_factory_credentials_or_network() {
    with_remote_source(|source| {
        let fixture = Fixture::new();
        let factory = Factory::new(SpeechMode::Success, TextMode::Success);
        let mut controller = PreparedController::with_factory(factory.clone());
        for language in [
            TextLanguage::TaiwanTraditional,
            TextLanguage::SimplifiedChinese,
            TextLanguage::Source(SpeechLanguage::English),
        ] {
            let embedded = request(source)
                .with_embedded_subtitle(Mp4SubtitleSelection::new(3, language).unwrap())
                .unwrap();
            assert_eq!(
                controller.start(embedded, options(None, SourceLanguage::Auto), None),
                Err(PreparedUiError::Media(MediaError::UnsupportedUrl))
            );
            assert!(controller.worker.is_none());
            assert!(controller.session.identity().is_none());
            assert!(controller.ready().is_none());
        }
        // A higher-priority Chinese sidecar cannot hide an unsupported embedded
        // choice: reject the complete request before reading the sidecar/key.
        let preferred_sidecar = LocalSubtitleInput::new(
            fixture.directory.path().join("missing-private.srt"),
            SubtitleFormat::Srt,
            TextLanguage::TaiwanTraditional,
        )
        .unwrap();
        let mixed = PreparedRequest::new(source.media_input(), vec![preferred_sidecar])
            .unwrap()
            .with_embedded_subtitle(
                Mp4SubtitleSelection::new(3, TextLanguage::Source(SpeechLanguage::English))
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(
            controller.start(mixed, options(None, SourceLanguage::Auto), key()),
            Err(PreparedUiError::Media(MediaError::UnsupportedUrl))
        );
        assert!(controller.worker.is_none());
        assert!(controller.session.identity().is_none());
        assert_eq!(factory.stats.builds.load(Ordering::SeqCst), 0);
        assert_eq!(factory.stats.keys.load(Ordering::SeqCst), 0);
        assert_no_provider_work(&factory);
        assert!(source.rows.lock().unwrap().is_empty());
    });
}

#[path = "prepared_remote_subtitle_tests.rs"]
mod subtitle;

#[test]
fn remote_controller_ambiguous_text_keeps_priority_before_url_or_service_errors() {
    with_remote_source(|source| {
        let fixture = Fixture::new();
        let factory = Factory::new(SpeechMode::Success, TextMode::Success);
        let mut controller = PreparedController::with_factory(factory.clone());
        let choices = ["first.srt", "second.srt"]
            .into_iter()
            .map(|name| {
                LocalSubtitleInput::new(
                    fixture.directory.path().join(name),
                    SubtitleFormat::Srt,
                    TextLanguage::TaiwanTraditional,
                )
                .unwrap()
            })
            .collect();
        assert_eq!(
            controller.start(
                PreparedRequest::new(source.media_input(), choices).unwrap(),
                options(None, SourceLanguage::Auto),
                None
            ),
            Err(PreparedUiError::Media(MediaError::AmbiguousSubtitle))
        );
        assert!(controller.worker.is_none());
        assert!(controller.session.identity().is_none());
        assert_eq!(factory.stats.builds.load(Ordering::SeqCst), 0);
        assert!(source.rows.lock().unwrap().is_empty());
    });
}

#[test]
fn remote_controller_language_provider_and_capability_preflight_precede_http() {
    with_remote_source(|source| {
        let factory = Factory::new(SpeechMode::Unsupported, TextMode::Success);
        let mut controller = PreparedController::with_factory(factory.clone());
        assert_eq!(
            controller.start(
                request(source),
                options(Some(SpeechProvider::OpenAi), SourceLanguage::Auto),
                key()
            ),
            Err(PreparedUiError::Speech(
                PreparedSpeechError::UnsupportedLanguage
            ))
        );
        assert_eq!(
            controller.start(
                request(source),
                options(None, SourceLanguage::English),
                None
            ),
            Err(PreparedUiError::ProviderRequired)
        );
        assert!(controller.worker.is_none());
        assert_eq!(factory.stats.builds.load(Ordering::SeqCst), 0);
        controller
            .start(
                request(source),
                options(Some(SpeechProvider::Gemini), SourceLanguage::English),
                key(),
            )
            .unwrap();
        assert_eq!(
            terminal(&mut controller),
            PreparedUiState::Failed(PreparedUiError::Speech(
                PreparedSpeechError::UnsupportedCapability
            ))
        );
        assert_eq!(factory.stats.builds.load(Ordering::SeqCst), 1);
        assert_no_provider_work(&factory);
        assert!(source.rows.lock().unwrap().is_empty());
        assert!(controller.ready().is_none());
    });
}

#[test]
fn remote_controller_transport_error_keeps_typed_cause_without_cloud_work() {
    with_remote_source(|source| {
        source.control.status.store(429, Ordering::Release);
        let factory = Factory::new(SpeechMode::Success, TextMode::Success);
        let mut controller = PreparedController::with_factory(factory.clone());
        controller
            .start(
                request(source),
                options(Some(SpeechProvider::OpenAi), SourceLanguage::French),
                key(),
            )
            .unwrap();
        let failed = PreparedUiState::Failed(PreparedUiError::Media(MediaError::Remote(
            RemoteMediaError::RateLimited,
        )));
        assert_eq!(terminal(&mut controller), failed);
        assert_eq!(factory.stats.builds.load(Ordering::SeqCst), 1);
        assert_eq!(source.rows.lock().unwrap().len(), 1);
        assert_no_provider_work(&factory);
        assert!(controller.ready().is_none());
        assert!(!failed.label().contains("SYNTHETIC"));
        assert!(!failed.label().contains("127.0.0.1"));
        assert!(!failed.label().contains("private="));
    });
}

#[test]
fn remote_controller_stop_during_http_body_reaps_without_recognition() {
    with_remote_source(|source| {
        let (mut entered, release) = source.pause();
        let factory = Factory::new(SpeechMode::Success, TextMode::Success);
        let mut controller = PreparedController::with_factory(factory.clone());
        controller
            .start(
                request(source),
                options(Some(SpeechProvider::Gemini), SourceLanguage::English),
                key(),
            )
            .unwrap();
        let deadline = Instant::now() + TEST_TIMEOUT;
        loop {
            match entered.try_recv() {
                Ok(()) => break,
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
                Err(error) => panic!("synthetic range did not enter: {error}"),
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        controller.stop();
        assert_eq!(terminal(&mut controller), PreparedUiState::Cancelled);
        drop(release);
        assert_eq!(source.rows.lock().unwrap().len(), 1);
        assert_no_provider_work(&factory);
        assert!(controller.ready().is_none());
        assert!(controller.worker.is_none());
    });
}

#[test]
fn remote_controller_noncooperative_asr_cannot_publish_after_stop() {
    with_remote_source(|source| {
        let (started, received) = mpsc::sync_channel(1);
        let release = Arc::new(Notify::new());
        let factory = Factory::new(
            SpeechMode::Late(Arc::new(Mutex::new(Some(started))), release.clone()),
            TextMode::Success,
        );
        let mut controller = PreparedController::with_factory(factory.clone());
        controller
            .start(
                request(source),
                options(Some(SpeechProvider::Gemini), SourceLanguage::German),
                key(),
            )
            .unwrap();
        let cancel = received.recv_timeout(TEST_TIMEOUT).unwrap();
        controller.stop();
        assert_eq!(controller.state(), PreparedUiState::Stopping);
        assert_eq!(
            controller.start(
                request(source),
                options(Some(SpeechProvider::OpenAi), SourceLanguage::English),
                key()
            ),
            Err(PreparedUiError::Busy)
        );
        release.notify_one();
        assert_eq!(terminal(&mut controller), PreparedUiState::Cancelled);
        assert!(cancel.is_cancelled());
        assert!(controller.ready().is_none());
        assert_eq!(factory.stats.analysis_calls.load(Ordering::SeqCst), 0);
        assert_eq!(factory.stats.translation_calls.load(Ordering::SeqCst), 0);
        assert_eq!(factory.stats.speech_dropped.load(Ordering::SeqCst), 1);
        assert_eq!(factory.stats.translation_dropped.load(Ordering::SeqCst), 1);
    });
}

#[test]
fn remote_controller_buffered_completion_is_retired_before_restart_ready() {
    with_remote_source(|source| {
        let factory = Factory::new(SpeechMode::Success, TextMode::Success);
        let mut controller = PreparedController::with_factory(factory.clone());
        controller
            .start(
                request(source),
                options(Some(SpeechProvider::OpenAi), SourceLanguage::English),
                key(),
            )
            .unwrap();
        let old = controller.session.identity().unwrap();
        finished(&controller);
        assert!(controller.ready().is_none());
        controller.stop();
        assert_eq!(terminal(&mut controller), PreparedUiState::Cancelled);
        controller
            .start(
                request(source),
                options(Some(SpeechProvider::Gemini), SourceLanguage::Korean),
                key(),
            )
            .unwrap();
        assert_eq!(
            terminal(&mut controller),
            PreparedUiState::Ready { cues: 1 }
        );
        let current = controller.ready().unwrap();
        assert_ne!(current.identity().session, old.session);
        assert_eq!(
            current.original().recognized().unwrap().language(),
            SpeechLanguage::Korean
        );
        assert!(
            factory.stats.selected.lock().unwrap().as_slice()
                == [
                    (SpeechProvider::OpenAi, PreparedServiceKind::Recognition),
                    (SpeechProvider::Gemini, PreparedServiceKind::Recognition),
                ]
        );
        assert_eq!(factory.stats.keys.load(Ordering::SeqCst), 2);
    });
}

#[test]
fn remote_controller_stop_during_translation_cancels_and_releases_owned_work() {
    with_remote_source(|source| {
        let (started, received) = mpsc::sync_channel(1);
        let factory = Factory::new(
            SpeechMode::Success,
            TextMode::Pending(Arc::new(Mutex::new(Some(started)))),
        );
        let mut controller = PreparedController::with_factory(factory.clone());
        controller
            .start(
                request(source),
                options(Some(SpeechProvider::OpenAi), SourceLanguage::English),
                key(),
            )
            .unwrap();
        let cancel = received.recv_timeout(TEST_TIMEOUT).unwrap();
        let requests = source.rows.lock().unwrap().len();
        assert!(controller.ready().is_none());
        controller.stop();
        assert_eq!(terminal(&mut controller), PreparedUiState::Cancelled);
        assert_eq!(cancel.reason(), Some(TranslationError::Cancelled));
        assert_eq!(source.rows.lock().unwrap().len(), requests);
        assert_eq!(factory.stats.speech_dropped.load(Ordering::SeqCst), 1);
        assert_eq!(factory.stats.translation_dropped.load(Ordering::SeqCst), 1);
        assert!(controller.worker.is_none());
        assert!(controller.ready().is_none());
    });
}
