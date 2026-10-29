//! Live recognition input only. No capture/mixer changes, speech classification or I/O.
use super::speech::{AudioIssue, AudioStage, SpeechError};
use crate::audio::{capture::BLOCK_FRAMES, preprocess::NormalizedAudio};

/// A modest recognition level, not a playback loudness target or a speech guarantee.
const TARGET_RMS: f64 = 0.03;
/// Bound amplification to 24 dB. Gain cannot recover speech from background noise.
const MAX_GAIN: f64 = 16.0;
/// Leave very faint input/dither alone rather than driving it toward the target.
const MIN_INPUT_RMS: f64 = 0.0002;
/// Reserve headroom on boosted blocks. Already loud input is never attenuated.
const BOOST_PEAK_LIMIT: f64 = 0.9;

#[derive(Clone, Copy)]
pub enum InputLevelPolicy {
    Original,
    BoundedGain,
}

/// A single, bounded block operation on the existing preprocessing worker.
/// Validate before mutation: never turn invalid PCM into apparently valid samples.
pub(super) fn boost_soft_input(input: &mut NormalizedAudio) -> Result<bool, SpeechError> {
    let invalid =
        || SpeechError::audio_failure(AudioStage::Preprocess, AudioIssue::ResamplerRejected);
    if input.frames == 0 || input.frames > BLOCK_FRAMES {
        return Err(invalid());
    }
    let mut square_sum = 0.0_f64;
    let mut peak = 0.0_f64;
    for sample in &input.samples[..input.frames] {
        if !sample.is_finite() || !(-1.0..=1.0).contains(sample) {
            return Err(invalid());
        }
        let sample = f64::from(*sample);
        square_sum += sample * sample;
        peak = peak.max(sample.abs());
    }
    let rms = (square_sum / input.frames as f64).sqrt();
    if !(MIN_INPUT_RMS..TARGET_RMS).contains(&rms) {
        return Ok(false);
    }
    let gain = (TARGET_RMS / rms)
        .min(MAX_GAIN)
        .min(BOOST_PEAK_LIMIT / peak)
        .max(1.0);
    if gain <= 1.0 {
        return Ok(false);
    }
    for sample in &mut input.samples[..input.frames] {
        *sample = (f64::from(*sample) * gain) as f32;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::capture::{AudioStamp, SessionId};
    use std::time::Instant;
    fn block(level: f32) -> NormalizedAudio {
        NormalizedAudio {
            stamp: AudioStamp {
                session: SessionId(1),
                sequence: 4,
                frame_position: Some(3840),
                qpc_100ns: Some(900),
                received: Instant::now(),
                discontinuity: false,
            },
            frames: BLOCK_FRAMES,
            samples: [level; BLOCK_FRAMES],
        }
    }
    #[test]
    fn phase11_soft_input_gain_is_bounded_and_preserves_identity_frames_and_time() {
        let mut input = block(0.0005);
        let stamp = input.stamp;
        assert!(boost_soft_input(&mut input).unwrap());
        assert!(
            input
                .samples
                .iter()
                .all(|sample| (*sample - 0.008).abs() < 1e-7)
        );
        assert_eq!(input.frames, BLOCK_FRAMES);
        assert_eq!(input.stamp.session, stamp.session);
        assert_eq!(input.stamp.sequence, stamp.sequence);
        assert_eq!(input.stamp.frame_position, stamp.frame_position);
        assert_eq!(input.stamp.qpc_100ns, stamp.qpc_100ns);
        assert_eq!(input.stamp.received, stamp.received);
        assert_eq!(input.stamp.discontinuity, stamp.discontinuity);
    }
    #[test]
    fn phase11_soft_input_gain_preserves_silence_faint_dither_and_normal_levels() {
        for level in [0.0, 0.00003, -0.00003, 0.1, -0.1, 1.0, -1.0] {
            let mut input = block(level);
            assert!(!boost_soft_input(&mut input).unwrap());
            assert!(
                input
                    .samples
                    .iter()
                    .all(|sample| sample.to_bits() == level.to_bits())
            );
        }
    }
    #[test]
    fn phase11_soft_input_gain_limits_peaks_without_flattening_or_hiding_invalid_pcm() {
        let mut input = block(0.0005);
        input.samples[0] = 0.8;
        assert!(boost_soft_input(&mut input).unwrap());
        assert!((input.samples[0] - 0.9).abs() < 1e-6);
        assert!(input.samples[1] > 0.0005 && input.samples[1] < 0.001);
        for invalid in [f32::NAN, f32::INFINITY, -f32::INFINITY, 1.01, -1.01] {
            let mut input = block(0.0005);
            input.samples[1] = invalid;
            assert_eq!(
                boost_soft_input(&mut input),
                Err(SpeechError::audio_failure(
                    AudioStage::Preprocess,
                    AudioIssue::ResamplerRejected
                ))
            );
            assert_eq!(input.samples[0], 0.0005);
        }
        for frames in [0, BLOCK_FRAMES + 1] {
            let mut input = block(0.0005);
            input.frames = frames;
            assert!(boost_soft_input(&mut input).is_err());
        }
        let mut input = block(f32::NAN);
        input.frames = 4;
        input.samples[..4].fill(0.001);
        assert!(boost_soft_input(&mut input).unwrap());
        assert!(input.samples[4].is_nan()); // Padding is not input audio.
    }
}
