//! Owned synthetic containers/temporary paths only; no user media or keys.
use super::*;
use crate::{audio::capture::SessionId, media::local_audio_format::mp4_subtitle_fixture};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};
fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(827),
        generation: 1,
    }
}
fn selected() -> Mp4SubtitleSelection {
    Mp4SubtitleSelection::new(3, TextLanguage::TaiwanTraditional).unwrap()
}
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}
#[test]
fn mp4_discovery_unpolled_drop_and_cpu_saturation_do_not_open_input() {
    let _serial = super::super::work::TEST_OWNERS.lock().unwrap();
    let cancel = ResolverCancellation::default();
    let future = discover_local_mp4_subtitle(
        MediaInput::remote("https://example.invalid/private").unwrap(),
        identity(),
        cancel.clone(),
    );
    drop(future);
    assert!(cancel.is_cancelled());
    let lane = super::super::work::workers();
    let held = lane
        .clone()
        .try_acquire_many_owned(lane.available_permits() as u32)
        .unwrap();
    let cancel = ResolverCancellation::default();
    assert_eq!(
        runtime().block_on(discover_local_mp4_subtitle(
            MediaInput::remote("https://example.invalid/private").unwrap(),
            identity(),
            cancel.clone()
        )),
        Err(MediaError::Overloaded)
    );
    assert!(cancel.is_cancelled());
    drop(held);
}
#[test]
fn embedded_mp4_subtitle_reads_owned_file_and_retains_exact_readonly_pin() {
    let _serial = super::super::work::TEST_OWNERS.lock().unwrap();
    let runtime = runtime();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("synthetic.data");
    let bytes = mp4_subtitle_fixture();
    std::fs::write(&path, &bytes).unwrap();
    let cancel = ResolverCancellation::default();
    let doc = runtime
        .block_on(extract_local_mp4_subtitle(
            MediaInput::local(path.clone()).unwrap(),
            identity(),
            selected(),
            cancel.clone(),
        ))
        .unwrap();
    assert!(!cancel.is_cancelled());
    assert_eq!(doc.media_byte_len(), bytes.len() as u64);
    assert_eq!(doc.original().identity, identity());
    assert_eq!(doc.original().cues[1].text.as_ref(), "第二句 😀");
    assert!(std::fs::OpenOptions::new().write(true).open(&path).is_err());
    assert!(std::fs::remove_file(&path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    drop(doc);
    assert!(std::fs::OpenOptions::new().write(true).open(&path).is_ok());
    std::fs::remove_file(&path).unwrap();
}
#[test]
fn embedded_mp4_subtitle_error_and_cancel_release_input_without_provider_or_fallback() {
    let _serial = super::super::work::TEST_OWNERS.lock().unwrap();
    let runtime = runtime();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("invalid.mp4");
    std::fs::write(&path, b"not an MP4").unwrap();
    let cancel = ResolverCancellation::default();
    assert!(matches!(
        runtime.block_on(extract_local_mp4_subtitle(
            MediaInput::local(path.clone()).unwrap(),
            identity(),
            selected(),
            cancel.clone()
        )),
        Err(MediaError::UnsupportedMedia)
    ));
    assert!(cancel.is_cancelled());
    std::fs::remove_file(&path).unwrap();
    let cancel = ResolverCancellation::default();
    cancel.cancel();
    assert!(matches!(
        runtime.block_on(extract_local_mp4_subtitle(
            MediaInput::local(path).unwrap(),
            identity(),
            selected(),
            cancel
        )),
        Err(MediaError::Cancelled)
    ));
    assert!(matches!(
        runtime.block_on(extract_local_mp4_subtitle(
            MediaInput::remote("https://example.invalid/media").unwrap(),
            identity(),
            selected(),
            ResolverCancellation::default()
        )),
        Err(MediaError::UnsupportedUrl)
    ));
}
#[test]
fn embedded_mp4_subtitle_unpolled_drop_cancels_without_opening_source() {
    let cancel = ResolverCancellation::default();
    let future = extract_local_mp4_subtitle(
        MediaInput::remote("https://example.invalid/media").unwrap(),
        identity(),
        selected(),
        cancel.clone(),
    );
    drop(future);
    assert!(cancel.is_cancelled());
    assert!(matches!(
        Mp4SubtitleSelection::new(0, TextLanguage::TaiwanTraditional),
        Err(MediaError::InvalidInput)
    ));
}
#[tokio::test]
async fn embedded_mp4_subtitle_rejects_invalid_identity_and_saturated_worker_before_operation() {
    for id in [
        StreamIdentity {
            session: SessionId(0),
            generation: 1,
        },
        StreamIdentity {
            session: SessionId(1),
            generation: 0,
        },
    ] {
        assert_eq!(
            run_with(
                id,
                ResolverCancellation::default(),
                Arc::new(Semaphore::new(1)),
                |_| -> Result<(), MediaError> { panic!("must not run") }
            )
            .await,
            Err(MediaError::InvalidInput)
        );
    }
    assert_eq!(
        run_with(
            identity(),
            ResolverCancellation::default(),
            Arc::new(Semaphore::new(0)),
            |_| -> Result<(), MediaError> { panic!("must not run") }
        )
        .await,
        Err(MediaError::Overloaded)
    );
}
#[tokio::test]
async fn embedded_mp4_subtitle_worker_panic_is_typed_and_capacity_recovers() {
    let lane = Arc::new(Semaphore::new(1));
    let cancel = ResolverCancellation::default();
    assert_eq!(
        run_with(
            identity(),
            cancel.clone(),
            lane.clone(),
            |_| -> Result<(), MediaError> { panic!("synthetic worker failure") }
        )
        .await,
        Err(MediaError::WorkerFailed)
    );
    assert!(cancel.is_cancelled());
    assert_eq!(lane.available_permits(), 1);
    assert_eq!(
        run_with(identity(), ResolverCancellation::default(), lane, |_| Ok(
            42
        ))
        .await,
        Ok(42)
    );
}
#[tokio::test]
async fn embedded_mp4_subtitle_future_drop_retains_capacity_until_worker_really_exits() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gated-owned-input.mp4");
    std::fs::write(&path, mp4_subtitle_fixture()).unwrap();
    let worker_path = path.clone();
    let lane = Arc::new(Semaphore::new(1));
    let cancel = ResolverCancellation::default();
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (exited_tx, exited_rx) = tokio::sync::oneshot::channel();
    let observed = Arc::new(AtomicBool::new(false));
    let worker_observed = observed.clone();
    let future = run_with(identity(), cancel.clone(), lane.clone(), move |scope| {
        let pin = open_media(&MediaInput::local(worker_path).unwrap(), scope).unwrap();
        entered_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        worker_observed.store(scope.is_cancelled(), Ordering::SeqCst);
        drop(pin);
        exited_tx.send(()).unwrap();
        Ok(())
    });
    let mut future = Box::pin(future);
    tokio::select! { _ = &mut future => panic!("must stay pending"), _ = entered_rx => {} }
    assert_eq!(lane.available_permits(), 0);
    assert!(std::fs::OpenOptions::new().write(true).open(&path).is_err());
    drop(future);
    assert!(cancel.is_cancelled());
    assert!(std::fs::remove_file(&path).is_err());
    assert_eq!(
        run_with(
            identity(),
            ResolverCancellation::default(),
            lane.clone(),
            |_| Ok(())
        )
        .await,
        Err(MediaError::Overloaded)
    );
    release_tx.send(()).unwrap();
    exited_rx.await.unwrap();
    let permit = tokio::time::timeout(Duration::from_secs(5), lane.clone().acquire_owned())
        .await
        .unwrap()
        .unwrap();
    drop(permit);
    assert!(observed.load(Ordering::SeqCst));
    std::fs::remove_file(&path).unwrap();
    assert_eq!(
        run_with(
            identity(),
            ResolverCancellation::default(),
            lane,
            |_| Ok(())
        )
        .await,
        Ok(())
    );
}
#[tokio::test]
async fn embedded_mp4_subtitle_providerless_worker_cannot_return_success_after_cancel() {
    let lane = Arc::new(Semaphore::new(1));
    let cancel = ResolverCancellation::default();
    let worker_cancel = cancel.clone();
    assert_eq!(
        run_with(identity(), cancel.clone(), lane, move |_| {
            worker_cancel.cancel();
            Ok(())
        })
        .await,
        Err(MediaError::Cancelled)
    );
    assert!(cancel.is_cancelled());
}
