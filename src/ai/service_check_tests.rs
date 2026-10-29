//! Local HTTP fixtures and owned futures only. No cloud or real credential access.
use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn fixture(response: Vec<u8>) -> (String, tokio::task::JoinHandle<String>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!(
        "http://{}/v1beta/{MODEL_NAME}",
        listener.local_addr().unwrap()
    );
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut byte = [0];
        while !request.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).await.unwrap();
            request.push(byte[0]);
            assert!(request.len() < 8192);
        }
        // An early size rejection is allowed to close before the entire body is sent.
        let _ = stream.write_all(&response).await;
        String::from_utf8(request).unwrap()
    });
    (endpoint, task)
}
fn response(status: u16, body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}
async fn query(bytes: Vec<u8>) -> (ModelOutcome, String) {
    let (endpoint, task) = fixture(bytes).await;
    // HTTP is enabled exclusively for this loopback fixture; production is HTTPS-only.
    let client = Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .default_headers({
            let mut headers = reqwest::header::HeaderMap::new();
            let mut value = HeaderValue::from_static("SYNTHETIC_ONLY");
            value.set_sensitive(true);
            headers.insert("x-goog-api-key", value);
            headers
        })
        .build()
        .unwrap();
    let outcome = tokio::time::timeout(Duration::from_secs(3), metadata(&client, &endpoint))
        .await
        .unwrap();
    (outcome, task.await.unwrap())
}

#[tokio::test]
async fn phase11_native_model_metadata_and_diagnostic_label_do_not_adopt_another_model() {
    use crate::ai::provider::LiveSpeechModel;
    let name = "models/gemini-3.5-live-translate-preview";
    for (actual, expected) in [
        (
            name,
            ModelOutcome::Available {
                advertises_streaming: true,
            },
        ),
        (MODEL_NAME, ModelOutcome::InvalidData),
    ] {
        let body = format!(
            "{{\"name\":\"{actual}\",\"supportedGenerationMethods\":[\"bidiGenerateContent\"]}}"
        );
        let (endpoint, task) = fixture(response(200, &body)).await;
        let client = Client::builder().no_proxy().build().unwrap();
        assert_eq!(metadata_model(&client, &endpoint, name).await, expected);
        task.await.unwrap();
    }
    let label = CheckReport::ForModel {
        selected: LiveSpeechModel::GeminiLiveTranslate,
        report: Box::new(CheckReport::Finished {
            model: ModelOutcome::Available {
                advertises_streaming: true,
            },
            initialization: Ok(()),
        }),
    }
    .label();
    assert!(label.contains("gemini-3.5-live-translate-preview"));
    assert!(!label.contains("gemini-3.5-transcribe-live"));
    assert!(label.contains("未擷取或傳送音訊"));
    let report = gemini_model(
        &ApiKey::from_text("SYNTHETIC_ONLY").unwrap(),
        SourceLanguage::English,
        LiveSpeechModel::GptLiveTranscribe,
    )
    .await;
    assert!(
        matches!(report, CheckReport::ForModel {report, ..} if matches!(*report, CheckReport::Finished {initialization: Err(SpeechError::UnsupportedCapability), ..}))
    );
}
#[tokio::test]
async fn fixed_model_get_has_no_body_or_key_in_url_and_reports_methods_separately() {
    for streaming in [true, false] {
        let methods = if streaming {
            "[\"bidiGenerateContent\"]"
        } else {
            "[]"
        };
        let body =
            format!("{{\"name\":\"{MODEL_NAME}\",\"supportedGenerationMethods\":{methods}}}");
        let (outcome, request) = query(response(200, &body)).await;
        assert_eq!(
            outcome,
            ModelOutcome::Available {
                advertises_streaming: streaming
            }
        );
        let first = request.lines().next().unwrap();
        assert_eq!(first, format!("GET /v1beta/{MODEL_NAME} HTTP/1.1"));
        assert!(!first.contains("SYNTHETIC_ONLY"));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("x-goog-api-key: synthetic_only")
        );
        assert!(!request.to_ascii_lowercase().contains("content-length:"));
        let label = CheckReport::Finished {
            model: outcome,
            initialization: Ok(()),
        }
        .label();
        assert!(label.contains("尚未驗收辨識或翻譯"));
        assert!(!label.contains("SYNTHETIC_ONLY"));
    }
}
#[tokio::test]
async fn numeric_http_failures_do_not_leak_body_or_follow_redirects() {
    for status in [302, 401, 403, 404, 429, 500] {
        let bytes = response(status, "PRIVATE_BODY_SYNTHETIC_ONLY");
        let bytes = if status == 302 {
            String::from_utf8(bytes)
                .unwrap()
                .replacen(
                    "\r\n",
                    "\r\nLocation: http://127.0.0.1:9/PRIVATE_REDIRECT\r\n",
                    1,
                )
                .into_bytes()
        } else {
            bytes
        };
        let (outcome, _) = query(bytes).await;
        assert_eq!(outcome, ModelOutcome::Http(status));
        let report = CheckReport::Finished {
            model: outcome,
            initialization: Err(SpeechError::ServiceInternal),
        };
        assert!(!format!("{report:?} {}", report.label()).contains("PRIVATE_BODY"));
        assert!(report.label().contains(&format!("HTTP {status}")));
    }
}
#[tokio::test]
async fn malformed_mismatched_and_oversized_metadata_are_not_availability() {
    for body in [
        "not json".into(),
        "{\"name\":\"models/another\"}".into(),
        "x".repeat(MAX_MODEL_BYTES + 1),
    ] {
        assert_eq!(
            query(response(200, &body)).await.0,
            ModelOutcome::InvalidData
        );
    }
    let bytes = format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{}\r\n0\r\n\r\n", MAX_MODEL_BYTES + 1, "x".repeat(MAX_MODEL_BYTES + 1)).into_bytes();
    assert_eq!(query(bytes).await.0, ModelOutcome::InvalidData);
}
#[tokio::test]
async fn production_client_rejects_http_without_contacting_fixture() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let client = client(&ApiKey::from_text("SYNTHETIC_ONLY").unwrap()).unwrap();
    assert_eq!(metadata(&client, &url).await, ModelOutcome::Transport);
    assert!(
        tokio::time::timeout(Duration::from_millis(20), listener.accept())
            .await
            .is_err()
    );
}
#[tokio::test]
async fn metadata_denied_still_initializes_once_without_retry_or_permission_inference() {
    let calls = AtomicUsize::new(0);
    let report = execute(
        async { ModelOutcome::Http(403) },
        || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Err(SpeechError::ServiceInternal)
        },
        Duration::from_secs(1),
        Duration::from_secs(2),
    )
    .await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        report,
        CheckReport::Finished {
            model: ModelOutcome::Http(403),
            initialization: Err(SpeechError::ServiceInternal)
        }
    );
}
#[tokio::test]
async fn model_timeout_still_attempts_initialization_within_shared_deadline() {
    let report = execute(
        std::future::pending(),
        || async { Ok(()) },
        Duration::from_millis(10),
        Duration::from_secs(1),
    )
    .await;
    assert_eq!(
        report,
        CheckReport::Finished {
            model: ModelOutcome::Timeout,
            initialization: Ok(())
        }
    );
    let began = tokio::time::Instant::now();
    let report = execute(
        std::future::pending(),
        std::future::pending,
        Duration::from_millis(30),
        Duration::from_millis(50),
    )
    .await;
    assert_eq!(
        report,
        CheckReport::Finished {
            model: ModelOutcome::Timeout,
            initialization: Err(SpeechError::Timeout)
        }
    );
    assert!(began.elapsed() < Duration::from_secs(1));
}
struct DropCount(Arc<AtomicUsize>);
#[test]
fn diagnostic_channel_has_no_source_or_session_and_cannot_replace_live_state() {
    use crate::ui::bridge::{self, UiSource, UiStatus};
    let (mut publisher, mut inbox) = bridge::channel();
    publisher.prepare_service_check().unwrap();
    let frame = inbox.poll().unwrap();
    assert!(frame.status == UiStatus::CheckingService);
    assert!(frame.source.is_none() && frame.identity.is_none() && frame.snapshot.is_none());
    assert!(publisher.prepare_service_check().is_err());
    publisher.finish(UiStatus::Stopped).unwrap();
    assert!(inbox.poll().unwrap().status == UiStatus::Stopped);
    assert!(publisher.prepare_service_check().is_err());
    let (mut publisher, inbox) = bridge::channel();
    publisher
        .prepare_source(UiSource {
            pid: 42,
            provider: SpeechProvider::Gemini,
        })
        .unwrap();
    assert!(publisher.prepare_service_check().is_err());
    drop(inbox);
    assert!(publisher.prepare_service_check().is_err());
}
impl Drop for DropCount {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
#[tokio::test]
async fn cancellation_drops_pending_work_without_detached_requests_or_next_stage() {
    for in_model in [true, false] {
        let dropped = Arc::new(AtomicUsize::new(0));
        let guard = DropCount(dropped.clone());
        let calls = AtomicUsize::new(0);
        let check = execute(
            async {
                let _guard = guard;
                if in_model {
                    std::future::pending::<()>().await;
                }
                ModelOutcome::Http(404)
            },
            || async {
                let _guard = DropCount(dropped.clone());
                calls.fetch_add(1, Ordering::SeqCst);
                std::future::pending().await
            },
            Duration::from_secs(1),
            Duration::from_secs(2),
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(15), check)
                .await
                .is_err()
        );
        assert_eq!(dropped.load(Ordering::SeqCst), if in_model { 1 } else { 2 });
        assert_eq!(calls.load(Ordering::SeqCst), usize::from(!in_model));
    }
}
