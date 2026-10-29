//! Provider-neutral contracts. Private audio/text/metadata intentionally lack Debug.
use crate::{
    ai::speech::{SpeechLanguage, StreamIdentity},
    audio::resample::ResampledAudio,
};
use futures_util::future::select_all;
use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::watch;

pub const MAX_CANDIDATES: usize = 4;
pub const MAX_METADATA_BYTES: usize = 1024;
pub const MAX_MOVIE_SIGNALS: usize = 8;
pub const MAX_FINGERPRINT_BYTES: usize = 64 * 1024;
pub const MAX_TEXT_CUES: usize = 8192;
pub const MAX_TEXT_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_CUE_BYTES: usize = 8192;
pub const MAX_WINDOW_AGE: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ContentType {
    #[default]
    Unknown,
    Speech,
    Music,
    MovieOrShow,
    Mixed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubtitleSource {
    LiveAi,
    ExistingSubtitle,
    ExistingLyrics,
    PreparedTranslation,
    Hybrid,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ResolverError {
    #[error("目前來源的視窗線索無法可靠核對，已略過內容查詢。")]
    SourceMetadataUnavailable,
    #[error("內容辨識音訊不連續，正在重新收集音訊。")]
    AudioDiscontinuity,
    #[error("字幕／歌詞來源與已確認內容的資料不符，已拒絕使用。")]
    ContentMismatch,
    #[error("尚未取得歌詞的來源語言，未啟動歌詞查詢。")]
    UnknownTextLanguage,
    #[error("尚未設定內容辨識服務；即時辨識與翻譯可繼續運作。")]
    NotConfigured,
    #[error("內容辨識服務不支援必要功能。")]
    UnsupportedCapability,
    #[error("內容辨識資料格式或工作階段無效。")]
    InvalidData,
    #[error("內容辨識資料已過期，已捨棄本次結果。")]
    Stale,
    #[error("內容辨識工作已取消。")]
    Cancelled,
    #[error("內容辨識服務逾時。")]
    Timeout,
    #[error("內容辨識服務連線中斷。")]
    Network,
    #[error("內容辨識服務暫時限制要求速率。")]
    RateLimited,
    #[error("字幕來源處理已達容量上限，未排入本次工作。")]
    TextOverloaded,
    #[error("內容辨識服務憑證無效或權限不足。")]
    Credential,
    #[error("無法建立內容辨識的 CPU 工作。")]
    WorkerStart,
    #[error("內容辨識的 CPU 工作異常結束。")]
    WorkerPanicked,
    #[error("此音訊視窗沒有足夠訊號可計算可用指紋。")]
    InsufficientSignal,
}
impl ResolverError {
    pub const fn retryable(self) -> bool {
        matches!(self, Self::Timeout | Self::Network | Self::RateLimited)
    }
}
/// A local cancellation scope with a bounded, flat snapshot of its ancestors.
/// Cancelling a child never writes to a parent, and awaiting uses no relay task.
#[derive(Clone)]
pub struct ResolverCancellation(Arc<ResolverCancellationState>);
struct ResolverCancellationState {
    own: watch::Sender<bool>,
    parents: Arc<[watch::Sender<bool>]>,
}
/// Session metadata, provider binding and publication scopes must remain shallow.
const MAX_CANCELLATION_PARENTS: usize = 4;
impl Default for ResolverCancellation {
    fn default() -> Self {
        Self(Arc::new(ResolverCancellationState {
            own: watch::channel(false).0,
            parents: Arc::from([]),
        }))
    }
}
impl ResolverCancellation {
    /// Retain each distinct ancestor channel once, never a recursive token tree.
    pub(crate) fn linked(parent: Self) -> Result<Self, ResolverError> {
        Self::linked_many(&[parent])
    }
    pub(crate) fn linked_many(parents: &[Self]) -> Result<Self, ResolverError> {
        let mut channels: Vec<watch::Sender<bool>> = Vec::new();
        for parent in parents {
            for channel in std::iter::once(&parent.0.own).chain(parent.0.parents.iter()) {
                if !channels.iter().any(|old| old.same_channel(channel)) {
                    if channels.len() == MAX_CANCELLATION_PARENTS {
                        return Err(ResolverError::InvalidData);
                    }
                    channels.push(channel.clone());
                }
            }
        }
        Ok(Self(Arc::new(ResolverCancellationState {
            own: watch::channel(false).0,
            parents: channels.into(),
        })))
    }
    pub fn cancel(&self) {
        self.0.own.send_replace(true);
    }
    pub fn is_cancelled(&self) -> bool {
        *self.0.own.borrow() || self.0.parents.iter().any(|parent| *parent.borrow())
    }
    /// Compare owner identity without exposing the watch channel or its content.
    pub(crate) fn same_scope(&self, other: &Self) -> bool {
        self.0.own.same_channel(&other.0.own)
    }
    pub async fn cancelled(&self) {
        // At most five watch futures. Their receivers are dropped with this
        // future; no task, lock across await, or recursive wait is introduced.
        let waits: Vec<_> = std::iter::once(&self.0.own)
            .chain(self.0.parents.iter())
            .map(|sender| Box::pin(wait_cancelled(sender.subscribe())))
            .collect();
        select_all(waits).await;
    }
}
async fn wait_cancelled(mut receiver: watch::Receiver<bool>) {
    loop {
        if *receiver.borrow_and_update() {
            return;
        }
        if receiver.changed().await.is_err() {
            return;
        }
    }
}
/// Bounded identifier score or measured evidence agreement, never a fabricated
/// match probability. No probability calibration claim.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Confidence(u16);
impl Confidence {
    pub fn new(permille: u16) -> Result<Self, ResolverError> {
        if permille > 1000 {
            Err(ResolverError::InvalidData)
        } else {
            Ok(Self(permille))
        }
    }
    pub const fn permille(self) -> u16 {
        self.0
    }
}
#[derive(Clone, Copy)]
pub struct WindowStamp {
    pub identity: StreamIdentity,
    pub sequence: u64,
    /// Monotonic capture estimate for the last block, not the oldest block in the window.
    pub ended_at: Instant,
    pub start_us: Option<u64>,
    pub duration_us: u64,
}
impl WindowStamp {
    pub fn validate(self, identity: StreamIdentity, now: Instant) -> Result<(), ResolverError> {
        if self.identity != identity
            || self.duration_us == 0
            || self
                .start_us
                .is_some_and(|start| start.checked_add(self.duration_us).is_none())
        {
            return Err(ResolverError::InvalidData);
        }
        if now
            .checked_duration_since(self.ended_at)
            .is_none_or(|age| age > MAX_WINDOW_AGE)
        {
            return Err(ResolverError::Stale);
        }
        Ok(())
    }
}
pub struct AudioWindow {
    pub stamp: WindowStamp,
    pub(crate) pcm: Arc<[i16]>,
}
impl AudioWindow {
    pub const SAMPLE_RATE: u32 = 16000;
    pub fn samples(&self) -> &[i16] {
        &self.pcm
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FingerprintFormat {
    Opaque,
    ChromaprintTest2Compressed,
}
pub struct Fingerprint {
    bytes: Box<[u8]>,
    format: FingerprintFormat,
}
impl Fingerprint {
    pub fn new(bytes: Box<[u8]>) -> Result<Self, ResolverError> {
        if bytes.is_empty() || bytes.len() > MAX_FINGERPRINT_BYTES {
            return Err(ResolverError::InvalidData);
        }
        Ok(Self {
            bytes,
            format: FingerprintFormat::Opaque,
        })
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn format(&self) -> FingerprintFormat {
        self.format
    }
    pub(crate) fn chromaprint(bytes: Box<[u8]>) -> Result<Self, ResolverError> {
        let mut value = Self::new(bytes)?;
        value.format = FingerprintFormat::ChromaprintTest2Compressed;
        Ok(value)
    }
}
/// CPU work must run on a bounded blocking worker, never capture/UI/Tokio executor threads.
pub trait FingerprintExtractor: Send + Sync {
    fn extract(
        &self,
        input: &AudioWindow,
        cancel: &ResolverCancellation,
    ) -> Result<Fingerprint, ResolverError>;
}
#[derive(Clone)]
pub struct SongCandidate {
    pub id: Box<str>,
    pub title: Box<str>,
    pub artist: Option<Box<str>>,
    pub confidence: Confidence,
    /// Service-provided match position; not permission to adopt a lyrics timeline.
    pub match_offset_us: Option<u64>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MovieSignal {
    WindowTitle,
    Filename,
    Process,
    Dialogue,
    Language,
    Duration,
    SubtitleMetadata,
    RemoteMetadata,
}
#[derive(Clone)]
pub struct MovieCandidate {
    pub id: Box<str>,
    pub title: Box<str>,
    pub year: Option<u16>,
    pub confidence: Confidence,
    pub signals: Vec<MovieSignal>,
}
pub struct ContentEvidence {
    pub kind: ContentType,
    pub confidence: Confidence,
    pub songs: Vec<SongCandidate>,
    pub movies: Vec<MovieCandidate>,
}
pub(crate) fn metadata(text: &str) -> bool {
    !text.trim().is_empty()
        && text.len() <= MAX_METADATA_BYTES
        && !text.chars().any(char::is_control)
}
impl ContentEvidence {
    pub fn validate(&self) -> Result<(), ResolverError> {
        if self.songs.len() > MAX_CANDIDATES
            || self.movies.len() > MAX_CANDIDATES
            || (!self.songs.is_empty()
                && !matches!(self.kind, ContentType::Music | ContentType::Mixed))
            || (!self.movies.is_empty()
                && !matches!(self.kind, ContentType::MovieOrShow | ContentType::Mixed))
        {
            return Err(ResolverError::InvalidData);
        }
        for (index, song) in self.songs.iter().enumerate() {
            if !metadata(&song.id)
                || !metadata(&song.title)
                || song
                    .artist
                    .as_deref()
                    .is_some_and(|artist| !metadata(artist))
                || self.songs[..index]
                    .iter()
                    .any(|previous| previous.id == song.id)
            {
                return Err(ResolverError::InvalidData);
            }
        }
        for (index, movie) in self.movies.iter().enumerate() {
            if !metadata(&movie.id)
                || !metadata(&movie.title)
                || movie.signals.is_empty()
                || movie.signals.len() > MAX_MOVIE_SIGNALS
                || movie
                    .signals
                    .iter()
                    .enumerate()
                    .any(|(index, signal)| movie.signals[..index].contains(signal))
                || self.movies[..index]
                    .iter()
                    .any(|previous| previous.id == movie.id)
            {
                return Err(ResolverError::InvalidData);
            }
        }
        Ok(())
    }
}
#[derive(Clone, Copy)]
pub struct ResolverCapabilities {
    pub classification: bool,
    pub songs: bool,
    pub movies: bool,
    pub cancellation: bool,
}
impl ResolverCapabilities {
    pub fn validate(self) -> Result<(), ResolverError> {
        if !self.cancellation || !(self.classification || self.songs || self.movies) {
            Err(ResolverError::UnsupportedCapability)
        } else {
            Ok(())
        }
    }
    pub fn accepts(self, evidence: &ContentEvidence) -> bool {
        (self.classification
            || evidence.kind == ContentType::Unknown
            || (self.songs && evidence.kind == ContentType::Music)
            || (self.movies && evidence.kind == ContentType::MovieOrShow))
            && (self.songs || evidence.songs.is_empty())
            && (self.movies || evidence.movies.is_empty())
    }
}
pub type IdentifyFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ContentEvidence, ResolverError>> + Send + 'a>>;
/// Actual service/extractor details stay in adapters. No implicit cross-provider fallback.
pub trait ContentIdentifier: Send + Sync {
    fn capabilities(&self) -> ResolverCapabilities;
    /// Dropping the future and/or cancelling this request token must release its work.
    fn identify<'a>(
        &'a self,
        window: &'a AudioWindow,
        cancel: ResolverCancellation,
    ) -> IdentifyFuture<'a>;
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TextLanguage {
    TaiwanTraditional,
    OtherTraditional,
    SimplifiedChinese,
    Source(SpeechLanguage),
}
pub struct TextCue {
    pub start_us: Option<u64>,
    pub duration_us: Option<u64>,
    pub text: Box<str>,
}
pub struct ExistingText {
    pub identity: StreamIdentity,
    pub source: SubtitleSource,
    pub content_id: Box<str>,
    pub language: TextLanguage,
    pub cues: Vec<TextCue>,
}
/// Session-only provider publication. Existing providers return unpaired text;
/// references must come from an explicitly configured provider, never a guess.
pub struct ExistingTextBundle {
    pub text: ExistingText,
    pub source_reference: Option<SourceTextReference>,
}
/// A provider-declared reference for the same edit. Matching content identifiers
/// alone do not establish that two tracks have the same timeline or release.
pub struct SourceTextReference {
    pub text: ExistingText,
    pub primary_edition_id: Box<str>,
    pub reference_edition_id: Box<str>,
}
impl SourceTextReference {
    pub fn validate(&self, primary: &ExistingText, query: &TextQuery) -> Result<(), ResolverError> {
        if primary.source != query.source || primary.content_id != query.content_id {
            return Err(ResolverError::InvalidData);
        }
        self.validate_document(primary, query.stamp.identity)
    }
    /// Structural publication validation without inventing a capture query for
    /// a Prepared/owned document. This does not prove edition or playback.
    pub fn validate_document(
        &self,
        primary: &ExistingText,
        identity: StreamIdentity,
    ) -> Result<(), ResolverError> {
        if !matches!(
            primary.language,
            TextLanguage::TaiwanTraditional
                | TextLanguage::OtherTraditional
                | TextLanguage::SimplifiedChinese
        ) || !matches!(self.text.language, TextLanguage::Source(_))
            || self.text.identity != primary.identity
            || self.text.source != primary.source
            || self.text.content_id != primary.content_id
            || !metadata(&self.primary_edition_id)
            || !metadata(&self.reference_edition_id)
            || self.primary_edition_id != self.reference_edition_id
        {
            return Err(ResolverError::InvalidData);
        }
        primary.validate_document(identity, primary.source, &primary.content_id)?;
        self.text
            .validate_document(identity, primary.source, &primary.content_id)
    }
}
impl ExistingTextBundle {
    /// Structural provenance only; actual ASR/cue alignment is checked later.
    pub fn validate(&self, query: &TextQuery) -> Result<(), ResolverError> {
        self.text.validate(query)?;
        if let Some(reference) = &self.source_reference {
            reference.validate(&self.text, query)?;
        }
        Ok(())
    }
}
impl ExistingText {
    /// Contract validation only. Phase 7 owns localization, alignment and adoption.
    pub fn validate(&self, query: &TextQuery) -> Result<(), ResolverError> {
        self.validate_document(query.stamp.identity, query.source, &query.content_id)
    }
    /// Structural document validation also serves Prepared work, which has no
    /// capture window. Live freshness remains on TextQuery/WindowStamp callers.
    pub(crate) fn validate_document(
        &self,
        identity: StreamIdentity,
        source: SubtitleSource,
        content_id: &str,
    ) -> Result<(), ResolverError> {
        if self.identity != identity
            || self.source != source
            || self.content_id.as_ref() != content_id
            || !metadata(&self.content_id)
            || self.cues.is_empty()
            || self.cues.len() > MAX_TEXT_CUES
            || !matches!(
                self.source,
                SubtitleSource::ExistingLyrics | SubtitleSource::ExistingSubtitle
            )
        {
            return Err(ResolverError::InvalidData);
        }
        let mut total = 0usize;
        let mut previous = None;
        for cue in &self.cues {
            total = total
                .checked_add(cue.text.len())
                .ok_or(ResolverError::InvalidData)?;
            if cue.text.trim().is_empty()
                || cue.text.len() > MAX_CUE_BYTES
                || total > MAX_TEXT_BYTES
                || cue
                    .text
                    .chars()
                    .any(|c| c.is_control() && c != '\n' && c != '\r' && c != '\t')
                || cue.start_us.is_some() != cue.duration_us.is_some()
                || (self.source == SubtitleSource::ExistingSubtitle && cue.start_us.is_none())
            {
                return Err(ResolverError::InvalidData);
            }
            if let Some((start, duration)) = cue.start_us.zip(cue.duration_us) {
                if duration == 0
                    || start.checked_add(duration).is_none()
                    || previous.is_some_and(|old| start < old)
                {
                    return Err(ResolverError::InvalidData);
                }
                previous = Some(start);
            }
        }
        Ok(())
    }
}
pub struct TextQuery {
    pub stamp: WindowStamp,
    pub source: SubtitleSource,
    pub content_id: Box<str>,
}
impl TextQuery {
    pub fn validate(&self, now: Instant) -> Result<(), ResolverError> {
        self.stamp.validate(self.stamp.identity, now)?;
        if !metadata(&self.content_id)
            || !matches!(
                self.source,
                SubtitleSource::ExistingLyrics | SubtitleSource::ExistingSubtitle
            )
        {
            return Err(ResolverError::InvalidData);
        }
        Ok(())
    }
}
pub type TextFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Option<ExistingText>, ResolverError>> + Send + 'a>>;
pub type TextBundleFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Option<ExistingTextBundle>, ResolverError>> + Send + 'a>>;
pub trait ExistingTextProvider: Send + Sync {
    fn supports(&self, source: SubtitleSource) -> bool;
    /// Optional Session metadata lifetime. Selection snapshots this guard before
    /// entering its short input gate and keeps it on the published text lease.
    fn publication_guard(&self) -> Option<ResolverCancellation> {
        None
    }
    /// Explicitly configured, bounded, Session-only lookup. Both cancellation
    /// and future Drop must release requests; never write directly to a renderer.
    fn lookup<'a>(&'a self, query: &'a TextQuery, cancel: ResolverCancellation) -> TextFuture<'a>;
    /// Optional paired publication. The default calls the existing provider once
    /// and adds no reference, service, request or automatic fallback.
    fn lookup_bundle<'a>(
        &'a self,
        query: &'a TextQuery,
        cancel: ResolverCancellation,
    ) -> TextBundleFuture<'a> {
        Box::pin(async move {
            self.lookup(query, cancel).await.map(|text| {
                text.map(|text| ExistingTextBundle {
                    text,
                    source_reference: None,
                })
            })
        })
    }
}

struct GuardedTextProvider {
    inner: Arc<dyn ExistingTextProvider>,
    availability: ResolverCancellation,
}
/// Attach Session metadata lifetime without changing the provider's paired
/// publication or transport. No request is started after metadata retirement.
pub(crate) fn guard_provider(
    inner: Arc<dyn ExistingTextProvider>,
    availability: Option<ResolverCancellation>,
) -> Arc<dyn ExistingTextProvider> {
    let Some(availability) = availability else {
        return inner;
    };
    let availability = match inner.publication_guard() {
        Some(inner_guard) if !availability.same_scope(&inner_guard) => {
            ResolverCancellation::linked_many(&[availability, inner_guard]).unwrap_or_else(|_| {
                // This signature cannot return a construction error. Excess
                // ancestry fails closed before any provider I/O can begin.
                let cancelled = ResolverCancellation::default();
                cancelled.cancel();
                cancelled
            })
        }
        _ => availability,
    };
    Arc::new(GuardedTextProvider {
        inner,
        availability,
    })
}
impl ExistingTextProvider for GuardedTextProvider {
    fn supports(&self, source: SubtitleSource) -> bool {
        self.inner.supports(source)
    }
    fn publication_guard(&self) -> Option<ResolverCancellation> {
        Some(self.availability.clone())
    }
    fn lookup<'a>(&'a self, query: &'a TextQuery, cancel: ResolverCancellation) -> TextFuture<'a> {
        Box::pin(async move {
            if self.availability.is_cancelled() || cancel.is_cancelled() {
                return Err(ResolverError::Cancelled);
            }
            let result = tokio::select! { biased;
                _ = self.availability.cancelled() => Err(ResolverError::Cancelled),
                _ = cancel.cancelled() => Err(ResolverError::Cancelled),
                result = self.inner.lookup(query, cancel.clone()) => result,
            };
            if self.availability.is_cancelled() || cancel.is_cancelled() {
                Err(ResolverError::Cancelled)
            } else {
                result
            }
        })
    }
    fn lookup_bundle<'a>(
        &'a self,
        query: &'a TextQuery,
        cancel: ResolverCancellation,
    ) -> TextBundleFuture<'a> {
        Box::pin(async move {
            if self.availability.is_cancelled() || cancel.is_cancelled() {
                return Err(ResolverError::Cancelled);
            }
            let result = tokio::select! { biased;
                _ = self.availability.cancelled() => Err(ResolverError::Cancelled),
                _ = cancel.cancelled() => Err(ResolverError::Cancelled),
                result = self.inner.lookup_bundle(query, cancel.clone()) => result,
            };
            if self.availability.is_cancelled() || cancel.is_cancelled() {
                Err(ResolverError::Cancelled)
            } else {
                result
            }
        })
    }
}

// ResampledAudio is an existing, bounded CPU-worker output. Window assembly accepts
// this value instead of introducing a second WASAPI/resampling pipeline.
pub type ResolverAudioBlock = ResampledAudio;

#[cfg(test)]
#[path = "contracts_cancellation_tests.rs"]
mod cancellation_tests;
