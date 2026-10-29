use super::*;
fn sink() -> CaptureSink {
    let (state, _) = watch::channel(CaptureState::Starting);
    CaptureSink {
        shared: Arc::new(Shared {
            resource_probe: OnceLock::new(),
            raw: ArrayQueue::new(RING_BLOCKS),
            normalized: ArrayQueue::new(RING_BLOCKS),
            cancelled: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            counters: Counters {
                timings: Some(Box::new(CaptureTimings::default())),
                ..Counters::default()
            },
        }),
        state,
        session: SessionId(99_501),
        sequence: 0,
    }
}
#[test]
fn phase10_raw_peak_survives_drain_and_overload_counts_real_evictions() {
    let mut sink = sink();
    for index in 0..RING_BLOCKS + 4 {
        sink.push_pcm(None, 1, index as u64, None, false).unwrap();
    }
    while sink.shared.raw.pop().is_some() {}
    assert_eq!(
        sink.shared.counters.raw_high_water.load(Ordering::Relaxed),
        RING_BLOCKS
    );
    assert_eq!(sink.shared.counters.dropped.load(Ordering::Relaxed), 4);
    assert_eq!(
        sink.shared
            .counters
            .timings
            .as_ref()
            .unwrap()
            .sink_call
            .snapshot()
            .samples,
        (RING_BLOCKS + 4) as u64
    );
    assert_eq!(
        self::sink()
            .shared
            .counters
            .raw_high_water
            .load(Ordering::Relaxed),
        0
    );
}
#[test]
fn phase10_normalized_peak_and_rejected_sink_calls_have_separate_meanings() {
    let mut sink = sink();
    for sequence in 1..=RING_BLOCKS as u64 + 2 {
        sink.push_normalized(NormalizedAudio {
            stamp: AudioStamp {
                session: sink.session,
                sequence,
                frame_position: Some(sequence * BLOCK_FRAMES as u64),
                qpc_100ns: None,
                received: Instant::now(),
                discontinuity: false,
            },
            frames: BLOCK_FRAMES,
            samples: [0.0; BLOCK_FRAMES],
        })
        .unwrap();
    }
    assert!(sink.push_pcm(Some(&[0]), 1, 0, None, false).is_err());
    assert_eq!(
        sink.shared
            .counters
            .normalized_high_water
            .load(Ordering::Relaxed),
        RING_BLOCKS
    );
    assert_eq!(
        sink.shared.counters.dropped.load(Ordering::Relaxed),
        (2 * BLOCK_FRAMES) as u64
    );
    assert_eq!(
        sink.shared
            .counters
            .timings
            .as_ref()
            .unwrap()
            .sink_call
            .snapshot()
            .samples,
        RING_BLOCKS as u64 + 3
    );
}
#[test]
fn phase10_delivery_age_is_measured_only_at_real_delivery_and_retained_after_stop() {
    let (ready, received) = std::sync::mpsc::sync_channel(1);
    let mut session = CaptureSession::start_pipeline_measured(
        move |mut sink| {
            sink.started();
            for sequence in 1..=3 {
                sink.push_normalized(NormalizedAudio {
                    stamp: AudioStamp {
                        session: sink.session_id(),
                        sequence,
                        frame_position: Some(sequence),
                        qpc_100ns: None,
                        received: Instant::now(),
                        discontinuity: false,
                    },
                    frames: 1,
                    samples: [0.0; BLOCK_FRAMES],
                })?;
            }
            ready.send(()).unwrap();
            while !sink.cancelled() {
                thread::park_timeout(Duration::from_millis(1));
            }
            Ok(())
        },
        None::<fn(&Shared)>,
        true,
    )
    .unwrap();
    received.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(session.metrics().timings.unwrap().delivered_age.samples, 0);
    for _ in 0..3 {
        assert!(session.try_next().is_some());
    }
    let metrics = session.metrics();
    assert_eq!(metrics.normalized_queue_depth, 0);
    assert_eq!(metrics.normalized_queue_observed_high_water, 3);
    assert_eq!(metrics.timings.unwrap().delivered_age.samples, 3);
    session.stop().unwrap();
    assert_eq!(session.metrics().timings.unwrap().delivered_age.samples, 3);
    assert!(session.try_next().is_none());
}
#[test]
fn phase10_disabled_timing_is_unmeasured_and_stale_input_does_not_create_queue_peaks() {
    let mut sink = sink();
    sink.push_pcm_received(
        None,
        1,
        0,
        None,
        false,
        Instant::now() - MAX_AUDIO_AGE - Duration::from_millis(1),
    )
    .unwrap();
    assert_eq!(
        sink.shared.counters.raw_high_water.load(Ordering::Relaxed),
        0
    );
    let mut session = CaptureSession::start_pipeline_measured(
        |sink| {
            while !sink.cancelled() {
                thread::park_timeout(Duration::from_millis(1));
            }
            Ok(())
        },
        None::<fn(&Shared)>,
        false,
    )
    .unwrap();
    assert!(session.metrics().timings.is_none());
    session.stop().unwrap();
}
