//! Typed errors with user-safe Display messages. Sources are for programmatic inspection.

use std::io;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, AppError>;

#[derive(Debug, Error)]
pub enum AppError {
    #[error(transparent)]
    Alignment(#[from] crate::session::alignment_control::AlignmentControlError),
    #[error("此程式目前僅支援 Windows。")]
    UnsupportedPlatform,
    #[error("找不到 Windows 本機應用程式資料夾，請確認 LOCALAPPDATA 設定。")]
    ConfigDirectoryUnavailable,
    #[error("無法讀取設定檔，請確認檔案權限與儲存裝置狀態。")]
    ConfigRead {
        #[source]
        source: io::Error,
    },
    #[error("無法儲存設定檔，請確認資料夾權限與可用空間。")]
    ConfigWrite {
        #[source]
        source: io::Error,
    },
    #[error("設定檔格式不正確，請依 README 檢查欄位與選項。")]
    ConfigInvalid {
        #[source]
        source: serde_json::Error,
    },
    #[error("無法產生設定檔內容。")]
    ConfigEncode {
        #[source]
        source: serde_json::Error,
    },
    #[error("設定檔版本 {found} 不受支援；目前支援版本為 {supported}。")]
    UnsupportedConfigVersion { found: u32, supported: u32 },
    #[error("設定檔超過允許的 {limit_bytes} 位元組上限。")]
    ConfigTooLarge { limit_bytes: u64 },
    #[error("日誌層級不正確，SUBTITLENOW_LOG 僅接受 ERROR、WARN、INFO、DEBUG 或 TRACE。")]
    InvalidLogLevel,
    #[error("無法初始化日誌系統。")]
    LoggingInitialization,
    #[error(transparent)]
    Discovery(#[from] crate::audio::sessions::DiscoveryError),
    #[error(transparent)]
    Capture(#[from] crate::audio::capture::CaptureError),
    #[error("命令列參數不正確，請使用 --help 查看可用選項。")]
    InvalidArguments,
    #[error("無法初始化音訊管線的非同步執行環境。")]
    RuntimeInitialization,
    #[error("無法寫入終端機輸出。")]
    ConsoleOutput,
    #[error("無法接收停止訊號。")]
    StopSignal,
    #[error(transparent)]
    Speech(#[from] crate::ai::speech::SpeechError),
    #[error(transparent)]
    Credential(#[from] crate::ai::credentials::CredentialError),
    #[error(transparent)]
    Translation(#[from] crate::ai::translation::TranslationError),
    #[error(transparent)]
    Content(#[from] crate::resolver::contracts::ResolverError),
    #[error("無法建立或更新字幕視窗。")]
    SubtitleWindow,
    #[error(transparent)]
    UiNotification(#[from] crate::ui::bridge::UiBridgeError),
    #[error("無法啟動或回收即時字幕背景工作。")]
    LiveWorker,
}

impl AppError {
    /// Stable identifiers suitable for diagnostics without exposing paths or content.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Alignment(_) => "alignment_control",
            Self::UnsupportedPlatform => "unsupported_platform",
            Self::ConfigDirectoryUnavailable => "config_directory_unavailable",
            Self::ConfigRead { .. } => "config_read",
            Self::ConfigWrite { .. } => "config_write",
            Self::ConfigInvalid { .. } => "config_invalid",
            Self::ConfigEncode { .. } => "config_encode",
            Self::UnsupportedConfigVersion { .. } => "unsupported_config_version",
            Self::ConfigTooLarge { .. } => "config_too_large",
            Self::InvalidLogLevel => "invalid_log_level",
            Self::LoggingInitialization => "logging_initialization",
            Self::Discovery(_) => "audio_discovery",
            Self::Capture(_) => "audio_capture",
            Self::InvalidArguments => "invalid_arguments",
            Self::RuntimeInitialization => "runtime_initialization",
            Self::ConsoleOutput => "console_output",
            Self::StopSignal => "stop_signal",
            Self::Speech(_) => "speech_recognition",
            Self::Credential(_) => "speech_credential",
            Self::Translation(_) => "translation",
            Self::Content(_) => "content_resolver",
            Self::SubtitleWindow => "subtitle_window",
            Self::UiNotification(_) => "ui_notification",
            Self::LiveWorker => "live_worker",
        }
    }
}
