//! Explicit HTTPS-subtitle preparation, synthetic local files and loopback only.
use super::*;
use crate::{
    media::{
        embedded_mp4::Mp4SubtitleSelection,
        remote_subtitle::{
            PreparedSubtitleUrl, RemoteSubtitleError, RemoteSubtitleFailure, fetch_remote_subtitle,
        },
    },
    resolver::contracts::SubtitleSource,
};
use std::{fs, path::PathBuf};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

struct LocalMedia {
    directory: tempfile::TempDir,
    path: PathBuf,
}
impl LocalMedia {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("synthetic-private.media");
        fs::write(&path, b"opaque synthetic media; selected text needs no ASR").unwrap();
        Self { directory, path }
    }
    fn input(&self) -> MediaInput {
        MediaInput::local(self.path.clone()).unwrap()
    }
    fn request(
        &self,
        server: &TextSource,
        format: SubtitleFormat,
        language: TextLanguage,
    ) -> PreparedRequest {
        PreparedRequest::new(self.input(), vec![])
            .unwrap()
            .with_remote_subtitle(server.input(format, language))
            .unwrap()
    }
    fn sidecar(&self, language: TextLanguage) -> LocalSubtitleInput {
        let path = self.directory.path().join("explicit-local.srt");
        fs::write(&path, body(SubtitleFormat::Srt, "软件与服务器")).unwrap();
        LocalSubtitleInput::new(path, SubtitleFormat::Srt, language).unwrap()
    }
    #[cfg(windows)]
    fn writable(&self) -> bool {
        fs::OpenOptions::new().write(true).open(&self.path).is_ok()
    }
}
struct TextSource(TcpListener, AtomicUsize);
impl TextSource {
    async fn new() -> Self {
        Self(
            TcpListener::bind("127.0.0.1:0").await.unwrap(),
            AtomicUsize::new(0),
        )
    }
    fn input(&self, format: SubtitleFormat, language: TextLanguage) -> PreparedSubtitleUrl {
        PreparedSubtitleUrl::local(
            &format!(
                "http://{}/private-subtitle?ticket=SYNTHETIC",
                self.0.local_addr().unwrap()
            ),
            format,
            language,
        )
        .unwrap()
    }
    async fn socket(&self) -> TcpStream {
        let (mut socket, _) = tokio::time::timeout(TEST_TIMEOUT, self.0.accept())
            .await
            .unwrap()
            .unwrap();
        let mut bytes = Vec::new();
        while !bytes.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            assert_eq!(socket.read(&mut byte).await.unwrap(), 1);
            bytes.push(byte[0]);
            assert!(bytes.len() <= 8192);
        }
        let request = String::from_utf8(bytes).unwrap().to_ascii_lowercase();
        assert!(request.starts_with("get /private-subtitle?ticket=synthetic http/1.1\r\n"));
        for forbidden in ["authorization:", "cookie:", "range:"] {
            assert!(!request.contains(forbidden));
        }
        self.1.fetch_add(1, Ordering::SeqCst);
        socket
    }
    async fn respond(&self, status: u16, body: &[u8]) {
        let mut socket = self.socket().await;
        socket.write_all(format!("HTTP/1.1 {status} Synthetic\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes()).await.unwrap();
        let _ = socket.write_all(body).await;
    }
    async fn blocked(&self) -> TcpStream {
        let mut socket = self.socket().await;
        socket.write_all(b"HTTP/1.1 200 Synthetic\r\nContent-Type: text/plain\r\nContent-Length: 99\r\nConnection: close\r\n\r\n1\n").await.unwrap();
        socket
    }
    async fn no_request(&self) {
        assert!(
            tokio::time::timeout(Duration::from_millis(25), self.0.accept())
                .await
                .is_err()
        );
    }
}
fn body(format: SubtitleFormat, text: &str) -> Vec<u8> {
    match format {
        SubtitleFormat::Srt => format!("1\n00:00:00,004 --> 00:00:00,016\n{text}\n"),
        SubtitleFormat::WebVtt => format!("WEBVTT\n\n00:00:00.004 --> 00:00:00.016\n{text}\n"),
    }
    .into_bytes()
}
async fn run_https(
    task: PreparedTask,
    provider: Option<&dyn PreparedTranslator>,
    quality: TranslationQuality,
) -> PreparedCompletion {
    tokio::time::timeout(
        TEST_TIMEOUT,
        task.run_with_https_subtitles(provider, quality, None),
    )
    .await
    .unwrap()
}
fn no_asr(services: &Services) {
    assert_eq!(services.asr.load(Ordering::SeqCst), 0);
    assert_eq!(services.capabilities.load(Ordering::SeqCst), 0);
}
fn assert_original(document: &PreparedDocument, language: TextLanguage, text: &str) {
    assert!(document.original().recognized().is_none());
    let original = document.original().existing().unwrap();
    assert_eq!(original.identity, document.identity());
    assert_eq!(original.source, SubtitleSource::ExistingSubtitle);
    assert!(original.language == language);
    assert_eq!(original.cues[0].start_us, Some(4_000));
    assert_eq!(original.cues[0].duration_us, Some(12_000));
    assert_eq!(original.cues[0].text.as_ref(), text);
    assert_eq!(document.target_language(), "zh-TW");
}

#[test]
fn https_prepared_three_chinese_languages_both_formats_are_localized_without_provider() {
    lane(async {
        let media = LocalMedia::new();
        for language in [
            TextLanguage::TaiwanTraditional,
            TextLanguage::OtherTraditional,
            TextLanguage::SimplifiedChinese,
        ] {
            for format in [SubtitleFormat::Srt, SubtitleFormat::WebVtt] {
                for supplied_provider in [false, true] {
                    let server = TextSource::new().await;
                    let services = Services::new();
                    let mut owner = PreparedSession::new();
                    let task = owner
                        .start(media.request(&server, format, language))
                        .unwrap();
                    let data = body(format, "软件与服务器");
                    let (_, completion) = tokio::join!(
                        server.respond(200, &data),
                        run_https(
                            task,
                            supplied_provider.then_some(&services as &dyn PreparedTranslator),
                            TranslationQuality::High
                        )
                    );
                    assert!(completion.result.is_ok(), "{:?}", completion.result.err());
                    assert!(owner.ready().is_none());
                    assert!(owner.apply(completion).unwrap());
                    let document = owner.ready().unwrap();
                    assert_original(document, language, "软件与服务器");
                    assert_eq!(document.translated()[0].as_ref(), "軟體與伺服器");
                    assert_eq!(services.counts(), (0, 0, 0));
                    no_asr(&services);
                    assert_eq!(server.1.load(Ordering::SeqCst), 1);
                    #[cfg(windows)]
                    assert!(!media.writable());
                    owner.stop();
                    #[cfg(windows)]
                    assert!(media.writable());
                    assert_eq!(fs::read_dir(media.directory.path()).unwrap().count(), 1);
                }
            }
        }
    });
}

#[test]
fn https_prepared_five_languages_three_qualities_share_analyzed_context_and_original_time() {
    lane(async {
        let media = LocalMedia::new();
        for speech in [
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
                    let server = TextSource::new().await;
                    let services = Services::new();
                    let mut owner = PreparedSession::new();
                    let language = TextLanguage::Source(speech);
                    let task = owner
                        .start(media.request(&server, format, language))
                        .unwrap();
                    let data = body(format, "synthetic source subtitle");
                    let (_, completion) = tokio::join!(
                        server.respond(200, &data),
                        run_https(task, Some(&services), quality)
                    );
                    assert!(completion.result.is_ok(), "{:?}", completion.result.err());
                    assert!(owner.apply(completion).unwrap());
                    let document = owner.ready().unwrap();
                    assert_original(document, language, "synthetic source subtitle");
                    let passes = if quality == TranslationQuality::Fast {
                        1
                    } else {
                        2
                    };
                    assert_eq!(services.counts(), (0, 1, passes));
                    no_asr(&services);
                    assert_eq!(
                        document.translated()[0].as_ref(),
                        if passes == 1 {
                            "軟體與伺服器 草稿"
                        } else {
                            "軟體與伺服器 校訂"
                        }
                    );
                    let rows = services.text_rows.lock().unwrap();
                    assert_eq!(rows.len(), passes);
                    assert!(rows.iter().all(|row| row.1 == rows[0].1));
                    owner.stop();
                }
            }
        }
    });
}

#[test]
fn https_prepared_selected_remote_text_with_remote_wave_reads_headers_without_pcm_or_asr() {
    lane(async {
        let wave_source = Source::new(wave(16_000, 1, &samples(1600, 1))).await;
        let server = TextSource::new().await;
        let request = PreparedRequest::new(wave_source.media_input(), vec![])
            .unwrap()
            .with_remote_subtitle(
                server.input(SubtitleFormat::Srt, TextLanguage::SimplifiedChinese),
            )
            .unwrap();
        let mut owner = PreparedSession::new();
        let task = owner.start(request).unwrap();
        let data = body(SubtitleFormat::Srt, "软件与服务器");
        let (_, completion) = tokio::join!(
            server.respond(200, &data),
            run_https(task, None, TranslationQuality::Standard)
        );
        assert!(owner.apply(completion).unwrap());
        assert_original(
            owner.ready().unwrap(),
            TextLanguage::SimplifiedChinese,
            "软件与服务器",
        );
        assert_eq!(owner.ready().unwrap().media_byte_len(), 3244);
        {
            let rows = wave_source.rows.lock().unwrap();
            assert!(!rows.is_empty());
            assert!(rows.iter().all(|(start, len)| start + *len as u64 <= 44));
        }
        owner.stop();
        wave_source.finish().await;
    });
}

#[test]
fn https_prepared_lower_ranked_remote_choices_are_not_fetched_when_local_or_embedded_wins() {
    lane(async {
        let media = LocalMedia::new();
        let server = TextSource::new().await;
        for embedded in [false, true] {
            let language = TextLanguage::TaiwanTraditional;
            let request = if embedded {
                fs::write(
                    &media.path,
                    crate::media::local_audio_format::mp4_prepared_fixture(language),
                )
                .unwrap();
                PreparedRequest::new(media.input(), vec![])
                    .unwrap()
                    .with_embedded_subtitle(Mp4SubtitleSelection::new(3, language).unwrap())
                    .unwrap()
            } else {
                PreparedRequest::new(media.input(), vec![media.sidecar(language)]).unwrap()
            }
            .with_remote_subtitle(server.input(
                SubtitleFormat::Srt,
                TextLanguage::Source(SpeechLanguage::English),
            ))
            .unwrap();
            let mut owner = PreparedSession::new();
            let completion = run_https(
                owner.start(request).unwrap(),
                None,
                TranslationQuality::Standard,
            )
            .await;
            assert!(completion.result.is_ok(), "{:?}", completion.result.err());
            assert!(owner.apply(completion).unwrap());
            assert!(
                owner
                    .ready()
                    .unwrap()
                    .original()
                    .existing()
                    .unwrap()
                    .language
                    == language
            );
            owner.stop();
        }
        server.no_request().await;
        assert_eq!(server.1.load(Ordering::SeqCst), 0);
    });
}

#[test]
fn https_prepared_metadata_errors_precede_network_and_any_media_pin() {
    lane(async {
        let media = LocalMedia::new();
        let server = TextSource::new().await;
        let wave_source = Source::new(wave(16_000, 1, &samples(1600, 1))).await;
        let services = Services::new();
        for scenario in 0..5 {
            let language = if scenario == 2 {
                TextLanguage::Source(SpeechLanguage::English)
            } else {
                TextLanguage::TaiwanTraditional
            };
            let mut request = PreparedRequest::new(
                if scenario == 1 {
                    wave_source.media_input()
                } else {
                    media.input()
                },
                vec![],
            )
            .unwrap();
            if scenario != 4 {
                request = request
                    .with_remote_subtitle(server.input(SubtitleFormat::Srt, language))
                    .unwrap();
            }
            if scenario == 0 {
                request = request
                    .with_remote_subtitle(server.input(SubtitleFormat::WebVtt, language))
                    .unwrap();
            }
            if scenario == 1 {
                request = request
                    .with_embedded_subtitle(
                        Mp4SubtitleSelection::new(3, TextLanguage::Source(SpeechLanguage::English))
                            .unwrap(),
                    )
                    .unwrap();
            }
            let mut owner = PreparedSession::new();
            let task = owner.start(request).unwrap();
            let old = SessionGlossary::new(StreamIdentity {
                generation: task.identity.generation + 1,
                ..task.identity
            });
            let completion = task
                .run_with_https_subtitles(
                    if scenario == 2 { None } else { Some(&services) },
                    TranslationQuality::Standard,
                    (scenario == 3).then_some(&old),
                )
                .await;
            let expected = match scenario {
                0 => MediaError::AmbiguousSubtitle,
                1 => MediaError::UnsupportedUrl,
                2 => MediaError::TranslationRequired,
                3 => MediaError::Translation(TranslationError::InvalidData),
                4 => MediaError::InvalidInput,
                _ => unreachable!(),
            };
            assert_eq!(completion.result.err(), Some(expected));
            assert!(owner.ready().is_none());
            #[cfg(windows)]
            assert!(media.writable());
        }
        assert!(wave_source.rows.lock().unwrap().is_empty());
        server.no_request().await;
        assert_eq!(services.counts(), (0, 0, 0));
        no_asr(&services);
        wave_source.finish().await;
    });
}

#[test]
fn https_prepared_remote_winner_does_not_read_missing_lower_priority_local_sidecar() {
    lane(async {
        let media = LocalMedia::new();
        let server = TextSource::new().await;
        let absent = LocalSubtitleInput::new(
            media.directory.path().join("missing.srt"),
            SubtitleFormat::Srt,
            TextLanguage::Source(SpeechLanguage::English),
        )
        .unwrap();
        let request = PreparedRequest::new(media.input(), vec![absent])
            .unwrap()
            .with_remote_subtitle(
                server.input(SubtitleFormat::Srt, TextLanguage::TaiwanTraditional),
            )
            .unwrap();
        let mut owner = PreparedSession::new();
        let data = body(SubtitleFormat::Srt, "軟體與伺服器");
        let (_, completion) = tokio::join!(
            server.respond(200, &data),
            run_https(
                owner.start(request).unwrap(),
                None,
                TranslationQuality::Standard
            )
        );
        assert!(owner.apply(completion).unwrap());
        assert_original(
            owner.ready().unwrap(),
            TextLanguage::TaiwanTraditional,
            "軟體與伺服器",
        );
        owner.stop();
    });
}

#[test]
fn https_prepared_transport_parse_and_provider_failures_do_not_fall_back_to_asr() {
    lane(async {
        let media = LocalMedia::new();
        for scenario in 0..4 {
            let server = TextSource::new().await;
            let mut services = Services::new();
            services.fail = match scenario {
                2 => Stage::Analysis,
                3 => Stage::Translation,
                _ => Stage::None,
            };
            let mut owner = PreparedSession::new();
            let task = owner
                .start(media.request(
                    &server,
                    SubtitleFormat::Srt,
                    TextLanguage::Source(SpeechLanguage::English),
                ))
                .unwrap();
            let data = if scenario == 1 {
                b"invalid synthetic subtitle".to_vec()
            } else {
                body(SubtitleFormat::Srt, "synthetic source subtitle")
            };
            let (_, completion) = tokio::join!(
                server.respond(if scenario == 0 { 404 } else { 200 }, &data),
                run_https(task, Some(&services), TranslationQuality::Standard)
            );
            let expected = match scenario {
                0 => MediaError::RemoteSubtitle(RemoteSubtitleFailure::NotFound),
                1 => MediaError::InvalidSubtitle,
                2 => MediaError::Translation(TranslationError::Refused),
                3 => MediaError::Translation(TranslationError::RateLimited),
                _ => unreachable!(),
            };
            assert_eq!(owner.apply(completion), Err(expected));
            assert!(owner.ready().is_none());
            no_asr(&services);
            assert_eq!(services.active.load(Ordering::SeqCst), 0);
            #[cfg(windows)]
            assert!(media.writable());
            for private in ["private-subtitle", "SYNTHETIC", "127.0.0.1", "ticket="] {
                assert!(!expected.to_string().contains(private));
            }
            server.no_request().await;
        }
    });
}

#[test]
fn https_prepared_unpolled_drop_and_active_stop_or_drop_release_subtitle_request_and_media() {
    lane(async {
        let media = LocalMedia::new();
        for action in 0..3 {
            let server = TextSource::new().await;
            let mut owner = PreparedSession::new();
            let task = owner
                .start(media.request(
                    &server,
                    SubtitleFormat::Srt,
                    TextLanguage::TaiwanTraditional,
                ))
                .unwrap();
            let future = task.run_with_https_subtitles(None, TranslationQuality::Standard, None);
            if action == 0 {
                drop(future);
                server.no_request().await;
            } else {
                let work = tokio::spawn(future);
                let mut socket = server.blocked().await;
                #[cfg(windows)]
                assert!(!media.writable());
                if action == 1 {
                    owner.stop();
                    let completion = tokio::time::timeout(TEST_TIMEOUT, work)
                        .await
                        .unwrap()
                        .unwrap();
                    assert_eq!(completion.result.err(), Some(MediaError::Cancelled));
                } else {
                    work.abort();
                    let Err(error) = work.await else {
                        panic!("synthetic blocked preparation completed");
                    };
                    assert!(error.is_cancelled());
                }
                let mut byte = [0];
                let closed = tokio::time::timeout(TEST_TIMEOUT, socket.read(&mut byte))
                    .await
                    .unwrap();
                assert!(matches!(closed, Ok(0) | Err(_)));
            }
            assert_eq!(owner.state(), PreparedState::Cancelled);
            assert!(owner.ready().is_none());
            #[cfg(windows)]
            assert!(media.writable());
        }
    });
}

#[test]
fn https_prepared_stop_and_future_drop_during_provider_wait_release_text_and_media() {
    lane(async {
        let media = LocalMedia::new();
        for stage in [Stage::Analysis, Stage::Translation] {
            for drop_future in [false, true] {
                let server = TextSource::new().await;
                let mut services = Services::new();
                services.gate = stage;
                let mut owner = PreparedSession::new();
                let task = owner
                    .start(media.request(
                        &server,
                        SubtitleFormat::Srt,
                        TextLanguage::Source(SpeechLanguage::English),
                    ))
                    .unwrap();
                let mut work = Box::pin(task.run_with_https_subtitles(
                    Some(&services),
                    TranslationQuality::Standard,
                    None,
                ));
                let data = body(SubtitleFormat::Srt, "synthetic source subtitle");
                tokio::join!(server.respond(200, &data), entered(&mut work, &services));
                assert_eq!(services.active.load(Ordering::SeqCst), 1);
                #[cfg(windows)]
                assert!(!media.writable());
                if drop_future {
                    drop(work);
                } else {
                    owner.stop();
                    assert_eq!(work.await.result.err(), Some(MediaError::Cancelled));
                }
                assert_eq!(services.active.load(Ordering::SeqCst), 0);
                assert!(owner.ready().is_none());
                assert_eq!(owner.state(), PreparedState::Cancelled);
                no_asr(&services);
                #[cfg(windows)]
                assert!(media.writable());
            }
        }
    });
}

async fn assert_global_subtitle_full(server: &TextSource, identity: StreamIdentity) {
    assert_eq!(
        fetch_remote_subtitle(
            server.input(SubtitleFormat::Srt, TextLanguage::TaiwanTraditional),
            identity,
            1,
            ResolverCancellation::default()
        )
        .await
        .err(),
        Some(RemoteSubtitleError::Overloaded)
    );
    server.no_request().await;
}
#[test]
fn https_prepared_subtitle_leases_remain_bounded_across_service_wait_completion_and_ready() {
    lane(async {
        let media = LocalMedia::new();
        let first = TextSource::new().await;
        let second = TextSource::new().await;
        let probe = TextSource::new().await;
        let mut first_services = Services::new();
        first_services.gate = Stage::Analysis;
        let mut second_services = Services::new();
        second_services.gate = Stage::Translation;
        let mut first_owner = PreparedSession::new();
        let mut second_owner = PreparedSession::new();
        let first_task = first_owner
            .start(media.request(
                &first,
                SubtitleFormat::Srt,
                TextLanguage::Source(SpeechLanguage::English),
            ))
            .unwrap();
        let first_identity = first_task.identity;
        let second_task = second_owner
            .start(media.request(
                &second,
                SubtitleFormat::Srt,
                TextLanguage::Source(SpeechLanguage::English),
            ))
            .unwrap();
        let mut first_work = Box::pin(first_task.run_with_https_subtitles(
            Some(&first_services),
            TranslationQuality::Standard,
            None,
        ));
        let mut second_work = Box::pin(second_task.run_with_https_subtitles(
            Some(&second_services),
            TranslationQuality::Fast,
            None,
        ));
        let data = body(SubtitleFormat::Srt, "synthetic source subtitle");
        tokio::join!(
            first.respond(200, &data),
            entered(&mut first_work, &first_services)
        );
        tokio::join!(
            second.respond(200, &data),
            entered(&mut second_work, &second_services)
        );
        assert_global_subtitle_full(&probe, first_identity).await;
        first_services.release.notify_one();
        let completion = first_work.await;
        assert!(completion.result.is_ok(), "{:?}", completion.result.err());
        assert!(first_owner.ready().is_none());
        assert_global_subtitle_full(&probe, first_identity).await;
        assert!(first_owner.apply(completion).unwrap());
        assert_global_subtitle_full(&probe, first_identity).await;
        // Model the private text-only reference held by an admitted CPU job.
        // Ready removal must not return its HTTP quota ahead of real cleanup.
        let retained_text = first_owner.ready().unwrap().body.clone();
        first_owner.stop();
        assert_global_subtitle_full(&probe, first_identity).await;
        drop(retained_text);
        let probe_work = fetch_remote_subtitle(
            probe.input(SubtitleFormat::Srt, TextLanguage::TaiwanTraditional),
            first_identity,
            2,
            ResolverCancellation::default(),
        );
        let (_, probe_result) = tokio::join!(probe.respond(200, &data), probe_work);
        drop(probe_result.unwrap());
        second_owner.stop();
        assert_eq!(second_work.await.result.err(), Some(MediaError::Cancelled));
        assert_eq!(second_services.active.load(Ordering::SeqCst), 0);
        no_asr(&first_services);
        no_asr(&second_services);
        #[cfg(windows)]
        assert!(media.writable());
    });
}

#[test]
fn https_prepared_old_completion_is_rejected_without_replacing_new_session_ready() {
    lane(async {
        let media = LocalMedia::new();
        let server = TextSource::new().await;
        let mut owner = PreparedSession::new();
        let data = body(SubtitleFormat::Srt, "软件与服务器");
        let old_task = owner
            .start(media.request(
                &server,
                SubtitleFormat::Srt,
                TextLanguage::SimplifiedChinese,
            ))
            .unwrap();
        let old_identity = old_task.identity;
        let (_, old) = tokio::join!(
            server.respond(200, &data),
            run_https(old_task, None, TranslationQuality::Standard)
        );
        assert!(old.result.is_ok());
        let new_task = owner
            .start(media.request(
                &server,
                SubtitleFormat::Srt,
                TextLanguage::TaiwanTraditional,
            ))
            .unwrap();
        let current = new_task.identity;
        assert_ne!(current.session, old_identity.session);
        let (_, completion) = tokio::join!(
            server.respond(200, &data),
            run_https(new_task, None, TranslationQuality::Standard)
        );
        assert!(owner.apply(completion).unwrap());
        assert_eq!(owner.apply(old), Err(MediaError::Stale));
        assert_eq!(owner.ready().unwrap().identity(), current);
        assert_original(
            owner.ready().unwrap(),
            TextLanguage::TaiwanTraditional,
            "软件与服务器",
        );
        owner.stop();
    });
}

#[test]
fn https_prepared_old_task_entries_reject_any_remote_choice_with_ambiguity_priority() {
    lane(async {
        let media = LocalMedia::new();
        let server = TextSource::new().await;
        let services = Services::new();
        for ambiguous in [false, true] {
            for entry in 0..7 {
                let choices = if ambiguous {
                    vec![]
                } else {
                    vec![media.sidecar(TextLanguage::TaiwanTraditional)]
                };
                let mut request = PreparedRequest::new(media.input(), choices)
                    .unwrap()
                    .with_remote_subtitle(server.input(
                        SubtitleFormat::Srt,
                        TextLanguage::Source(SpeechLanguage::English),
                    ))
                    .unwrap();
                if ambiguous {
                    request = request
                        .with_remote_subtitle(server.input(
                            SubtitleFormat::WebVtt,
                            TextLanguage::Source(SpeechLanguage::Japanese),
                        ))
                        .unwrap();
                }
                let mut owner = PreparedSession::new();
                let task = owner.start(request).unwrap();
                let work: Pin<Box<dyn Future<Output = PreparedCompletion> + '_>> = match entry {
                    0 => Box::pin(task.run()),
                    1 => Box::pin(task.run_with_translator(
                        &services,
                        TranslationQuality::Standard,
                        None,
                    )),
                    2 => Box::pin(task.run_with_analyzed_translator(
                        &services,
                        TranslationQuality::Standard,
                        None,
                    )),
                    3 => Box::pin(task.run_with_recognizer(
                        &services,
                        &services,
                        SourceLanguage::English,
                        PreparedSampleRate::Hz16000,
                        TranslationQuality::Standard,
                        None,
                    )),
                    4 => Box::pin(task.run_with_analyzed_recognizer(
                        &services,
                        &services,
                        SourceLanguage::English,
                        PreparedSampleRate::Hz16000,
                        TranslationQuality::Standard,
                        None,
                    )),
                    5 => Box::pin(task.run_with_remote_wave_subtitle(
                        Some(&services),
                        TranslationQuality::Standard,
                        None,
                    )),
                    6 => Box::pin(task.run_with_remote_wave_recognizer(
                        &services,
                        &services,
                        SourceLanguage::English,
                        PreparedSampleRate::Hz16000,
                        TranslationQuality::Standard,
                        None,
                    )),
                    _ => unreachable!(),
                };
                let completion = tokio::time::timeout(TEST_TIMEOUT, work).await.unwrap();
                assert_eq!(
                    completion.result.err(),
                    Some(if ambiguous {
                        MediaError::AmbiguousSubtitle
                    } else {
                        MediaError::UnsupportedUrl
                    })
                );
                assert!(owner.ready().is_none());
            }
        }
        assert_eq!(services.counts(), (0, 0, 0));
        no_asr(&services);
        server.no_request().await;
    });
}
