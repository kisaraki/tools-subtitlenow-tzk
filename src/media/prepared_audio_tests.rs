//! Synthetic PCM stays in test RAM. No device, provider, content file or key.
use super::*;
use crate::audio::capture::SessionId;

fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(81),
        generation: 2,
    }
}
fn format(rate: u32, channels: u16) -> DecodedAudioFormat {
    DecodedAudioFormat::new(rate, channels).unwrap()
}
fn input(
    sequence: u64,
    timestamp_us: u64,
    format: DecodedAudioFormat,
    samples: Vec<i16>,
) -> DecodedAudioBlock {
    DecodedAudioBlock::new(identity(), sequence, timestamp_us, format, samples).unwrap()
}
fn processor(
    format: DecodedAudioFormat,
    rate: PreparedSampleRate,
    cancel: &ResolverCancellation,
) -> PreparedAudioProcessor {
    PreparedAudioProcessor::new(identity(), format, rate, cancel).unwrap()
}
fn discard(_: &PreparedPcmBlock, _: &ResolverCancellation) -> Result<(), MediaError> {
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    identity: StreamIdentity,
    sequence: u64,
    timestamp_us: u64,
    duration_us: u64,
    end_us: u64,
    rate: u32,
    frames: usize,
    discontinuity: bool,
    samples: Vec<i16>,
}
fn record(output: &mut Vec<Snapshot>, block: &PreparedPcmBlock) {
    output.push(Snapshot {
        identity: block.identity(),
        sequence: block.sequence(),
        timestamp_us: block.timestamp_us(),
        duration_us: block.duration_us(),
        end_us: block.end_us(),
        rate: block.sample_rate(),
        frames: block.frames(),
        discontinuity: block.discontinuity(),
        samples: block.samples().to_vec(),
    });
}
fn preprocess(
    samples: &[i16],
    format: DecodedAudioFormat,
    rate: PreparedSampleRate,
    anchor_us: u64,
    sizes: &[usize],
) -> (PreparedAudioSummary, Vec<Snapshot>) {
    let cancel = ResolverCancellation::default();
    let mut processor = processor(format, rate, &cancel);
    let mut output = Vec::new();
    let mut emit = |block: &PreparedPcmBlock, _: &ResolverCancellation| {
        record(&mut output, block);
        Ok(())
    };
    let frames = samples.len() / usize::from(format.channels());
    let mut offset = 0;
    let mut sequence = 1;
    while offset < frames {
        let count = sizes[(sequence as usize - 1) % sizes.len()].min(frames - offset);
        let begin = offset * usize::from(format.channels());
        let end = (offset + count) * usize::from(format.channels());
        let timestamp_us = frame_time(anchor_us, offset as u64, format.sample_rate()).unwrap();
        let block = input(sequence, timestamp_us, format, samples[begin..end].to_vec());
        processor.push(&block, &cancel, &mut emit).unwrap();
        offset += count;
        sequence += 1;
    }
    let summary = processor.finish(&cancel, &mut emit).unwrap();
    (summary, output)
}
fn signal(frames: usize, channels: u16) -> Vec<i16> {
    (0..frames)
        .flat_map(|frame| {
            (0..channels).map(move |channel| {
                (((frame * 173 + usize::from(channel) * 1_311) % 50_001) as i32 - 25_000) as i16
            })
        })
        .collect()
}

#[test]
fn supported_output_rates_have_exact_twenty_millisecond_limits() {
    assert_eq!(PreparedSampleRate::Hz16000.hz(), 16_000);
    assert_eq!(PreparedSampleRate::Hz24000.hz(), 24_000);
    assert_eq!(PreparedSampleRate::Hz16000.block_frames(), 320);
    assert_eq!(PreparedSampleRate::Hz24000.block_frames(), 480);
}

#[test]
fn equal_rate_mono_is_exact_with_media_anchor_and_partial_tail() {
    for rate in [PreparedSampleRate::Hz16000, PreparedSampleRate::Hz24000] {
        let samples: Vec<_> = (0..997).map(|value| value as i16 - 500).collect();
        let anchor = 7_654_321;
        let (summary, output) =
            preprocess(&samples, format(rate.hz(), 1), rate, anchor, &[37, 311]);
        let observed: Vec<_> = output
            .iter()
            .flat_map(|block| block.samples.iter().copied())
            .collect();
        assert_eq!(observed, samples);
        assert_eq!(summary.identity(), identity());
        assert_eq!(summary.sample_rate(), rate.hz());
        assert_eq!(summary.frames(), samples.len() as u64);
        assert_eq!(summary.blocks(), output.len() as u64);
        assert_eq!(summary.first_timestamp_us(), anchor);
        assert_eq!(
            summary.last_end_us(),
            anchor + samples.len() as u64 * 1_000_000 / u64::from(rate.hz())
        );
        assert_eq!(summary.gaps(), 0);
        let mut sent = 0;
        for (index, block) in output.iter().enumerate() {
            assert_eq!(block.identity, identity());
            assert_eq!(block.sequence, index as u64 + 1);
            assert_eq!(
                block.timestamp_us,
                frame_time(anchor, sent, rate.hz()).unwrap()
            );
            assert_eq!(block.rate, rate.hz());
            assert_eq!(block.frames, block.samples.len());
            assert!(block.frames <= rate.block_frames());
            assert!(block.duration_us <= 20_000);
            assert!(!block.discontinuity);
            sent += block.frames as u64;
        }
        assert_eq!(output.last().unwrap().frames, 997 % rate.block_frames());
    }
}

#[test]
fn equal_weight_downmix_handles_all_channels_and_extrema_without_overflow() {
    for channels in 1..=8 {
        for sample in [i16::MIN, -1_234, 0, 12_345, i16::MAX] {
            let samples = vec![sample; 321 * usize::from(channels)];
            let (_, output) = preprocess(
                &samples,
                format(16_000, channels),
                PreparedSampleRate::Hz16000,
                0,
                &[77],
            );
            assert!(
                output
                    .iter()
                    .flat_map(|block| &block.samples)
                    .all(|actual| *actual == sample)
            );
        }
    }
    let samples: Vec<_> = (0..321)
        .flat_map(|_| [-30_000, 10_000, 20_000, 24_000])
        .collect();
    let (_, output) = preprocess(
        &samples,
        format(24_000, 4),
        PreparedSampleRate::Hz24000,
        0,
        &[73],
    );
    assert!(
        output
            .iter()
            .flat_map(|block| &block.samples)
            .all(|sample| *sample == 6_000)
    );
}

#[test]
fn supported_native_rates_and_channel_counts_keep_exact_floor_counts() {
    for native_rate in [
        8_000, 12_000, 16_000, 24_000, 44_100, 48_000, 96_000, 192_000,
    ] {
        for channels in [1, 2, 8] {
            let frames = native_rate as usize / 40 + 13;
            let samples = signal(frames, channels);
            for rate in [PreparedSampleRate::Hz16000, PreparedSampleRate::Hz24000] {
                let anchor = 123_456;
                let (summary, output) = preprocess(
                    &samples,
                    format(native_rate, channels),
                    rate,
                    anchor,
                    &[13, 101, 307],
                );
                let expected = frames as u64 * u64::from(rate.hz()) / u64::from(native_rate);
                assert_eq!(
                    summary.frames(),
                    expected,
                    "rate {native_rate} channels {channels}"
                );
                assert_eq!(
                    output.iter().map(|block| block.frames as u64).sum::<u64>(),
                    expected
                );
                assert!(
                    output
                        .iter()
                        .all(|block| block.frames <= rate.block_frames())
                );
                assert!(
                    summary.last_end_us()
                        <= frame_time(anchor, frames as u64, native_rate).unwrap()
                );
            }
        }
    }
}

#[test]
fn resampling_is_bit_identical_across_arbitrary_input_block_boundaries() {
    let samples = signal(4_097, 2);
    let native = format(44_100, 2);
    for rate in [PreparedSampleRate::Hz16000, PreparedSampleRate::Hz24000] {
        let (_, whole) = preprocess(&samples, native, rate, 987_654, &[4_097]);
        for sizes in [&[1][..], &[7, 31, 613][..], &[883, 2, 101][..]] {
            let (_, split) = preprocess(&samples, native, rate, 987_654, sizes);
            assert_eq!(whole, split);
        }
    }
}

#[test]
fn eof_flushes_filter_lookahead_and_short_final_output_once() {
    let samples = signal(997, 1);
    let (summary, output) = preprocess(
        &samples,
        format(48_000, 1),
        PreparedSampleRate::Hz16000,
        555_000,
        &[997],
    );
    assert_eq!(summary.frames(), 332);
    assert_eq!(
        output.iter().map(|block| block.frames).collect::<Vec<_>>(),
        [320, 12]
    );
    assert_eq!(summary.last_end_us(), 555_000 + 332 * 1_000_000 / 16_000);
}

#[test]
fn zero_input_is_invalid_and_finish_is_terminal() {
    let cancel = ResolverCancellation::default();
    let mut processor = processor(format(16_000, 1), PreparedSampleRate::Hz16000, &cancel);
    assert_eq!(
        processor.finish(&cancel, &mut discard).err(),
        Some(MediaError::InvalidAudio)
    );
    assert!(processor.terminal);
    assert!(processor.run.is_none());
    let block = input(1, 0, format(16_000, 1), vec![0; 320]);
    assert_eq!(
        processor.push(&block, &cancel, &mut discard),
        Err(MediaError::InvalidAudio)
    );
    assert_eq!(
        processor.finish(&cancel, &mut discard).err(),
        Some(MediaError::InvalidAudio)
    );
}

#[test]
fn valid_input_shorter_than_one_output_frame_is_explicitly_insufficient() {
    let cancel = ResolverCancellation::default();
    let native = format(192_000, 1);
    let mut processor = processor(native, PreparedSampleRate::Hz16000, &cancel);
    processor
        .push(&input(1, 100_000, native, vec![123]), &cancel, &mut discard)
        .unwrap();
    assert_eq!(
        processor.finish(&cancel, &mut discard).err(),
        Some(MediaError::InsufficientAudio)
    );
    assert!(processor.terminal);
    assert!(processor.run.is_none());
}

#[test]
fn true_gap_flushes_old_tail_and_resets_filter_without_padding_or_cross_bleed() {
    let native = format(48_000, 1);
    let rate = PreparedSampleRate::Hz16000;
    let first = vec![20_000; 301];
    let second = vec![-20_000; 305];
    let first_anchor = 10_000;
    let second_anchor = 910_000;
    let (_, first_alone) = preprocess(&first, native, rate, first_anchor, &[301]);
    let (_, second_alone) = preprocess(&second, native, rate, second_anchor, &[305]);
    let cancel = ResolverCancellation::default();
    let mut processor = processor(native, rate, &cancel);
    let mut output = Vec::new();
    let mut emit = |block: &PreparedPcmBlock, _: &ResolverCancellation| {
        record(&mut output, block);
        Ok(())
    };
    processor
        .push(&input(1, first_anchor, native, first), &cancel, &mut emit)
        .unwrap();
    processor
        .push(&input(2, second_anchor, native, second), &cancel, &mut emit)
        .unwrap();
    let summary = processor.finish(&cancel, &mut emit).unwrap();
    assert_eq!(summary.gaps(), 1);
    assert_eq!(summary.frames(), 100 + 101);
    assert_eq!(output.len(), 2);
    assert_eq!(output[0].samples, first_alone[0].samples);
    assert_eq!(output[1].samples, second_alone[0].samples);
    assert!(!output[0].discontinuity);
    assert!(output[1].discontinuity);
    assert_eq!(output[1].sequence, 2);
    assert_eq!(output[1].timestamp_us, second_anchor);
}

#[test]
fn only_first_output_block_after_gap_marks_discontinuity() {
    let cancel = ResolverCancellation::default();
    let native = format(16_000, 1);
    let mut processor = processor(native, PreparedSampleRate::Hz16000, &cancel);
    let mut observed = Vec::new();
    let mut emit = |block: &PreparedPcmBlock, _: &ResolverCancellation| {
        observed.push((block.sequence(), block.discontinuity()));
        Ok(())
    };
    processor
        .push(&input(1, 0, native, vec![1; 160]), &cancel, &mut emit)
        .unwrap();
    processor
        .push(&input(2, 100_000, native, vec![2; 700]), &cancel, &mut emit)
        .unwrap();
    processor.finish(&cancel, &mut emit).unwrap();
    assert_eq!(observed, [(1, false), (2, true), (3, false), (4, false)]);
}

#[test]
fn isolated_timestamp_rounding_is_accepted_but_cumulative_drift_is_rejected() {
    let cancel = ResolverCancellation::default();
    let native = format(16_000, 1);
    let mut rounded = processor(native, PreparedSampleRate::Hz16000, &cancel);
    for (sequence, timestamp) in [(1, 0), (2, 9_999), (3, 20_000)] {
        rounded
            .push(
                &input(sequence, timestamp, native, vec![0; 160]),
                &cancel,
                &mut discard,
            )
            .unwrap();
    }
    assert_eq!(rounded.finish(&cancel, &mut discard).unwrap().frames(), 480);
    let mut drifted = processor(native, PreparedSampleRate::Hz16000, &cancel);
    for (sequence, timestamp) in [(1, 0), (2, 10_001)] {
        drifted
            .push(
                &input(sequence, timestamp, native, vec![0; 160]),
                &cancel,
                &mut discard,
            )
            .unwrap();
    }
    assert_eq!(
        drifted.push(
            &input(3, 20_002, native, vec![0; 160]),
            &cancel,
            &mut discard
        ),
        Err(MediaError::InvalidAudio)
    );
    assert!(drifted.terminal);
    assert!(drifted.run.is_none());
}

#[test]
fn wrong_session_generation_sequence_format_and_overlap_are_terminal() {
    for case in 0..6 {
        let cancel = ResolverCancellation::default();
        let native = format(16_000, 1);
        let mut processor = processor(native, PreparedSampleRate::Hz16000, &cancel);
        processor
            .push(&input(1, 0, native, vec![0; 160]), &cancel, &mut discard)
            .unwrap();
        let mut id = identity();
        let mut sequence = 2;
        let mut timestamp = 10_000;
        let mut next_format = native;
        let expected = match case {
            0 => {
                id.session = SessionId(82);
                MediaError::Stale
            }
            1 => {
                id.generation += 1;
                MediaError::Stale
            }
            2 => {
                sequence = 1;
                MediaError::InvalidAudio
            }
            3 => {
                sequence = 3;
                MediaError::InvalidAudio
            }
            4 => {
                next_format = format(24_000, 1);
                MediaError::InvalidAudio
            }
            _ => {
                timestamp = 9_998;
                MediaError::InvalidAudio
            }
        };
        let block =
            DecodedAudioBlock::new(id, sequence, timestamp, next_format, vec![0; 160]).unwrap();
        assert_eq!(processor.push(&block, &cancel, &mut discard), Err(expected));
        assert!(processor.terminal);
        assert!(processor.run.is_none());
        assert_eq!(
            processor.finish(&cancel, &mut discard).err(),
            Some(MediaError::InvalidAudio)
        );
    }
}

#[test]
fn pre_cancel_and_invalid_identity_do_not_create_processor() {
    let cancel = ResolverCancellation::default();
    cancel.cancel();
    assert_eq!(
        PreparedAudioProcessor::new(
            identity(),
            format(16_000, 1),
            PreparedSampleRate::Hz16000,
            &cancel
        )
        .err(),
        Some(MediaError::Cancelled)
    );
    for id in [
        StreamIdentity {
            session: SessionId(0),
            generation: 1,
        },
        StreamIdentity {
            session: SessionId(1),
            generation: 0,
        },
    ] {
        assert_eq!(
            PreparedAudioProcessor::new(
                id,
                format(16_000, 1),
                PreparedSampleRate::Hz16000,
                &ResolverCancellation::default()
            )
            .err(),
            Some(MediaError::InvalidAudio)
        );
    }
}

#[test]
fn either_cancellation_scope_clears_pending_and_prevents_eof_output() {
    for own in [false, true] {
        let owned = ResolverCancellation::default();
        let supplied = ResolverCancellation::default();
        let native = format(48_000, 1);
        let mut processor = processor(native, PreparedSampleRate::Hz16000, &owned);
        processor
            .push(&input(1, 0, native, vec![1; 100]), &supplied, &mut discard)
            .unwrap();
        if own {
            owned.cancel();
        } else {
            supplied.cancel();
        }
        let mut calls = 0;
        assert_eq!(
            processor
                .finish(&supplied, &mut |_, _| {
                    calls += 1;
                    Ok(())
                })
                .err(),
            Some(MediaError::Cancelled)
        );
        assert_eq!(calls, 0);
        assert!(processor.run.is_none());
        assert!(processor.terminal);
    }
}

#[test]
fn cancellation_from_sink_is_checked_before_more_pcm_is_delivered() {
    let cancel = ResolverCancellation::default();
    let native = format(16_000, 1);
    let mut processor = processor(native, PreparedSampleRate::Hz16000, &cancel);
    let mut calls = 0;
    let result = processor.push(
        &input(1, 0, native, vec![123; 1_000]),
        &cancel,
        &mut |_, scope| {
            calls += 1;
            scope.cancel();
            Ok(())
        },
    );
    assert_eq!(result, Err(MediaError::Cancelled));
    assert_eq!(calls, 1);
    assert!(processor.run.is_none());
    assert!(processor.terminal);
}

#[test]
fn typed_sink_error_is_preserved_and_no_more_chunks_or_summary_follow() {
    let cancel = ResolverCancellation::default();
    let native = format(16_000, 1);
    let mut processor = processor(native, PreparedSampleRate::Hz16000, &cancel);
    let mut calls = 0;
    assert_eq!(
        processor.push(
            &input(1, 0, native, vec![0; 1_000]),
            &cancel,
            &mut |_, _| {
                calls += 1;
                Err(MediaError::Unavailable)
            }
        ),
        Err(MediaError::Unavailable)
    );
    assert_eq!(calls, 1);
    assert!(processor.run.is_none());
    assert_eq!(
        processor.finish(&cancel, &mut discard).err(),
        Some(MediaError::InvalidAudio)
    );
}

#[test]
fn sink_error_during_eof_tail_is_terminal_and_not_a_partial_summary() {
    let cancel = ResolverCancellation::default();
    let native = format(48_000, 1);
    let mut processor = processor(native, PreparedSampleRate::Hz16000, &cancel);
    processor
        .push(&input(1, 0, native, vec![123; 100]), &cancel, &mut discard)
        .unwrap();
    let mut calls = 0;
    assert_eq!(
        processor
            .finish(&cancel, &mut |_, _| {
                calls += 1;
                Err(MediaError::Overloaded)
            })
            .err(),
        Some(MediaError::Overloaded)
    );
    assert_eq!(calls, 1);
    assert!(processor.run.is_none());
    assert!(processor.terminal);
}

#[test]
fn source_and_output_end_respect_twenty_four_hour_absolute_media_limit() {
    let native = format(16_000, 1);
    let end = MAX_DECODED_MEDIA_DURATION_US;
    let (_, output) = preprocess(
        &[123; 160],
        native,
        PreparedSampleRate::Hz16000,
        end - 10_000,
        &[160],
    );
    assert_eq!(output.last().unwrap().end_us, end);
    let cancel = ResolverCancellation::default();
    let mut processor = processor(native, PreparedSampleRate::Hz16000, &cancel);
    assert_eq!(
        processor.push(
            &input(1, end - 9_999, native, vec![0; 160]),
            &cancel,
            &mut discard
        ),
        Err(MediaError::TooLarge)
    );
    assert!(processor.run.is_none());
    assert!(processor.terminal);
}

#[test]
fn checked_output_sequence_and_totals_refuse_overflow_without_delivery() {
    let native = format(16_000, 1);
    for blocks in [true, false] {
        let cancel = ResolverCancellation::default();
        let mut processor = processor(native, PreparedSampleRate::Hz16000, &cancel);
        if blocks {
            processor.output.blocks = u64::MAX - 1;
        } else {
            processor.output.frames = u64::MAX;
        }
        let mut calls = 0;
        assert_eq!(
            processor.push(&input(1, 0, native, vec![0; 320]), &cancel, &mut |_, _| {
                calls += 1;
                Ok(())
            }),
            Err(MediaError::InvalidAudio)
        );
        assert_eq!(calls, 0);
        assert!(processor.run.is_none());
    }
}

#[test]
fn successful_finish_releases_dsp_and_cannot_be_reused() {
    let cancel = ResolverCancellation::default();
    let native = format(16_000, 1);
    let mut processor = processor(native, PreparedSampleRate::Hz16000, &cancel);
    processor
        .push(&input(1, 0, native, vec![0; 320]), &cancel, &mut discard)
        .unwrap();
    assert_eq!(
        processor.finish(&cancel, &mut discard).unwrap().frames(),
        320
    );
    assert!(processor.run.is_none());
    assert!(processor.terminal);
    assert!(!cancel.is_cancelled());
    assert_eq!(
        processor.push(
            &input(2, 20_000, native, vec![0; 320]),
            &cancel,
            &mut discard
        ),
        Err(MediaError::InvalidAudio)
    );
    assert_eq!(
        processor.finish(&cancel, &mut discard).err(),
        Some(MediaError::InvalidAudio)
    );
}
