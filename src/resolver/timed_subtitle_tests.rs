//! Synthetic HTTP on literal loopback only; no real media, keys, files or service.
use super::*;
use crate::{
    ai::{
        speech::{SpeechLanguage, TranscriptEvent, TranscriptKind},
        translation::{
            TranslationCancellation, TranslationCapabilities, TranslationEmitter,
            TranslationFuture, TranslationRequest, Translator,
        },
    },
    audio::capture::SessionId,
    domain::TranslationQuality,
    resolver::text_session::{ExistingTextLease, ExistingTextSession, TextLookupStatus},
    session::{live_text::LiveTextPipeline, live_translation::LiveTranslation},
    subtitle::source_arbiter::PlaybackObservation,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

const FIXTURE_TIMEOUT: Duration = Duration::from_secs(2);
const ORIGINAL: &str = "1\n00:00:05,000 --> 00:00:06,000\nsynthetic source sentence\n";
const CHINESE: &str = "WEBVTT\n\n00:00:05.000 --> 00:00:06.000\n软件与服务器字幕\n";

fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(997),
        generation: 2,
    }
}
fn query() -> TextQuery {
    TextQuery {
        stamp: WindowStamp {
            identity: identity(),
            sequence: 1,
            ended_at: Instant::now(),
            start_us: Some(40_000_000),
            duration_us: 12_000_000,
        },
        source: SubtitleSource::ExistingSubtitle,
        content_id: "synthetic subtitle work".into(),
    }
}
async fn listener() -> TcpListener {
    TcpListener::bind("127.0.0.1:0").await.unwrap()
}
fn track(
    server: &TcpListener,
    path: &str,
    format: SubtitleFormat,
    language: TextLanguage,
) -> SubtitleTrack {
    SubtitleTrack::local(
        &format!("http://{}{path}", server.local_addr().unwrap()),
        format,
        language,
    )
    .unwrap()
}
fn provider(server: &TcpListener, paired: bool) -> TimedSubtitleProvider {
    let primary = if paired {
        track(
            server,
            "/primary",
            SubtitleFormat::WebVtt,
            TextLanguage::SimplifiedChinese,
        )
    } else {
        track(
            server,
            "/primary?ticket=synthetic",
            SubtitleFormat::Srt,
            TextLanguage::Source(SpeechLanguage::English),
        )
    };
    let reference = paired.then(|| {
        ReferenceTrack::new(
            track(
                server,
                "/reference",
                SubtitleFormat::Srt,
                TextLanguage::Source(SpeechLanguage::English),
            ),
            "synthetic edition 1".into(),
            "synthetic edition 1".into(),
        )
        .unwrap()
    });
    let mut value = TimedSubtitleProvider::new(
        identity(),
        "synthetic subtitle work".into(),
        primary,
        reference,
    )
    .unwrap();
    value.parser_workers = Arc::new(Semaphore::new(1));
    value
}
async fn accept(server: &TcpListener) -> TcpStream {
    tokio::time::timeout(FIXTURE_TIMEOUT, server.accept())
        .await
        .unwrap()
        .unwrap()
        .0
}
async fn request(socket: &mut TcpStream) -> String {
    tokio::time::timeout(FIXTURE_TIMEOUT, async {
        let mut bytes = Vec::new();
        let mut byte = [0; 1];
        while !bytes.ends_with(b"\r\n\r\n") {
            assert!(bytes.len() < 8192);
            assert_eq!(socket.read(&mut byte).await.unwrap(), 1);
            bytes.push(byte[0]);
        }
        String::from_utf8(bytes).unwrap()
    })
    .await
    .unwrap()
}
async fn serve(server: &TcpListener, status: u16, headers: &str, body: &[u8]) -> String {
    let mut socket = accept(server).await;
    let request = request(&mut socket).await;
    let headers = format!(
        "HTTP/1.1 {status} Fixture\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    socket.write_all(headers.as_bytes()).await.unwrap();
    // Rejected status, MIME or length may close before the body is consumed.
    let _ = socket.write_all(body).await;
    request
}
async fn closed(socket: &mut TcpStream) {
    let mut byte = [0; 1];
    let result = tokio::time::timeout(FIXTURE_TIMEOUT, socket.read(&mut byte))
        .await
        .unwrap();
    assert!(matches!(result, Ok(0) | Err(_)));
}
async fn no_request(server: &TcpListener) {
    assert!(
        tokio::time::timeout(Duration::from_millis(20), server.accept())
            .await
            .is_err()
    );
}
async fn roundtrip(
    status: u16,
    headers: &str,
    body: &[u8],
) -> Result<Option<ExistingTextBundle>, ResolverError> {
    let server = listener().await;
    let value = provider(&server, false);
    let input = query();
    let (_, result) = tokio::join!(
        serve(&server, status, headers, body),
        value.lookup_bundle(&input, ResolverCancellation::default())
    );
    result
}

#[test]
fn explicit_configuration_accepts_only_secure_urls_and_consistent_reference_editions() {
    let english = TextLanguage::Source(SpeechLanguage::English);
    for url in [
        "https://example.invalid/subtitle.srt",
        "https://example.invalid/subtitle.srt?ticket=synthetic",
    ] {
        assert!(SubtitleTrack::new(url, SubtitleFormat::Srt, english).is_ok());
    }
    for url in [
        "",
        "http://example.invalid/subtitle.srt",
        "file:///subtitle.srt",
        "https:///",
        "https://user:password@example.invalid/subtitle.srt",
        "https://@example.invalid/subtitle.srt",
        "https:example.invalid/subtitle.srt",
        "https:////example.invalid/subtitle.srt",
        "https://example.invalid\\subtitle.srt",
        "https://example.invalid/subtitle.srt#fragment",
        "https://example.invalid/\nsubtitle.srt",
    ] {
        let error = SubtitleTrack::new(url, SubtitleFormat::Srt, english)
            .err()
            .unwrap();
        assert_eq!(error, ResolverError::InvalidData);
        if !url.is_empty() {
            assert!(!error.to_string().contains(url));
        }
    }
    let too_long = format!("https://example.invalid/{}", "a".repeat(4096));
    assert!(SubtitleTrack::new(&too_long, SubtitleFormat::Srt, english).is_err());
    let source = || {
        SubtitleTrack::new(
            "https://example.invalid/source.srt",
            SubtitleFormat::Srt,
            english,
        )
        .unwrap()
    };
    for (first, second) in [("", ""), ("edition 1", "edition 2"), ("bad\n", "bad\n")] {
        assert!(ReferenceTrack::new(source(), first.into(), second.into()).is_err());
    }
    let edition = "e".repeat(1025);
    assert!(ReferenceTrack::new(source(), edition.clone().into(), edition.into()).is_err());
    let chinese = SubtitleTrack::new(
        "https://example.invalid/chinese.vtt",
        SubtitleFormat::WebVtt,
        TextLanguage::TaiwanTraditional,
    )
    .unwrap();
    assert!(ReferenceTrack::new(chinese, "same".into(), "same".into()).is_err());
    let reference = ReferenceTrack::new(source(), "same".into(), "same".into()).unwrap();
    assert!(
        TimedSubtitleProvider::new(identity(), "work".into(), source(), Some(reference)).is_err()
    );
    assert!(TimedSubtitleProvider::new(identity(), "".into(), source(), None).is_err());
    for url in [
        "http://localhost:1234/a",
        "http://127.0.0.2:1234/a",
        "http://[::1]:1234/a",
    ] {
        assert!(SubtitleTrack::local(url, SubtitleFormat::Srt, english).is_err());
    }
}

#[tokio::test]
async fn actual_get_preserves_original_timing_and_query_identity_without_extra_private_data() {
    let server = listener().await;
    let value = provider(&server, false);
    assert!(value.supports(SubtitleSource::ExistingSubtitle));
    assert!(!value.supports(SubtitleSource::ExistingLyrics));
    let input = query();
    let (request, result) = tokio::join!(
        serve(
            &server,
            200,
            "Content-Type: application/x-subrip; charset=UTF-8\r\n",
            ORIGINAL.as_bytes()
        ),
        value.lookup(&input, ResolverCancellation::default())
    );
    assert!(request.starts_with("GET /primary?ticket=synthetic HTTP/1.1\r\n"));
    assert!(!request.to_lowercase().contains("authorization"));
    assert!(!request.to_lowercase().contains("cookie"));
    assert!(!request.contains("synthetic subtitle work"));
    let text = result.unwrap().unwrap();
    text.validate(&input).unwrap();
    assert_eq!(text.identity, identity());
    assert_eq!(text.source, SubtitleSource::ExistingSubtitle);
    assert_eq!(text.cues.len(), 1);
    assert_eq!(text.cues[0].start_us, Some(5_000_000));
    assert_eq!(text.cues[0].duration_us, Some(1_000_000));
    assert_eq!(text.cues[0].text.as_ref(), "synthetic source sentence");
    no_request(&server).await;
}

struct UnusedTranslator(AtomicUsize);
impl Translator for UnusedTranslator {
    fn capabilities(&self) -> TranslationCapabilities {
        TranslationCapabilities {
            streaming: true,
            cancellation: true,
            supported_languages: [true; 5],
            max_concurrent: 1,
        }
    }
    fn translate<'a>(
        &'a self,
        _: &'a TranslationRequest,
        _: &'a mut TranslationEmitter,
        _: TranslationCancellation,
    ) -> TranslationFuture<'a> {
        Box::pin(async move {
            self.0.fetch_add(1, Ordering::SeqCst);
            panic!("Chinese subtitle localization must not call a cloud translator")
        })
    }
}

#[tokio::test]
async fn actual_paired_http_publication_reaches_session_alignment_localization_and_shared_arbiter()
{
    let server = listener().await;
    let value = Arc::new(provider(&server, true));
    let initial = query();
    let stamp = initial.stamp;
    let (mut owner, task) = ExistingTextSession::new(identity());
    owner.select(initial, value).unwrap();
    let job = tokio::spawn(task.run());
    let remote = async {
        let first = serve(
            &server,
            200,
            "Content-Type: text/vtt; charset=utf-8\r\n",
            CHINESE.as_bytes(),
        )
        .await;
        let second = serve(
            &server,
            200,
            "Content-Type: application/x-subrip\r\n",
            ORIGINAL.as_bytes(),
        )
        .await;
        assert!(first.starts_with("GET /primary HTTP/1.1\r\n"));
        assert!(second.starts_with("GET /reference HTTP/1.1\r\n"));
    };
    let session = async {
        tokio::time::timeout(FIXTURE_TIMEOUT, async {
            loop {
                let update = owner.next().await.unwrap();
                match update.status {
                    TextLookupStatus::Available => {
                        break ExistingTextLease::from_update(&update).unwrap();
                    }
                    TextLookupStatus::Failed(error) => {
                        panic!("synthetic subtitle failed: {error:?}")
                    }
                    _ => {}
                }
            }
        })
        .await
        .unwrap()
    };
    let ((), mut input) = tokio::join!(remote, session);
    input.renew(stamp, Confidence::new(900).unwrap()).unwrap();
    let reference = input.source_reference().unwrap();
    assert_eq!(reference.primary_edition_id, reference.reference_edition_id);
    assert_eq!(
        reference.text.cues[0].text.as_ref(),
        "synthetic source sentence"
    );
    assert_eq!(input.query().stamp.sequence, stamp.sequence);
    assert_eq!(input.query().stamp.ended_at, stamp.ended_at);
    owner.stop().unwrap();
    job.await.unwrap().unwrap();
    let translator = UnusedTranslator(AtomicUsize::new(0));
    let mut flow = LiveTranslation::new(identity(), TranslationQuality::High, &translator).unwrap();
    let source = TranscriptEvent {
        identity: identity(),
        sequence: 1,
        utterance: 1,
        revision: 1,
        kind: TranscriptKind::Final,
        language: Some(SpeechLanguage::English),
        start_us: Some(42_000_000),
        duration_us: Some(950_000),
        text: "synthetic source sentence".into(),
    };
    flow.ingest(source.clone()).unwrap();
    let (mut pipeline, task) = LiveTextPipeline::new(identity(), TranslationQuality::High);
    let version = flow.sources_mut().version();
    assert!(
        pipeline
            .offer(Some(input.clone()), Some(&source), version)
            .unwrap()
    );
    let worker = task.isolated().run(&translator);
    tokio::pin!(worker);
    let completed = tokio::time::timeout(FIXTURE_TIMEOUT, async {
        loop {
            tokio::select! {
                result = &mut worker => panic!("synthetic pipeline ended: {result:?}"),
                done = pipeline.changed() => if let Some(done) = done { break done; }
            }
        }
    })
    .await
    .unwrap();
    assert!(
        pipeline
            .apply(
                completed,
                &mut flow,
                Some(PlaybackObservation {
                    version,
                    sequence: 100,
                    observed_at: Instant::now(),
                    capture_position_us: 42_500_000,
                })
            )
            .unwrap()
    );
    assert_eq!(flow.timeline().items().len(), 1);
    assert_eq!(
        flow.timeline().items()[0].display_translation(),
        "軟體與伺服器字幕"
    );
    assert_eq!(
        flow.timeline().items()[0].display_origin(),
        SubtitleSource::ExistingSubtitle
    );
    input.retire();
    assert_eq!(
        flow.timeline().items()[0].display_origin(),
        SubtitleSource::LiveAi
    );
    assert_eq!(translator.0.load(Ordering::SeqCst), 0);
    pipeline.stop();
    worker.await.unwrap();
    no_request(&server).await;
}

#[tokio::test]
async fn wrong_work_source_session_generation_or_stale_query_never_opens_transport() {
    let server = listener().await;
    let value = provider(&server, false);
    for case in 0..5 {
        let mut input = query();
        match case {
            0 => input.content_id = "other work".into(),
            1 => input.source = SubtitleSource::ExistingLyrics,
            2 => input.stamp.identity.session = SessionId(998),
            3 => input.stamp.identity.generation += 1,
            _ => input.stamp.ended_at -= Duration::from_secs(6),
        }
        assert!(
            value
                .lookup_bundle(&input, ResolverCancellation::default())
                .await
                .is_err()
        );
    }
    no_request(&server).await;
    assert_eq!(value.requests.available_permits(), 1);
}

#[tokio::test]
async fn http_status_mapping_rejects_redirects_and_never_follows_to_another_endpoint() {
    let target = listener().await;
    let headers = format!(
        "Location: http://{}/redirected\r\n",
        target.local_addr().unwrap()
    );
    for (status, expected) in [
        (401, ResolverError::Credential),
        (403, ResolverError::Credential),
        (302, ResolverError::Network),
        (307, ResolverError::Network),
        (500, ResolverError::Network),
    ] {
        let error = roundtrip(status, &headers, b"SYNTHETIC_PRIVATE_ERROR")
            .await
            .err()
            .unwrap();
        assert_eq!(error, expected);
        assert!(!error.to_string().contains("SYNTHETIC_PRIVATE_ERROR"));
    }
    assert!(roundtrip(404, "", b"missing").await.unwrap().is_none());
    no_request(&target).await;
}

#[tokio::test]
async fn missing_or_invalid_reference_rejects_the_whole_successful_primary_publication() {
    for (status, body, expected) in [
        (404, "missing", ResolverError::ContentMismatch),
        (200, "not a subtitle document", ResolverError::InvalidData),
    ] {
        let server = listener().await;
        let value = provider(&server, true);
        let input = query();
        let remote = async {
            serve(
                &server,
                200,
                "Content-Type: text/vtt\r\n",
                CHINESE.as_bytes(),
            )
            .await;
            serve(
                &server,
                status,
                "Content-Type: application/x-subrip\r\n",
                body.as_bytes(),
            )
            .await;
        };
        let ((), result) = tokio::join!(
            remote,
            value.lookup_bundle(&input, ResolverCancellation::default())
        );
        assert_eq!(result.err().unwrap(), expected);
        assert_eq!(value.requests.available_permits(), 1);
    }
}

#[tokio::test]
async fn only_supported_document_mime_and_utf8_are_accepted_without_lossy_decoding() {
    for content_type in [
        "application/x-subrip",
        "Text/Plain; charset=\"UTF-8\"",
        "text/plain; charset=utf-8",
    ] {
        let headers = format!("Content-Type: {content_type}\r\n");
        assert!(
            roundtrip(200, &headers, ORIGINAL.as_bytes())
                .await
                .unwrap()
                .is_some()
        );
    }
    for content_type in [
        "text/html",
        "application/json",
        "text/plain; charset=big5",
        "text/plain; charset=iso-8859-1",
        "text/plain; charset=\"utf-8",
        "text/plain; charset=utf-8\"",
        "text/plain; charset=\"\"utf-8\"\"",
    ] {
        let headers = format!("Content-Type: {content_type}\r\n");
        assert!(matches!(
            roundtrip(200, &headers, ORIGINAL.as_bytes()).await,
            Err(ResolverError::InvalidData)
        ));
    }
    assert!(matches!(
        roundtrip(200, "Content-Type: text/plain\r\n", &[0xff, 0xfe, 0xfd]).await,
        Err(ResolverError::InvalidData)
    ));
    assert!(matches!(
        roundtrip(
            200,
            "Content-Type: text/plain\r\nContent-Encoding: gzip\r\n",
            ORIGINAL.as_bytes()
        )
        .await,
        Err(ResolverError::InvalidData)
    ));
}

fn numbered_srt(count: usize, text_bytes: usize) -> String {
    (0..count)
        .map(|index| {
            format!(
                "{}\n00:00:{:02},000 --> 00:00:{:02},000\n{}\n\n",
                index + 1,
                index % 59,
                index % 59 + 1,
                "x".repeat(text_bytes)
            )
        })
        .collect()
}
fn long_srt(count: usize, text_bytes: usize) -> String {
    (0..count).map(|index| {
        let minute = index / 60;
        let second = index % 60;
        let end_minute = (index + 1) / 60;
        let end_second = (index + 1) % 60;
        format!("{}\n00:{minute:02}:{second:02},000 --> 00:{end_minute:02}:{end_second:02},000\n{}\n\n", index + 1, "x".repeat(text_bytes))
    }).collect()
}

#[tokio::test]
async fn document_length_chunk_stream_and_combined_parsed_budgets_are_bounded() {
    let oversized = vec![b'x'; MAX_SUBTITLE_DOCUMENT_BYTES + 1];
    assert!(matches!(
        roundtrip(200, "Content-Type: text/plain\r\n", &oversized).await,
        Err(ResolverError::InvalidData)
    ));
    let server = listener().await;
    let value = provider(&server, false);
    let input = query();
    let remote = async {
        let mut socket = accept(&server).await;
        request(&mut socket).await;
        socket.write_all(b"HTTP/1.1 200 Fixture\r\nContent-Type: text/plain\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").await.unwrap();
        let chunk = vec![b'x'; MAX_SUBTITLE_DOCUMENT_BYTES / 2 + 1];
        let header = format!("{:x}\r\n", chunk.len());
        for _ in 0..2 {
            let _ = socket.write_all(header.as_bytes()).await;
            let _ = socket.write_all(&chunk).await;
            let _ = socket.write_all(b"\r\n").await;
        }
        let _ = socket.write_all(b"0\r\n\r\n").await;
    };
    let ((), result) = tokio::join!(
        remote,
        value.lookup_bundle(&input, ResolverCancellation::default())
    );
    assert_eq!(result.err().unwrap(), ResolverError::InvalidData);
    for (count, text_bytes) in [(513, 1), (17, 4096)] {
        let server = listener().await;
        let mut value = provider(&server, true);
        value.primary = track(
            &server,
            "/primary",
            SubtitleFormat::Srt,
            TextLanguage::TaiwanTraditional,
        );
        let body = if count == 513 {
            long_srt(count, text_bytes)
        } else {
            numbered_srt(count, text_bytes)
        };
        let input = query();
        let remote = async {
            serve(
                &server,
                200,
                "Content-Type: application/x-subrip\r\n",
                body.as_bytes(),
            )
            .await;
            serve(
                &server,
                200,
                "Content-Type: application/x-subrip\r\n",
                body.as_bytes(),
            )
            .await;
        };
        let ((), result) = tokio::join!(
            remote,
            value.lookup_bundle(&input, ResolverCancellation::default())
        );
        assert_eq!(result.err().unwrap(), ResolverError::InvalidData);
    }
}

#[tokio::test]
async fn cancellation_before_primary_during_primary_and_during_reference_releases_transport() {
    let server = listener().await;
    let value = provider(&server, false);
    let input = query();
    let token = ResolverCancellation::default();
    token.cancel();
    assert!(matches!(
        value.lookup_bundle(&input, token).await,
        Err(ResolverError::Cancelled)
    ));
    no_request(&server).await;
    for paired in [false, true] {
        let server = listener().await;
        let value = provider(&server, paired);
        let input = query();
        let token = ResolverCancellation::default();
        let (ready, notified) = tokio::sync::oneshot::channel();
        let remote = async {
            if paired {
                serve(
                    &server,
                    200,
                    "Content-Type: text/vtt\r\n",
                    CHINESE.as_bytes(),
                )
                .await;
            }
            let mut socket = accept(&server).await;
            request(&mut socket).await;
            ready.send(()).unwrap();
            closed(&mut socket).await;
        };
        let client = async {
            let mut job = Box::pin(value.lookup_bundle(&input, token.clone()));
            tokio::select! { _ = notified => {}, result = &mut job => panic!("early result: {:?}", result.err()) }
            token.cancel();
            assert!(matches!(job.await, Err(ResolverError::Cancelled)));
            assert_eq!(value.requests.available_permits(), 1);
        };
        tokio::join!(remote, client);
    }
}

#[tokio::test]
async fn dropping_lookup_future_closes_incomplete_body_and_releases_request_capacity() {
    let server = listener().await;
    let value = provider(&server, false);
    let input = query();
    let (ready, notified) = tokio::sync::oneshot::channel();
    let remote = async {
        let mut socket = accept(&server).await;
        request(&mut socket).await;
        socket.write_all(b"HTTP/1.1 200 Fixture\r\nContent-Type: text/plain\r\nContent-Length: 1000\r\n\r\n1\n").await.unwrap();
        ready.send(()).unwrap();
        closed(&mut socket).await;
    };
    let client = async {
        let mut job = Box::pin(value.lookup_bundle(&input, ResolverCancellation::default()));
        tokio::select! { _ = notified => {}, result = &mut job => panic!("early result: {:?}", result.err()) }
        drop(job);
        assert_eq!(value.requests.available_permits(), 1);
    };
    tokio::join!(remote, client);
}

#[tokio::test]
async fn overlapping_queries_are_rejected_without_a_request_backlog() {
    let server = listener().await;
    let value = provider(&server, false);
    let input = query();
    let token = ResolverCancellation::default();
    let (ready, notified) = tokio::sync::oneshot::channel();
    let remote = async {
        let mut socket = accept(&server).await;
        request(&mut socket).await;
        ready.send(()).unwrap();
        closed(&mut socket).await;
    };
    let client = async {
        let mut job = Box::pin(value.lookup_bundle(&input, token.clone()));
        tokio::select! { _ = notified => {}, result = &mut job => panic!("early result: {:?}", result.err()) }
        for sequence in 2..10 {
            let mut next = query();
            next.stamp.sequence = sequence;
            assert!(matches!(
                value
                    .lookup_bundle(&next, ResolverCancellation::default())
                    .await,
                Err(ResolverError::TextOverloaded)
            ));
        }
        token.cancel();
        assert!(matches!(job.await, Err(ResolverError::Cancelled)));
        assert_eq!(value.requests.available_permits(), 1);
    };
    tokio::join!(remote, client);
    no_request(&server).await;
    let server = listener().await;
    let value = provider(&server, false);
    let parser_permit = value.parser_workers.acquire().await.unwrap();
    let input = query();
    let (_, result) = tokio::join!(
        serve(
            &server,
            200,
            "Content-Type: text/plain\r\n",
            ORIGINAL.as_bytes()
        ),
        value.lookup_bundle(&input, ResolverCancellation::default())
    );
    assert_eq!(result.err().unwrap(), ResolverError::TextOverloaded);
    assert_eq!(value.requests.available_permits(), 1);
    drop(parser_permit);
}

#[tokio::test]
async fn original_query_deadline_bounds_both_tracks_and_cooldown_blocks_fresh_retries() {
    let server = listener().await;
    let value = provider(&server, true);
    let mut input = query();
    input.stamp.ended_at -= Duration::from_millis(3500);
    let started = Instant::now();
    let remote = async {
        serve(
            &server,
            200,
            "Content-Type: text/vtt\r\n",
            CHINESE.as_bytes(),
        )
        .await;
        let mut socket = accept(&server).await;
        request(&mut socket).await;
        closed(&mut socket).await;
    };
    let ((), result) = tokio::join!(
        remote,
        value.lookup_bundle(&input, ResolverCancellation::default())
    );
    assert!(matches!(result, Err(ResolverError::Stale)));
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(value.requests.available_permits(), 1);
    for header in [
        "Retry-After: 120\r\n".to_owned(),
        format!(
            "Retry-After: {}\r\n",
            httpdate::fmt_http_date(SystemTime::now() + Duration::from_secs(120))
        ),
        "Retry-After: invalid\r\n".to_owned(),
    ] {
        let server = listener().await;
        let value = provider(&server, false);
        let input = query();
        let (_, result) = tokio::join!(
            serve(&server, 429, &header, b"synthetic throttling"),
            value.lookup_bundle(&input, ResolverCancellation::default())
        );
        assert_eq!(result.err().unwrap(), ResolverError::RateLimited);
        {
            let gate = value.rate.lock().unwrap();
            assert!(gate.denied || gate.next > Instant::now() + Duration::from_secs(1));
        }
        assert!(matches!(
            value
                .lookup_bundle(&query(), ResolverCancellation::default())
                .await,
            Err(ResolverError::RateLimited)
        ));
        no_request(&server).await;
    }
}
