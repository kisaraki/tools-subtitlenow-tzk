//! Private Prepared media input and session-only document boundaries.
//! Local sidecars and bounded Windows WAVE/MPEG-1 MP3/AAC (ADTS or single-track
//! MP4) are supported; each container has an explicit content/clock gate.
//! No implicit provider selection, network fallback, playback, or persistent content.
pub mod decode_worker;
pub mod decoded;
pub mod decoded_clock;
pub mod embedded_mp4;
pub mod embedded_tx3g;
pub mod input;
#[cfg(any(windows, test))]
pub(crate) mod local_audio_format;
pub mod planning;
pub mod prepared_asr;
pub mod prepared_audio;
mod prepared_producer;
mod prepared_resample;
pub mod prepared_segments;
pub mod prepared_worker;
pub mod remote_http;
#[cfg(any(windows, test))]
pub mod remote_prepared;
#[cfg(any(windows, test))]
mod remote_prepared_source;
#[cfg(any(windows, test))]
pub mod remote_probe;
pub mod remote_subtitle;
#[cfg(any(windows, test))]
pub mod remote_wave;
pub(crate) mod work;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MediaError {
    #[error("媒體或字幕來源設定無效。")]
    InvalidInput,
    #[error("此 URL 媒體準備方式目前不支援；可改用本機媒體或 Live Mode。")]
    UnsupportedUrl,
    #[error("{0}")]
    Remote(remote_http::RemoteMediaError),
    #[error("{0}")]
    RemoteSubtitle(remote_subtitle::RemoteSubtitleFailure),
    #[error("來源必須是可讀取的一般本機檔案。")]
    NotFile,
    #[error("無法讀取本次媒體或字幕來源。")]
    Unavailable,
    #[error("媒體或字幕超過本次準備的容量上限。")]
    TooLarge,
    #[error("字幕格式、時間或本文無效。")]
    InvalidSubtitle,
    #[error("媒體音訊的格式、時間或資料無效。")]
    InvalidAudio,
    #[error("媒體音訊過短，無法產生可供辨識的音訊區塊。")]
    InsufficientAudio,
    #[error("此媒體格式或音訊編碼目前不支援預先解碼，可改用 Live Mode。")]
    UnsupportedMedia,
    #[error("此媒體的原生時間尚無法可靠對應本次辨識時鐘；請改用支援格式或 Live Mode。")]
    UnsupportedAudioClock,
    #[error("此媒體受內容保護，無法預先解碼；不會嘗試繞過保護。")]
    ProtectedMedia,
    #[error("無法啟動本機媒體解碼服務。")]
    DecoderUnavailable,
    #[error("此字幕含尚未支援的格式或標記。")]
    UnsupportedSubtitle,
    #[error("此來源字幕需要設定本次使用的翻譯服務。")]
    TranslationRequired,
    #[error("{0}")]
    Translation(crate::ai::translation::TranslationError),
    #[error("{0}")]
    Speech(crate::ai::prepared_speech::PreparedSpeechError),
    #[error("未提供可用字幕；Prepared ASR 尚未接入，可改用 Live Mode。")]
    AsrUnavailable,
    #[error("相同優先順序有多份字幕，請明確選擇本次來源。")]
    AmbiguousSubtitle,
    #[error("媒體準備已取消。")]
    Cancelled,
    #[error("媒體準備工作異常結束。")]
    WorkerFailed,
    #[error("媒體準備工作已達容量上限，未排入本次要求。")]
    Overloaded,
    #[error("這份媒體準備結果已被較新的工作取代。")]
    Stale,
    #[error("無法分配新的媒體工作階段。")]
    SessionExhausted,
}
