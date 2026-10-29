//! Actual capture cursor only. No wall-clock extrapolation, audio storage or seek.
use crate::{
    ai::speech::StreamIdentity,
    audio::{
        capture::{BLOCK_FRAMES, MAX_AUDIO_AGE},
        preprocess::NormalizedAudio,
    },
    subtitle::source_arbiter::{ArbitrationVersion, PlaybackObservation},
};
use std::time::Instant;
/// Metadata span only; no audio is retained by this clock.
pub const MAX_ALIGNMENT_TRANSCRIPT_SPAN_US: u64 = 30_000_000;
pub struct LivePlaybackClock {
    identity: StreamIdentity,
    last: Option<(u64, u64, Instant)>,
    continuity: crate::resolver::contracts::ResolverCancellation,
    continuous_start_us: Option<u64>,
}
/// Sealed observation of actual capture, never a media/playback clock.
#[derive(Clone)]
pub struct CaptureCursor {
    identity: StreamIdentity,
    sequence: u64,
    end_us: u64,
    observed_at: Instant,
    continuity: crate::resolver::contracts::ResolverCancellation,
    start_us: u64,
}
impl CaptureCursor {
    pub(crate) fn covers(&self, range: std::ops::Range<u64>) -> bool {
        self.is_fresh()
            && self.start_us <= range.start
            && range.start < range.end
            && range.end <= self.end_us
    }
    pub fn window(&self) -> Option<crate::resolver::contracts::WindowStamp> {
        if !self.is_fresh() {
            return None;
        }
        let limit = MAX_ALIGNMENT_TRANSCRIPT_SPAN_US;
        let start = self.start_us.max(self.end_us.saturating_sub(limit));
        Some(crate::resolver::contracts::WindowStamp {
            identity: self.identity,
            sequence: self.sequence,
            ended_at: self.observed_at,
            start_us: Some(start),
            duration_us: self.end_us.checked_sub(start)?,
        })
    }
    pub(crate) fn continuity_valid(&self) -> bool {
        !self.continuity.is_cancelled()
    }
    pub fn identity(&self) -> StreamIdentity {
        self.identity
    }
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
    pub fn end_us(&self) -> u64 {
        self.end_us
    }
    pub fn observed_at(&self) -> Instant {
        self.observed_at
    }
    pub fn is_fresh(&self) -> bool {
        !self.continuity.is_cancelled()
            && Instant::now()
                .checked_duration_since(self.observed_at)
                .is_some_and(|age| age <= MAX_AUDIO_AGE)
    }
}
impl LivePlaybackClock {
    pub fn new(identity: StreamIdentity) -> Self {
        Self {
            identity,
            last: None,
            continuity: crate::resolver::contracts::ResolverCancellation::default(),
            continuous_start_us: None,
        }
    }
    /// Returns true when a gap or loss of reliable timestamps requires arbiter
    /// invalidation. This reads metadata only before the block moves to STT.
    pub fn observe(&mut self, audio: &NormalizedAudio) -> bool {
        let previous = self.last.take();
        let end = audio
            .stamp
            .frame_position
            .and_then(|start| start.checked_add(audio.frames as u64));
        if audio.stamp.session != self.identity.session
            || audio.frames == 0
            || audio.frames > BLOCK_FRAMES
            || end.is_none_or(|end| end.checked_mul(1_000_000).is_none())
            || Instant::now()
                .checked_duration_since(audio.stamp.received)
                .is_none_or(|age| age > MAX_AUDIO_AGE)
        {
            self.reset_continuity();
            return previous.is_some();
        }
        let reset = audio.stamp.discontinuity
            || previous.is_some_and(|(sequence, frame_end, time)| {
                sequence.checked_add(1) != Some(audio.stamp.sequence)
                    || Some(frame_end) != audio.stamp.frame_position
                    || audio.stamp.received < time
            });
        if reset {
            self.reset_continuity();
        }
        if self.continuous_start_us.is_none() {
            self.continuous_start_us = audio.start_us();
        }
        if let Some(end) = end {
            self.last = Some((audio.stamp.sequence, end, audio.stamp.received));
        }
        reset
    }
    pub fn observation(&self, version: ArbitrationVersion) -> Option<PlaybackObservation> {
        let (sequence, end, observed_at) = self.last?;
        if version.identity() != self.identity
            || Instant::now()
                .checked_duration_since(observed_at)
                .is_none_or(|age| age > MAX_AUDIO_AGE)
        {
            return None;
        }
        Some(PlaybackObservation {
            version,
            sequence,
            observed_at,
            capture_position_us: end.checked_mul(1_000_000)?
                / u64::from(NormalizedAudio::SAMPLE_RATE),
        })
    }
    pub fn cursor(&self) -> Option<CaptureCursor> {
        let (sequence, end, observed_at) = self.last?;
        if sequence == 0 {
            return None;
        }
        let cursor = CaptureCursor {
            start_us: self.continuous_start_us?,
            identity: self.identity,
            sequence,
            observed_at,
            continuity: self.continuity.clone(),
            end_us: end.checked_mul(1_000_000)? / u64::from(NormalizedAudio::SAMPLE_RATE),
        };
        cursor.is_fresh().then_some(cursor)
    }
    fn reset_continuity(&mut self) {
        self.continuous_start_us = None;
        self.continuity.cancel();
        self.continuity = crate::resolver::contracts::ResolverCancellation::default();
    }
}
impl Drop for LivePlaybackClock {
    fn drop(&mut self) {
        self.continuity.cancel();
    }
}
