//! Synthetic local inputs and controlled completion races, no cloud or keys.
use super::*;
use crate::media::embedded_mp4::{Mp4SubtitleLanguage, Mp4SubtitleTrack};
use std::time::{Duration, Instant};
fn info(id: u32) -> Mp4SubtitleTracks {
    let mut tracks = Mp4SubtitleTracks::default();
    tracks
        .push(Mp4SubtitleTrack {
            track_id: id,
            language: Mp4SubtitleLanguage::Chinese,
        })
        .unwrap();
    tracks
}
fn terminal(owner: &mut PreparedDiscovery) -> DiscoveryState {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let state = owner.poll();
        if !state.busy() {
            return state;
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
}
#[test]
fn mp4_discovery_stop_retires_success_and_rejects_restart_until_real_worker_exit() {
    let mut owner = PreparedDiscovery::default();
    let (started, entered) = mpsc::sync_channel(1);
    let (release, wait) = mpsc::sync_channel(1);
    owner
        .start_with(move |identity, scope| {
            started.send((identity, scope)).unwrap();
            wait.recv().unwrap();
            Ok(info(3)) // Simulate stale success even after cancellation.
        })
        .unwrap();
    let (first, scope) = entered.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(owner.poll(), DiscoveryState::Scanning);
    owner.stop();
    assert!(scope.is_cancelled());
    assert_eq!(owner.poll(), DiscoveryState::Stopping);
    assert_eq!(
        owner.start_with(|_, _| panic!("must not start")),
        Err(MediaError::Overloaded)
    );
    release.send(()).unwrap();
    assert_eq!(terminal(&mut owner), DiscoveryState::Cancelled);
    let (sent, received) = mpsc::sync_channel(1);
    owner
        .start_with(move |identity, _| {
            sent.send(identity).unwrap();
            Ok(info(7))
        })
        .unwrap();
    assert_ne!(
        first,
        received.recv_timeout(Duration::from_secs(5)).unwrap()
    );
    assert_eq!(terminal(&mut owner), DiscoveryState::Found(info(7)));
}
#[test]
fn mp4_discovery_buffered_completion_cannot_publish_after_stop() {
    let mut owner = PreparedDiscovery::default();
    owner.start_with(|_, _| Ok(info(3))).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !owner.worker.as_ref().unwrap().thread.is_finished() {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    owner.stop();
    assert_eq!(owner.poll(), DiscoveryState::Cancelled);
}
#[test]
fn mp4_discovery_preserves_typed_failure_and_recovers_from_worker_panic() {
    let mut owner = PreparedDiscovery::default();
    owner
        .start_with(|_, cancel| {
            cancel.cancel();
            Err(MediaError::UnsupportedMedia)
        })
        .unwrap();
    assert_eq!(
        terminal(&mut owner),
        DiscoveryState::Failed(MediaError::UnsupportedMedia)
    );
    owner
        .start_with(|_, _| panic!("synthetic discovery panic"))
        .unwrap();
    assert_eq!(
        terminal(&mut owner),
        DiscoveryState::Failed(MediaError::WorkerFailed)
    );
    owner.start_with(|_, _| Ok(info(3))).unwrap();
    assert_eq!(terminal(&mut owner), DiscoveryState::Found(info(3)));
    owner.stop();
    assert_eq!(owner.state(), DiscoveryState::Idle);
}
#[cfg(windows)]
#[test]
fn mp4_discovery_real_owner_reads_metadata_and_releases_source_without_ready() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("synthetic.data");
    let bytes = crate::media::local_audio_format::mp4_subtitle_fixture();
    std::fs::write(&path, &bytes).unwrap();
    let mut owner = PreparedDiscovery::default();
    owner
        .start(MediaInput::local(path.clone()).unwrap())
        .unwrap();
    assert_eq!(terminal(&mut owner), DiscoveryState::Found(info(3)));
    assert!(std::fs::OpenOptions::new().write(true).open(&path).is_ok());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(std::fs::read_dir(fixture.path()).unwrap().count(), 1);
    owner
        .start(MediaInput::remote("https://example.invalid/private").unwrap())
        .unwrap();
    assert_eq!(
        terminal(&mut owner),
        DiscoveryState::Failed(MediaError::UnsupportedUrl)
    );
}
#[test]
fn mp4_discovery_drop_cancels_and_joins_owned_work() {
    let mut owner = PreparedDiscovery::default();
    let (started, entered) = mpsc::sync_channel(1);
    let (finished, exited) = mpsc::sync_channel(1);
    owner
        .start_with(move |_, cancel| {
            started.send(cancel.clone()).unwrap();
            while !cancel.is_cancelled() {
                std::thread::yield_now();
            }
            finished.send(()).unwrap();
            Ok(info(3))
        })
        .unwrap();
    let scope = entered.recv_timeout(Duration::from_secs(5)).unwrap();
    drop(owner);
    assert!(scope.is_cancelled());
    exited.try_recv().unwrap();
}
#[cfg(windows)]
#[test]
fn mp4_multitrack_discovery_owner_returns_all_choices_without_a_preferred_track() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("synthetic.data");
    let bytes = crate::media::local_audio_format::mp4_multi_subtitle_fixture(
        &[crate::resolver::contracts::TextLanguage::TaiwanTraditional; 8],
    );
    std::fs::write(&path, &bytes).unwrap();
    let mut owner = PreparedDiscovery::default();
    owner
        .start(MediaInput::local(path.clone()).unwrap())
        .unwrap();
    let DiscoveryState::Found(tracks) = terminal(&mut owner) else {
        panic!("missing tracks")
    };
    assert_eq!(tracks.len(), 8);
    assert!(owner.state().label().contains("請從清單明確選擇"));
    assert_eq!(tracks.get(7).unwrap().track_id, 10);
    assert!(std::fs::OpenOptions::new().write(true).open(&path).is_ok());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    owner.stop();
    assert_eq!(owner.state(), DiscoveryState::Idle);
}
