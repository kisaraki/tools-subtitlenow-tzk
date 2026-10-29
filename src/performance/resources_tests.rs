use super::*;
use std::time::Duration;
fn sample(at: Instant, cpu: u64) -> ProcessSample {
    ProcessSample {
        at,
        created_100ns: 1,
        cpu_100ns: cpu,
        private_commit_bytes: 100,
        working_set_bytes: 200,
        peak_working_set_bytes: 300,
        handles: 5,
    }
}
#[test]
fn phase10_resources_first_sample_is_not_a_zero_cpu_measurement() {
    let sample = sample(Instant::now(), 100);
    assert!(
        ProcessSampler::delta(None, sample, 4)
            .unwrap()
            .cpu_core_ppm
            .is_none()
    );
}
#[test]
fn phase10_resources_cpu_delta_preserves_multicore_and_machine_normalization() {
    let at = Instant::now();
    let reading = ProcessSampler::delta(
        Some(sample(at, 0)),
        sample(at + Duration::from_secs(1), 20_000_000),
        4,
    )
    .unwrap();
    assert_eq!(reading.cpu_core_ppm, Some(2_000_000));
    assert_eq!(reading.cpu_machine_ppm, Some(500_000));
}
#[test]
fn phase10_resources_invalid_time_identity_and_counter_do_not_become_zero() {
    let at = Instant::now();
    for mut next in [
        sample(at, 100),
        sample(at - Duration::from_millis(1), 100),
        sample(at + Duration::from_secs(1), 99),
        sample(at + Duration::from_secs(1), 100),
    ] {
        if next.cpu_100ns == 100 && next.at > at {
            next.created_100ns = 2;
        }
        assert!(ProcessSampler::delta(Some(sample(at, 100)), next, 4).is_err());
    }
}
#[test]
fn phase10_resources_native_current_process_query_is_read_only_and_numeric() {
    #[cfg(windows)]
    {
        let probe = crate::win32::performance::NativeProcessProbe::current();
        let reading = probe.sample().unwrap();
        assert!(reading.private_commit_bytes > 0);
        assert!(reading.peak_working_set_bytes >= reading.working_set_bytes);
        assert!(reading.handles > 0);
    }
}
