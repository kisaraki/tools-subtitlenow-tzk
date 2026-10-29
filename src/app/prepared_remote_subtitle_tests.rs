//! Explicit local sidecars with actual loopback PCM16 WAVE and injected AI only.
use super::*;

fn sidecar(
    fixture: &Fixture,
    format: SubtitleFormat,
    language: TextLanguage,
) -> LocalSubtitleInput {
    let (name, text) = match format {
        SubtitleFormat::Srt => (
            "private-selected.srt",
            "1\n00:00:00,004 --> 00:00:00,016\n软件与服务器\n",
        ),
        SubtitleFormat::WebVtt => (
            "private-selected.vtt",
            "WEBVTT\n\n00:00:00.004 --> 00:00:00.016\n软件与服务器\n",
        ),
    };
    let path = fixture.directory.path().join(name);
    fs::write(&path, text).unwrap();
    LocalSubtitleInput::new(path, format, language).unwrap()
}
fn selected_request(
    source: &Source,
    fixture: &Fixture,
    format: SubtitleFormat,
    language: TextLanguage,
) -> PreparedRequest {
    PreparedRequest::new(
        source.media_input(),
        vec![sidecar(fixture, format, language)],
    )
    .unwrap()
}
fn assert_existing_ready(controller: &mut PreparedController, language: TextLanguage) {
    assert_eq!(terminal(controller), PreparedUiState::Ready { cues: 1 });
    let ready = controller.ready().unwrap();
    assert_eq!(ready.media_byte_len(), 44 + SYNTHETIC_FRAMES as u64 * 2);
    assert_eq!(ready.target_language(), "zh-TW");
    assert!(ready.original().recognized().is_none());
    let original = ready.original().existing().unwrap();
    assert_eq!(original.identity, ready.identity());
    assert!(original.language == language);
    assert_eq!(original.cues[0].start_us, Some(4_000));
    assert_eq!(original.cues[0].duration_us, Some(12_000));
    assert!(ready.translated()[0].contains("軟體"));
    assert!(ready.translated()[0].contains("伺服器"));
}

#[test]
fn remote_sidecar_chinese_srt_and_vtt_skip_factory_keys_and_all_ai_work() {
    with_remote_source(|source| {
        let fixture = Fixture::new();
        for language in [
            TextLanguage::TaiwanTraditional,
            TextLanguage::OtherTraditional,
            TextLanguage::SimplifiedChinese,
        ] {
            for format in [SubtitleFormat::Srt, SubtitleFormat::WebVtt] {
                let factory = Factory::new(SpeechMode::Unsupported, TextMode::AnalysisFailure);
                let mut controller = PreparedController::with_factory(factory.clone());
                // Chinese ignores the service preference and passed key alike;
                // absence of all factory work also precludes secure-store I/O.
                controller
                    .start(
                        selected_request(source, &fixture, format, language),
                        options(None, SourceLanguage::Auto),
                        key(),
                    )
                    .unwrap();
                assert_existing_ready(&mut controller, language);
                assert_eq!(factory.stats.builds.load(Ordering::SeqCst), 0);
                assert_eq!(factory.stats.keys.load(Ordering::SeqCst), 0);
                assert_eq!(factory.stats.speech_capabilities.load(Ordering::SeqCst), 0);
                assert_no_provider_work(&factory);
                let requests = source.rows.lock().unwrap().len();
                assert!(requests > 0);
                assert_eq!(controller.poll(), PreparedUiState::Ready { cues: 1 });
                assert_eq!(source.rows.lock().unwrap().len(), requests);
                controller.stop();
                assert!(controller.ready().is_none());
            }
        }
    });
}

#[test]
fn remote_sidecar_two_services_three_qualities_use_translation_only_and_one_key() {
    with_remote_source(|source| {
        let fixture = Fixture::new();
        for provider in [SpeechProvider::OpenAi, SpeechProvider::Gemini] {
            for quality in [
                TranslationQuality::Fast,
                TranslationQuality::Standard,
                TranslationQuality::High,
            ] {
                for format in [SubtitleFormat::Srt, SubtitleFormat::WebVtt] {
                    let factory = Factory::new(SpeechMode::Unsupported, TextMode::Success);
                    factory
                        .stats
                        .forbid_file_metadata
                        .store(true, Ordering::SeqCst);
                    let mut controller = PreparedController::with_factory(factory.clone());
                    let mut selected = options(Some(provider), SourceLanguage::Auto);
                    selected.quality = quality;
                    let language = TextLanguage::Source(SpeechLanguage::Japanese);
                    controller
                        .start(
                            selected_request(source, &fixture, format, language),
                            selected,
                            key(),
                        )
                        .unwrap();
                    assert_existing_ready(&mut controller, language);
                    assert_eq!(factory.stats.builds.load(Ordering::SeqCst), 1);
                    assert_eq!(factory.stats.keys.load(Ordering::SeqCst), 1);
                    assert!(
                        factory.stats.selected.lock().unwrap().as_slice()
                            == [(provider, PreparedServiceKind::Translation)]
                    );
                    assert_eq!(factory.stats.speech_capabilities.load(Ordering::SeqCst), 0);
                    assert_eq!(factory.stats.speech_calls.load(Ordering::SeqCst), 0);
                    assert_eq!(factory.stats.speech_dropped.load(Ordering::SeqCst), 0);
                    assert_eq!(factory.stats.analysis_calls.load(Ordering::SeqCst), 1);
                    let passes: &[TranslationPass] = if quality == TranslationQuality::Fast {
                        &[TranslationPass::Draft]
                    } else {
                        &[TranslationPass::Draft, TranslationPass::Review]
                    };
                    assert_eq!(factory.stats.passes.lock().unwrap().as_slice(), passes);
                    assert_eq!(
                        factory.stats.translation_calls.load(Ordering::SeqCst),
                        passes.len()
                    );
                    assert_eq!(factory.stats.translation_dropped.load(Ordering::SeqCst), 1);
                    controller.stop();
                    assert!(controller.ready().is_none());
                }
            }
        }
    });
}

#[test]
fn remote_sidecar_missing_provider_and_ambiguity_fail_before_factory_or_io() {
    with_remote_source(|source| {
        let fixture = Fixture::new();
        let factory = Factory::new(SpeechMode::Success, TextMode::Success);
        let mut controller = PreparedController::with_factory(factory.clone());
        let language = TextLanguage::Source(SpeechLanguage::English);
        let missing = || {
            LocalSubtitleInput::new(
                fixture.directory.path().join("missing-sidecar.srt"),
                SubtitleFormat::Srt,
                language,
            )
            .unwrap()
        };
        assert_eq!(
            controller.start(
                PreparedRequest::new(source.media_input(), vec![missing()]).unwrap(),
                options(None, SourceLanguage::Auto),
                key(),
            ),
            Err(PreparedUiError::ProviderRequired)
        );
        assert!(controller.worker.is_none());
        assert!(controller.session.identity().is_none());
        for embedded in [false, true] {
            let request = if embedded {
                PreparedRequest::new(source.media_input(), vec![missing()])
                    .unwrap()
                    .with_embedded_subtitle(Mp4SubtitleSelection::new(3, language).unwrap())
                    .unwrap()
            } else {
                PreparedRequest::new(source.media_input(), vec![missing(), missing()]).unwrap()
            };
            assert_eq!(
                controller.start(request, options(None, SourceLanguage::Auto), key()),
                Err(PreparedUiError::Media(MediaError::AmbiguousSubtitle))
            );
            assert!(controller.worker.is_none());
            assert!(controller.session.identity().is_none());
        }
        assert_eq!(factory.stats.builds.load(Ordering::SeqCst), 0);
        assert_no_provider_work(&factory);
        assert!(source.rows.lock().unwrap().is_empty());
    });
}

#[test]
fn remote_sidecar_corrupt_or_missing_text_never_falls_back_to_recognition() {
    with_remote_source(|source| {
        let fixture = Fixture::new();
        for format in [SubtitleFormat::Srt, SubtitleFormat::WebVtt] {
            for missing in [false, true] {
                let factory = Factory::new(SpeechMode::Success, TextMode::Success);
                let mut controller = PreparedController::with_factory(factory.clone());
                let path = fixture.directory.path().join("invalid-private-sidecar");
                if !missing {
                    fs::write(&path, b"invalid synthetic document").unwrap();
                } else {
                    let _ = fs::remove_file(&path);
                }
                let subtitle = LocalSubtitleInput::new(
                    path,
                    format,
                    TextLanguage::Source(SpeechLanguage::English),
                )
                .unwrap();
                controller
                    .start(
                        PreparedRequest::new(source.media_input(), vec![subtitle]).unwrap(),
                        options(Some(SpeechProvider::OpenAi), SourceLanguage::Auto),
                        key(),
                    )
                    .unwrap();
                let expected = if missing {
                    MediaError::Unavailable
                } else {
                    MediaError::InvalidSubtitle
                };
                assert_eq!(
                    terminal(&mut controller),
                    PreparedUiState::Failed(PreparedUiError::Media(expected))
                );
                assert!(controller.ready().is_none());
                assert_eq!(factory.stats.builds.load(Ordering::SeqCst), 1);
                assert!(
                    factory.stats.selected.lock().unwrap().as_slice()
                        == [(SpeechProvider::OpenAi, PreparedServiceKind::Translation)]
                );
                assert_no_provider_work(&factory);
                assert_eq!(factory.stats.speech_capabilities.load(Ordering::SeqCst), 0);
                assert!(source.rows.lock().unwrap().is_empty());
            }
        }
    });
}

#[test]
fn remote_sidecar_http_and_translation_errors_remain_typed_without_asr() {
    with_remote_source(|source| {
        let fixture = Fixture::new();
        let language = TextLanguage::Source(SpeechLanguage::English);
        for remote_error in [true, false] {
            source
                .control
                .status
                .store(if remote_error { 429 } else { 0 }, Ordering::Release);
            let factory = Factory::new(
                SpeechMode::Success,
                TextMode::Fail(TranslationError::Refused),
            );
            let mut controller = PreparedController::with_factory(factory.clone());
            controller
                .start(
                    selected_request(source, &fixture, SubtitleFormat::Srt, language),
                    options(Some(SpeechProvider::Gemini), SourceLanguage::Auto),
                    key(),
                )
                .unwrap();
            let expected = if remote_error {
                PreparedUiError::Media(MediaError::Remote(RemoteMediaError::RateLimited))
            } else {
                PreparedUiError::Translation(TranslationError::Refused)
            };
            let state = PreparedUiState::Failed(expected);
            assert_eq!(terminal(&mut controller), state);
            assert!(controller.ready().is_none());
            assert_eq!(factory.stats.speech_calls.load(Ordering::SeqCst), 0);
            assert_eq!(factory.stats.speech_capabilities.load(Ordering::SeqCst), 0);
            assert_eq!(
                factory.stats.translation_calls.load(Ordering::SeqCst),
                usize::from(!remote_error)
            );
            for secret in ["127.0.0.1", "private=", "SYNTHETIC", "private-selected"] {
                assert!(!state.label().contains(secret));
            }
        }
    });
}

#[test]
fn remote_sidecar_stop_during_body_reaps_without_ai_and_drops_partial_document() {
    with_remote_source(|source| {
        let fixture = Fixture::new();
        let (mut entered, release) = source.pause();
        let factory = Factory::new(SpeechMode::Success, TextMode::Success);
        let mut controller = PreparedController::with_factory(factory.clone());
        controller
            .start(
                selected_request(
                    source,
                    &fixture,
                    SubtitleFormat::Srt,
                    TextLanguage::Source(SpeechLanguage::English),
                ),
                options(Some(SpeechProvider::OpenAi), SourceLanguage::Auto),
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
        assert_no_provider_work(&factory);
        assert!(controller.worker.is_none());
        assert!(controller.ready().is_none());
    });
}

#[test]
fn remote_sidecar_stop_during_translation_releases_work_and_allows_new_session() {
    with_remote_source(|source| {
        let fixture = Fixture::new();
        let (started, received) = mpsc::sync_channel(1);
        let factory = Factory::new(
            SpeechMode::Success,
            TextMode::Pending(Arc::new(Mutex::new(Some(started)))),
        );
        let mut controller = PreparedController::with_factory(factory.clone());
        controller
            .start(
                selected_request(
                    source,
                    &fixture,
                    SubtitleFormat::WebVtt,
                    TextLanguage::Source(SpeechLanguage::Korean),
                ),
                options(Some(SpeechProvider::Gemini), SourceLanguage::Auto),
                key(),
            )
            .unwrap();
        let old = controller.session.identity().unwrap();
        let cancel = received.recv_timeout(TEST_TIMEOUT).unwrap();
        controller.stop();
        assert_eq!(controller.state(), PreparedUiState::Stopping);
        assert_eq!(
            controller.start(
                selected_request(
                    source,
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
        assert_eq!(cancel.reason(), Some(TranslationError::Cancelled));
        assert_eq!(factory.stats.speech_calls.load(Ordering::SeqCst), 0);
        assert_eq!(factory.stats.translation_dropped.load(Ordering::SeqCst), 1);
        assert!(controller.ready().is_none());
        controller
            .start(
                selected_request(
                    source,
                    &fixture,
                    SubtitleFormat::Srt,
                    TextLanguage::SimplifiedChinese,
                ),
                options(None, SourceLanguage::Auto),
                None,
            )
            .unwrap();
        assert_existing_ready(&mut controller, TextLanguage::SimplifiedChinese);
        assert_ne!(controller.ready().unwrap().identity().session, old.session);
        assert_eq!(factory.stats.builds.load(Ordering::SeqCst), 1);
    });
}

#[test]
fn remote_sidecar_buffered_old_completion_cannot_publish_in_restarted_session() {
    with_remote_source(|source| {
        let fixture = Fixture::new();
        let factory = Factory::new(SpeechMode::Success, TextMode::Success);
        let mut controller = PreparedController::with_factory(factory.clone());
        controller
            .start(
                selected_request(
                    source,
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
        assert!(controller.ready().is_none());
        controller
            .start(
                selected_request(
                    source,
                    &fixture,
                    SubtitleFormat::WebVtt,
                    TextLanguage::Source(SpeechLanguage::German),
                ),
                options(Some(SpeechProvider::Gemini), SourceLanguage::Auto),
                key(),
            )
            .unwrap();
        assert_existing_ready(
            &mut controller,
            TextLanguage::Source(SpeechLanguage::German),
        );
        assert_ne!(controller.ready().unwrap().identity().session, old.session);
        assert!(
            factory.stats.selected.lock().unwrap().as_slice()
                == [
                    (SpeechProvider::OpenAi, PreparedServiceKind::Translation),
                    (SpeechProvider::Gemini, PreparedServiceKind::Translation),
                ]
        );
        assert_eq!(factory.stats.speech_calls.load(Ordering::SeqCst), 0);
        assert_eq!(factory.stats.keys.load(Ordering::SeqCst), 2);
    });
}

struct RecognitionInstead(Arc<Factory>);
impl SelectedPreparedServiceFactory for RecognitionInstead {
    fn build(
        &self,
        provider: SpeechProvider,
        key: Option<ApiKey>,
        kind: PreparedServiceKind,
    ) -> Result<PreparedServices, PreparedUiError> {
        assert!(kind == PreparedServiceKind::Translation);
        self.0
            .build(provider, key, PreparedServiceKind::Recognition)
    }
}
#[test]
fn remote_sidecar_faulty_factory_cannot_replace_selected_translation_with_recognition() {
    with_remote_source(|source| {
        let fixture = Fixture::new();
        let factory = Factory::new(SpeechMode::Success, TextMode::Success);
        let mut controller =
            PreparedController::with_factory(Arc::new(RecognitionInstead(factory.clone())));
        controller
            .start(
                selected_request(
                    source,
                    &fixture,
                    SubtitleFormat::Srt,
                    TextLanguage::Source(SpeechLanguage::English),
                ),
                options(Some(SpeechProvider::OpenAi), SourceLanguage::Auto),
                key(),
            )
            .unwrap();
        assert_eq!(
            terminal(&mut controller),
            PreparedUiState::Failed(PreparedUiError::WorkerFailed)
        );
        assert!(controller.ready().is_none());
        assert_no_provider_work(&factory);
        assert_eq!(factory.stats.speech_capabilities.load(Ordering::SeqCst), 0);
        assert!(source.rows.lock().unwrap().is_empty());
    });
}
