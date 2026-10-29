//! Provider-independent normalization; no resampling or speech processing in Phase 2.
use super::capture::{AudioStamp, BLOCK_FRAMES, PcmBlock, SAMPLE_RATE};

/// Mono f32 in [-1, 1), 48 kHz. Only samples[..frames] is valid.
/// Never serialize this session-only in-memory value.
pub struct NormalizedAudio {
    pub stamp: AudioStamp,
    pub frames: usize,
    pub samples: [f32; BLOCK_FRAMES],
}

impl NormalizedAudio {
    pub const SAMPLE_RATE: u32 = SAMPLE_RATE;
    pub const CHANNELS: u16 = 1;

    pub fn start_us(&self) -> Option<u64> {
        self.stamp
            .frame_position
            .and_then(|frame| frame.checked_mul(1_000_000))
            .map(|value| value / u64::from(SAMPLE_RATE))
    }

    pub fn duration_us(&self) -> u64 {
        self.frames as u64 * 1_000_000 / u64::from(SAMPLE_RATE)
    }
}

pub(crate) fn normalize(raw: PcmBlock) -> NormalizedAudio {
    let mut result = NormalizedAudio {
        stamp: raw.stamp,
        frames: raw.frames,
        samples: [0.0; BLOCK_FRAMES],
    };
    for (output, stereo) in result.samples[..raw.frames]
        .iter_mut()
        .zip(raw.samples.as_chunks::<2>().0)
    {
        *output = (f32::from(stereo[0]) + f32::from(stereo[1])) / 65536.0;
    }
    result
}
