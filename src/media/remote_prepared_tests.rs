//! Real loopback ranges and synthetic PCM; recognizers are explicit test mocks.
use super::*;
use crate::{
    ai::{
        prepared_speech::{
            PreparedChunkStamp, PreparedSpeechCapabilities, PreparedSpeechCue,
            PreparedSpeechFuture, PreparedSpeechRequest, PreparedSpeechResponse,
        },
        speech::SpeechLanguage,
    },
    media::{
        decoded::{DecodedAudioBlock, DecodedAudioFormat},
        prepared_audio::PreparedAudioProcessor,
        remote_http::RemoteMediaError,
        remote_wave::test_source::{Source, identity, samples, wave},
    },
};
use std::{
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::Notify;

#[derive(Clone, Copy)]
enum Mode {
    Good,
    Unsupported,
    Panic,
    Failure,
    Stale,
    Empty,
}
struct Probe {
    mode: Mode,
    expected: Option<Arc<Vec<i16>>>,
    rows: Mutex<Vec<(PreparedChunkStamp, u64, usize)>>,
    completed: Mutex<Vec<u64>>,
    active: AtomicUsize,
    maximum: AtomicUsize,
    capabilities: AtomicUsize,
    entered: Notify,
    hold: Option<Arc<Semaphore>>,
    changed: Option<Arc<crate::media::remote_wave::test_source::Control>>,
}
impl Probe {
    fn new(mode: Mode) -> Self {
        Self {
            mode,
            expected: None,
            rows: Mutex::new(Vec::new()),
            completed: Mutex::new(Vec::new()),
            active: AtomicUsize::new(0),
            maximum: AtomicUsize::new(0),
            capabilities: AtomicUsize::new(0),
            entered: Notify::new(),
            hold: None,
            changed: None,
        }
    }
    fn count(&self) -> usize {
        self.rows.lock().unwrap().len()
    }
}
struct Active<'a>(&'a AtomicUsize);
impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
impl PreparedSpeechRecognizer for Probe {
    fn capabilities(&self) -> PreparedSpeechCapabilities {
        self.capabilities.fetch_add(1, Ordering::SeqCst);
        assert!(
            !matches!(self.mode, Mode::Panic),
            "synthetic capability panic"
        );
        PreparedSpeechCapabilities {
            cancellation: !matches!(self.mode, Mode::Unsupported),
            timestamped_finals: true,
            supported_languages: [true; 5],
            constrained_auto_detection: true,
            sample_rates: [true; 2],
            max_concurrent: 4,
        }
    }
    fn recognize<'a>(
        &'a self,
        request: &'a PreparedSpeechRequest,
        _: ResolverCancellation,
    ) -> PreparedSpeechFuture<'a> {
        Box::pin(async move {
            let chunk = request.chunk();
            let stamp = chunk.stamp();
            assert_eq!(stamp.identity, identity());
            assert_eq!(stamp.run_id, 1);
            assert!(chunk.source_clock().is_none());
            assert!(chunk.samples().len() <= stamp.rate.hz() as usize * 11);
            if let Some(expected) = &self.expected {
                // Endpoint conversion is exact for these fixtures' chunk starts.
                let first =
                    (stamp.audio_start_us * u64::from(stamp.rate.hz()) / 1_000_000) as usize;
                assert_eq!(
                    chunk.samples(),
                    &expected[first..first + chunk.samples().len()]
                );
            }
            self.rows.lock().unwrap().push((
                stamp,
                chunk.audio_end_us(),
                request.previous_source().len(),
            ));
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.maximum.fetch_max(active, Ordering::SeqCst);
            let _active = Active(&self.active);
            self.entered.notify_one();
            if let Some(hold) = &self.hold {
                hold.acquire().await.unwrap().forget();
            }
            // Force parallel completion order to differ from source order.
            tokio::time::sleep(Duration::from_millis(2 * (4 - stamp.sequence % 4))).await;
            self.completed.lock().unwrap().push(stamp.sequence);
            if matches!(self.mode, Mode::Failure) {
                return Err(PreparedSpeechError::Credential);
            }
            if let Some(control) = &self.changed {
                control.changed.store(true, Ordering::Release);
            }
            let mut identity = stamp.identity;
            if matches!(self.mode, Mode::Stale) {
                identity.generation += 1;
            }
            Ok(PreparedSpeechResponse {
                identity,
                sequence: stamp.sequence,
                run_id: stamp.run_id,
                language: Some(SpeechLanguage::English),
                cues: if matches!(self.mode, Mode::Empty) {
                    vec![]
                } else {
                    vec![PreparedSpeechCue {
                        start_us: stamp.body_start_us - stamp.audio_start_us,
                        duration_us: stamp.body_end_us - stamp.body_start_us,
                        text: format!("synthetic {}", stamp.sequence).into(),
                    }]
                },
            })
        })
    }
}
fn assert_released(source: &Source) {
    assert_eq!(source.cpu.available_permits(), 1);
    assert_eq!(source.owners.available_permits(), 1);
    assert_eq!(source.buffers.available_permits(), 4);
    assert_eq!(source.pcm.available_permits(), 4);
}
fn reference(input_rate: u32, channels: u16, pcm: &[i16], rate: PreparedSampleRate) -> Vec<i16> {
    let cancel = ResolverCancellation::default();
    let format = DecodedAudioFormat::new(input_rate, channels).unwrap();
    let mut processor = PreparedAudioProcessor::new(identity(), format, rate, &cancel).unwrap();
    let mut output = Vec::new();
    let mut emit = |block: &crate::media::prepared_audio::PreparedPcmBlock,
                    _: &ResolverCancellation| {
        output.extend_from_slice(block.samples());
        Ok(())
    };
    let bytes = 32768 / usize::from(channels) * usize::from(channels);
    for (index, samples) in pcm.chunks(bytes).enumerate() {
        let frames = index as u64 * (bytes / usize::from(channels)) as u64;
        let block = DecodedAudioBlock::new(
            identity(),
            index as u64 + 1,
            frames * 1_000_000 / u64::from(input_rate),
            format,
            samples.to_vec(),
        )
        .unwrap();
        processor.push(&block, &cancel, &mut emit).unwrap();
    }
    let summary = processor.finish(&cancel, &mut emit).unwrap();
    assert_eq!(summary.gaps(), 0);
    assert_eq!(summary.frames() as usize, output.len());
    output
}

#[tokio::test]
async fn remote_prepared_real_ranges_dsp_overlap_tail_and_order_for_both_asr_rates() {
    for input_rate in [8000, 16000, 44100] {
        for rate in [PreparedSampleRate::Hz16000, PreparedSampleRate::Hz24000] {
            let channels = if input_rate == 44100 { 2 } else { 1 };
            let frames = input_rate as usize * 11 + 13;
            let pcm = samples(frames, channels);
            let expected = reference(input_rate, channels, &pcm, rate);
            let end = expected.len() as u64 * 1_000_000 / u64::from(rate.hz());
            let source = Source::new(wave(input_rate, channels, &pcm)).await;
            let (decoder, cancel) = source.decoder().await;
            let mut provider = Probe::new(Mode::Good);
            provider.expected = Some(Arc::new(expected));
            let result = recognize_with(
                decoder,
                SourceLanguage::English,
                rate,
                &provider,
                source.cpu.clone(),
            )
            .await
            .unwrap();
            assert_eq!(result.identity(), identity());
            let text = result.transcript().unwrap();
            assert_eq!(text.identity(), identity());
            assert_eq!(text.cues().len(), 2);
            for (index, cue) in text.cues().iter().enumerate() {
                assert_eq!(cue.start_us, index as u64 * 10_000_000);
                assert_eq!(&*cue.text, format!("synthetic {}", index + 1));
            }
            let tail = &text.cues()[1];
            assert_eq!(tail.start_us + tail.duration_us, end);
            assert!(!cancel.is_cancelled());
            assert_eq!(source.owners.available_permits(), 0);
            assert_eq!(source.pcm.available_permits(), 4);
            assert_eq!(source.cpu.available_permits(), 1);
            cancel.cancel();
            assert!(matches!(
                result.transcript(),
                Err(RemotePreparedError::Speech(PreparedSpeechError::Cancelled))
            ));
            drop(result);
            assert_released(&source);
            source.finish().await;
        }
    }
}

#[tokio::test]
async fn remote_prepared_batches_pause_read_ahead_release_cpu_and_preserve_context() {
    let source = Source::new(wave(16000, 1, &samples(16000 * 52 + 17, 1))).await;
    let (decoder, cancel) = source.decoder().await;
    let mut provider = Probe::new(Mode::Good);
    let hold = Arc::new(Semaphore::new(0));
    provider.hold = Some(hold.clone());
    let mut work = Box::pin(recognize_with(
        decoder,
        SourceLanguage::English,
        PreparedSampleRate::Hz16000,
        &provider,
        source.cpu.clone(),
    ));
    tokio::select! {result=&mut work=>panic!("early {:?}",result.err()),_=provider.entered.notified()=>{}}
    let reads = source.rows.lock().unwrap().len();
    assert!(source.rows.lock().unwrap().last().unwrap().0 < 44 + 16000 * 52 * 2);
    assert!((1..=4).contains(&provider.count()));
    assert_eq!(source.cpu.available_permits(), 1);
    assert_eq!(source.pcm.available_permits(), 4);
    assert_eq!(source.buffers.available_permits(), 4);
    tokio::select! {result=&mut work=>panic!("early {:?}",result.err()),_=tokio::time::sleep(Duration::from_millis(15))=>{}}
    assert_eq!(source.rows.lock().unwrap().len(), reads);
    hold.add_permits(12);
    let result = work.await.unwrap();
    assert_eq!(result.transcript().unwrap().cues().len(), 6);
    assert!(provider.maximum.load(Ordering::SeqCst) <= MAX_ASR_BATCH_CHUNKS);
    {
        let rows = provider.rows.lock().unwrap();
        assert!(
            rows.iter()
                .filter(|row| row.0.sequence <= 3)
                .all(|row| row.2 == 0)
        );
        assert!(
            rows.iter()
                .filter(|row| row.0.sequence > 3)
                .all(|row| row.2 > 0)
        );
    }
    {
        let completed = provider.completed.lock().unwrap();
        assert!(completed.windows(2).any(|pair| pair[0] > pair[1]));
    }
    drop(result);
    assert!(cancel.is_cancelled());
    assert_released(&source);
    source.finish().await;
}

#[tokio::test]
async fn remote_prepared_final_provider_wait_keeps_original_owner_through_result_drop() {
    let source = Source::new(wave(16000, 1, &samples(16013, 1))).await;
    let (decoder, cancel) = source.decoder().await;
    let mut provider = Probe::new(Mode::Good);
    let hold = Arc::new(Semaphore::new(0));
    provider.hold = Some(hold.clone());
    let mut work = Box::pin(recognize_with(
        decoder,
        SourceLanguage::English,
        PreparedSampleRate::Hz16000,
        &provider,
        source.cpu.clone(),
    ));
    tokio::select! {result=&mut work=>panic!("early {:?}",result.err()),_=provider.entered.notified()=>{}}
    assert_eq!(source.owners.available_permits(), 0);
    assert_eq!(source.cpu.available_permits(), 1);
    assert_eq!(source.pcm.available_permits(), 4);
    hold.add_permits(1);
    let result = work.await.unwrap();
    assert!(!cancel.is_cancelled());
    assert_eq!(source.owners.available_permits(), 0);
    drop(result);
    assert!(cancel.is_cancelled());
    assert_released(&source);
    source.finish().await;
}

#[tokio::test]
async fn remote_prepared_unpolled_public_future_has_no_new_io_or_provider_work() {
    let source = Source::new(wave(16000, 1, &samples(16013, 1))).await;
    let (decoder, cancel) = source.decoder().await;
    let provider = Probe::new(Mode::Panic);
    let reads = source.rows.lock().unwrap().len();
    let work = recognize_remote_wave(
        decoder,
        SourceLanguage::English,
        PreparedSampleRate::Hz16000,
        &provider,
    );
    assert_eq!(provider.capabilities.load(Ordering::SeqCst), 0);
    drop(work);
    assert!(cancel.is_cancelled());
    assert_eq!(source.rows.lock().unwrap().len(), reads);
    assert_released(&source);
    source.finish().await;
}

#[tokio::test]
async fn remote_prepared_capability_failure_and_prior_cancel_precede_new_io() {
    for mode in [Mode::Unsupported, Mode::Panic, Mode::Good] {
        let source = Source::new(wave(16000, 1, &samples(16013, 1))).await;
        let (decoder, cancel) = source.decoder().await;
        let provider = Probe::new(mode);
        let reads = source.rows.lock().unwrap().len();
        if matches!(mode, Mode::Good) {
            cancel.cancel();
        }
        let result = recognize_with(
            decoder,
            SourceLanguage::English,
            PreparedSampleRate::Hz16000,
            &provider,
            source.cpu.clone(),
        )
        .await;
        let expected = match mode {
            Mode::Good => PreparedSpeechError::Cancelled,
            Mode::Panic => PreparedSpeechError::InvalidData,
            _ => PreparedSpeechError::UnsupportedCapability,
        };
        assert!(matches!(result, Err(RemotePreparedError::Speech(error)) if error==expected));
        assert_eq!(provider.count(), 0);
        assert_eq!(source.rows.lock().unwrap().len(), reads);
        assert!(cancel.is_cancelled());
        assert_released(&source);
        source.finish().await;
    }
}

#[tokio::test]
async fn remote_prepared_provider_failure_stale_generation_and_no_speech_have_no_partial_result() {
    for (mode, expected) in [
        (Mode::Failure, PreparedSpeechError::Credential),
        (Mode::Stale, PreparedSpeechError::Stale),
        (Mode::Empty, PreparedSpeechError::NoSpeech),
    ] {
        let source = Source::new(wave(16000, 1, &samples(16013, 1))).await;
        let (decoder, cancel) = source.decoder().await;
        let provider = Probe::new(mode);
        let result = recognize_with(
            decoder,
            SourceLanguage::English,
            PreparedSampleRate::Hz16000,
            &provider,
            source.cpu.clone(),
        )
        .await;
        assert!(matches!(result, Err(RemotePreparedError::Speech(error)) if error==expected));
        assert!(cancel.is_cancelled());
        assert_eq!(provider.active.load(Ordering::SeqCst), 0);
        assert_released(&source);
        source.finish().await;
    }
}

#[tokio::test]
async fn remote_prepared_late_source_change_keeps_typed_error_after_committed_batch() {
    let source = Source::new(wave(16000, 1, &samples(16000 * 42, 1))).await;
    let (decoder, cancel) = source.decoder().await;
    let mut provider = Probe::new(Mode::Good);
    provider.changed = Some(source.control.clone());
    let result = recognize_with(
        decoder,
        SourceLanguage::English,
        PreparedSampleRate::Hz16000,
        &provider,
        source.cpu.clone(),
    )
    .await;
    assert!(matches!(
        result,
        Err(RemotePreparedError::Media(RemoteWaveError::Remote(
            RemoteMediaError::SourceChanged
        )))
    ));
    assert_eq!(provider.count(), 3);
    assert!(cancel.is_cancelled());
    assert_released(&source);
    source.finish().await;
}

#[tokio::test]
async fn remote_prepared_dsp_saturation_is_explicit_and_schedules_no_provider() {
    let source = Source::new(wave(16000, 1, &samples(16013, 1))).await;
    let (decoder, cancel) = source.decoder().await;
    let provider = Probe::new(Mode::Good);
    let result = recognize_with(
        decoder,
        SourceLanguage::English,
        PreparedSampleRate::Hz16000,
        &provider,
        Arc::new(Semaphore::new(0)),
    )
    .await;
    assert!(matches!(
        result,
        Err(RemotePreparedError::Media(RemoteWaveError::Media(
            MediaError::Overloaded
        )))
    ));
    assert_eq!(provider.count(), 0);
    assert!(cancel.is_cancelled());
    assert_released(&source);
    source.finish().await;
}

#[path = "remote_prepared_lifecycle_tests.rs"]
mod lifecycle;

#[tokio::test]
async fn remote_prepared_exact_body_and_short_eof_do_not_lose_tail_or_fabricate_speech() {
    for frames in [1, 160_000, 160_001, 168_000] {
        let source = Source::new(wave(16000, 1, &samples(frames, 1))).await;
        let (decoder, cancel) = source.decoder().await;
        let provider = Probe::new(Mode::Good);
        let result = recognize_with(
            decoder,
            SourceLanguage::English,
            PreparedSampleRate::Hz16000,
            &provider,
            source.cpu.clone(),
        )
        .await
        .unwrap();
        let text = result.transcript().unwrap();
        assert_eq!(text.cues().len(), frames.div_ceil(160_000));
        let tail = text.cues().last().unwrap();
        assert_eq!(
            tail.start_us + tail.duration_us,
            frames as u64 * 1_000_000 / 16000
        );
        drop(result);
        assert!(cancel.is_cancelled());
        assert_released(&source);
        source.finish().await;
    }
    let source = Source::new(wave(192000, 1, &samples(1, 1))).await;
    let (decoder, cancel) = source.decoder().await;
    let provider = Probe::new(Mode::Good);
    let result = recognize_with(
        decoder,
        SourceLanguage::English,
        PreparedSampleRate::Hz16000,
        &provider,
        source.cpu.clone(),
    )
    .await;
    assert!(matches!(
        result,
        Err(RemotePreparedError::Media(RemoteWaveError::Media(
            MediaError::InsufficientAudio
        )))
    ));
    assert_eq!(provider.count(), 0);
    assert!(cancel.is_cancelled());
    assert_released(&source);
    source.finish().await;
}

#[tokio::test]
async fn remote_prepared_public_entry_uses_shared_cpu_lane_and_keeps_eof_owner() {
    let source = Source::new(wave(16000, 1, &samples(16013, 1))).await;
    let (decoder, cancel) = source.decoder().await;
    let provider = Probe::new(Mode::Good);
    let runtime = tokio::runtime::Handle::current();
    let result = tokio::task::spawn_blocking(move || {
        let _owners = crate::media::work::TEST_OWNERS.lock().unwrap();
        runtime.block_on(recognize_remote_wave(
            decoder,
            SourceLanguage::English,
            PreparedSampleRate::Hz16000,
            &provider,
        ))
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result.transcript().unwrap().cues().len(), 1);
    assert_eq!(source.owners.available_permits(), 0);
    assert!(!cancel.is_cancelled());
    drop(result);
    assert!(cancel.is_cancelled());
    assert_released(&source);
    source.finish().await;
}
