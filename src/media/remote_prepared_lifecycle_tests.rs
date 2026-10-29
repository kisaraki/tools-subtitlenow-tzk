use super::*;

#[tokio::test]
async fn remote_prepared_network_cancel_and_future_drop_release_all_source_permits() {
    for drop_future in [false, true] {
        let source = Source::new(wave(16000, 1, &samples(16013, 1))).await;
        let (decoder, cancel) = source.decoder().await;
        let provider = Probe::new(Mode::Good);
        let (started, _resume) = source.pause();
        let mut work = Box::pin(recognize_with(
            decoder,
            SourceLanguage::English,
            PreparedSampleRate::Hz16000,
            &provider,
            source.cpu.clone(),
        ));
        tokio::select! {result=&mut work=>panic!("early {:?}",result.err()),_=started=>{}}
        assert_eq!(source.cpu.available_permits(), 1);
        if drop_future {
            drop(work);
        } else {
            cancel.cancel();
            assert!(work.await.is_err());
        }
        assert!(cancel.is_cancelled());
        assert_eq!(provider.count(), 0);
        assert_released(&source);
        source.finish().await;
    }
}

#[tokio::test]
async fn remote_prepared_provider_cancel_and_future_drop_stop_read_ahead_and_requests() {
    for drop_future in [false, true] {
        let source = Source::new(wave(16000, 1, &samples(16000 * 42, 1))).await;
        let (decoder, cancel) = source.decoder().await;
        let mut provider = Probe::new(Mode::Good);
        provider.hold = Some(Arc::new(Semaphore::new(0)));
        let mut work = Box::pin(recognize_with(
            decoder,
            SourceLanguage::English,
            PreparedSampleRate::Hz16000,
            &provider,
            source.cpu.clone(),
        ));
        tokio::select! {result=&mut work=>panic!("early {:?}",result.err()),_=provider.entered.notified()=>{}}
        let reads = source.rows.lock().unwrap().len();
        if drop_future {
            drop(work);
        } else {
            cancel.cancel();
            assert!(matches!(
                work.await,
                Err(RemotePreparedError::Speech(PreparedSpeechError::Cancelled))
            ));
        }
        assert!(cancel.is_cancelled());
        assert_eq!(provider.active.load(Ordering::SeqCst), 0);
        assert_eq!(source.rows.lock().unwrap().len(), reads);
        assert_released(&source);
        source.finish().await;
    }
}

#[test]
fn remote_prepared_queued_dsp_drop_retains_cpu_and_pcm_until_job_is_reaped() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(async {
        let source = Source::new(wave(16000, 1, &samples(16013, 1))).await;
        let (decoder, cancel) = source.decoder().await;
        let (release, blocked) = std::sync::mpsc::channel();
        let (entered, started) = tokio::sync::oneshot::channel();
        let blocker = tokio::task::spawn_blocking(move || { entered.send(()).unwrap(); blocked.recv().unwrap(); });
        started.await.unwrap();
        let provider = Probe::new(Mode::Good);
        let mut work = Box::pin(recognize_with(decoder, SourceLanguage::English, PreparedSampleRate::Hz16000, &provider, source.cpu.clone()));
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                tokio::select! { biased; result=&mut work=>panic!("early {:?}",result.err()),_=tokio::task::yield_now()=>{} }
                if source.cpu.available_permits()==0 && source.pcm.available_permits()==3 && source.buffers.available_permits()==4 { break; }
            }
        }).await.unwrap();
        // The wire buffer has been freed: conversion is over, and the only
        // blocking thread is occupied, so this is the queued DSP job.
        assert_eq!(provider.count(), 0);
        drop(work);
        assert!(cancel.is_cancelled());
        release.send(()).unwrap();
        blocker.await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            while source.cpu.available_permits()!=1 || source.pcm.available_permits()!=4 { tokio::task::yield_now().await; }
        }).await.unwrap();
        assert_released(&source);
        source.finish().await;
    });
}
