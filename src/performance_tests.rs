use super::*;
#[test]
fn phase10_empty_measurements_are_not_zero_latency_claims() {
    let snapshot = DurationHistogram::default().snapshot();
    assert_eq!(snapshot.samples, 0);
    assert_eq!(snapshot.p50_upper_ns, None);
    assert_eq!(snapshot.p95_upper_ns, None);
    assert_eq!(snapshot.max_ns, None);
}
#[test]
fn phase10_bucket_boundaries_cover_every_power_and_extreme_without_overflow() {
    assert_eq!(bucket(u64::MAX), DURATION_BUCKETS - 1);
    assert_eq!(upper(DURATION_BUCKETS - 1), u64::MAX);
    for index in 0..DURATION_BUCKETS {
        let high = upper(index);
        assert_eq!(bucket(high), index);
        if index > 0 {
            assert_eq!(bucket(upper(index - 1) + 1), index);
        }
    }
    for exponent in 0..64 {
        let value = 1u64 << exponent;
        for value in [value - 1, value, value.saturating_add(1)] {
            assert!(upper(bucket(value)) >= value);
        }
    }
}
#[test]
fn phase10_percentiles_use_nearest_rank_and_explicit_upper_bounds() {
    let histogram = DurationHistogram::default();
    for value in 1..=100 {
        assert!(histogram.record(Duration::from_nanos(value)));
    }
    let snapshot = histogram.snapshot();
    assert_eq!(snapshot.samples, 100);
    assert_eq!(snapshot.p50_upper_ns, Some(51));
    assert_eq!(snapshot.p95_upper_ns, Some(95));
    assert_eq!(snapshot.max_ns, Some(100));
    assert!(!snapshot.saturated);
    let zeros = DurationHistogram::default();
    assert!(zeros.record(Duration::ZERO));
    assert_eq!(zeros.snapshot().p50_upper_ns, Some(0));
    assert_eq!(zeros.snapshot().max_ns, Some(0));
}
#[test]
fn phase10_rejects_future_or_unrepresentable_time_without_clamping_to_valid_samples() {
    let histogram = DurationHistogram::default();
    assert!(!histogram.record(Duration::MAX));
    assert!(!histogram.record_since(Instant::now() + Duration::from_secs(1)));
    let snapshot = histogram.snapshot();
    assert_eq!(snapshot.samples, 0);
    assert_eq!(snapshot.rejected_samples, 2);
    assert_eq!(snapshot.p95_upper_ns, None);
}
#[test]
fn phase10_saturation_is_explicit_and_cannot_wrap_into_fresh_percentiles() {
    let histogram = DurationHistogram::default();
    histogram.buckets[0].store(u64::MAX, Ordering::Relaxed);
    histogram.record(Duration::ZERO);
    let snapshot = histogram.snapshot();
    assert_eq!(snapshot.samples, u64::MAX);
    assert!(snapshot.saturated);
    assert_eq!(snapshot.p50_upper_ns, None);
    assert_eq!(snapshot.p95_upper_ns, None);
    histogram.buckets[1].store(1, Ordering::Relaxed);
    assert!(histogram.snapshot().saturated);
}
#[test]
fn phase10_concurrent_recording_keeps_numeric_state_bounded_and_counted() {
    let histogram = std::sync::Arc::new(DurationHistogram::default());
    let handles: Vec<_> = (0..4)
        .map(|index| {
            let histogram = histogram.clone();
            std::thread::spawn(move || {
                for _ in 0..1000 {
                    histogram.record(Duration::from_nanos(index));
                }
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }
    let snapshot = histogram.snapshot();
    assert_eq!(snapshot.samples, 4000);
    assert_eq!(snapshot.max_ns, Some(3));
    assert_eq!(snapshot.p50_upper_ns, Some(1));
    assert_eq!(snapshot.p95_upper_ns, Some(3));
    assert!(std::mem::size_of::<DurationHistogram>() < 4096);
}
#[test]
fn phase10_timer_drop_records_once_and_sessions_do_not_share_distribution() {
    let first = DurationHistogram::default();
    {
        let _timer = first.start();
    }
    assert_eq!(first.snapshot().samples, 1);
    assert_eq!(DurationHistogram::default().snapshot().samples, 0);
}
