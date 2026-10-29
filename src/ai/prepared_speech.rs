//! Provider-neutral offline speech contracts. PCM and recognized text are
//! bounded, session-only owners; media time is independent of Live freshness.
use super::speech::{SpeechLanguage, StreamIdentity};
use crate::{
    domain::SourceLanguage,
    media::{
        MediaError, decode_worker::MAX_DECODED_MEDIA_DURATION_US,
        decoded_clock::DecodedClockSnapshot, prepared_audio::PreparedSampleRate,
    },
    resolver::contracts::{MAX_CUE_BYTES, ResolverCancellation},
};
use std::{future::Future, pin::Pin, sync::Arc};

pub const BODY_DURATION_MS: u32 = 10_000;
pub const OVERLAP_DURATION_MS: u32 = 500;
pub const MAX_CHUNK_DURATION_MS: u32 = 11_000;
pub const MAX_ASR_BATCH_CHUNKS: usize = 4;
pub const MAX_ASR_CUES_PER_CHUNK: usize = 512;
pub const MAX_ASR_RESPONSE_BYTES: usize = 128 * 1024;
pub const MAX_ASR_DOCUMENT_CUES: usize = 8192;
pub const MAX_ASR_DOCUMENT_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_ASR_CONTEXT_ITEMS: usize = 8;
pub const MAX_ASR_CONTEXT_BYTES: usize = 64 * 1024;
const MICROSECONDS_PER_SECOND: u64 = 1_000_000;
const MICROSECONDS_PER_MILLISECOND: u64 = 1000;
/// Match the existing Prepared frame-clock allowance, never accumulated drift.
const TIMESTAMP_ROUNDING_US: u64 = 1;

/// Recognition/audio sample clock. With a bound source map, these values must
/// be converted after provider validation before publishing a media timeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreparedChunkStamp {
    pub identity: StreamIdentity,
    pub sequence: u64,
    pub run_id: u64,
    pub rate: PreparedSampleRate,
    pub audio_start_us: u64,
    pub body_start_us: u64,
    pub body_end_us: u64,
    pub discontinuity: bool,
}

/// Audio may overlap adjacent chunks; only the body owns recognized content.
/// No Clone, Debug, serialization or permanent audio storage is provided.
pub struct PreparedSpeechChunk {
    stamp: PreparedChunkStamp,
    samples: Box<[i16]>,
    audio_end_us: u64,
    clock: Option<Arc<DecodedClockSnapshot>>,
}
impl PreparedSpeechChunk {
    pub fn new(stamp: PreparedChunkStamp, samples: Vec<i16>) -> Result<Self, PreparedSpeechError> {
        let max_frames = u64::from(stamp.rate.hz()) * u64::from(MAX_CHUNK_DURATION_MS) / 1000;
        let frames = u64::try_from(samples.len()).map_err(|_| PreparedSpeechError::InvalidData)?;
        if stamp.identity.session.0 == 0
            || stamp.identity.generation == 0
            || stamp.sequence == 0
            || stamp.run_id == 0
            || frames == 0
            || frames > max_frames
        {
            return Err(PreparedSpeechError::InvalidData);
        }
        let duration = frames
            .checked_mul(MICROSECONDS_PER_SECOND)
            .ok_or(PreparedSpeechError::InvalidData)?
            / u64::from(stamp.rate.hz());
        let audio_end_us = stamp
            .audio_start_us
            .checked_add(duration)
            .ok_or(PreparedSpeechError::InvalidData)?;
        let body_duration = stamp
            .body_end_us
            .checked_sub(stamp.body_start_us)
            .filter(|duration| *duration > 0)
            .ok_or(PreparedSpeechError::InvalidData)?;
        let prefix = stamp
            .body_start_us
            .checked_sub(stamp.audio_start_us)
            .ok_or(PreparedSpeechError::InvalidData)?;
        let suffix = audio_end_us
            .checked_sub(stamp.body_end_us)
            .ok_or(PreparedSpeechError::InvalidData)?;
        let overlap_limit =
            u64::from(OVERLAP_DURATION_MS) * MICROSECONDS_PER_MILLISECOND + TIMESTAMP_ROUNDING_US;
        if audio_end_us > MAX_DECODED_MEDIA_DURATION_US
            || body_duration > u64::from(BODY_DURATION_MS) * MICROSECONDS_PER_MILLISECOND
            || prefix > overlap_limit
            || suffix > overlap_limit
        {
            return Err(PreparedSpeechError::InvalidData);
        }
        Ok(Self {
            stamp,
            // Length validation alone cannot bound a Vec's reserved capacity.
            // Discard any caller-supplied spare allocation before retaining PCM.
            samples: samples.into_boxed_slice(),
            audio_end_us,
            clock: None,
        })
    }
    /// Bind the entire real audio/context extent to immutable source evidence.
    /// The stamp remains the recognition sample clock; source run and decoded
    /// block sequence are separate. Moving this chunk never clones its PCM.
    pub fn with_clock_map(
        mut self,
        clock: DecodedClockSnapshot,
    ) -> Result<Self, PreparedSpeechError> {
        if self.clock.is_some() {
            return Err(PreparedSpeechError::InvalidData);
        }
        if clock.identity() != self.stamp.identity {
            return Err(PreparedSpeechError::Stale);
        }
        if clock.sample_start_us() != self.stamp.audio_start_us
            || clock.sample_end_us() != self.audio_end_us
        {
            return Err(PreparedSpeechError::BoundaryMismatch);
        }
        clock
            .map_interval(
                self.stamp.identity,
                self.stamp.audio_start_us,
                self.audio_end_us,
            )
            .map_err(map_clock_error)?;
        clock
            .map_interval(
                self.stamp.identity,
                self.stamp.body_start_us,
                self.stamp.body_end_us,
            )
            .map_err(map_clock_error)?;
        self.clock = Some(Arc::new(clock));
        Ok(self)
    }
    /// Borrow metadata only; adapters still send PCM-relative timestamps and
    /// must not serialize this source map into provider payloads.
    pub fn source_clock(&self) -> Option<&DecodedClockSnapshot> {
        self.clock.as_deref()
    }
    pub(crate) fn clock_owner(&self) -> Option<&Arc<DecodedClockSnapshot>> {
        self.clock.as_ref()
    }
    pub const fn stamp(&self) -> PreparedChunkStamp {
        self.stamp
    }
    pub fn samples(&self) -> &[i16] {
        &self.samples
    }
    pub const fn audio_end_us(&self) -> u64 {
        self.audio_end_us
    }
}

pub(crate) fn map_clock_error(error: MediaError) -> PreparedSpeechError {
    match error {
        MediaError::Cancelled => PreparedSpeechError::Cancelled,
        MediaError::Stale => PreparedSpeechError::Stale,
        MediaError::Overloaded | MediaError::TooLarge => PreparedSpeechError::Overloaded,
        _ => PreparedSpeechError::InvalidData,
    }
}

/// Previous source is context data, never an instruction or a transcript cache.
pub struct PreparedSpeechRequest {
    chunk: PreparedSpeechChunk,
    language: SourceLanguage,
    previous_source: Box<[Box<str>]>,
}
impl PreparedSpeechRequest {
    pub fn new(
        chunk: PreparedSpeechChunk,
        language: SourceLanguage,
        previous_source: Vec<Box<str>>,
    ) -> Result<Self, PreparedSpeechError> {
        if previous_source.len() > MAX_ASR_CONTEXT_ITEMS {
            return Err(PreparedSpeechError::InvalidData);
        }
        let mut bytes = 0usize;
        for text in &previous_source {
            validate_text(text)?;
            bytes = bytes
                .checked_add(text.len())
                .filter(|bytes| *bytes <= MAX_ASR_CONTEXT_BYTES)
                .ok_or(PreparedSpeechError::InvalidData)?;
        }
        Ok(Self {
            chunk,
            language,
            previous_source: previous_source.into_boxed_slice(),
        })
    }
    pub const fn chunk(&self) -> &PreparedSpeechChunk {
        &self.chunk
    }
    pub const fn language(&self) -> SourceLanguage {
        self.language
    }
    pub fn previous_source(&self) -> &[Box<str>] {
        &self.previous_source
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PreparedSpeechCapabilities {
    pub cancellation: bool,
    pub timestamped_finals: bool,
    pub supported_languages: [bool; 5],
    pub constrained_auto_detection: bool,
    /// Index zero is 16 kHz; index one is 24 kHz mono PCM16.
    pub sample_rates: [bool; 2],
    pub max_concurrent: usize,
}
impl PreparedSpeechCapabilities {
    pub fn validate(
        self,
        language: SourceLanguage,
        rate: PreparedSampleRate,
    ) -> Result<(), PreparedSpeechError> {
        let rate_index = match rate {
            PreparedSampleRate::Hz16000 => 0,
            PreparedSampleRate::Hz24000 => 1,
        };
        if !self.cancellation
            || !self.timestamped_finals
            || !self.sample_rates[rate_index]
            || !(1..=MAX_ASR_BATCH_CHUNKS).contains(&self.max_concurrent)
        {
            return Err(PreparedSpeechError::UnsupportedCapability);
        }
        let supported = match SpeechLanguage::selected(language) {
            Some(language) => self.supported_languages[language as usize],
            None => {
                self.constrained_auto_detection
                    && self
                        .supported_languages
                        .into_iter()
                        .all(|supported| supported)
            }
        };
        if supported {
            Ok(())
        } else {
            Err(PreparedSpeechError::UnsupportedLanguage)
        }
    }
}

/// Relative to the request audio start. Adapters must retain genuine timing;
/// sentence-level crossing results cannot be trimmed without word evidence.
pub struct PreparedSpeechCue {
    pub start_us: u64,
    pub duration_us: u64,
    pub text: Box<str>,
}
pub struct PreparedSpeechResponse {
    pub identity: StreamIdentity,
    pub sequence: u64,
    pub run_id: u64,
    /// Selected mode may retain the caller's configured source language; this
    /// does not establish that a provider detected or verified that language.
    /// Auto requires verifiable provider language data and constrained support.
    pub language: Option<SpeechLanguage>,
    pub cues: Vec<PreparedSpeechCue>,
}
impl PreparedSpeechResponse {
    pub fn validate_for(&self, request: &PreparedSpeechRequest) -> Result<(), PreparedSpeechError> {
        let stamp = request.chunk.stamp;
        if self.identity != stamp.identity
            || self.sequence != stamp.sequence
            || self.run_id != stamp.run_id
        {
            return Err(PreparedSpeechError::Stale);
        }
        if self.cues.len() > MAX_ASR_CUES_PER_CHUNK
            || (!self.cues.is_empty() && self.language.is_none())
        {
            return Err(PreparedSpeechError::InvalidData);
        }
        if let Some(language) = self.language
            && SpeechLanguage::selected(request.language)
                .is_some_and(|selected| selected != language)
        {
            return Err(PreparedSpeechError::UnsupportedLanguage);
        }
        let mut bytes = 0usize;
        let mut previous_end = None;
        for cue in &self.cues {
            validate_text(&cue.text)?;
            bytes = bytes
                .checked_add(cue.text.len())
                .filter(|bytes| *bytes <= MAX_ASR_RESPONSE_BYTES)
                .ok_or(PreparedSpeechError::InvalidData)?;
            let start = stamp
                .audio_start_us
                .checked_add(cue.start_us)
                .ok_or(PreparedSpeechError::InvalidData)?;
            let end = start
                .checked_add(cue.duration_us)
                .filter(|end| cue.duration_us > 0 && *end <= request.chunk.audio_end_us)
                .ok_or(PreparedSpeechError::InvalidData)?;
            if previous_end.is_some_and(|previous| start < previous) {
                return Err(PreparedSpeechError::InvalidData);
            }
            if start < stamp.body_start_us || end > stamp.body_end_us {
                return Err(PreparedSpeechError::BoundaryMismatch);
            }
            previous_end = Some(end);
        }
        Ok(())
    }
}

pub(crate) fn validate_text(text: &str) -> Result<(), PreparedSpeechError> {
    if text.trim().is_empty()
        || text.len() > MAX_CUE_BYTES
        || text
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        Err(PreparedSpeechError::InvalidData)
    } else {
        Ok(())
    }
}

/// Safe messages contain neither request data nor private provider payloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PreparedSpeechError {
    #[error("尚未設定本次預先語音辨識服務。")]
    NotConfigured,
    #[error("此服務不支援必要的預先語音辨識功能。")]
    UnsupportedCapability,
    #[error("預先語音辨識不支援此語言或限制為五語的自動偵測。")]
    UnsupportedLanguage,
    #[error("本次辨識包含不同來源語言，目前無法可靠合併。")]
    MixedLanguages,
    #[error("本次預先語音辨識已取消。")]
    Cancelled,
    #[error("預先語音辨識工作已達容量上限。")]
    Overloaded,
    #[error("預先語音辨識資料、時間或工作階段無效。")]
    InvalidData,
    #[error("辨識句子跨越分段正文，無法在保留原始時間下可靠合併。")]
    BoundaryMismatch,
    #[error("預先語音辨識要求逾時。")]
    Timeout,
    #[error("預先語音辨識網路連線中斷。")]
    Network,
    #[error("預先語音辨識憑證無效或權限不足。")]
    Credential,
    #[error("預先語音辨識服務暫時限制要求速率。")]
    RateLimited,
    #[error("本次媒體未辨識到可用語音。")]
    NoSpeech,
    #[error("這份預先語音辨識結果已被較新的工作取代。")]
    Stale,
}
impl PreparedSpeechError {
    pub const fn retryable(self) -> bool {
        matches!(self, Self::Timeout | Self::Network | Self::RateLimited)
    }
}

pub type PreparedSpeechFuture<'a> =
    Pin<Box<dyn Future<Output = Result<PreparedSpeechResponse, PreparedSpeechError>> + Send + 'a>>;

/// The caller owns/polls the future and selects the provider explicitly. An
/// adapter must honor request cancellation and spawn no orphan background work.
pub trait PreparedSpeechRecognizer: Send + Sync {
    fn capabilities(&self) -> PreparedSpeechCapabilities;
    fn recognize<'a>(
        &'a self,
        request: &'a PreparedSpeechRequest,
        cancel: ResolverCancellation,
    ) -> PreparedSpeechFuture<'a>;
}

#[cfg(test)]
#[path = "prepared_speech_tests.rs"]
mod tests;
