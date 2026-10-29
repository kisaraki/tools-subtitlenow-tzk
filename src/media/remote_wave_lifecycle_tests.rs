use super::*;
use std::time::Duration;

#[tokio::test]
async fn remote_wave_network_wait_releases_cpu_and_overload_does_not_queue() {
    let source = Source::new(wave(16000, 1, &samples(100000, 1))).await;
    let (mut decoder, cancel) = source.decoder().await;
    let (started, resume) = source.pause();
    let mut work = Box::pin(decoder.read_next());
    tokio::select! {result=&mut work=>panic!("early result {:?}",result.err()),_=started=>{}}
    assert_eq!(source.cpu.available_permits(), 1);
    let held = source.cpu.clone().try_acquire_owned().unwrap();
    resume.send(()).unwrap();
    assert!(matches!(
        work.await,
        Err(RemoteWaveError::Media(MediaError::Overloaded))
    ));
    assert!(cancel.is_cancelled());
    assert_eq!(source.pcm.available_permits(), 4);
    assert_eq!(source.buffers.available_permits(), 4);
    drop(held);
    drop(decoder);
    source.finish().await;
}

#[tokio::test]
async fn remote_wave_slow_consumer_retains_pcm_quota_across_decoder_and_source_retirement() {
    let source = Source::new(wave(16000, 1, &samples(200000, 1))).await;
    let (mut decoder, cancel) = source.decoder().await;
    let mut retained = Vec::new();
    for _ in 0..4 {
        retained.push(decoder.read_next().await.unwrap().unwrap());
    }
    assert_eq!(source.pcm.available_permits(), 0);
    let requests = source.rows.lock().unwrap().len();
    assert!(matches!(
        decoder.read_next().await,
        Err(RemoteWaveError::Media(MediaError::Overloaded))
    ));
    assert_eq!(source.rows.lock().unwrap().len(), requests);
    assert!(cancel.is_cancelled());
    assert!(retained.iter().all(|block| block.block().is_err()));
    drop(decoder);
    assert_eq!(source.owners.available_permits(), 1);
    assert_eq!(source.pcm.available_permits(), 0);
    let mut id = identity();
    id.generation += 1;
    let (input, _) = source.input(id).await;
    let mut next = RemoteWaveDecoder::open_with(input, source.cpu.clone(), source.pcm.clone())
        .await
        .unwrap();
    let requests = source.rows.lock().unwrap().len();
    assert!(matches!(
        next.read_next().await,
        Err(RemoteWaveError::Media(MediaError::Overloaded))
    ));
    assert_eq!(source.rows.lock().unwrap().len(), requests);
    drop(next);
    drop(retained);
    assert_eq!(source.pcm.available_permits(), 4);
    source.finish().await;
}

#[tokio::test]
async fn remote_wave_unpolled_open_and_read_drop_cancel_without_new_io() {
    let source = Source::new(wave(16000, 1, &samples(100000, 1))).await;
    let (input, cancel) = source.input(identity()).await;
    let requests = source.rows.lock().unwrap().len();
    drop(RemoteWaveDecoder::open_with(
        input,
        source.cpu.clone(),
        source.pcm.clone(),
    ));
    assert!(cancel.is_cancelled());
    assert_eq!(source.rows.lock().unwrap().len(), requests);
    assert_eq!(source.owners.available_permits(), 1);
    let (mut decoder, cancel) = source.decoder().await;
    let old = decoder.read_next().await.unwrap().unwrap();
    let requests = source.rows.lock().unwrap().len();
    drop(decoder.read_next());
    assert!(cancel.is_cancelled());
    assert!(old.block().is_err());
    assert_eq!(source.rows.lock().unwrap().len(), requests);
    drop(decoder);
    drop(old);
    source.finish().await;
}

#[tokio::test]
async fn remote_wave_inflight_cancel_or_future_drop_closes_connection_and_reclaims_credits() {
    for explicit in [true, false] {
        let source = Source::new(wave(16000, 1, &samples(100000, 1))).await;
        let (mut decoder, cancel) = source.decoder().await;
        let (started, _resume) = source.pause();
        let mut work = Box::pin(decoder.read_next());
        tokio::select! {result=&mut work=>panic!("early result {:?}",result.err()),_=started=>{}}
        assert_eq!(source.cpu.available_permits(), 1);
        assert_eq!(source.pcm.available_permits(), 3);
        if explicit {
            cancel.cancel();
            assert!(matches!(
                work.await,
                Err(RemoteWaveError::Remote(RemoteMediaError::Cancelled))
            ));
        } else {
            drop(work);
        }
        assert!(cancel.is_cancelled());
        assert_eq!(source.pcm.available_permits(), 4);
        assert_eq!(source.buffers.available_permits(), 4);
        assert_eq!(source.cpu.available_permits(), 1);
        drop(decoder);
        assert_eq!(source.owners.available_permits(), 1);
        source.finish().await;
    }
}

#[tokio::test]
async fn remote_wave_drop_during_partial_conversion_releases_cpu_pcm_and_wire_buffers() {
    let source = Source::new(wave(16000, 1, &samples(100000, 1))).await;
    let (mut decoder, cancel) = source.decoder().await;
    let mut work = Box::pin(decoder.read_next());
    tokio::time::timeout(Duration::from_secs(3),async {
        loop {
            tokio::select! {biased;
                result=&mut work=>panic!("conversion completed before observation: {:?}",result.err()),
                _=tokio::task::yield_now()=>{}
            }
            if source.cpu.available_permits()==0 {break;}
        }
    }).await.unwrap();
    drop(work);
    assert!(cancel.is_cancelled());
    assert_eq!(source.cpu.available_permits(), 1);
    assert_eq!(source.pcm.available_permits(), 4);
    assert_eq!(source.buffers.available_permits(), 4);
    drop(decoder);
    source.finish().await;
}

#[tokio::test]
async fn remote_wave_generations_and_cancellation_are_isolated() {
    let old_source = Source::new(wave(16000, 1, &samples(100000, 1))).await;
    let (mut old, cancel) = old_source.decoder().await;
    let old_block = old.read_next().await.unwrap().unwrap();
    let source = Source::new(wave(16000, 1, &samples(100000, 1))).await;
    let mut id = identity();
    id.generation += 1;
    let (input, new_cancel) = source.input(id).await;
    let mut decoder = RemoteWaveDecoder::open_with(input, source.cpu.clone(), source.pcm.clone())
        .await
        .unwrap();
    let mut gate = DecodedAudioGate::new(id, decoder.format()).unwrap();
    assert_eq!(
        gate.validate(old_block.block().unwrap()),
        Err(MediaError::Stale)
    );
    cancel.cancel();
    assert!(old_block.block().is_err());
    assert!(!new_cancel.is_cancelled());
    let block = decoder.read_next().await.unwrap().unwrap();
    assert_eq!(block.block().unwrap().identity(), id);
    assert!(!gate.validate(block.block().unwrap()).unwrap());
    drop(block);
    drop(decoder);
    drop(old_block);
    drop(old);
    source.finish().await;
    old_source.finish().await;
}

#[test]
fn remote_wave_pull_without_runtime_returns_typed_error_before_new_network_io() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (source, mut decoder, cancel) = runtime.block_on(async {
        let source = Source::new(wave(16000, 1, &samples(100000, 1))).await;
        let (decoder, cancel) = source.decoder().await;
        (source, decoder, cancel)
    });
    let requests = source.rows.lock().unwrap().len();
    {
        let mut work = std::pin::pin!(decoder.read_next());
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        assert!(matches!(
            work.as_mut().poll(&mut context),
            std::task::Poll::Ready(Err(RemoteWaveError::Remote(
                RemoteMediaError::RuntimeUnavailable
            )))
        ));
    }
    assert!(cancel.is_cancelled());
    assert_eq!(source.rows.lock().unwrap().len(), requests);
    drop(decoder);
    runtime.block_on(source.finish());
}
