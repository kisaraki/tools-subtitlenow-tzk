//! Pure bounded timing evidence: no file, provider, network or retained PCM.
use super::*;
use crate::audio::capture::SessionId;

fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(81),
        generation: 3,
    }
}

fn spans_at(starts: &[u64]) -> Vec<DecodedClockSpan> {
    let cancel = ResolverCancellation::default();
    let format = DecodedAudioFormat::new(16_000, 1).unwrap();
    let mut normalizer = DecodedClockNormalizer::new(identity(), format, &cancel).unwrap();
    starts
        .iter()
        .enumerate()
        .map(|(index, start)| {
            normalizer
                .normalize(
                    DecodedAudioBlock::new(
                        identity(),
                        index as u64 + 1,
                        *start,
                        format,
                        vec![17; 160],
                    )
                    .unwrap(),
                    &cancel,
                )
                .unwrap()
                .span()
        })
        .collect()
}

fn packet_spans(
    identity: StreamIdentity,
    rate: u32,
    origin: u64,
    frames: &[usize],
) -> Vec<DecodedClockSpan> {
    let cancel = ResolverCancellation::default();
    let format = DecodedAudioFormat::new(rate, 1).unwrap();
    let mut normalizer = DecodedClockNormalizer::new(identity, format, &cancel).unwrap();
    let mut native_ticks = origin * 10;
    frames
        .iter()
        .enumerate()
        .map(|(index, frames)| {
            let block = DecodedAudioBlock::new(
                identity,
                index as u64 + 1,
                native_ticks / 10,
                format,
                vec![17; *frames],
            )
            .unwrap();
            native_ticks += *frames as u64 * 10_000_000 / u64::from(rate);
            normalizer.normalize(block, &cancel).unwrap().span()
        })
        .collect()
}

fn window_with(spans: &[DecodedClockSpan]) -> DecodedClockWindow {
    let cancel = ResolverCancellation::default();
    let mut window = DecodedClockWindow::new(spans[0].identity(), &cancel).unwrap();
    for span in spans {
        window.append(*span, &cancel).unwrap();
    }
    window
}

#[test]
fn dense_adts_frame_clock_fits_the_bounded_snapshot_without_extending_twelve_seconds() {
    let spans = packet_spans(identity(), 48_000, 0, &vec![1024; 560]);
    let mut window = window_with(&spans);
    let end = spans.last().unwrap().sample_end_us();
    assert!(end < MAX_CLOCK_WINDOW_DURATION_US);
    let snapshot = window
        .snapshot(identity(), 0, end, &ResolverCancellation::default())
        .unwrap();
    assert_eq!(snapshot.spans.len(), 560);
    let interval = snapshot.map_interval(identity(), 0, end).unwrap();
    assert_eq!(interval.end_us(), spans.last().unwrap().native_end_us());
    assert!(
        window
            .snapshot(
                identity(),
                0,
                MAX_CLOCK_WINDOW_DURATION_US + 1,
                &ResolverCancellation::default()
            )
            .is_err()
    );
}

fn snapshot(spans: &[DecodedClockSpan], start: u64, end: u64) -> DecodedClockSnapshot {
    window_with(spans)
        .snapshot(
            spans[0].identity(),
            start,
            end,
            &ResolverCancellation::default(),
        )
        .unwrap()
}

fn assert_failed(window: &DecodedClockWindow) {
    assert!(window.terminal);
    assert!(window.spans.is_empty());
    assert!(window.previous.is_none());
    assert!(window.coverage_start_us.is_none());
}

#[test]
fn detached_evidence_survives_complete_pruning_and_window_drop() {
    let cancel = ResolverCancellation::default();
    let spans = spans_at(&[100_000, 110_001, 120_001]);
    let mut window = window_with(&spans);
    let snapshot = window
        .snapshot(identity(), 105_000, 125_000, &cancel)
        .unwrap();
    let expected = window
        .map_interval(identity(), 1, 106_000, 124_000, &cancel)
        .unwrap();
    window.prune_before(130_000, &cancel).unwrap();
    assert!(window.spans.is_empty());
    drop(window);
    assert_eq!(
        snapshot.map_interval(identity(), 106_000, 124_000).unwrap(),
        expected
    );
}

#[test]
fn later_window_failure_does_not_revoke_an_already_detached_snapshot() {
    let cancel = ResolverCancellation::default();
    let spans = spans_at(&[0, 10_001]);
    let mut window = window_with(&spans);
    let snapshot = window.snapshot(identity(), 0, 20_000, &cancel).unwrap();
    assert_eq!(
        window.append(spans[1], &cancel),
        Err(MediaError::InvalidAudio)
    );
    assert_failed(&window);
    let mapped = snapshot.map_interval(identity(), 10_000, 20_000).unwrap();
    assert_eq!((mapped.start_us(), mapped.end_us()), (10_001, 20_001));
}

#[test]
fn exact_context_extent_is_owned_while_full_source_packets_are_preserved() {
    let spans = spans_at(&[0, 10_001, 20_001, 30_000]);
    let snapshot = snapshot(&spans, 5_000, 25_000);
    assert_eq!(snapshot.spans.as_ref(), &spans[..3]);
    assert_eq!(snapshot.spans.len(), 3);
    assert!(snapshot.map_interval(identity(), 5_000, 25_000).is_ok());
    assert_eq!(
        snapshot.map_interval(identity(), 4_999, 5_001),
        Err(MediaError::InvalidAudio)
    );
    assert_eq!(
        snapshot.map_interval(identity(), 24_999, 25_001),
        Err(MediaError::InvalidAudio)
    );
    // An invalid request cannot mutate immutable evidence used by other cues.
    assert!(snapshot.map_interval(identity(), 10_000, 20_000).is_ok());
}

#[test]
fn snapshot_boundaries_keep_the_right_start_and_left_end_packet() {
    for native_start in [9_999, 10_001] {
        let spans = spans_at(&[0, native_start]);
        let left = snapshot(&spans, 0, 10_000);
        let right = snapshot(&spans, 10_000, 20_000);
        let a = left.map_interval(identity(), 0, 10_000).unwrap();
        let b = right.map_interval(identity(), 10_000, 20_000).unwrap();
        assert_eq!(
            (a.first_source_sequence(), a.last_source_sequence()),
            (1, 1)
        );
        assert_eq!(
            (b.first_source_sequence(), b.last_source_sequence()),
            (2, 2)
        );
        assert_eq!(a.end_us(), 10_000);
        assert_eq!(
            (b.start_us(), b.end_us()),
            (native_start, native_start + 10_000)
        );
        assert_eq!(right.validate_successor(&left), Ok(()));
    }
}

#[test]
fn source_run_is_inferred_and_context_cannot_cross_a_true_gap() {
    let spans = spans_at(&[0, 20_000, 30_000]);
    let snapshot = snapshot(&spans, 20_000, 40_000);
    assert_eq!(snapshot.source_run_id(), 2);
    assert_eq!(
        snapshot
            .map_interval(identity(), 20_000, 40_000)
            .unwrap()
            .run_id(),
        2
    );
    let mut window = window_with(&spans);
    assert_eq!(
        window
            .snapshot(identity(), 5_000, 25_000, &ResolverCancellation::default())
            .err(),
        Some(MediaError::InvalidAudio)
    );
    assert_failed(&window);
}

#[test]
fn unseen_pruned_empty_and_reversed_context_requests_clear_the_window() {
    let spans = spans_at(&[0, 10_000]);
    for (start, end) in [(0, 0), (11_000, 10_000), (0, 20_001), (20_000, 20_001)] {
        let mut window = window_with(&spans);
        assert_eq!(
            window
                .snapshot(identity(), start, end, &ResolverCancellation::default())
                .err(),
            Some(MediaError::InvalidAudio)
        );
        assert_failed(&window);
    }
    let mut window = window_with(&spans);
    window
        .prune_before(10_000, &ResolverCancellation::default())
        .unwrap();
    assert_eq!(
        window
            .snapshot(identity(), 9_999, 10_001, &ResolverCancellation::default())
            .err(),
        Some(MediaError::InvalidAudio)
    );
    assert_failed(&window);
    let mut empty = DecodedClockWindow::new(identity(), &ResolverCancellation::default()).unwrap();
    assert_eq!(
        empty
            .snapshot(identity(), 0, 1, &ResolverCancellation::default())
            .err(),
        Some(MediaError::InvalidAudio)
    );
    assert_failed(&empty);
}

#[test]
fn stale_session_and_generation_cannot_create_or_use_snapshots() {
    let spans = spans_at(&[0, 10_000]);
    let valid = snapshot(&spans, 0, 20_000);
    for stale in [
        StreamIdentity {
            session: SessionId(82),
            ..identity()
        },
        StreamIdentity {
            generation: 4,
            ..identity()
        },
    ] {
        assert_eq!(valid.map_interval(stale, 0, 10_000), Err(MediaError::Stale));
        let mut window = window_with(&spans);
        assert_eq!(
            window
                .snapshot(stale, 0, 10_000, &ResolverCancellation::default())
                .err(),
            Some(MediaError::Stale)
        );
        assert_failed(&window);
    }
}

#[test]
fn cancellation_of_either_retained_scope_revokes_mapping_after_window_drop() {
    let spans = spans_at(&[0, 10_000]);
    for cancel_owner in [true, false] {
        let owner = ResolverCancellation::default();
        let creation = ResolverCancellation::default();
        let mut window = DecodedClockWindow::new(identity(), &owner).unwrap();
        for span in &spans {
            window
                .append(*span, &ResolverCancellation::default())
                .unwrap();
        }
        let snapshot = window.snapshot(identity(), 0, 20_000, &creation).unwrap();
        drop(window);
        if cancel_owner {
            owner.cancel();
        } else {
            creation.cancel();
        }
        assert_eq!(
            snapshot.map_interval(identity(), 0, 10_000),
            Err(MediaError::Cancelled)
        );
    }
}

#[test]
fn cancelled_snapshot_creation_is_terminal_with_either_cancellation_scope() {
    let spans = spans_at(&[0]);
    for cancel_owner in [true, false] {
        let owner = ResolverCancellation::default();
        let supplied = ResolverCancellation::default();
        let mut window = DecodedClockWindow::new(identity(), &owner).unwrap();
        window.append(spans[0], &supplied).unwrap();
        if cancel_owner {
            owner.cancel();
        } else {
            supplied.cancel();
        }
        assert_eq!(
            window.snapshot(identity(), 0, 10_000, &supplied).err(),
            Some(MediaError::Cancelled)
        );
        assert_failed(&window);
        let expected = if cancel_owner {
            MediaError::Cancelled
        } else {
            MediaError::InvalidAudio
        };
        assert_eq!(
            window
                .snapshot(identity(), 0, 10_000, &ResolverCancellation::default())
                .err(),
            Some(expected)
        );
    }
}

#[test]
fn fractional_packet_phase_is_not_recomputed_from_clipped_context() {
    let spans = packet_spans(identity(), 44_100, 0, &[1, 1]);
    assert_eq!((spans[1].sample_start_us, spans[1].sample_end_us), (22, 45));
    let snapshot = snapshot(&spans, 23, 44);
    assert_eq!(snapshot.spans.as_ref(), &spans[1..]);
    let mapped = snapshot.map_interval(identity(), 24, 44).unwrap();
    // Independent integer oracle using the original 23 us / 22 us durations.
    assert_eq!(mapped.start_us(), 22 + 2 * 22 / 23);
    assert_eq!(mapped.end_us(), 22 + 22 * 22 / 23);
    assert_eq!(
        snapshot.map_interval(identity(), 22, 24),
        Err(MediaError::InvalidAudio)
    );
}

#[test]
fn a_positive_sample_interval_that_collapses_in_native_time_is_rejected() {
    let spans = packet_spans(identity(), 44_100, 0, &[1, 1]);
    let snapshot = snapshot(&spans, 22, 45);
    assert_eq!(
        snapshot.map_interval(identity(), 22, 23),
        Err(MediaError::InvalidAudio)
    );
    assert_eq!(
        snapshot.map_interval(identity(), 30, 30),
        Err(MediaError::InvalidAudio)
    );
    assert_eq!(
        snapshot.map_interval(identity(), 31, 30),
        Err(MediaError::InvalidAudio)
    );
    let mut window = window_with(&spans);
    assert_eq!(
        window
            .snapshot(identity(), 22, 23, &ResolverCancellation::default())
            .err(),
        Some(MediaError::InvalidAudio)
    );
    assert_failed(&window);
}

#[test]
fn maximum_packet_count_is_copied_once_and_is_independent_of_window_overload() {
    let frames = vec![1; MAX_CLOCK_WINDOW_SPANS + 1];
    let spans = packet_spans(identity(), 16_000, 0, &frames);
    let cancel = ResolverCancellation::default();
    let mut window = window_with(&spans[..MAX_CLOCK_WINDOW_SPANS]);
    let end = spans[MAX_CLOCK_WINDOW_SPANS - 1].sample_end_us();
    let snapshot = window.snapshot(identity(), 0, end, &cancel).unwrap();
    assert_eq!(snapshot.spans.len(), MAX_CLOCK_WINDOW_SPANS);
    assert_ne!(
        snapshot.spans.as_ptr(),
        window.spans.front().unwrap() as *const _
    );
    assert_eq!(
        window.append(spans[MAX_CLOCK_WINDOW_SPANS], &cancel),
        Err(MediaError::Overloaded)
    );
    assert_failed(&window);
    let mapped = snapshot.map_interval(identity(), 0, end).unwrap();
    assert_eq!(mapped.last_source_sequence(), MAX_CLOCK_WINDOW_SPANS as u64);
    assert_eq!(
        mapped.end_us(),
        spans[MAX_CLOCK_WINDOW_SPANS - 1].native_end_us()
    );
}

#[test]
fn maximum_time_extent_and_a_small_exact_subrange_keep_bounded_source_evidence() {
    let spans = packet_spans(identity(), 8_000, 0, &[16_000; 6]);
    let full = snapshot(&spans, 0, MAX_CLOCK_WINDOW_DURATION_US);
    assert_eq!(full.spans.len(), 6);
    assert_eq!(
        full.map_interval(identity(), 0, MAX_CLOCK_WINDOW_DURATION_US)
            .unwrap()
            .end_us(),
        MAX_CLOCK_WINDOW_DURATION_US
    );
    let small = snapshot(&spans, 5_999_999, 6_000_001);
    assert_eq!(small.spans.as_ref(), &spans[2..4]);
    assert_eq!(
        small
            .map_interval(identity(), 5_999_999, 6_000_001)
            .unwrap()
            .first_source_sequence(),
        3
    );
    assert_eq!(
        small.map_interval(identity(), 5_999_998, 6_000_001),
        Err(MediaError::InvalidAudio)
    );
}

#[test]
fn successive_context_maps_share_identical_full_source_spans() {
    let spans = spans_at(&[0, 10_001, 20_001, 30_000]);
    let first = snapshot(&spans, 0, 25_000);
    let next = snapshot(&spans, 15_000, 40_000);
    assert_eq!(next.validate_successor(&first), Ok(()));
    assert_eq!(
        first.validate_successor(&next),
        Err(MediaError::InvalidAudio)
    );
    assert_eq!(first.validate_successor(&first), Ok(()));
    let mut altered = snapshot(&spans, 15_000, 40_000);
    altered.spans[0].native_start_us += 1;
    assert_eq!(
        altered.validate_successor(&first),
        Err(MediaError::InvalidAudio)
    );
}

#[test]
fn disjoint_successors_require_adjacent_sequence_frames_origin_and_native_time() {
    let spans = spans_at(&[0, 10_000, 20_000, 30_000]);
    let first = snapshot(&spans, 0, 10_000);
    assert_eq!(
        snapshot(&spans, 10_000, 20_000).validate_successor(&first),
        Ok(())
    );
    assert_eq!(
        snapshot(&spans, 20_000, 30_000).validate_successor(&first),
        Err(MediaError::InvalidAudio)
    );
    for change in 0..4 {
        let mut next = snapshot(&spans, 10_000, 20_000);
        match change {
            0 => next.spans[0].source_frame_start += 1,
            1 => next.spans[0].sample_run_origin_us += 1,
            2 => next.spans[0].native_start_us += 2,
            _ => next.spans[0].discontinuity = true,
        }
        assert_eq!(
            next.validate_successor(&first),
            Err(MediaError::InvalidAudio)
        );
    }
}

#[test]
fn distinct_normalizers_cannot_mix_their_fractional_phase_between_snapshots() {
    let a = packet_spans(identity(), 44_100, 0, &[2, 1]);
    let b = packet_spans(identity(), 44_100, 23, &[1, 1, 1]);
    assert_eq!(a[1].sample_end_us(), b[2].sample_start_us());
    let first = snapshot(&a, 0, a[1].sample_end_us());
    let next = snapshot(&b, b[2].sample_start_us(), b[2].sample_end_us());
    assert_eq!(
        next.validate_successor(&first),
        Err(MediaError::InvalidAudio)
    );
}

#[test]
fn skipped_source_runs_require_forward_sequence_and_true_sample_native_gaps() {
    let spans = spans_at(&[0, 20_000, 40_000]);
    let first = snapshot(&spans, 0, 10_000);
    let third = snapshot(&spans, 40_000, 50_000);
    assert_eq!(third.source_run_id(), 3);
    assert_eq!(third.validate_successor(&first), Ok(()));
    for change in 0..3 {
        let mut next = snapshot(&spans, 40_000, 50_000);
        match change {
            0 => next.spans[0].sequence = 1,
            1 => next.sample_start_us = 10_000,
            _ => next.spans[0].native_start_us = 10_000,
        }
        assert_eq!(
            next.validate_successor(&first),
            Err(MediaError::InvalidAudio)
        );
    }
}

#[test]
fn successor_validation_retains_session_format_and_both_cancellation_scopes() {
    let spans = spans_at(&[0, 10_000]);
    let first = snapshot(&spans, 0, 10_000);
    let mut next = snapshot(&spans, 10_000, 20_000);
    next.identity.generation += 1;
    assert_eq!(next.validate_successor(&first), Err(MediaError::Stale));
    let other_format = packet_spans(identity(), 24_000, 10_000, &[240]);
    let changed = snapshot(&other_format, 10_000, 20_000);
    assert_eq!(
        changed.validate_successor(&first),
        Err(MediaError::InvalidAudio)
    );
    for cancel_previous in [false, true] {
        let first = snapshot(&spans, 0, 10_000);
        let next = snapshot(&spans, 10_000, 20_000);
        if cancel_previous {
            first.creation_cancel.cancel();
        } else {
            next.cancel.cancel();
        }
        assert_eq!(next.validate_successor(&first), Err(MediaError::Cancelled));
    }
}

#[test]
fn large_pending_block_detaches_a_chunk_before_explicit_prune_and_commit() {
    let cancel = ResolverCancellation::default();
    // Retained evidence reaches 11 s. The next original 64 KiB-bounded block
    // reaches 13.048 s, but the completed chunk owns only its first half second.
    let spans = packet_spans(
        identity(),
        8_000,
        0,
        &[16_000, 16_000, 16_000, 16_000, 16_000, 8_000, 16_384],
    );
    let pending = spans[6];
    let mut window = window_with(&spans[..6]);
    let retained_previous = window.previous;
    window.validate_pending(pending, &cancel).unwrap();
    assert_eq!(window.previous, retained_previous);
    assert_eq!(window.spans.len(), 6);
    let mapped = window
        .snapshot_with_pending(identity(), 0, 11_500_000, pending, &cancel)
        .unwrap();
    assert_eq!(mapped.spans.as_ref(), spans.as_slice());
    assert_eq!(
        mapped
            .map_interval(identity(), 11_000_000, 11_500_000)
            .unwrap()
            .end_us(),
        11_500_000
    );
    assert_eq!(window.previous, retained_previous);
    assert_eq!(window.spans.len(), 6);
    window.prune_before(2_000_000, &cancel).unwrap();
    window.append(pending, &cancel).unwrap();
    assert_eq!(window.spans.len(), 6);
    assert_eq!(window.previous, Some(pending));
    assert!(mapped.map_interval(identity(), 0, 11_500_000).is_ok());
    // Public append still refuses the identical unpruned window extent.
    let mut unpruned = window_with(&spans[..6]);
    assert_eq!(
        unpruned.append(pending, &cancel),
        Err(MediaError::Overloaded)
    );
    assert_failed(&unpruned);
}

#[test]
fn pending_gap_larger_than_window_limit_can_detach_old_and_new_runs_separately() {
    let cancel = ResolverCancellation::default();
    let spans = spans_at(&[0, 20_000_000]);
    let mut window = window_with(&spans[..1]);
    window.validate_pending(spans[1], &cancel).unwrap();
    let old = window
        .snapshot_with_pending(identity(), 0, 10_000, spans[1], &cancel)
        .unwrap();
    let next = window
        .snapshot_with_pending(identity(), 20_000_000, 20_010_000, spans[1], &cancel)
        .unwrap();
    assert_eq!(old.spans.as_ref(), &spans[..1]);
    assert_eq!(next.spans.as_ref(), &spans[1..]);
    assert_eq!((old.source_run_id(), next.source_run_id()), (1, 2));
    assert_eq!(next.validate_successor(&old), Ok(()));
    window.prune_before(10_000, &cancel).unwrap();
    window.append(spans[1], &cancel).unwrap();
    assert_eq!(window.spans.as_slices().0, &spans[1..]);
}

#[test]
fn pending_snapshot_retains_the_exact_twelve_second_extent_limit() {
    let spans = packet_spans(
        identity(),
        8_000,
        0,
        &[16_000, 16_000, 16_000, 16_000, 16_000, 8_000, 16_384],
    );
    let pending = spans[6];
    let cancel = ResolverCancellation::default();
    let mut exact = window_with(&spans[..6]);
    let snapshot = exact
        .snapshot_with_pending(
            identity(),
            0,
            MAX_CLOCK_WINDOW_DURATION_US,
            pending,
            &cancel,
        )
        .unwrap();
    // The full source span remains uncut past the exact usable extent.
    assert!(snapshot.spans.last().unwrap().sample_end_us() > MAX_CLOCK_WINDOW_DURATION_US);
    assert_eq!(
        snapshot
            .map_interval(identity(), 11_000_000, MAX_CLOCK_WINDOW_DURATION_US)
            .unwrap()
            .end_us(),
        MAX_CLOCK_WINDOW_DURATION_US
    );
    assert_eq!(
        snapshot.map_interval(identity(), 11_000_000, MAX_CLOCK_WINDOW_DURATION_US + 1),
        Err(MediaError::InvalidAudio)
    );
    let mut too_long = window_with(&spans[..6]);
    assert_eq!(
        too_long
            .snapshot_with_pending(
                identity(),
                0,
                MAX_CLOCK_WINDOW_DURATION_US + 1,
                pending,
                &cancel
            )
            .err(),
        Some(MediaError::Overloaded)
    );
    assert_failed(&too_long);
}

#[test]
fn temporary_extra_row_does_not_increase_the_snapshot_or_retained_row_cap() {
    let cancel = ResolverCancellation::default();
    let spans = packet_spans(identity(), 16_000, 0, &[1; MAX_CLOCK_WINDOW_SPANS + 1]);
    let pending = spans[MAX_CLOCK_WINDOW_SPANS];
    let mut window = window_with(&spans[..MAX_CLOCK_WINDOW_SPANS]);
    window.validate_pending(pending, &cancel).unwrap();
    let snapshot = window
        .snapshot_with_pending(
            identity(),
            spans[1].sample_start_us(),
            pending.sample_end_us(),
            pending,
            &cancel,
        )
        .unwrap();
    assert_eq!(snapshot.spans.len(), MAX_CLOCK_WINDOW_SPANS);
    assert_eq!(snapshot.spans.as_ref(), &spans[1..]);
    assert_eq!(window.spans.len(), MAX_CLOCK_WINDOW_SPANS);
    assert_eq!(window.previous, Some(spans[MAX_CLOCK_WINDOW_SPANS - 1]));
    window
        .prune_before(spans[0].sample_end_us(), &cancel)
        .unwrap();
    window.append(pending, &cancel).unwrap();
    assert_eq!(window.spans.len(), MAX_CLOCK_WINDOW_SPANS);
    let mut too_many = window_with(&spans[..MAX_CLOCK_WINDOW_SPANS]);
    assert_eq!(
        too_many
            .snapshot_with_pending(identity(), 0, pending.sample_end_us(), pending, &cancel)
            .err(),
        Some(MediaError::Overloaded)
    );
    assert_failed(&too_many);
}

#[test]
fn deferred_source_validation_keeps_every_append_phase_and_order_rejection() {
    let cancel = ResolverCancellation::default();
    let spans = packet_spans(identity(), 44_100, 0, &[2, 1]);
    for change in 0..7 {
        let mut invalid = spans[1];
        match change {
            0 => invalid.sequence = 1,
            1 => invalid.sequence = 3,
            2 => invalid.run_id += 1,
            3 => invalid.sample_run_origin_us += 1,
            4 => invalid.native_start_us += 2,
            5 => invalid.source_frame_start += 1,
            _ => invalid.format = DecodedAudioFormat::new(48_000, 1).unwrap(),
        }
        let mut deferred = window_with(&spans[..1]);
        assert_eq!(
            deferred.validate_pending(invalid, &cancel),
            Err(MediaError::InvalidAudio)
        );
        assert_failed(&deferred);
        let mut direct = window_with(&spans[..1]);
        assert_eq!(
            direct.append(invalid, &cancel),
            Err(MediaError::InvalidAudio)
        );
        assert_failed(&direct);
    }
}

#[test]
fn pending_row_is_validated_even_when_an_old_only_snapshot_does_not_cover_it() {
    let cancel = ResolverCancellation::default();
    let spans = spans_at(&[0, 10_000]);
    let mut invalid = spans[1];
    invalid.sequence += 1;
    let mut window = window_with(&spans[..1]);
    assert_eq!(
        window
            .snapshot_with_pending(identity(), 0, 10_000, invalid, &cancel)
            .err(),
        Some(MediaError::InvalidAudio)
    );
    assert_failed(&window);
}

#[test]
fn pending_snapshots_preserve_sequence_and_phase_after_all_stored_rows_are_pruned() {
    let cancel = ResolverCancellation::default();
    let spans = packet_spans(identity(), 44_100, 0, &[2, 1]);
    let mut window = window_with(&spans[..1]);
    window
        .prune_before(spans[0].sample_end_us(), &cancel)
        .unwrap();
    assert!(window.spans.is_empty());
    let snapshot = window
        .snapshot_with_pending(
            identity(),
            spans[1].sample_start_us(),
            spans[1].sample_end_us(),
            spans[1],
            &cancel,
        )
        .unwrap();
    assert_eq!(snapshot.spans.as_ref(), &spans[1..]);
    assert_eq!(
        snapshot
            .map_interval(
                identity(),
                spans[1].sample_start_us(),
                spans[1].sample_end_us()
            )
            .unwrap()
            .end_us(),
        spans[1].native_end_us()
    );
    assert_eq!(window.previous, Some(spans[0]));
    let mut other_phase = spans[1];
    other_phase.sample_run_origin_us += 1;
    assert_eq!(
        window.validate_pending(other_phase, &cancel),
        Err(MediaError::InvalidAudio)
    );
    assert_failed(&window);
}

#[test]
fn pending_stale_source_or_snapshot_request_clears_all_retained_evidence() {
    let cancel = ResolverCancellation::default();
    let spans = spans_at(&[0, 10_000]);
    for stale in [
        StreamIdentity {
            session: SessionId(82),
            ..identity()
        },
        StreamIdentity {
            generation: 4,
            ..identity()
        },
    ] {
        let mut invalid = spans[1];
        invalid.identity = stale;
        let mut window = window_with(&spans[..1]);
        assert_eq!(
            window.validate_pending(invalid, &cancel),
            Err(MediaError::Stale)
        );
        assert_failed(&window);
        let mut window = window_with(&spans[..1]);
        assert_eq!(
            window
                .snapshot_with_pending(stale, 0, 20_000, spans[1], &cancel)
                .err(),
            Some(MediaError::Stale)
        );
        assert_failed(&window);
    }
}

#[test]
fn either_cancellation_scope_terminates_both_deferred_clock_operations() {
    let spans = spans_at(&[0, 10_000]);
    for cancel_owner in [true, false] {
        for snapshot_operation in [true, false] {
            let owner = ResolverCancellation::default();
            let supplied = ResolverCancellation::default();
            let mut window = DecodedClockWindow::new(identity(), &owner).unwrap();
            window.append(spans[0], &supplied).unwrap();
            if cancel_owner {
                owner.cancel();
            } else {
                supplied.cancel();
            }
            let result = if snapshot_operation {
                window
                    .snapshot_with_pending(identity(), 0, 20_000, spans[1], &supplied)
                    .map(|_| ())
            } else {
                window.validate_pending(spans[1], &supplied)
            };
            assert_eq!(result, Err(MediaError::Cancelled));
            assert_failed(&window);
            let expected = if cancel_owner {
                MediaError::Cancelled
            } else {
                MediaError::InvalidAudio
            };
            assert_eq!(
                window.validate_pending(spans[1], &ResolverCancellation::default()),
                Err(expected)
            );
        }
    }
}

#[test]
fn deferred_snapshot_cannot_extrapolate_or_cross_the_pending_true_gap() {
    let cancel = ResolverCancellation::default();
    let spans = spans_at(&[0, 20_000_000]);
    for (start, end) in [
        (5_000, 20_005_000),
        (20_000_000, 20_010_001),
        (19_999_999, 20_000_001),
    ] {
        let mut window = window_with(&spans[..1]);
        assert_eq!(
            window
                .snapshot_with_pending(identity(), start, end, spans[1], &cancel)
                .err(),
            Some(MediaError::InvalidAudio)
        );
        assert_failed(&window);
    }
}

fn retained_large_window(
    spans: &[DecodedClockSpan],
    cancel: &ResolverCancellation,
) -> DecodedClockWindow {
    let mut window = window_with(&spans[..2]);
    for span in &spans[2..5] {
        window.append_retained(*span, 9_500_000, cancel).unwrap();
    }
    window
}

#[test]
fn retained_straddler_supports_the_20480_to_20500_chunk_boundary_without_clipping() {
    let cancel = ResolverCancellation::default();
    // Each original 8 kHz mono block is exactly 64 KiB / 4.096 s. The left
    // context needs 9.5 s while its source row begins at 8.192 s.
    let spans = packet_spans(identity(), 8_000, 0, &[32_768; 6]);
    let mut window = retained_large_window(&spans, &cancel);
    assert_eq!(window.spans.front(), Some(&spans[2]));
    assert_eq!(window.spans.back(), Some(&spans[4]));
    assert_eq!(
        window.spans.back().unwrap().sample_end_us()
            - window.spans.front().unwrap().sample_start_us(),
        12_288_000
    );
    assert_eq!(window.coverage_start_us, Some(9_500_000));
    let snapshot = window
        .snapshot_with_pending(identity(), 9_500_000, 20_500_000, spans[5], &cancel)
        .unwrap();
    assert_eq!(snapshot.spans.as_ref(), &spans[2..]);
    assert_eq!(
        snapshot
            .map_interval(identity(), 20_480_000, 20_500_000)
            .unwrap()
            .end_us(),
        20_500_000
    );
    window
        .append_retained(spans[5], 19_500_000, &cancel)
        .unwrap();
    assert_eq!(window.spans.front(), Some(&spans[4]));
    assert_eq!(window.spans.back(), Some(&spans[5]));
    assert_eq!(window.coverage_start_us, Some(19_500_000));
    assert_eq!(
        snapshot
            .map_interval(identity(), 9_500_000, 20_500_000)
            .unwrap()
            .start_us(),
        9_500_000
    );
}

#[test]
fn retained_logical_watermark_refuses_old_queries_even_inside_the_kept_source_row() {
    let cancel = ResolverCancellation::default();
    let spans = packet_spans(identity(), 8_000, 0, &[32_768; 6]);
    for operation in 0..3 {
        let mut window = retained_large_window(&spans, &cancel);
        assert!(
            window
                .snapshot(identity(), 9_500_000, 20_480_000, &cancel)
                .is_ok()
        );
        let result = match operation {
            0 => window
                .map_interval(identity(), 1, 9_499_999, 9_500_001, &cancel)
                .map(|_| ()),
            1 => window
                .snapshot(identity(), 9_499_999, 9_500_001, &cancel)
                .map(|_| ()),
            _ => window
                .snapshot_with_pending(identity(), 9_499_999, 20_500_000, spans[5], &cancel)
                .map(|_| ()),
        };
        assert_eq!(result, Err(MediaError::InvalidAudio));
        assert_failed(&window);
    }
}

#[test]
fn retained_watermark_cannot_rewind_or_exceed_the_pending_observed_end() {
    let cancel = ResolverCancellation::default();
    let spans = packet_spans(identity(), 8_000, 0, &[32_768; 6]);
    for required in [9_499_999, spans[5].sample_end_us() + 1] {
        let mut window = retained_large_window(&spans, &cancel);
        assert_eq!(
            window.append_retained(spans[5], required, &cancel),
            Err(MediaError::InvalidAudio)
        );
        assert_failed(&window);
    }
    let mut exact_end = retained_large_window(&spans, &cancel);
    exact_end
        .append_retained(spans[5], spans[5].sample_end_us(), &cancel)
        .unwrap();
    assert_eq!(exact_end.spans.len(), 1);
    assert_eq!(exact_end.spans.front(), Some(&spans[5]));
    assert_eq!(exact_end.coverage_start_us, Some(spans[5].sample_end_us()));
}

#[test]
fn retained_watermark_may_advance_into_a_true_gap_but_never_restores_old_prefix() {
    let cancel = ResolverCancellation::default();
    let spans = spans_at(&[0, 20_000_000, 40_000_000]);
    let mut window = window_with(&spans[..1]);
    window
        .append_retained(spans[1], 19_000_000, &cancel)
        .unwrap();
    assert_eq!(window.spans.len(), 1);
    assert_eq!(window.coverage_start_us, Some(19_000_000));
    let next = window
        .snapshot_with_pending(identity(), 40_000_000, 40_010_000, spans[2], &cancel)
        .unwrap();
    assert_eq!(next.source_run_id(), 3);
    window
        .append_retained(spans[2], 39_000_000, &cancel)
        .unwrap();
    assert_eq!(window.coverage_start_us, Some(39_000_000));
    assert_eq!(
        window
            .snapshot(identity(), 38_999_999, 40_000_001, &cancel)
            .err(),
        Some(MediaError::InvalidAudio)
    );
    assert_failed(&window);
}

#[test]
fn retained_append_keeps_independent_logical_duration_and_source_row_caps() {
    let cancel = ResolverCancellation::default();
    let spans = packet_spans(identity(), 8_000, 0, &[32_768; 3]);
    let mut exact = window_with(&spans[..2]);
    exact.append_retained(spans[2], 288_000, &cancel).unwrap();
    assert_eq!(exact.spans.len(), 3);
    assert_eq!(exact.coverage_start_us, Some(288_000));
    let mut too_long = window_with(&spans[..2]);
    assert_eq!(
        too_long.append_retained(spans[2], 287_999, &cancel),
        Err(MediaError::Overloaded)
    );
    assert_failed(&too_long);
    let tiny = packet_spans(identity(), 16_000, 0, &[1; MAX_CLOCK_WINDOW_SPANS + 1]);
    let mut too_many = window_with(&tiny[..MAX_CLOCK_WINDOW_SPANS]);
    assert_eq!(
        too_many.append_retained(tiny[MAX_CLOCK_WINDOW_SPANS], 1, &cancel),
        Err(MediaError::Overloaded)
    );
    assert_failed(&too_many);
    let mut pruned = window_with(&tiny[..MAX_CLOCK_WINDOW_SPANS]);
    pruned
        .append_retained(
            tiny[MAX_CLOCK_WINDOW_SPANS],
            tiny[0].sample_end_us(),
            &cancel,
        )
        .unwrap();
    assert_eq!(pruned.spans.len(), MAX_CLOCK_WINDOW_SPANS);
}

#[test]
fn retained_append_rejects_bad_source_and_cancel_without_preserving_a_watermark() {
    let spans = spans_at(&[0, 10_000]);
    let cancel = ResolverCancellation::default();
    let mut invalid = spans[1];
    invalid.sequence += 1;
    let mut window = window_with(&spans[..1]);
    assert_eq!(
        window.append_retained(invalid, 0, &cancel),
        Err(MediaError::InvalidAudio)
    );
    assert_failed(&window);
    for cancel_owner in [true, false] {
        let owner = ResolverCancellation::default();
        let supplied = ResolverCancellation::default();
        let mut window = DecodedClockWindow::new(identity(), &owner).unwrap();
        window.append_retained(spans[0], 0, &supplied).unwrap();
        if cancel_owner {
            owner.cancel();
        } else {
            supplied.cancel();
        }
        assert_eq!(
            window.append_retained(spans[1], 10_000, &supplied),
            Err(MediaError::Cancelled)
        );
        assert_failed(&window);
    }
}
