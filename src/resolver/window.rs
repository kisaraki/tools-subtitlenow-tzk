//! CPU-worker-only sliding window assembly. Never call from WASAPI callbacks.
use super::contracts::{AudioWindow, ResolverAudioBlock, ResolverError, WindowStamp};
use crate::{
    ai::speech::StreamIdentity,
    audio::capture::{BLOCK_FRAMES, MAX_AUDIO_AGE},
};
use std::{collections::VecDeque, sync::Arc, time::Instant};

/// Initial engineering policy within SPEC's 8–15 s / 2–5 s ranges; not benchmarked.
pub const WINDOW_SECONDS: usize = 12;
pub const INTERVAL_SECONDS: usize = 3;
pub const WINDOW_FRAMES: usize = AudioWindow::SAMPLE_RATE as usize * WINDOW_SECONDS;
pub const INTERVAL_FRAMES: usize = AudioWindow::SAMPLE_RATE as usize * INTERVAL_SECONDS;
/// Normal 20 ms input uses 600 entries; excessive tiny blocks reset instead of growing.
const MAX_WINDOW_BLOCKS: usize = 1024;
const SAMPLE_TIME_TOLERANCE_US: u64 = 1_000_000u64.div_ceil(AudioWindow::SAMPLE_RATE as u64);
struct BlockTime {
    frames: usize,
    consumed: usize,
    start_us: Option<u64>,
}
pub struct SlidingAudioWindow {
    identity: StreamIdentity,
    pcm: VecDeque<i16>,
    times: VecDeque<BlockTime>,
    previous: Option<(u64, Option<u64>, usize, Instant)>,
    since_emit: usize,
    emitted: bool,
    resets: u64,
}
impl SlidingAudioWindow {
    pub fn new(identity: StreamIdentity) -> Self {
        Self {
            identity,
            pcm: VecDeque::with_capacity(WINDOW_FRAMES),
            times: VecDeque::new(),
            previous: None,
            since_emit: 0,
            emitted: false,
            resets: 0,
        }
    }
    pub fn buffered_frames(&self) -> usize {
        self.pcm.len()
    }
    pub fn reset_count(&self) -> u64 {
        self.resets
    }
    pub fn reset(&mut self) {
        self.pcm.clear();
        self.times.clear();
        self.previous = None;
        self.since_emit = 0;
        self.emitted = false;
        self.resets = self.resets.saturating_add(1);
    }
    /// Uses existing 16 kHz resampler output. Gaps discard all previous samples.
    /// A returned Arc is immutable and may be submitted through the latest-only mailbox.
    pub fn push(
        &mut self,
        block: &ResolverAudioBlock,
        now: Instant,
    ) -> Result<Option<Arc<AudioWindow>>, ResolverError> {
        if block.stamp.session != self.identity.session
            || block.sample_rate != AudioWindow::SAMPLE_RATE
            || block.frames == 0
            || block.frames > BLOCK_FRAMES / 3
        {
            return Err(ResolverError::InvalidData);
        }
        if now
            .checked_duration_since(block.stamp.received)
            .is_none_or(|age| age > MAX_AUDIO_AGE)
        {
            self.reset();
            return Err(ResolverError::Stale);
        }
        let start = block.start_us();
        let gap = block.stamp.discontinuity
            || self.previous.is_some_and(|(seq, time, frames, received)| {
                seq.checked_add(1) != Some(block.stamp.sequence)
                    || block.stamp.received < received
                    || time.zip(start).is_some_and(|(previous, current)| {
                        previous
                            .checked_add(
                                frames as u64 * 1_000_000 / u64::from(AudioWindow::SAMPLE_RATE),
                            )
                            .is_none_or(|expected| {
                                expected.abs_diff(current) > SAMPLE_TIME_TOLERANCE_US
                            })
                    })
            });
        if gap || self.times.len() >= MAX_WINDOW_BLOCKS {
            self.reset();
        }
        self.previous = Some((
            block.stamp.sequence,
            start,
            block.frames,
            block.stamp.received,
        ));
        // Evict before inserting, so even the allocation's logical capacity is bounded.
        let mut discard = (self.pcm.len() + block.frames).saturating_sub(WINDOW_FRAMES);
        self.pcm.drain(..discard);
        while discard > 0 {
            let Some(head) = self.times.front_mut() else {
                self.reset();
                return Err(ResolverError::InvalidData);
            };
            let take = discard.min(head.frames - head.consumed);
            head.consumed += take;
            discard -= take;
            if head.consumed == head.frames {
                self.times.pop_front();
            }
        }
        self.pcm.extend(&block.samples[..block.frames]);
        self.times.push_back(BlockTime {
            frames: block.frames,
            consumed: 0,
            start_us: start,
        });
        self.since_emit += block.frames;
        if self.pcm.len() < WINDOW_FRAMES || (self.emitted && self.since_emit < INTERVAL_FRAMES) {
            return Ok(None);
        }
        self.emitted = true;
        self.since_emit = 0;
        let start_us = self.times.front().and_then(|head| {
            head.start_us?
                .checked_add(head.consumed as u64 * 1_000_000 / u64::from(AudioWindow::SAMPLE_RATE))
        });
        Ok(Some(Arc::new(AudioWindow {
            stamp: WindowStamp {
                identity: self.identity,
                sequence: block.stamp.sequence,
                ended_at: block.stamp.received,
                start_us,
                duration_us: WINDOW_SECONDS as u64 * 1_000_000,
            },
            pcm: self.pcm.iter().copied().collect::<Vec<_>>().into(),
        })))
    }
}
