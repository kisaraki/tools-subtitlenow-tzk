//! Loopback HTTP and synthetic mono PCM only. Production TLS, real keys and
//! cloud accuracy are not bypassed or claimed by these explicit test fixtures.
use super::*;
use crate::{
    ai::{
        prepared_speech::{PreparedChunkStamp, PreparedSpeechChunk},
        speech::{SpeechLanguage, StreamIdentity},
    },
    audio::capture::SessionId,
    domain::SourceLanguage,
    media::prepared_audio::PreparedSampleRate,
};
use std::{future::Future, net::SocketAddr, pin::Pin};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

const TEST_TIMEOUT: Duration = Duration::from_secs(10);
const TEST_REQUEST_LIMIT: usize = 2 * 1024 * 1024;
fn expected_capacity(recognizer: &HttpPreparedRecognizer) -> usize {
    recognizer.capabilities().max_concurrent
}
const TEST_KEY: &str = "SYNTHETIC_PREPARED_HTTP_TEST_KEY";

fn run(future: impl Future<Output = ()>) {
    // The real shared CPU/native lanes are serialized outside any await.
    let _owners = crate::media::work::TEST_OWNERS.lock().unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async { tokio::time::timeout(TEST_TIMEOUT, future).await.unwrap() });
}
fn request(sequence: u64, language: SourceLanguage) -> PreparedSpeechRequest {
    PreparedSpeechRequest::new(
        PreparedSpeechChunk::new(
            PreparedChunkStamp {
                identity: StreamIdentity {
                    session: SessionId(614),
                    generation: 2,
                },
                sequence,
                run_id: 1,
                rate: PreparedSampleRate::Hz16000,
                audio_start_us: 0,
                body_start_us: 0,
                body_end_us: 100_000,
                discontinuity: false,
            },
            vec![901; 1600],
        )
        .unwrap(),
        language,
        Vec::new(),
    )
    .unwrap()
}
fn client(provider: SpeechProvider, address: SocketAddr) -> HttpPreparedRecognizer {
    let key = ApiKey::from_text(TEST_KEY).unwrap();
    let mut recognizer = HttpPreparedRecognizer::new(provider, &key).unwrap();
    // Only this private test constructor permits unencrypted loopback transport.
    let mut headers = HeaderMap::new();
    let mut value = HeaderValue::from_str(&match provider {
        SpeechProvider::OpenAi => format!("Bearer {TEST_KEY}"),
        SpeechProvider::Gemini => TEST_KEY.to_owned(),
    })
    .unwrap();
    value.set_sensitive(true);
    headers.insert(
        match provider {
            SpeechProvider::OpenAi => AUTHORIZATION,
            SpeechProvider::Gemini => reqwest::header::HeaderName::from_static("x-goog-api-key"),
        },
        value,
    );
    recognizer.client = Client::builder()
        .default_headers(headers)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    recognizer.endpoint = format!("http://{address}/fixture");
    recognizer
}
fn response(provider: SpeechProvider) -> Vec<u8> {
    serde_json::to_vec(&match provider {
        SpeechProvider::OpenAi => serde_json::json!({"task":"transcribe","language":"english","duration":0.1,"text":"Alpha","segments":[{"id":0,"start":0.004,"end":0.016,"text":"Alpha"}]}),
        SpeechProvider::Gemini => serde_json::json!({"id":"interactions/synthetic-fixture","status":"completed","steps":[{"id":"synthetic-step","type":"model_output","content":[{"type":"text","text":"Alpha","annotations":[{"type":"word_info","text":"Alpha","start_index":0,"end_index":5,"start_offset":"0.004s","end_offset":"0.016s"}]}]}]}),
    }).unwrap()
}
struct Received {
    headers: String,
    body: Vec<u8>,
}
async fn receive(listener: &TcpListener) -> (TcpStream, Received) {
    let (mut socket, _) = listener.accept().await.unwrap();
    let mut bytes = Vec::new();
    let mut scratch = [0u8; 8192];
    let (header_end, body_len) = loop {
        let count = socket.read(&mut scratch).await.unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&scratch[..count]);
        assert!(bytes.len() <= TEST_REQUEST_LIMIT);
        if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            let headers = std::str::from_utf8(&bytes[..end]).unwrap();
            let length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|value| value.trim().parse::<usize>().unwrap())
                })
                .unwrap();
            assert!(length <= TEST_REQUEST_LIMIT - end - 4);
            break (end + 4, length);
        }
    };
    while bytes.len() < header_end + body_len {
        let count = socket.read(&mut scratch).await.unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&scratch[..count]);
        assert!(bytes.len() <= TEST_REQUEST_LIMIT);
    }
    (
        socket,
        Received {
            headers: String::from_utf8(bytes[..header_end].to_vec()).unwrap(),
            body: bytes[header_end..header_end + body_len].to_vec(),
        },
    )
}
async fn write(socket: &mut TcpStream, status: u16, mime: &str, extra: &str, body: &[u8]) {
    socket.write_all(format!("HTTP/1.1 {status} Fixture\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n", body.len()).as_bytes()).await.unwrap();
    socket.write_all(body).await.unwrap();
}
async fn drive_to_request<'a>(
    listener: &TcpListener,
    work: &mut Pin<Box<PreparedSpeechFuture<'a>>>,
) -> (TcpStream, Received) {
    tokio::select! {
        result = work.as_mut() => panic!("synthetic request finished before server: {:?}", result.err()),
        received = receive(listener) => received,
    }
}
async fn no_more_requests(listener: &TcpListener) {
    assert!(
        tokio::time::timeout(Duration::from_millis(35), listener.accept())
            .await
            .is_err()
    );
}

async fn await_request_cleanup(recognizer: &HttpPreparedRecognizer) {
    // Cancellation may beat a started encoder's cooperative checkpoint. The
    // worker intentionally retains its request lease until actual cleanup.
    let permits = tokio::time::timeout(
        Duration::from_secs(5),
        recognizer
            .requests
            .clone()
            .acquire_many_owned(expected_capacity(recognizer) as u32),
    )
    .await
    .unwrap()
    .unwrap();
    drop(permits);
    assert_eq!(
        recognizer.requests.available_permits(),
        expected_capacity(recognizer)
    );
}

#[test]
fn constructors_keep_fixed_hosts_and_truthful_selected_language_capabilities() {
    run(async {
        for provider in [SpeechProvider::OpenAi, SpeechProvider::Gemini] {
            let key = ApiKey::from_text(TEST_KEY).unwrap();
            let recognizer = HttpPreparedRecognizer::new(provider, &key).unwrap();
            let endpoint = reqwest::Url::parse(&recognizer.endpoint).unwrap();
            assert_eq!(endpoint.scheme(), "https");
            assert!(endpoint.query().is_none());
            assert_eq!(
                endpoint.host_str(),
                Some(match provider {
                    SpeechProvider::OpenAi => "api.openai.com",
                    SpeechProvider::Gemini => "generativelanguage.googleapis.com",
                })
            );
            let capabilities = recognizer.capabilities();
            assert!(!capabilities.constrained_auto_detection);
            assert!((1..=2).contains(&capabilities.max_concurrent));
            assert_eq!(
                capabilities.max_concurrent,
                crate::media::work::worker_limit()
            );
            for language in [
                SourceLanguage::English,
                SourceLanguage::Japanese,
                SourceLanguage::French,
                SourceLanguage::German,
                SourceLanguage::Korean,
            ] {
                for rate in [PreparedSampleRate::Hz16000, PreparedSampleRate::Hz24000] {
                    capabilities.validate(language, rate).unwrap();
                }
            }
            assert_eq!(
                capabilities.validate(SourceLanguage::Auto, PreparedSampleRate::Hz16000),
                Err(PreparedSpeechError::UnsupportedLanguage)
            );
            assert!(
                recognizer
                    .client
                    .post("http://127.0.0.1:1/fixture")
                    .send()
                    .await
                    .is_err()
            );
        }
    });
}

#[test]
fn pre_cancel_auto_and_unpolled_drop_make_no_http_request() {
    run(async {
        for provider in [SpeechProvider::OpenAi, SpeechProvider::Gemini] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let recognizer = client(provider, listener.local_addr().unwrap());
            for mode in 0..3 {
                let request = request(
                    1,
                    if mode == 1 {
                        SourceLanguage::Auto
                    } else {
                        SourceLanguage::English
                    },
                );
                let cancel = ResolverCancellation::default();
                if mode == 0 {
                    cancel.cancel();
                }
                let work = recognizer.recognize(&request, cancel.clone());
                if mode == 2 {
                    drop(work);
                } else {
                    assert_eq!(
                        work.await.err(),
                        Some(if mode == 0 {
                            PreparedSpeechError::Cancelled
                        } else {
                            PreparedSpeechError::UnsupportedLanguage
                        })
                    );
                }
                assert_eq!(cancel.is_cancelled(), mode == 0);
                assert_eq!(
                    recognizer.requests.available_permits(),
                    expected_capacity(&recognizer)
                );
                no_more_requests(&listener).await;
            }
        }
    });
}

#[test]
fn both_protocols_upload_only_ram_audio_and_preserve_native_result_binding() {
    run(async {
        for provider in [SpeechProvider::OpenAi, SpeechProvider::Gemini] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let recognizer = client(provider, listener.local_addr().unwrap());
            let request = request(7, SourceLanguage::English);
            let cancel = ResolverCancellation::default();
            let server = async {
                let (mut socket, received) = receive(&listener).await;
                let headers = received.headers.to_ascii_lowercase();
                assert!(headers.starts_with("post /fixture http/1.1"));
                assert!(
                    !headers
                        .lines()
                        .next()
                        .unwrap()
                        .contains(&TEST_KEY.to_ascii_lowercase())
                );
                match provider {
                    SpeechProvider::OpenAi => {
                        assert!(
                            headers
                                .contains("authorization: bearer synthetic_prepared_http_test_key")
                        );
                        assert!(headers.contains("multipart/form-data"));
                        assert!(received.body.windows(4).any(|part| part == b"RIFF"));
                        assert!(received.body.windows(9).any(|part| part == b"whisper-1"));
                        assert!(!headers.contains("x-goog-api-key"));
                    }
                    SpeechProvider::Gemini => {
                        assert!(
                            headers.contains("x-goog-api-key: synthetic_prepared_http_test_key")
                        );
                        assert!(!headers.contains("authorization:"));
                        let body: serde_json::Value =
                            serde_json::from_slice(&received.body).unwrap();
                        assert_eq!(body["model"], "gemini-3.5-transcribe");
                        assert_eq!(body["store"], false);
                        assert_eq!(body["background"], false);
                        assert!(body.get("previous_interaction_id").is_none());
                        assert!(body.get("speech_config").is_none());
                        assert_eq!(
                            body["generation_config"]["transcription_config"]["mode"]["type"],
                            "verbatim"
                        );
                    }
                }
                write(
                    &mut socket,
                    200,
                    "application/json; charset=utf-8",
                    "",
                    &response(provider),
                )
                .await;
            };
            let ((), result) = tokio::join!(server, recognizer.recognize(&request, cancel.clone()));
            let result = result.unwrap();
            result.validate_for(&request).unwrap();
            assert_eq!(result.identity, request.chunk().stamp().identity);
            assert_eq!(result.sequence, 7);
            assert_eq!(result.language, Some(SpeechLanguage::English));
            assert_eq!(result.cues.len(), 1);
            assert_eq!(result.cues[0].start_us, 4000);
            assert_eq!(result.cues[0].duration_us, 12_000);
            assert_eq!(result.cues[0].text.as_ref(), "Alpha");
            assert!(!cancel.is_cancelled());
            assert_eq!(
                recognizer.requests.available_permits(),
                expected_capacity(&recognizer)
            );
        }
    });
}

#[test]
fn non_retryable_http_failures_are_typed_and_never_switch_provider() {
    run(async {
        for provider in [SpeechProvider::OpenAi, SpeechProvider::Gemini] {
            for status in [400, 401, 403, 302] {
                let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
                let recognizer = client(provider, listener.local_addr().unwrap());
                let request = request(1, SourceLanguage::English);
                let server = async {
                    let (mut socket, _) = receive(&listener).await;
                    write(
                        &mut socket,
                        status,
                        "application/json",
                        "Location: http://127.0.0.1:1/unselected\r\n",
                        b"private synthetic error must not appear in domain messages",
                    )
                    .await;
                };
                let ((), result) = tokio::join!(
                    server,
                    recognizer.recognize(&request, ResolverCancellation::default())
                );
                assert_eq!(
                    result.err(),
                    Some(if status == 401 || status == 403 {
                        PreparedSpeechError::Credential
                    } else {
                        PreparedSpeechError::InvalidData
                    })
                );
                assert_eq!(
                    recognizer.requests.available_permits(),
                    expected_capacity(&recognizer)
                );
                no_more_requests(&listener).await;
            }
        }
    });
}

#[test]
fn retryable_status_retries_once_with_the_same_selected_audio_request() {
    run(async {
        for provider in [SpeechProvider::OpenAi, SpeechProvider::Gemini] {
            for status in [429, 503] {
                let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
                let recognizer = client(provider, listener.local_addr().unwrap());
                let request = request(1, SourceLanguage::English);
                let server = async {
                    let (mut first, initial) = receive(&listener).await;
                    write(
                        &mut first,
                        status,
                        "application/json",
                        "Retry-After: 0\r\n",
                        b"{}",
                    )
                    .await;
                    let (mut second, retry) = receive(&listener).await;
                    assert_eq!(retry.body, initial.body);
                    write(
                        &mut second,
                        200,
                        "application/json",
                        "",
                        &response(provider),
                    )
                    .await;
                };
                let ((), result) = tokio::join!(
                    server,
                    recognizer.recognize(&request, ResolverCancellation::default())
                );
                result.unwrap().validate_for(&request).unwrap();
                assert_eq!(
                    recognizer.requests.available_permits(),
                    expected_capacity(&recognizer)
                );
                no_more_requests(&listener).await;
            }
        }
    });
}

#[test]
fn oversized_retry_after_is_not_shortened_into_an_early_retry() {
    run(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let recognizer = client(SpeechProvider::OpenAi, listener.local_addr().unwrap());
        let request = request(1, SourceLanguage::English);
        let server = async {
            let (mut socket, _) = receive(&listener).await;
            write(
                &mut socket,
                429,
                "application/json",
                "Retry-After: 120\r\n",
                b"{}",
            )
            .await;
        };
        let ((), result) = tokio::join!(
            server,
            recognizer.recognize(&request, ResolverCancellation::default())
        );
        assert_eq!(result.err(), Some(PreparedSpeechError::RateLimited));
        assert!(
            *recognizer.next_request.lock().await
                > tokio::time::Instant::now() + Duration::from_secs(100)
        );
        assert_eq!(
            recognizer.requests.available_permits(),
            expected_capacity(&recognizer)
        );
        no_more_requests(&listener).await;
    });
}

#[test]
fn malformed_mime_json_and_unfinished_response_never_publish_cues() {
    run(async {
        for provider in [SpeechProvider::OpenAi, SpeechProvider::Gemini] {
            for mode in 0..3 {
                let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
                let recognizer = client(provider, listener.local_addr().unwrap());
                let request = request(1, SourceLanguage::English);
                let server = async {
                    let (mut socket, _) = receive(&listener).await;
                    let mut bytes = response(provider);
                    if mode == 1 {
                        bytes = b"{invalid synthetic JSON".to_vec();
                    }
                    if mode == 2 {
                        bytes.truncate(bytes.len() / 2);
                    }
                    write(
                        &mut socket,
                        200,
                        if mode == 0 {
                            "text/plain"
                        } else {
                            "application/json"
                        },
                        "",
                        &bytes,
                    )
                    .await;
                };
                let cancel = ResolverCancellation::default();
                let ((), result) =
                    tokio::join!(server, recognizer.recognize(&request, cancel.clone()));
                assert_eq!(result.err(), Some(PreparedSpeechError::InvalidData));
                assert!(!cancel.is_cancelled());
                assert_eq!(
                    recognizer.requests.available_permits(),
                    expected_capacity(&recognizer)
                );
                no_more_requests(&listener).await;
            }
        }
    });
}

#[test]
fn excessive_declared_and_chunked_response_is_rejected_without_body_accumulation() {
    run(async {
        for chunked in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let recognizer = client(SpeechProvider::OpenAi, listener.local_addr().unwrap());
            let request = request(1, SourceLanguage::English);
            let server = async {
                let (mut socket, _) = receive(&listener).await;
                if chunked {
                    socket.write_all(b"HTTP/1.1 200 Fixture\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").await.unwrap();
                    let bytes = vec![
                        b'x';
                        super::super::prepared_speech_wire::MAX_ASR_HTTP_RESPONSE_BYTES
                            + 1
                    ];
                    let _ = socket
                        .write_all(format!("{:x}\r\n", bytes.len()).as_bytes())
                        .await;
                    let _ = socket.write_all(&bytes).await;
                    let _ = socket.write_all(b"\r\n0\r\n\r\n").await;
                } else {
                    socket.write_all(format!("HTTP/1.1 200 Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", super::super::prepared_speech_wire::MAX_ASR_HTTP_RESPONSE_BYTES + 1).as_bytes()).await.unwrap();
                }
            };
            let ((), result) = tokio::join!(
                server,
                recognizer.recognize(&request, ResolverCancellation::default())
            );
            assert_eq!(result.err(), Some(PreparedSpeechError::InvalidData));
            assert_eq!(
                recognizer.requests.available_permits(),
                expected_capacity(&recognizer)
            );
        }
    });
}

#[test]
fn cancel_and_drop_close_pending_http_and_restore_request_capacity() {
    run(async {
        for provider in [SpeechProvider::OpenAi, SpeechProvider::Gemini] {
            for should_drop in [false, true] {
                let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
                let recognizer = client(provider, listener.local_addr().unwrap());
                let request = request(1, SourceLanguage::English);
                let cancel = ResolverCancellation::default();
                let cpu = crate::media::work::workers();
                let cpu_before = cpu.available_permits();
                let mut work = Box::pin(recognizer.recognize(&request, cancel.clone()));
                let (mut socket, _) = drive_to_request(&listener, &mut work).await;
                assert_eq!(
                    recognizer.requests.available_permits(),
                    expected_capacity(&recognizer) - 1
                );
                assert_eq!(cpu.available_permits(), cpu_before);
                if should_drop {
                    drop(work);
                } else {
                    cancel.cancel();
                    assert_eq!(work.await.err(), Some(PreparedSpeechError::Cancelled));
                }
                assert_eq!(cancel.is_cancelled(), !should_drop);
                assert_eq!(
                    recognizer.requests.available_permits(),
                    expected_capacity(&recognizer)
                );
                let mut scratch = [0u8; 1];
                assert_eq!(socket.read(&mut scratch).await.unwrap(), 0);
                no_more_requests(&listener).await;
            }
        }
    });
}

#[test]
fn shared_client_rejects_excess_callers_instead_of_an_unbounded_wait_queue() {
    run(async {
        for serial in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let mut recognizer = client(SpeechProvider::OpenAi, listener.local_addr().unwrap());
            if serial {
                // Exercise one-core admission even when this fixture host has more
                // cores; the real request guard and transport remain unchanged.
                recognizer.request_limit = 1;
                recognizer.requests = Arc::new(Semaphore::new(1));
            }
            let first = request(1, SourceLanguage::English);
            let second = request(2, SourceLanguage::English);
            let third = request(3, SourceLanguage::English);
            let mut first_work =
                Box::pin(recognizer.recognize(&first, ResolverCancellation::default()));
            let (mut first_socket, _) = drive_to_request(&listener, &mut first_work).await;
            let mut second_work = (expected_capacity(&recognizer) > 1)
                .then(|| Box::pin(recognizer.recognize(&second, ResolverCancellation::default())));
            let mut second_socket = if let Some(work) = &mut second_work {
                Some(drive_to_request(&listener, work).await.0)
            } else {
                None
            };
            assert_eq!(recognizer.requests.available_permits(), 0);
            assert_eq!(
                recognizer
                    .recognize(&third, ResolverCancellation::default())
                    .await
                    .err(),
                Some(PreparedSpeechError::Overloaded)
            );
            no_more_requests(&listener).await;
            write(
                &mut first_socket,
                200,
                "application/json",
                "",
                &response(SpeechProvider::OpenAi),
            )
            .await;
            first_work.await.unwrap();
            if let (Some(mut socket), Some(work)) = (second_socket.take(), second_work) {
                write(
                    &mut socket,
                    200,
                    "application/json",
                    "",
                    &response(SpeechProvider::OpenAi),
                )
                .await;
                work.await.unwrap();
            }
            assert_eq!(
                recognizer.requests.available_permits(),
                expected_capacity(&recognizer)
            );
        }
    });
}

#[test]
fn whole_request_timeout_covers_transport_and_releases_resources() {
    run(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let recognizer = client(SpeechProvider::OpenAi, listener.local_addr().unwrap());
        let request = request(1, SourceLanguage::English);
        let cancel = ResolverCancellation::default();
        let mut work = Box::pin(recognizer.recognize_with_timeout(
            &request,
            cancel.clone(),
            Duration::from_millis(100),
        ));
        let (mut socket, _) = drive_to_request(&listener, &mut work).await;
        assert_eq!(work.await.err(), Some(PreparedSpeechError::Timeout));
        assert!(!cancel.is_cancelled());
        assert_eq!(
            recognizer.requests.available_permits(),
            expected_capacity(&recognizer)
        );
        let mut scratch = [0u8; 1];
        assert_eq!(socket.read(&mut scratch).await.unwrap(), 0);
        no_more_requests(&listener).await;
    });
}

#[test]
fn cancellation_with_rate_lock_held_waits_for_started_wire_cleanup_without_sending() {
    run(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let recognizer = client(SpeechProvider::Gemini, listener.local_addr().unwrap());
        let request = request(1, SourceLanguage::English);
        let cancel = ResolverCancellation::default();
        let locked = recognizer.next_request.lock().await;
        let stopper = async {
            tokio::task::yield_now().await;
            cancel.cancel();
        };
        let (result, ()) = tokio::join!(recognizer.recognize(&request, cancel.clone()), stopper);
        assert_eq!(result.err(), Some(PreparedSpeechError::Cancelled));
        drop(locked);
        await_request_cleanup(&recognizer).await;
        no_more_requests(&listener).await;
    });
}

#[test]
fn terminal_network_failures_stop_after_two_attempts_and_restore_capacity() {
    run(async {
        for disconnect in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let recognizer = client(SpeechProvider::OpenAi, listener.local_addr().unwrap());
            let request = request(1, SourceLanguage::English);
            let server = async {
                for _ in 0..2 {
                    let (mut socket, _) = receive(&listener).await;
                    if !disconnect {
                        write(&mut socket, 503, "application/json", "", b"{}").await;
                    }
                    drop(socket);
                }
            };
            let cancel = ResolverCancellation::default();
            let ((), result) = tokio::join!(server, recognizer.recognize(&request, cancel.clone()));
            assert_eq!(result.err(), Some(PreparedSpeechError::Network));
            assert!(!cancel.is_cancelled());
            assert_eq!(
                recognizer.requests.available_permits(),
                expected_capacity(&recognizer)
            );
            no_more_requests(&listener).await;
        }
    });
}

#[test]
fn malformed_overflow_and_long_date_retry_after_never_trigger_a_retry() {
    run(async {
        let date = httpdate::fmt_http_date(std::time::SystemTime::now() + Duration::from_secs(120));
        for retry_after in [
            "invalid synthetic retry".to_owned(),
            u64::MAX.to_string(),
            date,
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let recognizer = client(SpeechProvider::Gemini, listener.local_addr().unwrap());
            let request = request(1, SourceLanguage::English);
            let server = async {
                let (mut socket, _) = receive(&listener).await;
                write(
                    &mut socket,
                    429,
                    "application/json",
                    &format!("Retry-After: {retry_after}\r\n"),
                    b"{}",
                )
                .await;
            };
            let ((), result) = tokio::join!(
                server,
                recognizer.recognize(&request, ResolverCancellation::default())
            );
            assert_eq!(result.err(), Some(PreparedSpeechError::RateLimited));
            assert_eq!(
                recognizer.requests.available_permits(),
                expected_capacity(&recognizer)
            );
            no_more_requests(&listener).await;
        }
    });
}

#[cfg(windows)]
#[test]
fn native_prepared_session_uses_both_http_adapters_and_retains_the_original_media_pin() {
    use crate::{
        ai::translation::{
            TranslationCancellation, TranslationCapabilities, TranslationEmitter,
            TranslationFuture, TranslationRequest, Translator,
        },
        domain::TranslationQuality,
        media::{input::MediaInput, planning::PreparedRequest},
        session::prepared::{PreparedSession, PreparedState},
    };
    struct MockTranslator;
    impl Translator for MockTranslator {
        fn capabilities(&self) -> TranslationCapabilities {
            TranslationCapabilities {
                streaming: true,
                cancellation: true,
                supported_languages: [true; 5],
                max_concurrent: 2,
            }
        }
        fn translate<'a>(
            &'a self,
            request: &'a TranslationRequest,
            _: &'a mut TranslationEmitter,
            _: TranslationCancellation,
        ) -> TranslationFuture<'a> {
            Box::pin(async move {
                assert_eq!(request.source.text.as_ref(), "Alpha");
                assert_eq!(request.source.start_us, Some(4000));
                assert_eq!(request.source.duration_us, Some(12_000));
                Ok("软件".into())
            })
        }
    }
    run(async {
        for provider in [SpeechProvider::OpenAi, SpeechProvider::Gemini] {
            for malformed in [false, true] {
                let directory = tempfile::tempdir().unwrap();
                let path = directory.path().join("synthetic-http-input.wav");
                let mut wave = Vec::new();
                wave.extend_from_slice(b"RIFF");
                wave.extend_from_slice(&3236u32.to_le_bytes());
                wave.extend_from_slice(b"WAVEfmt ");
                wave.extend_from_slice(&16u32.to_le_bytes());
                wave.extend_from_slice(&1u16.to_le_bytes());
                wave.extend_from_slice(&1u16.to_le_bytes());
                wave.extend_from_slice(&16_000u32.to_le_bytes());
                wave.extend_from_slice(&32_000u32.to_le_bytes());
                wave.extend_from_slice(&2u16.to_le_bytes());
                wave.extend_from_slice(&16u16.to_le_bytes());
                wave.extend_from_slice(b"data");
                wave.extend_from_slice(&3200u32.to_le_bytes());
                for _ in 0..1600 {
                    wave.extend_from_slice(&901i16.to_le_bytes());
                }
                std::fs::write(&path, &wave).unwrap();
                let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
                let recognizer = client(provider, listener.local_addr().unwrap());
                let mut session = PreparedSession::new();
                let task = session
                    .start(
                        PreparedRequest::new(MediaInput::local(path.clone()).unwrap(), Vec::new())
                            .unwrap(),
                    )
                    .unwrap();
                let identity = session.identity().unwrap();
                let server = async {
                    let (mut socket, _) = receive(&listener).await;
                    let payload = if malformed {
                        b"{invalid synthetic JSON".to_vec()
                    } else {
                        response(provider)
                    };
                    write(&mut socket, 200, "application/json", "", &payload).await;
                };
                let ((), completion) = tokio::join!(
                    server,
                    task.run_with_recognizer(
                        &recognizer,
                        &MockTranslator,
                        SourceLanguage::English,
                        PreparedSampleRate::Hz16000,
                        TranslationQuality::Fast,
                        None
                    )
                );
                if malformed {
                    let error = crate::media::MediaError::Speech(PreparedSpeechError::InvalidData);
                    assert_eq!(session.apply(completion), Err(error));
                    assert_eq!(session.state(), PreparedState::Failed(error));
                    assert!(session.ready().is_none());
                    assert!(std::fs::OpenOptions::new().write(true).open(&path).is_ok());
                    assert_eq!(
                        recognizer.requests.available_permits(),
                        expected_capacity(&recognizer)
                    );
                    continue;
                }
                assert!(session.apply(completion).unwrap());
                assert_eq!(session.state(), PreparedState::Ready);
                let ready = session.ready().unwrap();
                assert_eq!(ready.identity(), identity);
                assert!(ready.original().existing().is_none());
                assert_eq!(
                    ready.original().recognized().unwrap().cues()[0]
                        .text
                        .as_ref(),
                    "Alpha"
                );
                assert_eq!(ready.translated()[0].as_ref(), "軟體");
                assert_eq!(ready.media_byte_len(), wave.len() as u64);
                assert!(std::fs::OpenOptions::new().write(true).open(&path).is_err());
                assert_eq!(std::fs::read(&path).unwrap(), wave);
                assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
                session.stop();
                assert!(std::fs::OpenOptions::new().write(true).open(&path).is_ok());
                assert_eq!(
                    recognizer.requests.available_permits(),
                    expected_capacity(&recognizer)
                );
            }
        }
    });
}
