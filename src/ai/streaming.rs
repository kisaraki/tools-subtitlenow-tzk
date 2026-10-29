//! Fixed secure provider endpoints; no implicit provider fallback or content logs.
use super::{
    credentials::ApiKey,
    preprocess_worker::{EndpointPolicy, PreparedBlock, PreprocessWorker},
    provider::{LiveSpeechModel, SpeechProvider},
    speech::{
        AudioIssue, AudioStage, Cancellation, ConnectionStage, EVENT_QUEUE_ITEMS, RemoteStatusKind,
        SpeechAudio, SpeechCapabilities, SpeechError, SpeechFuture, SpeechLanguage, SpeechQueue,
        SpeechRecognizer, StreamIdentity, TranscriptKind, TranscriptOutput,
    },
    transcript_adapter::{MAX_WIRE_BYTES, TranscriptAdapter, UtteranceContext},
};
use crate::{
    audio::capture::MAX_AUDIO_AGE,
    domain::{SourceLanguage, TranslationQuality},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::Mutex,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpStream,
};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream,
    tungstenite::{self, Message, client::IntoClientRequest, protocol::WebSocketConfig},
};

const OPENAI_ENDPOINT: &str = "wss://api.openai.com/v1/realtime?intent=transcription";
const GEMINI_ENDPOINT: &str = "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent";
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
pub const SEND_TIMEOUT: Duration = Duration::from_millis(100);
pub const FINAL_TIMEOUT: Duration = Duration::from_secs(8);
/// Total time from the first unverified partial; later partials cannot renew it.
const AUTO_LANGUAGE_TIMEOUT: Duration = FINAL_TIMEOUT;
/// Leave six seconds of the existing eight-second budget for a real final reply.
const AUTO_LANGUAGE_FINALIZE_AFTER: Duration = Duration::from_secs(2);
/// Auto-VAD needs an input-end signal after a data interruption longer than one
/// second. This measures transport inactivity, never capture/media position.
const GEMINI_INPUT_IDLE: Duration = Duration::from_millis(1100);
/// Retain only a short, not-yet-sent prefix from this selected live source.
const OPENAI_PREFIX_WINDOW: Duration = Duration::from_millis(80);
/// Reserve the existing write budget; never refresh cached capture timestamps.
const OPENAI_PREFIX_MAX_AGE: Duration = MAX_AUDIO_AGE.saturating_sub(SEND_TIMEOUT);
/// Rotate before Gemini's documented 10-minute limit. Fresh Session IDs discard old work.
pub const CONNECTION_LIFETIME: Duration = Duration::from_secs(9 * 60);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);
const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(30);
type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub struct NetworkRecognizer {
    provider: SpeechProvider,
    model: LiveSpeechModel,
    quality: TranslationQuality,
    socket: Mutex<Option<Socket>>,
}
impl NetworkRecognizer {
    /// Handshake completes before capture starts, so DNS/TLS cannot queue stale audio.
    pub async fn connect(
        provider: SpeechProvider,
        key: &ApiKey,
        language: SourceLanguage,
    ) -> Result<Self, SpeechError> {
        Self::connect_model(
            LiveSpeechModel::default_for(provider),
            key,
            language,
            TranslationQuality::Standard,
        )
        .await
    }
    pub async fn connect_model(
        model: LiveSpeechModel,
        key: &ApiKey,
        language: SourceLanguage,
        quality: TranslationQuality,
    ) -> Result<Self, SpeechError> {
        let provider = model.provider();
        model.validate_live(language, quality)?;
        let request = match provider {
            SpeechProvider::OpenAi => {
                let mut request = OPENAI_ENDPOINT
                    .into_client_request()
                    .map_err(|_| SpeechError::InvalidData)?;
                let bearer = zeroize::Zeroizing::new(format!("Bearer {}", key.expose()));
                let mut value = tungstenite::http::HeaderValue::from_str(&bearer)
                    .map_err(|_| SpeechError::Credential)?;
                value.set_sensitive(true);
                request.headers_mut().insert("Authorization", value);
                request
            }
            SpeechProvider::Gemini => {
                // Encode every non-unreserved byte; never include this URI in errors/logs.
                let encoded = encode_key(key);
                let uri =
                    zeroize::Zeroizing::new(format!("{GEMINI_ENDPOINT}?key={}", encoded.as_str()));
                uri.as_str()
                    .into_client_request()
                    .map_err(|_| SpeechError::Credential)?
            }
        };
        Self::connect_request_model(model, request, language, quality).await
    }

    // Tests exercise this exact handshake/setup path using only localhost and fake keys.
    // Production callers can only construct requests through the fixed endpoints above.
    #[cfg(test)]
    async fn connect_request(
        provider: SpeechProvider,
        request: tungstenite::handshake::client::Request,
        language: SourceLanguage,
    ) -> Result<Self, SpeechError> {
        Self::connect_request_model(
            LiveSpeechModel::default_for(provider),
            request,
            language,
            TranslationQuality::Standard,
        )
        .await
    }
    async fn connect_request_model(
        model: LiveSpeechModel,
        request: tungstenite::handshake::client::Request,
        language: SourceLanguage,
        quality: TranslationQuality,
    ) -> Result<Self, SpeechError> {
        let provider = model.provider();
        let mut config = WebSocketConfig::default();
        config.max_message_size = Some(MAX_WIRE_BYTES);
        config.max_frame_size = Some(MAX_WIRE_BYTES);
        config.write_buffer_size = 0;
        config.max_write_buffer_size = MAX_WIRE_BYTES * 2;
        let deadline = tokio::time::Instant::now() + CONNECT_TIMEOUT;
        let (mut socket, _) = connection_step(ConnectionStage::Handshake, deadline, async {
            tokio_tungstenite::connect_async_with_config(request, Some(config), false)
                .await
                .map_err(map_socket_error)
        })
        .await?;
        connection_step(
            ConnectionStage::SetupSend,
            deadline,
            send_json(&mut socket, model.plan(language, quality).setup),
        )
        .await?;
        connection_step(ConnectionStage::SetupReceive, deadline, async {
            loop {
                let Some(payload) = read_message(&mut socket, provider).await? else {
                    continue;
                };
                let message = parse(&payload)?;
                check_service_error(&message)?;
                let ready = match provider {
                    SpeechProvider::OpenAi => message["type"] == "session.updated",
                    SpeechProvider::Gemini => match message.get("setupComplete") {
                        Some(value) if value.is_object() => true,
                        Some(_) => return Err(SpeechError::InvalidData),
                        None => false,
                    },
                };
                if ready {
                    return Ok(());
                }
            }
        })
        .await?;
        Ok(Self {
            provider,
            model,
            quality,
            socket: Mutex::new(Some(socket)),
        })
    }
}
async fn connection_step<T>(
    stage: ConnectionStage,
    deadline: tokio::time::Instant,
    work: impl std::future::Future<Output = Result<T, SpeechError>>,
) -> Result<T, SpeechError> {
    tokio::time::timeout_at(deadline, work)
        .await
        .unwrap_or(Err(SpeechError::Timeout))
        .map_err(|cause| SpeechError::Connection {
            stage,
            cause: Box::new(cause),
        })
}
fn encode_key(key: &ApiKey) -> zeroize::Zeroizing<String> {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = zeroize::Zeroizing::new(String::with_capacity(key.expose().len() * 3));
    for byte in key.expose().bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[(byte >> 4) as usize]));
            encoded.push(char::from(HEX[(byte & 15) as usize]));
        }
    }
    encoded
}
impl SpeechRecognizer for NetworkRecognizer {
    fn capabilities(&self) -> SpeechCapabilities {
        SpeechCapabilities {
            streaming: true,
            partials: true,
            cancellation: true,
            supported_languages: [true; 5],
            constrained_auto_detection: self.model == LiveSpeechModel::GeminiTranscribeLive,
        }
    }
    fn run<'a>(
        &'a self,
        language: SourceLanguage,
        audio: SpeechAudio,
        output: TranscriptOutput,
        cancel: Cancellation,
    ) -> SpeechFuture<'a> {
        // Extract ownership synchronously. No mutex is held across await.
        let socket = self
            .socket
            .lock()
            .map_err(|_| SpeechError::InvalidData)
            .and_then(|mut socket| socket.take().ok_or(SpeechError::InvalidData));
        Box::pin(async move {
            if self.model.provider() != self.provider {
                return Err(SpeechError::InvalidData);
            }
            run_socket_model(
                socket?,
                self.model,
                self.quality,
                language,
                audio,
                output,
                cancel,
            )
            .await
        })
    }
}
fn map_socket_error(error: tungstenite::Error) -> SpeechError {
    match error {
        tungstenite::Error::Http(response) => SpeechError::RemoteStatus {
            kind: RemoteStatusKind::Http,
            code: response.status().as_u16(),
            cause: Box::new(match response.status().as_u16() {
                401 | 403 => SpeechError::Credential,
                429 => SpeechError::RateLimited,
                500..=599 => SpeechError::Network,
                _ => SpeechError::InvalidData,
            }),
        },
        // A transport EOF without a WebSocket close handshake is a disconnect,
        // not malformed provider data. Keep other protocol errors terminal.
        tungstenite::Error::Protocol(
            tungstenite::error::ProtocolError::ResetWithoutClosingHandshake,
        ) => SpeechError::Network,
        tungstenite::Error::Capacity(_)
        | tungstenite::Error::Utf8(_)
        | tungstenite::Error::Protocol(_) => SpeechError::InvalidData,
        // tokio-tungstenite's rustls connector wraps handshake validation
        // failures in Io, not Tls. Inspect only the concrete type, never its text.
        tungstenite::Error::Io(error)
            if error
                .get_ref()
                .is_some_and(|cause| cause.is::<rustls::Error>()) =>
        {
            SpeechError::SecureTransport
        }
        tungstenite::Error::Tls(_) => SpeechError::SecureTransport,
        _ => SpeechError::Network,
    }
}
fn parse(payload: &[u8]) -> Result<Value, SpeechError> {
    if payload.len() > MAX_WIRE_BYTES {
        return Err(SpeechError::InvalidData);
    }
    let value: Value = serde_json::from_slice(payload).map_err(|_| SpeechError::InvalidData)?;
    if !value.is_object() {
        return Err(SpeechError::InvalidData);
    }
    Ok(value)
}
fn check_service_error(message: &Value) -> Result<(), SpeechError> {
    if let Some(error) = message.get("error") {
        use super::speech::RequestErrorCode;
        let code = match error["code"].as_str().or_else(|| error["type"].as_str()) {
            Some("insufficient_quota") => Some(RequestErrorCode::InsufficientQuota),
            Some("invalid_request_error") => Some(RequestErrorCode::InvalidRequest),
            Some("invalid_value") => Some(RequestErrorCode::InvalidValue),
            Some("unknown_parameter") => Some(RequestErrorCode::UnknownParameter),
            Some("unsupported_parameter") => Some(RequestErrorCode::UnsupportedParameter),
            Some("model_not_found") => Some(RequestErrorCode::ModelNotFound),
            Some("input_audio_buffer_commit_empty") => Some(RequestErrorCode::EmptyCommit),
            _ => None,
        };
        if let Some(code) = code {
            return Err(SpeechError::RequestRejected {
                code,
                cause: Box::new(SpeechError::InvalidData),
            });
        }
        let result = match error.get("code").and_then(Value::as_str) {
            Some("invalid_api_key" | "authentication_error" | "permission_denied") => {
                SpeechError::Credential
            }
            Some("rate_limit_exceeded") => SpeechError::RateLimited,
            _ => match error.get("code").and_then(Value::as_u64) {
                Some(401 | 403) => SpeechError::Credential,
                Some(429) => SpeechError::RateLimited,
                Some(500..=599) => SpeechError::Network,
                _ => SpeechError::InvalidData,
            },
        };
        return Err(result);
    }
    Ok(())
}
fn runtime_data_error(
    provider: SpeechProvider,
    stage: ConnectionStage,
    error: SpeechError,
) -> SpeechError {
    if provider == SpeechProvider::OpenAi
        && matches!(
            &error,
            SpeechError::InvalidData | SpeechError::RequestRejected { .. }
        )
    {
        SpeechError::Connection {
            stage,
            cause: Box::new(error),
        }
    } else {
        error
    }
}
async fn send_json<S: AsyncRead + AsyncWrite + Unpin>(
    socket: &mut WebSocketStream<S>,
    message: Value,
) -> Result<(), SpeechError> {
    let text = serde_json::to_string(&message).map_err(|_| SpeechError::InvalidData)?;
    tokio::time::timeout(SEND_TIMEOUT, socket.send(Message::Text(text.into())))
        .await
        .map_err(|_| SpeechError::Timeout)?
        .map_err(map_socket_error)
}
async fn read_message<S: AsyncRead + AsyncWrite + Unpin>(
    socket: &mut WebSocketStream<S>,
    provider: SpeechProvider,
) -> Result<Option<Vec<u8>>, SpeechError> {
    match socket
        .next()
        .await
        .ok_or(SpeechError::Network)?
        .map_err(map_socket_error)?
    {
        Message::Text(text) => Ok(Some(text.as_bytes().to_vec())),
        // Gemini's JSON envelope may arrive in a Binary frame. It still passes
        // the caller's same bounded UTF-8/JSON/object validation as Text;
        // these bytes are never interpreted as PCM or decoded audio.
        Message::Binary(bytes) if provider == SpeechProvider::Gemini => {
            if bytes.len() > MAX_WIRE_BYTES {
                return Err(SpeechError::InvalidData);
            }
            Ok(Some(bytes.to_vec()))
        }
        Message::Binary(_) => Err(SpeechError::InvalidData),
        Message::Ping(_) => {
            tokio::time::timeout(SEND_TIMEOUT, socket.flush())
                .await
                .map_err(|_| SpeechError::Timeout)?
                .map_err(map_socket_error)?;
            Ok(None)
        }
        Message::Pong(_) => Ok(None),
        Message::Close(frame) => Err(match frame {
            Some(frame) => {
                let code = u16::from(frame.code);
                SpeechError::RemoteStatus {
                    kind: RemoteStatusKind::WebSocket,
                    code,
                    cause: Box::new(match code {
                        1008 => SpeechError::Credential,
                        1002 | 1003 | 1007 => SpeechError::InvalidData,
                        1013 => SpeechError::RateLimited,
                        1011 => SpeechError::ServiceInternal,
                        _ => SpeechError::Network,
                    }),
                }
            }
            None => SpeechError::Network,
        }),
        Message::Frame(_) => Err(SpeechError::InvalidData),
    }
}
struct Turn {
    key: String,
    context: UtteranceContext,
    frames: u64,
    /// Gemini response inactivity only; never an inferred capture/media timestamp.
    last_response: Option<Instant>,
    final_received: bool,
}
struct Protocol {
    provider: SpeechProvider,
    language: SourceLanguage,
    adapter: TranscriptAdapter,
    current: Option<Turn>,
    pending: VecDeque<(UtteranceContext, Instant)>,
    awaiting_final: Vec<(String, Instant)>,
    turn_id: u64,
    gemini_audio_sent: bool,
    last_gemini_audio: Option<Instant>,
    /// Bounded unknown-language partials, released only after a supported detection.
    held: Vec<super::speech::TranscriptEvent>,
    held_since: Option<Instant>,
    language_end_sent: bool,
    message_sequence: u64,
    client_endpointing: bool,
    early: super::openai_early::EarlyTranscripts,
    openai_prefix: VecDeque<crate::audio::resample::ResampledAudio>,
    openai_prefix_frames: u64,
    prefix_merged: usize,
}
impl Protocol {
    fn new(provider: SpeechProvider, language: SourceLanguage, identity: StreamIdentity) -> Self {
        Self {
            provider,
            language,
            adapter: TranscriptAdapter::new(provider, identity),
            current: None,
            pending: VecDeque::new(),
            awaiting_final: Vec::new(),
            turn_id: 0,
            gemini_audio_sent: false,
            last_gemini_audio: None,
            held: Vec::new(),
            held_since: None,
            language_end_sent: false,
            message_sequence: 0,
            client_endpointing: false,
            early: super::openai_early::EarlyTranscripts::default(),
            openai_prefix: VecDeque::new(),
            openai_prefix_frames: 0,
            prefix_merged: 0,
        }
    }
    fn for_model(
        model: LiveSpeechModel,
        language: SourceLanguage,
        identity: StreamIdentity,
    ) -> Self {
        let mut protocol = Self::new(model.provider(), language, identity);
        protocol.client_endpointing =
            model == LiveSpeechModel::GeminiTranscribeLive && language == SourceLanguage::Auto;
        protocol
    }
    fn audio(&mut self, block: PreparedBlock) -> Result<Vec<Value>, SpeechError> {
        self.prefix_merged = 0;
        self.prune_openai_prefix(Instant::now());
        if Instant::now()
            .checked_duration_since(block.audio.stamp.received)
            .is_none_or(|age| age > MAX_AUDIO_AGE)
        {
            return Err(SpeechError::audio_failure(
                AudioStage::Protocol,
                AudioIssue::Age,
            ));
        }
        if self.provider == SpeechProvider::Gemini {
            // A client endpoint is only a server-finalization request. Server
            // replies still own utterance IDs/finality; fresh PCM never waits.
            self.gemini_audio_sent = true;
            if !self.client_endpointing || block.active {
                self.last_gemini_audio = Some(Instant::now());
            }
            let data = STANDARD.encode(block.audio.pcm_le_bytes().collect::<Vec<_>>());
            let mut messages = vec![
                json!({"realtimeInput":{"audio":{"data":data,"mimeType":"audio/pcm;rate=16000"}}}),
            ];
            if self.client_endpointing && block.end {
                // The preprocessor's quiet/maximum-span endpoint also works
                // before the first transcript. Subsequent quiet PCM must not
                // restart idle finalization. Do not renew any response deadline.
                self.last_gemini_audio = None;
                if self.held_since.is_none() || !self.language_end_sent {
                    // A partial may arrive only after this request. Remember
                    // it before that partial starts the language-wait timer.
                    self.language_end_sent = true;
                    messages.push(json!({"realtimeInput":{"audioStreamEnd":true}}));
                }
            }
            return Ok(messages);
        }
        if !block.active {
            if self.current.is_some() {
                return Err(SpeechError::InvalidData);
            }
            self.remember_openai_prefix(block.audio)?;
            return Ok(Vec::new());
        }
        let mut messages = Vec::with_capacity(3);
        if block.begin {
            if self.current.is_some() {
                return Err(SpeechError::queue_full(SpeechQueue::CurrentTurn, 1));
            }
            self.turn_id = self
                .turn_id
                .checked_add(1)
                .ok_or(SpeechError::InvalidData)?;
            let key = format!("turn-{}", self.turn_id);
            if self.openai_prefix.front().is_some_and(|prefix| {
                prefix.sample_rate != block.audio.sample_rate
                    || prefix.stamp.session != block.audio.stamp.session
            }) {
                return Err(SpeechError::InvalidData);
            }
            let context = UtteranceContext {
                utterance: self.turn_id,
                sequence: self
                    .openai_prefix
                    .front()
                    .map_or(block.audio.stamp.sequence, |prefix| prefix.stamp.sequence),
                start_us: self
                    .openai_prefix
                    .front()
                    .map(|prefix| prefix.start_us())
                    .unwrap_or_else(|| block.audio.start_us()),
                duration_us: None,
                language: None,
            };
            self.current = Some(Turn {
                key,
                context,
                frames: self.openai_prefix_frames,
                last_response: None,
                final_received: false,
            });
        }
        if block.active {
            let turn = self.current.as_mut().ok_or(SpeechError::InvalidData)?;
            if turn.last_response.is_some() || turn.final_received {
                return Err(SpeechError::InvalidData);
            }
            turn.frames += block.audio.frames as u64;
            let mut pcm =
                Vec::with_capacity((self.openai_prefix_frames as usize + block.audio.frames) * 2);
            self.prefix_merged = self.openai_prefix.len();
            for prefix in self.openai_prefix.drain(..) {
                pcm.extend(prefix.pcm_le_bytes());
            }
            self.openai_prefix_frames = 0;
            pcm.extend(block.audio.pcm_le_bytes());
            let data = STANDARD.encode(pcm);
            messages.push(json!({"type":"input_audio_buffer.append","audio":data}));
        }
        if block.end {
            let turn = self.current.as_mut().ok_or(SpeechError::InvalidData)?;
            let duration = turn.frames * 1_000_000 / u64::from(block.audio.sample_rate);
            turn.context.duration_us = Some(duration);
            {
                if self.pending.len() + self.awaiting_final.len() >= EVENT_QUEUE_ITEMS {
                    return Err(SpeechError::queue_full(
                        SpeechQueue::PendingFinal,
                        EVENT_QUEUE_ITEMS,
                    ));
                }
                self.pending.push_back((turn.context, Instant::now()));
                messages.push(json!({"type":"input_audio_buffer.commit"}));
                self.current = None;
            }
        }
        Ok(messages)
    }
    fn pop_openai_prefix(&mut self) {
        if let Some(prefix) = self.openai_prefix.pop_front() {
            self.openai_prefix_frames -= prefix.frames as u64;
        }
    }
    fn prune_openai_prefix(&mut self, now: Instant) {
        while self.openai_prefix.front().is_some_and(|prefix| {
            now.checked_duration_since(prefix.stamp.received)
                .is_none_or(|age| age > OPENAI_PREFIX_MAX_AGE)
        }) {
            self.pop_openai_prefix();
        }
    }
    fn remember_openai_prefix(
        &mut self,
        audio: crate::audio::resample::ResampledAudio,
    ) -> Result<(), SpeechError> {
        if audio.frames == 0 {
            return Ok(());
        }
        if audio.sample_rate == 0
            || self.openai_prefix.back().is_some_and(|previous| {
                previous.sample_rate != audio.sample_rate
                    || previous.stamp.session != audio.stamp.session
            })
        {
            return Err(SpeechError::InvalidData);
        }
        let maximum_frames =
            u64::from(audio.sample_rate) * OPENAI_PREFIX_WINDOW.as_micros() as u64 / 1_000_000;
        self.openai_prefix_frames += audio.frames as u64;
        self.openai_prefix.push_back(audio);
        while self.openai_prefix_frames > maximum_frames
            || self.openai_prefix.len() > super::speech::AUDIO_QUEUE_BLOCKS
        {
            self.pop_openai_prefix();
        }
        Ok(())
    }
    fn idle_input(&mut self, now: Instant, drained: bool) -> Option<Value> {
        if self.provider != SpeechProvider::Gemini
            || (self.held_since.is_some() && self.language_end_sent)
            || !drained
            || self.last_gemini_audio.is_none_or(|last| {
                now.checked_duration_since(last)
                    .is_none_or(|elapsed| elapsed < GEMINI_INPUT_IDLE)
            })
        {
            return None;
        }
        // Exactly once per interruption. Fresh PCM reopens the same stream.
        self.last_gemini_audio = None;
        if self.held_since.is_some() {
            self.language_end_sent = true;
        }
        if let Some(turn) = &mut self.current {
            // The one flush starts the original bounded final-response wait.
            turn.last_response = Some(now);
        }
        Some(json!({"realtimeInput":{"audioStreamEnd":true}}))
    }
    fn language_input_end(&mut self, now: Instant) -> Option<Value> {
        if self.provider != SpeechProvider::Gemini
            || self.language != SourceLanguage::Auto
            || self.language_end_sent
            || self.held_since.is_none_or(|since| {
                now.checked_duration_since(since).is_none_or(|age| {
                    age < AUTO_LANGUAGE_FINALIZE_AFTER || age >= AUTO_LANGUAGE_TIMEOUT
                })
            })
        {
            return None;
        }
        // Request a real server final for the already-sent PCM prefix. Fresh
        // PCM continues on the same socket; this is never a local final/ID/time.
        self.language_end_sent = true;
        self.last_gemini_audio = None;
        Some(json!({"realtimeInput":{"audioStreamEnd":true}}))
    }
    #[cfg(test)]
    fn expired(&self) -> bool {
        self.expiration_error(Instant::now()).is_some()
    }
    fn language_wait_expired(&self, now: Instant) -> bool {
        self.held_since.is_some_and(|since| {
            now.checked_duration_since(since)
                .is_some_and(|age| age >= AUTO_LANGUAGE_TIMEOUT)
        })
    }
    fn expiration_error(&self, now: Instant) -> Option<SpeechError> {
        if self.early.expired(now, FINAL_TIMEOUT) {
            return Some(SpeechError::Connection {
                stage: ConnectionStage::CorrelateTranscript,
                cause: Box::new(SpeechError::Timeout),
            });
        }
        if self.language_wait_expired(now) {
            return Some(SpeechError::LanguageNotReported);
        }
        let response_expired = self
            .pending
            .iter()
            .any(|(_, time)| time.elapsed() > FINAL_TIMEOUT)
            || self
                .awaiting_final
                .iter()
                .any(|(_, time)| time.elapsed() > FINAL_TIMEOUT)
            || self
                .current
                .as_ref()
                .and_then(|turn| turn.last_response)
                .is_some_and(|time| time.elapsed() > FINAL_TIMEOUT);
        response_expired.then_some(SpeechError::Timeout)
    }
    #[cfg(test)]
    fn receive(&mut self, payload: &[u8], output: &TranscriptOutput) -> Result<(), SpeechError> {
        self.receive_assessed(
            payload,
            output,
            &super::language_detection::LanguageAssessment::default(),
        )
    }
    async fn assess_missing_language(
        &self,
        payload: &[u8],
        identity: StreamIdentity,
        cancel: &Cancellation,
    ) -> Result<super::language_detection::LanguageAssessment, SpeechError> {
        if self.provider != SpeechProvider::Gemini
            || self.language != SourceLanguage::Auto
            || self
                .current
                .as_ref()
                .and_then(|turn| turn.context.language)
                .is_some()
        {
            return Ok(super::language_detection::LanguageAssessment::default());
        }
        let message = parse(payload)?;
        let missing_tag_text = |field| -> Result<Option<&str>, SpeechError> {
            let Some(transcription) = message["serverContent"].get(field) else {
                return Ok(None);
            };
            // Do not override a present tag, even if malformed or unsupported.
            if transcription.get("languageCode").is_some() {
                return Ok(None);
            }
            transcription["text"]
                .as_str()
                .map(Some)
                .ok_or(SpeechError::InvalidData)
        };
        super::language_detection::assess_texts(
            identity,
            super::language_detection::LanguageInput {
                message_sequence: self
                    .message_sequence
                    .checked_add(1)
                    .ok_or(SpeechError::InvalidData)?,
                partial: missing_tag_text("interimInputTranscription")?,
                final_text: missing_tag_text("inputTranscription")?,
            },
            cancel,
        )
        .await
    }
    fn receive_assessed(
        &mut self,
        payload: &[u8],
        output: &TranscriptOutput,
        local: &super::language_detection::LanguageAssessment,
    ) -> Result<(), SpeechError> {
        if self.language_wait_expired(Instant::now()) {
            return Err(SpeechError::LanguageNotReported);
        }
        self.message_sequence = self
            .message_sequence
            .checked_add(1)
            .ok_or(SpeechError::InvalidData)?;
        let message = parse(payload)?;
        check_service_error(&message)?;
        if message.get("goAway").is_some() {
            if !message["goAway"].is_object() {
                return Err(SpeechError::InvalidData);
            }
            return Err(SpeechError::SessionExpired);
        }
        if self.provider == SpeechProvider::OpenAi
            && message["type"] == "input_audio_buffer.committed"
        {
            // These are ordered commit acknowledgements on one WebSocket, not STT completions.
            let key = message["item_id"]
                .as_str()
                .ok_or(SpeechError::InvalidData)?;
            let (context, since) = self.pending.pop_front().ok_or(SpeechError::InvalidData)?;
            self.adapter.register(key, context)?;
            self.awaiting_final.push((key.to_owned(), since));
            for message in self.early.take(key).into_iter().flatten() {
                let payload = serde_json::to_vec(&message).map_err(|_| SpeechError::InvalidData)?;
                self.deliver(&payload, &message, output, local)?;
            }
            return Ok(());
        }
        if self.provider == SpeechProvider::OpenAi
            && super::openai_early::EarlyTranscripts::is_event(&message)
            && !message["item_id"]
                .as_str()
                .is_some_and(|key| self.adapter.contains(key))
        {
            if self.pending.is_empty() && self.current.as_ref().is_none_or(|turn| turn.frames == 0)
            {
                return Err(SpeechError::InvalidData);
            }
            self.early.hold(&message, self.language)?;
            output
                .progress
                .increment(super::speech_progress::ProgressCount::EarlyHeld);
            return Ok(());
        }
        self.deliver(payload, &message, output, local)
    }
    fn deliver(
        &mut self,
        payload: &[u8],
        message: &Value,
        output: &TranscriptOutput,
        local: &super::language_detection::LanguageAssessment,
    ) -> Result<(), SpeechError> {
        if self.provider == SpeechProvider::Gemini {
            let content = &message["serverContent"];
            if content.get("interimInputTranscription").is_some()
                || content.get("inputTranscription").is_some()
            {
                if !self.gemini_audio_sent {
                    return Err(SpeechError::InvalidData);
                }
                if self.current.is_none() {
                    self.turn_id = self
                        .turn_id
                        .checked_add(1)
                        .ok_or(SpeechError::InvalidData)?;
                    let key = format!("response-{}", self.turn_id);
                    // Ordered server utterance identity only. The service does
                    // not correlate these events to our individual PCM blocks.
                    let context = UtteranceContext {
                        utterance: self.turn_id,
                        sequence: self.turn_id,
                        start_us: None,
                        duration_us: None,
                        language: None,
                    };
                    self.adapter.register(&key, context)?;
                    self.current = Some(Turn {
                        key,
                        context,
                        frames: 0,
                        last_response: Some(Instant::now()),
                        final_received: false,
                    });
                }
            }
        }
        let gemini_key = self.current.as_ref().map(|turn| turn.key.as_str());
        let mut events = self.adapter.decode(payload, gemini_key)?;
        if self.provider == SpeechProvider::Gemini
            && !events.is_empty()
            && let Some(turn) = &mut self.current
        {
            turn.last_response = Some(Instant::now());
        }
        for event in &mut events {
            let mut locally_estimated = false;
            output.progress.increment(match event.kind {
                TranscriptKind::Partial => super::speech_progress::ProgressCount::Partials,
                TranscriptKind::Final => super::speech_progress::ProgressCount::Finals,
            });
            let codes: Vec<&str> = match self.provider {
                SpeechProvider::OpenAi => match message.get("languages") {
                    Some(value) => value
                        .as_array()
                        .ok_or(SpeechError::InvalidData)?
                        .iter()
                        .map(|entry| entry["code"].as_str().ok_or(SpeechError::InvalidData))
                        .collect::<Result<_, _>>()?,
                    None => Vec::new(),
                },
                SpeechProvider::Gemini => {
                    let field = if event.kind == TranscriptKind::Final {
                        "inputTranscription"
                    } else {
                        "interimInputTranscription"
                    };
                    match message["serverContent"][field].get("languageCode") {
                        Some(value) => vec![value.as_str().ok_or(SpeechError::InvalidData)?],
                        None => Vec::new(),
                    }
                }
            };
            if !codes.is_empty() {
                let supported: Vec<_> = codes
                    .into_iter()
                    .map(|code| {
                        SpeechLanguage::from_code(code).ok_or(SpeechError::UnsupportedLanguage)
                    })
                    .collect::<Result<_, _>>()?;
                if let Some(selected) = SpeechLanguage::selected(self.language)
                    && supported.iter().any(|language| *language != selected)
                {
                    return Err(SpeechError::UnsupportedLanguage);
                }
                // Multiple supported languages are permitted, but not reported as a made-up single language.
                event.language = if supported.iter().all(|language| *language == supported[0]) {
                    Some(supported[0])
                } else {
                    None
                };
                output
                    .progress
                    .language_basis(super::speech_progress::LanguageBasis::Provider);
                if self.provider == SpeechProvider::Gemini
                    && let Some(turn) = &mut self.current
                {
                    // Only metadata actually reported for this server utterance.
                    // Taking/retiring the turn clears it before the next utterance.
                    turn.context.language = event.language;
                    self.held_since = None;
                    output.progress.waiting_language(false);
                }
            } else if self.language == SourceLanguage::Auto {
                if let Some(language) = self
                    .current
                    .as_ref()
                    .filter(|_| self.provider == SpeechProvider::Gemini)
                    .and_then(|turn| turn.context.language)
                {
                    event.language = Some(language);
                    output
                        .progress
                        .language_basis(super::speech_progress::LanguageBasis::Provider);
                } else {
                    let decision = (self.provider == SpeechProvider::Gemini)
                        .then(|| {
                            local.decision(
                                event.identity,
                                self.message_sequence,
                                event.kind,
                                &event.text,
                            )
                        })
                        .flatten();
                    match decision {
                        Some(super::language_detection::LanguageDecision::Supported(language)) => {
                            // Reassess every missing-tag revision. Never store a text estimate
                            // as provider context or assign it to an earlier unassessed partial.
                            event.language = Some(language);
                            locally_estimated = true;
                            self.held_since = None;
                            output.progress.waiting_language(false);
                            output
                                .progress
                                .language_basis(super::speech_progress::LanguageBasis::LocalText);
                            output.progress.increment(
                                super::speech_progress::ProgressCount::LocalLanguageUsed,
                            );
                        }
                        Some(super::language_detection::LanguageDecision::Unsupported) => {
                            output
                                .progress
                                .language_basis(super::speech_progress::LanguageBasis::LocalText);
                            return Err(SpeechError::UnsupportedLanguage);
                        }
                        Some(
                            super::language_detection::LanguageDecision::Uncertain
                            | super::language_detection::LanguageDecision::Busy,
                        ) if event.kind == TranscriptKind::Final => {
                            output
                                .progress
                                .increment(super::speech_progress::ProgressCount::LanguageSkipped);
                            output
                                .progress
                                .language_basis(super::speech_progress::LanguageBasis::Unconfirmed);
                            continue; // Retire only because the service actually finalized this utterance.
                        }
                        _ => {}
                    }
                    if !locally_estimated {
                        if self.provider == SpeechProvider::Gemini {
                            self.held_since.get_or_insert_with(Instant::now);
                            output.progress.waiting_language(true);
                            output
                                .progress
                                .language_basis(super::speech_progress::LanguageBasis::Unconfirmed);
                        }
                        if event.kind == TranscriptKind::Final {
                            return Err(if self.provider == SpeechProvider::Gemini {
                                SpeechError::LanguageNotReported
                            } else {
                                SpeechError::UnsupportedLanguage
                            });
                        }
                        if let Some(existing) = self
                            .held
                            .iter_mut()
                            .find(|held| held.utterance == event.utterance)
                        {
                            existing.revision = event.revision;
                            existing.text = event.text.clone();
                        } else {
                            if self.held.len() >= EVENT_QUEUE_ITEMS {
                                return Err(SpeechError::queue_full(
                                    SpeechQueue::LanguagePartials,
                                    EVENT_QUEUE_ITEMS,
                                ));
                            }
                            self.held.push(super::speech::TranscriptEvent {
                                identity: event.identity,
                                utterance: event.utterance,
                                revision: event.revision,
                                sequence: event.sequence,
                                start_us: event.start_us,
                                duration_us: event.duration_us,
                                language: None,
                                kind: event.kind,
                                text: event.text.clone(),
                            });
                        }
                        continue;
                    }
                }
            }
            if let Some(index) = self
                .held
                .iter()
                .position(|held| held.utterance == event.utterance)
            {
                let mut partial = self.held.remove(index);
                if !locally_estimated {
                    partial.language = event.language;
                    output.emit(partial)?;
                }
            }
            output.emit(super::speech::TranscriptEvent {
                identity: event.identity,
                utterance: event.utterance,
                revision: event.revision,
                sequence: event.sequence,
                start_us: event.start_us,
                duration_us: event.duration_us,
                language: event.language,
                kind: event.kind,
                text: event.text.clone(),
            })?;
        }
        for _event in events
            .iter()
            .filter(|event| event.kind == TranscriptKind::Final)
        {
            match self.provider {
                SpeechProvider::OpenAi => {
                    let key = message["item_id"]
                        .as_str()
                        .ok_or(SpeechError::InvalidData)?;
                    self.adapter.retire(key)?;
                    self.awaiting_final.retain(|(item, _)| item != key);
                }
                SpeechProvider::Gemini => {
                    let turn = self.current.as_mut().ok_or(SpeechError::InvalidData)?;
                    turn.final_received = true;
                }
            }
        }
        // Transcribe Live's final ends this server utterance. It never blocks
        // incoming PCM, and agent turnComplete does not identify an utterance.
        if self.provider == SpeechProvider::Gemini
            && self
                .current
                .as_ref()
                .is_some_and(|turn| turn.last_response.is_some() && turn.final_received)
        {
            let turn = self.current.take().ok_or(SpeechError::InvalidData)?;
            self.adapter.retire(&turn.key)?;
            self.language_end_sent = false;
            self.held
                .retain(|held| held.utterance != turn.context.utterance);
            self.held_since = None;
            output.progress.waiting_language(false);
        }
        Ok(())
    }
}
#[cfg(test)]
async fn run_socket<S: AsyncRead + AsyncWrite + Unpin + Send>(
    socket: WebSocketStream<S>,
    provider: SpeechProvider,
    language: SourceLanguage,
    audio: SpeechAudio,
    output: TranscriptOutput,
    cancel: Cancellation,
) -> Result<(), SpeechError> {
    run_socket_model(
        socket,
        LiveSpeechModel::default_for(provider),
        TranslationQuality::Standard,
        language,
        audio,
        output,
        cancel,
    )
    .await
}
async fn run_socket_model<S: AsyncRead + AsyncWrite + Unpin + Send>(
    mut socket: WebSocketStream<S>,
    model: LiveSpeechModel,
    quality: TranslationQuality,
    language: SourceLanguage,
    mut audio: SpeechAudio,
    output: TranscriptOutput,
    cancel: Cancellation,
) -> Result<(), SpeechError> {
    let provider = model.provider();
    let mut worker = PreprocessWorker::start(
        output.identity(),
        provider.plan(language).pcm_sample_rate,
        cancel.clone(),
        if provider == SpeechProvider::Gemini {
            super::input_level::InputLevelPolicy::BoundedGain
        } else {
            super::input_level::InputLevelPolicy::Original
        },
        match model {
            LiveSpeechModel::GptLiveTranscribe => EndpointPolicy::PreferPauseNearLimit,
            _ => EndpointPolicy::FixedLimit,
        },
    )?;
    let mut protocol = Protocol::for_model(model, language, output.identity());
    let mut native = if model.native_translation() {
        Some(super::live_translate_wire::LiveTranslateWire::new(
            output.identity(),
            language,
            quality,
        )?)
    } else {
        None
    };
    let deadline = tokio::time::sleep(CONNECTION_LIFETIME);
    tokio::pin!(deadline);
    let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut received = Instant::now();
    let mut pending_audio = VecDeque::new();
    let mut preprocessing = 0usize;
    let mut check = tokio::time::interval(Duration::from_millis(100));
    check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        if let Some(error) = cancel.reason() {
            return Err(error);
        }
        protocol.prune_openai_prefix(Instant::now());
        if let Some(error) = protocol.expiration_error(Instant::now()) {
            return Err(error);
        }
        if pending_audio.front().is_some_and(|block: &PreparedBlock| {
            block.audio.stamp.received.elapsed() > MAX_AUDIO_AGE
        }) {
            return Err(SpeechError::audio_failure(
                AudioStage::SendQueue,
                AudioIssue::Age,
            ));
        }
        if let Some(block) = pending_audio.pop_front() {
            for message in protocol.audio(block)? {
                let carries_audio = message["realtimeInput"].get("audio").is_some()
                    || message["type"] == "input_audio_buffer.append";
                let client_endpoint = message["realtimeInput"]["audioStreamEnd"] == true;
                send_json(&mut socket, message).await?;
                if carries_audio {
                    output
                        .progress
                        .increment(super::speech_progress::ProgressCount::AudioSent);
                    for _ in 0..protocol.prefix_merged {
                        output
                            .progress
                            .increment(super::speech_progress::ProgressCount::PrefixBlocks);
                    }
                }
                if client_endpoint {
                    output
                        .progress
                        .increment(super::speech_progress::ProgressCount::AudioEnds);
                    if protocol.held_since.is_some() {
                        output
                            .progress
                            .increment(super::speech_progress::ProgressCount::LanguageEnds);
                    }
                }
            }
        }
        tokio::select! { biased;
            error = cancel.cancelled() => return Err(error),
            _ = &mut deadline => return Err(SpeechError::SessionExpired),
            payload = read_message(&mut socket, provider) => { let payload = payload.map_err(|error| runtime_data_error(provider, ConnectionStage::StreamReceive, error))?; received = Instant::now(); if let Some(payload) = payload {
                output.progress.increment(super::speech_progress::ProgressCount::Messages);
                if let Some(native)=&mut native {
                    let message=parse(&payload)?;check_service_error(&message)?;
                    if message.get("goAway").is_some() {return Err(SpeechError::SessionExpired);}
                    if message.get("serverContent").is_some() && !protocol.gemini_audio_sent {return Err(SpeechError::InvalidData);}
                    if let Some(caption)=native.receive(&message)? {output.emit_caption(caption)?;}
                } else {
                    let local = protocol.assess_missing_language(&payload, output.identity(), &cancel).await?;
                    protocol.receive_assessed(&payload, &output, &local).map_err(|error| runtime_data_error(provider, ConnectionStage::StreamEvent, error))?;
                }
                // This future is also polled inline by the live owner. A ready
                // socket must let that owner consume bounded transcript updates.
                if provider == SpeechProvider::OpenAi { tokio::task::yield_now().await; }
            } },
            _ = check.tick() => {
                if let Some(native)=&mut native && let Some(caption)=native.flush()? {output.emit_caption(caption)?;}
                if let Some(error) = protocol.expiration_error(Instant::now()) { return Err(error); }
                let drained = preprocessing == 0 && pending_audio.is_empty() && audio.is_empty();
                let language_wait = protocol.held_since.is_some();
                if let Some(message) = protocol.language_input_end(Instant::now()).or_else(|| protocol.idle_input(Instant::now(), drained)) {
                    send_json(&mut socket, message).await?;
                    if language_wait { output.progress.increment(super::speech_progress::ProgressCount::LanguageEnds); }
                }
            },
            block = worker.output.recv() => {
                let block = block.ok_or_else(|| SpeechError::audio_failure(AudioStage::Preprocess, AudioIssue::WorkerDisconnected))??;
                if block.boosted { output.progress.increment(super::speech_progress::ProgressCount::Boosted); }
                preprocessing = preprocessing.checked_sub(1).ok_or(SpeechError::InvalidData)?;
                if pending_audio.len() >= super::speech::AUDIO_QUEUE_BLOCKS { return Err(SpeechError::queue_full(SpeechQueue::SendAudio, super::speech::AUDIO_QUEUE_BLOCKS)); }
                pending_audio.push_back(block);
            },
            block = audio.recv() => {
                worker.push(block?)?;
                preprocessing = preprocessing.checked_add(1).ok_or(SpeechError::InvalidData)?;
            },
            _ = heartbeat.tick() => {
                if received.elapsed() > HEARTBEAT_TIMEOUT { return Err(SpeechError::Timeout); }
                tracing::debug!(stt_preprocess_queue=worker.output.len(),stt_send_queue=pending_audio.len(),
                    pending_commits=protocol.pending.len(),pending_finals=protocol.awaiting_final.len(),
                    held_auto_partials=protocol.held.len(),"串流辨識佇列統計");
                tokio::time::timeout(SEND_TIMEOUT, socket.send(Message::Ping(Vec::new().into())))
                    .await.map_err(|_| SpeechError::Timeout)?.map_err(map_socket_error)?;
            },
        }
    }
}

#[cfg(test)]
#[path = "streaming_tests.rs"]
mod tests;
