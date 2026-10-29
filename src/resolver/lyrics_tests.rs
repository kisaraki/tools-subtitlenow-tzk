//! Synthetic metadata/lyrics and localhost HTTP only. Never access real lyrics,
//! accounts, keys, native audio or persistent media stores.
use super::*;
use crate::{ai::speech::SpeechLanguage, audio::capture::SessionId};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

#[tokio::test]
async fn confirmed_song_session_uses_real_http_adapter_once_and_clears_text_on_transition() {
    use crate::resolver::text_session::{ExistingTextSession, TextLookupStatus};
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let service = local_service(listener.local_addr().unwrap());
    let provider: Arc<dyn ExistingTextProvider> = Arc::new(service.for_song(track(None, None)));
    let (mut session, task) = ExistingTextSession::new(identity());
    session
        .select_confirmed_lyrics(query().stamp, &resolution(), Some(provider.clone()))
        .unwrap();
    let worker = tokio::spawn(task.run());
    let body = record().to_string();
    let (sent, update) = tokio::join!(
        serve(&listener, 200, "Content-Type: application/json\r\n", &body),
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let update = session.next().await.unwrap();
                if update.status == TextLookupStatus::Available {
                    break update;
                }
            }
        })
    );
    let update = update.unwrap();
    assert!(sent.contains("track_name=Synthetic+%26+Song%3F"));
    assert!(!sent.contains("duration="));
    assert_eq!(
        update.text.as_ref().unwrap().cues[0].start_us,
        Some(1_250_000)
    );
    assert_eq!(
        update.text.as_ref().unwrap().content_id.as_ref(),
        "catalog:synthetic"
    );
    let weak = Arc::downgrade(update.text.as_ref().unwrap());
    drop(update);
    let mut second = query().stamp;
    second.sequence += 1;
    assert!(
        !session
            .select_confirmed_lyrics(second, &resolution(), Some(provider.clone()))
            .unwrap()
    );
    let mut transition = resolution();
    transition.kind = ContentType::Mixed;
    transition.song_state = SongState::Transition;
    let mut third = query().stamp;
    third.sequence += 2;
    session
        .select_confirmed_lyrics(third, &transition, None)
        .unwrap();
    assert!(weak.upgrade().is_none());
    assert_eq!(session.poll().unwrap().status, TextLookupStatus::Cleared);
    assert!(
        tokio::time::timeout(Duration::from_millis(50), listener.accept())
            .await
            .is_err()
    );
    session.stop().unwrap();
    worker.await.unwrap().unwrap();
}

#[tokio::test]
async fn source_reset_cancels_text_session_and_real_http_connection_before_any_result() {
    use crate::resolver::text_session::{ExistingTextSession, TextLookupStatus};
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let service = local_service(listener.local_addr().unwrap());
    let provider: Arc<dyn ExistingTextProvider> = Arc::new(service.for_song(track(None, None)));
    let (mut session, task) = ExistingTextSession::new(identity());
    session
        .select_confirmed_lyrics(query().stamp, &resolution(), Some(provider))
        .unwrap();
    let worker = tokio::spawn(task.run());
    let mut socket = accept(&listener).await;
    let sent = request(&mut socket).await;
    assert!(sent.starts_with("GET /api/get?"));
    session.clear().unwrap();
    assert_eq!(session.poll().unwrap().status, TextLookupStatus::Cleared);
    let mut byte = [0u8; 1];
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), socket.read(&mut byte))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    assert_eq!(service.requests.available_permits(), 1);
    assert!(session.poll().is_none());
    session.stop().unwrap();
    worker.await.unwrap().unwrap();
}

fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(710),
        generation: 2,
    }
}
fn resolution() -> Resolution {
    Resolution {
        kind: ContentType::Music,
        song_state: SongState::Confirmed,
        song: Some(Arc::new(SongCandidate {
            id: "catalog:synthetic".into(),
            title: "Synthetic & Song?".into(),
            artist: Some("Artist / 測試".into()),
            confidence: Confidence::new(950).unwrap(),
            match_offset_us: None,
        })),
        movies: Vec::new(),
    }
}
fn track(album: Option<&str>, duration: Option<u64>) -> LyricsTrack {
    LyricsTrack::from_resolution(
        &resolution(),
        TextLanguage::Source(SpeechLanguage::English),
        album.map(Into::into),
        duration,
    )
    .unwrap()
}
fn query() -> TextQuery {
    TextQuery {
        stamp: WindowStamp {
            identity: identity(),
            sequence: 10,
            ended_at: Instant::now(),
            start_us: None,
            duration_us: 12_000_000,
        },
        source: SubtitleSource::ExistingLyrics,
        content_id: "catalog:synthetic".into(),
    }
}
fn record() -> Value {
    json!({"id":7,"trackName":"Synthetic & Song?","artistName":"Artist / 測試","albumName":"Fixture album","duration":6.5,"instrumental":false,
        "syncedLyrics":"[00:01.25] synthetic one\n[00:02.500] synthetic two\n[00:04.00] \n[00:05.00] synthetic three\n[00:06.00] ","plainLyrics":"synthetic plain",
        "lyricsfile":"version: unknown; do not execute or deserialize this field"})
}
fn local_service(address: std::net::SocketAddr) -> Arc<LrcLibService> {
    assert!(address.ip().is_loopback());
    Arc::new(LrcLibService {
        identity: identity(),
        client: Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap(),
        endpoint: format!("http://{address}/api/get"),
        requests: Semaphore::new(1),
        rate: Mutex::new(RateGate {
            next: Instant::now(),
            denied: false,
        }),
    })
}
async fn request(socket: &mut TcpStream) -> String {
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut bytes = Vec::new();
        let mut buffer = [0; 1024];
        while !bytes.ends_with(b"\r\n\r\n") {
            let read = socket.read(&mut buffer).await.unwrap();
            assert!(read > 0 && bytes.len() + read <= 16 * 1024);
            bytes.extend_from_slice(&buffer[..read]);
        }
        String::from_utf8(bytes).unwrap()
    })
    .await
    .unwrap()
}
async fn accept(listener: &TcpListener) -> TcpStream {
    tokio::time::timeout(Duration::from_secs(5), listener.accept())
        .await
        .unwrap()
        .unwrap()
        .0
}
async fn serve(listener: &TcpListener, status: u16, headers: &str, body: &str) -> String {
    let (mut socket, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
        .await
        .unwrap()
        .unwrap();
    let request = request(&mut socket).await;
    let header = format!(
        "HTTP/1.1 {status} Fixture\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    socket.write_all(header.as_bytes()).await.unwrap();
    // Some negative cases deliberately cause rejection before the body is read.
    let _ = socket.write_all(body.as_bytes()).await;
    request
}
async fn roundtrip(
    value: Value,
    album: Option<&str>,
    duration: Option<u64>,
) -> Result<Option<ExistingText>, ResolverError> {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let provider = local_service(listener.local_addr().unwrap()).for_song(track(album, duration));
    let body = value.to_string();
    let query = query();
    let (_, result) = tokio::join!(
        serve(&listener, 200, "Content-Type: application/json\r\n", &body),
        provider.lookup(&query, ResolverCancellation::default())
    );
    result
}

#[tokio::test]
async fn explicit_configuration_requires_confirmed_metadata_and_fixed_secure_endpoint() {
    let service = LrcLibService::new(identity()).unwrap();
    assert_eq!(service.endpoint, "https://lrclib.net/api/get");
    let binding = service.for_song(track(None, None));
    assert!(binding.supports(SubtitleSource::ExistingLyrics));
    assert!(!binding.supports(SubtitleSource::ExistingSubtitle));
    let second = service.for_song(track(None, None));
    assert!(Arc::ptr_eq(&binding.service, &second.service));
    for state in [
        SongState::Unknown,
        SongState::Candidate,
        SongState::Transition,
    ] {
        let mut candidate = resolution();
        candidate.song_state = state;
        assert!(matches!(
            LyricsTrack::from_resolution(&candidate, TextLanguage::TaiwanTraditional, None, None),
            Err(ResolverError::ContentMismatch)
        ));
    }
    let mut no_artist = resolution();
    Arc::get_mut(no_artist.song.as_mut().unwrap())
        .unwrap()
        .artist = None;
    assert!(matches!(
        LyricsTrack::from_resolution(&no_artist, TextLanguage::TaiwanTraditional, None, None),
        Err(ResolverError::InvalidData)
    ));
    for duration in [0, MAX_TRACK_DURATION_US + 1] {
        assert!(matches!(
            LyricsTrack::from_resolution(
                &resolution(),
                TextLanguage::TaiwanTraditional,
                None,
                Some(duration)
            ),
            Err(ResolverError::InvalidData)
        ));
    }
}

#[tokio::test]
async fn actual_get_encodes_only_song_metadata_and_returns_original_timing_in_session_ram() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let provider = local_service(listener.local_addr().unwrap()).for_song(track(None, None));
    let body = record().to_string();
    let query = query();
    let (request, result) = tokio::join!(
        serve(
            &listener,
            200,
            "Content-Type: Application/JSON; charset=utf-8\r\n",
            &body
        ),
        provider.lookup(&query, ResolverCancellation::default())
    );
    let uri = request
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap();
    let parsed = reqwest::Url::parse(&format!("http://localhost{uri}")).unwrap();
    let parameters: std::collections::HashMap<_, _> = parsed.query_pairs().collect();
    assert_eq!(parameters.len(), 2);
    assert_eq!(parameters["track_name"], "Synthetic & Song?");
    assert_eq!(parameters["artist_name"], "Artist / 測試");
    assert!(!request.contains("catalog:synthetic"));
    assert!(!request.to_lowercase().contains("authorization"));
    assert!(!request.to_lowercase().contains("cookie"));
    let text = result.unwrap().unwrap();
    text.validate(&query).unwrap();
    assert_eq!(text.identity, identity());
    assert_eq!(text.content_id, query.content_id);
    assert!(matches!(
        text.language,
        TextLanguage::Source(SpeechLanguage::English)
    ));
    assert_eq!(text.cues.len(), 3);
    assert_eq!(text.cues[0].start_us, Some(1_250_000));
    assert_eq!(text.cues[0].duration_us, Some(1_250_000));
    assert_eq!(text.cues[1].duration_us, Some(1_500_000)); // Blank marker preserves the gap.
    assert_eq!(text.cues[2].duration_us, Some(1_000_000));
}

#[tokio::test]
async fn full_track_metadata_is_optional_and_never_replaced_by_window_duration() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let provider = local_service(listener.local_addr().unwrap())
        .for_song(track(Some("Fixture album"), Some(6_500_000)));
    let body = record().to_string();
    let query = query();
    let (request, result) = tokio::join!(
        serve(&listener, 200, "Content-Type: application/json\r\n", &body),
        provider.lookup(&query, ResolverCancellation::default())
    );
    result.unwrap().unwrap();
    assert!(request.contains("duration=6.5"));
    assert!(request.contains("album_name=Fixture+album"));
    assert!(!request.contains("duration=12"));
    let mut matching = record();
    matching["trackName"] = json!("  synthetic   & song? ");
    matching["artistName"] = json!("ARTIST / 測試");
    assert!(roundtrip(matching, None, None).await.unwrap().is_some());
}

#[tokio::test]
async fn plain_instrumental_missing_lyrics_and_not_found_are_distinct_from_malformed_data() {
    let mut plain = record();
    plain["syncedLyrics"] = Value::Null;
    plain["plainLyrics"] = json!("synthetic first\r\n\nsynthetic second");
    let result = roundtrip(plain, None, None).await.unwrap().unwrap();
    assert_eq!(result.cues.len(), 2);
    assert!(
        result
            .cues
            .iter()
            .all(|cue| cue.start_us.is_none() && cue.duration_us.is_none())
    );
    for instrumental in [false, true] {
        let mut missing = record();
        missing["instrumental"] = json!(instrumental);
        missing["syncedLyrics"] = Value::Null;
        missing["plainLyrics"] = Value::Null;
        assert!(roundtrip(missing, None, None).await.unwrap().is_none());
    }
    let mut contradictory = record();
    contradictory["instrumental"] = json!(true);
    assert!(matches!(
        roundtrip(contradictory, None, None).await,
        Err(ResolverError::InvalidData)
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let provider = local_service(listener.local_addr().unwrap()).for_song(track(None, None));
    let query = query();
    let (_, result) = tokio::join!(
        serve(
            &listener,
            404,
            "Content-Type: text/plain\r\n",
            "synthetic private error"
        ),
        provider.lookup(&query, ResolverCancellation::default())
    );
    assert!(result.unwrap().is_none());
}

#[tokio::test]
async fn mismatched_title_artist_album_and_duration_cannot_supply_lyrics() {
    for (field, value, album, duration) in [
        ("trackName", json!("Synthetic & Song? Live"), None, None),
        ("artistName", json!("Other artist"), None, None),
        (
            "albumName",
            json!("Other album"),
            Some("Fixture album"),
            None,
        ),
        ("duration", json!(9.0), None, Some(6_500_000)),
    ] {
        let mut bad = record();
        bad[field] = value;
        assert!(matches!(
            roundtrip(bad, album, duration).await,
            Err(ResolverError::ContentMismatch)
        ));
    }
}

#[tokio::test]
async fn foreign_generation_wrong_content_id_unsupported_source_and_precancel_never_request() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let provider = local_service(listener.local_addr().unwrap()).for_song(track(None, None));
    for case in 0..5 {
        let mut query = query();
        let token = ResolverCancellation::default();
        let expected = match case {
            0 => {
                query.stamp.identity.generation += 1;
                ResolverError::InvalidData
            }
            1 => {
                query.content_id = "old-song".into();
                ResolverError::ContentMismatch
            }
            2 => {
                query.source = SubtitleSource::ExistingSubtitle;
                ResolverError::UnsupportedCapability
            }
            3 => {
                query.stamp.ended_at -= MAX_WINDOW_AGE + Duration::from_secs(1);
                ResolverError::Stale
            }
            _ => {
                token.cancel();
                ResolverError::Cancelled
            }
        };
        assert!(matches!(provider.lookup(&query, token).await, Err(error) if error == expected));
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(50), listener.accept())
            .await
            .is_err()
    );
}

#[test]
fn lrc_metadata_repeated_chorus_equal_times_blank_gaps_and_fraction_precision_are_preserved() {
    let text = "[ar:fixture]\n[ti:fixture]\n[offset:0]\n[00:04.5][00:00.500]chorus\n[00:00.5]simultaneous\n[00:01] \n[00:05]end";
    let cues = lrc::synced(text, 6_500_000).unwrap();
    assert_eq!(cues.len(), 4);
    assert_eq!(cues[0].start_us, Some(500_000));
    assert_eq!(cues[1].start_us, Some(500_000));
    assert_eq!(cues[0].duration_us, Some(500_000));
    assert_eq!(cues[2].text.as_ref(), "chorus");
    assert_eq!(cues[2].start_us, Some(4_500_000));
    assert_eq!(cues[3].duration_us, Some(1_500_000));
}

#[tokio::test]
async fn malformed_synced_text_cannot_silently_fall_back_to_plain_or_fake_timestamps() {
    for synced in [
        "untimed",
        "[00:99]bad",
        "[00:01.1234]bad",
        "[00:07]past end",
        "[00:06.5]zero duration",
        "[00:01]a<00:02>b",
        "[offset:100]\n[00:01]bad",
    ] {
        let mut bad = record();
        bad["syncedLyrics"] = json!(synced);
        assert!(matches!(
            roundtrip(bad, None, None).await,
            Err(ResolverError::InvalidData | ResolverError::UnsupportedCapability)
        ));
    }
    let mut controls = record();
    controls["syncedLyrics"] = json!("[00:01]synthetic\u{0}");
    assert!(matches!(
        roundtrip(controls, None, None).await,
        Err(ResolverError::InvalidData)
    ));
}

#[test]
fn lyric_parser_bounds_marker_expansion_cue_bytes_count_and_json_record_types() {
    let overflow = "[00:01]".repeat(lrc::MAX_LYRIC_CUES + 1) + "a";
    assert!(lrc::synced(&overflow, 6_500_000).is_err());
    let expanded = "[00:01]".repeat(30) + &"x".repeat(6000);
    assert!(lrc::synced(&expanded, 6_500_000).is_err());
    assert!(lrc::plain(&"a\n".repeat(lrc::MAX_LYRIC_CUES + 1)).is_err());
    assert!(lrc::plain(&"a".repeat(MAX_CUE_BYTES + 1)).is_err());
    assert!(lrc::synced("[00:01] ", 6_500_000).is_err());
    for field in ["duration", "id", "instrumental"] {
        let mut bad = record();
        bad[field] = json!("synthetic wrong type");
        assert!(serde_json::from_value::<Record>(bad).is_err());
    }
}

#[tokio::test]
async fn content_type_body_limit_truncated_and_invalid_json_are_typed_without_private_payload() {
    for (content_type, body) in [
        ("text/plain", record().to_string()),
        ("application/json", "x".repeat(MAX_RESPONSE_BYTES + 1)),
        (
            "application/json",
            "{\"private-token\": \"SYNTHETIC_SECRET\"".into(),
        ),
        ("application/json", "[]".into()),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let provider = local_service(listener.local_addr().unwrap()).for_song(track(None, None));
        let query = query();
        let headers = format!("Content-Type: {content_type}\r\n");
        let (_, result) = tokio::join!(
            serve(&listener, 200, &headers, &body),
            provider.lookup(&query, ResolverCancellation::default())
        );
        let error = result.err().unwrap();
        assert_eq!(error, ResolverError::InvalidData);
        assert!(!error.to_string().contains("SYNTHETIC_SECRET"));
    }
    // No Content-Length: enforce the same cap while streaming actual chunked bytes.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let provider = local_service(listener.local_addr().unwrap()).for_song(track(None, None));
    let query = query();
    let server = async {
        let mut socket = accept(&listener).await;
        request(&mut socket).await;
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").await.unwrap();
        let body = "x".repeat(MAX_RESPONSE_BYTES + 1);
        let frame = format!("{:X}\r\n{}\r\n0\r\n\r\n", body.len(), body);
        let _ = socket.write_all(frame.as_bytes()).await;
    };
    let ((), result) = tokio::join!(
        server,
        provider.lookup(&query, ResolverCancellation::default())
    );
    assert!(matches!(result, Err(ResolverError::InvalidData)));
}

#[tokio::test]
async fn retry_after_seconds_date_default_and_invalid_headers_apply_across_song_bindings() {
    for (status, header) in [
        (429, "Retry-After: 120\r\n".to_owned()),
        (
            503,
            format!(
                "Retry-After: {}\r\n",
                httpdate::fmt_http_date(SystemTime::now() + Duration::from_secs(600))
            ),
        ),
        (429, String::new()),
        (429, "Retry-After: invalid date\r\n".into()),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let service = local_service(listener.local_addr().unwrap());
        let first = service.for_song(track(None, None));
        let second = service.for_song(track(None, None));
        let query = query();
        let (_, result) = tokio::join!(
            serve(&listener, status, &header, "synthetic error"),
            first.lookup(&query, ResolverCancellation::default())
        );
        assert!(matches!(
            result,
            Err(ResolverError::RateLimited | ResolverError::Network)
        ));
        {
            let rate = service.rate.lock().unwrap();
            assert!(
                rate.denied || rate.next.duration_since(Instant::now()) >= Duration::from_secs(59)
            );
        }
        assert!(matches!(
            second.lookup(&query, ResolverCancellation::default()).await,
            Err(ResolverError::RateLimited)
        ));
        assert!(
            tokio::time::timeout(Duration::from_millis(20), listener.accept())
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn cancellation_and_future_drop_close_pending_lookup_release_permit_and_never_queue() {
    for cancel_request in [true, false] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let service = local_service(listener.local_addr().unwrap());
        let provider = service.for_song(track(None, None));
        let other = service.for_song(track(None, None));
        let query = query();
        let token = ResolverCancellation::default();
        let (ready, notified) = tokio::sync::oneshot::channel();
        let server = async {
            let mut socket = accept(&listener).await;
            request(&mut socket).await;
            ready.send(()).unwrap();
            let mut byte = [0; 1];
            let read = tokio::time::timeout(Duration::from_secs(2), socket.read(&mut byte))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(read, 0);
        };
        let client = async {
            let mut job = Box::pin(provider.lookup(&query, token.clone()));
            tokio::select! { _ = notified => {}, result = &mut job => panic!("unexpected early result: {:?}", result.err()) }
            assert!(matches!(
                other.lookup(&query, ResolverCancellation::default()).await,
                Err(ResolverError::RateLimited)
            ));
            if cancel_request {
                token.cancel();
                assert!(matches!(job.await, Err(ResolverError::Cancelled)));
            } else {
                drop(job);
            }
            assert_eq!(service.requests.available_permits(), 1);
        };
        tokio::join!(server, client);
    }
}

#[tokio::test]
async fn request_timeout_closes_transport_and_late_success_cannot_escape_freshness_gate() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let service = local_service(listener.local_addr().unwrap());
    let provider = service.for_song(track(None, None));
    let query = query();
    let server = async {
        let mut socket = accept(&listener).await;
        request(&mut socket).await;
        let mut byte = [0; 1];
        assert_eq!(
            tokio::time::timeout(
                LOOKUP_TIMEOUT + Duration::from_secs(2),
                socket.read(&mut byte)
            )
            .await
            .unwrap()
            .unwrap(),
            0
        );
    };
    let ((), result) = tokio::join!(
        server,
        provider.lookup(&query, ResolverCancellation::default())
    );
    assert!(matches!(result, Err(ResolverError::Timeout)));
    assert_eq!(service.requests.available_permits(), 1);
    for status in [200, 404] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let provider = local_service(listener.local_addr().unwrap()).for_song(track(None, None));
        let mut old = crate::resolver::lyrics::tests::query();
        old.stamp.ended_at -= Duration::from_secs(4);
        let body = record().to_string();
        let server = async {
            let mut socket = accept(&listener).await;
            request(&mut socket).await;
            tokio::time::sleep(Duration::from_millis(1200)).await;
            let header = format!(
                "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(header.as_bytes()).await.unwrap();
        };
        let ((), result) = tokio::join!(
            server,
            provider.lookup(&old, ResolverCancellation::default())
        );
        assert!(matches!(result, Err(ResolverError::Stale)));
    }
}

#[tokio::test]
async fn errors_and_redirects_do_not_leak_metadata_to_another_endpoint() {
    let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
    for status in [401, 403, 400, 500, 302] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let provider = local_service(listener.local_addr().unwrap()).for_song(track(None, None));
        let query = query();
        let headers = format!(
            "Location: http://{}/private\r\n",
            target.local_addr().unwrap()
        );
        let (_, result) = tokio::join!(
            serve(&listener, status, &headers, "SYNTHETIC_PRIVATE_ERROR"),
            provider.lookup(&query, ResolverCancellation::default())
        );
        let error = result.err().unwrap();
        let expected = match status {
            401 | 403 => ResolverError::Credential,
            500 => ResolverError::Network,
            _ => ResolverError::InvalidData,
        };
        assert_eq!(error, expected);
        assert!(!error.to_string().contains("SYNTHETIC_PRIVATE_ERROR"));
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(50), target.accept())
            .await
            .is_err()
    );
}
