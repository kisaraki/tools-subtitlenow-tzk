//! Real synthetic MP3 -> DSP -> owned chunks/maps -> explicit mock ASR. The
//! direct ownership proof and the production native actor use synthetic media.
//! This is not cloud recognition quality or actual playback evidence.
use super::*;
use crate::{
    ai::{
        prepared_recognition::PreparedRecognitionSession,
        prepared_speech::{
            MAX_ASR_BATCH_CHUNKS, OVERLAP_DURATION_MS, PreparedChunkStamp,
            PreparedSpeechCapabilities, PreparedSpeechCue, PreparedSpeechError,
            PreparedSpeechFuture, PreparedSpeechRecognizer, PreparedSpeechRequest,
            PreparedSpeechResponse,
        },
        speech::SpeechLanguage,
    },
    audio::capture::SessionId,
    domain::SourceLanguage,
    media::{
        decoded_clock::{DecodedClockNormalizer, DecodedClockWindow},
        prepared_audio::{PreparedAudioProcessor, PreparedSampleRate},
        prepared_segments::PreparedSegmentBuilder,
    },
};
use std::{fs, fs::OpenOptions, sync::Mutex};

fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(361),
        generation: 6,
    }
}
struct OracleSpan {
    sample_start: u64,
    sample_end: u64,
    native_start: u64,
    native_end: u64,
}
fn oracle_at(spans: &[OracleSpan], sample: u64, end: bool) -> u64 {
    let span = spans
        .iter()
        .find(|span| {
            if end {
                span.sample_start < sample && sample <= span.sample_end
            } else {
                span.sample_start <= sample && sample < span.sample_end
            }
        })
        .unwrap();
    // Independent wide-integer endpoint oracle uses the original decoder's
    // timestamp/frame endpoint, not any map result returned to the mock.
    let offset = u128::from(sample - span.sample_start);
    span.native_start
        + (offset * u128::from(span.native_end - span.native_start)
            / u128::from(span.sample_end - span.sample_start)) as u64
}
struct MockRecognizer {
    observed: Mutex<Vec<PreparedChunkStamp>>,
}
impl PreparedSpeechRecognizer for MockRecognizer {
    fn capabilities(&self) -> PreparedSpeechCapabilities {
        PreparedSpeechCapabilities {
            cancellation: true,
            timestamped_finals: true,
            supported_languages: [true; 5],
            constrained_auto_detection: false,
            sample_rates: [true; 2],
            max_concurrent: MAX_ASR_BATCH_CHUNKS,
        }
    }
    fn recognize<'a>(
        &'a self,
        request: &'a PreparedSpeechRequest,
        cancel: ResolverCancellation,
    ) -> PreparedSpeechFuture<'a> {
        Box::pin(async move {
            if cancel.is_cancelled() {
                return Err(PreparedSpeechError::Cancelled);
            }
            let stamp = request.chunk().stamp();
            assert!(request.chunk().source_clock().is_some());
            self.observed.lock().unwrap().push(stamp);
            Ok(PreparedSpeechResponse {
                identity: stamp.identity,
                sequence: stamp.sequence,
                run_id: stamp.run_id,
                language: Some(SpeechLanguage::English),
                cues: vec![PreparedSpeechCue {
                    start_us: stamp.body_start_us - stamp.audio_start_us,
                    duration_us: stamp.body_end_us - stamp.body_start_us,
                    text: format!("explicit synthetic recognition {}", stamp.sequence).into(),
                }],
            })
        })
    }
}

#[test]
fn mp3_chunk_maps_outlive_decoder_window_and_preserve_native_final_timeline() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    let bytes = crate::win32::media_mp3_fixture::synthetic_mp3(44_100, 2, 13 * 44_100).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("synthetic-mapped-chunks.bin");
    fs::write(&path, &bytes).unwrap();
    for rate in [PreparedSampleRate::Hz16000, PreparedSampleRate::Hz24000] {
        let cancel = ResolverCancellation::default();
        let (mut decoder, pin) = open_native_pinned(
            MediaInput::local(path.clone()).unwrap(),
            identity(),
            cancel.clone(),
        )
        .unwrap();
        let mut normalizer =
            DecodedClockNormalizer::new(identity(), decoder.format(), &cancel).unwrap();
        let mut window = DecodedClockWindow::new(identity(), &cancel).unwrap();
        let mut dsp =
            PreparedAudioProcessor::new(identity(), decoder.format(), rate, &cancel).unwrap();
        let mut segments = PreparedSegmentBuilder::new(identity(), rate).unwrap();
        let mut chunks = Vec::with_capacity(MAX_ASR_BATCH_CHUNKS);
        let mut spans = Vec::new();
        // Reborrow the window in a short callback each push, so metadata is
        // appended before that push's emitted PCM needs its source evidence.
        while let Some(block) = decoder.next(&cancel).unwrap() {
            let native_start = block.timestamp_us();
            let native_end = block.end_us();
            let mapped = normalizer.normalize(block, &cancel).unwrap();
            let span = mapped.span();
            spans.push(OracleSpan {
                sample_start: span.sample_start_us(),
                sample_end: span.sample_end_us(),
                native_start,
                native_end,
            });
            window.append(span, &cancel).unwrap();
            dsp.push_owned(
                mapped.sample_clock_block(),
                &cancel,
                &mut |block, gap, scope| {
                    deliver(block, gap, scope, &mut segments, &mut window, &mut chunks)
                },
            )
            .unwrap();
        }
        dsp.finish_owned(&cancel, &mut |block, gap, scope| {
            deliver(block, gap, scope, &mut segments, &mut window, &mut chunks)
        })
        .unwrap();
        segments.finish(&cancel).unwrap();
        attach_ready(&mut segments, &mut window, &mut chunks, &cancel).unwrap();
        assert_eq!(chunks.len(), 3);
        assert!(chunks.last().unwrap().audio_end_us() > 12_000_000);
        assert!(chunks[1].audio_end_us() > chunks[1].stamp().body_end_us);
        assert!(chunks[2].stamp().audio_start_us < chunks[2].stamp().body_start_us);
        let last_sample = chunks.last().unwrap().audio_end_us();
        window.prune_before(last_sample, &cancel).unwrap();
        drop(window);
        drop(dsp);
        drop(segments);
        drop(normalizer);
        drop(decoder);
        assert!(OpenOptions::new().write(true).open(&path).is_err());
        let provider = MockRecognizer {
            observed: Mutex::new(Vec::new()),
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let transcript = runtime.block_on(async {
            let mut recognition =
                PreparedRecognitionSession::new(identity(), SourceLanguage::English, rate, &cancel)
                    .unwrap();
            recognition
                .recognize_batch(chunks, &provider)
                .await
                .unwrap();
            recognition.finish().unwrap()
        });
        let mut observed = provider.observed.lock().unwrap();
        observed.sort_by_key(|stamp| stamp.sequence);
        assert_eq!(transcript.cues().len(), observed.len());
        for (cue, stamp) in transcript.cues().iter().zip(observed.iter()) {
            let native_start = oracle_at(&spans, stamp.body_start_us, false);
            let native_end = oracle_at(&spans, stamp.body_end_us, true);
            assert_eq!(cue.start_us, native_start);
            assert_eq!(cue.duration_us, native_end - native_start);
        }
        let last = transcript.cues().last().unwrap();
        assert!(last_sample.abs_diff(last.start_us + last.duration_us) > 10);
        assert!(
            !cancel.is_cancelled(),
            "successful request-child cleanup must not cancel source maps"
        );
        drop(observed);
        drop(transcript);
        drop(pin);
        assert!(OpenOptions::new().write(true).open(&path).is_ok());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
fn deliver(
    block: DecodedAudioBlock,
    gap: bool,
    cancel: &ResolverCancellation,
    segments: &mut PreparedSegmentBuilder,
    window: &mut DecodedClockWindow,
    chunks: &mut Vec<crate::ai::prepared_speech::PreparedSpeechChunk>,
) -> Result<(), MediaError> {
    let gap = gap && block.sequence() != 1;
    segments.push(block, gap, cancel).unwrap();
    attach_ready(segments, window, chunks, cancel)
}
fn attach_ready(
    segments: &mut PreparedSegmentBuilder,
    window: &mut DecodedClockWindow,
    chunks: &mut Vec<crate::ai::prepared_speech::PreparedSpeechChunk>,
    cancel: &ResolverCancellation,
) -> Result<(), MediaError> {
    while let Some(chunk) = segments.take() {
        let stamp = chunk.stamp();
        let snapshot = window.snapshot(
            identity(),
            stamp.audio_start_us,
            chunk.audio_end_us(),
            cancel,
        )?;
        chunks.push(chunk.with_clock_map(snapshot).unwrap());
        assert!(chunks.len() <= MAX_ASR_BATCH_CHUNKS);
        window.prune_before(
            stamp
                .body_end_us
                .saturating_sub(u64::from(OVERLAP_DURATION_MS) * 1000),
            cancel,
        )?;
    }
    Ok(())
}

#[test]
fn production_mp3_actor_recognizes_all_native_formats_with_original_timestamps() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("synthetic-actor.bin");
    for source_rate in [32_000, 44_100, 48_000] {
        for channels in [1, 2] {
            let seconds = if source_rate == 44_100 && channels == 2 {
                13
            } else {
                2
            };
            let bytes = crate::win32::media_mp3_fixture::synthetic_mp3(
                source_rate,
                channels,
                seconds * source_rate as usize,
            )
            .unwrap();
            fs::write(&path, &bytes).unwrap();
            let cancel = ResolverCancellation::default();
            let (mut decoder, pin) = open_native_pinned(
                MediaInput::local(path.clone()).unwrap(),
                identity(),
                cancel.clone(),
            )
            .unwrap();
            assert!(decoder.uses_packet_clock());
            let mut normalizer =
                DecodedClockNormalizer::new(identity(), decoder.format(), &cancel).unwrap();
            let mut oracle = Vec::new();
            while let Some(block) = decoder.next(&cancel).unwrap() {
                let native_start = block.timestamp_us();
                let native_end = block.end_us();
                let mapped = normalizer.normalize(block, &cancel).unwrap();
                oracle.push(OracleSpan {
                    sample_start: mapped.span().sample_start_us(),
                    sample_end: mapped.span().sample_end_us(),
                    native_start,
                    native_end,
                });
                assert!(oracle.len() <= 600);
            }
            drop(decoder);
            drop(pin);
            for rate in [PreparedSampleRate::Hz16000, PreparedSampleRate::Hz24000] {
                let cancel = ResolverCancellation::default();
                let mock = MockRecognizer {
                    observed: Mutex::new(Vec::new()),
                };
                let transcript = runtime.block_on(async {
                    tokio::time::timeout(
                        std::time::Duration::from_secs(15),
                        crate::media::prepared_asr::recognize_local_media(
                            MediaInput::local(path.clone()).unwrap(),
                            identity(),
                            SourceLanguage::English,
                            rate,
                            cancel.clone(),
                            &mock,
                        ),
                    )
                    .await
                    .unwrap()
                    .unwrap()
                });
                assert!(!cancel.is_cancelled());
                let mut stamps = mock.observed.lock().unwrap();
                stamps.sort_unstable_by_key(|stamp| stamp.sequence);
                assert_eq!(transcript.cues().len(), stamps.len());
                for (cue, stamp) in transcript.cues().iter().zip(stamps.iter()) {
                    assert_eq!(cue.start_us, oracle_at(&oracle, stamp.body_start_us, false));
                    assert_eq!(
                        cue.start_us + cue.duration_us,
                        oracle_at(&oracle, stamp.body_end_us, true)
                    );
                }
                assert!(OpenOptions::new().write(true).open(&path).is_ok());
                assert_eq!(
                    crate::media::work::workers().available_permits(),
                    crate::media::work::worker_limit()
                );
                assert_eq!(
                    crate::media::work::native_owners().available_permits(),
                    crate::media::work::worker_limit()
                );
                assert_eq!(fs::read(&path).unwrap(), bytes);
                assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
            }
        }
    }
}
