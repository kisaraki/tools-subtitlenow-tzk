//! Provider-neutral, pull-based decoded PCM. Audio belongs to the current
//! Prepared session and has no file writer, serialization, or retained history.
use super::MediaError;
use crate::{ai::speech::StreamIdentity, resolver::contracts::ResolverCancellation};

/// One owned block holds at most 64 KiB of interleaved signed PCM16.
pub const MAX_DECODED_BLOCK_BYTES: usize = 64 * 1024;
pub const MAX_DECODED_BLOCK_SAMPLES: usize = MAX_DECODED_BLOCK_BYTES / size_of::<i16>();
pub const MIN_DECODED_SAMPLE_RATE: u32 = 8_000;
pub const MAX_DECODED_SAMPLE_RATE: u32 = 192_000;
pub const MAX_DECODED_CHANNELS: u16 = 8;
/// Flooring separate sample-derived start and duration values can differ from
/// flooring their sum by one microsecond. This is rounding, not a timing gap.
pub const DECODED_TIMESTAMP_ROUNDING_US: u64 = 1;
const MICROSECONDS_PER_SECOND: u64 = 1_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecodedAudioFormat {
    sample_rate: u32,
    channels: u16,
}
impl DecodedAudioFormat {
    pub fn new(sample_rate: u32, channels: u16) -> Result<Self, MediaError> {
        if !(MIN_DECODED_SAMPLE_RATE..=MAX_DECODED_SAMPLE_RATE).contains(&sample_rate)
            || !(1..=MAX_DECODED_CHANNELS).contains(&channels)
        {
            return Err(MediaError::InvalidAudio);
        }
        Ok(Self {
            sample_rate,
            channels,
        })
    }
    pub const fn sample_rate(self) -> u32 {
        self.sample_rate
    }
    pub const fn channels(self) -> u16 {
        self.channels
    }
}

/// Immutable audio with actual media time, never a wall-clock or Live stamp.
/// Private PCM deliberately has no Debug, Clone, or serialization implementation.
pub struct DecodedAudioBlock {
    identity: StreamIdentity,
    sequence: u64,
    timestamp_us: u64,
    duration_us: u64,
    end_us: u64,
    format: DecodedAudioFormat,
    samples: Box<[i16]>,
}
impl DecodedAudioBlock {
    pub fn new(
        identity: StreamIdentity,
        sequence: u64,
        timestamp_us: u64,
        format: DecodedAudioFormat,
        samples: Vec<i16>,
    ) -> Result<Self, MediaError> {
        validate_identity(identity)?;
        if sequence == 0
            || samples.is_empty()
            || samples.len() > MAX_DECODED_BLOCK_SAMPLES
            || !samples.len().is_multiple_of(usize::from(format.channels))
        {
            return Err(MediaError::InvalidAudio);
        }
        let frames = samples.len() / usize::from(format.channels);
        let duration_us = u64::try_from(frames)
            .ok()
            .and_then(|value| value.checked_mul(MICROSECONDS_PER_SECOND))
            .map(|value| value / u64::from(format.sample_rate))
            .filter(|value| *value > 0)
            .ok_or(MediaError::InvalidAudio)?;
        let end_us = timestamp_us
            .checked_add(duration_us)
            .ok_or(MediaError::InvalidAudio)?;
        Ok(Self {
            identity,
            sequence,
            timestamp_us,
            duration_us,
            end_us,
            format,
            samples: samples.into_boxed_slice(),
        })
    }
    pub const fn identity(&self) -> StreamIdentity {
        self.identity
    }
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }
    pub const fn timestamp_us(&self) -> u64 {
        self.timestamp_us
    }
    pub const fn duration_us(&self) -> u64 {
        self.duration_us
    }
    /// Overflow was rejected at construction; audio and timing are immutable.
    pub const fn end_us(&self) -> u64 {
        self.end_us
    }
    pub const fn format(&self) -> DecodedAudioFormat {
        self.format
    }
    pub fn samples(&self) -> &[i16] {
        &self.samples
    }
    pub fn frames(&self) -> usize {
        self.samples.len() / usize::from(self.format.channels)
    }

    /// Private clock bridge: move the existing bounded allocation unchanged.
    /// Only the explicit decoded-clock owner may expose this as sample time;
    /// its separate span retains the observed native media interval.
    pub(super) fn into_sample_clock(mut self, timestamp_us: u64) -> Result<Self, MediaError> {
        let end_us = timestamp_us
            .checked_add(self.duration_us)
            .ok_or(MediaError::InvalidAudio)?;
        self.timestamp_us = timestamp_us;
        self.end_us = end_us;
        Ok(self)
    }
}

/// A decoder is owned and called on one bounded worker. It is intentionally not
/// Send: native COM objects must stay on the thread that created them. `next`
/// returns one bounded block and checks cancellation before and after native
/// decoding. End of stream is `None`; malformed, unsupported, or protected media
/// must return an explicit error, with no decoder or provider fallback.
pub trait MediaDecoder {
    fn format(&self) -> DecodedAudioFormat;
    fn next(
        &mut self,
        cancel: &ResolverCancellation,
    ) -> Result<Option<DecodedAudioBlock>, MediaError>;
}

/// Constant-memory ordering and session check; it retains no audio. A forward
/// gap is reported so downstream preprocessing can reset instead of inventing
/// continuity. Seeking requires a new gate and is outside this decode contract.
pub struct DecodedAudioGate {
    identity: StreamIdentity,
    format: DecodedAudioFormat,
    next_sequence: u64,
    previous: Option<(u64, u64)>,
}
impl DecodedAudioGate {
    pub fn new(identity: StreamIdentity, format: DecodedAudioFormat) -> Result<Self, MediaError> {
        validate_identity(identity)?;
        Ok(Self {
            identity,
            format,
            next_sequence: 1,
            previous: None,
        })
    }
    /// Return true for a forward timing gap exceeding the rounding tolerance.
    /// Rejected blocks never advance the accepted sequence or timestamp.
    pub fn validate(&mut self, block: &DecodedAudioBlock) -> Result<bool, MediaError> {
        if block.identity != self.identity {
            return Err(MediaError::Stale);
        }
        if block.format != self.format || block.sequence != self.next_sequence {
            return Err(MediaError::InvalidAudio);
        }
        let next_sequence = block
            .sequence
            .checked_add(1)
            .ok_or(MediaError::InvalidAudio)?;
        let mut discontinuity = false;
        if let Some((previous_start, previous_end)) = self.previous {
            if block.timestamp_us < previous_start
                || previous_end.saturating_sub(block.timestamp_us) > DECODED_TIMESTAMP_ROUNDING_US
            {
                return Err(MediaError::InvalidAudio);
            }
            discontinuity =
                block.timestamp_us.saturating_sub(previous_end) > DECODED_TIMESTAMP_ROUNDING_US;
        }
        self.next_sequence = next_sequence;
        self.previous = Some((block.timestamp_us, block.end_us));
        Ok(discontinuity)
    }
}

fn validate_identity(identity: StreamIdentity) -> Result<(), MediaError> {
    if identity.session.0 == 0 || identity.generation == 0 {
        Err(MediaError::InvalidAudio)
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "decoded_tests.rs"]
mod tests;
