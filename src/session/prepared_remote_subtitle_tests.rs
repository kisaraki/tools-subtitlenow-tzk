//! Explicit synthetic local text plus loopback WAVE headers; no PCM or cloud.
use super::*;
use crate::{
    media::embedded_mp4::Mp4SubtitleSelection,
    resolver::{contracts::SubtitleSource, timed_subtitle_parse::MAX_PREPARED_DOCUMENT_BYTES},
};
use std::{fs, path::PathBuf};

struct Sidecars(tempfile::TempDir);
impl Sidecars {
    fn new() -> Self {
        Self(tempfile::tempdir().unwrap())
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.path().join(name)
    }
    fn input(
        &self,
        name: &str,
        format: SubtitleFormat,
        language: TextLanguage,
        text: &str,
    ) -> LocalSubtitleInput {
        let body = match format {
            SubtitleFormat::Srt => format!("1\n00:00:00,001 --> 00:00:00,051\n{text}\n"),
            SubtitleFormat::WebVtt => {
                format!("WEBVTT\n\n00:00:00.001 --> 00:00:00.051\n{text}\n")
            }
        };
        fs::write(self.path(name), body).unwrap();
        LocalSubtitleInput::new(self.path(name), format, language).unwrap()
    }
    fn request(
        &self,
        source: &Source,
        format: SubtitleFormat,
        language: TextLanguage,
    ) -> PreparedRequest {
        PreparedRequest::new(
            source.media_input(),
            vec![self.input("explicit.data", format, language, "synthetic subtitle")],
        )
        .unwrap()
    }
    fn chinese(&self, source: &Source) -> PreparedRequest {
        PreparedRequest::new(
            source.media_input(),
            vec![self.input(
                "explicit.data",
                SubtitleFormat::Srt,
                TextLanguage::SimplifiedChinese,
                "软件与服务器",
            )],
        )
        .unwrap()
    }
}
async fn run_subtitle(
    task: PreparedTask,
    provider: Option<&dyn PreparedTranslator>,
    quality: TranslationQuality,
) -> PreparedCompletion {
    tokio::time::timeout(
        TEST_TIMEOUT,
        task.run_with_remote_wave_subtitle(provider, quality, None),
    )
    .await
    .unwrap()
}
fn headers_only(source: &Source) {
    // Canonical fixture PCM begins at byte 44. Content probing and RIFF/fmt/
    // data headers may be read, but not even one byte of the audio payload.
    let rows = source.rows.lock().unwrap();
    assert!(!rows.is_empty());
    assert!(
        rows.iter()
            .all(|(start, length)| start + *length as u64 <= 44)
    );
    assert!(rows.contains(&(36, 8)));
}
fn no_speech(services: &Services) {
    assert_eq!(services.capabilities.load(Ordering::SeqCst), 0);
    assert_eq!(services.asr.load(Ordering::SeqCst), 0);
}

#[test]
fn remote_subtitle_chinese_srt_and_vtt_stay_local_with_header_only_ram_ready() {
    lane(async {
        for format in [SubtitleFormat::Srt, SubtitleFormat::WebVtt] {
            for language in [
                TextLanguage::TaiwanTraditional,
                TextLanguage::OtherTraditional,
                TextLanguage::SimplifiedChinese,
            ] {
                for supplied_service in [false, true] {
                    let source = Source::new(wave(16000, 1, &samples(1600, 1))).await;
                    let sidecars = Sidecars::new();
                    let input = sidecars.input("local.data", format, language, "软件与服务器");
                    let before = fs::read(sidecars.path("local.data")).unwrap();
                    let mut owner = PreparedSession::new();
                    let task = owner
                        .start(PreparedRequest::new(source.media_input(), vec![input]).unwrap())
                        .unwrap();
                    let identity = task.identity;
                    let services = Services::new();
                    let completion = run_subtitle(
                        task,
                        supplied_service.then_some(&services as &dyn PreparedTranslator),
                        TranslationQuality::High,
                    )
                    .await;
                    assert!(owner.ready().is_none());
                    assert!(completion.result.is_ok(), "{:?}", completion.result.err());
                    assert!(owner.apply(completion).unwrap());
                    let document = owner.ready().unwrap();
                    assert_eq!(document.identity(), identity);
                    assert_eq!(document.sequence(), 1);
                    assert_eq!(document.media_byte_len(), 3244);
                    assert_eq!(document.cue_count(), 1);
                    assert_eq!(document.target_language(), "zh-TW");
                    assert!(document.original().recognized().is_none());
                    let original = document.original().existing().unwrap();
                    assert_eq!(original.source, SubtitleSource::ExistingSubtitle);
                    assert!(original.language == language);
                    assert_eq!(&*original.cues[0].text, "软件与服务器");
                    assert_eq!(original.cues[0].start_us, Some(1000));
                    assert_eq!(original.cues[0].duration_us, Some(50000));
                    assert_eq!(document.translated()[0].as_ref(), "軟體與伺服器");
                    assert!(matches!(document._media, PreparedMediaOwner::Remote(_)));
                    assert_eq!(services.counts(), (0, 0, 0));
                    no_speech(&services);
                    headers_only(&source);
                    assert_eq!(fs::read(sidecars.path("local.data")).unwrap(), before);
                    assert_eq!(fs::read_dir(sidecars.0.path()).unwrap().count(), 1);
                    let reads = source.rows.lock().unwrap().len();
                    owner.stop();
                    assert!(owner.ready().is_none());
                    assert_eq!(source.rows.lock().unwrap().len(), reads);
                    source.finish().await;
                }
            }
        }
    });
}

#[test]
fn remote_subtitle_five_languages_all_qualities_reuse_analysis_and_translation_without_asr() {
    lane(async {
        for language in [
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
                for format in [SubtitleFormat::Srt, SubtitleFormat::WebVtt] {
                    let source = Source::new(wave(44100, 2, &samples(4410, 2))).await;
                    let sidecars = Sidecars::new();
                    let mut owner = PreparedSession::new();
                    let task = owner
                        .start(sidecars.request(&source, format, TextLanguage::Source(language)))
                        .unwrap();
                    let services = Services::new();
                    let completion = run_subtitle(task, Some(&services), quality).await;
                    assert!(completion.result.is_ok(), "{:?}", completion.result.err());
                    assert!(owner.apply(completion).unwrap());
                    let document = owner.ready().unwrap();
                    assert_eq!(document.cue_count(), 1);
                    let original = document.original().existing().unwrap();
                    assert!(original.language == TextLanguage::Source(language));
                    assert_eq!(&*original.cues[0].text, "synthetic subtitle");
                    assert_eq!(original.cues[0].start_us, Some(1000));
                    assert_eq!(original.cues[0].duration_us, Some(50000));
                    assert_eq!(
                        document.translated()[0].as_ref(),
                        if quality == TranslationQuality::Fast {
                            "軟體與伺服器 草稿"
                        } else {
                            "軟體與伺服器 校訂"
                        }
                    );
                    let expected_passes = if quality == TranslationQuality::Fast {
                        1
                    } else {
                        2
                    };
                    assert_eq!(services.counts(), (0, 1, expected_passes));
                    no_speech(&services);
                    {
                        let rows = services.text_rows.lock().unwrap();
                        assert_eq!(rows.len(), expected_passes);
                        assert!(rows.iter().all(|row| row.1 == rows[0].1));
                    }
                    headers_only(&source);
                    owner.stop();
                    source.finish().await;
                }
            }
        }
    });
}

#[test]
fn remote_subtitle_priority_reads_only_selected_sidecar_and_does_not_claim_media_alignment() {
    lane(async {
        let source = Source::new(wave(16000, 1, &samples(1600, 1))).await;
        let sidecars = Sidecars::new();
        let mut owner = PreparedSession::new();
        let preferred = sidecars.input(
            "preferred.data",
            SubtitleFormat::Srt,
            TextLanguage::TaiwanTraditional,
            "軟體與伺服器",
        );
        // The lower-ranked selection does not exist. It must not be opened or
        // used to force ASR, cloud service creation or a failed preparation.
        let absent = LocalSubtitleInput::new(
            sidecars.path("absent.data"),
            SubtitleFormat::Srt,
            TextLanguage::Source(SpeechLanguage::English),
        )
        .unwrap();
        let task = owner
            .start(PreparedRequest::new(source.media_input(), vec![absent, preferred]).unwrap())
            .unwrap();
        assert!(
            owner
                .apply(run_subtitle(task, None, TranslationQuality::Standard).await)
                .unwrap()
        );
        let document = owner.ready().unwrap();
        let original = document.original().existing().unwrap();
        assert_eq!(original.source, SubtitleSource::ExistingSubtitle);
        assert!(original.language == TextLanguage::TaiwanTraditional);
        assert_eq!(original.cues[0].start_us, Some(1000));
        headers_only(&source);
        owner.stop();
        source.finish().await;
    });
}

#[test]
fn remote_subtitle_metadata_preflight_preserves_ambiguity_and_rejects_embedded_or_missing_text() {
    lane(async {
        let source = Source::new(wave(16000, 1, &samples(1600, 1))).await;
        let sidecars = Sidecars::new();
        let services = Services::new();
        for scenario in 0..5 {
            let mut owner = PreparedSession::new();
            let choices = match scenario {
                0 | 1 => vec![],
                _ => vec![sidecars.input(
                    "first.data",
                    SubtitleFormat::Srt,
                    TextLanguage::TaiwanTraditional,
                    "第一份",
                )],
            };
            let mut request = PreparedRequest::new(source.media_input(), choices).unwrap();
            if matches!(scenario, 1 | 2) {
                request = request
                    .with_embedded_subtitle(
                        Mp4SubtitleSelection::new(7, TextLanguage::Source(SpeechLanguage::English))
                            .unwrap(),
                    )
                    .unwrap();
            }
            if matches!(scenario, 3 | 4) {
                request.subtitles.push(sidecars.input(
                    "second.data",
                    SubtitleFormat::WebVtt,
                    TextLanguage::TaiwanTraditional,
                    "第二份",
                ));
            }
            if scenario == 4 {
                request = request
                    .with_embedded_subtitle(
                        Mp4SubtitleSelection::new(7, TextLanguage::Source(SpeechLanguage::English))
                            .unwrap(),
                    )
                    .unwrap();
            }
            let task = owner.start(request).unwrap();
            let completion =
                run_subtitle(task, Some(&services), TranslationQuality::Standard).await;
            assert_eq!(
                owner.apply(completion),
                Err(if matches!(scenario, 3 | 4) {
                    MediaError::AmbiguousSubtitle
                } else {
                    MediaError::UnsupportedUrl
                })
            );
        }
        assert!(source.rows.lock().unwrap().is_empty());
        assert_eq!(services.counts(), (0, 0, 0));
        no_speech(&services);
        source.finish().await;
    });
}

#[test]
fn remote_subtitle_missing_translator_stale_glossary_and_local_input_fail_before_reads() {
    lane(async {
        let source = Source::new(wave(16000, 1, &samples(1600, 1))).await;
        let sidecars = Sidecars::new();
        let services = Services::new();
        for scenario in 0..3 {
            let language = if scenario == 1 {
                TextLanguage::TaiwanTraditional
            } else {
                TextLanguage::Source(SpeechLanguage::English)
            };
            let subtitle = LocalSubtitleInput::new(
                sidecars.path("absent.data"),
                SubtitleFormat::Srt,
                language,
            )
            .unwrap();
            let media = if scenario == 2 {
                MediaInput::local(sidecars.path("absent.media")).unwrap()
            } else {
                source.media_input()
            };
            let mut owner = PreparedSession::new();
            let task = owner
                .start(PreparedRequest::new(media, vec![subtitle]).unwrap())
                .unwrap();
            let old = SessionGlossary::new(StreamIdentity {
                session: task.identity.session,
                generation: task.identity.generation + 1,
            });
            let completion = task
                .run_with_remote_wave_subtitle(
                    (scenario != 0).then_some(&services as &dyn PreparedTranslator),
                    TranslationQuality::Standard,
                    (scenario == 1).then_some(&old),
                )
                .await;
            assert_eq!(
                owner.apply(completion),
                Err(match scenario {
                    0 => MediaError::TranslationRequired,
                    1 => MediaError::Translation(TranslationError::InvalidData),
                    _ => MediaError::InvalidInput,
                })
            );
        }
        assert!(source.rows.lock().unwrap().is_empty());
        assert_eq!(services.counts(), (0, 0, 0));
        no_speech(&services);
        source.finish().await;
    });
}

#[test]
fn remote_subtitle_bad_selected_sidecar_fails_before_http_or_provider_without_audio_fallback() {
    lane(async {
        let source = Source::new(wave(16000, 1, &samples(1600, 1))).await;
        let sidecars = Sidecars::new();
        let services = Services::new();
        for scenario in 0..3 {
            let path = sidecars.path("selected.data");
            match scenario {
                0 => {}
                1 => fs::write(&path, b"synthetic malformed subtitle").unwrap(),
                _ => fs::write(&path, vec![b'x'; MAX_PREPARED_DOCUMENT_BYTES + 1]).unwrap(),
            }
            let subtitle = LocalSubtitleInput::new(
                path,
                SubtitleFormat::Srt,
                TextLanguage::Source(SpeechLanguage::English),
            )
            .unwrap();
            let mut owner = PreparedSession::new();
            let task = owner
                .start(PreparedRequest::new(source.media_input(), vec![subtitle]).unwrap())
                .unwrap();
            let completion = run_subtitle(task, Some(&services), TranslationQuality::High).await;
            assert_eq!(
                owner.apply(completion),
                Err(match scenario {
                    0 => MediaError::Unavailable,
                    1 => MediaError::InvalidSubtitle,
                    _ => MediaError::TooLarge,
                })
            );
            assert!(owner.ready().is_none());
        }
        assert!(source.rows.lock().unwrap().is_empty());
        assert_eq!(services.counts(), (0, 0, 0));
        no_speech(&services);
        source.finish().await;
    });
}

#[test]
fn remote_subtitle_preserves_http_and_codec_errors_before_text_service_work() {
    lane(async {
        let sidecars = Sidecars::new();
        for (status, error) in [
            (429, RemoteMediaError::RateLimited),
            (401, RemoteMediaError::AccessDenied),
            (302, RemoteMediaError::Redirected),
            (200, RemoteMediaError::UnsupportedRange),
        ] {
            let source = Source::new(wave(16000, 1, &samples(1600, 1))).await;
            source.control.status.store(status, Ordering::Release);
            let services = Services::new();
            let mut owner = PreparedSession::new();
            let task = owner
                .start(sidecars.request(
                    &source,
                    SubtitleFormat::Srt,
                    TextLanguage::Source(SpeechLanguage::English),
                ))
                .unwrap();
            let completion =
                run_subtitle(task, Some(&services), TranslationQuality::Standard).await;
            assert_eq!(owner.apply(completion), Err(MediaError::Remote(error)));
            assert_eq!(services.counts(), (0, 0, 0));
            no_speech(&services);
            source.finish().await;
        }
        // A RIFF container is insufficient: the same canonical PCM16 gate is
        // mandatory even when the selected subtitle makes ASR unnecessary.
        let mut bytes = wave(16000, 1, &samples(1600, 1));
        bytes[20..22].copy_from_slice(&3u16.to_le_bytes());
        let source = Source::new(bytes).await;
        let mut owner = PreparedSession::new();
        let task = owner.start(sidecars.chinese(&source)).unwrap();
        assert_eq!(
            owner.apply(run_subtitle(task, None, TranslationQuality::Standard).await),
            Err(MediaError::UnsupportedMedia)
        );
        assert!(owner.ready().is_none());
        assert!(
            source
                .rows
                .lock()
                .unwrap()
                .iter()
                .all(|(start, length)| start + *length as u64 <= 44)
        );
        source.finish().await;
    });
}

#[test]
fn remote_subtitle_changed_header_source_keeps_typed_error_without_translation() {
    lane(async {
        let source = Source::new(wave(16000, 1, &samples(1600, 1))).await;
        let sidecars = Sidecars::new();
        let mut owner = PreparedSession::new();
        let task = owner
            .start(sidecars.request(
                &source,
                SubtitleFormat::Srt,
                TextLanguage::Source(SpeechLanguage::English),
            ))
            .unwrap();
        let services = Services::new();
        let (started, resume) = source.pause();
        let mut work = Box::pin(run_subtitle(
            task,
            Some(&services),
            TranslationQuality::High,
        ));
        tokio::select! {
            result = work.as_mut() => panic!("early {:?}", result.result.err()),
            result = tokio::time::timeout(TEST_TIMEOUT, started) => result.unwrap().unwrap(),
        }
        // Initial representation headers already carried v1; the next range
        // carries v2. Neither selected text nor a good first response permits
        // accepting a different representation during the source preflight.
        source.control.changed.store(true, Ordering::Release);
        resume.send(()).unwrap();
        assert_eq!(
            owner.apply(work.await),
            Err(MediaError::Remote(RemoteMediaError::SourceChanged))
        );
        assert_eq!(services.counts(), (0, 0, 0));
        no_speech(&services);
        assert!(owner.ready().is_none());
        source.finish().await;
    });
}

#[test]
fn remote_subtitle_cpu_and_http_saturation_remain_bounded_before_service_work() {
    lane(async {
        let source = Source::new(wave(16000, 1, &samples(1600, 1))).await;
        let sidecars = Sidecars::new();
        let mut owner = PreparedSession::new();
        let services = Services::new();
        let cpu = crate::media::work::workers();
        let held = cpu
            .clone()
            .try_acquire_many_owned(crate::media::work::worker_limit() as u32)
            .unwrap();
        let task = owner.start(sidecars.chinese(&source)).unwrap();
        assert_eq!(
            owner.apply(run_subtitle(task, Some(&services), TranslationQuality::Standard).await),
            Err(MediaError::Overloaded)
        );
        assert!(source.rows.lock().unwrap().is_empty());
        drop(held);
        let task = owner.start(sidecars.chinese(&source)).unwrap();
        let identity = task.identity;
        let first = RemoteHttpMedia::open(
            source.media_input(),
            identity,
            ResolverCancellation::default(),
        )
        .await
        .unwrap();
        let second = RemoteHttpMedia::open(
            source.media_input(),
            identity,
            ResolverCancellation::default(),
        )
        .await
        .unwrap();
        assert_eq!(
            owner.apply(run_subtitle(task, Some(&services), TranslationQuality::Standard).await),
            Err(MediaError::Remote(RemoteMediaError::Overloaded))
        );
        assert_eq!(source.rows.lock().unwrap().len(), 2);
        assert!(owner.ready().is_none());
        drop(first);
        drop(second);
        assert_eq!(cpu.available_permits(), crate::media::work::worker_limit());
        assert_eq!(services.counts(), (0, 0, 0));
        no_speech(&services);
        source.finish().await;
    });
}

#[test]
fn remote_subtitle_analysis_translation_failures_never_publish_partial_document() {
    lane(async {
        for scenario in 0..3 {
            let source = Source::new(wave(16000, 1, &samples(1600, 1))).await;
            let sidecars = Sidecars::new();
            let mut owner = PreparedSession::new();
            let task = owner
                .start(sidecars.request(
                    &source,
                    SubtitleFormat::Srt,
                    TextLanguage::Source(SpeechLanguage::English),
                ))
                .unwrap();
            let mut services = Services::new();
            services.fail = match scenario {
                0 => Stage::Analysis,
                1 => Stage::Translation,
                _ => Stage::None,
            };
            services.invalid_summary = scenario == 2;
            let completion =
                run_subtitle(task, Some(&services), TranslationQuality::Standard).await;
            assert_eq!(
                owner.apply(completion),
                Err(MediaError::Translation(match scenario {
                    0 => TranslationError::Refused,
                    1 => TranslationError::RateLimited,
                    _ => TranslationError::InvalidData,
                }))
            );
            assert!(owner.ready().is_none());
            assert_eq!(services.active.load(Ordering::SeqCst), 0);
            if scenario != 1 {
                assert_eq!(services.translations.load(Ordering::SeqCst), 0);
            }
            no_speech(&services);
            headers_only(&source);
            source.finish().await;
        }
    });
}

#[test]
fn remote_subtitle_unpolled_prior_cancel_and_network_stop_never_publish_or_start_services() {
    lane(async {
        for scenario in 0..3 {
            let source = Source::new(wave(16000, 1, &samples(1600, 1))).await;
            let sidecars = Sidecars::new();
            let mut owner = PreparedSession::new();
            let task = owner.start(sidecars.chinese(&source)).unwrap();
            let services = Services::new();
            if scenario == 0 {
                drop(task.run_with_remote_wave_subtitle(
                    Some(&services),
                    TranslationQuality::High,
                    None,
                ));
            } else if scenario == 1 {
                owner.cancellation.cancel();
                assert_eq!(
                    run_subtitle(task, Some(&services), TranslationQuality::High)
                        .await
                        .result
                        .err(),
                    Some(MediaError::Cancelled)
                );
            } else {
                let (started, _resume) = source.pause();
                let mut work = Box::pin(run_subtitle(
                    task,
                    Some(&services),
                    TranslationQuality::High,
                ));
                tokio::select! {
                    result = work.as_mut() => panic!("early {:?}", result.result.err()),
                    result = tokio::time::timeout(TEST_TIMEOUT, started) => result.unwrap().unwrap(),
                }
                owner.stop();
                assert_eq!(work.await.result.err(), Some(MediaError::Cancelled));
            }
            assert_eq!(owner.state(), PreparedState::Cancelled);
            assert!(owner.ready().is_none());
            if scenario != 2 {
                assert!(source.rows.lock().unwrap().is_empty());
            }
            assert_eq!(services.counts(), (0, 0, 0));
            no_speech(&services);
            source.finish().await;
        }
    });
}

#[test]
fn remote_subtitle_stop_and_drop_during_service_waits_release_resources_without_ready() {
    lane(async {
        for stage in [Stage::Analysis, Stage::Translation] {
            for drop_future in [false, true] {
                let source = Source::new(wave(16000, 1, &samples(1600, 1))).await;
                let sidecars = Sidecars::new();
                let mut owner = PreparedSession::new();
                let task = owner
                    .start(sidecars.request(
                        &source,
                        SubtitleFormat::Srt,
                        TextLanguage::Source(SpeechLanguage::English),
                    ))
                    .unwrap();
                let mut services = Services::new();
                services.gate = stage;
                let mut work = Box::pin(run_subtitle(
                    task,
                    Some(&services),
                    TranslationQuality::Fast,
                ));
                entered(&mut work, &services).await;
                headers_only(&source);
                let reads = source.rows.lock().unwrap().len();
                if drop_future {
                    drop(work);
                } else {
                    owner.stop();
                    assert_eq!(work.await.result.err(), Some(MediaError::Cancelled));
                }
                assert_eq!(owner.state(), PreparedState::Cancelled);
                assert!(owner.ready().is_none());
                assert_eq!(services.active.load(Ordering::SeqCst), 0);
                assert_eq!(source.rows.lock().unwrap().len(), reads);
                assert_eq!(
                    crate::media::work::workers().available_permits(),
                    crate::media::work::worker_limit()
                );
                no_speech(&services);
                source.finish().await;
            }
        }
    });
}

#[test]
fn remote_subtitle_same_http_owner_survives_service_waits_completion_ready_and_stops_cleanly() {
    lane(async {
        for stage in [Stage::Analysis, Stage::Translation] {
            let source = Source::new(wave(16000, 1, &samples(1600, 1))).await;
            let sidecars = Sidecars::new();
            let mut owner = PreparedSession::new();
            let task = owner
                .start(sidecars.request(
                    &source,
                    SubtitleFormat::Srt,
                    TextLanguage::Source(SpeechLanguage::English),
                ))
                .unwrap();
            let identity = task.identity;
            let mut services = Services::new();
            services.gate = stage;
            let mut work = Box::pin(run_subtitle(
                task,
                Some(&services),
                TranslationQuality::Fast,
            ));
            entered(&mut work, &services).await;
            headers_only(&source);
            let reads = source.rows.lock().unwrap().len();
            assert_eq!(
                crate::media::work::workers().available_permits(),
                crate::media::work::worker_limit()
            );
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
            assert_eq!(source.rows.lock().unwrap().len(), reads + 1);
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
            no_speech(&services);
            source.finish().await;
        }
    });
}

#[test]
fn remote_subtitle_late_completion_cannot_replace_new_session_and_stop_reclaims_owner_quota() {
    lane(async {
        let source = Source::new(wave(16000, 1, &samples(1600, 1))).await;
        let sidecars = Sidecars::new();
        let mut owner = PreparedSession::new();
        let task = owner.start(sidecars.chinese(&source)).unwrap();
        let old = run_subtitle(task, None, TranslationQuality::Standard).await;
        assert!(old.result.is_ok());
        let task = owner.start(sidecars.chinese(&source)).unwrap();
        let identity = task.identity;
        assert_eq!(owner.apply(old), Err(MediaError::Stale));
        assert_eq!(owner.identity(), Some(identity));
        assert!(owner.ready().is_none());
        assert!(
            owner
                .apply(run_subtitle(task, None, TranslationQuality::Standard).await)
                .unwrap()
        );
        assert_eq!(owner.ready().unwrap().identity(), identity);
        headers_only(&source);
        owner.stop();
        let first = RemoteHttpMedia::open(
            source.media_input(),
            identity,
            ResolverCancellation::default(),
        )
        .await
        .unwrap();
        let second = RemoteHttpMedia::open(
            source.media_input(),
            identity,
            ResolverCancellation::default(),
        )
        .await
        .unwrap();
        drop(first);
        drop(second);
        source.finish().await;
    });
}
