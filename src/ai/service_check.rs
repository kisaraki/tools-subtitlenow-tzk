//! Explicit Gemini diagnostic: one model GET and one initialization, never audio.
//! No production endpoint override, model adoption, retries, persistence or logging.
use super::{
    credentials::ApiKey, provider::SpeechProvider, speech::SpeechError,
    streaming::NetworkRecognizer,
};
use crate::domain::SourceLanguage;
use reqwest::{Client, header::HeaderValue};
use serde::Deserialize;
use std::{future::Future, time::Duration};

const MODEL_ENDPOINT: &str =
    "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.5-transcribe-live";
const MODEL_NAME: &str = "models/gemini-3.5-transcribe-live";
const MODEL_TIMEOUT: Duration = Duration::from_secs(8);
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_MODEL_BYTES: usize = 16 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ModelOutcome {
    Available { advertises_streaming: bool },
    Http(u16),
    Timeout,
    Transport,
    InvalidData,
}
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CheckReport {
    ForModel {
        selected: super::provider::LiveSpeechModel,
        report: Box<CheckReport>,
    },
    Cancelled,
    Finished {
        model: ModelOutcome,
        initialization: Result<(), SpeechError>,
    },
}
impl CheckReport {
    pub fn label(&self) -> String {
        match self {
            Self::ForModel { selected, report } => report
                .label()
                .replace("gemini-3.5-transcribe-live", selected.model()),
            Self::Cancelled => "Gemini 無音訊診斷已取消；未擷取或傳送音訊。".into(),
            Self::Finished {
                model,
                initialization,
            } => {
                let model = match model {
                    ModelOutcome::Available {
                        advertises_streaming: true,
                    } => "模型資訊可讀，列有串流方法；不代表執行權限".into(),
                    ModelOutcome::Available {
                        advertises_streaming: false,
                    } => "模型資訊可讀，但未列出串流方法；不能據此判定不可使用".into(),
                    ModelOutcome::Http(code) => format!("模型資訊 HTTP {code}；僅代表該查詢結果"),
                    ModelOutcome::Timeout => "模型資訊查詢逾時".into(),
                    ModelOutcome::Transport => "模型資訊查詢連線失敗".into(),
                    ModelOutcome::InvalidData => "模型資訊回應無法核對".into(),
                };
                let initialization = match initialization {
                    Ok(()) => "串流初始化成功；尚未驗收辨識或翻譯".into(),
                    Err(error) => format!("串流初始化失敗：{error}"),
                };
                format!(
                    "Gemini 無音訊診斷（gemini-3.5-transcribe-live）\r\n{model}\r\n{initialization}\r\n未擷取或傳送音訊；不會自動開始翻譯。"
                )
            }
        }
    }
}

pub(crate) async fn gemini(key: &ApiKey, language: SourceLanguage) -> CheckReport {
    execute(
        async {
            match client(key) {
                Ok(client) => metadata(&client, MODEL_ENDPOINT).await,
                Err(outcome) => outcome,
            }
        },
        || async {
            // connect completes setup before any capture. Dropping this owned
            // recognizer closes the only socket; run/audio are never invoked.
            let recognizer =
                NetworkRecognizer::connect(SpeechProvider::Gemini, key, language).await?;
            drop(recognizer);
            Ok(())
        },
        MODEL_TIMEOUT,
        CHECK_TIMEOUT,
    )
    .await
}
pub(crate) async fn gemini_model(
    key: &ApiKey,
    language: SourceLanguage,
    model: super::provider::LiveSpeechModel,
) -> CheckReport {
    if model.provider() != SpeechProvider::Gemini {
        return CheckReport::ForModel {
            selected: model,
            report: Box::new(CheckReport::Finished {
                model: ModelOutcome::InvalidData,
                initialization: Err(SpeechError::UnsupportedCapability),
            }),
        };
    }
    if model == super::provider::LiveSpeechModel::GeminiTranscribeLive {
        return gemini(key, language).await;
    }
    let name = format!("models/{}", model.model());
    let endpoint = format!("https://generativelanguage.googleapis.com/v1beta/{name}");
    let report = execute(
        async {
            match client(key) {
                Ok(client) => metadata_model(&client, &endpoint, &name).await,
                Err(outcome) => outcome,
            }
        },
        || async {
            let recognizer = NetworkRecognizer::connect_model(
                model,
                key,
                language,
                crate::domain::TranslationQuality::Standard,
            )
            .await?;
            drop(recognizer);
            Ok(())
        },
        MODEL_TIMEOUT,
        CHECK_TIMEOUT,
    )
    .await;
    CheckReport::ForModel {
        selected: model,
        report: Box::new(report),
    }
}
fn client(key: &ApiKey) -> Result<Client, ModelOutcome> {
    let mut header = HeaderValue::from_str(key.expose()).map_err(|_| ModelOutcome::InvalidData)?;
    header.set_sensitive(true);
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("x-goog-api-key", header);
    Client::builder()
        .default_headers(headers)
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .referer(false)
        .connect_timeout(MODEL_TIMEOUT)
        .timeout(MODEL_TIMEOUT)
        .pool_max_idle_per_host(1)
        .build()
        .map_err(|_| ModelOutcome::Transport)
}
async fn execute<M, I, F>(
    model: M,
    initialize: F,
    model_timeout: Duration,
    total: Duration,
) -> CheckReport
where
    M: Future<Output = ModelOutcome>,
    I: Future<Output = Result<(), SpeechError>>,
    F: FnOnce() -> I,
{
    let deadline = tokio::time::Instant::now() + total;
    let model = tokio::time::timeout_at(
        deadline.min(tokio::time::Instant::now() + model_timeout),
        model,
    )
    .await
    .unwrap_or(ModelOutcome::Timeout);
    let initialization = tokio::time::timeout_at(deadline, initialize())
        .await
        .unwrap_or(Err(SpeechError::Timeout));
    CheckReport::Finished {
        model,
        initialization,
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelDescription {
    name: String,
    #[serde(default)]
    supported_generation_methods: Vec<String>,
}
async fn metadata(client: &Client, endpoint: &str) -> ModelOutcome {
    metadata_model(client, endpoint, MODEL_NAME).await
}
async fn metadata_model(client: &Client, endpoint: &str, expected_name: &str) -> ModelOutcome {
    let mut response = match client.get(endpoint).send().await {
        Ok(response) => response,
        Err(error) => {
            return if error.is_timeout() {
                ModelOutcome::Timeout
            } else {
                ModelOutcome::Transport
            };
        }
    };
    let code = response.status().as_u16();
    if code != 200 {
        return ModelOutcome::Http(code);
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_MODEL_BYTES as u64)
    {
        return ModelOutcome::InvalidData;
    }
    let mut body = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                if chunk.len() > MAX_MODEL_BYTES - body.len() {
                    return ModelOutcome::InvalidData;
                }
                body.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            Err(error) => {
                return if error.is_timeout() {
                    ModelOutcome::Timeout
                } else {
                    ModelOutcome::Transport
                };
            }
        }
    }
    match serde_json::from_slice::<ModelDescription>(&body) {
        Ok(model) if model.name == expected_name => ModelOutcome::Available {
            advertises_streaming: model
                .supported_generation_methods
                .iter()
                .any(|method| method == "bidiGenerateContent"),
        },
        _ => ModelOutcome::InvalidData,
    }
}

#[cfg(test)]
#[path = "service_check_tests.rs"]
mod tests;
