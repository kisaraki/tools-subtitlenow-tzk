//! Synthetic in-memory PCM only. No recorded audio, file, device, or network.
use super::*;
use crate::audio::capture::SessionId;

fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(41),
        generation: 1,
    }
}
fn mono() -> DecodedAudioFormat {
    DecodedAudioFormat::new(16_000, 1).unwrap()
}
fn block(sequence: u64, timestamp_us: u64) -> DecodedAudioBlock {
    DecodedAudioBlock::new(identity(), sequence, timestamp_us, mono(), vec![0; 160]).unwrap()
}
fn gate() -> DecodedAudioGate {
    DecodedAudioGate::new(identity(), mono()).unwrap()
}

#[test]
fn format_accepts_only_bounded_pcm_rates_and_channel_counts() {
    for sample_rate in [8_000, 16_000, 44_100, 48_000, 192_000] {
        for channels in [1, 2, 8] {
            let format = DecodedAudioFormat::new(sample_rate, channels).unwrap();
            assert_eq!(format.sample_rate(), sample_rate);
            assert_eq!(format.channels(), channels);
        }
    }
    for sample_rate in [0, 7_999, 192_001, u32::MAX] {
        assert_eq!(
            DecodedAudioFormat::new(sample_rate, 1),
            Err(MediaError::InvalidAudio)
        );
    }
    for channels in [0, 9, u16::MAX] {
        assert_eq!(
            DecodedAudioFormat::new(16_000, channels),
            Err(MediaError::InvalidAudio)
        );
    }
}

#[test]
fn interleaved_pcm_duration_counts_frames_rather_than_samples() {
    let format = DecodedAudioFormat::new(48_000, 2).unwrap();
    let samples: Vec<_> = (0..960).map(|value| value as i16 - 480).collect();
    let decoded = DecodedAudioBlock::new(identity(), 1, 500_000, format, samples.clone()).unwrap();
    assert_eq!(decoded.identity(), identity());
    assert_eq!(decoded.sequence(), 1);
    assert_eq!(decoded.timestamp_us(), 500_000);
    assert_eq!(decoded.format(), format);
    assert_eq!(decoded.samples(), samples);
    assert_eq!(decoded.frames(), 480);
    assert_eq!(decoded.duration_us(), 10_000);
    assert_eq!(decoded.end_us(), 510_000);
}

#[test]
fn owned_block_limit_is_exact_and_does_not_truncate_audio() {
    let maximum = DecodedAudioBlock::new(
        identity(),
        1,
        0,
        mono(),
        vec![i16::MAX; MAX_DECODED_BLOCK_SAMPLES],
    )
    .unwrap();
    assert_eq!(std::mem::size_of_val(maximum.samples()), 64 * 1024);
    assert!(maximum.samples().iter().all(|sample| *sample == i16::MAX));
    assert_eq!(
        DecodedAudioBlock::new(
            identity(),
            1,
            0,
            mono(),
            vec![0; MAX_DECODED_BLOCK_SAMPLES + 1]
        )
        .err(),
        Some(MediaError::InvalidAudio)
    );
}

#[test]
fn empty_or_incomplete_interleaved_frames_are_rejected() {
    assert_eq!(
        DecodedAudioBlock::new(identity(), 1, 0, mono(), Vec::new()).err(),
        Some(MediaError::InvalidAudio)
    );
    let stereo = DecodedAudioFormat::new(48_000, 2).unwrap();
    assert_eq!(
        DecodedAudioBlock::new(identity(), 1, 0, stereo, vec![1, 2, 3]).err(),
        Some(MediaError::InvalidAudio)
    );
}

#[test]
fn audio_requires_nonzero_session_generation_and_sequence() {
    for invalid_identity in [
        StreamIdentity {
            session: SessionId(0),
            generation: 1,
        },
        StreamIdentity {
            session: SessionId(41),
            generation: 0,
        },
    ] {
        assert_eq!(
            DecodedAudioBlock::new(invalid_identity, 1, 0, mono(), vec![0]).err(),
            Some(MediaError::InvalidAudio)
        );
        assert_eq!(
            DecodedAudioGate::new(invalid_identity, mono()).err(),
            Some(MediaError::InvalidAudio)
        );
    }
    assert_eq!(
        DecodedAudioBlock::new(identity(), 0, 0, mono(), vec![0]).err(),
        Some(MediaError::InvalidAudio)
    );
}

#[test]
fn timestamp_overflow_is_rejected_before_block_publication() {
    assert_eq!(
        DecodedAudioBlock::new(identity(), 1, u64::MAX, mono(), vec![0]).err(),
        Some(MediaError::InvalidAudio)
    );
    let decoded = DecodedAudioBlock::new(identity(), 1, u64::MAX - 62, mono(), vec![0]).unwrap();
    assert_eq!(decoded.duration_us(), 62);
    assert_eq!(decoded.end_us(), u64::MAX);
}

#[test]
fn media_duration_uses_sample_rational_without_wall_clock() {
    let format = DecodedAudioFormat::new(44_100, 1).unwrap();
    let decoded = DecodedAudioBlock::new(identity(), 1, 123, format, vec![0; 100]).unwrap();
    assert_eq!(decoded.duration_us(), 2_267);
    assert_eq!(decoded.end_us(), 2_390);
    let fastest = DecodedAudioFormat::new(192_000, 1).unwrap();
    let decoded = DecodedAudioBlock::new(identity(), 1, 0, fastest, vec![0]).unwrap();
    assert_eq!(decoded.duration_us(), 5);
}

#[test]
fn gate_requires_first_sequence_and_preserves_first_actual_media_time() {
    let mut gate = gate();
    assert_eq!(
        gate.validate(&block(2, 900_000)),
        Err(MediaError::InvalidAudio)
    );
    assert_eq!(gate.validate(&block(1, 900_000)), Ok(false));
    assert_eq!(gate.validate(&block(2, 910_000)), Ok(false));
}

#[test]
fn old_session_or_generation_cannot_advance_current_gate() {
    let mut gate = gate();
    assert_eq!(gate.validate(&block(1, 0)), Ok(false));
    for stale_identity in [
        StreamIdentity {
            session: SessionId(40),
            generation: 1,
        },
        StreamIdentity {
            session: SessionId(41),
            generation: 2,
        },
    ] {
        let stale =
            DecodedAudioBlock::new(stale_identity, 2, 10_000, mono(), vec![0; 160]).unwrap();
        assert_eq!(gate.validate(&stale), Err(MediaError::Stale));
    }
    assert_eq!(gate.validate(&block(2, 10_000)), Ok(false));
}

#[test]
fn changed_audio_format_is_rejected_without_advancing() {
    let mut gate = gate();
    assert_eq!(gate.validate(&block(1, 0)), Ok(false));
    let changed = DecodedAudioBlock::new(
        identity(),
        2,
        10_000,
        DecodedAudioFormat::new(48_000, 2).unwrap(),
        vec![0; 960],
    )
    .unwrap();
    assert_eq!(gate.validate(&changed), Err(MediaError::InvalidAudio));
    assert_eq!(gate.validate(&block(2, 10_000)), Ok(false));
}

#[test]
fn gate_rejects_duplicate_and_out_of_order_completion() {
    let mut gate = gate();
    assert_eq!(gate.validate(&block(1, 0)), Ok(false));
    assert_eq!(gate.validate(&block(1, 0)), Err(MediaError::InvalidAudio));
    assert_eq!(
        gate.validate(&block(3, 20_000)),
        Err(MediaError::InvalidAudio)
    );
    assert_eq!(gate.validate(&block(2, 10_000)), Ok(false));
    assert_eq!(gate.validate(&block(3, 20_000)), Ok(false));
}

#[test]
fn forward_gap_is_explicit_and_does_not_insert_synthetic_silence() {
    let mut gate = gate();
    assert_eq!(gate.validate(&block(1, 0)), Ok(false));
    let later = block(2, 50_000);
    assert_eq!(gate.validate(&later), Ok(true));
    assert_eq!(later.samples().len(), 160);
    assert_eq!(later.timestamp_us(), 50_000);
    assert_eq!(gate.validate(&block(3, 60_000)), Ok(false));
}

#[test]
fn independent_microsecond_rounding_does_not_create_false_gap_or_overlap() {
    for difference in [-1_i64, 0, 1] {
        let mut gate = gate();
        let format = DecodedAudioFormat::new(44_100, 1).unwrap();
        gate.format = format;
        let first = DecodedAudioBlock::new(identity(), 1, 0, format, vec![0; 100]).unwrap();
        assert_eq!(gate.validate(&first), Ok(false));
        let timestamp = u64::try_from(first.end_us() as i64 + difference).unwrap();
        let next = DecodedAudioBlock::new(identity(), 2, timestamp, format, vec![0; 100]).unwrap();
        assert_eq!(gate.validate(&next), Ok(false));
    }
}

#[test]
fn actual_overlap_and_backwards_media_time_do_not_advance_gate() {
    let mut gate = gate();
    assert_eq!(gate.validate(&block(1, 50_000)), Ok(false));
    for timestamp in [49_999, 50_000, 59_998] {
        assert_eq!(
            gate.validate(&block(2, timestamp)),
            Err(MediaError::InvalidAudio)
        );
    }
    assert_eq!(gate.validate(&block(2, 60_000)), Ok(false));
}

#[test]
fn sequence_exhaustion_cannot_wrap_or_advance_gate() {
    let mut gate = gate();
    gate.next_sequence = u64::MAX;
    assert_eq!(
        gate.validate(&block(u64::MAX, 0)),
        Err(MediaError::InvalidAudio)
    );
    assert_eq!(gate.next_sequence, u64::MAX);
    assert_eq!(gate.previous, None);
}

#[test]
fn pull_contract_can_stop_without_emitting_audio_when_cancelled() {
    struct SyntheticDecoder {
        next_block: Option<DecodedAudioBlock>,
    }
    impl MediaDecoder for SyntheticDecoder {
        fn format(&self) -> DecodedAudioFormat {
            mono()
        }
        fn next(
            &mut self,
            cancel: &ResolverCancellation,
        ) -> Result<Option<DecodedAudioBlock>, MediaError> {
            if cancel.is_cancelled() {
                return Err(MediaError::Cancelled);
            }
            Ok(self.next_block.take())
        }
    }
    let mut decoder = SyntheticDecoder {
        next_block: Some(block(1, 0)),
    };
    let cancel = ResolverCancellation::default();
    assert_eq!(decoder.format(), mono());
    cancel.cancel();
    assert_eq!(decoder.next(&cancel).err(), Some(MediaError::Cancelled));
    assert!(decoder.next_block.is_some());
    let active = ResolverCancellation::default();
    assert_eq!(decoder.next(&active).unwrap().unwrap().sequence(), 1);
    assert!(decoder.next(&active).unwrap().is_none());
}
