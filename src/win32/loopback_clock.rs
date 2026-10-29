//! Process-loopback device positions can remain zero. Count observed PCM frames
//! only while consecutive native QPC packet times agree with that frame count.
//! This is capture-relative timing, never a media/playback position or wall clock.
use crate::audio::capture::{CaptureError, SAMPLE_RATE};

const QPC_TICKS_PER_SECOND: u128 = 10_000_000;
// GetBuffer converts QPC to integer 100 ns ticks. Two rounded endpoints can
// differ by at most one tick; this is far smaller than one 48 kHz audio frame.
const QPC_ROUNDING_TICKS: u128 = 1;

#[derive(Clone, Copy, Debug)]
pub(super) struct PacketTimestamp {
    pub position: u64,
    pub qpc: Option<u64>,
    pub discontinuity: bool,
}

struct Previous {
    qpc: u64,
    frames: u64,
    next_position: u64,
}

#[derive(Default)]
pub(super) struct LoopbackClock {
    native_counter_seen: bool,
    origin_qpc: Option<u64>,
    previous: Option<Previous>,
    timing_lost: bool,
}

impl LoopbackClock {
    pub fn observe(
        &mut self,
        native_position: u64,
        qpc: Option<u64>,
        frames: u32,
        mut discontinuity: bool,
    ) -> Result<PacketTimestamp, CaptureError> {
        if frames == 0 {
            return Err(CaptureError::InvalidData("packet_size"));
        }
        self.native_counter_seen |= native_position != 0;
        // Preserve genuinely supplied device positions and their existing checks.
        if self.native_counter_seen {
            return Ok(PacketTimestamp {
                position: native_position,
                qpc,
                discontinuity,
            });
        }
        let Some(qpc) = qpc.filter(|qpc| *qpc != 0) else {
            return Ok(self.unknown());
        };
        let position = if let Some(previous) = &self.previous {
            let Some(delta) = qpc.checked_sub(previous.qpc).filter(|delta| *delta != 0) else {
                return Ok(self.unknown());
            };
            let measured = u128::from(delta) * u128::from(SAMPLE_RATE);
            let expected = u128::from(previous.frames) * QPC_TICKS_PER_SECOND;
            if measured.abs_diff(expected) <= QPC_ROUNDING_TICKS * u128::from(SAMPLE_RATE) {
                previous.next_position
            } else {
                discontinuity = true;
                // Keep a real QPC gap in the capture position instead of closing
                // the hole with invented silence or a packet-count-only clock.
                let delta = qpc
                    .checked_sub(
                        self.origin_qpc
                            .ok_or(CaptureError::InvalidData("position"))?,
                    )
                    .ok_or(CaptureError::InvalidData("position"))?;
                u64::try_from(u128::from(delta) * u128::from(SAMPLE_RATE) / QPC_TICKS_PER_SECOND)
                    .map_err(|_| CaptureError::InvalidData("position"))?
            }
        } else {
            self.origin_qpc = Some(qpc);
            discontinuity |= self.timing_lost;
            self.timing_lost = false;
            0
        };
        self.previous = Some(Previous {
            qpc,
            frames: u64::from(frames),
            next_position: position
                .checked_add(u64::from(frames))
                .ok_or(CaptureError::InvalidData("position"))?,
        });
        Ok(PacketTimestamp {
            position,
            qpc: Some(qpc),
            discontinuity,
        })
    }

    fn unknown(&mut self) -> PacketTimestamp {
        self.previous = None;
        self.origin_qpc = None;
        self.timing_lost = true;
        PacketTimestamp {
            position: 0,
            qpc: None,
            discontinuity: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_native_positions_follow_observed_frames_and_real_qpc() {
        let mut clock = LoopbackClock::default();
        for index in 0..100 {
            let packet = clock
                .observe(0, Some(1_000_000 + index * 100_000), 480, false)
                .unwrap();
            assert_eq!(packet.position, index * 480);
            assert_eq!(packet.qpc, Some(1_000_000 + index * 100_000));
            assert!(!packet.discontinuity);
        }
    }

    #[test]
    fn variable_packets_keep_exact_frame_counts_across_qpc_rounding() {
        let mut clock = LoopbackClock::default();
        let mut total_frames = 0_u64;
        for frames in [1_u32, 137, 959, 480, 1, 961, 97]
            .into_iter()
            .cycle()
            .take(1000)
        {
            let qpc = 1_000_000 + total_frames * 10_000_000 / u64::from(SAMPLE_RATE);
            let packet = clock.observe(0, Some(qpc), frames, false).unwrap();
            assert_eq!(packet.position, total_frames);
            assert_eq!(packet.qpc, Some(qpc));
            assert!(!packet.discontinuity);
            total_frames += u64::from(frames);
        }
    }

    #[test]
    fn qpc_gap_and_native_glitch_are_preserved_without_inventing_pcm() {
        let mut clock = LoopbackClock::default();
        clock.observe(0, Some(1_000_000), 480, false).unwrap();
        let gap = clock.observe(0, Some(1_200_000), 480, false).unwrap();
        assert_eq!(gap.position, 960);
        assert!(gap.discontinuity);
        let next = clock.observe(0, Some(1_300_000), 480, false).unwrap();
        assert_eq!(next.position, 1440);
        assert!(!next.discontinuity);
        assert!(
            clock
                .observe(0, Some(1_400_000), 480, true)
                .unwrap()
                .discontinuity
        );
    }

    #[test]
    fn unknown_repeated_or_backward_qpc_never_fabricates_continuity() {
        for invalid in [None, Some(0), Some(1_000_000), Some(999_999)] {
            let mut clock = LoopbackClock::default();
            clock.observe(0, Some(1_000_000), 480, false).unwrap();
            let unknown = clock.observe(0, invalid, 480, false).unwrap();
            assert_eq!(unknown.qpc, None);
            assert!(unknown.discontinuity);
            let recovered = clock.observe(0, Some(1_300_000), 480, false).unwrap();
            assert!(recovered.discontinuity);
        }
    }

    #[test]
    fn integer_qpc_rounding_is_bounded_below_a_single_lost_frame() {
        for rounding in [-1_i64, 0, 1] {
            let mut clock = LoopbackClock::default();
            clock.observe(0, Some(1_000_000), 480, false).unwrap();
            assert!(
                !clock
                    .observe(0, Some((1_100_000 + rounding) as u64), 480, false)
                    .unwrap()
                    .discontinuity
            );
        }
        let mut clock = LoopbackClock::default();
        clock.observe(0, Some(1_000_000), 480, false).unwrap();
        assert!(
            clock
                .observe(0, Some(1_100_209), 480, false)
                .unwrap()
                .discontinuity
        );
    }

    #[test]
    fn real_native_counter_is_never_replaced_by_zero_position_fallback() {
        let mut clock = LoopbackClock::default();
        assert_eq!(
            clock
                .observe(0, Some(1_000_000), 480, false)
                .unwrap()
                .position,
            0
        );
        assert_eq!(
            clock
                .observe(480, Some(1_100_000), 480, false)
                .unwrap()
                .position,
            480
        );
        assert_eq!(
            clock
                .observe(0, Some(1_200_000), 480, false)
                .unwrap()
                .position,
            0
        );
        assert!(clock.observe(0, Some(1_300_000), 0, false).is_err());
    }
}
