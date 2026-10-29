//! Explicit synthetic-only probe. No WASAPI, app discovery, cloud or file I/O.
use std::{
    error::Error,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use subtitlenow::audio::capture::{BLOCK_FRAMES, CaptureSession};

const BLOCKS: usize = 50;
const SETTLE: Duration = Duration::from_millis(100);
const LIMIT: Duration = Duration::from_secs(4);

fn run(
    scenario: &str,
    pace: Duration,
    initial_consumer_delay: Duration,
) -> Result<(), Box<dyn Error>> {
    let done = Arc::new(AtomicBool::new(false));
    let producer_done = done.clone();
    let mut session = CaptureSession::start(move |mut sink| {
        sink.started();
        for index in 0..BLOCKS {
            if sink.cancelled() {
                break;
            }
            sink.push_pcm(
                None,
                BLOCK_FRAMES,
                (index * BLOCK_FRAMES) as u64,
                Some(index as u64 * 200_000),
                false,
            )?;
            if !pace.is_zero() {
                thread::sleep(pace);
            }
        }
        producer_done.store(true, Ordering::Release);
        while !sink.cancelled() {
            thread::park_timeout(Duration::from_millis(1));
        }
        Ok(())
    })?;
    let started = Instant::now();
    thread::sleep(initial_consumer_delay);
    let mut completion = None;
    let mut settled = false;
    let mut delivered = 0u64;
    while started.elapsed() < LIMIT {
        while session.try_next().is_some() {
            delivered += 1;
        }
        if done.load(Ordering::Acquire) {
            let at = completion.get_or_insert_with(Instant::now);
            if at.elapsed() >= SETTLE {
                settled = true;
                break;
            }
        }
        thread::sleep(Duration::from_millis(1));
    }
    let completed = done.load(Ordering::Acquire) && settled;
    session.stop()?;
    if !completed {
        return Err("synthetic producer deadline exceeded".into());
    }
    let metrics = session.metrics();
    let timings = metrics.timings.ok_or("DEBUG timing is unavailable")?;
    let distribution = |value: subtitlenow::performance::DurationDistribution| {
        serde_json::json!({
            "samples": value.samples, "rejected_samples": value.rejected_samples,
            "saturated": value.saturated, "p50_upper_ns": value.p50_upper_ns,
            "p95_upper_ns": value.p95_upper_ns, "max_ns": value.max_ns,
        })
    };
    println!(
        "{}",
        serde_json::json!({
            "scenario": scenario, "synthetic_only": true, "profile": if cfg!(debug_assertions) {"debug"} else {"release"},
            "captured_frames": metrics.captured_frames, "delivered_blocks": delivered,
            "dropped_frames": metrics.dropped_frames, "stale_frames": metrics.stale_frames,
            "raw_queue_observed_high_water": metrics.raw_queue_observed_high_water,
            "normalized_queue_observed_high_water": metrics.normalized_queue_observed_high_water,
            "sink_call": distribution(timings.sink_call), "delivered_age": distribution(timings.delivered_age),
        })
    );
    Ok(())
}
fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(std::io::stderr)
        .try_init()
        .map_err(|_| "diagnostic logger initialization failed")?;
    run(
        "synthetic_steady_20ms",
        Duration::from_millis(20),
        Duration::ZERO,
    )?;
    run(
        "synthetic_burst_slow_consumer",
        Duration::ZERO,
        Duration::from_millis(100),
    )
}
