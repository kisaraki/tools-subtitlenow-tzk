use super::*;

#[test]
fn remote_ready_unpolled_and_prior_cancel_start_no_media_or_service_work() {
    lane(async {
        for unpolled in [false, true] {
            let source = Source::new(wave(16000, 1, &samples(1600, 1))).await;
            let mut owner = PreparedSession::new();
            let task = owner.start(request(&source)).unwrap();
            let services = Services::new();
            if unpolled {
                let work = task.run_with_remote_wave_recognizer(
                    &services,
                    &services,
                    SourceLanguage::English,
                    PreparedSampleRate::Hz16000,
                    TranslationQuality::Standard,
                    None,
                );
                drop(work);
            } else {
                owner.cancellation.cancel();
                let result = run(
                    task,
                    &services,
                    PreparedSampleRate::Hz16000,
                    TranslationQuality::Standard,
                )
                .await;
                assert_eq!(result.result.err(), Some(MediaError::Cancelled));
            }
            assert_eq!(owner.state(), PreparedState::Cancelled);
            assert!(source.rows.lock().unwrap().is_empty());
            assert_eq!(services.capabilities.load(Ordering::SeqCst), 0);
            assert_eq!(services.counts(), (0, 0, 0));
            source.finish().await;
        }
    });
}

#[test]
fn remote_ready_preflight_rejects_unsupported_service_and_stale_glossary_without_io() {
    lane(async {
        for unsupported in [false, true] {
            let source = Source::new(wave(16000, 1, &samples(1600, 1))).await;
            let mut owner = PreparedSession::new();
            let task = owner.start(request(&source)).unwrap();
            let mut services = Services::new();
            services.unsupported = unsupported;
            let old = SessionGlossary::new(StreamIdentity {
                session: task.identity.session,
                generation: 2,
            });
            let completion = task
                .run_with_remote_wave_recognizer(
                    &services,
                    &services,
                    SourceLanguage::English,
                    PreparedSampleRate::Hz16000,
                    TranslationQuality::Standard,
                    if unsupported { None } else { Some(&old) },
                )
                .await;
            let expected = if unsupported {
                MediaError::Speech(PreparedSpeechError::UnsupportedCapability)
            } else {
                MediaError::Translation(TranslationError::InvalidData)
            };
            assert_eq!(owner.apply(completion), Err(expected));
            assert!(source.rows.lock().unwrap().is_empty());
            assert_eq!(
                services.capabilities.load(Ordering::SeqCst),
                usize::from(unsupported)
            );
            assert_eq!(services.counts(), (0, 0, 0));
            source.finish().await;
        }
    });
}

#[test]
fn remote_ready_existing_text_or_ambiguity_never_turns_into_an_audio_upload() {
    lane(async {
        let source = Source::new(wave(16000, 1, &samples(1600, 1))).await;
        for language in [
            TextLanguage::TaiwanTraditional,
            TextLanguage::OtherTraditional,
            TextLanguage::SimplifiedChinese,
            TextLanguage::Source(SpeechLanguage::English),
        ] {
            for ambiguous in [false, true] {
                let mut choices = Vec::new();
                for index in 0..if ambiguous { 2 } else { 1 } {
                    // Deliberately nonexistent: unsupported combinations must
                    // be rejected by metadata, not opened or treated as absent.
                    choices.push(
                        LocalSubtitleInput::new(
                            std::env::current_dir()
                                .unwrap()
                                .join(format!("missing-remote-ready-{index}.srt")),
                            SubtitleFormat::Srt,
                            language,
                        )
                        .unwrap(),
                    );
                }
                let mut owner = PreparedSession::new();
                let task = owner
                    .start(PreparedRequest::new(source.media_input(), choices).unwrap())
                    .unwrap();
                let services = Services::new();
                let completion = run(
                    task,
                    &services,
                    PreparedSampleRate::Hz16000,
                    TranslationQuality::Standard,
                )
                .await;
                assert_eq!(
                    owner.apply(completion),
                    Err(if ambiguous {
                        MediaError::AmbiguousSubtitle
                    } else {
                        MediaError::UnsupportedUrl
                    })
                );
                assert_eq!(services.capabilities.load(Ordering::SeqCst), 0);
                assert_eq!(services.counts(), (0, 0, 0));
            }
        }
        assert!(source.rows.lock().unwrap().is_empty());
        source.finish().await;
    });
}

#[test]
fn remote_ready_explicit_remote_method_does_not_accept_a_local_input() {
    lane(async {
        let mut owner = PreparedSession::new();
        let input = MediaInput::local(
            std::env::current_dir()
                .unwrap()
                .join("missing-remote-ready-local.wav"),
        )
        .unwrap();
        let task = owner
            .start(PreparedRequest::new(input, vec![]).unwrap())
            .unwrap();
        let services = Services::new();
        let completion = run(
            task,
            &services,
            PreparedSampleRate::Hz16000,
            TranslationQuality::Standard,
        )
        .await;
        assert_eq!(owner.apply(completion), Err(MediaError::InvalidInput));
        assert_eq!(services.capabilities.load(Ordering::SeqCst), 0);
        assert_eq!(services.counts(), (0, 0, 0));
    });
}

#[test]
fn remote_ready_stop_and_future_drop_cancel_each_service_stage_without_ready() {
    lane(async {
        for stage in [Stage::Asr, Stage::Analysis, Stage::Translation] {
            for drop_future in [false, true] {
                let source = Source::new(wave(16000, 1, &samples(1600, 1))).await;
                let mut owner = PreparedSession::new();
                let task = owner.start(request(&source)).unwrap();
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
                if drop_future {
                    drop(work);
                } else {
                    owner.stop();
                    let completion = work.await;
                    assert_eq!(completion.result.err(), Some(MediaError::Cancelled));
                }
                assert_eq!(owner.state(), PreparedState::Cancelled);
                assert!(owner.ready().is_none());
                assert_eq!(services.active.load(Ordering::SeqCst), 0);
                assert_eq!(source.rows.lock().unwrap().len(), reads);
                assert_eq!(
                    crate::media::work::workers().available_permits(),
                    crate::media::work::worker_limit()
                );
                source.finish().await;
            }
        }
    });
}

#[test]
fn remote_ready_network_cancellation_and_old_completion_cannot_replace_new_session() {
    lane(async {
        let source = Source::new(wave(16000, 1, &samples(1600, 1))).await;
        let mut owner = PreparedSession::new();
        let task = owner.start(request(&source)).unwrap();
        let services = Services::new();
        let (started, _resume) = source.pause();
        let mut work = Box::pin(run(
            task,
            &services,
            PreparedSampleRate::Hz16000,
            TranslationQuality::Fast,
        ));
        tokio::select! { result=work.as_mut()=>panic!("early {:?}",result.result.err()),result=tokio::time::timeout(TEST_TIMEOUT,started)=>result.unwrap().unwrap() }
        owner.stop();
        let old = work.await;
        assert_eq!(old.result.err(), Some(MediaError::Cancelled));
        assert_eq!(services.counts(), (0, 0, 0));
        source.finish().await;

        let source = Source::new(wave(16000, 1, &samples(1600, 1))).await;
        let task = owner.start(request(&source)).unwrap();
        let old = run(
            task,
            &services,
            PreparedSampleRate::Hz16000,
            TranslationQuality::Fast,
        )
        .await;
        assert!(old.result.is_ok());
        let new_task = owner.start(request(&source)).unwrap();
        let new_id = new_task.identity;
        assert_eq!(owner.apply(old), Err(MediaError::Stale));
        assert!(owner.ready().is_none());
        assert_eq!(owner.identity(), Some(new_id));
        let services = Services::new();
        assert!(
            owner
                .apply(
                    run(
                        new_task,
                        &services,
                        PreparedSampleRate::Hz16000,
                        TranslationQuality::Fast
                    )
                    .await
                )
                .unwrap()
        );
        assert_eq!(owner.ready().unwrap().identity(), new_id);
        owner.stop();
        source.finish().await;
    });
}

#[test]
fn remote_ready_http_owner_and_cpu_saturation_fail_without_any_service_requests() {
    lane(async {
        let source = Source::new(wave(16000, 1, &samples(1600, 1))).await;
        let mut owner = PreparedSession::new();
        let task = owner.start(request(&source)).unwrap();
        let identity = task.identity;
        let services = Services::new();
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
        let completion = run(
            task,
            &services,
            PreparedSampleRate::Hz16000,
            TranslationQuality::Fast,
        )
        .await;
        assert_eq!(
            owner.apply(completion),
            Err(MediaError::Remote(RemoteMediaError::Overloaded))
        );
        assert_eq!(source.rows.lock().unwrap().len(), 2);
        drop(first);
        drop(second);
        let cpu = crate::media::work::workers();
        let held = cpu
            .clone()
            .try_acquire_many_owned(crate::media::work::worker_limit() as u32)
            .unwrap();
        let task = owner.start(request(&source)).unwrap();
        let completion = run(
            task,
            &services,
            PreparedSampleRate::Hz16000,
            TranslationQuality::Fast,
        )
        .await;
        assert_eq!(owner.apply(completion), Err(MediaError::Overloaded));
        assert_eq!(services.counts(), (0, 0, 0));
        drop(held);
        assert_eq!(cpu.available_permits(), crate::media::work::worker_limit());
        source.finish().await;
    });
}

#[test]
fn remote_ready_transport_failures_and_unsupported_codec_preserve_errors_without_fallback() {
    lane(async {
        for (status, error) in [
            (429, RemoteMediaError::RateLimited),
            (401, RemoteMediaError::AccessDenied),
            (302, RemoteMediaError::Redirected),
            (200, RemoteMediaError::UnsupportedRange),
        ] {
            let source = Source::new(wave(16000, 1, &samples(1600, 1))).await;
            source.control.status.store(status, Ordering::Release);
            let mut owner = PreparedSession::new();
            let services = Services::new();
            let task = owner.start(request(&source)).unwrap();
            let completion = run(
                task,
                &services,
                PreparedSampleRate::Hz16000,
                TranslationQuality::Fast,
            )
            .await;
            assert_eq!(owner.apply(completion), Err(MediaError::Remote(error)));
            assert_eq!(services.counts(), (0, 0, 0));
            source.finish().await;
        }
        let mut bytes = vec![0; 834];
        for offset in [0, 417] {
            bytes[offset..offset + 4].copy_from_slice(&[0xff, 0xfb, 0x90, 0xc0]);
        }
        let source = Source::new(bytes).await;
        let mut owner = PreparedSession::new();
        let services = Services::new();
        let task = owner.start(request(&source)).unwrap();
        let completion = run(
            task,
            &services,
            PreparedSampleRate::Hz16000,
            TranslationQuality::Fast,
        )
        .await;
        assert_eq!(owner.apply(completion), Err(MediaError::UnsupportedMedia));
        assert_eq!(services.counts(), (0, 0, 0));
        source.finish().await;
    });
}
