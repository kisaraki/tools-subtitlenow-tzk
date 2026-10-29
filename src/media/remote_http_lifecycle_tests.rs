use super::*;

#[test]
fn remote_http_request_limits_identity_and_error_messages_are_bounded() {
    let mut zero = identity();
    zero.session = SessionId(0);
    let mut generation = identity();
    generation.generation = 0;
    for (id, sequence, offset, length) in [
        (zero, 1, 0, 1),
        (generation, 1, 0, 1),
        (identity(), 0, 0, 1),
        (identity(), MAX_REMOTE_RANGE_REQUESTS + 1, 0, 1),
        (identity(), 1, 0, 0),
        (identity(), 1, 0, MAX_REMOTE_RANGE_BYTES + 1),
        (identity(), 1, u64::MAX, 1),
    ] {
        assert!(matches!(
            RemoteRangeRequest::new(id, sequence, offset, length),
            Err(RemoteMediaError::InvalidRequest)
        ));
    }
    assert!(
        RemoteRangeRequest::new(
            identity(),
            MAX_REMOTE_RANGE_REQUESTS,
            0,
            MAX_REMOTE_RANGE_BYTES
        )
        .is_ok()
    );
    for error in [
        RemoteMediaError::Network,
        RemoteMediaError::InvalidResponse,
        RemoteMediaError::SourceChanged,
        RemoteMediaError::AccessDenied,
        RemoteMediaError::Redirected,
    ] {
        assert!(!format!("{error} {error:?}").contains("SYNTHETIC_PRIVATE"));
    }
    assert!(RemoteMediaError::Timeout.retryable());
    assert!(RemoteMediaError::RateLimited.retryable());
    assert!(!RemoteMediaError::SourceChanged.retryable());

    // An invalid executor context is a typed failure, before any connection.
    let cancel = ResolverCancellation::default();
    let future = RemoteHttpMedia::open(
        MediaInput::remote("http://127.0.0.1:9/synthetic").unwrap(),
        identity(),
        cancel.clone(),
    );
    let mut future = std::pin::pin!(future);
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    assert!(matches!(
        future.as_mut().poll(&mut context),
        std::task::Poll::Ready(Err(RemoteMediaError::RuntimeUnavailable))
    ));
    assert!(cancel.is_cancelled());
}

#[tokio::test]
async fn remote_http_changed_source_retires_old_bytes_and_fresh_session_reads_new_version() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        for (index, (range, tag, bytes)) in [
            ("bytes 0-0/100", "\"v1\"", &b"a"[..]),
            ("bytes 1-2/100", "\"v1\"", &b"bc"[..]),
            ("bytes 3-4/100", "\"v2\"", &b"XY"[..]),
            ("bytes 0-0/100", "\"v2\"", &b"X"[..]),
            ("bytes 1-2/100", "\"v2\"", &b"YZ"[..]),
        ]
        .into_iter()
        .enumerate()
        {
            let (mut socket, _) = listener.accept().await.unwrap();
            let text = request(&mut socket).await.to_ascii_lowercase();
            if matches!(index, 1 | 2) {
                assert!(text.contains("if-match: \"v1\"\r\n"));
            }
            if index == 4 {
                assert!(text.contains("if-match: \"v2\"\r\n"));
            }
            reply(&mut socket, range, tag, bytes).await;
        }
    });
    let owners = lane();
    let cancel = ResolverCancellation::default();
    let mut media = open(address, cancel.clone(), owners.clone()).await.unwrap();
    let old = media
        .read_range(RemoteRangeRequest::new(identity(), 1, 1, 2).unwrap())
        .await
        .unwrap();
    assert_eq!(old.bytes().unwrap(), b"bc");
    assert!(matches!(
        media
            .read_range(RemoteRangeRequest::new(identity(), 2, 3, 2).unwrap())
            .await,
        Err(RemoteMediaError::SourceChanged)
    ));
    assert!(cancel.is_cancelled());
    assert_eq!(old.bytes().err(), Some(RemoteMediaError::Cancelled));
    drop(media);
    let fresh = StreamIdentity {
        generation: 3,
        ..identity()
    };
    let mut media = RemoteHttpMedia::open_with(
        input(address),
        fresh,
        ResolverCancellation::default(),
        owners.clone(),
        buffer_lane(),
        Duration::from_secs(3),
    )
    .await
    .unwrap();
    let bytes = media
        .read_range(RemoteRangeRequest::new(fresh, 1, 1, 2).unwrap())
        .await
        .unwrap();
    assert_eq!(bytes.bytes().unwrap(), b"YZ");
    assert_eq!(bytes.identity(), fresh);
    assert_eq!(old.bytes().err(), Some(RemoteMediaError::Cancelled));
    drop(media);
    assert_eq!(owners.available_permits(), 2);
    server.await.unwrap();
}

#[tokio::test]
async fn remote_http_redirects_and_ignored_ranges_never_fall_back_or_contact_location() {
    let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target_address = target.local_addr().unwrap();
    for (status, error) in [
        (302, RemoteMediaError::Redirected),
        (200, RemoteMediaError::UnsupportedRange),
        (401, RemoteMediaError::AccessDenied),
        (429, RemoteMediaError::RateLimited),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            request(&mut socket).await;
            socket.write_all(format!("HTTP/1.1 {status} Synthetic\r\nLocation: http://{target_address}/secret\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
        });
        let owners = lane();
        let cancel = ResolverCancellation::default();
        assert!(
            matches!(open(address,cancel.clone(),owners.clone()).await,Err(actual) if actual==error)
        );
        assert_eq!(owners.available_permits(), 2);
        assert!(cancel.is_cancelled());
        server.await.unwrap();
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(20), target.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn remote_http_bad_range_response_or_truncated_body_never_delivers_partial_bytes() {
    for response in [
        "HTTP/1.1 412 Changed\r\nContent-Length: 0\r\n\r\n",
        "HTTP/1.1 416 Changed\r\nContent-Length: 0\r\n\r\n",
        "HTTP/1.1 206 Partial\r\nContent-Range: bytes 2-3/100\r\nETag: \"v1\"\r\nContent-Length: 2\r\n\r\ncd",
        "HTTP/1.1 206 Partial\r\nContent-Range: bytes 1-2/100\r\nETag: \"v1\"\r\nContent-Length: 2\r\nContent-Encoding: gzip\r\n\r\nbc",
        "HTTP/1.1 206 Partial\r\nContent-Range: bytes 1-2/100\r\nETag: \"v1\"\r\nContent-Length: 2\r\n\r\nb",
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            request(&mut socket).await;
            reply(&mut socket, "bytes 0-0/100", "\"v1\"", b"a").await;
            drop(socket);
            let (mut socket, _) = listener.accept().await.unwrap();
            request(&mut socket).await;
            socket.write_all(response.as_bytes()).await.unwrap();
        });
        let cancel = ResolverCancellation::default();
        let mut media = open(address, cancel.clone(), lane()).await.unwrap();
        assert!(
            media
                .read_range(RemoteRangeRequest::new(identity(), 1, 1, 2).unwrap())
                .await
                .is_err()
        );
        assert!(cancel.is_cancelled());
        assert!(matches!(
            media
                .read_range(RemoteRangeRequest::new(identity(), 1, 1, 2).unwrap())
                .await,
            Err(RemoteMediaError::Cancelled)
        ));
        drop(media);
        server.await.unwrap();
    }
}

#[tokio::test]
async fn remote_http_unpolled_open_overload_and_cancel_make_no_network_requests() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let owners = lane();
    let cancel = ResolverCancellation::default();
    let future = RemoteHttpMedia::open_with(
        input(address),
        identity(),
        cancel.clone(),
        owners.clone(),
        buffer_lane(),
        Duration::from_secs(3),
    );
    drop(future);
    assert!(cancel.is_cancelled());
    assert_eq!(owners.available_permits(), 2);
    let occupied = owners.clone().try_acquire_many_owned(2).unwrap();
    assert!(matches!(
        open(address, ResolverCancellation::default(), owners.clone()).await,
        Err(RemoteMediaError::Overloaded)
    ));
    drop(occupied);
    let cancel = ResolverCancellation::default();
    cancel.cancel();
    assert!(matches!(
        open(address, cancel, owners).await,
        Err(RemoteMediaError::Cancelled)
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(20), listener.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn remote_http_unpolled_read_revokes_previous_ranges_without_another_request() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        for range in ["bytes 0-0/100", "bytes 1-1/100"] {
            let (mut socket, _) = listener.accept().await.unwrap();
            request(&mut socket).await;
            reply(&mut socket, range, "\"v1\"", b"x").await;
        }
    });
    let owners = lane();
    let cancel = ResolverCancellation::default();
    let mut media = open(address, cancel.clone(), owners.clone()).await.unwrap();
    let old = media
        .read_range(RemoteRangeRequest::new(identity(), 1, 1, 1).unwrap())
        .await
        .unwrap();
    let future = media.read_range(RemoteRangeRequest::new(identity(), 2, 2, 1).unwrap());
    drop(future);
    assert!(cancel.is_cancelled());
    assert_eq!(old.bytes().err(), Some(RemoteMediaError::Cancelled));
    assert_eq!(owners.available_permits(), 1);
    drop(media);
    assert_eq!(owners.available_permits(), 2);
    server.await.unwrap();
}

async fn stalled(listener: TcpListener, ready: tokio::sync::oneshot::Sender<()>) {
    let (mut socket, _) = listener.accept().await.unwrap();
    request(&mut socket).await;
    reply(&mut socket, "bytes 0-0/100", "\"v1\"", b"a").await;
    drop(socket);
    let (mut socket, _) = listener.accept().await.unwrap();
    request(&mut socket).await;
    socket.write_all(b"HTTP/1.1 206 Partial\r\nContent-Range: bytes 1-2/100\r\nETag: \"v1\"\r\nContent-Length: 2\r\n\r\n").await.unwrap();
    let _ = ready.send(());
    let closed = tokio::time::timeout(Duration::from_secs(3), socket.read(&mut [0u8; 1]))
        .await
        .unwrap();
    assert!(matches!(closed, Ok(0) | Err(_)));
}

#[tokio::test]
async fn remote_http_inflight_cancel_closes_response_and_reclaims_owner_capacity() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (ready, received) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(stalled(listener, ready));
    let owners = lane();
    let cancel = ResolverCancellation::default();
    let mut media = open(address, cancel.clone(), owners.clone()).await.unwrap();
    let (result, ()) = tokio::join!(
        media.read_range(RemoteRangeRequest::new(identity(), 1, 1, 2).unwrap()),
        async {
            received.await.unwrap();
            cancel.cancel();
        }
    );
    assert!(matches!(result, Err(RemoteMediaError::Cancelled)));
    assert_eq!(
        media.buffers.available_permits(),
        MAX_REMOTE_OUTSTANDING_RANGES
    );
    drop(media);
    assert_eq!(owners.available_permits(), 2);
    server.await.unwrap();
}

#[tokio::test]
async fn remote_http_drop_of_inflight_future_cancels_source_and_closes_connection() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (ready, received) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(stalled(listener, ready));
    let cancel = ResolverCancellation::default();
    let mut media = open(address, cancel.clone(), lane()).await.unwrap();
    let mut work =
        Box::pin(media.read_range(RemoteRangeRequest::new(identity(), 1, 1, 2).unwrap()));
    tokio::select! {_=received=>{}, _=&mut work=>panic!("synthetic body unexpectedly completed")};
    drop(work);
    assert_eq!(
        media.buffers.available_permits(),
        MAX_REMOTE_OUTSTANDING_RANGES
    );
    assert!(cancel.is_cancelled());
    drop(media);
    server.await.unwrap();
}

#[tokio::test]
async fn remote_http_deadline_retires_stalled_response_without_retry() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (ready, _received) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(stalled(listener, ready));
    let cancel = ResolverCancellation::default();
    let owners = lane();
    let mut media = RemoteHttpMedia::open_with(
        input(address),
        identity(),
        cancel.clone(),
        owners.clone(),
        buffer_lane(),
        Duration::from_millis(100),
    )
    .await
    .unwrap();
    assert!(matches!(
        media
            .read_range(RemoteRangeRequest::new(identity(), 1, 1, 2).unwrap())
            .await,
        Err(RemoteMediaError::Timeout)
    ));
    assert_eq!(
        media.buffers.available_permits(),
        MAX_REMOTE_OUTSTANDING_RANGES
    );
    assert!(cancel.is_cancelled());
    drop(media);
    assert_eq!(owners.available_permits(), 2);
    server.await.unwrap();
}

#[tokio::test]
async fn remote_http_foreign_session_sequence_and_out_of_file_ranges_fail_before_io() {
    let mut foreign = identity();
    foreign.generation += 1;
    for (id, sequence, offset, length, error) in [
        (foreign, 1, 1, 1, RemoteMediaError::Stale),
        (identity(), 2, 1, 1, RemoteMediaError::Stale),
        (identity(), 1, 100, 1, RemoteMediaError::InvalidRequest),
        (identity(), 1, 99, 2, RemoteMediaError::InvalidRequest),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            request(&mut socket).await;
            reply(&mut socket, "bytes 0-0/100", "\"v1\"", b"a").await;
        });
        let cancel = ResolverCancellation::default();
        let mut media = open(address, cancel.clone(), lane()).await.unwrap();
        assert!(
            matches!(media.read_range(RemoteRangeRequest::new(id,sequence,offset,length).unwrap()).await,Err(actual) if actual==error)
        );
        assert!(cancel.is_cancelled());
        drop(media);
        server.await.unwrap();
    }
}

#[test]
fn remote_http_public_entry_reads_maximum_range_over_one_persistent_connection() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        for length in [1, MAX_REMOTE_RANGE_BYTES] {
            request(&mut socket).await;
            let header = format!(
                "HTTP/1.1 206 Partial\r\nContent-Range: bytes 0-{}/{MAX_REMOTE_RANGE_BYTES}\r\nETag: \"v1\"\r\nContent-Length: {length}\r\nConnection: keep-alive\r\n\r\n",
                length - 1
            );
            socket.write_all(header.as_bytes()).await.unwrap();
            socket.write_all(&vec![123; length]).await.unwrap();
        }
    });
    let mut media =
        RemoteHttpMedia::open(input(address), identity(), ResolverCancellation::default())
            .await
            .unwrap();
    let bytes = media
        .read_range(RemoteRangeRequest::new(identity(), 1, 0, MAX_REMOTE_RANGE_BYTES).unwrap())
        .await
        .unwrap();
    assert_eq!(bytes.bytes().unwrap().len(), MAX_REMOTE_RANGE_BYTES);
    assert!(bytes.bytes().unwrap().iter().all(|byte| *byte == 123));
    drop(media);
    server.await.unwrap();
        });
}

#[tokio::test]
async fn remote_http_https_never_falls_back_to_plain_http_or_leaks_query_in_errors() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = [0u8; 2048];
        let count = socket.read(&mut bytes).await.unwrap();
        assert!(count > 0);
        assert_eq!(bytes[0], 0x16); // TLS handshake record, not an HTTP GET.
        assert!(
            !bytes[..count]
                .windows(b"SYNTHETIC_PRIVATE".len())
                .any(|part| part == b"SYNTHETIC_PRIVATE")
        );
    });
    let input = MediaInput::remote(&format!(
        "https://{address}/private?token=SYNTHETIC_PRIVATE"
    ))
    .unwrap();
    let result = RemoteHttpMedia::open_with(
        input,
        identity(),
        ResolverCancellation::default(),
        lane(),
        buffer_lane(),
        Duration::from_secs(3),
    )
    .await;
    assert!(matches!(result, Err(RemoteMediaError::Network)));
    server.await.unwrap();
}

#[tokio::test]
async fn remote_http_slow_consumer_cannot_retain_unbounded_payload_across_retired_sources() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        for offset in 0..=MAX_REMOTE_OUTSTANDING_RANGES {
            let (mut socket, _) = listener.accept().await.unwrap();
            request(&mut socket).await;
            reply(
                &mut socket,
                &format!("bytes {offset}-{offset}/100"),
                "\"v1\"",
                b"x",
            )
            .await;
        }
    });
    let buffers = buffer_lane();
    let owners = lane();
    let cancel = ResolverCancellation::default();
    let mut media = RemoteHttpMedia::open_with(
        input(address),
        identity(),
        cancel.clone(),
        owners.clone(),
        buffers.clone(),
        Duration::from_secs(3),
    )
    .await
    .unwrap();
    let mut held = Vec::new();
    for offset in 1..=MAX_REMOTE_OUTSTANDING_RANGES {
        held.push(
            media
                .read_range(
                    RemoteRangeRequest::new(identity(), offset as u64, offset as u64, 1).unwrap(),
                )
                .await
                .unwrap(),
        );
    }
    assert_eq!(buffers.available_permits(), 0);
    assert!(matches!(
        media
            .read_range(RemoteRangeRequest::new(identity(), 5, 5, 1).unwrap())
            .await,
        Err(RemoteMediaError::Overloaded)
    ));
    assert!(cancel.is_cancelled());
    assert!(held.iter().all(|range| range.bytes().is_err()));
    drop(media);
    assert_eq!(owners.available_permits(), 2);
    assert_eq!(buffers.available_permits(), 0);
    // Retired bytes still own their budget until the downstream owner drops
    // them, so switching sources cannot defeat the process-wide payload bound.
    drop(held);
    assert_eq!(buffers.available_permits(), MAX_REMOTE_OUTSTANDING_RANGES);
    server.await.unwrap();
}
