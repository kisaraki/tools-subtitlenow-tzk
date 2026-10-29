//! Integration through public portable APIs with explicit mock speech and RAM
//! PCM only. No native borrowed callback, file, key, network or Ready document.
use crate::{
    ai::{
        prepared_recognition::PreparedRecognitionSession,
        prepared_speech::{
            MAX_ASR_BATCH_CHUNKS, PreparedChunkStamp, PreparedSpeechCapabilities,
            PreparedSpeechChunk, PreparedSpeechCue, PreparedSpeechError, PreparedSpeechFuture,
            PreparedSpeechRecognizer, PreparedSpeechRequest, PreparedSpeechResponse,
        },
        prepared_transcript::PreparedTranscript,
        speech::{SpeechLanguage, StreamIdentity},
    },
    audio::capture::SessionId,
    domain::SourceLanguage,
    media::{
        decoded::{DecodedAudioBlock, DecodedAudioFormat},
        prepared_audio::PreparedSampleRate,
        prepared_segments::PreparedSegmentBuilder,
    },
    resolver::contracts::ResolverCancellation,
};
use std::{
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::Semaphore;

const RATE_HZ: u32 = 16_000;
const ORIGIN_US: u64 = 123_456;
const SECOND_RUN_US: u64 = ORIGIN_US + 30_000_000;
const TEST_DEADLINE: Duration = Duration::from_secs(5);

fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(708),
        generation: 4,
    }
}

fn owned_block(sequence: u64, origin_us: u64, offset: usize, frames: usize) -> DecodedAudioBlock {
    DecodedAudioBlock::new(
        identity(),
        sequence,
        origin_us + offset as u64 * 1_000_000 / u64::from(RATE_HZ),
        DecodedAudioFormat::new(RATE_HZ, 1).unwrap(),
        (offset..offset + frames)
            .map(|frame| ((frame % 30_001) as i32 - 15_000) as i16)
            .collect(),
    )
    .unwrap()
}

#[derive(Clone, Copy)]
enum MockMode {
    Ordered,
    Pending,
}

struct MockSpeech {
    mode: MockMode,
    active: AtomicUsize,
    peak: AtomicUsize,
    first: Semaphore,
    started: Semaphore,
    finished: Mutex<Vec<u64>>,
    tokens: Mutex<Vec<ResolverCancellation>>,
}

impl MockSpeech {
    fn new(mode: MockMode) -> Self {
        Self {
            mode,
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            first: Semaphore::new(0),
            started: Semaphore::new(0),
            finished: Mutex::new(Vec::new()),
            tokens: Mutex::new(Vec::new()),
        }
    }
}

struct ActiveRequest<'a>(&'a AtomicUsize);
impl Drop for ActiveRequest<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl PreparedSpeechRecognizer for MockSpeech {
    fn capabilities(&self) -> PreparedSpeechCapabilities {
        PreparedSpeechCapabilities {
            cancellation: true,
            timestamped_finals: true,
            supported_languages: [true; 5],
            constrained_auto_detection: true,
            sample_rates: [true; 2],
            max_concurrent: 2,
        }
    }

    fn recognize<'a>(
        &'a self,
        request: &'a PreparedSpeechRequest,
        cancel: ResolverCancellation,
    ) -> PreparedSpeechFuture<'a> {
        Box::pin(async move {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(active, Ordering::SeqCst);
            let _active = ActiveRequest(&self.active);
            self.tokens.lock().unwrap().push(cancel.clone());
            self.started.add_permits(1);
            let stamp = request.chunk().stamp();
            match self.mode {
                MockMode::Pending => {
                    cancel.cancelled().await;
                    return Err(PreparedSpeechError::Cancelled);
                }
                MockMode::Ordered if stamp.sequence == 1 => {
                    tokio::select! {
                        biased;
                        _ = cancel.cancelled() => return Err(PreparedSpeechError::Cancelled),
                        permit = self.first.acquire() => permit.unwrap().forget(),
                    }
                }
                MockMode::Ordered if stamp.sequence == 4 => self.first.add_permits(1),
                _ => {}
            }
            // A genuine short final wholly within this body's real interval.
            // In particular the one-second EOF bodies require no clipping.
            let body_duration = stamp.body_end_us - stamp.body_start_us;
            let start_us = stamp.body_start_us - stamp.audio_start_us + body_duration / 4;
            self.finished.lock().unwrap().push(stamp.sequence);
            Ok(PreparedSpeechResponse {
                identity: stamp.identity,
                sequence: stamp.sequence,
                run_id: stamp.run_id,
                language: Some(SpeechLanguage::English),
                cues: vec![PreparedSpeechCue {
                    start_us,
                    duration_us: body_duration / 2,
                    text: stamp.sequence.to_string().into_boxed_str(),
                }],
            })
        })
    }
}

#[derive(Clone, Copy)]
struct ChunkEvidence {
    stamp: PreparedChunkStamp,
    audio_end_us: u64,
    frames: usize,
}

async fn drain_chunks(
    builder: &mut PreparedSegmentBuilder,
    session: &mut PreparedRecognitionSession,
    provider: &MockSpeech,
    batch: &mut Vec<PreparedSpeechChunk>,
    evidence: &mut Vec<ChunkEvidence>,
) {
    while let Some(chunk) = builder.take() {
        evidence.push(ChunkEvidence {
            stamp: chunk.stamp(),
            audio_end_us: chunk.audio_end_us(),
            frames: chunk.samples().len(),
        });
        batch.push(chunk);
        assert!(batch.len() <= MAX_ASR_BATCH_CHUNKS);
        if batch.len() == MAX_ASR_BATCH_CHUNKS {
            session
                .recognize_batch(std::mem::take(batch), provider)
                .await
                .unwrap();
        }
    }
}

#[tokio::test]
async fn owned_pcm_segments_gap_and_eof_merge_into_one_ordered_source_transcript() {
    let cancel = ResolverCancellation::default();
    let provider = MockSpeech::new(MockMode::Ordered);
    let mut builder = PreparedSegmentBuilder::new(identity(), PreparedSampleRate::Hz16000).unwrap();
    let mut session = PreparedRecognitionSession::new(
        identity(),
        SourceLanguage::English,
        PreparedSampleRate::Hz16000,
        &cancel,
    )
    .unwrap();
    let mut batch = Vec::with_capacity(MAX_ASR_BATCH_CHUNKS);
    let mut evidence = Vec::new();
    let transcript: PreparedTranscript = tokio::time::timeout(TEST_DEADLINE, async {
        let mut sequence = 1;
        for (run_index, (origin, seconds)) in [(ORIGIN_US, 21), (SECOND_RUN_US, 1)]
            .into_iter()
            .enumerate()
        {
            let total_frames = RATE_HZ as usize * seconds;
            let mut offset = 0;
            while offset < total_frames {
                let frames =
                    [317, 997, 4003][(sequence - 1) as usize % 3].min(total_frames - offset);
                builder
                    .push(
                        owned_block(sequence, origin, offset, frames),
                        run_index == 1 && offset == 0,
                        &cancel,
                    )
                    .unwrap();
                // Every producer step drains owned output; it cannot wait for
                // queue capacity or retain an unbounded file-sized PCM list.
                drain_chunks(
                    &mut builder,
                    &mut session,
                    &provider,
                    &mut batch,
                    &mut evidence,
                )
                .await;
                sequence += 1;
                offset += frames;
            }
        }
        builder.finish(&cancel).unwrap();
        drain_chunks(
            &mut builder,
            &mut session,
            &provider,
            &mut batch,
            &mut evidence,
        )
        .await;
        if !batch.is_empty() {
            session
                .recognize_batch(std::mem::take(&mut batch), &provider)
                .await
                .unwrap();
        }
        assert!(builder.take().is_none());
        session.finish().unwrap()
    })
    .await
    .unwrap();
    drop(builder);

    assert_eq!(evidence.len(), 4);
    let expected_bodies = [
        (ORIGIN_US, ORIGIN_US + 10_000_000),
        (ORIGIN_US + 10_000_000, ORIGIN_US + 20_000_000),
        (ORIGIN_US + 20_000_000, ORIGIN_US + 21_000_000),
        (SECOND_RUN_US, SECOND_RUN_US + 1_000_000),
    ];
    for (index, item) in evidence.iter().enumerate() {
        assert_eq!(item.stamp.identity, identity());
        assert_eq!(item.stamp.sequence, index as u64 + 1);
        assert_eq!(item.stamp.run_id, if index == 3 { 2 } else { 1 });
        assert_eq!(item.stamp.discontinuity, index == 3);
        assert_eq!(
            (item.stamp.body_start_us, item.stamp.body_end_us),
            expected_bodies[index]
        );
        assert!(item.frames <= RATE_HZ as usize * 11);
    }
    // Both runs delivered their actual final body, including the 21-second
    // tail; overlap-only audio was neither omitted nor made into duplicate text.
    assert_eq!(evidence[2].audio_end_us, evidence[2].stamp.body_end_us);
    assert_eq!(evidence[3].audio_end_us, evidence[3].stamp.body_end_us);
    assert_eq!(evidence[0].stamp.audio_start_us, ORIGIN_US);
    assert_eq!(evidence[3].stamp.audio_start_us, SECOND_RUN_US);
    assert_eq!(provider.peak.load(Ordering::SeqCst), 2);
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
    assert_eq!(*provider.finished.lock().unwrap(), vec![2, 3, 4, 1]);
    assert!(!cancel.is_cancelled());
    assert!(
        provider
            .tokens
            .lock()
            .unwrap()
            .iter()
            .all(ResolverCancellation::is_cancelled)
    );
    assert_eq!(transcript.identity(), identity());
    assert_eq!(transcript.language(), SpeechLanguage::English);
    assert_eq!(transcript.cues().len(), 4);
    for (index, cue) in transcript.cues().iter().enumerate() {
        let (body_start, body_end) = expected_bodies[index];
        let duration = body_end - body_start;
        assert_eq!(cue.text.as_ref(), (index + 1).to_string());
        assert_eq!(cue.start_us, body_start + duration / 4);
        assert_eq!(cue.duration_us, duration / 2);
        assert!(cue.start_us + cue.duration_us <= body_end);
        if index > 0 {
            let previous = &transcript.cues()[index - 1];
            assert!(cue.start_us >= previous.start_us + previous.duration_us);
        }
    }
}

#[tokio::test]
async fn cancelled_owned_pipeline_releases_requests_and_rejects_buffered_eof_tail() {
    let cancel = ResolverCancellation::default();
    let provider = MockSpeech::new(MockMode::Pending);
    let mut builder = PreparedSegmentBuilder::new(identity(), PreparedSampleRate::Hz16000).unwrap();
    let mut session = PreparedRecognitionSession::new(
        identity(),
        SourceLanguage::English,
        PreparedSampleRate::Hz16000,
        &cancel,
    )
    .unwrap();
    let mut chunks = Vec::new();
    let total_frames = RATE_HZ as usize * 12;
    let mut offset = 0;
    let mut sequence = 1;
    while offset < total_frames {
        let frames = 4003.min(total_frames - offset);
        builder
            .push(
                owned_block(sequence, ORIGIN_US, offset, frames),
                false,
                &cancel,
            )
            .unwrap();
        while let Some(chunk) = builder.take() {
            chunks.push(chunk);
        }
        assert!(chunks.len() <= MAX_ASR_BATCH_CHUNKS);
        sequence += 1;
        offset += frames;
    }
    assert_eq!(chunks.len(), 1);
    let control = async {
        provider.started.acquire().await.unwrap().forget();
        cancel.cancel();
    };
    let (result, ()) = tokio::time::timeout(TEST_DEADLINE, async {
        tokio::join!(session.recognize_batch(chunks, &provider), control)
    })
    .await
    .unwrap();
    assert_eq!(result, Err(PreparedSpeechError::Cancelled));
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
    assert!(provider.finished.lock().unwrap().is_empty());
    assert!(
        provider
            .tokens
            .lock()
            .unwrap()
            .iter()
            .all(ResolverCancellation::is_cancelled)
    );
    assert_eq!(builder.finish(&cancel), Err(PreparedSpeechError::Cancelled));
    assert!(builder.take().is_none());
    assert_eq!(session.finish().err(), Some(PreparedSpeechError::Cancelled));
}
