//! Explicit RAM-only native soak. Resource metadata is sampled; PCM is never logged.
use crate::audio::capture::{CaptureState, MAX_AUDIO_AGE, RING_BLOCKS};
use std::{
    mem::size_of,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    time::{Duration, Instant},
};
use windows::Win32::{
    Foundation::{ERROR_NO_MORE_FILES, FILETIME, HANDLE},
    System::{Diagnostics::ToolHelp::*, ProcessStatus::*, Threading::*},
};

const SOAK_DURATION: Duration = Duration::from_secs(30 * 60);
const WARMUP: Duration = Duration::from_secs(10);
const SAMPLE_INTERVAL: Duration = Duration::from_secs(60);
const NO_PCM_TIMEOUT: Duration = Duration::from_secs(5);
// One second often falls below GetProcessTimes accounting resolution. Keep
// this explicit performance sample bounded and distinct from the 30-minute soak.
const PERFORMANCE_STEADY_WINDOW: Duration = Duration::from_secs(10);
// Warmed processes have fixed queues and workers. These are regression alarms, not
// product working-set limits or an assertion about every Windows driver/version.
const PRIVATE_GROWTH_ALLOWANCE: usize = 8 * 1024 * 1024;
const HANDLE_GROWTH_ALLOWANCE: u32 = 4;
const THREAD_GROWTH_ALLOWANCE: u32 = 2;

#[test]
#[ignore = "real loopback clock metadata from this test's silent source; no user audio or cloud"]
fn native_process_capture_clock_metadata_is_continuous() {
    check_clock_metadata(false);
}

#[test]
#[ignore = "real loopback of this test's synthetic tone/silence transitions; no user audio or cloud"]
fn native_process_capture_active_clock_metadata_is_continuous() {
    check_clock_metadata(true);
}

fn check_clock_metadata(active: bool) {
    let _serial = super::capture::ACTIVATION_TEST_LOCK.lock().unwrap();
    let mut fixture = super::tests::FixtureProcess::start();
    if active {
        fixture.play(true);
    }
    let mut session = super::tests::start_fixture_capture(&fixture);
    let started = Instant::now();
    let mut previous = None;
    let mut previous_qpc = None;
    let mut blocks = 0;
    let mut later_flags = 0;
    let mut position_gaps = 0;
    let mut unknown = 0;
    let target_blocks = if active { 1000 } else { 100 };
    while blocks < target_blocks {
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "{:?}",
            session.state()
        );
        if let Some(block) = session.try_next() {
            if blocks < 8 || (block.stamp.discontinuity && later_flags < 8) {
                println!(
                    "CLOCK seq={} frames={} position={:?} qpc={:?} previous_qpc={previous_qpc:?} discontinuity={}",
                    block.stamp.sequence,
                    block.frames,
                    block.stamp.frame_position,
                    block.stamp.qpc_100ns,
                    block.stamp.discontinuity
                );
            }
            if blocks > 0 {
                later_flags += usize::from(block.stamp.discontinuity);
                position_gaps += usize::from(previous != block.stamp.frame_position);
            }
            unknown += usize::from(
                block.stamp.qpc_100ns.is_none() || block.stamp.frame_position.is_none(),
            );
            previous = block
                .stamp
                .frame_position
                .and_then(|position| position.checked_add(block.frames as u64));
            previous_qpc = block.stamp.qpc_100ns;
            blocks += 1;
            if active && blocks % 100 == 0 && blocks < target_blocks {
                fixture.play(blocks % 200 == 0);
            }
        } else {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    session.stop().unwrap();
    assert_eq!(session.metrics().workers_running, 0);
    println!(
        "CLOCK blocks={blocks} later_flags={later_flags} position_gaps={position_gaps} unknown={unknown}"
    );
    assert_eq!(unknown, 0);
    assert_eq!(position_gaps, 0);
    assert_eq!(later_flags, 0);
}

fn own(handle: HANDLE) -> OwnedHandle {
    // SAFETY: callers pass a successful owning Win32 handle, transferred exactly once.
    unsafe { OwnedHandle::from_raw_handle(handle.0) }
}
fn raw(handle: &OwnedHandle) -> HANDLE {
    HANDLE(handle.as_raw_handle())
}
fn no_more(error: windows::core::Error) {
    assert_eq!(
        error.code(),
        windows::core::HRESULT::from_win32(ERROR_NO_MORE_FILES.0)
    );
}
fn helper_pid(fixture: u32) -> u32 {
    // SAFETY: read-only system snapshot, owned until enumeration finishes.
    let snapshot = own(unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }.unwrap());
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut found = None;
    // SAFETY: correct initialized struct size and writable entry; snapshot stays live.
    unsafe { Process32FirstW(raw(&snapshot), &mut entry) }.unwrap();
    loop {
        if entry.th32ParentProcessID == std::process::id() && entry.th32ProcessID != fixture {
            assert!(
                found.replace(entry.th32ProcessID).is_none(),
                "unexpected second test child"
            );
        }
        // SAFETY: same valid snapshot and entry as above.
        if let Err(error) = unsafe { Process32NextW(raw(&snapshot), &mut entry) } {
            no_more(error);
            break;
        }
    }
    found.expect("capture helper must be a child of this test")
}

#[derive(Clone, Copy, Debug)]
struct Sample {
    private_bytes: usize,
    handles: u32,
    threads: u32,
    cpu_100ns: u64,
}
fn sample(handle: &OwnedHandle, pid: u32) -> Sample {
    let mut memory = PROCESS_MEMORY_COUNTERS_EX {
        cb: size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    let mut handles = 0;
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: owned query/read process handle, correctly sized counters and four writable
    // FILETIMEs. EX shares the documented base prefix; cb requests the complete EX struct.
    unsafe {
        GetProcessMemoryInfo(
            raw(handle),
            (&mut memory as *mut PROCESS_MEMORY_COUNTERS_EX).cast(),
            memory.cb,
        )
        .unwrap();
        GetProcessHandleCount(raw(handle), &mut handles).unwrap();
        GetProcessTimes(
            raw(handle),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
        .unwrap();
    }
    // SAFETY: read-only thread snapshot, no thread handles or suspension.
    let snapshot = own(unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) }.unwrap());
    let mut entry = THREADENTRY32 {
        dwSize: size_of::<THREADENTRY32>() as u32,
        ..Default::default()
    };
    let mut threads = 0;
    // SAFETY: live snapshot and correctly sized writable entry.
    unsafe { Thread32First(raw(&snapshot), &mut entry) }.unwrap();
    loop {
        if entry.th32OwnerProcessID == pid {
            threads += 1;
        }
        // SAFETY: same valid snapshot and entry as above.
        if let Err(error) = unsafe { Thread32Next(raw(&snapshot), &mut entry) } {
            no_more(error);
            break;
        }
    }
    let ticks =
        |time: FILETIME| (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
    Sample {
        private_bytes: memory.PrivateUsage,
        handles,
        threads,
        cpu_100ns: ticks(kernel) + ticks(user),
    }
}
fn process(pid: u32) -> OwnedHandle {
    // SAFETY: only this test process or its discovered helper, query/read/sync permissions.
    own(unsafe {
        OpenProcess(
            PROCESS_QUERY_INFORMATION | PROCESS_VM_READ | PROCESS_SYNCHRONIZE,
            false,
            pid,
        )
    }
    .unwrap())
}
fn check_growth(current: Sample, baseline: Sample) {
    assert!(
        current.private_bytes <= baseline.private_bytes + PRIVATE_GROWTH_ALLOWANCE,
        "private bytes grew: {baseline:?} -> {current:?}"
    );
    assert!(
        current.handles <= baseline.handles + HANDLE_GROWTH_ALLOWANCE,
        "handles grew: {baseline:?} -> {current:?}"
    );
    assert!(
        current.threads <= baseline.threads + THREAD_GROWTH_ALLOWANCE,
        "threads grew: {baseline:?} -> {current:?}"
    );
}

#[test]
#[ignore = "30-minute real capture; default playback device, RAM-only silence, no audio files"]
fn native_process_capture_thirty_minute_soak() {
    let _serial = super::capture::ACTIVATION_TEST_LOCK.lock().unwrap();
    let fixture = super::tests::FixtureProcess::start();
    let mut session = super::tests::start_fixture_capture(&fixture);
    let began = Instant::now();
    let mut last_pcm = began;
    let mut frames = 0_u64;
    let mut last_sequence = 0;
    let mut max_age = Duration::ZERO;
    let mut max_age_at_read_start = Duration::ZERO;
    let mut max_queue = 0;
    while began.elapsed() < WARMUP {
        while let Some(block) = session.try_next() {
            last_pcm = Instant::now();
            frames += block.frames as u64;
        }
        assert!(
            !matches!(session.state(), CaptureState::Stopped(_)),
            "{:?}",
            session.state()
        );
        assert!(last_pcm.elapsed() < NO_PCM_TIMEOUT);
        std::thread::sleep(Duration::from_millis(2));
    }
    let parent = process(std::process::id());
    let helper_id = helper_pid(fixture.pid());
    let helper = process(helper_id);
    let parent_baseline = sample(&parent, std::process::id());
    let helper_baseline = sample(&helper, helper_id);
    println!(
        "SOAK baseline: parent_pid={}, helper_pid={helper_id}, parent={parent_baseline:?}, helper={helper_baseline:?}",
        std::process::id()
    );
    let mut next_sample = WARMUP + SAMPLE_INTERVAL;
    let mut previous_time = began.elapsed();
    let mut previous_parent = parent_baseline;
    let mut previous_helper = helper_baseline;
    let mut samples = 0;
    let mut max_cpu_parent = 0.0_f64;
    let mut max_cpu_helper = 0.0_f64;
    while began.elapsed() < SOAK_DURATION {
        loop {
            let read_started = Instant::now();
            let Some(block) = session.try_next() else {
                break;
            };
            assert_eq!(block.stamp.session, session.id());
            assert!(block.stamp.sequence > last_sequence);
            last_sequence = block.stamp.sequence;
            let age = block.stamp.received.elapsed();
            // try_next checks age internally. Windows can deschedule this test after
            // that check: post-return age is an observation, not a realtime guarantee.
            // Reject data already expired at call entry; report both ages and stalls.
            let age_at_read_start = read_started.saturating_duration_since(block.stamp.received);
            assert!(age_at_read_start <= MAX_AUDIO_AGE);
            max_age_at_read_start = max_age_at_read_start.max(age_at_read_start);
            max_age = max_age.max(age);
            assert!(
                block.samples[..block.frames]
                    .iter()
                    .all(|sample| sample.is_finite() && sample.abs() <= 2.0 / 32768.0)
            );
            frames += block.frames as u64;
            last_pcm = Instant::now();
        }
        assert_eq!(session.state(), CaptureState::Running);
        assert!(
            last_pcm.elapsed() < NO_PCM_TIMEOUT,
            "no fresh PCM for five seconds"
        );
        let queue = session.metrics().normalized_queue_depth;
        assert!(queue <= RING_BLOCKS);
        max_queue = max_queue.max(queue);
        if began.elapsed() >= next_sample {
            let parent_sample = sample(&parent, std::process::id());
            let helper_sample = sample(&helper, helper_id);
            check_growth(parent_sample, parent_baseline);
            check_growth(helper_sample, helper_baseline);
            let now = began.elapsed();
            let cpu = |current: Sample, previous: Sample| {
                (current.cpu_100ns - previous.cpu_100ns) as f64
                    / 100_000.0
                    / (now - previous_time).as_secs_f64()
            };
            let cpu_parent = cpu(parent_sample, previous_parent);
            let cpu_helper = cpu(helper_sample, previous_helper);
            max_cpu_parent = max_cpu_parent.max(cpu_parent);
            max_cpu_helper = max_cpu_helper.max(cpu_helper);
            println!(
                "SOAK elapsed={}s frames={frames} queue_max={max_queue} age_max_ms={:.3} parent={parent_sample:?} helper={helper_sample:?} cpu_one_core_pct={cpu_parent:.3}/{cpu_helper:.3}",
                now.as_secs(),
                max_age.as_secs_f64() * 1000.0
            );
            previous_time = now;
            previous_parent = parent_sample;
            previous_helper = helper_sample;
            samples += 1;
            next_sample += SAMPLE_INTERVAL;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    let parent_final = sample(&parent, std::process::id());
    let helper_final = sample(&helper, helper_id);
    check_growth(parent_final, parent_baseline);
    check_growth(helper_final, helper_baseline);
    assert!(samples >= 29 && frames > 0);
    let stopping = Instant::now();
    session.stop().unwrap();
    let stop = stopping.elapsed();
    assert_eq!(session.metrics().normalized_queue_depth, 0);
    assert!(session.try_next().is_none());
    // SAFETY: live owned synchronization handle for this test's helper only.
    assert_eq!(
        unsafe { WaitForSingleObject(raw(&helper), 0) },
        windows::Win32::Foundation::WAIT_OBJECT_0
    );
    println!(
        "SOAK complete: elapsed={}s samples={samples} frames={frames} parent_final={parent_final:?} helper_final={helper_final:?} queue_max={max_queue} age_at_call_max={max_age_at_read_start:?} observed_age_max={max_age:?} cpu_one_core_pct_max={max_cpu_parent:.3}/{max_cpu_helper:.3} stop={stop:?}",
        began.elapsed().as_secs()
    );
}

#[test]
#[ignore = "real native capture with a one-second consumer stall; RAM-only silence"]
fn native_process_capture_slow_consumer_stays_bounded() {
    let _serial = super::capture::ACTIVATION_TEST_LOCK.lock().unwrap();
    let timing = std::env::var("SUBTITLENOW_PERF_TIMING").as_deref() != Ok("0");
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(if timing {
            tracing::Level::DEBUG
        } else {
            tracing::Level::INFO
        })
        .with_writer(std::io::sink)
        .finish();
    let _subscriber = tracing::subscriber::set_default(subscriber);
    let fixture = super::tests::FixtureProcess::start();
    use crate::performance::resources::ProcessSampler;
    let mut parent = ProcessSampler::new(super::performance::NativeProcessProbe::current());
    parent.sample().unwrap();
    std::thread::sleep(Duration::from_secs(1));
    let idle = parent.sample().unwrap();
    let mut session = super::tests::start_fixture_capture(&fixture);
    let deadline = Instant::now() + NO_PCM_TIMEOUT;
    while session.try_next().is_none() {
        assert!(Instant::now() < deadline && !matches!(session.state(), CaptureState::Stopped(_)));
        std::thread::sleep(Duration::from_millis(2));
    }
    let mut helper = ProcessSampler::new(super::performance::NativeProcessProbe::owned(process(
        helper_pid(fixture.pid()),
    )));
    parent.sample().unwrap();
    helper.sample().unwrap();
    let steady_start = Instant::now();
    let mut steady_blocks = 0u64;
    while steady_start.elapsed() < PERFORMANCE_STEADY_WINDOW {
        while session.try_next().is_some() {
            steady_blocks += 1;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    let steady_parent = parent.sample().unwrap();
    let steady_helper = helper.sample().unwrap();
    println!(
        "PHASE10_NATIVE {}",
        serde_json::json!({"timing":timing,"steady_window_ms":PERFORMANCE_STEADY_WINDOW.as_millis(),"steady_blocks":steady_blocks,"idle_parent_cpu_core_ppm":idle.cpu_core_ppm,"steady_parent_cpu_core_ppm":steady_parent.cpu_core_ppm,"steady_helper_cpu_core_ppm":steady_helper.cpu_core_ppm,"parent_private_commit_bytes":steady_parent.sample.private_commit_bytes,"helper_private_commit_bytes":steady_helper.sample.private_commit_bytes,"parent_handles":steady_parent.sample.handles,"helper_handles":steady_helper.sample.handles,"helper_probe_attached":session.resource_probe().is_some()})
    );
    // The receiver must continue while this consumer deliberately stops reading.
    std::thread::sleep(Duration::from_secs(1));
    let metrics = session.metrics();
    assert!(metrics.normalized_queue_depth > 0 && metrics.normalized_queue_depth <= RING_BLOCKS);
    assert!(metrics.dropped_frames > 0);
    let block = session
        .try_next()
        .expect("fresh audio must supersede stalled audio");
    assert_eq!(block.stamp.session, session.id());
    assert!(block.stamp.received.elapsed() <= MAX_AUDIO_AGE + Duration::from_millis(5));
    println!(
        "SLOW consumer: metrics={metrics:?}, resumed_age={:?}",
        block.stamp.received.elapsed()
    );
    session.stop().unwrap();
    let final_metrics = session.metrics();
    println!(
        "PHASE10_NATIVE_FINAL {}",
        serde_json::json!({"timing":timing,"workers_running":final_metrics.workers_running,"captured_frames":final_metrics.captured_frames,"dropped_frames":final_metrics.dropped_frames,"stale_frames":final_metrics.stale_frames,"normalized_observed_high_water":final_metrics.normalized_queue_observed_high_water,"sink_samples":final_metrics.timings.map(|t|t.sink_call.samples),"sink_p50_upper_ns":final_metrics.timings.and_then(|t|t.sink_call.p50_upper_ns),"sink_p95_upper_ns":final_metrics.timings.and_then(|t|t.sink_call.p95_upper_ns),"sink_max_ns":final_metrics.timings.and_then(|t|t.sink_call.max_ns),"delivery_p50_upper_ns":final_metrics.timings.and_then(|t|t.delivered_age.p50_upper_ns),"delivery_p95_upper_ns":final_metrics.timings.and_then(|t|t.delivered_age.p95_upper_ns),"delivery_max_ns":final_metrics.timings.and_then(|t|t.delivered_age.max_ns)})
    );
    assert_eq!(session.metrics().workers_running, 0);
    assert_eq!(session.metrics().normalized_queue_depth, 0);
    assert!(session.try_next().is_none());
}

#[test]
#[ignore = "terminates only this test's active helper and verifies explicit recovery; RAM-only silence"]
fn native_process_capture_helper_crash_requires_new_session() {
    let _serial = super::capture::ACTIVATION_TEST_LOCK.lock().unwrap();
    let fixture = super::tests::FixtureProcess::start();
    let mut session = super::tests::start_fixture_capture(&fixture);
    let deadline = Instant::now() + NO_PCM_TIMEOUT;
    while session.try_next().is_none() {
        assert!(Instant::now() < deadline && !matches!(session.state(), CaptureState::Stopped(_)));
        std::thread::sleep(Duration::from_millis(2));
    }
    let old_session = session.id();
    let source = process(fixture.pid());
    // SAFETY: this PID was identified as our test's sole non-fixture child, then pinned.
    let helper = own(unsafe {
        OpenProcess(
            PROCESS_TERMINATE | PROCESS_SYNCHRONIZE,
            false,
            helper_pid(fixture.pid()),
        )
    }
    .unwrap());
    // SAFETY: terminate only our test-owned capture helper, never a user application.
    unsafe { TerminateProcess(raw(&helper), 99) }.unwrap();
    while !matches!(session.state(), CaptureState::Stopped(_)) {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(
        session.stop(),
        Err(crate::audio::capture::CaptureError::WorkerExited)
    );
    assert!(session.try_next().is_none());
    // SAFETY: read-only liveness check on our fixture; it must survive helper failure.
    assert_eq!(
        unsafe { WaitForSingleObject(raw(&source), 0) },
        windows::Win32::Foundation::WAIT_TIMEOUT
    );
    let mut replacement = super::tests::start_fixture_capture(&fixture);
    assert_ne!(replacement.id(), old_session);
    let deadline = Instant::now() + NO_PCM_TIMEOUT;
    loop {
        if let Some(block) = replacement.try_next() {
            assert_eq!(block.stamp.session, replacement.id());
            break;
        }
        assert!(
            Instant::now() < deadline && !matches!(replacement.state(), CaptureState::Stopped(_))
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    replacement.stop().unwrap();
    println!("HELPER crash: WorkerExited, source alive, replacement session isolated");
}

#[test]
#[ignore = "manual application fixture; requires explicit SUBTITLENOW_TEST_SOURCE_PID and SUBTITLENOW_TEST_EXPECTED_HZ"]
fn manual_application_source_tone_is_verified() {
    let pid: u32 = std::env::var("SUBTITLENOW_TEST_SOURCE_PID")
        .expect("select the visible test source PID explicitly")
        .parse()
        .unwrap();
    let hz: f64 = std::env::var("SUBTITLENOW_TEST_EXPECTED_HZ")
        .expect("specify the synthetic test frequency")
        .parse()
        .unwrap();
    assert!([0.0, 440.0, 880.0, 1320.0, 1730.0].contains(&hz));
    println!(
        "MANUAL capture source PID={pid}; process tree scope; expected frequency={hz}; RAM-only, no audio logs"
    );
    // Use the production boundary: the parent pins the selected PID and the session
    // helper performs fresh audio-session confirmation before any native client starts.
    let target = super::capture::ProcessTarget::open(pid).unwrap();
    let mut session = crate::audio::capture::CaptureSession::start_normalized(move |sink| {
        super::capture::capture(target, sink)
    })
    .unwrap();
    let samples = super::tests::signal_window(&mut session);
    assert_eq!(samples.len(), 4800, "must receive a complete 100 ms window");
    if hz == 0.0 {
        let peak = samples
            .iter()
            .copied()
            .map(f32::abs)
            .fold(0.0_f32, f32::max);
        println!("MANUAL silence peak={peak:.6}");
        assert!(peak <= 2.0 / 32768.0);
    } else {
        let amplitude = super::tests::tone_amplitude(&samples, hz);
        println!("MANUAL tone {hz} Hz amplitude={amplitude:.6}");
        if std::env::var("SUBTITLENOW_TEST_TONE_ABSENT").as_deref() == Ok("1") {
            assert!(
                amplitude < 0.005,
                "paused synthetic tone must disappear below detection threshold"
            );
        } else {
            assert!(
                amplitude > 0.005,
                "selected source must contain the expected synthetic tone"
            );
        }
    }
    session.stop().unwrap();
    assert!(session.try_next().is_none());
}

#[test]
#[ignore = "three batches of 100 real silent capture restarts; same process, original two-handle tolerance"]
fn native_process_capture_restart_batches_do_not_accumulate_handles() {
    const WARMUP_RUNS: usize = 3;
    const BATCHES: usize = 3;
    const RUNS_PER_BATCH: usize = 100;
    const TRANSIENT_HANDLES: u32 = 2; // same allowance as the original restart test
    let _serial = super::capture::ACTIVATION_TEST_LOCK.lock().unwrap();
    // Exercise Phase 10's optional duplicated helper query handle on every
    // restart, rather than only proving the default INFO path is reclaimed.
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(std::io::sink)
        .finish();
    let _subscriber = tracing::subscriber::set_default(subscriber);
    let fixture = super::tests::FixtureProcess::start();
    let count = || {
        let mut handles = 0;
        // SAFETY: current-process pseudo handle and writable counter; no content read.
        unsafe { GetProcessHandleCount(GetCurrentProcess(), &mut handles) }.unwrap();
        handles
    };
    let restart = || {
        let mut session = super::tests::start_fixture_capture(&fixture);
        let deadline = Instant::now() + NO_PCM_TIMEOUT;
        while session.try_next().is_none() {
            assert!(
                Instant::now() < deadline && !matches!(session.state(), CaptureState::Stopped(_))
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(session.resource_probe().is_some());
        assert!(session.metrics().timings.is_some());
        session.stop().unwrap();
        assert!(session.try_next().is_none());
        assert_eq!(session.metrics().normalized_queue_depth, 0);
    };
    for _ in 0..WARMUP_RUNS {
        restart();
    }
    let baseline = count();

    for batch in 1..=BATCHES {
        for _ in 0..RUNS_PER_BATCH {
            restart();
        }
        let deadline = Instant::now() + NO_PCM_TIMEOUT;
        let mut final_count = count();
        while final_count > baseline + TRANSIENT_HANDLES && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
            final_count = count();
        }
        println!(
            "RESTART batches: completed={} baseline={baseline} handles={final_count}",
            batch * RUNS_PER_BATCH
        );
        assert!(
            final_count <= baseline + TRANSIENT_HANDLES,
            "handles accumulate across restart batches"
        );
    }
}
