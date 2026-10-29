//! Synthetic PCM only. No native capture, cloud, keys or media files.
use super::*;
use crate::{
    audio::capture::{AudioStamp, SessionId},
    resolver::window::WINDOW_FRAMES,
};
use std::{
    sync::{
        Barrier,
        atomic::AtomicBool,
        mpsc::{Receiver, SyncSender, sync_channel},
    },
    time::Duration,
};

fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(81),
        generation: 2,
    }
}
fn block(seq: u64) -> NormalizedAudio {
    NormalizedAudio {
        stamp: AudioStamp {
            session: identity().session,
            sequence: seq,
            frame_position: Some(960 + seq * 960),
            qpc_100ns: None,
            received: Instant::now(),
            discontinuity: false,
        },
        frames: BLOCK_FRAMES,
        samples: [0.25; BLOCK_FRAMES],
    }
}
fn wait(mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !predicate() {
        assert!(Instant::now() < deadline, "worker timed out");
        thread::sleep(Duration::from_millis(1));
    }
}
fn feed(
    worker: &ResolverAudioWorker,
    start: u64,
    count: u64,
    mut adjust: impl FnMut(&mut NormalizedAudio),
) {
    for seq in start..start + count {
        let before = worker.metrics().processed_blocks;
        let mut audio = block(seq);
        adjust(&mut audio);
        worker.input().try_push(audio).unwrap();
        wait(|| worker.metrics().processed_blocks > before);
    }
}
fn gated() -> (ResolverAudioWorker, Receiver<()>, SyncSender<()>) {
    let (entered_tx, entered) = sync_channel(1);
    let (release, release_rx) = sync_channel(1);
    let once = AtomicBool::new(false);
    let worker = ResolverAudioWorker::start(identity(), move || {
        if !once.swap(true, Ordering::SeqCst) {
            entered_tx.send(()).unwrap();
            // Test panic/drop disconnects this channel; a failed assertion cannot
            // leave the owner's join waiting forever on a test-only barrier.
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        }
    })
    .unwrap();
    (worker, entered, release)
}

#[test]
fn actual_fir_windows_keep_timing_interval_and_release_owned_pcm_on_stop() {
    let mut worker = ResolverAudioWorker::new(identity()).unwrap();
    assert_eq!(worker.metrics().worker_count, 1);
    feed(&worker, 0, 600, |_| {});
    let first = worker.latest();
    let window = first.window.as_ref().unwrap();
    assert_eq!(window.samples().len(), WINDOW_FRAMES);
    assert_eq!(window.stamp.sequence, 599);
    assert_eq!(window.stamp.identity, identity());
    assert_eq!(window.stamp.start_us, Some((960 - 63) * 1_000_000 / 48000));
    let mut reference = StreamingResampler::new(identity().session, 16000).unwrap();
    let expected: Vec<i16> = (0..600)
        .flat_map(|seq| {
            let output = reference.process(&block(seq)).unwrap();
            output.samples[..output.frames].to_vec()
        })
        .collect();
    assert_eq!(window.samples(), expected);
    let weak = Arc::downgrade(window);
    drop(first);
    feed(&worker, 600, 149, |_| {});
    assert_eq!(
        worker.current().window.as_ref().unwrap().stamp.sequence,
        599
    );
    feed(&worker, 749, 1, |_| {});
    assert_eq!(
        worker.current().window.as_ref().unwrap().stamp.sequence,
        749
    );
    assert!(weak.upgrade().is_none());
    let last = worker.latest();
    let weak = Arc::downgrade(last.window.as_ref().unwrap());
    drop(last);
    let input = worker.input();
    worker.stop().unwrap();
    worker.stop().unwrap();
    assert_eq!(worker.metrics().worker_count, 0);
    assert_eq!(worker.metrics().queue_depth, 0);
    assert!(weak.upgrade().is_none());
    assert_eq!(input.try_push(block(750)), Err(ResolverError::Cancelled));
}

#[test]
fn ingress_gap_clears_latest_window_and_requires_twelve_new_seconds() {
    let mut worker = ResolverAudioWorker::new(identity()).unwrap();
    feed(&worker, 0, 600, |audio| audio.stamp.frame_position = None);
    assert!(
        worker
            .latest()
            .window
            .as_ref()
            .unwrap()
            .stamp
            .start_us
            .is_none()
    );
    // Missing seq 600: reset is published by admission, before the CPU sees seq 601.
    feed(&worker, 601, 599, |audio| {
        audio.samples.fill(0.1);
        audio.stamp.frame_position = None;
    });
    let reset = worker.latest();
    assert_eq!(reset.epoch, 1);
    assert_eq!(reset.error, Some(ResolverError::AudioDiscontinuity));
    assert!(reset.window.is_none());
    feed(&worker, 1200, 1, |audio| {
        audio.samples.fill(0.1);
        audio.stamp.frame_position = None;
    });
    let recovered = worker.latest();
    assert_eq!(recovered.epoch, 1); // Reset persists even when mailbox coalesces.
    let window = recovered.window.as_ref().unwrap();
    assert_eq!(window.stamp.sequence, 1200);
    assert!(window.stamp.start_us.is_none());
    assert!(window.samples().iter().all(|sample| sample.abs() < 4000));
}

#[test]
fn saturation_discards_backlog_and_active_old_epoch_cannot_publish() {
    let (mut worker, entered, release) = gated();
    worker.input().try_push(block(0)).unwrap();
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    for seq in 1..=50 {
        worker.input().try_push(block(seq)).unwrap();
        assert!(worker.metrics().queue_depth <= AUDIO_QUEUE_BLOCKS);
    }
    assert!(worker.metrics().dropped_blocks >= 48);
    assert_eq!(worker.latest().epoch, 4);
    release.send(()).unwrap();
    wait(|| worker.metrics().queue_depth == 0 && worker.metrics().processed_blocks >= 3);
    assert!(worker.metrics().dropped_blocks >= 49); // Also the gated old active block.
    assert!(worker.latest().window.is_none());
    assert_eq!(worker.metrics().resets, 4);
}

#[test]
fn queued_stale_audio_resets_without_requiring_another_input_packet() {
    let (mut worker, entered, release) = gated();
    worker.input().try_push(block(0)).unwrap();
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    thread::sleep(MAX_AUDIO_AGE + Duration::from_millis(30));
    release.send(()).unwrap();
    wait(|| worker.metrics().processed_blocks == 1);
    let stale = worker.latest();
    assert_eq!(stale.epoch, 1);
    assert_eq!(stale.error, Some(ResolverError::Stale));
    assert!(stale.window.is_none());
    feed(&worker, 1, 1, |_| {});
    assert_eq!(worker.metrics().resets, 1);
}

#[test]
fn wrong_source_and_old_sequence_do_not_reset_current_audio_but_bad_input_does() {
    let mut worker = ResolverAudioWorker::new(identity()).unwrap();
    feed(&worker, 0, 1, |_| {});
    let input = worker.input();
    let mut foreign = block(1);
    foreign.stamp.session = SessionId(82);
    assert_eq!(input.try_push(foreign), Err(ResolverError::InvalidData));
    assert_eq!(input.try_push(block(0)), Err(ResolverError::Stale));
    assert_eq!(worker.metrics().resets, 0);
    let mut bad = block(1);
    bad.frames = BLOCK_FRAMES + 1;
    assert_eq!(input.try_push(bad), Err(ResolverError::InvalidData));
    let mut nan = block(1);
    nan.samples[0] = f32::NAN;
    assert_eq!(input.try_push(nan), Err(ResolverError::InvalidData));
    let mut old = block(1);
    old.stamp.received = Instant::now() - Duration::from_secs(1);
    assert_eq!(input.try_push(old), Err(ResolverError::Stale));
    assert!(worker.latest().window.is_none());
    // Its timestamp precedes the admitted stream: reject it without resetting again.
    assert_eq!(worker.metrics().resets, 2);
    feed(&worker, 1, 1, |audio| audio.stamp.discontinuity = true);
    assert_eq!(worker.metrics().resets, 3);
    feed(&worker, 2, 1, |audio| {
        audio.stamp.frame_position = Some(99000)
    });
    assert_eq!(worker.metrics().resets, 4);
    let mut idle = ResolverAudioWorker::new(identity()).unwrap();
    let mut stale = block(0);
    stale.stamp.received = Instant::now() - Duration::from_secs(1);
    assert_eq!(idle.input().try_push(stale), Err(ResolverError::Stale));
    assert_eq!(idle.latest().error, Some(ResolverError::Stale));
    assert_eq!(idle.metrics().resets, 1);
}

#[test]
fn panic_is_typed_and_owner_drop_rejects_surviving_input_handles() {
    let mut worker =
        ResolverAudioWorker::start(identity(), || panic!("synthetic audio worker panic")).unwrap();
    let input = worker.input();
    input.try_push(block(0)).unwrap();
    wait(|| worker.metrics().worker_count == 0);
    assert_eq!(worker.latest().error, Some(ResolverError::WorkerPanicked));
    drop(worker);
    assert_eq!(input.try_push(block(1)), Err(ResolverError::Cancelled));
    assert!(input.0.queue.is_empty());
    assert!(input.0.output.borrow().window.is_none());
}

#[test]
fn stop_and_submit_races_leave_no_pending_pcm_or_live_worker() {
    for _ in 0..20 {
        let mut worker = ResolverAudioWorker::new(identity()).unwrap();
        let input = worker.input();
        let barrier = Arc::new(Barrier::new(2));
        let b = Arc::clone(&barrier);
        let producer = thread::spawn(move || {
            b.wait();
            input.try_push(block(0))
        });
        barrier.wait();
        worker.stop().unwrap();
        assert!(matches!(
            producer.join().unwrap(),
            Ok(()) | Err(ResolverError::Cancelled)
        ));
        assert_eq!(worker.metrics().queue_depth, 0);
        assert_eq!(worker.metrics().worker_count, 0);
        assert!(worker.current().window.is_none());
    }
}
