use super::*;

#[tokio::test]
async fn remote_probe_drop_keeps_real_owner_and_cpu_capacity_until_started_worker_exits() {
    let endpoint = Endpoint::new(mp3(), Behavior::Normal).await;
    let (mut media, cancel, owners, buffers) = endpoint.open().await;
    let old = media
        .read_range(RemoteRangeRequest::new(identity(), 1, 0, 4).unwrap())
        .await
        .unwrap();
    let lane = cpu();
    let (ready, started) = oneshot::channel();
    let (release, wait) = std::sync::mpsc::sync_channel(1);
    let mut future = Box::pin(run(media, lane.clone(), move |_, _| {
        ready.send(()).unwrap();
        wait.recv_timeout(Duration::from_secs(5)).unwrap();
        Ok(LocalAudioFormat::Wave)
    }));
    tokio::select! { result=&mut future=>panic!("early result: {:?}",result.err()), _=started=>{} }
    drop(future);
    assert!(cancel.is_cancelled());
    assert_eq!(old.bytes().err(), Some(RemoteMediaError::Cancelled));
    assert_eq!(owners.available_permits(), 0);
    assert_eq!(lane.available_permits(), 0);
    assert_eq!(buffers.available_permits(), 3);
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while owners.available_permits() != 1 || lane.available_permits() != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    drop(old);
    assert_eq!(buffers.available_permits(), 4);
    assert_eq!(endpoint.requests.lock().unwrap().len(), 2);
    endpoint.finish().await;
}

#[tokio::test]
async fn remote_probe_explicit_cancel_waits_for_cpu_worker_exit_before_returning() {
    let endpoint = Endpoint::new(mp3(), Behavior::Normal).await;
    let (media, cancel, owners, _) = endpoint.open().await;
    let lane = cpu();
    let (ready, started) = oneshot::channel();
    let (release, wait) = std::sync::mpsc::sync_channel(1);
    let mut future = Box::pin(run(media, lane.clone(), move |_, _| {
        ready.send(()).unwrap();
        wait.recv_timeout(Duration::from_secs(5)).unwrap();
        Ok(LocalAudioFormat::Wave)
    }));
    tokio::select! { result=&mut future=>panic!("early result: {:?}",result.err()), _=started=>{} }
    cancel.cancel();
    assert!(futures_util::poll!(future.as_mut()).is_pending());
    assert_eq!(owners.available_permits(), 0);
    release.send(()).unwrap();
    assert!(matches!(
        future.await,
        Err(RemoteProbeError::Remote(RemoteMediaError::Cancelled))
    ));
    assert_eq!(owners.available_permits(), 1);
    assert_eq!(lane.available_permits(), 1);
    endpoint.finish().await;
}

#[tokio::test]
async fn remote_probe_worker_panic_is_typed_and_reclaims_all_capacity() {
    let endpoint = Endpoint::new(mp3(), Behavior::Normal).await;
    let (media, cancel, owners, buffers) = endpoint.open().await;
    let lane = cpu();
    let result = run(media, lane.clone(), |_, _| {
        panic!("explicit synthetic probe panic")
    })
    .await;
    assert!(matches!(
        result,
        Err(RemoteProbeError::Media(MediaError::WorkerFailed))
    ));
    assert!(cancel.is_cancelled());
    assert_eq!(owners.available_permits(), 1);
    assert_eq!(lane.available_permits(), 1);
    assert_eq!(buffers.available_permits(), 4);
    endpoint.finish().await;
}

#[test]
fn remote_probe_public_entry_outside_runtime_fails_before_worker_or_network() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (endpoint, media, cancel, owners) = runtime.block_on(async {
        let endpoint = Endpoint::new(mp3(), Behavior::Normal).await;
        let (media, cancel, owners, _) = endpoint.open().await;
        (endpoint, media, cancel, owners)
    });
    let mut future = std::pin::pin!(probe_remote_audio(media));
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    assert!(matches!(
        future.as_mut().poll(&mut context),
        std::task::Poll::Ready(Err(RemoteProbeError::Remote(
            RemoteMediaError::RuntimeUnavailable
        )))
    ));
    assert!(cancel.is_cancelled());
    assert_eq!(owners.available_permits(), 1);
    assert_eq!(endpoint.requests.lock().unwrap().len(), 1);
    runtime.block_on(endpoint.finish());
}

#[tokio::test]
async fn remote_probe_public_entry_uses_shared_cpu_lane_on_current_thread_runtime() {
    let endpoint = Endpoint::new(mp3(), Behavior::Normal).await;
    let (media, cancel, owners, _) = endpoint.open().await;
    let runtime = Handle::current();
    let result = tokio::task::spawn_blocking(move || {
        // Serialize only the production lane fixture; private tests inject lanes.
        let _serial = super::super::super::work::TEST_OWNERS.lock().unwrap();
        runtime.block_on(probe_remote_audio(media))
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result.format(), RemoteAudioFormat::Mp3);
    assert!(!cancel.is_cancelled());
    drop(result);
    assert_eq!(owners.available_permits(), 1);
    endpoint.finish().await;
}
