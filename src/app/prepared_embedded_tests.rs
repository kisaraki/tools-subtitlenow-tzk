//! Same selected controller and source pin, with explicit offline services only.
use super::*;
use crate::media::embedded_mp4::Mp4SubtitleSelection;

type CropFixture = fn(TextLanguage, u32, u32, u32) -> Vec<u8>;

#[test]
fn prepared_context_controller_invalid_summary_fails_before_translation_without_ready_or_pin() {
    let _serial = OWNERS.lock().unwrap();
    for provider in [SpeechProvider::OpenAi, SpeechProvider::Gemini] {
        let fixture = Fixture::new();
        let request = fixture.embedded(TextLanguage::Source(SpeechLanguage::English));
        let original = fs::read(&fixture.media).unwrap();
        let factory = Factory::new(Behavior::Analysis(AnalysisMode::InvalidSummary));
        let mut controller = PreparedController::with_factory(factory.clone());
        controller
            .start(request, options(Some(provider)), key())
            .unwrap();
        assert_eq!(
            terminal(&mut controller),
            PreparedUiState::Failed(PreparedUiError::Translation(TranslationError::InvalidData))
        );
        assert!(controller.ready().is_none());
        assert_eq!(factory.stats.analysis_calls.load(Ordering::SeqCst), 1);
        assert_eq!(factory.stats.calls.load(Ordering::SeqCst), 0);
        assert!(
            fs::OpenOptions::new()
                .write(true)
                .open(&fixture.media)
                .is_ok()
        );
        assert_eq!(fs::read(&fixture.media).unwrap(), original);
    }
}

#[test]
fn mp4_text_crop_controller_chinese_ready_keeps_trimmed_times_and_pin() {
    cropped_chinese_ready(crate::media::local_audio_format::mp4_cropped_subtitle_fixture);
}
#[test]
fn mp4_text_v1_controller_chinese_ready_keeps_trimmed_times_and_pin() {
    cropped_chinese_ready(crate::media::local_audio_format::mp4_v1_subtitle_fixture);
}
fn cropped_chinese_ready(make_fixture: CropFixture) {
    let _serial = OWNERS.lock().unwrap();
    for language in [
        TextLanguage::TaiwanTraditional,
        TextLanguage::OtherTraditional,
        TextLanguage::SimplifiedChinese,
    ] {
        let fixture = Fixture::new();
        let bytes = make_fixture(language, 200, 1400, 500);
        fs::write(&fixture.media, &bytes).unwrap();
        let factory = Factory::new(Behavior::Panic);
        let mut controller = PreparedController::with_factory(factory.clone());
        let request =
            PreparedRequest::new(MediaInput::local(fixture.media.clone()).unwrap(), vec![])
                .unwrap()
                .with_embedded_subtitle(Mp4SubtitleSelection::new(3, language).unwrap())
                .unwrap();
        controller.start(request, options(None), key()).unwrap();
        assert_eq!(
            terminal(&mut controller),
            PreparedUiState::Ready { cues: 2 }
        );
        let doc = controller.ready().unwrap();
        let original = doc.original().existing().unwrap();
        assert_eq!(original.cues[0].start_us, Some(500_000));
        assert_eq!(original.cues[0].duration_us, Some(500_000));
        assert_eq!(original.cues[1].start_us, Some(1_400_000));
        assert_eq!(original.cues[1].duration_us, Some(500_000));
        assert_eq!(doc.translated()[0].as_ref(), "軟體與記憶體");
        assert_eq!(factory.builds.load(Ordering::SeqCst), 0);
        assert!(
            fs::OpenOptions::new()
                .write(true)
                .open(&fixture.media)
                .is_err()
        );
        controller.stop();
        assert!(
            fs::OpenOptions::new()
                .write(true)
                .open(&fixture.media)
                .is_ok()
        );
        assert_eq!(fs::read(&fixture.media).unwrap(), bytes);
    }
}
#[test]
fn mp4_text_crop_controller_only_visible_source_cue_is_analyzed_and_translated() {
    cropped_source_ready(crate::media::local_audio_format::mp4_cropped_subtitle_fixture);
}
#[test]
fn mp4_text_v1_controller_only_visible_source_cue_is_analyzed_and_translated() {
    cropped_source_ready(crate::media::local_audio_format::mp4_v1_subtitle_fixture);
}
fn cropped_source_ready(make_fixture: CropFixture) {
    let _serial = OWNERS.lock().unwrap();
    for provider in [SpeechProvider::OpenAi, SpeechProvider::Gemini] {
        for quality in [
            TranslationQuality::Fast,
            TranslationQuality::Standard,
            TranslationQuality::High,
        ] {
            let fixture = Fixture::new();
            let language = TextLanguage::Source(SpeechLanguage::English);
            fs::write(&fixture.media, make_fixture(language, 1100, 500, 250)).unwrap();
            let request =
                PreparedRequest::new(MediaInput::local(fixture.media.clone()).unwrap(), vec![])
                    .unwrap()
                    .with_embedded_subtitle(Mp4SubtitleSelection::new(3, language).unwrap())
                    .unwrap();
            let factory = Factory::new(Behavior::Analysis(AnalysisMode::Cropped));
            let mut controller = PreparedController::with_factory(factory.clone());
            let mut choices = options(Some(provider));
            choices.quality = quality;
            controller.start(request, choices, key()).unwrap();
            assert_eq!(
                terminal(&mut controller),
                PreparedUiState::Ready { cues: 1 }
            );
            let cue = &controller
                .ready()
                .unwrap()
                .original()
                .existing()
                .unwrap()
                .cues[0];
            assert_eq!(cue.text.as_ref(), "Aurora Engine two");
            assert_eq!(cue.start_us, Some(250_000));
            assert_eq!(cue.duration_us, Some(500_000));
            assert_eq!(*factory.providers.lock().unwrap(), [provider]);
            assert_eq!(factory.stats.analysis_calls.load(Ordering::SeqCst), 1);
            assert_eq!(
                factory.stats.calls.load(Ordering::SeqCst),
                if quality == TranslationQuality::Fast {
                    1
                } else {
                    2
                }
            );
            controller.stop();
        }
    }
}
#[test]
fn mp4_text_crop_controller_empty_visible_interval_fails_before_cloud_requests() {
    cropped_empty_fails(crate::media::local_audio_format::mp4_cropped_subtitle_fixture);
}
#[test]
fn mp4_text_v1_controller_empty_visible_interval_fails_before_cloud_requests() {
    cropped_empty_fails(crate::media::local_audio_format::mp4_v1_subtitle_fixture);
}
fn cropped_empty_fails(make_fixture: CropFixture) {
    let _serial = OWNERS.lock().unwrap();
    let fixture = Fixture::new();
    let language = TextLanguage::Source(SpeechLanguage::English);
    fs::write(&fixture.media, make_fixture(language, 700, 400, 0)).unwrap();
    let request = PreparedRequest::new(MediaInput::local(fixture.media.clone()).unwrap(), vec![])
        .unwrap()
        .with_embedded_subtitle(Mp4SubtitleSelection::new(3, language).unwrap())
        .unwrap();
    let factory = Factory::new(Behavior::Analysis(AnalysisMode::Grounded));
    let mut controller = PreparedController::with_factory(factory.clone());
    controller
        .start(request, options(Some(SpeechProvider::OpenAi)), key())
        .unwrap();
    assert_eq!(
        terminal(&mut controller),
        PreparedUiState::Failed(PreparedUiError::Media(MediaError::InvalidSubtitle))
    );
    assert_eq!(factory.stats.analysis_calls.load(Ordering::SeqCst), 0);
    assert_eq!(factory.stats.calls.load(Ordering::SeqCst), 0);
    assert!(
        fs::OpenOptions::new()
            .write(true)
            .open(&fixture.media)
            .is_ok()
    );
}

#[test]
fn mp4_text_edit_controller_chinese_ready_preserves_presentation_times_and_source_pin() {
    let _serial = OWNERS.lock().unwrap();
    for language in [
        TextLanguage::TaiwanTraditional,
        TextLanguage::OtherTraditional,
        TextLanguage::SimplifiedChinese,
    ] {
        let fixture = Fixture::new();
        let bytes = crate::media::local_audio_format::mp4_delayed_subtitle_fixture(language, 500);
        fs::write(&fixture.media, &bytes).unwrap();
        let factory = Factory::new(Behavior::Panic);
        let mut controller = PreparedController::with_factory(factory.clone());
        let request =
            PreparedRequest::new(MediaInput::local(fixture.media.clone()).unwrap(), vec![])
                .unwrap()
                .with_embedded_subtitle(Mp4SubtitleSelection::new(3, language).unwrap())
                .unwrap();
        controller.start(request, options(None), key()).unwrap();
        assert_eq!(
            terminal(&mut controller),
            PreparedUiState::Ready { cues: 2 }
        );
        let document = controller.ready().unwrap();
        let original = document.original().existing().unwrap();
        assert_eq!(original.cues[0].start_us, Some(500_000));
        assert_eq!(original.cues[0].duration_us, Some(700_000));
        assert_eq!(original.cues[1].start_us, Some(1_600_000));
        assert_eq!(original.cues[1].duration_us, Some(1_000_000));
        assert_eq!(original.identity, document.identity());
        assert_eq!(document.translated()[0].as_ref(), "軟體與記憶體");
        assert_eq!(document.translated()[1].as_ref(), "滑鼠與伺服器");
        assert_eq!(factory.builds.load(Ordering::SeqCst), 0);
        assert!(
            fs::OpenOptions::new()
                .write(true)
                .open(&fixture.media)
                .is_err()
        );
        assert!(fs::remove_file(&fixture.media).is_err());
        controller.stop();
        assert!(controller.ready().is_none());
        assert!(
            fs::OpenOptions::new()
                .write(true)
                .open(&fixture.media)
                .is_ok()
        );
        assert_eq!(fs::read(&fixture.media).unwrap(), bytes);
        assert_eq!(fs::read_dir(fixture.directory.path()).unwrap().count(), 1);
    }
}
#[test]
fn mp4_text_edit_controller_selected_translation_strategies_keep_delayed_original_times() {
    let _serial = OWNERS.lock().unwrap();
    for provider in [SpeechProvider::OpenAi, SpeechProvider::Gemini] {
        for quality in [
            TranslationQuality::Fast,
            TranslationQuality::Standard,
            TranslationQuality::High,
        ] {
            let fixture = Fixture::new();
            let language = TextLanguage::Source(SpeechLanguage::English);
            fs::write(
                &fixture.media,
                crate::media::local_audio_format::mp4_delayed_subtitle_fixture(language, 500),
            )
            .unwrap();
            let request =
                PreparedRequest::new(MediaInput::local(fixture.media.clone()).unwrap(), vec![])
                    .unwrap()
                    .with_embedded_subtitle(Mp4SubtitleSelection::new(3, language).unwrap())
                    .unwrap();
            let factory = Factory::new(Behavior::Analysis(AnalysisMode::Grounded));
            let mut controller = PreparedController::with_factory(factory.clone());
            let mut choices = options(Some(provider));
            choices.quality = quality;
            controller.start(request, choices, key()).unwrap();
            assert_eq!(
                terminal(&mut controller),
                PreparedUiState::Ready { cues: 2 }
            );
            let document = controller.ready().unwrap();
            assert_eq!(
                document.original().existing().unwrap().cues[0].start_us,
                Some(500_000)
            );
            assert_eq!(
                document.original().existing().unwrap().cues[1].start_us,
                Some(1_600_000)
            );
            assert_eq!(factory.builds.load(Ordering::SeqCst), 1);
            assert_eq!(*factory.providers.lock().unwrap(), [provider]);
            assert_eq!(factory.stats.analysis_calls.load(Ordering::SeqCst), 1);
            assert_eq!(
                factory.stats.calls.load(Ordering::SeqCst),
                if quality == TranslationQuality::Fast {
                    2
                } else {
                    4
                }
            );
            controller.stop();
            assert!(
                fs::OpenOptions::new()
                    .write(true)
                    .open(&fixture.media)
                    .is_ok()
            );
        }
    }
}
#[test]
fn mp4_text_edit_controller_out_of_range_edit_never_analyzes_translates_or_falls_back() {
    let _serial = OWNERS.lock().unwrap();
    let fixture = Fixture::new();
    let language = TextLanguage::Source(SpeechLanguage::English);
    let mut bytes = crate::media::local_audio_format::mp4_delayed_subtitle_fixture(language, 500);
    let at = bytes.windows(4).position(|b| b == b"elst").unwrap() + 4;
    bytes[at + 24..at + 28].copy_from_slice(&1u32.to_be_bytes()); // active media_time crop
    fs::write(&fixture.media, &bytes).unwrap();
    let request = PreparedRequest::new(MediaInput::local(fixture.media.clone()).unwrap(), vec![])
        .unwrap()
        .with_embedded_subtitle(Mp4SubtitleSelection::new(3, language).unwrap())
        .unwrap();
    let factory = Factory::new(Behavior::Analysis(AnalysisMode::Grounded));
    let mut controller = PreparedController::with_factory(factory.clone());
    controller
        .start(request, options(Some(SpeechProvider::OpenAi)), key())
        .unwrap();
    assert_eq!(
        terminal(&mut controller),
        PreparedUiState::Failed(PreparedUiError::Media(MediaError::UnsupportedMedia))
    );
    assert_eq!(factory.stats.analysis_calls.load(Ordering::SeqCst), 0);
    assert_eq!(factory.stats.calls.load(Ordering::SeqCst), 0);
    assert!(controller.ready().is_none());
    assert!(
        fs::OpenOptions::new()
            .write(true)
            .open(&fixture.media)
            .is_ok()
    );
    assert_eq!(fs::read(&fixture.media).unwrap(), bytes);
}

#[test]
fn mp4_multitrack_controller_runs_only_explicit_track_and_keeps_selected_service_boundary() {
    let _serial = OWNERS.lock().unwrap();
    let languages = [
        TextLanguage::Source(SpeechLanguage::English),
        TextLanguage::SimplifiedChinese,
    ];
    for provider in [SpeechProvider::OpenAi, SpeechProvider::Gemini] {
        for (id, language) in [(3, languages[0]), (4, languages[1])] {
            let fixture = Fixture::new();
            let bytes = crate::media::local_audio_format::mp4_multi_subtitle_fixture(&languages);
            fs::write(&fixture.media, &bytes).unwrap();
            let factory = Factory::new(if id == 4 {
                Behavior::Panic
            } else {
                Behavior::Analysis(AnalysisMode::Grounded)
            });
            let mut controller = PreparedController::with_factory(factory.clone());
            let request =
                PreparedRequest::new(MediaInput::local(fixture.media.clone()).unwrap(), vec![])
                    .unwrap()
                    .with_embedded_subtitle(Mp4SubtitleSelection::new(id, language).unwrap())
                    .unwrap();
            controller
                .start(request, options(Some(provider)), key())
                .unwrap();
            assert_eq!(
                terminal(&mut controller),
                PreparedUiState::Ready { cues: 2 }
            );
            let original = controller.ready().unwrap().original().existing().unwrap();
            assert!(
                original
                    .cues
                    .iter()
                    .all(|cue| cue.text.contains(&id.to_string()))
            );
            assert_eq!(original.cues[1].start_us, Some(1_100_000));
            if id == 4 {
                assert_eq!(factory.builds.load(Ordering::SeqCst), 0);
                assert_eq!(
                    controller.ready().unwrap().translated()[0].as_ref(),
                    "字幕軌 4 軟體與記憶體"
                );
            } else {
                assert_eq!(factory.builds.load(Ordering::SeqCst), 1);
                assert_eq!(*factory.providers.lock().unwrap(), [provider]);
                assert_eq!(factory.stats.analysis_calls.load(Ordering::SeqCst), 1);
            }
            assert!(
                fs::OpenOptions::new()
                    .write(true)
                    .open(&fixture.media)
                    .is_err()
            );
            controller.stop();
            assert!(
                fs::OpenOptions::new()
                    .write(true)
                    .open(&fixture.media)
                    .is_ok()
            );
            assert_eq!(fs::read(&fixture.media).unwrap(), bytes);
        }
    }
}

#[test]
fn embedded_prepared_controller_chinese_never_builds_service_or_uses_key() {
    let _serial = OWNERS.lock().unwrap();
    for language in [
        TextLanguage::TaiwanTraditional,
        TextLanguage::OtherTraditional,
        TextLanguage::SimplifiedChinese,
    ] {
        let fixture = Fixture::new();
        let request = fixture.embedded(language);
        let before = fs::read(&fixture.media).unwrap();
        let factory = Factory::new(Behavior::Panic);
        let mut controller = PreparedController::with_factory(factory.clone());
        controller.start(request, options(None), key()).unwrap();
        assert_eq!(
            terminal(&mut controller),
            PreparedUiState::Ready { cues: 2 }
        );
        assert_eq!(factory.builds.load(Ordering::SeqCst), 0);
        let document = controller.ready().unwrap();
        assert_eq!(document.translated()[0].as_ref(), "軟體與記憶體");
        assert_eq!(
            document.original().existing().unwrap().cues[1].start_us,
            Some(1_100_000)
        );
        assert!(
            fs::OpenOptions::new()
                .write(true)
                .open(&fixture.media)
                .is_err()
        );
        assert_eq!(fs::read(&fixture.media).unwrap(), before);
        controller.stop();
        assert!(controller.ready().is_none());
        assert!(
            fs::OpenOptions::new()
                .write(true)
                .open(&fixture.media)
                .is_ok()
        );
    }
}
#[test]
fn embedded_prepared_controller_all_languages_qualities_and_selected_services_share_analysis() {
    let _serial = OWNERS.lock().unwrap();
    for provider in [SpeechProvider::OpenAi, SpeechProvider::Gemini] {
        for language in SpeechLanguage::ALL {
            for quality in [
                TranslationQuality::Fast,
                TranslationQuality::Standard,
                TranslationQuality::High,
            ] {
                let fixture = Fixture::new();
                let request = fixture.embedded(TextLanguage::Source(language));
                let factory = Factory::new(Behavior::Analysis(AnalysisMode::Grounded));
                let mut controller = PreparedController::with_factory(factory.clone());
                let mut choices = options(Some(provider));
                choices.quality = quality;
                controller.start(request, choices, key()).unwrap();
                assert_eq!(
                    terminal(&mut controller),
                    PreparedUiState::Ready { cues: 2 }
                );
                assert_eq!(factory.builds.load(Ordering::SeqCst), 1);
                assert_eq!(factory.explicit_keys.load(Ordering::SeqCst), 1);
                assert_eq!(*factory.providers.lock().unwrap(), [provider]);
                assert_eq!(factory.stats.analysis_calls.load(Ordering::SeqCst), 1);
                let passes = if quality == TranslationQuality::Fast {
                    1
                } else {
                    2
                };
                assert_eq!(factory.stats.calls.load(Ordering::SeqCst), 2 * passes);
                assert!(
                    factory
                        .stats
                        .qualities
                        .lock()
                        .unwrap()
                        .iter()
                        .all(|seen| *seen == quality)
                );
                assert_eq!(factory.stats.stages.lock().unwrap()[0], "analysis");
                assert!(
                    factory
                        .stats
                        .glossaries
                        .lock()
                        .unwrap()
                        .iter()
                        .all(|(_, terms)| terms
                            .iter()
                            .any(|(source, translated)| source == "Aurora Engine"
                                && translated == "極光引擎"))
                );
                let doc = controller.ready().unwrap();
                assert_eq!(
                    doc.original().existing().unwrap().cues[1].start_us,
                    Some(1_100_000)
                );
                assert_eq!(doc.translated()[0].as_ref(), "臺灣譯文 0");
                assert_eq!(doc.translated()[1].as_ref(), "臺灣譯文 1");
                assert_eq!(
                    doc.original().existing().unwrap().cues[0].text.as_ref(),
                    "Aurora Engine one"
                );
                assert!(fs::remove_file(&fixture.media).is_err());
                controller.stop();
                assert!(
                    fs::OpenOptions::new()
                        .write(true)
                        .open(&fixture.media)
                        .is_ok()
                );
            }
        }
    }
}
#[test]
fn embedded_prepared_controller_invalid_track_or_container_never_requests_analysis_or_asr() {
    let _serial = OWNERS.lock().unwrap();
    for invalid_track in [false, true] {
        let fixture = Fixture::new();
        let mut request = fixture.embedded(TextLanguage::Source(SpeechLanguage::English));
        if invalid_track {
            request.embedded = Some(
                Mp4SubtitleSelection::new(4, TextLanguage::Source(SpeechLanguage::English))
                    .unwrap(),
            );
        } else {
            fs::write(&fixture.media, b"invalid container").unwrap();
        }
        let factory = Factory::new(Behavior::Success);
        let mut controller = PreparedController::with_factory(factory.clone());
        controller
            .start(request, options(Some(SpeechProvider::OpenAi)), key())
            .unwrap();
        assert_eq!(
            terminal(&mut controller),
            PreparedUiState::Failed(PreparedUiError::Media(MediaError::UnsupportedMedia))
        );
        assert!(controller.ready().is_none());
        assert_eq!(factory.stats.analysis_calls.load(Ordering::SeqCst), 0);
        assert_eq!(factory.stats.calls.load(Ordering::SeqCst), 0);
        assert!(
            fs::OpenOptions::new()
                .write(true)
                .open(&fixture.media)
                .is_ok()
        );
    }
}
#[test]
fn embedded_prepared_controller_cancelled_analysis_releases_same_pin_and_restarts() {
    let _serial = OWNERS.lock().unwrap();
    let fixture = Fixture::new();
    let request = fixture.embedded(TextLanguage::Source(SpeechLanguage::English));
    let (entered, received) = mpsc::sync_channel(1);
    let factory = Factory::new(Behavior::AnalysisPending(Mutex::new(Some(entered))));
    let mut controller = PreparedController::with_factory(factory.clone());
    controller
        .start(request, options(Some(SpeechProvider::Gemini)), key())
        .unwrap();
    let token = received.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(
        fs::OpenOptions::new()
            .write(true)
            .open(&fixture.media)
            .is_err()
    );
    assert!(fs::remove_file(&fixture.media).is_err());
    assert_eq!(
        crate::media::work::workers().available_permits(),
        crate::media::work::worker_limit()
    );
    controller.stop();
    assert_eq!(terminal(&mut controller), PreparedUiState::Cancelled);
    assert_eq!(token.reason(), Some(TranslationError::Cancelled));
    assert_eq!(factory.stats.calls.load(Ordering::SeqCst), 0);
    assert!(controller.ready().is_none());
    assert!(
        fs::OpenOptions::new()
            .write(true)
            .open(&fixture.media)
            .is_ok()
    );
    controller
        .start(
            fixture.embedded(TextLanguage::TaiwanTraditional),
            options(None),
            None,
        )
        .unwrap();
    assert_eq!(
        terminal(&mut controller),
        PreparedUiState::Ready { cues: 2 }
    );
    assert_eq!(factory.builds.load(Ordering::SeqCst), 1);
}
#[test]
fn embedded_prepared_controller_typed_provider_error_does_not_fallback_and_requires_service() {
    let _serial = OWNERS.lock().unwrap();
    let fixture = Fixture::new();
    let factory = Factory::new(Behavior::Analysis(AnalysisMode::Fail(
        TranslationError::Network,
    )));
    let mut controller = PreparedController::with_factory(factory.clone());
    controller
        .start(
            fixture.embedded(TextLanguage::Source(SpeechLanguage::English)),
            options(Some(SpeechProvider::OpenAi)),
            key(),
        )
        .unwrap();
    assert_eq!(
        terminal(&mut controller),
        PreparedUiState::Failed(PreparedUiError::Translation(TranslationError::Network))
    );
    assert_eq!(factory.stats.calls.load(Ordering::SeqCst), 0);
    assert_eq!(*factory.providers.lock().unwrap(), [SpeechProvider::OpenAi]);
    assert!(
        fs::OpenOptions::new()
            .write(true)
            .open(&fixture.media)
            .is_ok()
    );
    assert_eq!(
        controller.start(
            fixture.embedded(TextLanguage::Source(SpeechLanguage::English)),
            options(None),
            key()
        ),
        Err(PreparedUiError::ProviderRequired)
    );
    assert_eq!(factory.builds.load(Ordering::SeqCst), 1);
}
