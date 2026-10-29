//! Synthetic bounded PCM and timing metadata only: no file, device or provider.
use super::*;
use crate::audio::capture::SessionId;

fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(73),
        generation: 2,
    }
}

fn format(rate: u32, channels: u16) -> DecodedAudioFormat {
    DecodedAudioFormat::new(rate, channels).unwrap()
}

fn block_for(
    id: StreamIdentity,
    sequence: u64,
    native_start: u64,
    pcm_format: DecodedAudioFormat,
    frames: usize,
) -> DecodedAudioBlock {
    DecodedAudioBlock::new(
        id,
        sequence,
        native_start,
        pcm_format,
        vec![17; frames * usize::from(pcm_format.channels())],
    )
    .unwrap()
}

fn block(sequence: u64, native_start: u64) -> DecodedAudioBlock {
    block_for(identity(), sequence, native_start, format(16_000, 1), 160)
}

fn spans_at(starts: &[u64]) -> Vec<DecodedClockSpan> {
    let cancel = ResolverCancellation::default();
    let mut normalizer =
        DecodedClockNormalizer::new(identity(), format(16_000, 1), &cancel).unwrap();
    starts
        .iter()
        .enumerate()
        .map(|(index, start)| {
            normalizer
                .normalize(block(index as u64 + 1, *start), &cancel)
                .unwrap()
                .span()
        })
        .collect()
}

fn window_with(spans: &[DecodedClockSpan]) -> DecodedClockWindow {
    let cancel = ResolverCancellation::default();
    let mut window = DecodedClockWindow::new(identity(), &cancel).unwrap();
    for span in spans {
        window.append(*span, &cancel).unwrap();
    }
    window
}

fn assert_window_failed(window: &mut DecodedClockWindow, span: DecodedClockSpan) {
    assert!(window.terminal);
    assert!(window.spans.is_empty());
    assert!(window.previous.is_none());
    assert_eq!(
        window.append(span, &ResolverCancellation::default()),
        Err(MediaError::InvalidAudio)
    );
}

#[test]
fn normalization_moves_the_same_pcm_allocation_and_keeps_native_evidence() {
    let cancel = ResolverCancellation::default();
    let stereo = format(44_100, 2);
    let samples: Vec<i16> = (0..4608).map(|index| (index as i16) - 2304).collect();
    let expected = samples.clone();
    let input = DecodedAudioBlock::new(identity(), 1, 700_000, stereo, samples).unwrap();
    let original_pointer = input.samples().as_ptr();
    let original_end = input.end_us();
    let mut normalizer = DecodedClockNormalizer::new(identity(), stereo, &cancel).unwrap();
    let mapped = normalizer.normalize(input, &cancel).unwrap();
    let span = mapped.span();
    assert_eq!(span.identity(), identity());
    assert_eq!(span.sequence(), 1);
    assert_eq!(span.run_id(), 1);
    assert_eq!(span.format(), stereo);
    assert_eq!(span.native_start_us(), 700_000);
    assert_eq!(span.native_end_us(), original_end);
    assert!(!span.discontinuity());
    let (pcm, detached_span) = mapped.into_parts();
    assert_eq!(detached_span, span);
    assert_eq!(pcm.samples().as_ptr(), original_pointer);
    assert_eq!(pcm.samples(), expected);
    assert_eq!(pcm.frames(), 2304);
    assert_eq!(pcm.identity(), identity());
    assert_eq!(pcm.sequence(), 1);
    assert_eq!(pcm.format(), stereo);
    assert_eq!(pcm.timestamp_us(), span.sample_start_us());
}

#[test]
fn cumulative_44100_clock_and_tiny_gap_do_not_reset_away_packet_drift() {
    const PACKETS: u64 = 2048;
    const FRAMES: u64 = 1152;
    let pcm_format = format(44_100, 1);
    let cancel = ResolverCancellation::default();
    let mut normalizer = DecodedClockNormalizer::new(identity(), pcm_format, &cancel).unwrap();
    // Independent MPEG packet-clock oracle: each packet duration is floored in
    // 100 ns units before the accumulated native clock is converted to µs.
    let native_packet_ticks = FRAMES * 10_000_000 / 44_100;
    let mut previous = None;
    for packet in 0..PACKETS {
        let input = block_for(
            identity(),
            packet + 1,
            packet * native_packet_ticks / 10,
            pcm_format,
            FRAMES as usize,
        );
        let native_start = input.timestamp_us();
        let native_end = input.end_us();
        let mapped = normalizer.normalize(input, &cancel).unwrap();
        let span = mapped.span();
        assert_eq!(span.sample_start_us(), packet * FRAMES * 1_000_000 / 44_100);
        assert_eq!(
            span.sample_end_us(),
            (packet + 1) * FRAMES * 1_000_000 / 44_100
        );
        assert_eq!(span.native_start_us(), native_start);
        assert_eq!(span.native_end_us(), native_end);
        assert_eq!(span.run_id(), 1);
        assert!(!span.discontinuity());
        assert!(
            mapped
                .sample_clock_block()
                .end_us()
                .abs_diff(span.sample_end_us())
                <= 1
        );
        previous = Some(span);
    }
    let previous = previous.unwrap();
    assert!(previous.sample_end_us() - previous.native_end_us() > 50);
    let next = normalizer
        .normalize(
            block_for(
                identity(),
                PACKETS + 1,
                previous.native_end_us() + 2,
                pcm_format,
                1152,
            ),
            &cancel,
        )
        .unwrap()
        .span();
    assert_eq!(next.run_id(), 2);
    assert!(next.discontinuity());
    assert_eq!(next.sample_start_us(), previous.sample_end_us() + 2);
    assert_eq!(next.native_start_us(), previous.native_end_us() + 2);
}

#[test]
fn one_microsecond_native_gap_or_overlap_remains_the_same_run() {
    for native_start in [109_999, 110_000, 110_001] {
        let spans = spans_at(&[100_000, native_start]);
        assert_eq!(spans[1].run_id(), 1);
        assert!(!spans[1].discontinuity());
        assert_eq!(spans[1].sample_start_us(), spans[0].sample_end_us());
        assert_eq!(spans[1].native_start_us(), native_start);
    }
}

#[test]
fn true_native_gap_retains_its_measured_length_and_new_run() {
    let spans = spans_at(&[100_000, 126_500]);
    assert_eq!(spans[1].run_id(), 2);
    assert!(spans[1].discontinuity());
    assert_eq!(
        spans[1].sample_start_us() - spans[0].sample_end_us(),
        16_500
    );
    assert_eq!(
        spans[1].native_start_us() - spans[0].native_end_us(),
        16_500
    );
}

#[test]
fn larger_native_overlap_and_rewind_are_terminal() {
    for invalid_start in [109_998, 99_999] {
        let cancel = ResolverCancellation::default();
        let mut normalizer =
            DecodedClockNormalizer::new(identity(), format(16_000, 1), &cancel).unwrap();
        normalizer.normalize(block(1, 100_000), &cancel).unwrap();
        assert_eq!(
            normalizer.normalize(block(2, invalid_start), &cancel).err(),
            Some(MediaError::InvalidAudio)
        );
        assert!(normalizer.terminal);
        assert!(normalizer.previous.is_none());
        assert_eq!(
            normalizer.normalize(block(2, 110_000), &cancel).err(),
            Some(MediaError::InvalidAudio)
        );
    }
}

#[test]
fn stale_session_and_generation_are_each_rejected_and_clear_normalizer() {
    for stale in [
        StreamIdentity {
            session: SessionId(74),
            ..identity()
        },
        StreamIdentity {
            generation: 3,
            ..identity()
        },
    ] {
        let cancel = ResolverCancellation::default();
        let mut normalizer =
            DecodedClockNormalizer::new(identity(), format(16_000, 1), &cancel).unwrap();
        normalizer.normalize(block(1, 0), &cancel).unwrap();
        assert_eq!(
            normalizer
                .normalize(block_for(stale, 2, 10_000, format(16_000, 1), 160), &cancel)
                .err(),
            Some(MediaError::Stale)
        );
        assert!(normalizer.previous.is_none());
        assert!(normalizer.terminal);
    }
}

#[test]
fn sequence_duplicate_skip_and_format_change_are_rejected() {
    for (sequence, pcm_format) in [
        (1, format(16_000, 1)),
        (3, format(16_000, 1)),
        (2, format(24_000, 1)),
        (2, format(16_000, 2)),
    ] {
        let cancel = ResolverCancellation::default();
        let mut normalizer =
            DecodedClockNormalizer::new(identity(), format(16_000, 1), &cancel).unwrap();
        normalizer.normalize(block(1, 0), &cancel).unwrap();
        assert_eq!(
            normalizer
                .normalize(
                    block_for(identity(), sequence, 10_000, pcm_format, 160),
                    &cancel
                )
                .err(),
            Some(MediaError::InvalidAudio)
        );
        assert!(normalizer.previous.is_none());
        assert!(normalizer.terminal);
    }
}

#[test]
fn constructors_reject_zero_identity_and_pre_cancelled_scope() {
    for invalid in [
        StreamIdentity {
            session: SessionId(0),
            ..identity()
        },
        StreamIdentity {
            generation: 0,
            ..identity()
        },
    ] {
        let cancel = ResolverCancellation::default();
        assert_eq!(
            DecodedClockNormalizer::new(invalid, format(16_000, 1), &cancel).err(),
            Some(MediaError::InvalidAudio)
        );
        assert_eq!(
            DecodedClockWindow::new(invalid, &cancel).err(),
            Some(MediaError::InvalidAudio)
        );
    }
    let cancel = ResolverCancellation::default();
    cancel.cancel();
    assert_eq!(
        DecodedClockNormalizer::new(identity(), format(16_000, 1), &cancel).err(),
        Some(MediaError::Cancelled)
    );
    assert_eq!(
        DecodedClockWindow::new(identity(), &cancel).err(),
        Some(MediaError::Cancelled)
    );
}

#[test]
fn normalizer_cancellation_from_owner_or_supplied_scope_is_terminal() {
    for cancel_owner in [true, false] {
        let owner = ResolverCancellation::default();
        let supplied = ResolverCancellation::default();
        let mut normalizer =
            DecodedClockNormalizer::new(identity(), format(16_000, 1), &owner).unwrap();
        normalizer.normalize(block(1, 0), &supplied).unwrap();
        if cancel_owner {
            owner.cancel();
        } else {
            supplied.cancel();
        }
        assert_eq!(
            normalizer.normalize(block(2, 10_000), &supplied).err(),
            Some(MediaError::Cancelled)
        );
        assert!(normalizer.previous.is_none());
        assert!(normalizer.terminal);
        let expected = if cancel_owner {
            MediaError::Cancelled
        } else {
            MediaError::InvalidAudio
        };
        assert_eq!(
            normalizer
                .normalize(block(2, 10_000), &ResolverCancellation::default())
                .err(),
            Some(expected)
        );
    }
}

#[test]
fn native_duration_limit_accepts_exact_end_and_rejects_one_microsecond_more() {
    for (start, expected) in [
        (MAX_DECODED_MEDIA_DURATION_US - 10_000, None),
        (
            MAX_DECODED_MEDIA_DURATION_US - 9_999,
            Some(MediaError::TooLarge),
        ),
    ] {
        let cancel = ResolverCancellation::default();
        let mut normalizer =
            DecodedClockNormalizer::new(identity(), format(16_000, 1), &cancel).unwrap();
        let result = normalizer.normalize(block(1, start), &cancel);
        assert_eq!(result.err(), expected);
    }
}

#[test]
fn sample_clock_duration_limit_is_checked_independently_of_native_end() {
    let cancel = ResolverCancellation::default();
    let mut normalizer =
        DecodedClockNormalizer::new(identity(), format(16_000, 1), &cancel).unwrap();
    normalizer.normalize(block(1, 0), &cancel).unwrap();
    // Simulate accumulated clock state near the bound, without allocating a
    // day's PCM or changing the incoming block's small valid native interval.
    normalizer.run_origin_us = MAX_DECODED_MEDIA_DURATION_US - 10_000;
    assert_eq!(
        normalizer.normalize(block(2, 10_000), &cancel).err(),
        Some(MediaError::TooLarge)
    );
    assert!(normalizer.terminal);
    assert!(normalizer.previous.is_none());
}

#[test]
fn frame_counter_and_run_counter_overflow_fail_without_wraparound() {
    let cancel = ResolverCancellation::default();
    let mut normalizer =
        DecodedClockNormalizer::new(identity(), format(16_000, 1), &cancel).unwrap();
    normalizer.normalize(block(1, 0), &cancel).unwrap();
    normalizer.run_frames = u64::MAX;
    assert_eq!(
        normalizer.normalize(block(2, 10_000), &cancel).err(),
        Some(MediaError::InvalidAudio)
    );
    assert!(normalizer.terminal);
    let mut normalizer =
        DecodedClockNormalizer::new(identity(), format(16_000, 1), &cancel).unwrap();
    normalizer.normalize(block(1, 0), &cancel).unwrap();
    normalizer.previous.as_mut().unwrap().run_id = u64::MAX;
    assert_eq!(
        normalizer.normalize(block(2, 10_002), &cancel).err(),
        Some(MediaError::InvalidAudio)
    );
    assert!(normalizer.terminal);
    assert_eq!(
        frame_time(0, u64::MAX, 16_000),
        Err(MediaError::InvalidAudio)
    );
    assert_eq!(
        frame_time(u64::MAX, 1, 16_000),
        Err(MediaError::InvalidAudio)
    );
}

#[test]
fn half_open_boundaries_restore_the_right_start_and_left_end() {
    let cancel = ResolverCancellation::default();
    for next_native in [109_999, 110_001] {
        let spans = spans_at(&[100_000, next_native]);
        let mut window = window_with(&spans);
        assert_eq!(
            window
                .map_interval(identity(), 1, 100_000, 110_000, &cancel)
                .unwrap(),
            NativeMediaInterval {
                identity: identity(),
                run_id: 1,
                first_source_sequence: 1,
                last_source_sequence: 1,
                start_us: 100_000,
                end_us: 110_000
            }
        );
        assert_eq!(
            window
                .map_interval(identity(), 1, 110_000, 120_000, &cancel)
                .unwrap(),
            NativeMediaInterval {
                identity: identity(),
                run_id: 1,
                first_source_sequence: 2,
                last_source_sequence: 2,
                start_us: next_native,
                end_us: next_native + 10_000
            }
        );
    }
}

#[test]
fn interval_across_many_continuous_spans_uses_observed_outer_endpoints() {
    let cancel = ResolverCancellation::default();
    let spans = spans_at(&[100_000, 110_001, 120_001, 130_000]);
    let mut window = window_with(&spans);
    assert_eq!(
        window
            .map_interval(identity(), 1, 105_000, 135_000, &cancel)
            .unwrap(),
        NativeMediaInterval {
            identity: identity(),
            run_id: 1,
            first_source_sequence: 1,
            last_source_sequence: 4,
            start_us: 105_000,
            end_us: 135_000
        }
    );
    assert_eq!(
        window
            .map_interval(identity(), 1, 130_000, 140_000, &cancel)
            .unwrap(),
        NativeMediaInterval {
            identity: identity(),
            run_id: 1,
            first_source_sequence: 4,
            last_source_sequence: 4,
            start_us: 130_000,
            end_us: 140_000
        }
    );
}

#[test]
fn cumulative_rounding_span_uses_one_rational_conversion_with_exact_endpoints() {
    let cancel = ResolverCancellation::default();
    let pcm_format = format(44_100, 1);
    let mut normalizer = DecodedClockNormalizer::new(identity(), pcm_format, &cancel).unwrap();
    normalizer
        .normalize(block_for(identity(), 1, 0, pcm_format, 1), &cancel)
        .unwrap();
    let span = normalizer
        .normalize(block_for(identity(), 2, 22, pcm_format, 1), &cancel)
        .unwrap()
        .span();
    assert_eq!((span.sample_start_us(), span.sample_end_us()), (22, 45));
    assert_eq!((span.native_start_us(), span.native_end_us()), (22, 44));
    for position in span.sample_start_us()..=span.sample_end_us() {
        let offset = u128::from(position - span.sample_start_us());
        let native_duration = u128::from(span.native_end_us() - span.native_start_us());
        let sample_duration = u128::from(span.sample_end_us() - span.sample_start_us());
        let expected = span.native_start_us() + (offset * native_duration / sample_duration) as u64;
        assert_eq!(span.at(position).unwrap(), expected);
    }
    assert_eq!(span.at(21), Err(MediaError::InvalidAudio));
    assert_eq!(span.at(46), Err(MediaError::InvalidAudio));
}

#[test]
fn interval_collapsed_by_rounding_is_rejected_instead_of_inventing_duration() {
    let cancel = ResolverCancellation::default();
    let pcm_format = format(44_100, 1);
    let mut normalizer = DecodedClockNormalizer::new(identity(), pcm_format, &cancel).unwrap();
    let first = normalizer
        .normalize(block_for(identity(), 1, 0, pcm_format, 1), &cancel)
        .unwrap()
        .span();
    let second = normalizer
        .normalize(block_for(identity(), 2, 22, pcm_format, 1), &cancel)
        .unwrap()
        .span();
    let mut window = window_with(&[first, second]);
    assert_eq!(
        window.map_interval(identity(), 1, 22, 23, &cancel),
        Err(MediaError::InvalidAudio)
    );
    assert_window_failed(&mut window, first);
}

#[test]
fn empty_reversed_unseen_and_wrong_run_intervals_fail_terminally() {
    let cancel = ResolverCancellation::default();
    let spans = spans_at(&[100_000, 110_000]);
    for (run, start, end) in [
        (0, 100_000, 105_000),
        (2, 100_000, 105_000),
        (1, 100_000, 100_000),
        (1, 110_000, 100_000),
        (1, 99_999, 100_001),
        (1, 119_999, 120_001),
        (1, 120_000, 120_001),
    ] {
        let mut window = window_with(&spans);
        assert_eq!(
            window.map_interval(identity(), run, start, end, &cancel),
            Err(MediaError::InvalidAudio)
        );
        assert_window_failed(&mut window, spans[0]);
    }
}

#[test]
fn cues_cannot_cross_a_true_gap_even_when_both_outer_endpoints_are_observed() {
    let cancel = ResolverCancellation::default();
    let spans = spans_at(&[100_000, 120_000]);
    for run in [1, 2] {
        let mut window = window_with(&spans);
        assert_eq!(
            window.map_interval(identity(), run, 105_000, 125_000, &cancel),
            Err(MediaError::InvalidAudio)
        );
        assert_window_failed(&mut window, spans[0]);
    }
    let mut window = window_with(&spans);
    assert_eq!(
        window
            .map_interval(identity(), 2, 120_000, 130_000, &cancel)
            .unwrap(),
        NativeMediaInterval {
            identity: identity(),
            run_id: 2,
            first_source_sequence: 2,
            last_source_sequence: 2,
            start_us: 120_000,
            end_us: 130_000
        }
    );
}

#[test]
fn window_stale_session_or_generation_rejects_append_and_mapping() {
    let cancel = ResolverCancellation::default();
    let spans = spans_at(&[0, 10_000]);
    for stale in [
        StreamIdentity {
            session: SessionId(74),
            ..identity()
        },
        StreamIdentity {
            generation: 3,
            ..identity()
        },
    ] {
        let mut window = window_with(&spans[..1]);
        let mut stale_span = spans[1];
        stale_span.identity = stale;
        assert_eq!(window.append(stale_span, &cancel), Err(MediaError::Stale));
        assert_window_failed(&mut window, spans[0]);
        let mut window = window_with(&spans);
        assert_eq!(
            window.map_interval(stale, 1, 0, 10_000, &cancel),
            Err(MediaError::Stale)
        );
        assert_window_failed(&mut window, spans[0]);
    }
}

#[test]
fn window_enforces_initial_and_successive_sequence_format_and_run_evidence() {
    let cancel = ResolverCancellation::default();
    let spans = spans_at(&[0, 10_000]);
    for change in 0..6 {
        let mut second = spans[1];
        match change {
            0 => second.sequence = 1,
            1 => second.sequence = 3,
            2 => second.format = format(24_000, 1),
            3 => second.run_id = 2,
            4 => second.sample_start_us += 1,
            _ => second.discontinuity = true,
        }
        let mut window = window_with(&spans[..1]);
        assert_eq!(
            window.append(second, &cancel),
            Err(MediaError::InvalidAudio)
        );
        assert_window_failed(&mut window, spans[0]);
    }
    for change in 0..3 {
        let mut first = spans[0];
        match change {
            0 => first.sequence = 2,
            1 => first.run_id = 2,
            _ => first.discontinuity = true,
        }
        let mut window = DecodedClockWindow::new(identity(), &cancel).unwrap();
        assert_eq!(window.append(first, &cancel), Err(MediaError::InvalidAudio));
        assert_window_failed(&mut window, spans[0]);
    }
}

#[test]
fn copied_spans_from_distinct_normalizers_cannot_conceal_native_overlap_or_gap() {
    let cancel = ResolverCancellation::default();
    let first = spans_at(&[100_000])[0];
    for second_native in [109_998, 110_002] {
        // Each source normalizer was individually valid; its tiny first-packet
        // timestamp offset vanished from cumulative sample time at packet two.
        let other = spans_at(&[second_native - 10_000, second_native]);
        let mut second = other[1];
        // Keep the plausible public binding, isolating independent validation
        // of the native timing rather than another obvious sample/run mismatch.
        second.sample_start_us = first.sample_end_us();
        second.sample_end_us = first.sample_end_us() + 10_000;
        second.run_id = 1;
        second.discontinuity = false;
        let mut window = window_with(&[first]);
        assert_eq!(
            window.append(second, &cancel),
            Err(MediaError::InvalidAudio)
        );
        assert_window_failed(&mut window, first);
    }
}

#[test]
fn public_copied_spans_cannot_mix_different_fractional_frame_phases() {
    let cancel = ResolverCancellation::default();
    let pcm_format = format(44_100, 1);
    let mut first = DecodedClockNormalizer::new(identity(), pcm_format, &cancel).unwrap();
    let a1 = first
        .normalize(block_for(identity(), 1, 0, pcm_format, 2), &cancel)
        .unwrap()
        .span();
    let a2 = first
        .normalize(block_for(identity(), 2, 45, pcm_format, 1), &cancel)
        .unwrap()
        .span();
    let mut other = DecodedClockNormalizer::new(identity(), pcm_format, &cancel).unwrap();
    other
        .normalize(block_for(identity(), 1, 23, pcm_format, 1), &cancel)
        .unwrap();
    other
        .normalize(block_for(identity(), 2, 45, pcm_format, 1), &cancel)
        .unwrap();
    let b3 = other
        .normalize(block_for(identity(), 3, 67, pcm_format, 1), &cancel)
        .unwrap()
        .span();
    // All public endpoint/sequence/run values seem adjacent, but the other
    // origin's cumulative sample frame phase gives end91 instead of end90.
    assert_eq!(a2.sample_end_us(), b3.sample_start_us());
    assert_eq!(b3.sample_end_us(), 91);
    let mut window = window_with(&[a1, a2]);
    assert_eq!(window.append(b3, &cancel), Err(MediaError::InvalidAudio));
    assert_window_failed(&mut window, a1);
}

#[test]
fn discontinuity_must_preserve_native_gap_length_and_sequential_run_id() {
    let cancel = ResolverCancellation::default();
    let spans = spans_at(&[0, 20_000]);
    for change in 0..4 {
        let mut second = spans[1];
        match change {
            0 => second.run_id = 3,
            1 => second.sample_start_us += 1,
            2 => second.native_start_us -= 1,
            _ => second.native_start_us = spans[0].native_end_us() + 1,
        }
        let mut window = window_with(&spans[..1]);
        assert_eq!(
            window.append(second, &cancel),
            Err(MediaError::InvalidAudio)
        );
        assert_window_failed(&mut window, spans[0]);
    }
}

#[test]
fn twelve_second_metadata_limit_is_exact_and_never_discards_needed_evidence() {
    let cancel = ResolverCancellation::default();
    let pcm_format = format(8_000, 1);
    let mut normalizer = DecodedClockNormalizer::new(identity(), pcm_format, &cancel).unwrap();
    let mut window = DecodedClockWindow::new(identity(), &cancel).unwrap();
    let mut first = None;
    for index in 0..6_u64 {
        let span = normalizer
            .normalize(
                block_for(identity(), index + 1, index * 2_000_000, pcm_format, 16_000),
                &cancel,
            )
            .unwrap()
            .span();
        first.get_or_insert(span);
        window.append(span, &cancel).unwrap();
    }
    assert_eq!(window.spans.len(), 6);
    assert_eq!(
        window.spans.back().unwrap().sample_end_us(),
        MAX_CLOCK_WINDOW_DURATION_US
    );
    assert_eq!(
        window
            .map_interval(identity(), 1, 0, MAX_CLOCK_WINDOW_DURATION_US, &cancel)
            .unwrap(),
        NativeMediaInterval {
            identity: identity(),
            run_id: 1,
            first_source_sequence: 1,
            last_source_sequence: 6,
            start_us: 0,
            end_us: MAX_CLOCK_WINDOW_DURATION_US
        }
    );
    let extra = normalizer
        .normalize(
            block_for(identity(), 7, MAX_CLOCK_WINDOW_DURATION_US, pcm_format, 1),
            &cancel,
        )
        .unwrap()
        .span();
    assert_eq!(window.append(extra, &cancel), Err(MediaError::Overloaded));
    assert_window_failed(&mut window, first.unwrap());
}

#[test]
fn pathological_tiny_packet_count_has_an_independent_exact_limit() {
    let cancel = ResolverCancellation::default();
    let pcm_format = format(16_000, 1);
    let mut normalizer = DecodedClockNormalizer::new(identity(), pcm_format, &cancel).unwrap();
    let mut window = DecodedClockWindow::new(identity(), &cancel).unwrap();
    let mut first = None;
    for index in 0..MAX_CLOCK_WINDOW_SPANS {
        let span = normalizer
            .normalize(
                block_for(
                    identity(),
                    index as u64 + 1,
                    index as u64 * 1_000_000 / 16_000,
                    pcm_format,
                    1,
                ),
                &cancel,
            )
            .unwrap()
            .span();
        first.get_or_insert(span);
        window.append(span, &cancel).unwrap();
    }
    assert_eq!(window.spans.len(), MAX_CLOCK_WINDOW_SPANS);
    let extra = normalizer
        .normalize(
            block_for(
                identity(),
                MAX_CLOCK_WINDOW_SPANS as u64 + 1,
                MAX_CLOCK_WINDOW_SPANS as u64 * 1_000_000 / 16_000,
                pcm_format,
                1,
            ),
            &cancel,
        )
        .unwrap()
        .span();
    assert!(extra.sample_end_us() < MAX_CLOCK_WINDOW_DURATION_US);
    assert_eq!(window.append(extra, &cancel), Err(MediaError::Overloaded));
    assert_window_failed(&mut window, first.unwrap());
}

#[test]
fn pruning_keeps_the_straddling_span_and_removes_only_complete_evidence() {
    let cancel = ResolverCancellation::default();
    let spans = spans_at(&[100_000, 110_000, 120_000]);
    let mut window = window_with(&spans);
    window.prune_before(105_000, &cancel).unwrap();
    assert_eq!(window.spans.len(), 3);
    assert_eq!(window.spans.front().unwrap(), &spans[0]);
    window.prune_before(110_000, &cancel).unwrap();
    assert_eq!(window.spans.len(), 2);
    assert_eq!(window.spans.front().unwrap(), &spans[1]);
    assert_eq!(
        window
            .map_interval(identity(), 1, 110_000, 130_000, &cancel)
            .unwrap(),
        NativeMediaInterval {
            identity: identity(),
            run_id: 1,
            first_source_sequence: 2,
            last_source_sequence: 3,
            start_us: 110_000,
            end_us: 130_000
        }
    );
    assert_eq!(
        window.map_interval(identity(), 1, 109_999, 110_001, &cancel),
        Err(MediaError::InvalidAudio)
    );
    assert_window_failed(&mut window, spans[0]);
}

#[test]
fn pruning_reclaims_duration_capacity_without_resetting_sequence_or_run() {
    let cancel = ResolverCancellation::default();
    let pcm_format = format(8_000, 1);
    let mut normalizer = DecodedClockNormalizer::new(identity(), pcm_format, &cancel).unwrap();
    let mut window = DecodedClockWindow::new(identity(), &cancel).unwrap();
    for index in 0..6_u64 {
        let span = normalizer
            .normalize(
                block_for(identity(), index + 1, index * 2_000_000, pcm_format, 16_000),
                &cancel,
            )
            .unwrap()
            .span();
        window.append(span, &cancel).unwrap();
    }
    window.prune_before(2_000_000, &cancel).unwrap();
    let next = normalizer
        .normalize(
            block_for(identity(), 7, 12_000_000, pcm_format, 16_000),
            &cancel,
        )
        .unwrap()
        .span();
    window.append(next, &cancel).unwrap();
    assert_eq!(window.spans.len(), 6);
    assert_eq!(window.spans.front().unwrap().sequence(), 2);
    assert_eq!(window.previous.unwrap().sequence(), 7);
    assert_eq!(window.previous.unwrap().run_id(), 1);
    window.prune_before(14_000_000, &cancel).unwrap();
    assert!(window.spans.is_empty());
    assert_eq!(window.previous.unwrap().sequence(), 7);
    let last = normalizer
        .normalize(block_for(identity(), 8, 14_000_000, pcm_format, 8), &cancel)
        .unwrap()
        .span();
    window.append(last, &cancel).unwrap();
    assert_eq!(window.spans.len(), 1);
    assert_eq!(window.spans.front().unwrap().sequence(), 8);
}

#[test]
fn pruning_unobserved_future_or_an_empty_window_is_terminal() {
    let cancel = ResolverCancellation::default();
    let spans = spans_at(&[100_000]);
    let mut window = window_with(&spans);
    assert_eq!(
        window.prune_before(110_001, &cancel),
        Err(MediaError::InvalidAudio)
    );
    assert_window_failed(&mut window, spans[0]);
    let mut empty = DecodedClockWindow::new(identity(), &cancel).unwrap();
    assert_eq!(
        empty.prune_before(0, &cancel),
        Err(MediaError::InvalidAudio)
    );
    assert_window_failed(&mut empty, spans[0]);
}

#[test]
fn window_owner_and_supplied_cancellation_clear_each_public_operation() {
    let spans = spans_at(&[0, 10_000]);
    for cancel_owner in [true, false] {
        for operation in 0..3 {
            let owner = ResolverCancellation::default();
            let supplied = ResolverCancellation::default();
            let mut window = DecodedClockWindow::new(identity(), &owner).unwrap();
            window.append(spans[0], &supplied).unwrap();
            if cancel_owner {
                owner.cancel();
            } else {
                supplied.cancel();
            }
            let result = match operation {
                0 => window.append(spans[1], &supplied),
                1 => window.prune_before(10_000, &supplied),
                _ => window
                    .map_interval(identity(), 1, 0, 10_000, &supplied)
                    .map(|_| ()),
            };
            assert_eq!(result, Err(MediaError::Cancelled));
            assert!(window.terminal);
            assert!(window.spans.is_empty());
            assert!(window.previous.is_none());
            let expected = if cancel_owner {
                MediaError::Cancelled
            } else {
                MediaError::InvalidAudio
            };
            assert_eq!(
                window.append(spans[1], &ResolverCancellation::default()),
                Err(expected)
            );
        }
    }
}
