//! Self-owned TCP loopback and synthetic bytes. No external URL/media/key access.
use super::*;
use crate::audio::capture::SessionId;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
#[path = "remote_http_lifecycle_tests.rs"]
mod lifecycle;

fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(934),
        generation: 2,
    }
}
fn input(address: std::net::SocketAddr) -> MediaInput {
    MediaInput::remote(&format!(
        "http://{address}/synthetic.data?token=SYNTHETIC_PRIVATE"
    ))
    .unwrap()
}
fn lane() -> Arc<Semaphore> {
    Arc::new(Semaphore::new(MAX_REMOTE_MEDIA_OWNERS))
}
fn buffer_lane() -> Arc<Semaphore> {
    Arc::new(Semaphore::new(MAX_REMOTE_OUTSTANDING_RANGES))
}
async fn open(
    address: std::net::SocketAddr,
    cancel: ResolverCancellation,
    owners: Arc<Semaphore>,
) -> Result<RemoteHttpMedia, RemoteMediaError> {
    RemoteHttpMedia::open_with(
        input(address),
        identity(),
        cancel,
        owners,
        buffer_lane(),
        Duration::from_secs(3),
    )
    .await
}
async fn request(socket: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 1024];
    while !bytes.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
        let count = socket.read(&mut chunk).await.unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&chunk[..count]);
        assert!(bytes.len() <= 8192);
    }
    String::from_utf8(bytes).unwrap()
}
async fn reply(socket: &mut TcpStream, range: &str, etag: &str, bytes: &[u8]) {
    let header = format!(
        "HTTP/1.1 206 Partial Content\r\nContent-Range: {range}\r\nETag: {etag}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        bytes.len()
    );
    socket.write_all(header.as_bytes()).await.unwrap();
    for chunk in bytes.chunks(7) {
        socket.write_all(chunk).await.unwrap();
    }
}
fn headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_RANGE,
        HeaderValue::from_static("bytes 0-0/100"),
    );
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from_static("1"));
    headers.insert(header::ETAG, HeaderValue::from_static("\"v1\""));
    headers
}
#[test]
fn remote_http_headers_reject_weak_missing_duplicate_tags_and_invalid_ranges() {
    assert!(validate_headers(206, &headers(), (0, 1), None).is_ok());
    for name in [header::CONTENT_RANGE, header::CONTENT_LENGTH, header::ETAG] {
        let mut missing = headers();
        missing.remove(&name);
        assert!(validate_headers(206, &missing, (0, 1), None).is_err());
        let mut duplicate = headers();
        duplicate.append(name.clone(), headers()[&name].clone());
        assert!(validate_headers(206, &duplicate, (0, 1), None).is_err());
    }
    for tag in [
        "W/\"v1\"",
        "*",
        "v1",
        "\"bad\"tag\"",
        "\"bad\u{7f}\"",
        &format!("\"{}\"", "x".repeat(MAX_ETAG_BYTES)),
    ] {
        let mut changed = headers();
        if let Ok(value) = HeaderValue::from_str(tag) {
            changed.insert(header::ETAG, value);
            assert!(validate_headers(206, &changed, (0, 1), None).is_err());
        }
    }
    for range in [
        "bytes */100",
        "bytes 0-1/100",
        "bytes 1-1/100",
        "bytes 0-0/*",
        "bytes 0-0/0",
        "bytes 1-0/100",
        "bytes +0-0/100",
        "bytes 0-0/18446744073709551616",
        "bytes 0-0/100, 1-1/100",
        "items 0-0/100",
    ] {
        let mut changed = headers();
        changed.insert(header::CONTENT_RANGE, HeaderValue::from_str(range).unwrap());
        assert!(validate_headers(206, &changed, (0, 1), None).is_err());
    }
    for (name, value) in [
        (header::CONTENT_ENCODING, "gzip"),
        (header::TRANSFER_ENCODING, "chunked"),
        (header::CONTENT_TYPE, "multipart/byteranges; boundary=x"),
        (header::CONTENT_LENGTH, "2"),
    ] {
        let mut changed = headers();
        changed.insert(name, HeaderValue::from_static(value));
        assert!(validate_headers(206, &changed, (0, 1), None).is_err());
    }
}
#[test]
fn remote_http_headers_enforce_total_limit_version_and_status_types() {
    let original = validate_headers(206, &headers(), (0, 1), None).unwrap();
    for (name, value) in [
        (header::ETAG, "\"v2\""),
        (header::CONTENT_RANGE, "bytes 0-0/101"),
    ] {
        let mut changed = headers();
        changed.insert(name, HeaderValue::from_static(value));
        assert!(matches!(
            validate_headers(206, &changed, (0, 1), Some(&original)),
            Err(RemoteMediaError::SourceChanged)
        ));
    }
    for (status, error) in [
        (200, RemoteMediaError::UnsupportedRange),
        (302, RemoteMediaError::Redirected),
        (304, RemoteMediaError::UnsupportedRange),
        (401, RemoteMediaError::AccessDenied),
        (412, RemoteMediaError::SourceChanged),
        (416, RemoteMediaError::SourceChanged),
        (429, RemoteMediaError::RateLimited),
        (503, RemoteMediaError::Network),
    ] {
        assert!(
            matches!(validate_headers(status,&headers(),(0,1),Some(&original)),Err(actual) if actual==error)
        );
    }
    let mut exact = headers();
    exact.insert(
        header::CONTENT_RANGE,
        HeaderValue::from_str(&format!("bytes 0-0/{MAX_REMOTE_MEDIA_BYTES}")).unwrap(),
    );
    assert!(validate_headers(206, &exact, (0, 1), None).is_ok());
    exact.insert(
        header::CONTENT_RANGE,
        HeaderValue::from_str(&format!("bytes 0-0/{}", MAX_REMOTE_MEDIA_BYTES + 1)).unwrap(),
    );
    assert!(matches!(
        validate_headers(206, &exact, (0, 1), None),
        Err(RemoteMediaError::TooLarge)
    ));
}
#[tokio::test]
async fn remote_http_reads_exact_ranges_and_revokes_delivered_bytes_when_owner_drops() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        for (start, end) in [(0, 0), (3, 9), (97, 99)] {
            let (mut socket, _) = listener.accept().await.unwrap();
            let text = request(&mut socket).await;
            let lower = text.to_ascii_lowercase();
            assert!(lower.contains(&format!("range: bytes={start}-{end}\r\n")));
            assert!(lower.contains("accept-encoding: identity\r\n"));
            assert!(lower.contains("cache-control: no-store\r\n"));
            assert!(!lower.contains("authorization:"));
            assert!(!lower.contains("cookie:"));
            assert!(!lower.contains("referer:"));
            if start > 0 {
                assert!(lower.contains("if-match: \"v1\"\r\n"));
            } else {
                assert!(!lower.contains("if-match:"));
            }
            let bytes: Vec<u8> = (start..=end).map(|value| value as u8).collect();
            reply(
                &mut socket,
                &format!("bytes {start}-{end}/100"),
                "\"v1\"",
                &bytes,
            )
            .await;
        }
    });
    let cancel = ResolverCancellation::default();
    let owners = lane();
    let mut media = open(address, cancel.clone(), owners.clone()).await.unwrap();
    assert_eq!(media.byte_len(), 100);
    assert_eq!(media.identity(), identity());
    assert_eq!(owners.available_permits(), 1);
    let first = media
        .read_range(RemoteRangeRequest::new(identity(), 1, 3, 7).unwrap())
        .await
        .unwrap();
    assert_eq!(first.bytes().unwrap(), [3, 4, 5, 6, 7, 8, 9]);
    assert_eq!(first.offset(), 3);
    assert_eq!(first.sequence(), 1);
    assert_eq!(first.identity(), identity());
    let last = media
        .read_range(RemoteRangeRequest::new(identity(), 2, 97, 3).unwrap())
        .await
        .unwrap();
    assert_eq!(last.bytes().unwrap(), [97, 98, 99]);
    drop(media);
    assert_eq!(owners.available_permits(), 2);
    assert!(cancel.is_cancelled());
    assert_eq!(first.bytes().err(), Some(RemoteMediaError::Cancelled));
    assert_eq!(last.bytes().err(), Some(RemoteMediaError::Cancelled));
    server.await.unwrap();
}
