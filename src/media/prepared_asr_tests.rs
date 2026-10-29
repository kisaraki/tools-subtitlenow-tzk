//! Public preflight and owner tests. No file is opened and no provider request
//! starts: every fixture exits before native source construction/pulling.
use super::*;
use crate::{
    ai::prepared_speech::{
        PreparedSpeechCapabilities, PreparedSpeechFuture, PreparedSpeechRequest,
    },
    audio::capture::SessionId,
};
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone, Copy)]
enum CapabilityMode {
    Valid,
    NoCancellation,
    NoTimestamps,
    No16k,
    No24k,
    NoEnglish,
    NoAuto,
    ZeroConcurrency,
    ExcessConcurrency,
    Panic,
}
struct Probe {
    mode: CapabilityMode,
    capabilities_calls: AtomicUsize,
    recognition_calls: AtomicUsize,
    cancel_on_capabilities: Option<ResolverCancellation>,
}
impl Probe {
    fn new(mode: CapabilityMode) -> Self {
        Self {
            mode,
            capabilities_calls: AtomicUsize::new(0),
            recognition_calls: AtomicUsize::new(0),
            cancel_on_capabilities: None,
        }
    }
    fn counts(&self) -> (usize, usize) {
        (
            self.capabilities_calls.load(Ordering::SeqCst),
            self.recognition_calls.load(Ordering::SeqCst),
        )
    }
}
impl PreparedSpeechRecognizer for Probe {
    fn capabilities(&self) -> PreparedSpeechCapabilities {
        self.capabilities_calls.fetch_add(1, Ordering::SeqCst);
        if let Some(cancel) = &self.cancel_on_capabilities {
            cancel.cancel();
        }
        let mut capabilities = PreparedSpeechCapabilities {
            cancellation: true,
            timestamped_finals: true,
            supported_languages: [true; 5],
            constrained_auto_detection: true,
            sample_rates: [true; 2],
            max_concurrent: 4,
        };
        match self.mode {
            CapabilityMode::Valid => {}
            CapabilityMode::NoCancellation => capabilities.cancellation = false,
            CapabilityMode::NoTimestamps => capabilities.timestamped_finals = false,
            CapabilityMode::No16k => capabilities.sample_rates[0] = false,
            CapabilityMode::No24k => capabilities.sample_rates[1] = false,
            CapabilityMode::NoEnglish => capabilities.supported_languages[0] = false,
            CapabilityMode::NoAuto => capabilities.constrained_auto_detection = false,
            CapabilityMode::ZeroConcurrency => capabilities.max_concurrent = 0,
            CapabilityMode::ExcessConcurrency => capabilities.max_concurrent = 5,
            CapabilityMode::Panic => panic!("synthetic capability panic"),
        }
        capabilities
    }
    fn recognize<'a>(
        &'a self,
        _: &'a PreparedSpeechRequest,
        _: ResolverCancellation,
    ) -> PreparedSpeechFuture<'a> {
        self.recognition_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err(PreparedSpeechError::NotConfigured) })
    }
}
fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(807),
        generation: 1,
    }
}
fn missing_input() -> MediaInput {
    static NEXT: AtomicUsize = AtomicUsize::new(1);
    let path = std::env::current_dir().unwrap().join(format!(
        "missing-prepared-asr-{}-{}.wav",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    MediaInput::local(path).unwrap()
}

#[test]
fn unpolled_public_future_starts_no_io_and_cancels_its_owner_on_drop() {
    let provider = Probe::new(CapabilityMode::Valid);
    let cancel = ResolverCancellation::default();
    let future = recognize_local_media(
        missing_input(),
        identity(),
        SourceLanguage::English,
        PreparedSampleRate::Hz16000,
        cancel.clone(),
        &provider,
    );
    assert_eq!(provider.counts(), (0, 0));
    assert!(!cancel.is_cancelled());
    drop(future);
    assert!(cancel.is_cancelled());
    assert_eq!(provider.counts(), (0, 0));
}

#[tokio::test]
async fn prior_cancellation_precedes_capability_query_and_missing_media() {
    let provider = Probe::new(CapabilityMode::Panic);
    let cancel = ResolverCancellation::default();
    cancel.cancel();
    let result = recognize_local_media(
        missing_input(),
        identity(),
        SourceLanguage::English,
        PreparedSampleRate::Hz16000,
        cancel.clone(),
        &provider,
    )
    .await;
    assert!(matches!(
        result,
        Err(PreparedAsrError::Speech(PreparedSpeechError::Cancelled))
    ));
    assert_eq!(provider.counts(), (0, 0));
}

#[tokio::test]
async fn invalid_session_identity_precedes_provider_and_native_io() {
    for invalid in [
        StreamIdentity {
            session: SessionId(0),
            generation: 1,
        },
        StreamIdentity {
            session: SessionId(807),
            generation: 0,
        },
    ] {
        let provider = Probe::new(CapabilityMode::Panic);
        let cancel = ResolverCancellation::default();
        let result = recognize_local_media(
            missing_input(),
            invalid,
            SourceLanguage::English,
            PreparedSampleRate::Hz16000,
            cancel.clone(),
            &provider,
        )
        .await;
        assert!(matches!(
            result,
            Err(PreparedAsrError::Speech(PreparedSpeechError::InvalidData))
        ));
        assert!(cancel.is_cancelled());
        assert_eq!(provider.counts(), (0, 0));
    }
}

#[tokio::test]
async fn missing_cancellation_or_timestamps_is_rejected_before_file_access() {
    for mode in [CapabilityMode::NoCancellation, CapabilityMode::NoTimestamps] {
        let provider = Probe::new(mode);
        let cancel = ResolverCancellation::default();
        let result = recognize_local_media(
            missing_input(),
            identity(),
            SourceLanguage::English,
            PreparedSampleRate::Hz16000,
            cancel.clone(),
            &provider,
        )
        .await;
        assert!(matches!(
            result,
            Err(PreparedAsrError::Speech(
                PreparedSpeechError::UnsupportedCapability
            ))
        ));
        assert!(cancel.is_cancelled());
        assert_eq!(provider.counts(), (1, 0));
    }
}

#[tokio::test]
async fn selected_sample_rate_is_validated_before_file_access() {
    for (mode, rate) in [
        (CapabilityMode::No16k, PreparedSampleRate::Hz16000),
        (CapabilityMode::No24k, PreparedSampleRate::Hz24000),
    ] {
        let provider = Probe::new(mode);
        let cancel = ResolverCancellation::default();
        let result = recognize_local_media(
            missing_input(),
            identity(),
            SourceLanguage::English,
            rate,
            cancel.clone(),
            &provider,
        )
        .await;
        assert!(matches!(
            result,
            Err(PreparedAsrError::Speech(
                PreparedSpeechError::UnsupportedCapability
            ))
        ));
        assert!(cancel.is_cancelled());
        assert_eq!(provider.counts(), (1, 0));
    }
}

#[tokio::test]
async fn unsupported_selected_language_and_unconstrained_auto_precede_media_io() {
    for (mode, language) in [
        (CapabilityMode::NoEnglish, SourceLanguage::English),
        (CapabilityMode::NoEnglish, SourceLanguage::Auto),
        (CapabilityMode::NoAuto, SourceLanguage::Auto),
    ] {
        let provider = Probe::new(mode);
        let cancel = ResolverCancellation::default();
        let result = recognize_local_media(
            missing_input(),
            identity(),
            language,
            PreparedSampleRate::Hz16000,
            cancel.clone(),
            &provider,
        )
        .await;
        assert!(matches!(
            result,
            Err(PreparedAsrError::Speech(
                PreparedSpeechError::UnsupportedLanguage
            ))
        ));
        assert!(cancel.is_cancelled());
        assert_eq!(provider.counts(), (1, 0));
    }
}

#[tokio::test]
async fn provider_concurrency_must_be_one_to_four_before_any_media_work() {
    for mode in [
        CapabilityMode::ZeroConcurrency,
        CapabilityMode::ExcessConcurrency,
    ] {
        let provider = Probe::new(mode);
        let cancel = ResolverCancellation::default();
        let result = recognize_local_media(
            missing_input(),
            identity(),
            SourceLanguage::English,
            PreparedSampleRate::Hz16000,
            cancel.clone(),
            &provider,
        )
        .await;
        assert!(matches!(
            result,
            Err(PreparedAsrError::Speech(
                PreparedSpeechError::UnsupportedCapability
            ))
        ));
        assert!(cancel.is_cancelled());
        assert_eq!(provider.counts(), (1, 0));
    }
}

#[tokio::test]
async fn capability_panic_becomes_typed_error_and_cancels_without_native_io() {
    let provider = Probe::new(CapabilityMode::Panic);
    let cancel = ResolverCancellation::default();
    let result = recognize_local_media(
        missing_input(),
        identity(),
        SourceLanguage::English,
        PreparedSampleRate::Hz16000,
        cancel.clone(),
        &provider,
    )
    .await;
    assert!(matches!(
        result,
        Err(PreparedAsrError::Speech(PreparedSpeechError::InvalidData))
    ));
    assert!(cancel.is_cancelled());
    assert_eq!(provider.counts(), (1, 0));
}

#[tokio::test]
async fn cancellation_inside_capability_query_is_checked_before_native_source() {
    let cancel = ResolverCancellation::default();
    let mut provider = Probe::new(CapabilityMode::Valid);
    provider.cancel_on_capabilities = Some(cancel.clone());
    let result = recognize_local_media(
        missing_input(),
        identity(),
        SourceLanguage::English,
        PreparedSampleRate::Hz16000,
        cancel.clone(),
        &provider,
    )
    .await;
    assert!(matches!(
        result,
        Err(PreparedAsrError::Speech(PreparedSpeechError::Cancelled))
    ));
    assert_eq!(provider.counts(), (1, 0));
}

#[test]
fn public_error_preserves_safe_domain_kind_without_private_payload() {
    let media = PreparedAsrError::from(MediaError::ProtectedMedia);
    let speech = PreparedAsrError::from(PreparedSpeechError::Credential);
    assert_eq!(media, PreparedAsrError::Media(MediaError::ProtectedMedia));
    assert_eq!(
        speech,
        PreparedAsrError::Speech(PreparedSpeechError::Credential)
    );
    assert_eq!(media.to_string(), MediaError::ProtectedMedia.to_string());
    assert_eq!(
        speech.to_string(),
        PreparedSpeechError::Credential.to_string()
    );
}

#[cfg(windows)]
mod native {
    use super::*;
    use crate::ai::{
        prepared_speech::{PreparedChunkStamp, PreparedSpeechCue, PreparedSpeechResponse},
        speech::SpeechLanguage,
    };
    use std::{
        fs::{self, OpenOptions},
        path::PathBuf,
        pin::Pin,
        sync::Mutex,
        time::Duration,
    };
    use tokio::sync::Semaphore;

    struct WaveFixture {
        directory: tempfile::TempDir,
        path: PathBuf,
        bytes: Vec<u8>,
    }
    impl WaveFixture {
        fn new(seconds: usize) -> Self {
            let frames = 16_000 * seconds;
            let data_bytes = u32::try_from(frames * 2).unwrap();
            let mut bytes = Vec::with_capacity(44 + frames * 2);
            bytes.extend_from_slice(b"RIFF");
            bytes.extend_from_slice(&(36 + data_bytes).to_le_bytes());
            bytes.extend_from_slice(b"WAVEfmt ");
            bytes.extend_from_slice(&16u32.to_le_bytes());
            bytes.extend_from_slice(&1u16.to_le_bytes());
            bytes.extend_from_slice(&1u16.to_le_bytes());
            bytes.extend_from_slice(&16_000u32.to_le_bytes());
            bytes.extend_from_slice(&32_000u32.to_le_bytes());
            bytes.extend_from_slice(&2u16.to_le_bytes());
            bytes.extend_from_slice(&16u16.to_le_bytes());
            bytes.extend_from_slice(b"data");
            bytes.extend_from_slice(&data_bytes.to_le_bytes());
            for index in 0..frames {
                bytes.extend_from_slice(&sample(index).to_le_bytes());
            }
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("synthetic-owned-asr.wav");
            fs::write(&path, &bytes).unwrap();
            Self {
                directory,
                path,
                bytes,
            }
        }
        fn input(&self) -> MediaInput {
            MediaInput::local(self.path.clone()).unwrap()
        }
        fn assert_unchanged(&self) {
            assert_eq!(fs::read(&self.path).unwrap(), self.bytes);
            assert_eq!(fs::read_dir(self.directory.path()).unwrap().count(), 1);
        }
    }
    fn sample(index: usize) -> i16 {
        ((index % 40_001) as i32 - 20_000) as i16
    }
    #[derive(Clone, Copy)]
    struct Observation {
        stamp: PreparedChunkStamp,
        audio_end_us: u64,
        frames: usize,
    }
    /// This explicit test recognizer verifies real decoded PCM and either
    /// returns synthetic text with the request's genuine body timing or waits
    /// until the caller cancels. It is not a production speech implementation.
    struct MockRecognizer {
        pending: bool,
        error: Option<PreparedSpeechError>,
        release: Semaphore,
        observations: Mutex<Vec<Observation>>,
        tokens: Mutex<Vec<ResolverCancellation>>,
        started: Semaphore,
        released: AtomicUsize,
    }
    impl MockRecognizer {
        fn new(pending: bool) -> Self {
            Self {
                pending,
                error: None,
                release: Semaphore::new(0),
                observations: Mutex::new(Vec::new()),
                tokens: Mutex::new(Vec::new()),
                started: Semaphore::new(0),
                released: AtomicUsize::new(0),
            }
        }
    }
    struct RequestLease<'a>(&'a AtomicUsize);
    impl Drop for RequestLease<'_> {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    impl PreparedSpeechRecognizer for MockRecognizer {
        fn capabilities(&self) -> PreparedSpeechCapabilities {
            PreparedSpeechCapabilities {
                cancellation: true,
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
            cancel: ResolverCancellation,
        ) -> PreparedSpeechFuture<'a> {
            Box::pin(async move {
                let _lease = RequestLease(&self.released);
                let stamp = request.chunk().stamp();
                assert_eq!(stamp.rate, PreparedSampleRate::Hz16000);
                let first_frame =
                    usize::try_from(stamp.audio_start_us * 16_000 / 1_000_000).unwrap();
                for (offset, value) in request.chunk().samples().iter().copied().enumerate() {
                    assert_eq!(value, sample(first_frame + offset));
                }
                self.observations.lock().unwrap().push(Observation {
                    stamp,
                    audio_end_us: request.chunk().audio_end_us(),
                    frames: request.chunk().samples().len(),
                });
                self.tokens.lock().unwrap().push(cancel.clone());
                self.started.add_permits(1);
                if self.pending {
                    self.release.acquire().await.unwrap().forget();
                }
                if let Some(error) = self.error {
                    return Err(error);
                }
                Ok(PreparedSpeechResponse {
                    identity: stamp.identity,
                    sequence: stamp.sequence,
                    run_id: stamp.run_id,
                    language: Some(SpeechLanguage::English),
                    cues: vec![PreparedSpeechCue {
                        start_us: stamp.body_start_us - stamp.audio_start_us,
                        duration_us: stamp.body_end_us - stamp.body_start_us,
                        text: format!("synthetic source {}", stamp.sequence).into_boxed_str(),
                    }],
                })
            })
        }
    }

    #[test]
    fn actual_twenty_one_second_wave_reaches_owned_asr_and_closed_eof_transcript() {
        let _owners = super::super::super::work::TEST_OWNERS.lock().unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let fixture = WaveFixture::new(21);
            let provider = MockRecognizer::new(false);
            let cancel = ResolverCancellation::default();
            let native_before = super::super::super::work::native_owners().available_permits();
            let transcript = recognize_local_media(
                fixture.input(),
                identity(),
                SourceLanguage::English,
                PreparedSampleRate::Hz16000,
                cancel.clone(),
                &provider,
            )
            .await
            .unwrap();
            assert!(!cancel.is_cancelled());
            assert_eq!(transcript.identity(), identity());
            assert_eq!(transcript.language(), SpeechLanguage::English);
            assert_eq!(transcript.cues().len(), 3);
            for (index, cue) in transcript.cues().iter().enumerate() {
                assert_eq!(cue.start_us, index as u64 * 10_000_000);
                assert_eq!(
                    cue.duration_us,
                    if index == 2 { 1_000_000 } else { 10_000_000 }
                );
            }
            let observations = provider.observations.lock().unwrap();
            assert_eq!(observations.len(), 3);
            assert_eq!(provider.released.load(Ordering::SeqCst), 3);
            for observation in observations.iter() {
                assert_eq!(observation.stamp.identity, identity());
                assert_eq!(observation.stamp.run_id, 1);
                assert!(!observation.stamp.discontinuity);
                assert!(observation.frames <= 16_000 * 11);
            }
            let last = observations
                .iter()
                .max_by_key(|record| record.stamp.sequence)
                .unwrap();
            assert_eq!(last.audio_end_us, 21_000_000);
            assert_eq!(last.stamp.body_end_us, last.audio_end_us);
            assert_eq!(
                super::super::super::work::native_owners().available_permits(),
                native_before
            );
            assert!(OpenOptions::new().write(true).open(&fixture.path).is_ok());
            fixture.assert_unchanged();
        });
    }

    #[test]
    fn actual_long_wave_pauses_before_eof_during_asr_and_cancels_every_owner() {
        let _owners = super::super::super::work::TEST_OWNERS.lock().unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let fixture = WaveFixture::new(65);
            let provider = MockRecognizer::new(true);
            let cancel = ResolverCancellation::default();
            let cpu = super::super::super::work::workers();
            let native = super::super::super::work::native_owners();
            let cpu_before = cpu.available_permits();
            let native_before = native.available_permits();
            assert!(cpu_before > 0 && native_before > 0);
            let future = recognize_local_media(
                fixture.input(),
                identity(),
                SourceLanguage::English,
                PreparedSampleRate::Hz16000,
                cancel.clone(),
                &provider,
            );
            tokio::pin!(future);
            tokio::time::timeout(Duration::from_secs(10), async {
                tokio::select! {
                    result = &mut future => panic!("pending fixture completed: {}", result.is_ok()),
                    started = provider.started.acquire() => started.unwrap().forget(),
                }
            })
            .await
            .unwrap();
            let active = provider.observations.lock().unwrap().len();
            assert!((1..=MAX_ASR_BATCH_CHUNKS).contains(&active));
            {
                let observations = provider.observations.lock().unwrap();
                assert!(
                    observations
                        .iter()
                        .all(|record| record.audio_end_us < 65_000_000)
                );
                assert!(
                    observations
                        .iter()
                        .all(|record| record.frames <= 16_000 * 11)
                );
            }
            assert_eq!(cpu.available_permits(), cpu_before);
            assert_eq!(native.available_permits(), native_before - 1);
            assert!(OpenOptions::new().write(true).open(&fixture.path).is_err());
            // Holding every idle CPU permit must not keep cancelled native cleanup
            // alive: its lifetime is bounded by the separate native-owner permit.
            let all_cpu = cpu
                .clone()
                .try_acquire_many_owned(cpu_before as u32)
                .unwrap();
            cancel.cancel();
            let result = tokio::time::timeout(Duration::from_secs(5), &mut future)
                .await
                .unwrap();
            assert!(matches!(
                result,
                Err(PreparedAsrError::Speech(PreparedSpeechError::Cancelled))
            ));
            assert_eq!(provider.released.load(Ordering::SeqCst), active);
            assert!(
                provider
                    .tokens
                    .lock()
                    .unwrap()
                    .iter()
                    .all(ResolverCancellation::is_cancelled)
            );
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    if native.available_permits() == native_before
                        && OpenOptions::new().write(true).open(&fixture.path).is_ok()
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap();
            drop(all_cpu);
            assert_eq!(cpu.available_permits(), cpu_before);
            fixture.assert_unchanged();
        });
    }

    async fn final_asr_wait<F>(work: &mut Pin<Box<F>>, provider: &MockRecognizer)
    where
        F: Future<Output = Result<RecognizedLocalMedia, PreparedAsrError>>,
    {
        tokio::time::timeout(Duration::from_secs(10), async {
            tokio::select! {
                result = work => panic!("held final request completed: {}", result.is_ok()),
                started = provider.started.acquire() => started.unwrap().forget(),
            }
        })
        .await
        .unwrap();
    }

    #[test]
    fn original_wave_pin_survives_native_eof_final_asr_and_recognized_owner_transfer() {
        let _owners = super::super::super::work::TEST_OWNERS.lock().unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let fixture = WaveFixture::new(21);
            let provider = MockRecognizer::new(true);
            let cancel = ResolverCancellation::default();
            let cpu = super::super::super::work::workers();
            let native = super::super::super::work::native_owners();
            let cpu_before = cpu.available_permits();
            let native_before = native.available_permits();
            let mut work = Box::pin(recognize_pinned_local_media(
                fixture.input(),
                identity(),
                SourceLanguage::English,
                PreparedSampleRate::Hz16000,
                cancel.clone(),
                &provider,
            ));
            final_asr_wait(&mut work, &provider).await;
            let active = provider.observations.lock().unwrap().len();
            assert_eq!(active, 3);
            assert_eq!(cpu.available_permits(), cpu_before);
            // COM/MF and their owner permits are already gone, but the exact
            // original file object remains pinned during this final ASR wait.
            assert_eq!(native.available_permits(), native_before);
            assert!(OpenOptions::new().write(true).open(&fixture.path).is_err());
            assert!(fs::remove_file(&fixture.path).is_err());
            provider.release.add_permits(active);
            let recognized = work.await.unwrap();
            assert_eq!(recognized.transcript.identity(), identity());
            assert_eq!(recognized.transcript.cues().len(), 3);
            assert_eq!(recognized.media.byte_len(), fixture.bytes.len() as u64);
            assert!(!cancel.is_cancelled());
            assert_eq!(provider.released.load(Ordering::SeqCst), active);
            assert!(OpenOptions::new().write(true).open(&fixture.path).is_err());
            assert!(fs::remove_file(&fixture.path).is_err());
            fixture.assert_unchanged();
            drop(recognized);
            assert!(OpenOptions::new().write(true).open(&fixture.path).is_ok());
            assert!(fs::remove_file(&fixture.path).is_ok());
        });
    }

    #[test]
    fn final_asr_cancel_drop_and_failure_release_original_wave_pin_and_requests() {
        let _owners = super::super::super::work::TEST_OWNERS.lock().unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            for outcome in 0..3 {
                let fixture = WaveFixture::new(21);
                let mut provider = MockRecognizer::new(true);
                if outcome == 2 {
                    provider.error = Some(PreparedSpeechError::Credential);
                }
                let cancel = ResolverCancellation::default();
                let native = super::super::super::work::native_owners();
                let native_before = native.available_permits();
                let mut work = Box::pin(recognize_pinned_local_media(
                    fixture.input(),
                    identity(),
                    SourceLanguage::English,
                    PreparedSampleRate::Hz16000,
                    cancel.clone(),
                    &provider,
                ));
                final_asr_wait(&mut work, &provider).await;
                let active = provider.observations.lock().unwrap().len();
                assert_eq!(active, 3);
                assert_eq!(native.available_permits(), native_before);
                assert!(OpenOptions::new().write(true).open(&fixture.path).is_err());
                assert!(fs::remove_file(&fixture.path).is_err());
                match outcome {
                    0 => {
                        cancel.cancel();
                        assert!(matches!(
                            work.await,
                            Err(PreparedAsrError::Speech(PreparedSpeechError::Cancelled))
                        ));
                    }
                    1 => drop(work),
                    _ => {
                        provider.release.add_permits(active);
                        assert!(matches!(
                            work.await,
                            Err(PreparedAsrError::Speech(PreparedSpeechError::Credential))
                        ));
                    }
                }
                assert!(cancel.is_cancelled());
                assert_eq!(provider.released.load(Ordering::SeqCst), active);
                assert!(
                    provider
                        .tokens
                        .lock()
                        .unwrap()
                        .iter()
                        .all(ResolverCancellation::is_cancelled)
                );
                assert_eq!(native.available_permits(), native_before);
                assert!(OpenOptions::new().write(true).open(&fixture.path).is_ok());
                fixture.assert_unchanged();
                assert!(fs::remove_file(&fixture.path).is_ok());
            }
        });
    }
}
