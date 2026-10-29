//! Session-owned causal FIR decimation. Call on a bounded preprocessing worker,
//! never from WASAPI callbacks. No allocation, network, or file I/O per block.
use super::{
    capture::{AudioStamp, BLOCK_FRAMES, SAMPLE_RATE, SessionId},
    preprocess::NormalizedAudio,
};
use std::f64::consts::PI;

/// Odd linear-phase Hamming FIR; 63 input frames (1.3125 ms) group delay.
pub const FILTER_TAPS: usize = 127;
pub const FILTER_DELAY_FRAMES: usize = (FILTER_TAPS - 1) / 2;
/// 90% of output Nyquist, leaving a transition band before decimation aliases.
const CUTOFF_NYQUIST_FRACTION: f64 = 0.9;
pub const MAX_OUTPUT_FRAMES: usize = BLOCK_FRAMES / 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ResampleError {
    #[error("重採樣只支援 48 kHz 單聲道轉為 24 或 16 kHz。")]
    UnsupportedRate,
    #[error("重採樣輸入格式或工作階段無效。")]
    InvalidInput,
    #[error("重採樣音訊不連續，必須建立新的重採樣工作階段。")]
    Discontinuity,
}

pub struct ResampledAudio {
    pub stamp: AudioStamp,
    pub sample_rate: u32,
    pub frames: usize,
    pub samples: [i16; MAX_OUTPUT_FRAMES],
    /// First output's sampling position relative to this input block (0..factor).
    pub first_input_offset: usize,
}
impl ResampledAudio {
    /// Content time after compensating FIR delay. Startup or unreliable timestamps
    /// remain unknown rather than clamping negative content time to a fake zero.
    pub fn start_us(&self) -> Option<u64> {
        if self.frames == 0 {
            return None;
        }
        self.stamp
            .frame_position?
            .checked_add(self.first_input_offset as u64)?
            .checked_sub(FILTER_DELAY_FRAMES as u64)?
            .checked_mul(1_000_000)
            .map(|value| value / u64::from(SAMPLE_RATE))
    }
    pub fn pcm_le_bytes(&self) -> impl Iterator<Item = u8> + '_ {
        self.samples[..self.frames]
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
    }
}

pub struct StreamingResampler {
    session: SessionId,
    factor: usize,
    sample_rate: u32,
    coefficients: [f64; FILTER_TAPS],
    history: [f32; FILTER_TAPS],
    head: usize,
    phase: usize,
    previous: Option<(u64, Option<u64>, usize)>,
    failed: bool,
}
impl StreamingResampler {
    pub fn new(session: SessionId, output_rate: u32) -> Result<Self, ResampleError> {
        let factor = match output_rate {
            24000 => 2,
            16000 => 3,
            _ => return Err(ResampleError::UnsupportedRate),
        };
        let cutoff = CUTOFF_NYQUIST_FRACTION / (2.0 * factor as f64);
        let mut coefficients = [0.0; FILTER_TAPS];
        for (index, coefficient) in coefficients.iter_mut().enumerate() {
            let offset = index as f64 - FILTER_DELAY_FRAMES as f64;
            let sinc = if offset == 0.0 {
                2.0 * cutoff
            } else {
                (2.0 * PI * cutoff * offset).sin() / (PI * offset)
            };
            let window = 0.54 - 0.46 * (2.0 * PI * index as f64 / (FILTER_TAPS - 1) as f64).cos();
            *coefficient = sinc * window;
        }
        let sum: f64 = coefficients.iter().sum();
        for coefficient in &mut coefficients {
            *coefficient /= sum;
        }
        Ok(Self {
            session,
            factor,
            sample_rate: output_rate,
            coefficients,
            history: [0.0; FILTER_TAPS],
            head: 0,
            phase: 0,
            previous: None,
            failed: false,
        })
    }
    pub fn process(&mut self, input: &NormalizedAudio) -> Result<ResampledAudio, ResampleError> {
        if self.failed {
            return Err(ResampleError::Discontinuity);
        }
        if input.stamp.session != self.session
            || input.frames == 0
            || input.frames > BLOCK_FRAMES
            || input.samples[..input.frames]
                .iter()
                .any(|sample| !sample.is_finite() || !(-1.0..=1.0).contains(sample))
        {
            return Err(ResampleError::InvalidInput);
        }
        if self.previous.is_some_and(|(sequence, position, frames)| {
            input.stamp.discontinuity
                || sequence.checked_add(1) != Some(input.stamp.sequence)
                || match (position, input.stamp.frame_position) {
                    (Some(previous), Some(current)) => {
                        previous.checked_add(frames as u64) != Some(current)
                    }
                    _ => false,
                }
        }) {
            self.failed = true;
            self.history.fill(0.0);
            return Err(ResampleError::Discontinuity);
        }
        let mut output = ResampledAudio {
            stamp: input.stamp,
            sample_rate: self.sample_rate,
            frames: 0,
            samples: [0; MAX_OUTPUT_FRAMES],
            first_input_offset: (self.factor - self.phase) % self.factor,
        };
        for sample in &input.samples[..input.frames] {
            self.history[self.head] = *sample;
            if self.phase == 0 {
                let filtered: f64 = self
                    .coefficients
                    .iter()
                    .enumerate()
                    .map(|(lag, coefficient)| {
                        coefficient
                            * f64::from(self.history[(self.head + FILTER_TAPS - lag) % FILTER_TAPS])
                    })
                    .sum();
                output.samples[output.frames] =
                    (filtered * 32768.0).round().clamp(-32768.0, 32767.0) as i16;
                output.frames += 1;
            }
            self.head = (self.head + 1) % FILTER_TAPS;
            self.phase = (self.phase + 1) % self.factor;
        }
        self.previous = Some((
            input.stamp.sequence,
            input.stamp.frame_position,
            input.frames,
        ));
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    fn block(sequence: u64, position: u64, input: &[f32]) -> NormalizedAudio {
        let mut samples = [0.0; BLOCK_FRAMES];
        samples[..input.len()].copy_from_slice(input);
        NormalizedAudio {
            stamp: AudioStamp {
                session: SessionId(19),
                sequence,
                frame_position: Some(position),
                qpc_100ns: None,
                received: Instant::now(),
                discontinuity: false,
            },
            frames: input.len(),
            samples,
        }
    }
    fn tone(hz: f64) -> Vec<f32> {
        (0..9600)
            .map(|index| {
                (0.5 * (2.0 * PI * hz * index as f64 / f64::from(SAMPLE_RATE)).sin()) as f32
            })
            .collect()
    }
    fn converted(input: &[f32], rate: u32, chunk: usize) -> Vec<i16> {
        let mut resampler = StreamingResampler::new(SessionId(19), rate).unwrap();
        input
            .chunks(chunk)
            .enumerate()
            .flat_map(|(index, part)| {
                let output = resampler
                    .process(&block(index as u64, (index * chunk) as u64, part))
                    .unwrap();
                output.samples[..output.frames].to_vec()
            })
            .collect()
    }
    fn rms(samples: &[i16]) -> f64 {
        (samples
            .iter()
            .map(|sample| (f64::from(*sample) / 32768.0).powi(2))
            .sum::<f64>()
            / samples.len() as f64)
            .sqrt()
    }

    #[test]
    fn cross_block_state_is_bit_identical_for_arbitrary_chunk_boundaries() {
        let source = tone(1730.0);
        for rate in [24000, 16000] {
            let regular = converted(&source, rate, BLOCK_FRAMES);
            for chunk in [1, 137, 959] {
                assert_eq!(regular, converted(&source, rate, chunk));
            }
            assert_eq!(
                regular.len(),
                source.len() * rate as usize / SAMPLE_RATE as usize
            );
        }
    }
    #[test]
    fn passband_is_preserved_and_aliasing_is_attenuated() {
        for (rate, rejected_hz) in [(24000, 18000.0), (16000, 12000.0)] {
            let pass = converted(&tone(1000.0), rate, BLOCK_FRAMES);
            let rejected = converted(&tone(rejected_hz), rate, BLOCK_FRAMES);
            let pass_rms = rms(&pass[256..]);
            let stop_rms = rms(&rejected[256..]);
            assert!(
                (0.345..0.362).contains(&pass_rms),
                "passband amplitude {pass_rms}"
            );
            assert!(stop_rms < 0.001, "stopband amplitude {stop_rms}");
        }
    }
    #[test]
    fn silence_and_exact_pcm_byte_order_are_preserved() {
        assert!(
            converted(&vec![0.0; 1920], 16000, 960)
                .iter()
                .all(|sample| *sample == 0)
        );
        let mut resampler = StreamingResampler::new(SessionId(19), 24000).unwrap();
        let output = resampler.process(&block(0, 0, &[0.0; 960])).unwrap();
        assert_eq!(output.frames, 480);
        assert_eq!(output.pcm_le_bytes().count(), 960);
    }
    #[test]
    fn source_and_sequence_gaps_require_a_fresh_filter() {
        let mut resampler = StreamingResampler::new(SessionId(19), 16000).unwrap();
        let mut wrong = block(0, 0, &[0.0; 960]);
        wrong.stamp.session = SessionId(20);
        assert!(matches!(
            resampler.process(&wrong),
            Err(ResampleError::InvalidInput)
        ));
        resampler.process(&block(0, 0, &[0.1; 960])).unwrap();
        assert!(matches!(
            resampler.process(&block(2, 1920, &[0.1; 960])),
            Err(ResampleError::Discontinuity)
        ));
        assert!(matches!(
            resampler.process(&block(1, 960, &[0.1; 960])),
            Err(ResampleError::Discontinuity)
        ));
        assert!(StreamingResampler::new(SessionId(19), 44100).is_err());
    }
    #[test]
    fn filter_delay_is_explicit_and_unknown_timing_stays_unknown() {
        let mut resampler = StreamingResampler::new(SessionId(19), 24000).unwrap();
        assert!(
            resampler
                .process(&block(0, 0, &[0.0; 960]))
                .unwrap()
                .start_us()
                .is_none()
        );
        assert_eq!(
            resampler
                .process(&block(1, 960, &[0.0; 960]))
                .unwrap()
                .start_us(),
            Some(18687)
        );
        let mut unknown = block(2, 1920, &[0.0; 960]);
        unknown.stamp.frame_position = None;
        assert!(resampler.process(&unknown).unwrap().start_us().is_none());
    }
}
