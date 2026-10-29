//! Pull-driven, session-only Prepared ASR segmentation. Body intervals partition
//! each continuous media run; the bounded audio overlap is recognition context.
use super::{
    MediaError,
    decode_worker::MAX_DECODED_MEDIA_DURATION_US,
    decoded::{
        DECODED_TIMESTAMP_ROUNDING_US, DecodedAudioBlock, DecodedAudioFormat, DecodedAudioGate,
    },
    prepared_audio::PreparedSampleRate,
};
use crate::{
    ai::{
        prepared_speech::{
            BODY_DURATION_MS, MAX_CHUNK_DURATION_MS, OVERLAP_DURATION_MS, PreparedChunkStamp,
            PreparedSpeechChunk, PreparedSpeechError,
        },
        speech::StreamIdentity,
    },
    resolver::contracts::ResolverCancellation,
};
use std::collections::VecDeque;

/// EOF or a gap may leave one complete body and one short body to deliver.
pub const MAX_PREPARED_READY_CHUNKS: usize = 2;
/// Cooperative checks bound a synchronous cancellation response during copying.
const CANCELLATION_CHECK_FRAMES: usize = 256;
const MICROSECONDS_PER_SECOND: u64 = 1_000_000;

enum State {
    Active,
    Finished,
    Failed(PreparedSpeechError),
}

struct Run {
    id: u64,
    origin_us: u64,
    frames: u64,
    buffer_start: u64,
    body_start: u64,
    buffer: Vec<i16>,
    first_chunk: bool,
    discontinuity: bool,
}

/// Owns only one bounded working buffer and at most two completed chunks.
/// Callers must `take` all ready chunks before the next `push`. No worker,
/// network request, disk write, blocking queue, or Live wall-clock stamp exists.
/// Input ownership is deliberately separate from the borrowed preprocessing
/// callback: retaining that callback's PCM remains forbidden by its contract.
pub struct PreparedSegmentBuilder {
    identity: StreamIdentity,
    rate: PreparedSampleRate,
    gate: DecodedAudioGate,
    body_frames: usize,
    overlap_frames: usize,
    max_buffer_frames: usize,
    run: Option<Run>,
    ready: VecDeque<PreparedSpeechChunk>,
    next_run_id: u64,
    next_chunk_sequence: u64,
    cancel: Option<ResolverCancellation>,
    state: State,
}

impl PreparedSegmentBuilder {
    pub fn new(
        identity: StreamIdentity,
        rate: PreparedSampleRate,
    ) -> Result<Self, PreparedSpeechError> {
        let format = DecodedAudioFormat::new(rate.hz(), 1).map_err(map_media)?;
        let gate = DecodedAudioGate::new(identity, format).map_err(map_media)?;
        let mut ready = VecDeque::new();
        ready
            .try_reserve_exact(MAX_PREPARED_READY_CHUNKS)
            .map_err(|_| PreparedSpeechError::Overloaded)?;
        Ok(Self {
            identity,
            rate,
            gate,
            body_frames: duration_frames(rate, u64::from(BODY_DURATION_MS))?,
            overlap_frames: duration_frames(rate, u64::from(OVERLAP_DURATION_MS))?,
            max_buffer_frames: duration_frames(rate, u64::from(MAX_CHUNK_DURATION_MS))?,
            run: None,
            ready,
            next_run_id: 1,
            next_chunk_sequence: 1,
            cancel: None,
            state: State::Active,
        })
    }

    /// Accept one owned mono block at the configured rate. `discontinuity` must
    /// equal the gate's actual forward-gap result and must be false initially;
    /// a fabricated gap may not reset the run clock to conceal cumulative drift.
    /// An untaken ready chunk causes terminal `Overloaded`, never silent loss.
    pub fn push(
        &mut self,
        block: DecodedAudioBlock,
        discontinuity: bool,
        cancel: &ResolverCancellation,
    ) -> Result<(), PreparedSpeechError> {
        let result = self.push_inner(&block, discontinuity, cancel);
        if let Err(error) = result {
            self.fail(error);
        }
        result
    }

    fn push_inner(
        &mut self,
        block: &DecodedAudioBlock,
        discontinuity: bool,
        cancel: &ResolverCancellation,
    ) -> Result<(), PreparedSpeechError> {
        self.check_active(cancel)?;
        if !self.ready.is_empty() {
            return Err(PreparedSpeechError::Overloaded);
        }
        let gap = self.gate.validate(block).map_err(map_media)?;
        // A supplied flag must describe a real forward gap. An invented reset
        // would let a caller conceal accumulated source-frame clock drift.
        if gap != discontinuity {
            return Err(PreparedSpeechError::BoundaryMismatch);
        }
        if block.end_us() > MAX_DECODED_MEDIA_DURATION_US {
            return Err(PreparedSpeechError::InvalidData);
        }
        if gap {
            self.finish_run(cancel)?;
        }
        if self.run.is_none() {
            self.start_run(block.timestamp_us(), gap)?;
        }
        let run = self.run.as_ref().ok_or(PreparedSpeechError::InvalidData)?;
        let expected = frame_time(run.origin_us, run.frames, self.rate)?;
        if expected.abs_diff(block.timestamp_us()) > DECODED_TIMESTAMP_ROUNDING_US {
            return Err(PreparedSpeechError::BoundaryMismatch);
        }
        let final_frames = run
            .frames
            .checked_add(
                u64::try_from(block.frames()).map_err(|_| PreparedSpeechError::InvalidData)?,
            )
            .ok_or(PreparedSpeechError::InvalidData)?;
        if frame_time(run.origin_us, final_frames, self.rate)? > MAX_DECODED_MEDIA_DURATION_US {
            return Err(PreparedSpeechError::InvalidData);
        }
        for (index, sample) in block.samples().iter().copied().enumerate() {
            if index.is_multiple_of(CANCELLATION_CHECK_FRAMES) {
                self.check_cancel(cancel)?;
            }
            let run = self.run.as_mut().ok_or(PreparedSpeechError::InvalidData)?;
            if run.buffer.len() >= self.max_buffer_frames {
                return Err(PreparedSpeechError::Overloaded);
            }
            run.buffer.push(sample);
            run.frames = run
                .frames
                .checked_add(1)
                .ok_or(PreparedSpeechError::InvalidData)?;
            let threshold = run
                .body_start
                .checked_add(self.body_frames as u64)
                .and_then(|end| end.checked_add(self.overlap_frames as u64))
                .ok_or(PreparedSpeechError::InvalidData)?;
            if run.frames == threshold {
                let body_end = run
                    .body_start
                    .checked_add(self.body_frames as u64)
                    .ok_or(PreparedSpeechError::InvalidData)?;
                self.emit(body_end, cancel)?;
            }
        }
        self.check_cancel(cancel)
    }

    /// Move the next owned chunk out. Cancellation clears all retained PCM,
    /// including chunks completed before the token was cancelled.
    pub fn take(&mut self) -> Option<PreparedSpeechChunk> {
        if self
            .cancel
            .as_ref()
            .is_some_and(|cancel| cancel.is_cancelled())
        {
            self.fail(PreparedSpeechError::Cancelled);
            return None;
        }
        if matches!(self.state, State::Failed(_)) {
            None
        } else {
            self.ready.pop_front()
        }
    }

    /// Flush real EOF without synthesized audio or a body containing only
    /// overlap. A successful finish still allows the residual chunks to be taken.
    pub fn finish(&mut self, cancel: &ResolverCancellation) -> Result<(), PreparedSpeechError> {
        let result = self.finish_inner(cancel);
        match result {
            Ok(()) => self.state = State::Finished,
            Err(error) => self.fail(error),
        }
        result
    }

    /// Close a verified native source run even if the next run emits no target
    /// frame. Preserve the mono gate and the next nonempty run's sequence.
    pub(crate) fn end_source_run(
        &mut self,
        cancel: &ResolverCancellation,
    ) -> Result<(), PreparedSpeechError> {
        let result = (|| {
            self.check_active(cancel)?;
            if !self.ready.is_empty() {
                return Err(PreparedSpeechError::Overloaded);
            }
            self.finish_run(cancel)
        })();
        if let Err(error) = result {
            self.fail(error);
        }
        result
    }

    /// Earliest PCM retained for a future body, including its left context.
    pub(crate) fn retained_audio_start(&self) -> Result<Option<u64>, PreparedSpeechError> {
        self.run
            .as_ref()
            .map(|run| frame_time(run.origin_us, run.buffer_start, self.rate))
            .transpose()
    }

    fn finish_inner(&mut self, cancel: &ResolverCancellation) -> Result<(), PreparedSpeechError> {
        self.check_active(cancel)?;
        if self.run.is_none() && self.next_chunk_sequence == 1 {
            return Err(PreparedSpeechError::InvalidData);
        }
        self.finish_run(cancel)?;
        self.check_cancel(cancel)
    }

    fn start_run(
        &mut self,
        origin_us: u64,
        discontinuity: bool,
    ) -> Result<(), PreparedSpeechError> {
        let id = self.next_run_id;
        self.next_run_id = id.checked_add(1).ok_or(PreparedSpeechError::InvalidData)?;
        let mut buffer = Vec::new();
        buffer
            .try_reserve_exact(self.max_buffer_frames)
            .map_err(|_| PreparedSpeechError::Overloaded)?;
        self.run = Some(Run {
            id,
            origin_us,
            frames: 0,
            buffer_start: 0,
            body_start: 0,
            buffer,
            first_chunk: true,
            discontinuity,
        });
        Ok(())
    }

    fn finish_run(&mut self, cancel: &ResolverCancellation) -> Result<(), PreparedSpeechError> {
        loop {
            self.check_cancel(cancel)?;
            let Some(run) = self.run.as_ref() else {
                return Ok(());
            };
            if run.body_start == run.frames {
                break;
            }
            let end = run
                .body_start
                .checked_add(self.body_frames as u64)
                .ok_or(PreparedSpeechError::InvalidData)?
                .min(run.frames);
            self.emit(end, cancel)?;
        }
        self.run = None;
        Ok(())
    }

    fn emit(
        &mut self,
        body_end: u64,
        cancel: &ResolverCancellation,
    ) -> Result<(), PreparedSpeechError> {
        self.check_cancel(cancel)?;
        if self.ready.len() == MAX_PREPARED_READY_CHUNKS {
            return Err(PreparedSpeechError::Overloaded);
        }
        let sequence = self.next_chunk_sequence;
        let next_sequence = sequence
            .checked_add(1)
            .ok_or(PreparedSpeechError::InvalidData)?;
        let run = self.run.as_mut().ok_or(PreparedSpeechError::InvalidData)?;
        if body_end <= run.body_start || body_end > run.frames {
            return Err(PreparedSpeechError::BoundaryMismatch);
        }
        let audio_end = body_end
            .checked_add(self.overlap_frames as u64)
            .ok_or(PreparedSpeechError::InvalidData)?
            .min(run.frames);
        let audio_length = usize::try_from(
            audio_end
                .checked_sub(run.buffer_start)
                .ok_or(PreparedSpeechError::InvalidData)?,
        )
        .map_err(|_| PreparedSpeechError::InvalidData)?;
        if audio_length > self.max_buffer_frames || audio_length > run.buffer.len() {
            return Err(PreparedSpeechError::InvalidData);
        }
        let stamp = PreparedChunkStamp {
            identity: self.identity,
            sequence,
            run_id: run.id,
            rate: self.rate,
            audio_start_us: frame_time(run.origin_us, run.buffer_start, self.rate)?,
            body_start_us: frame_time(run.origin_us, run.body_start, self.rate)?,
            body_end_us: frame_time(run.origin_us, body_end, self.rate)?,
            discontinuity: run.first_chunk && run.discontinuity,
        };
        let retain_start = body_end.saturating_sub(self.overlap_frames as u64);
        let retain_offset = usize::try_from(
            retain_start
                .checked_sub(run.buffer_start)
                .ok_or(PreparedSpeechError::InvalidData)?,
        )
        .map_err(|_| PreparedSpeechError::InvalidData)?;
        let retained_samples = run
            .buffer
            .get(retain_offset..)
            .filter(|samples| samples.len() <= self.max_buffer_frames)
            .ok_or(PreparedSpeechError::InvalidData)?;
        // Only required overlap and any real future body are copied. The main
        // PCM buffer moves into the completed chunk and is never cloned in full.
        let mut retained = Vec::new();
        retained
            .try_reserve_exact(self.max_buffer_frames)
            .map_err(|_| PreparedSpeechError::Overloaded)?;
        retained.extend_from_slice(retained_samples);
        let mut samples = std::mem::replace(&mut run.buffer, retained);
        samples.truncate(audio_length);
        let chunk = PreparedSpeechChunk::new(stamp, samples)?;
        self.check_cancel(cancel)?;
        let run = self.run.as_mut().ok_or(PreparedSpeechError::InvalidData)?;
        run.buffer_start = retain_start;
        run.body_start = body_end;
        run.first_chunk = false;
        self.next_chunk_sequence = next_sequence;
        self.ready.push_back(chunk);
        Ok(())
    }

    fn check_active(&mut self, cancel: &ResolverCancellation) -> Result<(), PreparedSpeechError> {
        if self.cancel.is_none() {
            self.cancel = Some(cancel.clone());
        }
        self.check_cancel(cancel)?;
        match self.state {
            State::Active => Ok(()),
            State::Finished => Err(PreparedSpeechError::InvalidData),
            State::Failed(error) => Err(error),
        }
    }

    fn check_cancel(&self, cancel: &ResolverCancellation) -> Result<(), PreparedSpeechError> {
        if cancel.is_cancelled() || self.cancel.as_ref().is_some_and(|own| own.is_cancelled()) {
            Err(PreparedSpeechError::Cancelled)
        } else {
            Ok(())
        }
    }

    fn fail(&mut self, error: PreparedSpeechError) {
        self.run = None;
        self.ready.clear();
        self.state = State::Failed(error);
    }
}

fn duration_frames(
    rate: PreparedSampleRate,
    milliseconds: u64,
) -> Result<usize, PreparedSpeechError> {
    let frames = u64::from(rate.hz())
        .checked_mul(milliseconds)
        .filter(|value| value.is_multiple_of(1_000))
        .map(|value| value / 1_000)
        .ok_or(PreparedSpeechError::InvalidData)?;
    usize::try_from(frames).map_err(|_| PreparedSpeechError::InvalidData)
}

fn frame_time(
    origin_us: u64,
    frames: u64,
    rate: PreparedSampleRate,
) -> Result<u64, PreparedSpeechError> {
    frames
        .checked_mul(MICROSECONDS_PER_SECOND)
        .map(|value| value / u64::from(rate.hz()))
        .and_then(|offset| origin_us.checked_add(offset))
        .ok_or(PreparedSpeechError::InvalidData)
}

fn map_media(error: MediaError) -> PreparedSpeechError {
    match error {
        MediaError::Cancelled => PreparedSpeechError::Cancelled,
        MediaError::Stale => PreparedSpeechError::Stale,
        MediaError::Overloaded => PreparedSpeechError::Overloaded,
        _ => PreparedSpeechError::InvalidData,
    }
}

#[cfg(test)]
#[path = "prepared_segments_tests.rs"]
mod tests;
