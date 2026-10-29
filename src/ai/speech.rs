//! Bounded streaming contracts. A caller owns/polls the worker future; no detached tasks.
use crate::performance::live::{Activity, Count, LiveDiagnostics};
use crate::{
    audio::{
        capture::{BLOCK_FRAMES, MAX_AUDIO_AGE, SessionId},
        preprocess::NormalizedAudio,
    },
    domain::SourceLanguage,
};
use std::sync::Arc;
use std::{future::Future, pin::Pin, time::Instant};
use tokio::sync::{mpsc, watch};

/// 240 ms of 20 ms capture blocks. A full queue is an explicit restart condition.
pub const AUDIO_QUEUE_BLOCKS: usize = 12;
/// Bound provider language metadata; support regular language/script/region/variant tags.
const MAX_LANGUAGE_TAG_BYTES: usize = 64;
const MAX_LANGUAGE_VARIANTS: usize = MAX_LANGUAGE_TAG_BYTES / 5;
/// Up to 64 bounded transcript updates; overflow never silently loses final text.
pub const EVENT_QUEUE_ITEMS: usize = 64;
pub const MAX_TRANSCRIPT_BYTES: usize = 8192;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamIdentity {
    pub session: SessionId,
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpeechLanguage {
    English,
    Japanese,
    French,
    German,
    Korean,
}
impl SpeechLanguage {
    pub const ALL: [Self; 5] = [
        Self::English,
        Self::Japanese,
        Self::French,
        Self::German,
        Self::Korean,
    ];
    pub const fn iso_code(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::Japanese => "ja",
            Self::French => "fr",
            Self::German => "de",
            Self::Korean => "ko",
        }
    }
    pub const fn locale(self) -> &'static str {
        match self {
            Self::English => "en-US",
            Self::Japanese => "ja-JP",
            Self::French => "fr-FR",
            Self::German => "de-DE",
            Self::Korean => "ko-KR",
        }
    }
    pub const fn selected(source: SourceLanguage) -> Option<Self> {
        match source {
            SourceLanguage::Auto => None,
            SourceLanguage::English => Some(Self::English),
            SourceLanguage::Japanese => Some(Self::Japanese),
            SourceLanguage::French => Some(Self::French),
            SourceLanguage::German => Some(Self::German),
            SourceLanguage::Korean => Some(Self::Korean),
        }
    }
    pub fn from_code(code: &str) -> Option<Self> {
        if code.len() > MAX_LANGUAGE_TAG_BYTES || !code.is_ascii() {
            return None;
        }
        let mut parts = code.split('-').peekable();
        let base = parts.next()?;
        let language = Self::ALL
            .into_iter()
            .find(|language| base.eq_ignore_ascii_case(language.iso_code()))?;
        if parts.peek().is_some_and(|part| {
            part.len() == 4 && part.bytes().all(|byte| byte.is_ascii_alphabetic())
        }) {
            parts.next();
        }
        if parts.peek().is_some_and(|part| {
            (part.len() == 2 && part.bytes().all(|byte| byte.is_ascii_alphabetic()))
                || (part.len() == 3 && part.bytes().all(|byte| byte.is_ascii_digit()))
        }) {
            parts.next();
        }
        let mut variants = [""; MAX_LANGUAGE_VARIANTS];
        for (index, part) in parts.enumerate() {
            let valid = (5..=8).contains(&part.len())
                || (part.len() == 4 && part.as_bytes()[0].is_ascii_digit());
            if index >= variants.len()
                || !valid
                || !part.bytes().all(|byte| byte.is_ascii_alphanumeric())
                || variants[..index]
                    .iter()
                    .any(|previous| part.eq_ignore_ascii_case(previous))
            {
                return None;
            }
            variants[index] = part;
        }
        Some(language)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SpeechCapabilities {
    pub streaming: bool,
    pub partials: bool,
    pub cancellation: bool,
    pub supported_languages: [bool; 5],
    pub constrained_auto_detection: bool,
}
impl SpeechCapabilities {
    pub fn validate(self, language: SourceLanguage) -> Result<(), SpeechError> {
        if !self.streaming || !self.partials || !self.cancellation {
            return Err(SpeechError::UnsupportedCapability);
        }
        let supported = match SpeechLanguage::selected(language) {
            Some(language) => self.supported_languages[language as usize],
            None => {
                self.constrained_auto_detection
                    && self.supported_languages.into_iter().all(|value| value)
            }
        };
        if supported {
            Ok(())
        } else {
            Err(SpeechError::UnsupportedLanguage)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ConnectionStage {
    #[error("建立安全串流連線")]
    Handshake,
    #[error("送出服務初始化設定")]
    SetupSend,
    #[error("等待服務初始化完成")]
    SetupReceive,
    #[error("接收辨識串流")]
    StreamReceive,
    #[error("處理辨識事件")]
    StreamEvent,
    #[error("等待辨識回覆與音訊提交對應")]
    CorrelateTranscript,
}

/// Fixed allowlist only. Never display arbitrary provider error fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RequestErrorCode {
    #[error("insufficient_quota")]
    InsufficientQuota,
    #[error("invalid_request_error")]
    InvalidRequest,
    #[error("invalid_value")]
    InvalidValue,
    #[error("unknown_parameter")]
    UnknownParameter,
    #[error("unsupported_parameter")]
    UnsupportedParameter,
    #[error("model_not_found")]
    ModelNotFound,
    #[error("input_audio_buffer_commit_empty")]
    EmptyCommit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RemoteStatusKind {
    #[error("HTTP 狀態碼")]
    Http,
    #[error("WebSocket 關閉碼")]
    WebSocket,
}

/// Fixed diagnostic labels only; never retain audio, source names or provider data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AudioStage {
    #[error("音訊送入辨識佇列")]
    Input,
    #[error("辨識端取出音訊")]
    Receive,
    #[error("音訊重採樣")]
    Preprocess,
    #[error("準備傳送音訊")]
    Protocol,
    #[error("音訊傳送佇列")]
    SendQueue,
    #[error("等待上一段辨識完成")]
    AwaitingFinal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AudioIssue {
    #[error("音訊已超過允許期限，或本機時間戳記無效")]
    Age,
    #[error("擷取端標記音訊不連續或時間戳記不可靠")]
    DiscontinuityFlag,
    #[error("擷取取樣位置未知，且帶有不連續標記")]
    TimestampUnavailable,
    #[error("音訊區塊序號不連續")]
    SequenceGap,
    #[error("音訊取樣位置不連續")]
    PositionGap,
    #[error("重採樣器拒收音訊")]
    ResamplerRejected,
    #[error("音訊預處理工作已結束")]
    WorkerDisconnected,
}

/// Fixed queue identities; no provider payload or private content.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SpeechQueue {
    #[error("原生翻譯輸出佇列")]
    NativeCaptionOutput,
    #[error("辨識音訊輸入佇列")]
    AudioInput,
    #[error("辨識文字輸出佇列")]
    TranscriptOutput,
    #[error("重採樣輸入佇列")]
    PreprocessInput,
    #[error("重採樣輸出佇列")]
    PreprocessOutput,
    #[error("音訊傳送佇列")]
    SendAudio,
    #[error("未完成的辨識段落")]
    CurrentTurn,
    #[error("等待最終辨識結果的佇列")]
    PendingFinal,
    #[error("等待語言確認的暫定文字佇列")]
    LanguagePartials,
    #[error("等待音訊提交對應的辨識回覆佇列")]
    EarlyTranscripts,
}

/// Safe messages only: no vendor payload, credentials, URLs, or transcript text.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SpeechError {
    #[error("即時服務中斷本段輸出；未將未完成譯文標為完成。")]
    GenerationInterrupted,
    #[error("Live Translate 目前支援快速與標準；高品質校訂請選 Transcribe 模式。")]
    NativeHighUnavailable,
    #[error("此即時模型需明確選擇英文、日文、法文、德文或韓文；目前不支援自動偵測。")]
    ExplicitLanguageRequired,
    #[error("尚未設定語音辨識供應商與串流連線。")]
    NotConfigured,
    #[error("此供應商不支援必要的串流辨識功能。")]
    UnsupportedCapability,
    #[error("語音辨識不支援此語言或限制為五語的自動偵測。")]
    UnsupportedLanguage,
    #[error(
        "自動偵測：本段來源語言未能確認；服務未提供標記或本機判定不確定，請手選英文、日文、法文、德文或韓文。"
    )]
    LanguageNotReported,
    #[error("語音辨識工作階段已停止。")]
    Cancelled,
    #[error("語音辨識佇列已滿；請以新的工作階段重新啟動。")]
    Overloaded,
    #[error("{queue}已滿（上限 {capacity} 項）；{cause}")]
    QueueFull {
        queue: SpeechQueue,
        capacity: usize,
        cause: Box<SpeechError>,
    },
    #[error("語音辨識音訊已過期或不連續；請重新啟動辨識。")]
    AudioDiscontinuity,
    #[error("語音辨識資料格式或工作階段無效。")]
    InvalidData,
    #[error("服務拒絕要求（{code}）：{cause}")]
    RequestRejected {
        code: RequestErrorCode,
        cause: Box<SpeechError>,
    },
    #[error("語音辨識連線逾時。")]
    Timeout,
    #[error("語音辨識網路連線已中斷。")]
    Network,
    #[error("語音辨識服務回報內部錯誤；請稍後重試。")]
    ServiceInternal,
    #[error("語音辨識憑證無效或權限不足。")]
    Credential,
    #[error("語音辨識服務暫時限制要求速率。")]
    RateLimited,
    #[error("語音辨識連線已達使用期限，正在建立新的工作階段。")]
    SessionExpired,
    #[error("無法建立語音辨識預處理工作。")]
    WorkerStart,
    #[error("無法建立受信任的 TLS 連線；請檢查系統憑證與安全網路設定。")]
    SecureTransport,
    #[error("{stage}失敗：{cause}")]
    Connection {
        stage: ConnectionStage,
        cause: Box<SpeechError>,
    },
    #[error("{kind} {code}：{cause}")]
    RemoteStatus {
        kind: RemoteStatusKind,
        code: u16,
        cause: Box<SpeechError>,
    },
    #[error("{stage}：{issue}；{cause}{detail}")]
    AudioFailure {
        stage: AudioStage,
        issue: AudioIssue,
        cause: Box<SpeechError>,
        /// Bounded numeric continuity metadata only, formatted at failure time.
        detail: String,
    },
}
impl SpeechError {
    pub(crate) fn queue_full(queue: SpeechQueue, capacity: usize) -> Self {
        Self::QueueFull {
            queue,
            capacity,
            cause: Box::new(Self::Overloaded),
        }
    }
    pub(crate) fn audio_failure(stage: AudioStage, issue: AudioIssue) -> Self {
        Self::AudioFailure {
            stage,
            issue,
            cause: Box::new(Self::AudioDiscontinuity),
            detail: String::new(),
        }
    }

    pub fn root_cause(&self) -> &Self {
        let mut error = self;
        while let Self::Connection { cause, .. }
        | Self::RemoteStatus { cause, .. }
        | Self::AudioFailure { cause, .. }
        | Self::QueueFull { cause, .. }
        | Self::RequestRejected { cause, .. } = error
        {
            error = cause;
        }
        error
    }
    pub fn retryable(&self) -> bool {
        matches!(
            self.root_cause(),
            Self::Network
                | Self::Timeout
                | Self::RateLimited
                | Self::ServiceInternal
                | Self::GenerationInterrupted
        )
    }
}

#[derive(Clone)]
pub struct Cancellation(watch::Sender<Option<SpeechError>>);
impl Cancellation {
    fn new() -> Self {
        Self(watch::channel(None).0)
    }
    pub fn cancel(&self) {
        self.fail(SpeechError::Cancelled);
    }
    pub(crate) fn fail(&self, error: SpeechError) {
        self.0.send_if_modified(|state| {
            if state.is_none() {
                *state = Some(error);
                true
            } else {
                false
            }
        });
    }
    pub fn reason(&self) -> Option<SpeechError> {
        self.0.borrow().clone()
    }
    pub async fn cancelled(&self) -> SpeechError {
        let mut state = self.0.subscribe();
        loop {
            if let Some(error) = state.borrow_and_update().clone() {
                return error;
            }
            // This Cancellation owns a sender, so the channel cannot close here.
            if state.changed().await.is_err() {
                return SpeechError::Cancelled;
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TranscriptKind {
    Partial,
    Final,
}
/// Full replacement text for an utterance revision, never one bubble per token.
/// Unknown native timing/language stays None; adapters must not fabricate it.
#[derive(Clone)]
pub struct TranscriptEvent {
    pub identity: StreamIdentity,
    pub utterance: u64,
    pub revision: u64,
    /// Ordered recognition output identity; not necessarily a captured block ID.
    /// Capture/media matching also requires explicit, trustworthy timing.
    pub sequence: u64,
    pub start_us: Option<u64>,
    pub duration_us: Option<u64>,
    pub language: Option<SpeechLanguage>,
    pub kind: TranscriptKind,
    pub text: Box<str>,
}

/// One producer is owned by the session. try_push never waits on network or queue space.
pub struct SpeechInput {
    diagnostics: Option<Arc<LiveDiagnostics>>,
    identity: StreamIdentity,
    sender: mpsc::Sender<NormalizedAudio>,
    cancel: Cancellation,
}
impl SpeechInput {
    pub fn queue_depth(&self) -> usize {
        AUDIO_QUEUE_BLOCKS - self.sender.capacity()
    }
    pub fn try_push(&self, audio: NormalizedAudio) -> Result<(), SpeechError> {
        if let Some(error) = self.cancel.reason() {
            return Err(error);
        }
        if audio.stamp.session != self.identity.session
            || audio.frames == 0
            || audio.frames > BLOCK_FRAMES
            || audio.samples[..audio.frames]
                .iter()
                .any(|value| !value.is_finite() || !(-1.0..=1.0).contains(value))
        {
            return Err(SpeechError::InvalidData);
        }
        if Instant::now()
            .checked_duration_since(audio.stamp.received)
            .is_none_or(|age| age > MAX_AUDIO_AGE)
        {
            if let Some(metrics) = &self.diagnostics {
                metrics.increment(Count::AudioExpired);
            }
            let error = SpeechError::audio_failure(AudioStage::Input, AudioIssue::Age);
            self.cancel.fail(error.clone());
            return Err(error);
        }
        if let Some(metrics) = &self.diagnostics {
            metrics.observe_audio(&audio);
        }
        self.sender
            .try_send(audio)
            .map(|()| {
                if let Some(metrics) = &self.diagnostics {
                    metrics.observe(Activity::SpeechAudioQueue, self.queue_depth().max(1));
                }
            })
            .map_err(|error| {
                if let Some(metrics) = &self.diagnostics {
                    metrics.increment(Count::AudioRejected);
                    if matches!(error, mpsc::error::TrySendError::Full(_)) {
                        metrics.observe(Activity::SpeechAudioQueue, AUDIO_QUEUE_BLOCKS);
                    }
                }
                let error = match error {
                    mpsc::error::TrySendError::Full(_) => {
                        SpeechError::queue_full(SpeechQueue::AudioInput, AUDIO_QUEUE_BLOCKS)
                    }
                    mpsc::error::TrySendError::Closed(_) => SpeechError::Cancelled,
                };
                self.cancel.fail(error.clone());
                error
            })
    }
}

pub struct SpeechAudio {
    progress: Arc<super::speech_progress::SpeechProgress>,
    diagnostics: Option<Arc<LiveDiagnostics>>,
    receiver: mpsc::Receiver<NormalizedAudio>,
    cancel: Cancellation,
    previous: Option<PreviousAudio>,
}

#[derive(Clone, Copy)]
struct PreviousAudio {
    sequence: u64,
    position: Option<u64>,
    qpc: Option<u64>,
    frames: usize,
}

impl SpeechAudio {
    /// Query queued real input only; never infer playback time from inactivity.
    pub(crate) fn is_empty(&self) -> bool {
        self.receiver.is_empty()
    }
    pub async fn recv(&mut self) -> Result<NormalizedAudio, SpeechError> {
        let result = tokio::select! { biased;
            error = self.cancel.cancelled() => return Err(error),
            audio = self.receiver.recv() => audio.ok_or(SpeechError::Cancelled)?,
        };
        // Capture's flag also aggregates queue/position gaps. Report observable
        // gaps first without changing which blocks are rejected or retried.
        let issue = if Instant::now()
            .checked_duration_since(result.stamp.received)
            .is_none_or(|age| age > MAX_AUDIO_AGE)
        {
            Some(AudioIssue::Age)
        } else if let Some(previous) = self.previous {
            if previous.sequence.checked_add(1) != Some(result.stamp.sequence) {
                Some(AudioIssue::SequenceGap)
            } else if previous
                .position
                .zip(result.stamp.frame_position)
                .is_some_and(|(position, current)| {
                    position.checked_add(previous.frames as u64) != Some(current)
                })
            {
                Some(AudioIssue::PositionGap)
            } else if result.stamp.discontinuity {
                Some(if result.stamp.frame_position.is_none() {
                    AudioIssue::TimestampUnavailable
                } else {
                    AudioIssue::DiscontinuityFlag
                })
            } else {
                None
            }
        } else {
            None
        };
        if let Some(issue) = issue {
            if let Some(metrics) = &self.diagnostics {
                metrics.increment(Count::AudioExpired);
            }
            let mut error = SpeechError::audio_failure(AudioStage::Receive, issue);
            if let SpeechError::AudioFailure { detail, .. } = &mut error {
                // No absolute QPC, source identity, PCM or transcript is retained.
                let frame_gap = self.previous.and_then(|previous| {
                    previous
                        .position
                        .zip(result.stamp.frame_position)
                        .map(|(position, current)| {
                            i128::from(current) - i128::from(position) - previous.frames as i128
                        })
                });
                let qpc_delta = self.previous.and_then(|previous| {
                    previous
                        .qpc
                        .zip(result.stamp.qpc_100ns)
                        .map(|(previous, current)| i128::from(current) - i128::from(previous))
                });
                let number = |value: Option<i128>| {
                    value.map_or_else(|| "未知".to_owned(), |value| value.to_string())
                };
                *detail = format!(
                    "（序號 {}→{}；前包 {} frames；位置差 {} frames；QPC 間隔 {}×100 ns；標記 {}）",
                    number(self.previous.map(|previous| i128::from(previous.sequence))),
                    result.stamp.sequence,
                    number(self.previous.map(|previous| previous.frames as i128)),
                    number(frame_gap),
                    number(qpc_delta),
                    u8::from(result.stamp.discontinuity)
                );
            }
            self.cancel.fail(error.clone());
            return Err(error);
        }
        self.previous = Some(PreviousAudio {
            sequence: result.stamp.sequence,
            position: result.stamp.frame_position,
            qpc: result.stamp.qpc_100ns,
            frames: result.frames,
        });
        self.progress
            .increment(super::speech_progress::ProgressCount::AudioReceived);
        if let Some(metrics) = &self.diagnostics {
            metrics.audio_consumed(&result);
            metrics.observe(Activity::SpeechAudioQueue, self.receiver.len());
        }
        Ok(result)
    }
}
impl Drop for SpeechAudio {
    fn drop(&mut self) {
        if let Some(metrics) = &self.diagnostics {
            metrics.observe(Activity::SpeechAudioQueue, 0);
        }
    }
}

pub struct TranscriptOutput {
    pub(crate) progress: Arc<super::speech_progress::SpeechProgress>,
    diagnostics: Option<Arc<LiveDiagnostics>>,
    identity: StreamIdentity,
    language: SourceLanguage,
    sender: mpsc::Sender<TranscriptEvent>,
    captions: mpsc::Sender<super::native_caption::NativeCaption>,
    cancel: Cancellation,
}
impl TranscriptOutput {
    pub fn emit_caption(
        &self,
        caption: super::native_caption::NativeCaption,
    ) -> Result<(), SpeechError> {
        if let Some(error) = self.cancel.reason() {
            return Err(error);
        }
        if caption.identity != self.identity
            || caption.sequence == 0
            || caption.revision == 0
            || caption.text.len() > MAX_TRANSCRIPT_BYTES
        {
            return Err(SpeechError::InvalidData);
        }
        self.captions.try_send(caption).map_err(|error| {
            let error = match error {
                mpsc::error::TrySendError::Full(_) => {
                    SpeechError::queue_full(SpeechQueue::NativeCaptionOutput, EVENT_QUEUE_ITEMS)
                }
                mpsc::error::TrySendError::Closed(_) => SpeechError::Cancelled,
            };
            self.cancel.fail(error.clone());
            error
        })
    }
    pub fn identity(&self) -> StreamIdentity {
        self.identity
    }
    pub fn emit(&self, event: TranscriptEvent) -> Result<(), SpeechError> {
        if let Some(error) = self.cancel.reason() {
            return Err(error);
        }
        if event.identity != self.identity
            || event.text.len() > MAX_TRANSCRIPT_BYTES
            || event
                .start_us
                .zip(event.duration_us)
                .is_some_and(|(start, duration)| start.checked_add(duration).is_none())
        {
            return Err(SpeechError::InvalidData);
        }
        if let (Some(selected), Some(detected)) =
            (SpeechLanguage::selected(self.language), event.language)
            && selected != detected
        {
            return Err(SpeechError::UnsupportedLanguage);
        }
        let permit = self.sender.try_reserve().map_err(|error| {
            if let Some(metrics) = &self.diagnostics {
                metrics.increment(Count::SpeechEventsRejected);
                if event.kind == TranscriptKind::Final {
                    metrics.increment(Count::FinalTranscriptsRejected);
                }
                if matches!(error, mpsc::error::TrySendError::Full(_)) {
                    metrics.observe(Activity::SpeechEventQueue, EVENT_QUEUE_ITEMS);
                }
            }
            let error = match error {
                mpsc::error::TrySendError::Full(_) => {
                    SpeechError::queue_full(SpeechQueue::TranscriptOutput, EVENT_QUEUE_ITEMS)
                }
                mpsc::error::TrySendError::Closed(_) => SpeechError::Cancelled,
            };
            self.cancel.fail(error.clone());
            error
        })?;
        if let Some(metrics) = &self.diagnostics {
            metrics.speech_emitted(&event);
        }
        permit.send(event);
        self.progress
            .increment(super::speech_progress::ProgressCount::Emitted);
        if let Some(metrics) = &self.diagnostics {
            metrics.observe(
                Activity::SpeechEventQueue,
                (EVENT_QUEUE_ITEMS - self.sender.capacity()).max(1),
            );
        }
        Ok(())
    }
}

pub type SpeechFuture<'a> = Pin<Box<dyn Future<Output = Result<(), SpeechError>> + Send + 'a>>;
/// Implementations own one persistent stream, honor cancellation, and spawn no orphan tasks.
/// Provider-specific parsing, connection setup, and resampling belong behind this trait.
pub trait SpeechRecognizer: Send + Sync {
    fn capabilities(&self) -> SpeechCapabilities;
    fn run<'a>(
        &'a self,
        language: SourceLanguage,
        audio: SpeechAudio,
        output: TranscriptOutput,
        cancellation: Cancellation,
    ) -> SpeechFuture<'a>;
}

pub struct SpeechSession {
    progress: Arc<super::speech_progress::SpeechProgress>,
    diagnostics: Option<Arc<LiveDiagnostics>>,
    pub input: SpeechInput,
    events: mpsc::Receiver<TranscriptEvent>,
    captions: Option<mpsc::Receiver<super::native_caption::NativeCaption>>,
    cancel: Cancellation,
}
pub struct SpeechTask {
    language: SourceLanguage,
    audio: Option<SpeechAudio>,
    output: Option<TranscriptOutput>,
    cancel: Cancellation,
}
impl SpeechSession {
    pub fn progress_snapshot(&self) -> super::speech_progress::SpeechProgressSnapshot {
        self.progress.snapshot()
    }
    pub fn event_queue_depth(&self) -> usize {
        self.events.len()
    }
    /// No network access or background task; the returned task must be polled by its owner.
    pub fn new(
        identity: StreamIdentity,
        language: SourceLanguage,
        capabilities: SpeechCapabilities,
    ) -> Result<(Self, SpeechTask), SpeechError> {
        Self::diagnosed(identity, language, capabilities, None)
    }
    pub fn diagnosed(
        identity: StreamIdentity,
        language: SourceLanguage,
        capabilities: SpeechCapabilities,
        diagnostics: Option<Arc<LiveDiagnostics>>,
    ) -> Result<(Self, SpeechTask), SpeechError> {
        if diagnostics
            .as_ref()
            .is_some_and(|d| d.identity() != identity)
        {
            return Err(SpeechError::InvalidData);
        }
        capabilities.validate(language)?;
        let cancel = Cancellation::new();
        let progress = Arc::new(super::speech_progress::SpeechProgress::new(identity));
        let (sender, receiver) = mpsc::channel(AUDIO_QUEUE_BLOCKS);
        let (events_sender, events) = mpsc::channel(EVENT_QUEUE_ITEMS);
        let (caption_sender, captions) = mpsc::channel(EVENT_QUEUE_ITEMS);
        Ok((
            Self {
                progress: progress.clone(),
                diagnostics: diagnostics.clone(),
                input: SpeechInput {
                    diagnostics: diagnostics.clone(),
                    identity,
                    sender,
                    cancel: cancel.clone(),
                },
                events,
                captions: Some(captions),
                cancel: cancel.clone(),
            },
            SpeechTask {
                language,
                audio: Some(SpeechAudio {
                    progress: progress.clone(),
                    diagnostics: diagnostics.clone(),
                    receiver,
                    cancel: cancel.clone(),
                    previous: None,
                }),
                output: Some(TranscriptOutput {
                    progress,
                    diagnostics,
                    identity,
                    language,
                    sender: events_sender,
                    captions: caption_sender,
                    cancel: cancel.clone(),
                }),
                cancel,
            },
        ))
    }
    pub fn cancellation(&self) -> Cancellation {
        self.cancel.clone()
    }
    pub async fn next(&mut self) -> Option<TranscriptEvent> {
        let event = tokio::select! { biased; _ = self.cancel.cancelled() => { self.events.close(); while self.events.try_recv().is_ok() {} None }, event = self.events.recv() => event };
        if let Some(metrics) = &self.diagnostics {
            metrics.observe(Activity::SpeechEventQueue, self.events.len());
            if let Some(event) = &event {
                metrics.speech_consumed(event);
            }
        }
        event
    }
    pub fn take_captions(
        &mut self,
    ) -> Option<mpsc::Receiver<super::native_caption::NativeCaption>> {
        self.captions.take()
    }
}
impl Drop for SpeechSession {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(metrics) = &self.diagnostics {
            metrics.observe(Activity::SpeechEventQueue, 0);
        }
    }
}
impl SpeechTask {
    #[cfg(test)]
    pub(crate) fn output_ref_for_test(&self) -> &TranscriptOutput {
        self.output.as_ref().unwrap()
    }
    #[cfg(test)]
    pub(crate) fn take_audio_for_test(&mut self) -> SpeechAudio {
        self.audio.take().unwrap()
    }
    #[cfg(test)]
    pub(crate) fn take_output_for_test(&mut self) -> TranscriptOutput {
        self.output.take().unwrap()
    }
    pub async fn run(mut self, provider: &dyn SpeechRecognizer) -> Result<(), SpeechError> {
        if let Some(error) = self.cancel.reason() {
            return Err(error);
        }
        provider.capabilities().validate(self.language)?;
        let audio = self.audio.take().ok_or(SpeechError::InvalidData)?;
        let output = self.output.take().ok_or(SpeechError::InvalidData)?;
        let _operation = output
            .diagnostics
            .as_ref()
            .map(|metrics| metrics.operation(Activity::SpeechOperation));
        let result = tokio::select! { biased;
            error = self.cancel.cancelled() => Err(error),
            result = provider.run(self.language, audio, output, self.cancel.clone()) => result,
        };
        let result = self.cancel.reason().map_or(result, Err);
        match &result {
            Err(error) => self.cancel.fail(error.clone()),
            Ok(()) => self.cancel.cancel(),
        }
        result
    }
}
impl Drop for SpeechTask {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// Bounded revision gate; final text is terminal, and delayed older utterances stay ordered.
pub struct TranscriptLedger {
    identity: StreamIdentity,
    entries: Vec<TranscriptEvent>,
    retired_through: Option<u64>,
}
impl TranscriptLedger {
    pub fn new(identity: StreamIdentity) -> Self {
        Self {
            identity,
            entries: Vec::new(),
            retired_through: None,
        }
    }
    pub fn entries(&self) -> &[TranscriptEvent] {
        &self.entries
    }
    pub fn apply(&mut self, event: TranscriptEvent) -> bool {
        if event.identity != self.identity
            || event.text.len() > MAX_TRANSCRIPT_BYTES
            || event
                .start_us
                .zip(event.duration_us)
                .is_some_and(|(start, duration)| start.checked_add(duration).is_none())
            || self
                .retired_through
                .is_some_and(|sequence| event.sequence <= sequence)
        {
            return false;
        }
        if let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.utterance == event.utterance)
        {
            let previous = &self.entries[index];
            if previous.kind == TranscriptKind::Final
                || event.revision <= previous.revision
                || event.sequence != previous.sequence
                || event.start_us != previous.start_us
            {
                return false;
            }
            self.entries[index] = event;
        } else {
            if self
                .entries
                .iter()
                .any(|entry| entry.sequence == event.sequence)
            {
                return false;
            }
            self.entries.push(event);
            self.entries
                .sort_by_key(|entry| (entry.sequence, entry.utterance));
        }
        if self.entries.len() > EVENT_QUEUE_ITEMS {
            let retired = self.entries.remove(0);
            self.retired_through = Some(retired.sequence);
        }
        true
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
