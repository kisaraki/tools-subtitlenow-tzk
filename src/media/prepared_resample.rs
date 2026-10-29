//! Bounded Prepared-only mono PCM resampling. Media timestamps belong to the
//! caller; this DSP has no Live capture stamp, wall clock, I/O, or worker.
use super::{
    MediaError,
    decoded::{MAX_DECODED_SAMPLE_RATE, MIN_DECODED_SAMPLE_RATE},
};
use crate::resolver::contracts::ResolverCancellation;
use std::f64::consts::PI;

/// Fixed engineering budget: centered Blackman-windowed sinc with 128 source
/// frames of lookahead. The phase table never grows with a rational rate ratio.
pub(super) const FILTER_RADIUS: usize = 128;
pub(super) const FILTER_TAPS: usize = FILTER_RADIUS * 2 + 1;
pub(super) const FILTER_PHASES: usize = 256;
/// Leave a transition band below the smaller input/output Nyquist frequency.
const CUTOFF_FRACTION: f64 = 0.45;
const PCM_MIN: f64 = -32768.0;
const PCM_MAX: f64 = 32767.0;

/// One continuous media interval. The owner must finish before a gap and then
/// create a new filter. No audio or coefficient table is cloned/serialized.
pub(super) struct PreparedResampler {
    input_rate: u32,
    output_rate: u32,
    coefficients: Box<[[f64; FILTER_TAPS]]>,
    history: [f64; FILTER_TAPS],
    first: Option<f64>,
    last: Option<f64>,
    source_frames: u64,
    output_frames: u64,
    finished: bool,
    failed: bool,
}

impl PreparedResampler {
    pub(super) fn new(
        input_rate: u32,
        output_rate: u32,
        cancel: &ResolverCancellation,
    ) -> Result<Self, MediaError> {
        check(cancel)?;
        if !(MIN_DECODED_SAMPLE_RATE..=MAX_DECODED_SAMPLE_RATE).contains(&input_rate)
            || !matches!(output_rate, 16_000 | 24_000)
        {
            return Err(MediaError::InvalidAudio);
        }
        let mut coefficients = Vec::new();
        if input_rate != output_rate {
            coefficients
                .try_reserve_exact(FILTER_PHASES)
                .map_err(|_| MediaError::Overloaded)?;
            let cutoff =
                CUTOFF_FRACTION * (f64::from(output_rate) / f64::from(input_rate)).min(1.0);
            for phase in 0..FILTER_PHASES {
                check(cancel)?;
                let fraction = phase as f64 / FILTER_PHASES as f64;
                let mut row = [0.0; FILTER_TAPS];
                for (tap, coefficient) in row.iter_mut().enumerate() {
                    let offset = tap as f64 - FILTER_RADIUS as f64 - fraction;
                    let sinc = if offset == 0.0 {
                        2.0 * cutoff
                    } else {
                        (2.0 * PI * cutoff * offset).sin() / (PI * offset)
                    };
                    let angle = 2.0 * PI * tap as f64 / (FILTER_TAPS - 1) as f64;
                    let window = 0.42 - 0.5 * angle.cos() + 0.08 * (2.0 * angle).cos();
                    *coefficient = sinc * window;
                }
                let sum: f64 = row.iter().sum();
                if !sum.is_finite() || sum <= 0.0 {
                    return Err(MediaError::InvalidAudio);
                }
                for coefficient in &mut row {
                    *coefficient /= sum;
                }
                coefficients.push(row);
            }
        }
        check(cancel)?;
        Ok(Self {
            input_rate,
            output_rate,
            coefficients: coefficients.into_boxed_slice(),
            history: [0.0; FILTER_TAPS],
            first: None,
            last: None,
            source_frames: 0,
            output_frames: 0,
            finished: false,
            failed: false,
        })
    }

    pub(super) const fn source_frames(&self) -> u64 {
        self.source_frames
    }

    pub(super) const fn output_frames(&self) -> u64 {
        self.output_frames
    }

    pub(super) fn push(
        &mut self,
        sample: f64,
        cancel: &ResolverCancellation,
        emit: &mut impl FnMut(i16) -> Result<(), MediaError>,
    ) -> Result<(), MediaError> {
        let result = self.push_inner(sample, cancel, emit);
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn push_inner(
        &mut self,
        sample: f64,
        cancel: &ResolverCancellation,
        emit: &mut impl FnMut(i16) -> Result<(), MediaError>,
    ) -> Result<(), MediaError> {
        self.check_active(cancel)?;
        if !sample.is_finite() || !(PCM_MIN..=PCM_MAX).contains(&sample) {
            return Err(MediaError::InvalidAudio);
        }
        let next = self
            .source_frames
            .checked_add(1)
            .ok_or(MediaError::InvalidAudio)?;
        let slot = (self.source_frames % FILTER_TAPS as u64) as usize;
        self.history[slot] = sample;
        self.first.get_or_insert(sample);
        self.last = Some(sample);
        self.source_frames = next;
        if self.input_rate == self.output_rate {
            return self.emit(sample, cancel, emit);
        }
        let limit = self.output_limit()?;
        while self.output_frames < limit {
            check(cancel)?;
            let (center, phase) = self.position()?;
            let right = center
                .checked_add(FILTER_RADIUS as u64)
                .ok_or(MediaError::InvalidAudio)?;
            if right >= self.source_frames {
                break;
            }
            let filtered = self.filtered(center, phase, false)?;
            self.emit(filtered, cancel, emit)?;
        }
        check(cancel)
    }

    /// Extend the first/last sample only within the FIR's edge calculation.
    /// Output positions remain inside the source duration, with exactly
    /// floor(source_frames * output_rate / input_rate) output frames.
    pub(super) fn finish(
        &mut self,
        cancel: &ResolverCancellation,
        emit: &mut impl FnMut(i16) -> Result<(), MediaError>,
    ) -> Result<(), MediaError> {
        let result = self.finish_inner(cancel, emit);
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn finish_inner(
        &mut self,
        cancel: &ResolverCancellation,
        emit: &mut impl FnMut(i16) -> Result<(), MediaError>,
    ) -> Result<(), MediaError> {
        self.check_active(cancel)?;
        self.finished = true;
        let limit = self.output_limit()?;
        while self.output_frames < limit {
            check(cancel)?;
            let (center, phase) = self.position()?;
            let filtered = self.filtered(center, phase, true)?;
            self.emit(filtered, cancel, emit)?;
        }
        check(cancel)
    }

    fn check_active(&self, cancel: &ResolverCancellation) -> Result<(), MediaError> {
        check(cancel)?;
        if self.finished || self.failed {
            Err(MediaError::InvalidAudio)
        } else {
            Ok(())
        }
    }

    fn output_limit(&self) -> Result<u64, MediaError> {
        let numerator = u128::from(self.source_frames)
            .checked_mul(u128::from(self.output_rate))
            .ok_or(MediaError::InvalidAudio)?;
        u64::try_from(numerator / u128::from(self.input_rate)).map_err(|_| MediaError::InvalidAudio)
    }

    fn position(&self) -> Result<(u64, usize), MediaError> {
        let numerator = u128::from(self.output_frames)
            .checked_mul(u128::from(self.input_rate))
            .ok_or(MediaError::InvalidAudio)?;
        let denominator = u128::from(self.output_rate);
        let center =
            u64::try_from(numerator / denominator).map_err(|_| MediaError::InvalidAudio)?;
        // Floor the phase-table index; timing continues to use the exact
        // rational position. Approximation is below 1/256 of a source frame.
        let phase =
            usize::try_from((numerator % denominator) * FILTER_PHASES as u128 / denominator)
                .map_err(|_| MediaError::InvalidAudio)?;
        Ok((center, phase))
    }

    fn filtered(&self, center: u64, phase: usize, ended: bool) -> Result<f64, MediaError> {
        let row = self
            .coefficients
            .get(phase)
            .ok_or(MediaError::InvalidAudio)?;
        let mut value = 0.0;
        for (tap, coefficient) in row.iter().enumerate() {
            let index = i128::from(center) + tap as i128 - FILTER_RADIUS as i128;
            value += coefficient * self.sample_at(index, ended)?;
        }
        if value.is_finite() {
            Ok(value)
        } else {
            Err(MediaError::InvalidAudio)
        }
    }

    fn sample_at(&self, index: i128, ended: bool) -> Result<f64, MediaError> {
        if index < 0 {
            return self.first.ok_or(MediaError::InvalidAudio);
        }
        let index = u64::try_from(index).map_err(|_| MediaError::InvalidAudio)?;
        if index >= self.source_frames {
            return if ended {
                self.last.ok_or(MediaError::InvalidAudio)
            } else {
                Err(MediaError::InvalidAudio)
            };
        }
        if index < self.source_frames.saturating_sub(FILTER_TAPS as u64) {
            return Err(MediaError::InvalidAudio);
        }
        Ok(self.history[(index % FILTER_TAPS as u64) as usize])
    }

    fn emit(
        &mut self,
        value: f64,
        cancel: &ResolverCancellation,
        emit: &mut impl FnMut(i16) -> Result<(), MediaError>,
    ) -> Result<(), MediaError> {
        check(cancel)?;
        let next = self
            .output_frames
            .checked_add(1)
            .ok_or(MediaError::InvalidAudio)?;
        emit(value.round().clamp(PCM_MIN, PCM_MAX) as i16)?;
        self.output_frames = next;
        check(cancel)
    }
}

fn check(cancel: &ResolverCancellation) -> Result<(), MediaError> {
    if cancel.is_cancelled() {
        Err(MediaError::Cancelled)
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "prepared_resample_tests.rs"]
mod tests;
