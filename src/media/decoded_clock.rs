//! Explicit sample/native clock bridge. Normalized PCM time is for DSP/ASR,
//! never playback evidence. Only observed spans can restore media cue times.
//! Packet-clock ASR chunks retain their maps through ordered result merging.
use super::{
    MediaError,
    decode_worker::MAX_DECODED_MEDIA_DURATION_US,
    decoded::{DecodedAudioBlock, DecodedAudioFormat, DecodedAudioGate},
};
use crate::{ai::speech::StreamIdentity, resolver::contracts::ResolverCancellation};
use std::collections::VecDeque;

const MICROSECONDS_PER_SECOND: u64 = 1_000_000;
/// One 11-second ASR chunk plus a bounded decoder pull/lookahead. Owners must
/// prune spans after all outstanding chunks/context have taken their maps.
/// Public windows bound the full source extent; the native producer privately
/// bounds usable coverage and keeps one uncut source row before its watermark.
pub const MAX_CLOCK_WINDOW_DURATION_US: u64 = 12 * MICROSECONDS_PER_SECOND;
/// Packet metadata must also remain bounded for pathological tiny samples.
/// Capacity exhaustion rejects work; it never drops still-needed evidence.
/// AAC uses 1024-frame packets: 12 seconds at 48 kHz needs up to 564 rows.
/// Keep room for boundary rows while preserving the independent 12-second cap.
pub const MAX_CLOCK_WINDOW_SPANS: usize = 640;

/// Metadata only: no PCM, text, filename, serialization or persistent cache.
/// Private fields prevent callers inventing native timing evidence.
/// Native start is the decoder timestamp; native end is that start plus the
/// duration of the real decoded frames, not an IMF sample-duration observation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecodedClockSpan {
    identity: StreamIdentity,
    sequence: u64,
    run_id: u64,
    format: DecodedAudioFormat,
    sample_run_origin_us: u64,
    source_frame_start: u64,
    source_frame_end: u64,
    sample_start_us: u64,
    sample_end_us: u64,
    native_start_us: u64,
    native_end_us: u64,
    discontinuity: bool,
}
impl DecodedClockSpan {
    pub const fn identity(self) -> StreamIdentity {
        self.identity
    }
    pub const fn sequence(self) -> u64 {
        self.sequence
    }
    pub const fn run_id(self) -> u64 {
        self.run_id
    }
    pub const fn format(self) -> DecodedAudioFormat {
        self.format
    }
    pub const fn sample_run_origin_us(self) -> u64 {
        self.sample_run_origin_us
    }
    pub const fn source_frame_start(self) -> u64 {
        self.source_frame_start
    }
    pub const fn source_frame_end(self) -> u64 {
        self.source_frame_end
    }
    pub const fn sample_start_us(self) -> u64 {
        self.sample_start_us
    }
    pub const fn sample_end_us(self) -> u64 {
        self.sample_end_us
    }
    pub const fn native_start_us(self) -> u64 {
        self.native_start_us
    }
    pub const fn native_end_us(self) -> u64 {
        self.native_end_us
    }
    pub const fn discontinuity(self) -> bool {
        self.discontinuity
    }

    fn at(self, sample_us: u64) -> Result<u64, MediaError> {
        if sample_us == self.sample_end_us {
            // Cumulative frame rounding can make a sample span one microsecond
            // longer than the independently rounded native block duration.
            // Use the decoded-frame endpoint, not accumulated offset drift.
            return Ok(self.native_end_us);
        }
        let offset = sample_us
            .checked_sub(self.sample_start_us)
            .filter(|_| sample_us < self.sample_end_us)
            .ok_or(MediaError::InvalidAudio)?;
        // Interpolate within this observed PCM interval, rounding once. The
        // independently rounded native duration and cumulative sample duration
        // differ by at most one microsecond; do not carry that error forward.
        let sample_duration = self
            .sample_end_us
            .checked_sub(self.sample_start_us)
            .filter(|duration| *duration > 0)
            .ok_or(MediaError::InvalidAudio)?;
        let native_duration = self
            .native_end_us
            .checked_sub(self.native_start_us)
            .filter(|duration| *duration > 0)
            .ok_or(MediaError::InvalidAudio)?;
        let native_offset = offset
            .checked_mul(native_duration)
            .ok_or(MediaError::InvalidAudio)?
            / sample_duration;
        self.native_start_us
            .checked_add(native_offset)
            .filter(|time| *time <= self.native_end_us)
            .ok_or(MediaError::InvalidAudio)
    }
}

/// The moved PCM and its clock evidence are handed out together. No audio
/// allocation is cloned, retimed in place behind a borrow, trimmed or filled.
pub struct MappedDecodedAudioBlock {
    sample_clock: DecodedAudioBlock,
    span: DecodedClockSpan,
}
impl MappedDecodedAudioBlock {
    pub const fn span(&self) -> DecodedClockSpan {
        self.span
    }
    pub fn sample_clock_block(&self) -> &DecodedAudioBlock {
        &self.sample_clock
    }
    pub fn into_parts(self) -> (DecodedAudioBlock, DecodedClockSpan) {
        (self.sample_clock, self.span)
    }
}

/// Constant-size normalization state. Native validation remains unchanged;
/// a forward native gap creates a new run with the same measured gap in sample
/// time. Continuous packet rounding is represented by spans, never a relaxed
/// DSP tolerance or an origin reset masquerading as a new native run.
pub struct DecodedClockNormalizer {
    gate: DecodedAudioGate,
    format: DecodedAudioFormat,
    cancel: ResolverCancellation,
    previous: Option<DecodedClockSpan>,
    run_origin_us: u64,
    run_frames: u64,
    terminal: bool,
}
impl DecodedClockNormalizer {
    pub fn new(
        identity: StreamIdentity,
        format: DecodedAudioFormat,
        cancel: &ResolverCancellation,
    ) -> Result<Self, MediaError> {
        check(cancel, cancel)?;
        Ok(Self {
            gate: DecodedAudioGate::new(identity, format)?,
            format,
            cancel: cancel.clone(),
            previous: None,
            run_origin_us: 0,
            run_frames: 0,
            terminal: false,
        })
    }

    /// Consume one real decoder block on the caller's bounded CPU worker.
    /// Any error is terminal; cancellation cannot be bypassed by a fresh token.
    pub fn normalize(
        &mut self,
        block: DecodedAudioBlock,
        cancel: &ResolverCancellation,
    ) -> Result<MappedDecodedAudioBlock, MediaError> {
        let result = self.normalize_inner(block, cancel);
        if result.is_err() {
            self.terminal = true;
            self.previous = None;
            self.run_origin_us = 0;
            self.run_frames = 0;
        }
        result
    }

    fn normalize_inner(
        &mut self,
        block: DecodedAudioBlock,
        cancel: &ResolverCancellation,
    ) -> Result<MappedDecodedAudioBlock, MediaError> {
        check(&self.cancel, cancel)?;
        if self.terminal {
            return Err(MediaError::InvalidAudio);
        }
        let gap = self.gate.validate(&block)?;
        if block.end_us() > MAX_DECODED_MEDIA_DURATION_US {
            return Err(MediaError::TooLarge);
        }
        let run_id = match self.previous {
            None => {
                self.run_origin_us = block.timestamp_us();
                self.run_frames = 0;
                1
            }
            Some(previous) if gap => {
                let gap_us = block
                    .timestamp_us()
                    .checked_sub(previous.native_end_us)
                    .ok_or(MediaError::InvalidAudio)?;
                self.run_origin_us = previous
                    .sample_end_us
                    .checked_add(gap_us)
                    .ok_or(MediaError::InvalidAudio)?;
                self.run_frames = 0;
                previous
                    .run_id
                    .checked_add(1)
                    .ok_or(MediaError::InvalidAudio)?
            }
            Some(previous) => previous.run_id,
        };
        let sample_start_us = frame_time(
            self.run_origin_us,
            self.run_frames,
            self.format.sample_rate(),
        )?;
        let next_frames = self
            .run_frames
            .checked_add(block.frames() as u64)
            .ok_or(MediaError::InvalidAudio)?;
        let sample_end_us = frame_time(self.run_origin_us, next_frames, self.format.sample_rate())?;
        if sample_end_us > MAX_DECODED_MEDIA_DURATION_US {
            return Err(MediaError::TooLarge);
        }
        let span = DecodedClockSpan {
            identity: block.identity(),
            sequence: block.sequence(),
            run_id,
            format: self.format,
            sample_run_origin_us: self.run_origin_us,
            source_frame_start: self.run_frames,
            source_frame_end: next_frames,
            sample_start_us,
            sample_end_us,
            native_start_us: block.timestamp_us(),
            native_end_us: block.end_us(),
            discontinuity: gap,
        };
        let sample_clock = block.into_sample_clock(sample_start_us)?;
        check(&self.cancel, cancel)?;
        self.run_frames = next_frames;
        self.previous = Some(span);
        Ok(MappedDecodedAudioBlock { sample_clock, span })
    }
}

/// Source-bound endpoint mapping, not an alignment score or playback position.
/// Sequence bounds refer to decoded source blocks, never ASR chunk sequence;
/// the run is also the source normalizer's run, not the output builder's run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeMediaInterval {
    identity: StreamIdentity,
    run_id: u64,
    first_source_sequence: u64,
    last_source_sequence: u64,
    start_us: u64,
    end_us: u64,
}
impl NativeMediaInterval {
    pub const fn identity(self) -> StreamIdentity {
        self.identity
    }
    pub const fn run_id(self) -> u64 {
        self.run_id
    }
    pub const fn first_source_sequence(self) -> u64 {
        self.first_source_sequence
    }
    pub const fn last_source_sequence(self) -> u64 {
        self.last_source_sequence
    }
    pub const fn start_us(self) -> u64 {
        self.start_us
    }
    pub const fn end_us(self) -> u64 {
        self.end_us
    }
}

/// Immutable evidence owned by one ASR chunk, including its context. Only the
/// exact requested extent is usable; full source spans are retained so packet
/// phase and rational interpolation never depend on clipped endpoints.
/// This bounded owner contains metadata only and is deliberately not clonable,
/// printable or serializable. Pruning or dropping its source window is safe.
pub struct DecodedClockSnapshot {
    identity: StreamIdentity,
    source_run_id: u64,
    sample_start_us: u64,
    sample_end_us: u64,
    spans: Box<[DecodedClockSpan]>,
    cancel: ResolverCancellation,
    creation_cancel: ResolverCancellation,
}
impl DecodedClockSnapshot {
    pub const fn identity(&self) -> StreamIdentity {
        self.identity
    }
    pub const fn source_run_id(&self) -> u64 {
        self.source_run_id
    }
    pub const fn sample_start_us(&self) -> u64 {
        self.sample_start_us
    }
    pub const fn sample_end_us(&self) -> u64 {
        self.sample_end_us
    }
    /// The merger retains only distinct preparation scopes until whole-document
    /// handoff. It need not retain old packet maps or completed request audio.
    pub(crate) fn cancellation_scopes(&self) -> [&ResolverCancellation; 2] {
        [&self.cancel, &self.creation_cancel]
    }

    /// Map only observed, nonempty sample time within this snapshot. Both the
    /// original window's scope and the snapshot creation scope remain binding.
    pub fn map_interval(
        &self,
        identity: StreamIdentity,
        start_us: u64,
        end_us: u64,
    ) -> Result<NativeMediaInterval, MediaError> {
        self.check_active()?;
        if identity != self.identity {
            return Err(MediaError::Stale);
        }
        if start_us < self.sample_start_us || end_us > self.sample_end_us {
            return Err(MediaError::InvalidAudio);
        }
        map_spans(
            self.identity,
            identity,
            Some(self.source_run_id),
            start_us,
            end_us,
            (&self.cancel, &self.creation_cancel),
            self.spans.iter(),
        )
        .map(|(interval, _, _)| interval)
    }

    /// Validate successive chunk maps independently of the chunk's own run ID.
    /// Shared source packets must be identical, including their frame phase.
    /// Disjoint maps in one source run require the immediately adjacent packet;
    /// differing source runs may skip IDs when a DSP run emitted no frame.
    pub(crate) fn validate_successor(&self, previous: &Self) -> Result<(), MediaError> {
        self.check_active()?;
        previous.check_active()?;
        if self.identity != previous.identity {
            return Err(MediaError::Stale);
        }
        let first = self.spans.first().ok_or(MediaError::InvalidAudio)?;
        let last = self.spans.last().ok_or(MediaError::InvalidAudio)?;
        let previous_first = previous.spans.first().ok_or(MediaError::InvalidAudio)?;
        let previous_last = previous.spans.last().ok_or(MediaError::InvalidAudio)?;
        if first.format != previous_first.format
            || self.sample_end_us < previous.sample_end_us
            || self.source_run_id < previous.source_run_id
            || last.sequence < previous_last.sequence
        {
            return Err(MediaError::InvalidAudio);
        }
        if self.source_run_id == previous.source_run_id {
            // A short EOF body can reuse the previous right context, including
            // an equal audio end or the same entire context range. Chunk body
            // ownership/sequence advances in the merger, not in this evidence.
            if first.sample_run_origin_us != previous_first.sample_run_origin_us {
                return Err(MediaError::InvalidAudio);
            }
            let shared_start = first.sequence.max(previous_first.sequence);
            let shared_end = last.sequence.min(previous_last.sequence);
            if shared_start <= shared_end {
                // No source span is clipped: equality checks every timing and
                // phase field, even when only a tiny context portion overlaps.
                let offset = usize::try_from(
                    shared_start
                        .checked_sub(previous_first.sequence)
                        .ok_or(MediaError::InvalidAudio)?,
                )
                .map_err(|_| MediaError::InvalidAudio)?;
                let current_offset = usize::try_from(
                    shared_start
                        .checked_sub(first.sequence)
                        .ok_or(MediaError::InvalidAudio)?,
                )
                .map_err(|_| MediaError::InvalidAudio)?;
                for (index, span) in self
                    .spans
                    .iter()
                    .skip(current_offset)
                    .take_while(|span| span.sequence <= shared_end)
                    .enumerate()
                {
                    self.check_active()?;
                    let shared = previous
                        .spans
                        .get(offset.checked_add(index).ok_or(MediaError::InvalidAudio)?)
                        .ok_or(MediaError::InvalidAudio)?;
                    if span != shared {
                        return Err(MediaError::InvalidAudio);
                    }
                }
            } else if previous_last.sequence.checked_add(1) != Some(first.sequence)
                || first.source_frame_start != previous_last.source_frame_end
                || first.sample_start_us != previous_last.sample_end_us
                || first.native_start_us < previous_last.native_start_us
                || first.native_start_us.abs_diff(previous_last.native_end_us)
                    > super::decoded::DECODED_TIMESTAMP_ROUNDING_US
                || first.discontinuity
            {
                return Err(MediaError::InvalidAudio);
            }
        } else {
            let native =
                self.map_interval(self.identity, self.sample_start_us, self.sample_end_us)?;
            let previous_native = previous.map_interval(
                previous.identity,
                previous.sample_start_us,
                previous.sample_end_us,
            )?;
            if first.sequence <= previous_last.sequence
                || self.sample_start_us <= previous.sample_end_us
                || native.start_us <= previous_native.end_us
            {
                return Err(MediaError::InvalidAudio);
            }
        }
        self.check_active()?;
        previous.check_active()
    }

    fn check_active(&self) -> Result<(), MediaError> {
        check(&self.cancel, &self.creation_cancel)
    }
}

/// Rolling metadata owner. Explicit pruning keeps memory and lookup bounded.
/// It cannot extrapolate unseen/pruned audio or map a cue across native gaps.
/// No wall clock, seek heuristic, source file access or provider call exists.
pub struct DecodedClockWindow {
    identity: StreamIdentity,
    cancel: ResolverCancellation,
    spans: VecDeque<DecodedClockSpan>,
    previous: Option<DecodedClockSpan>,
    coverage_start_us: Option<u64>,
    terminal: bool,
}
impl DecodedClockWindow {
    pub fn new(
        identity: StreamIdentity,
        cancel: &ResolverCancellation,
    ) -> Result<Self, MediaError> {
        check(cancel, cancel)?;
        if identity.session.0 == 0 || identity.generation == 0 {
            return Err(MediaError::InvalidAudio);
        }
        let mut spans = VecDeque::new();
        spans
            .try_reserve_exact(MAX_CLOCK_WINDOW_SPANS)
            .map_err(|_| MediaError::Overloaded)?;
        Ok(Self {
            identity,
            cancel: cancel.clone(),
            spans,
            previous: None,
            coverage_start_us: None,
            terminal: false,
        })
    }

    pub fn append(
        &mut self,
        span: DecodedClockSpan,
        cancel: &ResolverCancellation,
    ) -> Result<(), MediaError> {
        let result = self.append_inner(span, cancel);
        if result.is_err() {
            self.fail();
        }
        result
    }
    fn append_inner(
        &mut self,
        span: DecodedClockSpan,
        cancel: &ResolverCancellation,
    ) -> Result<(), MediaError> {
        self.validate_span(span, cancel)?;
        let start = self
            .spans
            .front()
            .map_or(span.sample_start_us, |first| first.sample_start_us);
        if self.spans.len() == MAX_CLOCK_WINDOW_SPANS
            || span
                .sample_end_us
                .checked_sub(start)
                .is_none_or(|duration| duration > MAX_CLOCK_WINDOW_DURATION_US)
        {
            return Err(MediaError::Overloaded);
        }
        self.spans.push_back(span);
        self.previous = Some(span);
        check(&self.cancel, cancel)
    }

    /// Commit a producer's pending span after completed chunks own snapshots.
    /// Retain the original packet straddling the required context boundary, but
    /// make its older prefix unusable. Only the required logical extent counts
    /// toward 12 s; row count stays bounded and no source endpoint is clipped.
    pub(crate) fn append_retained(
        &mut self,
        span: DecodedClockSpan,
        required_start_us: u64,
        cancel: &ResolverCancellation,
    ) -> Result<(), MediaError> {
        let result = (|| {
            self.validate_span(span, cancel)?;
            if required_start_us > span.sample_end_us
                || self
                    .coverage_start_us
                    .is_some_and(|old| required_start_us < old)
            {
                return Err(MediaError::InvalidAudio);
            }
            while self
                .spans
                .front()
                .is_some_and(|old| old.sample_end_us <= required_start_us)
            {
                self.spans.pop_front();
            }
            let logical_start = self
                .spans
                .front()
                .map_or(span.sample_start_us, |first| first.sample_start_us)
                .max(required_start_us);
            if self.spans.len() == MAX_CLOCK_WINDOW_SPANS
                || span
                    .sample_end_us
                    .checked_sub(logical_start)
                    .is_none_or(|duration| duration > MAX_CLOCK_WINDOW_DURATION_US)
            {
                return Err(MediaError::Overloaded);
            }
            self.spans.push_back(span);
            self.previous = Some(span);
            self.coverage_start_us = Some(required_start_us);
            check(&self.cancel, cancel)
        })();
        if result.is_err() {
            self.fail();
        }
        result
    }

    /// Validate one actor-owned pending source block before DSP can flush an
    /// old run. This does not retain or commit the span: `append` or private
    /// `append_retained` applies capacity rules after completed chunks detach.
    pub(crate) fn validate_pending(
        &mut self,
        span: DecodedClockSpan,
        cancel: &ResolverCancellation,
    ) -> Result<(), MediaError> {
        let result = self.validate_span(span, cancel);
        if result.is_err() {
            self.fail();
        }
        result
    }

    /// Source validation shared by stored and temporarily staged metadata.
    /// Capacity is deliberately separate; phase, order and identity are not.
    fn validate_span(
        &self,
        span: DecodedClockSpan,
        cancel: &ResolverCancellation,
    ) -> Result<(), MediaError> {
        self.check_active(cancel)?;
        if span.identity != self.identity {
            return Err(MediaError::Stale);
        }
        // The same Session may still be misused with spans copied from another
        // normalizer. Endpoint continuity alone cannot prove the fractional
        // frame phase, especially at 44.1 kHz. Validate the full source clock.
        let frames = span
            .source_frame_end
            .checked_sub(span.source_frame_start)
            .filter(|frames| *frames > 0)
            .ok_or(MediaError::InvalidAudio)?;
        if frame_time(
            span.sample_run_origin_us,
            span.source_frame_start,
            span.format.sample_rate(),
        )? != span.sample_start_us
            || frame_time(
                span.sample_run_origin_us,
                span.source_frame_end,
                span.format.sample_rate(),
            )? != span.sample_end_us
            || frame_time(span.native_start_us, frames, span.format.sample_rate())?
                != span.native_end_us
        {
            return Err(MediaError::InvalidAudio);
        }
        if let Some(previous) = self.previous {
            let native_gap = span.native_start_us.saturating_sub(previous.native_end_us);
            if span.format != previous.format
                || previous.sequence.checked_add(1) != Some(span.sequence)
                || span.native_start_us < previous.native_start_us
                || previous.native_end_us.saturating_sub(span.native_start_us)
                    > super::decoded::DECODED_TIMESTAMP_ROUNDING_US
                || span.discontinuity
                    != (native_gap > super::decoded::DECODED_TIMESTAMP_ROUNDING_US)
                || (span.discontinuity
                    && (previous.run_id.checked_add(1) != Some(span.run_id)
                        || span.source_frame_start != 0
                        || span.sample_run_origin_us != span.sample_start_us
                        || span.sample_start_us.checked_sub(previous.sample_end_us)
                            != Some(native_gap)))
                || (!span.discontinuity
                    && (span.run_id != previous.run_id
                        || span.sample_run_origin_us != previous.sample_run_origin_us
                        || span.source_frame_start != previous.source_frame_end
                        || span.sample_start_us != previous.sample_end_us))
            {
                return Err(MediaError::InvalidAudio);
            }
        } else if span.sequence != 1
            || span.run_id != 1
            || span.discontinuity
            || span.source_frame_start != 0
            || span.sample_run_origin_us != span.sample_start_us
            || span.native_start_us != span.sample_start_us
        {
            return Err(MediaError::InvalidAudio);
        }
        check(&self.cancel, cancel)
    }

    /// Discard complete spans ending at/before this sample time. The owner must
    /// first detach evidence for any pending chunk, including its left context.
    /// Keeping the straddling span avoids inventing a rounded native endpoint.
    pub fn prune_before(
        &mut self,
        sample_us: u64,
        cancel: &ResolverCancellation,
    ) -> Result<(), MediaError> {
        let result = (|| {
            self.check_active(cancel)?;
            if self
                .previous
                .is_none_or(|last| sample_us > last.sample_end_us)
            {
                return Err(MediaError::InvalidAudio);
            }
            while self
                .spans
                .front()
                .is_some_and(|span| span.sample_end_us <= sample_us)
            {
                self.spans.pop_front();
            }
            check(&self.cancel, cancel)
        })();
        if result.is_err() {
            self.fail();
        }
        result
    }

    /// Map a nonempty half-open interval wholly within one observed run. At a
    /// packet boundary the start uses the right packet and the end the left;
    /// adjacent packet quantization is retained instead of accumulated drift.
    pub fn map_interval(
        &mut self,
        identity: StreamIdentity,
        run_id: u64,
        start_us: u64,
        end_us: u64,
        cancel: &ResolverCancellation,
    ) -> Result<NativeMediaInterval, MediaError> {
        let result = self.map_inner(identity, run_id, start_us, end_us, cancel);
        if result.is_err() {
            self.fail();
        }
        result
    }
    fn map_inner(
        &self,
        identity: StreamIdentity,
        run_id: u64,
        start_us: u64,
        end_us: u64,
        cancel: &ResolverCancellation,
    ) -> Result<NativeMediaInterval, MediaError> {
        self.check_mapping_scope(identity, start_us, cancel)?;
        map_spans(
            self.identity,
            identity,
            Some(run_id),
            start_us,
            end_us,
            (&self.cancel, cancel),
            self.spans.iter(),
        )
        .map(|(interval, _, _)| interval)
    }

    /// Detach only the source spans covering an exact sample/context interval.
    /// The source run is inferred from all covered evidence, never supplied by
    /// a chunk builder. Failure clears this window like every other operation.
    pub fn snapshot(
        &mut self,
        identity: StreamIdentity,
        sample_start_us: u64,
        sample_end_us: u64,
        cancel: &ResolverCancellation,
    ) -> Result<DecodedClockSnapshot, MediaError> {
        let result = self.snapshot_inner(identity, sample_start_us, sample_end_us, cancel);
        if result.is_err() {
            self.fail();
        }
        result
    }
    fn snapshot_inner(
        &self,
        identity: StreamIdentity,
        sample_start_us: u64,
        sample_end_us: u64,
        cancel: &ResolverCancellation,
    ) -> Result<DecodedClockSnapshot, MediaError> {
        self.snapshot_from_spans(
            identity,
            sample_start_us,
            sample_end_us,
            cancel,
            self.spans.iter(),
        )
    }

    /// Map a chunk while its final native block remains actor-owned. Chaining
    /// exactly one validated original span avoids a temporary metadata buffer;
    /// the search sees at most the row cap plus one pending span; a copied
    /// snapshot still cannot exceed the row cap.
    /// Exact context duration stays bounded even across a large block or gap.
    pub(crate) fn snapshot_with_pending(
        &mut self,
        identity: StreamIdentity,
        sample_start_us: u64,
        sample_end_us: u64,
        span: DecodedClockSpan,
        cancel: &ResolverCancellation,
    ) -> Result<DecodedClockSnapshot, MediaError> {
        let result = (|| {
            self.validate_span(span, cancel)?;
            self.snapshot_from_spans(
                identity,
                sample_start_us,
                sample_end_us,
                cancel,
                self.spans.iter().chain(std::iter::once(&span)),
            )
        })();
        if result.is_err() {
            self.fail();
        }
        result
    }

    fn snapshot_from_spans<'a, I>(
        &self,
        identity: StreamIdentity,
        sample_start_us: u64,
        sample_end_us: u64,
        cancel: &ResolverCancellation,
        source_spans: I,
    ) -> Result<DecodedClockSnapshot, MediaError>
    where
        I: Iterator<Item = &'a DecodedClockSpan> + Clone,
    {
        self.check_mapping_scope(identity, sample_start_us, cancel)?;
        let (interval, first, last) = map_spans(
            self.identity,
            identity,
            None,
            sample_start_us,
            sample_end_us,
            (&self.cancel, cancel),
            source_spans.clone(),
        )?;
        let count = last - first + 1;
        if count > MAX_CLOCK_WINDOW_SPANS
            || sample_end_us - sample_start_us > MAX_CLOCK_WINDOW_DURATION_US
        {
            return Err(MediaError::Overloaded);
        }
        let mut spans = Vec::new();
        spans
            .try_reserve_exact(count)
            .map_err(|_| MediaError::Overloaded)?;
        for span in source_spans.skip(first).take(count) {
            check(&self.cancel, cancel)?;
            spans.push(*span);
        }
        check(&self.cancel, cancel)?;
        Ok(DecodedClockSnapshot {
            identity: self.identity,
            source_run_id: interval.run_id,
            sample_start_us,
            sample_end_us,
            spans: spans.into_boxed_slice(),
            cancel: self.cancel.clone(),
            creation_cancel: cancel.clone(),
        })
    }

    fn check_active(&self, cancel: &ResolverCancellation) -> Result<(), MediaError> {
        check(&self.cancel, cancel)?;
        if self.terminal {
            Err(MediaError::InvalidAudio)
        } else {
            Ok(())
        }
    }
    fn check_mapping_scope(
        &self,
        identity: StreamIdentity,
        start_us: u64,
        cancel: &ResolverCancellation,
    ) -> Result<(), MediaError> {
        self.check_active(cancel)?;
        if identity != self.identity {
            return Err(MediaError::Stale);
        }
        if self
            .coverage_start_us
            .is_some_and(|coverage| start_us < coverage)
        {
            return Err(MediaError::InvalidAudio);
        }
        Ok(())
    }
    fn fail(&mut self) {
        self.spans.clear();
        self.previous = None;
        self.coverage_start_us = None;
        self.terminal = true;
    }
}

/// Both mutable windows and immutable snapshots use these same half-open
/// coverage and rational endpoint rules. `None` infers the observed source run.
fn map_spans<'a, I>(
    owner_identity: StreamIdentity,
    identity: StreamIdentity,
    run_id: Option<u64>,
    start_us: u64,
    end_us: u64,
    cancellation: (&ResolverCancellation, &ResolverCancellation),
    spans: I,
) -> Result<(NativeMediaInterval, usize, usize), MediaError>
where
    I: Iterator<Item = &'a DecodedClockSpan> + Clone,
{
    let (own, supplied) = cancellation;
    check(own, supplied)?;
    if identity != owner_identity {
        return Err(MediaError::Stale);
    }
    if run_id == Some(0) || start_us >= end_us {
        return Err(MediaError::InvalidAudio);
    }
    let (first, first_span) = spans
        .clone()
        .enumerate()
        .find(|(_, span)| span.sample_start_us <= start_us && start_us < span.sample_end_us)
        .ok_or(MediaError::InvalidAudio)?;
    let (last, last_span) = spans
        .clone()
        .enumerate()
        .find(|(_, span)| span.sample_start_us < end_us && end_us <= span.sample_end_us)
        .filter(|(last, _)| *last >= first)
        .ok_or(MediaError::InvalidAudio)?;
    let run_id = run_id.unwrap_or(first_span.run_id);
    for span in spans.skip(first).take(last - first + 1) {
        check(own, supplied)?;
        if span.run_id != run_id {
            return Err(MediaError::InvalidAudio);
        }
    }
    let start = first_span.at(start_us)?;
    let end = last_span.at(end_us)?;
    if start >= end {
        return Err(MediaError::InvalidAudio);
    }
    check(own, supplied)?;
    Ok((
        NativeMediaInterval {
            identity: owner_identity,
            run_id,
            first_source_sequence: first_span.sequence,
            last_source_sequence: last_span.sequence,
            start_us: start,
            end_us: end,
        },
        first,
        last,
    ))
}

fn frame_time(origin: u64, frames: u64, rate: u32) -> Result<u64, MediaError> {
    frames
        .checked_mul(MICROSECONDS_PER_SECOND)
        .map(|value| value / u64::from(rate))
        .and_then(|offset| origin.checked_add(offset))
        .ok_or(MediaError::InvalidAudio)
}
fn check(own: &ResolverCancellation, supplied: &ResolverCancellation) -> Result<(), MediaError> {
    if own.is_cancelled() || supplied.is_cancelled() {
        Err(MediaError::Cancelled)
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "decoded_clock_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "decoded_clock_snapshot_tests.rs"]
mod snapshot_tests;
