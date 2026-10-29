//! Bounded, session-only Prepared PCM preprocessing. Native media timestamps
//! remain media time; this module never invents a Live capture or wall-clock stamp.
use super::{
    MediaError,
    decode_worker::MAX_DECODED_MEDIA_DURATION_US,
    decoded::{
        DECODED_TIMESTAMP_ROUNDING_US, DecodedAudioBlock, DecodedAudioFormat, DecodedAudioGate,
    },
    prepared_resample::PreparedResampler,
};
use crate::{ai::speech::StreamIdentity, resolver::contracts::ResolverCancellation};

const MICROSECONDS_PER_SECOND: u64 = 1_000_000;
/// A synchronous sink borrows at most 20 ms of mono PCM in each invocation.
pub const PREPARED_PCM_BLOCK_DURATION_MS: u32 = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreparedSampleRate {
    Hz16000,
    Hz24000,
}
impl PreparedSampleRate {
    pub const fn hz(self) -> u32 {
        match self {
            Self::Hz16000 => 16_000,
            Self::Hz24000 => 24_000,
        }
    }
    const fn block_frames(self) -> usize {
        (self.hz() * PREPARED_PCM_BLOCK_DURATION_MS / 1_000) as usize
    }
}

/// One borrowed, mono PCM16 output block. It has no Clone, Debug or persistence.
/// A discontinuity starts a new independently filtered run, without silence.
pub struct PreparedPcmBlock {
    decoded: DecodedAudioBlock,
    discontinuity: bool,
}
impl PreparedPcmBlock {
    pub const fn identity(&self) -> StreamIdentity {
        self.decoded.identity()
    }
    pub const fn sequence(&self) -> u64 {
        self.decoded.sequence()
    }
    pub const fn timestamp_us(&self) -> u64 {
        self.decoded.timestamp_us()
    }
    pub const fn duration_us(&self) -> u64 {
        self.decoded.duration_us()
    }
    pub const fn end_us(&self) -> u64 {
        self.decoded.end_us()
    }
    pub const fn sample_rate(&self) -> u32 {
        self.decoded.format().sample_rate()
    }
    pub fn samples(&self) -> &[i16] {
        self.decoded.samples()
    }
    pub fn frames(&self) -> usize {
        self.decoded.frames()
    }
    pub const fn discontinuity(&self) -> bool {
        self.discontinuity
    }
}

/// Returned only after complete successful EOF processing. It contains neither
/// PCM nor transcript, path, playback state or serialization support.
pub struct PreparedAudioSummary {
    identity: StreamIdentity,
    sample_rate: u32,
    blocks: u64,
    frames: u64,
    first_timestamp_us: u64,
    last_end_us: u64,
    gaps: u64,
}
impl PreparedAudioSummary {
    pub const fn identity(&self) -> StreamIdentity {
        self.identity
    }
    pub const fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    pub const fn blocks(&self) -> u64 {
        self.blocks
    }
    pub const fn frames(&self) -> u64 {
        self.frames
    }
    pub const fn first_timestamp_us(&self) -> u64 {
        self.first_timestamp_us
    }
    pub const fn last_end_us(&self) -> u64 {
        self.last_end_us
    }
    pub const fn gaps(&self) -> u64 {
        self.gaps
    }
}

#[derive(Default)]
struct OutputState {
    blocks: u64,
    frames: u64,
    first_timestamp_us: Option<u64>,
    last_end_us: u64,
    gaps: u64,
}
struct Run {
    origin_us: u64,
    native_frames: u64,
    emitted_frames: u64,
    discontinuity: bool,
    resampler: Option<PreparedResampler>,
    pending: Vec<i16>,
}

/// Runs entirely on the caller's bounded CPU worker. A public borrowed sink must
/// perform short bounded work and must not retain PCM or perform network, disk,
/// UI or queue waits. Callback output is preprocessing data, never a Ready state.
///
/// Each continuous run has one source-frame clock. Input block rounding may
/// differ by at most 1 µs; cumulative block-by-block timestamp drift is rejected.
/// EOF produces floor(source_frames * output_rate / input_rate) frames, so its
/// tail can be shorter than the native run by less than one output frame.
pub struct PreparedAudioProcessor {
    identity: StreamIdentity,
    format: DecodedAudioFormat,
    rate: PreparedSampleRate,
    gate: DecodedAudioGate,
    cancel: ResolverCancellation,
    run: Option<Run>,
    output: OutputState,
    terminal: bool,
}
impl PreparedAudioProcessor {
    pub fn new(
        identity: StreamIdentity,
        format: DecodedAudioFormat,
        rate: PreparedSampleRate,
        cancel: &ResolverCancellation,
    ) -> Result<Self, MediaError> {
        check(cancel, cancel)?;
        Ok(Self {
            identity,
            format,
            rate,
            gate: DecodedAudioGate::new(identity, format)?,
            cancel: cancel.clone(),
            run: None,
            output: OutputState::default(),
            terminal: false,
        })
    }

    pub fn push(
        &mut self,
        block: &DecodedAudioBlock,
        cancel: &ResolverCancellation,
        emit: &mut impl FnMut(&PreparedPcmBlock, &ResolverCancellation) -> Result<(), MediaError>,
    ) -> Result<(), MediaError> {
        self.push_emission(block, cancel, &mut |output, scope| emit(&output, scope))
    }

    /// Private ownership bridge for the demand-driven native actor. The PCM
    /// moves directly into a bounded segment builder; this synchronous sink
    /// still performs no network, disk, UI, or queue-capacity wait.
    pub(crate) fn push_owned(
        &mut self,
        block: &DecodedAudioBlock,
        cancel: &ResolverCancellation,
        emit: &mut impl FnMut(DecodedAudioBlock, bool, &ResolverCancellation) -> Result<(), MediaError>,
    ) -> Result<(), MediaError> {
        self.push_emission(block, cancel, &mut |output, scope| {
            let PreparedPcmBlock {
                decoded,
                discontinuity,
            } = output;
            emit(decoded, discontinuity, scope)
        })
    }

    fn push_emission(
        &mut self,
        block: &DecodedAudioBlock,
        cancel: &ResolverCancellation,
        emit: &mut impl FnMut(PreparedPcmBlock, &ResolverCancellation) -> Result<(), MediaError>,
    ) -> Result<(), MediaError> {
        let result = self.push_inner(block, cancel, emit);
        if result.is_err() {
            self.terminate();
        }
        result
    }

    fn push_inner(
        &mut self,
        block: &DecodedAudioBlock,
        cancel: &ResolverCancellation,
        emit: &mut impl FnMut(PreparedPcmBlock, &ResolverCancellation) -> Result<(), MediaError>,
    ) -> Result<(), MediaError> {
        self.check_active(cancel)?;
        let discontinuity = self.gate.validate(block)?;
        if block.end_us() > MAX_DECODED_MEDIA_DURATION_US {
            return Err(MediaError::TooLarge);
        }
        if discontinuity {
            if self.run.is_some() {
                self.finish_run(cancel, emit)?;
            }
            self.output.gaps = self
                .output
                .gaps
                .checked_add(1)
                .ok_or(MediaError::InvalidAudio)?;
        }
        if self.run.is_none() {
            self.run = Some(Run {
                origin_us: block.timestamp_us(),
                native_frames: 0,
                emitted_frames: 0,
                discontinuity,
                resampler: Some(PreparedResampler::new(
                    self.format.sample_rate(),
                    self.rate.hz(),
                    &self.cancel,
                )?),
                pending: Vec::with_capacity(self.rate.block_frames()),
            });
        }
        let run = self.run.as_mut().ok_or(MediaError::InvalidAudio)?;
        let expected = frame_time(run.origin_us, run.native_frames, self.format.sample_rate())?;
        if expected.abs_diff(block.timestamp_us()) > DECODED_TIMESTAMP_ROUNDING_US {
            return Err(MediaError::InvalidAudio);
        }
        run.native_frames = run
            .native_frames
            .checked_add(u64::try_from(block.frames()).map_err(|_| MediaError::InvalidAudio)?)
            .ok_or(MediaError::InvalidAudio)?;
        if frame_time(run.origin_us, run.native_frames, self.format.sample_rate())?
            > MAX_DECODED_MEDIA_DURATION_US
        {
            return Err(MediaError::TooLarge);
        }
        let mut resampler = run.resampler.take().ok_or(MediaError::InvalidAudio)?;
        let mut sink = OutputSink {
            identity: self.identity,
            format: self.format,
            rate: self.rate,
            output: &mut self.output,
            own_cancel: &self.cancel,
            cancel,
            emit,
        };
        let mut output_sample = |sample| sink.append(run, sample);
        for frame in block
            .samples()
            .chunks_exact(usize::from(self.format.channels()))
        {
            check(&self.cancel, cancel)?;
            // At most eight i16 channels; i64 accumulation cannot overflow.
            // Keep the fractional average until final PCM16 quantization.
            let sum: i64 = frame.iter().map(|sample| i64::from(*sample)).sum();
            let mono = sum as f64 / f64::from(self.format.channels());
            resampler.push(mono, cancel, &mut output_sample)?;
        }
        check(&self.cancel, cancel)?;
        run.resampler = Some(resampler);
        Ok(())
    }

    pub fn finish(
        &mut self,
        cancel: &ResolverCancellation,
        emit: &mut impl FnMut(&PreparedPcmBlock, &ResolverCancellation) -> Result<(), MediaError>,
    ) -> Result<PreparedAudioSummary, MediaError> {
        self.finish_emission(cancel, &mut |output, scope| emit(&output, scope))
    }

    /// Flush the actual EOF tail by moving its PCM to the private bounded owner.
    /// Public `finish` continues to expose only a temporary borrowed block.
    pub(crate) fn finish_owned(
        &mut self,
        cancel: &ResolverCancellation,
        emit: &mut impl FnMut(DecodedAudioBlock, bool, &ResolverCancellation) -> Result<(), MediaError>,
    ) -> Result<PreparedAudioSummary, MediaError> {
        self.finish_emission(cancel, &mut |output, scope| {
            let PreparedPcmBlock {
                decoded,
                discontinuity,
            } = output;
            emit(decoded, discontinuity, scope)
        })
    }

    /// The private native actor has already validated an actual source gap.
    /// Flush its preceding run before admitting new clock evidence. Keep the
    /// input gate intact: the next push must still prove the same real gap.
    pub(crate) fn flush_run_owned(
        &mut self,
        cancel: &ResolverCancellation,
        emit: &mut impl FnMut(DecodedAudioBlock, bool, &ResolverCancellation) -> Result<(), MediaError>,
    ) -> Result<(), MediaError> {
        let result = (|| {
            self.check_active(cancel)?;
            if self.run.is_some() {
                self.finish_run(cancel, &mut |output, scope| {
                    emit(output.decoded, output.discontinuity, scope)
                })?;
            }
            check(&self.cancel, cancel)
        })();
        if result.is_err() {
            self.terminate();
        }
        result
    }

    /// Includes pending PCM and FIR output not yet delivered to segmentation.
    pub(crate) fn retained_audio_start(&self) -> Result<Option<u64>, MediaError> {
        self.run
            .as_ref()
            .map(|run| frame_time(run.origin_us, run.emitted_frames, self.rate.hz()))
            .transpose()
    }

    fn finish_emission(
        &mut self,
        cancel: &ResolverCancellation,
        emit: &mut impl FnMut(PreparedPcmBlock, &ResolverCancellation) -> Result<(), MediaError>,
    ) -> Result<PreparedAudioSummary, MediaError> {
        let result = self.finish_inner(cancel, emit);
        self.terminate();
        result
    }

    fn finish_inner(
        &mut self,
        cancel: &ResolverCancellation,
        emit: &mut impl FnMut(PreparedPcmBlock, &ResolverCancellation) -> Result<(), MediaError>,
    ) -> Result<PreparedAudioSummary, MediaError> {
        self.check_active(cancel)?;
        if self.run.is_none() {
            return Err(MediaError::InvalidAudio);
        }
        self.finish_run(cancel, emit)?;
        check(&self.cancel, cancel)?;
        if self.output.frames == 0 {
            return Err(MediaError::InsufficientAudio);
        }
        Ok(PreparedAudioSummary {
            identity: self.identity,
            sample_rate: self.rate.hz(),
            blocks: self.output.blocks,
            frames: self.output.frames,
            first_timestamp_us: self
                .output
                .first_timestamp_us
                .ok_or(MediaError::InvalidAudio)?,
            last_end_us: self.output.last_end_us,
            gaps: self.output.gaps,
        })
    }

    fn finish_run(
        &mut self,
        cancel: &ResolverCancellation,
        emit: &mut impl FnMut(PreparedPcmBlock, &ResolverCancellation) -> Result<(), MediaError>,
    ) -> Result<(), MediaError> {
        let Some(mut run) = self.run.take() else {
            return Err(MediaError::InvalidAudio);
        };
        check(&self.cancel, cancel)?;
        let mut resampler = run.resampler.take().ok_or(MediaError::InvalidAudio)?;
        let mut sink = OutputSink {
            identity: self.identity,
            format: self.format,
            rate: self.rate,
            output: &mut self.output,
            own_cancel: &self.cancel,
            cancel,
            emit,
        };
        resampler.finish(cancel, &mut |sample| sink.append(&mut run, sample))?;
        if resampler.source_frames() != run.native_frames
            || resampler.output_frames()
                != run
                    .native_frames
                    .checked_mul(u64::from(self.rate.hz()))
                    .ok_or(MediaError::InvalidAudio)?
                    / u64::from(self.format.sample_rate())
        {
            return Err(MediaError::InvalidAudio);
        }
        sink.flush(&mut run)?;
        if run.emitted_frames != resampler.output_frames() {
            return Err(MediaError::InvalidAudio);
        }
        check(&self.cancel, cancel)
    }

    fn check_active(&self, cancel: &ResolverCancellation) -> Result<(), MediaError> {
        check(&self.cancel, cancel)?;
        if self.terminal {
            Err(MediaError::InvalidAudio)
        } else {
            Ok(())
        }
    }

    fn terminate(&mut self) {
        self.terminal = true;
        self.run = None;
    }
}

struct OutputSink<'a, E> {
    identity: StreamIdentity,
    format: DecodedAudioFormat,
    rate: PreparedSampleRate,
    output: &'a mut OutputState,
    own_cancel: &'a ResolverCancellation,
    cancel: &'a ResolverCancellation,
    emit: &'a mut E,
}
impl<E> OutputSink<'_, E>
where
    E: FnMut(PreparedPcmBlock, &ResolverCancellation) -> Result<(), MediaError>,
{
    fn append(&mut self, run: &mut Run, sample: i16) -> Result<(), MediaError> {
        check(self.own_cancel, self.cancel)?;
        if run.pending.len() >= self.rate.block_frames() {
            return Err(MediaError::InvalidAudio);
        }
        run.pending.push(sample);
        if run.pending.len() == self.rate.block_frames() {
            self.flush(run)?;
        }
        Ok(())
    }
    fn flush(&mut self, run: &mut Run) -> Result<(), MediaError> {
        check(self.own_cancel, self.cancel)?;
        if run.pending.is_empty() {
            return Ok(());
        }
        let frames = u64::try_from(run.pending.len()).map_err(|_| MediaError::InvalidAudio)?;
        let next_frames = run
            .emitted_frames
            .checked_add(frames)
            .ok_or(MediaError::InvalidAudio)?;
        let next_total = self
            .output
            .frames
            .checked_add(frames)
            .ok_or(MediaError::InvalidAudio)?;
        let sequence = self
            .output
            .blocks
            .checked_add(1)
            .filter(|value| *value < u64::MAX)
            .ok_or(MediaError::InvalidAudio)?;
        let timestamp_us = frame_time(run.origin_us, run.emitted_frames, self.rate.hz())?;
        let decoded = DecodedAudioBlock::new(
            self.identity,
            sequence,
            timestamp_us,
            DecodedAudioFormat::new(self.rate.hz(), 1)?,
            std::mem::replace(
                &mut run.pending,
                Vec::with_capacity(self.rate.block_frames()),
            ),
        )?;
        let native_end = frame_time(run.origin_us, run.native_frames, self.format.sample_rate())?;
        if decoded.end_us() > native_end || decoded.end_us() > MAX_DECODED_MEDIA_DURATION_US {
            return Err(MediaError::InvalidAudio);
        }
        let end_us = decoded.end_us();
        let block = PreparedPcmBlock {
            decoded,
            discontinuity: run.discontinuity,
        };
        (self.emit)(block, self.cancel)?;
        check(self.own_cancel, self.cancel)?;
        self.output.blocks = sequence;
        self.output.frames = next_total;
        self.output.first_timestamp_us.get_or_insert(timestamp_us);
        self.output.last_end_us = end_us;
        run.emitted_frames = next_frames;
        run.discontinuity = false;
        Ok(())
    }
}

fn frame_time(origin_us: u64, frames: u64, rate: u32) -> Result<u64, MediaError> {
    let offset = frames
        .checked_mul(MICROSECONDS_PER_SECOND)
        .ok_or(MediaError::InvalidAudio)?
        / u64::from(rate);
    origin_us
        .checked_add(offset)
        .ok_or(MediaError::InvalidAudio)
}

fn check(
    own_cancel: &ResolverCancellation,
    cancel: &ResolverCancellation,
) -> Result<(), MediaError> {
    if own_cancel.is_cancelled() || cancel.is_cancelled() {
        Err(MediaError::Cancelled)
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "prepared_audio_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "prepared_audio_owned_tests.rs"]
mod owned_tests;
