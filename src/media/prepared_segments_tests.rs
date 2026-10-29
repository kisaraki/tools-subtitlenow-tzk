//! Explicit synthetic test audio is RAM-only and never comes from a device.
use super::*;
use crate::{audio::capture::SessionId, media::decoded::MAX_DECODED_BLOCK_SAMPLES};

fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(208),
        generation: 3,
    }
}
fn builder(rate: PreparedSampleRate) -> PreparedSegmentBuilder {
    PreparedSegmentBuilder::new(identity(), rate).unwrap()
}
fn sample(frame: usize) -> i16 {
    ((frame % 30_001) as i32 - 15_000) as i16
}
fn block(
    rate: PreparedSampleRate,
    sequence: u64,
    anchor: u64,
    offset: usize,
    frames: usize,
) -> DecodedAudioBlock {
    DecodedAudioBlock::new(
        identity(),
        sequence,
        frame_time(anchor, offset as u64, rate).unwrap(),
        DecodedAudioFormat::new(rate.hz(), 1).unwrap(),
        (offset..offset + frames).map(sample).collect(),
    )
    .unwrap()
}
fn push_frames(
    target: &mut PreparedSegmentBuilder,
    rate: PreparedSampleRate,
    anchor: u64,
    total_frames: usize,
    sizes: &[usize],
    mut take: impl FnMut(&mut PreparedSegmentBuilder),
) -> u64 {
    let cancel = ResolverCancellation::default();
    let mut offset = 0;
    let mut sequence = 1;
    while offset < total_frames {
        let frames = sizes[(sequence - 1) as usize % sizes.len()].min(total_frames - offset);
        target
            .push(
                block(rate, sequence, anchor, offset, frames),
                false,
                &cancel,
            )
            .unwrap();
        take(target);
        offset += frames;
        sequence += 1;
    }
    sequence
}

#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    stamp: PreparedChunkStamp,
    audio_end_us: u64,
    samples: Vec<i16>,
}
fn drain(target: &mut PreparedSegmentBuilder, output: &mut Vec<Snapshot>) {
    while let Some(chunk) = target.take() {
        output.push(Snapshot {
            stamp: chunk.stamp(),
            audio_end_us: chunk.audio_end_us(),
            samples: chunk.samples().to_vec(),
        });
    }
}
fn segment(rate: PreparedSampleRate, anchor: u64, frames: usize, sizes: &[usize]) -> Vec<Snapshot> {
    let cancel = ResolverCancellation::default();
    let mut target = builder(rate);
    let mut output = Vec::new();
    push_frames(&mut target, rate, anchor, frames, sizes, |target| {
        drain(target, &mut output)
    });
    target.finish(&cancel).unwrap();
    drain(&mut target, &mut output);
    assert!(target.run.is_none());
    assert!(target.ready.is_empty());
    output
}

#[test]
fn bodies_partition_and_exact_overlap_audio_is_real() {
    for rate in [PreparedSampleRate::Hz16000, PreparedSampleRate::Hz24000] {
        let hz = rate.hz() as usize;
        let anchor = 781_003;
        let frames = hz * 21 + 7;
        let chunks = segment(rate, anchor, frames, &[317, 997, 32000]);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].stamp.audio_start_us, anchor);
        assert_eq!(chunks[0].stamp.body_start_us, anchor);
        assert_eq!(chunks[0].stamp.body_end_us, anchor + 10_000_000);
        assert_eq!(chunks[0].audio_end_us, anchor + 10_500_000);
        assert_eq!(chunks[1].stamp.audio_start_us, anchor + 9_500_000);
        assert_eq!(chunks[1].stamp.body_start_us, anchor + 10_000_000);
        assert_eq!(chunks[1].stamp.body_end_us, anchor + 20_000_000);
        assert_eq!(chunks[1].audio_end_us, anchor + 20_500_000);
        assert_eq!(chunks[2].stamp.audio_start_us, anchor + 19_500_000);
        assert_eq!(chunks[2].stamp.body_start_us, anchor + 20_000_000);
        let real_end = frame_time(anchor, frames as u64, rate).unwrap();
        assert_eq!(chunks[2].stamp.body_end_us, real_end);
        assert_eq!(chunks[2].audio_end_us, real_end);
        let mut bodies = Vec::new();
        for (index, chunk) in chunks.iter().enumerate() {
            assert_eq!(chunk.stamp.identity, identity());
            assert_eq!(chunk.stamp.sequence, index as u64 + 1);
            assert_eq!(chunk.stamp.run_id, 1);
            assert!(!chunk.stamp.discontinuity);
            assert_eq!(chunk.stamp.rate, rate);
            let begin = if index == 0 {
                0
            } else {
                index * hz * 10 - hz / 2
            };
            let end = if index < 2 {
                (index + 1) * hz * 10 + hz / 2
            } else {
                frames
            };
            assert_eq!(chunk.samples, (begin..end).map(sample).collect::<Vec<_>>());
            let body_begin = index * hz * 10 - begin;
            let body_end = ((index + 1) * hz * 10).min(frames) - begin;
            bodies.extend_from_slice(&chunk.samples[body_begin..body_end]);
        }
        assert_eq!(bodies, (0..frames).map(sample).collect::<Vec<_>>());
    }
}

#[test]
fn different_input_partitions_are_bit_identical() {
    for rate in [PreparedSampleRate::Hz16000, PreparedSampleRate::Hz24000] {
        let frames = rate.hz() as usize * 30 + 73;
        let expected = segment(rate, 31_007, frames, &[MAX_DECODED_BLOCK_SAMPLES]);
        for sizes in [&[1_003][..], &[1, 19, 480, 16_000, 32_767][..]] {
            assert_eq!(expected, segment(rate, 31_007, frames, sizes));
        }
    }
}

#[test]
fn long_stream_is_demand_driven_and_retained_pcm_stays_bounded() {
    for rate in [PreparedSampleRate::Hz16000, PreparedSampleRate::Hz24000] {
        let mut target = builder(rate);
        let max = rate.hz() as usize * 11;
        let mut count = 0;
        let frames = rate.hz() as usize * 130 + 3;
        push_frames(&mut target, rate, 0, frames, &[4093], |target| {
            assert!(target.run.as_ref().unwrap().buffer.len() <= max);
            assert!(target.ready.len() <= MAX_PREPARED_READY_CHUNKS);
            while let Some(chunk) = target.take() {
                assert!(chunk.samples().len() <= max);
                count += 1;
            }
        });
        target.finish(&ResolverCancellation::default()).unwrap();
        while let Some(chunk) = target.take() {
            assert!(chunk.samples().len() <= max);
            count += 1;
        }
        assert_eq!(count, 14);
        assert!(target.run.is_none());
    }
}

#[test]
fn short_eof_has_no_fake_padding_or_left_context() {
    for rate in [PreparedSampleRate::Hz16000, PreparedSampleRate::Hz24000] {
        let chunks = segment(rate, 49, 7, &[7]);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].samples, (0..7).map(sample).collect::<Vec<_>>());
        assert_eq!(chunks[0].stamp.audio_start_us, 49);
        assert_eq!(chunks[0].stamp.body_start_us, 49);
        assert_eq!(
            chunks[0].stamp.body_end_us,
            frame_time(49, 7, rate).unwrap()
        );
        assert_eq!(chunks[0].audio_end_us, chunks[0].stamp.body_end_us);
    }
}

#[test]
fn exact_ten_seconds_is_one_body_and_no_overlap_only_tail() {
    for rate in [PreparedSampleRate::Hz16000, PreparedSampleRate::Hz24000] {
        let chunks = segment(rate, 0, rate.hz() as usize * 10, &[32_000]);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].stamp.body_end_us, 10_000_000);
        assert_eq!(chunks[0].audio_end_us, 10_000_000);
    }
}

#[test]
fn eof_ten_point_one_seconds_has_two_ready_chunks() {
    let rate = PreparedSampleRate::Hz24000;
    let cancel = ResolverCancellation::default();
    let mut target = builder(rate);
    push_frames(&mut target, rate, 0, 242_400, &[20_000], |_| {});
    assert!(target.ready.is_empty());
    target.finish(&cancel).unwrap();
    assert_eq!(target.ready.len(), 2);
    let first = target.take().unwrap();
    let tail = target.take().unwrap();
    assert_eq!(first.stamp().body_end_us, 10_000_000);
    assert_eq!(first.audio_end_us(), 10_100_000);
    assert_eq!(tail.stamp().audio_start_us, 9_500_000);
    assert_eq!(tail.stamp().body_start_us, 10_000_000);
    assert_eq!(tail.stamp().body_end_us, 10_100_000);
    assert_eq!(tail.samples().len(), 14_400);
    assert!(target.take().is_none());
}

#[test]
fn finish_with_one_ready_chunk_can_add_one_tail() {
    let rate = PreparedSampleRate::Hz16000;
    let cancel = ResolverCancellation::default();
    let mut target = builder(rate);
    push_frames(&mut target, rate, 0, 168_000, &[32_000], |_| {});
    assert_eq!(target.ready.len(), 1);
    target.finish(&cancel).unwrap();
    assert_eq!(target.ready.len(), 2);
    let full = target.take().unwrap();
    let tail = target.take().unwrap();
    assert_eq!(full.stamp().body_end_us, 10_000_000);
    assert_eq!(tail.stamp().body_start_us, 10_000_000);
    assert_eq!(tail.stamp().body_end_us, 10_500_000);
}

#[test]
fn gap_flushes_old_full_and_tail_then_keeps_new_run_separate() {
    let rate = PreparedSampleRate::Hz16000;
    let cancel = ResolverCancellation::default();
    let mut target = builder(rate);
    let next = push_frames(&mut target, rate, 100, 161_600, &[32_000], |_| {});
    target
        .push(block(rate, next, 20_000_100, 0, 1600), true, &cancel)
        .unwrap();
    assert_eq!(target.ready.len(), 2);
    let full = target.take().unwrap();
    let tail = target.take().unwrap();
    assert_eq!(full.stamp().run_id, 1);
    assert_eq!(tail.stamp().run_id, 1);
    assert_eq!(tail.stamp().body_end_us, 10_100_100);
    assert!(full.audio_end_us() <= 10_100_100);
    assert!(tail.audio_end_us() <= 10_100_100);
    assert_eq!(
        target.run.as_ref().unwrap().buffer,
        (0..1600).map(sample).collect::<Vec<_>>()
    );
    target.finish(&cancel).unwrap();
    let fresh = target.take().unwrap();
    assert_eq!(fresh.stamp().run_id, 2);
    assert_eq!(fresh.stamp().sequence, 3);
    assert!(fresh.stamp().discontinuity);
    assert_eq!(fresh.stamp().audio_start_us, 20_000_100);
    assert_eq!(fresh.stamp().body_start_us, 20_000_100);
    assert_eq!(fresh.samples(), (0..1600).map(sample).collect::<Vec<_>>());
}

#[test]
fn first_chunk_only_marks_each_gap_run_discontinuity() {
    let rate = PreparedSampleRate::Hz16000;
    let cancel = ResolverCancellation::default();
    let mut target = builder(rate);
    target
        .push(block(rate, 1, 0, 0, 1600), false, &cancel)
        .unwrap();
    target
        .push(block(rate, 2, 1_000_000, 0, 1600), true, &cancel)
        .unwrap();
    let old = target.take().unwrap();
    assert!(!old.stamp().discontinuity);
    let mut sequence = 3;
    let mut offset = 1600;
    while offset < 21 * 16000 {
        let count = 1600;
        target
            .push(
                block(rate, sequence, 1_000_000, offset, count),
                false,
                &cancel,
            )
            .unwrap();
        if let Some(chunk) = target.take() {
            assert_eq!(chunk.stamp().run_id, 2);
            assert_eq!(chunk.stamp().discontinuity, chunk.stamp().sequence == 2);
        }
        sequence += 1;
        offset += count;
    }
    target.finish(&cancel).unwrap();
    while let Some(chunk) = target.take() {
        assert_eq!(chunk.stamp().run_id, 2);
        assert!(!chunk.stamp().discontinuity);
    }
}

#[test]
fn ready_backpressure_is_terminal_and_does_not_drop_work_silently() {
    let rate = PreparedSampleRate::Hz16000;
    let cancel = ResolverCancellation::default();
    let mut target = builder(rate);
    let next = push_frames(&mut target, rate, 0, 168_000, &[32_000], |_| {});
    assert_eq!(target.ready.len(), 1);
    assert_eq!(
        target.push(block(rate, next, 0, 168_000, 1), false, &cancel),
        Err(PreparedSpeechError::Overloaded)
    );
    assert!(target.run.is_none());
    assert!(target.take().is_none());
    assert_eq!(target.finish(&cancel), Err(PreparedSpeechError::Overloaded));
}

#[test]
fn finish_that_would_exceed_two_ready_chunks_rejects_and_clears() {
    let rate = PreparedSampleRate::Hz16000;
    let cancel = ResolverCancellation::default();
    let mut target = builder(rate);
    let next = push_frames(&mut target, rate, 0, 161_600, &[32_000], |_| {});
    target
        .push(block(rate, next, 20_000_000, 0, 1600), true, &cancel)
        .unwrap();
    assert_eq!(target.ready.len(), 2);
    assert_eq!(target.finish(&cancel), Err(PreparedSpeechError::Overloaded));
    assert!(target.run.is_none());
    assert!(target.ready.is_empty());
    assert!(target.take().is_none());
}

#[test]
fn cancellation_before_input_produces_no_retained_pcm() {
    let rate = PreparedSampleRate::Hz16000;
    let cancel = ResolverCancellation::default();
    cancel.cancel();
    let mut target = builder(rate);
    assert_eq!(
        target.push(block(rate, 1, 0, 0, 10), false, &cancel),
        Err(PreparedSpeechError::Cancelled)
    );
    assert!(target.run.is_none());
    assert!(target.ready.is_empty());
}

#[test]
fn cancellation_after_ready_discards_chunk_before_take() {
    let rate = PreparedSampleRate::Hz16000;
    let cancel = ResolverCancellation::default();
    let mut target = builder(rate);
    let mut offset = 0;
    let mut sequence = 1;
    while offset < 168_000 {
        let count = 32_000.min(168_000 - offset);
        target
            .push(block(rate, sequence, 0, offset, count), false, &cancel)
            .unwrap();
        sequence += 1;
        offset += count;
    }
    assert_eq!(target.ready.len(), 1);
    cancel.cancel();
    assert!(target.take().is_none());
    assert!(target.run.is_none());
    assert!(target.ready.is_empty());
    assert_eq!(
        target.finish(&ResolverCancellation::default()),
        Err(PreparedSpeechError::Cancelled)
    );
}

#[test]
fn cancellation_after_finished_prevents_take() {
    let rate = PreparedSampleRate::Hz24000;
    let cancel = ResolverCancellation::default();
    let mut target = builder(rate);
    target
        .push(block(rate, 1, 0, 0, 100), false, &cancel)
        .unwrap();
    target.finish(&cancel).unwrap();
    cancel.cancel();
    assert!(target.take().is_none());
    assert!(target.ready.is_empty());
}

#[test]
fn first_token_cannot_be_replaced_to_escape_cancellation() {
    let rate = PreparedSampleRate::Hz16000;
    let first = ResolverCancellation::default();
    let mut target = builder(rate);
    target.push(block(rate, 1, 0, 0, 1), false, &first).unwrap();
    first.cancel();
    assert_eq!(
        target.push(
            block(rate, 2, 0, 1, 1),
            false,
            &ResolverCancellation::default()
        ),
        Err(PreparedSpeechError::Cancelled)
    );
    assert!(target.run.is_none());
}

#[test]
fn push_after_successful_finish_is_rejected() {
    let rate = PreparedSampleRate::Hz16000;
    let cancel = ResolverCancellation::default();
    let mut target = builder(rate);
    target
        .push(block(rate, 1, 0, 0, 10), false, &cancel)
        .unwrap();
    target.finish(&cancel).unwrap();
    assert!(target.take().is_some());
    assert_eq!(
        target.push(block(rate, 2, 0, 10, 10), false, &cancel),
        Err(PreparedSpeechError::InvalidData)
    );
    assert!(target.take().is_none());
}

#[test]
fn empty_eof_is_explicit_error() {
    let mut target = builder(PreparedSampleRate::Hz16000);
    assert_eq!(
        target.finish(&ResolverCancellation::default()),
        Err(PreparedSpeechError::InvalidData)
    );
    assert!(target.take().is_none());
}

#[test]
fn invalid_identity_is_rejected_before_any_audio_allocation() {
    for identity in [
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
            PreparedSegmentBuilder::new(identity, PreparedSampleRate::Hz16000).err(),
            Some(PreparedSpeechError::InvalidData)
        );
    }
}

#[test]
fn old_session_and_generation_are_rejected() {
    let rate = PreparedSampleRate::Hz16000;
    let cancel = ResolverCancellation::default();
    for other in [
        StreamIdentity {
            session: SessionId(209),
            generation: 3,
        },
        StreamIdentity {
            session: SessionId(208),
            generation: 4,
        },
    ] {
        let mut target = builder(rate);
        let stale = DecodedAudioBlock::new(
            other,
            1,
            0,
            DecodedAudioFormat::new(rate.hz(), 1).unwrap(),
            vec![1; 10],
        )
        .unwrap();
        assert_eq!(
            target.push(stale, false, &cancel),
            Err(PreparedSpeechError::Stale)
        );
        assert!(target.run.is_none());
    }
}

#[test]
fn mismatched_rate_or_stereo_audio_is_rejected() {
    let cancel = ResolverCancellation::default();
    for format in [
        DecodedAudioFormat::new(24_000, 1).unwrap(),
        DecodedAudioFormat::new(16_000, 2).unwrap(),
    ] {
        let mut target = builder(PreparedSampleRate::Hz16000);
        let wrong = DecodedAudioBlock::new(identity(), 1, 0, format, vec![1; 10]).unwrap();
        assert_eq!(
            target.push(wrong, false, &cancel),
            Err(PreparedSpeechError::InvalidData)
        );
        assert!(target.run.is_none());
    }
}

#[test]
fn missing_or_out_of_order_sequence_is_rejected() {
    let rate = PreparedSampleRate::Hz16000;
    let cancel = ResolverCancellation::default();
    for sequence in [1, 3] {
        let mut target = builder(rate);
        target
            .push(block(rate, 1, 0, 0, 10), false, &cancel)
            .unwrap();
        assert_eq!(
            target.push(block(rate, sequence, 0, 10, 10), false, &cancel),
            Err(PreparedSpeechError::InvalidData)
        );
        assert!(target.run.is_none());
    }
}

#[test]
fn first_and_continuous_blocks_cannot_claim_false_gap() {
    let rate = PreparedSampleRate::Hz16000;
    let cancel = ResolverCancellation::default();
    let mut first = builder(rate);
    assert_eq!(
        first.push(block(rate, 1, 0, 0, 10), true, &cancel),
        Err(PreparedSpeechError::BoundaryMismatch)
    );
    let mut continuous = builder(rate);
    continuous
        .push(block(rate, 1, 0, 0, 10), false, &cancel)
        .unwrap();
    assert_eq!(
        continuous.push(block(rate, 2, 0, 10, 10), true, &cancel),
        Err(PreparedSpeechError::BoundaryMismatch)
    );
    assert!(continuous.run.is_none());
}

#[test]
fn real_forward_gap_requires_discontinuity_flag() {
    let rate = PreparedSampleRate::Hz16000;
    let cancel = ResolverCancellation::default();
    let mut target = builder(rate);
    target
        .push(block(rate, 1, 0, 0, 10), false, &cancel)
        .unwrap();
    assert_eq!(
        target.push(block(rate, 2, 1_000_000, 0, 10), false, &cancel),
        Err(PreparedSpeechError::BoundaryMismatch)
    );
    assert!(target.run.is_none());
}

#[test]
fn one_microsecond_rounding_is_allowed_without_drift() {
    let rate = PreparedSampleRate::Hz24000;
    let cancel = ResolverCancellation::default();
    let mut target = builder(rate);
    target
        .push(block(rate, 1, 0, 0, 1), false, &cancel)
        .unwrap();
    // floor(1e6/24000)=41; one microsecond of representation rounding is allowed.
    let rounded = DecodedAudioBlock::new(
        identity(),
        2,
        42,
        DecodedAudioFormat::new(24_000, 1).unwrap(),
        vec![2],
    )
    .unwrap();
    target.push(rounded, false, &cancel).unwrap();
    target
        .push(block(rate, 3, 0, 2, 1), false, &cancel)
        .unwrap();
    target.finish(&cancel).unwrap();
    let chunk = target.take().unwrap();
    assert_eq!(chunk.samples(), &[sample(0), 2, sample(2)]);
    assert_eq!(chunk.stamp().body_end_us, 125);
}

#[test]
fn per_block_rounding_cannot_accumulate_clock_drift() {
    let rate = PreparedSampleRate::Hz24000;
    let cancel = ResolverCancellation::default();
    let mut target = builder(rate);
    // A one-frame 24k block lasts 41us. Repeatedly adding only that floored
    // duration stays gate-contiguous but eventually differs from the run clock.
    for index in 0..3 {
        let input = DecodedAudioBlock::new(
            identity(),
            index + 1,
            index * 41,
            DecodedAudioFormat::new(24_000, 1).unwrap(),
            vec![1],
        )
        .unwrap();
        target.push(input, false, &cancel).unwrap();
    }
    let drifted = DecodedAudioBlock::new(
        identity(),
        4,
        123,
        DecodedAudioFormat::new(24_000, 1).unwrap(),
        vec![1],
    )
    .unwrap();
    assert_eq!(
        target.push(drifted, false, &cancel),
        Err(PreparedSpeechError::BoundaryMismatch)
    );
    assert!(target.run.is_none());
}

#[test]
fn backward_or_overlapping_time_is_rejected() {
    let rate = PreparedSampleRate::Hz16000;
    let cancel = ResolverCancellation::default();
    for time in [0, 50, 500] {
        let mut target = builder(rate);
        target
            .push(block(rate, 1, 100, 0, 10), false, &cancel)
            .unwrap();
        let bad = DecodedAudioBlock::new(
            identity(),
            2,
            time,
            DecodedAudioFormat::new(16_000, 1).unwrap(),
            vec![1; 10],
        )
        .unwrap();
        assert_eq!(
            target.push(bad, false, &cancel),
            Err(PreparedSpeechError::InvalidData)
        );
        assert!(target.run.is_none());
    }
}

#[test]
fn absolute_twenty_four_hour_bound_is_enforced() {
    let rate = PreparedSampleRate::Hz16000;
    let cancel = ResolverCancellation::default();
    let mut target = builder(rate);
    assert_eq!(
        target.push(
            block(rate, 1, MAX_DECODED_MEDIA_DURATION_US, 0, 10),
            false,
            &cancel
        ),
        Err(PreparedSpeechError::InvalidData)
    );
    assert!(target.run.is_none());
}

#[test]
fn real_eof_can_end_exactly_at_absolute_duration_bound() {
    let rate = PreparedSampleRate::Hz16000;
    let cancel = ResolverCancellation::default();
    let mut target = builder(rate);
    target
        .push(
            block(rate, 1, MAX_DECODED_MEDIA_DURATION_US - 625, 0, 10),
            false,
            &cancel,
        )
        .unwrap();
    target.finish(&cancel).unwrap();
    assert_eq!(
        target.take().unwrap().audio_end_us(),
        MAX_DECODED_MEDIA_DURATION_US
    );
}

#[test]
fn sequence_and_run_identifier_overflow_fail_without_wraparound() {
    let rate = PreparedSampleRate::Hz16000;
    let cancel = ResolverCancellation::default();
    let mut sequence = builder(rate);
    sequence.next_chunk_sequence = u64::MAX;
    sequence
        .push(block(rate, 1, 0, 0, 1), false, &cancel)
        .unwrap();
    assert_eq!(
        sequence.finish(&cancel),
        Err(PreparedSpeechError::InvalidData)
    );
    assert!(sequence.run.is_none());
    assert!(sequence.take().is_none());
    let mut run = builder(rate);
    run.next_run_id = u64::MAX;
    assert_eq!(
        run.push(block(rate, 1, 0, 0, 1), false, &cancel),
        Err(PreparedSpeechError::InvalidData)
    );
    assert!(run.run.is_none());
}

#[test]
fn frame_time_and_frame_count_overflow_are_checked() {
    assert_eq!(
        frame_time(u64::MAX, 1, PreparedSampleRate::Hz16000),
        Err(PreparedSpeechError::InvalidData)
    );
    assert_eq!(
        frame_time(0, u64::MAX, PreparedSampleRate::Hz16000),
        Err(PreparedSpeechError::InvalidData)
    );
    let rate = PreparedSampleRate::Hz16000;
    let cancel = ResolverCancellation::default();
    let mut target = builder(rate);
    target
        .push(block(rate, 1, 0, 0, 1), false, &cancel)
        .unwrap();
    target.run.as_mut().unwrap().frames = u64::MAX;
    assert_eq!(
        target.push(block(rate, 2, 0, 1, 1), false, &cancel),
        Err(PreparedSpeechError::InvalidData)
    );
    assert!(target.run.is_none());
}

#[test]
fn repeated_finish_is_terminal_error_and_clears_untaken_audio() {
    let rate = PreparedSampleRate::Hz16000;
    let cancel = ResolverCancellation::default();
    let mut target = builder(rate);
    target
        .push(block(rate, 1, 0, 0, 1), false, &cancel)
        .unwrap();
    target.finish(&cancel).unwrap();
    assert_eq!(
        target.finish(&cancel),
        Err(PreparedSpeechError::InvalidData)
    );
    assert!(target.ready.is_empty());
    assert!(target.take().is_none());
}
