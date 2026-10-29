//! One-run Prepared controls. Private paths and keys are never status payloads.
use crate::{
    ai::{
        credentials::{ApiKey, CredentialError},
        prepared_speech::PreparedSpeechError,
        provider::SpeechProvider,
        translation::TranslationError,
    },
    domain::{SourceLanguage, TranslationQuality},
    media::{
        MediaError,
        embedded_mp4::Mp4SubtitleSelection,
        input::{LocalSubtitleInput, MediaInput},
        planning::PreparedRequest,
        remote_subtitle::PreparedSubtitleUrl,
    },
    resolver::{contracts::TextLanguage, timed_subtitle::SubtitleFormat},
};
use std::path::PathBuf;
use zeroize::Zeroize;

/// Longest decimal spelling of a nonzero MP4 u32 track ID.
const MAX_TRACK_ID_DIGITS: usize = 10;

/// An explicit HTTP(S) scheme selects remote preparation; no filename or
/// extension inference and no whitespace trimming. This is classification only:
/// MediaInput::remote must still validate the complete private URL before work.
pub(crate) fn is_remote_media_text(raw: &str) -> bool {
    raw.get(..7)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("http://"))
        || raw
            .get(..8)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("https://"))
}

/// UI text exists only for this request. No Debug, Clone or serialization:
/// paths and signed URLs are private and are not persistent preferences.
pub struct PreparedInputDraft {
    pub media: String,
    pub subtitle: String,
    pub format: SubtitleFormat,
    pub language: TextLanguage,
}
impl PreparedInputDraft {
    /// Explicit network-text mode only. Missing or invalid text never requests
    /// ASR. Validation is passive, before passwords or any worker is started.
    /// Legacy sidecar/embedded entry points keep their existing contracts.
    pub fn into_https_subtitle_request(self) -> Result<PreparedRequest, MediaError> {
        let media = if is_remote_media_text(&self.media) {
            MediaInput::remote(&self.media)?
        } else {
            MediaInput::local(PathBuf::from(self.media.as_str()))?
        };
        let subtitle = PreparedSubtitleUrl::new(&self.subtitle, self.format, self.language)
            .map_err(|_| MediaError::InvalidInput)?;
        PreparedRequest::new(media, Vec::new())?.with_remote_subtitle(subtitle)
    }

    /// Explicit embedded mode. An empty/invalid ID is never an ASR request.
    /// Keep exact text (no trim, sign, locale digits or filename inference).
    pub fn into_embedded_request(self) -> Result<PreparedRequest, MediaError> {
        if is_remote_media_text(&self.media) {
            MediaInput::remote(&self.media)?;
            return Err(MediaError::UnsupportedUrl);
        }
        if self.subtitle.is_empty()
            || self.subtitle.len() > MAX_TRACK_ID_DIGITS
            || !self.subtitle.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(MediaError::InvalidInput);
        }
        let track_id = self
            .subtitle
            .parse::<u32>()
            .map_err(|_| MediaError::InvalidInput)?;
        let selection = Mp4SubtitleSelection::new(track_id, self.language)?;
        let media = MediaInput::local(PathBuf::from(self.media.as_str()))?;
        PreparedRequest::new(media, Vec::new())?.with_embedded_subtitle(selection)
    }
    /// Validate syntax without opening files on the UI thread. An exactly empty
    /// subtitle field explicitly requests ASR; actual reads belong to the worker.
    /// HTTP(S) text explicitly selects the limited remote WAVE path. Selected
    /// local sidecars retain their declared format/language and prevent ASR.
    /// Do not trim: spaces can be meaningful parts of the selected local path.
    pub fn into_request(self) -> Result<PreparedRequest, MediaError> {
        let media = if is_remote_media_text(&self.media) {
            MediaInput::remote(&self.media)?
        } else {
            MediaInput::local(PathBuf::from(self.media.as_str()))?
        };
        if self.subtitle.is_empty() {
            return PreparedRequest::new(media, Vec::new());
        }
        let subtitle = LocalSubtitleInput::new(
            PathBuf::from(self.subtitle.as_str()),
            self.format,
            self.language,
        )?;
        PreparedRequest::new(media, vec![subtitle])
    }
}
impl Drop for PreparedInputDraft {
    fn drop(&mut self) {
        self.media.zeroize();
        self.subtitle.zeroize();
    }
}
/// Passive optional same-edition reference choice; no I/O or credential access.
pub struct PreparedReferenceDraft {
    pub path: String,
    pub edition: String,
    pub format: Option<SubtitleFormat>,
    pub language: Option<crate::ai::speech::SpeechLanguage>,
}
impl PreparedReferenceDraft {
    pub fn apply(self, request: PreparedRequest) -> Result<PreparedRequest, MediaError> {
        if self.path.is_empty()
            && self.edition.is_empty()
            && self.format.is_none()
            && self.language.is_none()
        {
            return Ok(request);
        }
        let input = LocalSubtitleInput::new(
            PathBuf::from(&self.path),
            self.format.ok_or(MediaError::InvalidSubtitle)?,
            TextLanguage::Source(self.language.ok_or(MediaError::InvalidSubtitle)?),
        )?;
        request.with_source_reference(input, &self.edition)
    }
}
impl Drop for PreparedReferenceDraft {
    fn drop(&mut self) {
        self.path.zeroize();
        self.edition.zeroize();
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PreparedOptions {
    /// Explicit service for ASR and source-text translation. Chinese localization
    /// stays local; speech and translation never switch to another provider.
    pub provider: Option<SpeechProvider>,
    pub quality: TranslationQuality,
    /// Used only without explicit text. Independent of subtitle language;
    /// the current HTTP recognizers reject Auto before any media/cloud work.
    pub source_language: SourceLanguage,
}

/// Typed failures contain no private path, subtitle text or provider payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PreparedUiError {
    #[error("{0}")]
    Alignment(#[from] crate::subtitle::fuzzy_alignment::FuzzyAlignmentError),
    #[error("中文字幕需要明確的同版本來源語言參考軌才能進行播放對齊。")]
    ReferenceRequired,
    #[error("{0}")]
    Media(#[from] MediaError),
    #[error("{0}")]
    Credential(#[from] CredentialError),
    #[error("{0}")]
    Translation(#[from] TranslationError),
    #[error("{0}")]
    Speech(#[from] PreparedSpeechError),
    #[error("語音辨識或來源字幕翻譯需要選擇本次使用的服務。")]
    ProviderRequired,
    #[error("目前的媒體準備工作尚未停止，請稍後再試。")]
    Busy,
    #[error("無法啟動媒體準備工作。")]
    WorkerStart,
    #[error("媒體準備工作異常結束。")]
    WorkerFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreparedUiState {
    Idle,
    Preparing,
    Stopping,
    Ready { cues: usize },
    Cancelled,
    Failed(PreparedUiError),
}
impl PreparedUiState {
    /// Only typed state and cue count enter presentation. Ready proves document
    /// preparation, not playback matching, alignment or playable output.
    pub fn label(self) -> String {
        match self {
            Self::Idle => "媒體準備：尚未啟動".into(),
            Self::Preparing => "媒體準備：正在辨識或處理本次字幕".into(),
            Self::Stopping => "媒體準備：正在取消並釋放本次資料".into(),
            Self::Ready { cues } => {
                format!("字幕準備完成：共 {cues} 段；可保留返回，中文字幕需同版本來源參考軌。")
            }
            Self::Cancelled => "媒體準備：已取消".into(),
            Self::Failed(error) => format!("媒體準備：{error}"),
        }
    }
}

/// Native controls own this interface on the UI thread. Implementations start
/// bounded work and return promptly; poll must not wait for disk or network.
/// Keys are moved to the selected service and never returned in status values.
pub trait PreparedBackend {
    fn take_ready(
        &mut self,
    ) -> Result<crate::session::prepared::PreparedAttachment, PreparedUiError> {
        Err(PreparedUiError::Media(MediaError::InvalidInput))
    }
    fn start(
        &mut self,
        request: PreparedRequest,
        options: PreparedOptions,
        key: Option<ApiKey>,
    ) -> Result<(), PreparedUiError>;
    /// Opt in only after the caller has disclosed the selected HTTPS subtitle
    /// destination and text processing. Backends without this support fail
    /// closed; they must never delegate to an audio-recognition entry point.
    fn start_with_https_subtitles(
        &mut self,
        _request: PreparedRequest,
        _options: PreparedOptions,
        _key: Option<ApiKey>,
    ) -> Result<(), PreparedUiError> {
        Err(PreparedUiError::Media(MediaError::UnsupportedUrl))
    }
    fn poll(&mut self) -> PreparedUiState;
    fn stop(&mut self);
    fn state(&self) -> PreparedUiState;
}

#[cfg(test)]
#[path = "prepared_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "prepared_https_tests.rs"]
mod https_tests;
