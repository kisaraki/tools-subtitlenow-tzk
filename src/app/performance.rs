//! Numeric diagnostics only. No file sink or content-bearing payload.
use crate::audio::capture::{CaptureMetrics, SessionId};
pub(crate) fn report_resources(
    stage: &'static str,
    sampler: &mut crate::performance::resources::ProcessSampler,
) {
    match sampler.sample() {
        Ok(reading) => tracing::debug!(
            stage,
            cpu_core_ppm = reading.cpu_core_ppm,
            cpu_machine_ppm = reading.cpu_machine_ppm,
            private_commit_bytes = reading.sample.private_commit_bytes,
            working_set_bytes = reading.sample.working_set_bytes,
            peak_working_set_bytes = reading.sample.peak_working_set_bytes,
            handles = reading.sample.handles,
            "程序資源觀測"
        ),
        Err(error) => tracing::warn!(stage,error=%error,"程序資源量測無法取得"),
    }
}
pub(crate) fn report_live(metrics: &crate::performance::live::LiveDiagnostics) {
    use crate::performance::live::{Activity, Count, Latency};
    for stage in Latency::ALL {
        let distribution = metrics.distribution(stage);
        tracing::debug!(session=metrics.identity().session.0,stage=?stage,samples=distribution.samples,p50_upper_ns=distribution.p50_upper_ns,p95_upper_ns=distribution.p95_upper_ns,max_ns=distribution.max_ns,rejected_samples=distribution.rejected_samples,saturated=distribution.saturated,"同 Session 管線延遲");
    }
    for stage in Activity::ALL {
        let gauge = metrics.gauge(stage);
        tracing::debug!(session=metrics.identity().session.0,stage=?stage,observed_current=gauge.current,observed_high_water=gauge.observed_high_water,"管線並行與佇列觀測");
    }
    for reason in Count::ALL {
        tracing::debug!(session=metrics.identity().session.0,reason=?reason,count=metrics.count(reason),"管線淘汰與診斷計數");
    }
}

pub(crate) fn report_capture(identity: SessionId, metrics: CaptureMetrics) {
    tracing::debug!(
        session = identity.0,
        owned_capture_workers = metrics.workers_running,
        raw_queue_observed_high_water = metrics.raw_queue_observed_high_water,
        normalized_queue_observed_high_water = metrics.normalized_queue_observed_high_water,
        capture_timing_enabled = metrics.timings.is_some(),
        "音訊效能觀測"
    );
    if let Some(timings) = metrics.timings {
        for (stage, distribution) in [
            ("sink_call", timings.sink_call),
            ("delivered_age", timings.delivered_age),
        ] {
            tracing::debug!(
                session = identity.0,
                stage,
                samples = distribution.samples,
                rejected_samples = distribution.rejected_samples,
                saturated = distribution.saturated,
                p50_upper_ns = distribution.p50_upper_ns,
                p95_upper_ns = distribution.p95_upper_ns,
                max_ns = distribution.max_ns,
                "音訊延遲分布（區間上界，非端到端字幕延遲）"
            );
        }
    }
}
