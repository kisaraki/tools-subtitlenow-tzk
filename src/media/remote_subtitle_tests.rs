//! Synthetic whole-document HTTP fixtures; no user content or cloud service.
use super::*;
use crate::{
    ai::speech::SpeechLanguage, audio::capture::SessionId, resolver::contracts::SubtitleSource,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

const FIXTURE_TIMEOUT: Duration = Duration::from_secs(5);
const ORIGINAL: &str = "1\n00:00:05,000 --> 00:00:06,250\nsynthetic source sentence\n";
const CHINESE: &str = "WEBVTT\n\n00:00:05.000 --> 00:00:06.250\n软件与服务器字幕\n";

fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(997),
        generation: 2,
    }
}
fn language() -> TextLanguage {
    TextLanguage::Source(SpeechLanguage::English)
}
async fn listener() -> TcpListener {
    TcpListener::bind("127.0.0.1:0").await.unwrap()
}
fn input(
    server: &TcpListener,
    format: SubtitleFormat,
    language: TextLanguage,
) -> PreparedSubtitleUrl {
    PreparedSubtitleUrl::local(
        &format!(
            "http://{}/private?ticket=synthetic-secret",
            server.local_addr().unwrap()
        ),
        format,
        language,
    )
    .unwrap()
}
fn policy() -> FetchPolicy {
    FetchPolicy {
        cpu: Arc::new(Semaphore::new(1)),
        budget: Arc::new(Semaphore::new(2)),
        timeout: FIXTURE_TIMEOUT,
        before_parse: None,
    }
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
        let mut byte = [0];
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
    let observed = request(&mut socket).await;
    let headers = format!(
        "HTTP/1.1 {status} Fixture\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    socket.write_all(headers.as_bytes()).await.unwrap();
    let _ = socket.write_all(body).await;
    observed
}
async fn no_request(server: &TcpListener) {
    assert!(
        tokio::time::timeout(Duration::from_millis(25), server.accept())
            .await
            .is_err()
    );
}
async fn closed(socket: &mut TcpStream) {
    let mut byte = [0];
    let result = tokio::time::timeout(FIXTURE_TIMEOUT, socket.read(&mut byte))
        .await
        .unwrap();
    assert!(matches!(result, Ok(0) | Err(_)));
}
async fn roundtrip(
    status: u16,
    headers: &str,
    body: &[u8],
    format: SubtitleFormat,
    language: TextLanguage,
) -> Result<PreparedRemoteSubtitle, RemoteSubtitleError> {
    let server = listener().await;
    let (observed, result) = tokio::join!(
        serve(&server, status, headers, body),
        fetch_with(
            input(&server, format, language),
            identity(),
            3,
            ResolverCancellation::default(),
            policy()
        )
    );
    let lower = observed.to_ascii_lowercase();
    assert!(lower.contains("accept-encoding: identity\r\n"));
    assert!(!lower.contains("authorization:"));
    assert!(!lower.contains("cookie:"));
    assert!(!lower.contains("range:"));
    result
}
fn error(result: Result<PreparedRemoteSubtitle, RemoteSubtitleError>) -> RemoteSubtitleError {
    result.err().unwrap()
}

#[test]
fn explicit_prepared_locations_are_https_only_private_and_passive() {
    for raw in [
        "https://example.invalid/no-extension",
        "HTTPS://example.invalid/cues?ticket=synthetic-secret",
    ] {
        assert!(PreparedSubtitleUrl::new(raw, SubtitleFormat::Srt, language()).is_ok());
    }
    for raw in [
        "",
        "http://example.invalid/a",
        "file:///a.srt",
        "https:///a.srt",
        "https:example.invalid/a",
        "https:////example.invalid/a",
        "https://@example.invalid/a",
        "https://user:secret@example.invalid/a",
        "https://example.invalid/a#private",
        "https://example.invalid/a b",
        " https://example.invalid/a",
        "https://example.invalid/a\n",
        "https://example.invalid\\a",
    ] {
        assert!(
            matches!(
                PreparedSubtitleUrl::new(raw, SubtitleFormat::Srt, language()),
                Err(RemoteSubtitleError::InvalidRequest)
            ),
            "{raw:?}"
        );
    }
    let large = format!(
        "https://example.invalid/{}",
        "a".repeat(super::super::input::MAX_PREPARED_URL_BYTES)
    );
    assert!(PreparedSubtitleUrl::new(&large, SubtitleFormat::Srt, language()).is_err());
    for raw in [
        "http://localhost/a",
        "http://127.0.0.2/a",
        "https://127.0.0.1/a",
    ] {
        assert!(PreparedSubtitleUrl::local(raw, SubtitleFormat::Srt, language()).is_err());
    }
}

#[tokio::test(flavor = "current_thread")]
async fn declared_formats_languages_and_genuine_session_timing_survive_without_inference() {
    for language in [
        TextLanguage::TaiwanTraditional,
        TextLanguage::OtherTraditional,
        TextLanguage::SimplifiedChinese,
        language(),
        TextLanguage::Source(SpeechLanguage::Japanese),
        TextLanguage::Source(SpeechLanguage::French),
        TextLanguage::Source(SpeechLanguage::German),
        TextLanguage::Source(SpeechLanguage::Korean),
    ] {
        for (format, mime, body) in [
            (SubtitleFormat::Srt, "application/x-subrip", ORIGINAL),
            (SubtitleFormat::WebVtt, "text/vtt", CHINESE),
        ] {
            let result = roundtrip(
                200,
                &format!("Content-Type: {mime}; charset=\"UTF-8\"\r\n"),
                body.as_bytes(),
                format,
                language,
            )
            .await
            .unwrap();
            assert_eq!(result.identity(), identity());
            assert_eq!(result.sequence(), 3);
            let original = result.original().unwrap();
            assert_eq!(original.identity, identity());
            assert!(original.language == language);
            assert_eq!(original.source, SubtitleSource::ExistingSubtitle);
            assert_eq!(&*original.content_id, "prepared:997:2:3");
            assert_eq!(original.cues[0].start_us, Some(5_000_000));
            assert_eq!(original.cues[0].duration_us, Some(1_250_000));
            assert_eq!(
                &*original.cues[0].text,
                if format == SubtitleFormat::Srt {
                    "synthetic source sentence"
                } else {
                    "软件与服务器字幕"
                }
            );
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn selected_http_failures_are_typed_without_fallback_or_private_payload() {
    for (status, expected) in [
        (401, RemoteSubtitleError::AccessDenied),
        (403, RemoteSubtitleError::AccessDenied),
        (404, RemoteSubtitleError::NotFound),
        (429, RemoteSubtitleError::RateLimited),
        (302, RemoteSubtitleError::Redirected),
        (500, RemoteSubtitleError::Network),
        (206, RemoteSubtitleError::Network),
    ] {
        let found = error(
            roundtrip(
                status,
                "Location: http://127.0.0.1:1/private\r\nContent-Type: text/plain\r\n",
                ORIGINAL.as_bytes(),
                SubtitleFormat::Srt,
                language(),
            )
            .await,
        );
        assert_eq!(found, expected);
        assert!(!format!("{found:?} {found}").contains("synthetic-secret"));
        assert!(!format!("{found:?} {found}").contains("127.0.0.1"));
    }
}

#[test]
fn response_headers_require_explicit_declared_mime_utf8_and_unambiguous_framing() {
    let valid = |mime: &str| {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE, mime.parse().unwrap());
        headers
    };
    for mime in [
        "text/plain",
        "application/x-subrip",
        "TEXT/PLAIN; CHARSET=utf-8",
        "text/plain; charset=\"UTF-8\"",
    ] {
        let mut headers = valid(mime);
        assert_eq!(response_length(&headers, SubtitleFormat::Srt), Ok(None));
        headers.insert(header::CONTENT_LENGTH, "0".parse().unwrap());
        assert_eq!(response_length(&headers, SubtitleFormat::Srt), Ok(Some(0)));
        headers.insert(
            header::CONTENT_LENGTH,
            MAX_PREPARED_DOCUMENT_BYTES.to_string().parse().unwrap(),
        );
        assert_eq!(
            response_length(&headers, SubtitleFormat::Srt),
            Ok(Some(MAX_PREPARED_DOCUMENT_BYTES))
        );
    }
    for mime in [
        "text/vtt",
        "application/octet-stream",
        "text/html",
        "text/plain; charset=latin1",
        "text/plain; unknown=utf-8",
        "text/plain; charset=utf-8; charset=utf-8",
        "text/plain;",
    ] {
        assert_eq!(
            response_length(&valid(mime), SubtitleFormat::Srt),
            Err(RemoteSubtitleError::InvalidResponse)
        );
    }
    let mut headers = HeaderMap::new();
    assert!(response_length(&headers, SubtitleFormat::Srt).is_err());
    headers = valid("text/plain");
    headers.append(header::CONTENT_TYPE, "text/plain".parse().unwrap());
    assert!(response_length(&headers, SubtitleFormat::Srt).is_err());
    for encoding in ["gzip", "br", "identity, identity"] {
        let mut headers = valid("text/plain");
        headers.insert(header::CONTENT_ENCODING, encoding.parse().unwrap());
        assert!(response_length(&headers, SubtitleFormat::Srt).is_err());
    }
    for length in ["+5", "5,5", "-1", "18446744073709551616"] {
        let mut headers = valid("text/plain");
        headers.insert(header::CONTENT_LENGTH, length.parse().unwrap());
        assert!(response_length(&headers, SubtitleFormat::Srt).is_err());
    }
    let mut headers = valid("text/plain");
    headers.insert(
        header::CONTENT_LENGTH,
        (MAX_PREPARED_DOCUMENT_BYTES + 1)
            .to_string()
            .parse()
            .unwrap(),
    );
    assert_eq!(
        response_length(&headers, SubtitleFormat::Srt),
        Err(RemoteSubtitleError::TooLarge)
    );
    for transfer in ["gzip", "chunked, chunked"] {
        let mut headers = valid("text/plain");
        headers.insert(header::TRANSFER_ENCODING, transfer.parse().unwrap());
        assert!(response_length(&headers, SubtitleFormat::Srt).is_err());
    }
    let mut headers = valid("text/plain");
    headers.insert(header::TRANSFER_ENCODING, "chunked".parse().unwrap());
    assert_eq!(response_length(&headers, SubtitleFormat::Srt), Ok(None));
    headers.insert(header::CONTENT_LENGTH, "5".parse().unwrap());
    assert_eq!(
        response_length(&headers, SubtitleFormat::Srt),
        Err(RemoteSubtitleError::InvalidResponse)
    );
    let mut headers = valid("text/plain");
    headers.append(header::CONTENT_LENGTH, "5".parse().unwrap());
    headers.append(header::CONTENT_LENGTH, "5".parse().unwrap());
    assert_eq!(
        response_length(&headers, SubtitleFormat::Srt),
        Err(RemoteSubtitleError::InvalidResponse)
    );
}

#[tokio::test(flavor = "current_thread")]
async fn chunked_and_connection_close_documents_have_bounded_real_message_completion() {
    for chunked in [false, true] {
        let server = listener().await;
        let fixture = async {
            let mut socket = accept(&server).await;
            request(&mut socket).await;
            let framing = if chunked {
                "Transfer-Encoding: chunked\r\n"
            } else {
                ""
            };
            socket.write_all(format!("HTTP/1.1 200 Fixture\r\nContent-Type: text/plain\r\n{framing}Connection: close\r\n\r\n").as_bytes()).await.unwrap();
            if chunked {
                socket
                    .write_all(
                        format!("{:x}\r\n{ORIGINAL}\r\n0\r\n\r\n", ORIGINAL.len()).as_bytes(),
                    )
                    .await
                    .unwrap();
            } else {
                socket.write_all(ORIGINAL.as_bytes()).await.unwrap();
            }
        };
        let (_, result) = tokio::join!(
            fixture,
            fetch_with(
                input(&server, SubtitleFormat::Srt, language()),
                identity(),
                1,
                ResolverCancellation::default(),
                policy()
            )
        );
        assert_eq!(result.unwrap().original().unwrap().cues.len(), 1);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn malformed_declared_format_utf8_markup_and_empty_content_never_succeed() {
    for body in [
        b"".as_slice(),
        b"\xff",
        b"<html>private</html>",
        b"1\n00:00:05,000 --> 00:00:06,000\n<i>markup</i>\n",
        b"1\n00:00:06,000 --> 00:00:05,000\nwrong\n",
        CHINESE.as_bytes(),
    ] {
        let found = error(
            roundtrip(
                200,
                "Content-Type: text/plain\r\n",
                body,
                SubtitleFormat::Srt,
                language(),
            )
            .await,
        );
        assert!(matches!(
            found,
            RemoteSubtitleError::Media(
                MediaError::InvalidSubtitle | MediaError::UnsupportedSubtitle
            )
        ));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn raw_document_budget_accepts_exact_limit_and_rejects_declared_or_actual_excess() {
    // Exercise the wire-byte boundary with blank lines, without turning this
    // capacity fixture into eight million per-line cancellation checks under
    // its unchanged five-second HTTP/parse deadline.
    const PADDING_LINE_BYTES: usize = 4 * 1024;
    let mut body = ORIGINAL.as_bytes().to_vec();
    body.resize(MAX_PREPARED_DOCUMENT_BYTES, b' ');
    for padding in body[ORIGINAL.len()..].chunks_mut(PADDING_LINE_BYTES) {
        padding[0] = b'\n';
    }
    assert_eq!(
        roundtrip(
            200,
            "Content-Type: text/plain\r\n",
            &body,
            SubtitleFormat::Srt,
            language()
        )
        .await
        .unwrap()
        .original()
        .unwrap()
        .cues
        .len(),
        1
    );
    body.push(b'\n');
    assert_eq!(
        error(
            roundtrip(
                200,
                "Content-Type: text/plain\r\n",
                &body,
                SubtitleFormat::Srt,
                language()
            )
            .await
        ),
        RemoteSubtitleError::TooLarge
    );
    let server = listener().await;
    let fixture = async {
        let mut socket = accept(&server).await;
        request(&mut socket).await;
        socket
            .write_all(
                b"HTTP/1.1 200 Fixture\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n",
            )
            .await
            .unwrap();
        let _ = socket.write_all(&body).await;
    };
    let (_, result) = tokio::join!(
        fixture,
        fetch_with(
            input(&server, SubtitleFormat::Srt, language()),
            identity(),
            1,
            ResolverCancellation::default(),
            policy()
        )
    );
    assert_eq!(error(result), RemoteSubtitleError::TooLarge);
}

#[tokio::test(flavor = "current_thread")]
async fn decoded_cue_and_text_limits_remain_independent_of_wire_budget() {
    fn cues(count: usize, width: usize, extra_first: bool) -> Vec<u8> {
        let mut document = String::new();
        for index in 1..=count {
            let text = "x".repeat(width + usize::from(extra_first && index == 1));
            document.push_str(&format!(
                "{index}\n00:00:05,000 --> 00:00:06,000\n{text}\n\n"
            ));
        }
        document.into_bytes()
    }
    let max_cues = crate::resolver::contracts::MAX_TEXT_CUES;
    let width = crate::resolver::contracts::MAX_TEXT_BYTES / max_cues;
    let body = cues(max_cues, width, false);
    let result = roundtrip(
        200,
        "Content-Type: text/plain\r\n",
        &body,
        SubtitleFormat::Srt,
        language(),
    )
    .await
    .unwrap();
    assert_eq!(result.original().unwrap().cues.len(), max_cues);
    assert_eq!(
        result
            .original()
            .unwrap()
            .cues
            .iter()
            .map(|cue| cue.text.len())
            .sum::<usize>(),
        crate::resolver::contracts::MAX_TEXT_BYTES
    );
    drop(result);
    for body in [cues(max_cues + 1, 1, false), cues(max_cues, width, true)] {
        assert_eq!(
            error(
                roundtrip(
                    200,
                    "Content-Type: text/plain\r\n",
                    &body,
                    SubtitleFormat::Srt,
                    language()
                )
                .await
            ),
            RemoteSubtitleError::Media(MediaError::InvalidSubtitle)
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn truncated_content_length_and_chunked_framing_fail_without_partial_document() {
    for chunked in [false, true] {
        let server = listener().await;
        let fixture = async {
            let mut socket = accept(&server).await;
            request(&mut socket).await;
            let framing = if chunked {
                "Transfer-Encoding: chunked".to_owned()
            } else {
                format!("Content-Length: {}", ORIGINAL.len() + 5)
            };
            socket.write_all(format!("HTTP/1.1 200 Fixture\r\nContent-Type: text/plain\r\n{framing}\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
            if chunked {
                socket
                    .write_all(format!("{:x}\r\n{ORIGINAL}", ORIGINAL.len()).as_bytes())
                    .await
                    .unwrap();
            } else {
                socket.write_all(ORIGINAL.as_bytes()).await.unwrap();
            }
        };
        let (_, result) = tokio::join!(
            fixture,
            fetch_with(
                input(&server, SubtitleFormat::Srt, language()),
                identity(),
                1,
                ResolverCancellation::default(),
                policy()
            )
        );
        assert!(matches!(
            error(result),
            RemoteSubtitleError::Network | RemoteSubtitleError::InvalidResponse
        ));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn tls_failure_is_safe_and_never_retried_as_plain_http() {
    let server = listener().await;
    let url = PreparedSubtitleUrl::new(
        &format!(
            "https://{}/private?ticket=synthetic-secret",
            server.local_addr().unwrap()
        ),
        SubtitleFormat::Srt,
        language(),
    )
    .unwrap();
    let fixture = async {
        let mut socket = accept(&server).await;
        let mut byte = [0];
        assert_eq!(socket.read(&mut byte).await.unwrap(), 1);
        assert_eq!(byte[0], 0x16);
        drop(socket);
    };
    let (_, result) = tokio::join!(
        fixture,
        fetch_with(
            url,
            identity(),
            1,
            ResolverCancellation::default(),
            policy()
        )
    );
    assert_eq!(error(result), RemoteSubtitleError::Network);
    no_request(&server).await;
}

#[path = "remote_subtitle_lifecycle_tests.rs"]
mod lifecycle;
