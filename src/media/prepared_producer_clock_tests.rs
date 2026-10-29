//! Affine synthetic native PCM and explicit mock recognition. No files,
//! credentials, devices, actual codecs, network or playback are used here.
use super::*;
use crate::{
    ai::{
        prepared_recognition::PreparedRecognitionSession,
        prepared_speech::{
            PreparedSpeechCapabilities, PreparedSpeechCue, PreparedSpeechFuture,
            PreparedSpeechRecognizer, PreparedSpeechRequest, PreparedSpeechResponse,
        },
        speech::SpeechLanguage,
    },
    audio::capture::SessionId,
    domain::SourceLanguage,
    media::decoded::{DecodedAudioBlock, DecodedAudioFormat, MAX_DECODED_BLOCK_SAMPLES},
};
use std::{
    collections::VecDeque,
    rc::Rc,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread::{self, ThreadId},
    time::Duration,
};

const TEST_TIMEOUT: Duration = Duration::from_secs(15);
type Events = Arc<Mutex<Vec<(&'static str, ThreadId)>>>;

fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(1517),
        generation: 3,
    }
}
fn block(rate: u32, sequence: u64, start: u64, frames: usize) -> DecodedAudioBlock {
    DecodedAudioBlock::new(
        identity(),
        sequence,
        start,
        DecodedAudioFormat::new(rate, 1).unwrap(),
        vec![1_000; frames],
    )
    .unwrap()
}
fn run(rate: u32, sequence: &mut u64, origin: u64, frames: usize) -> Vec<DecodedAudioBlock> {
    let mut offset = 0;
    let mut blocks = Vec::new();
    while offset < frames {
        let count = MAX_DECODED_BLOCK_SAMPLES.min(frames - offset);
        *sequence += 1;
        blocks.push(block(
            rate,
            *sequence,
            origin + offset as u64 * 1_000_000 / u64::from(rate),
            count,
        ));
        offset += count;
    }
    blocks
}
fn event(events: &Events, name: &'static str) {
    events.lock().unwrap().push((name, thread::current().id()));
}
struct SyntheticDecoder {
    format: DecodedAudioFormat,
    blocks: VecDeque<DecodedAudioBlock>,
    pulls: Arc<AtomicUsize>,
    events: Events,
    owner: ThreadId,
    _affine: Rc<()>,
}
impl MediaDecoder for SyntheticDecoder {
    fn format(&self) -> DecodedAudioFormat {
        self.format
    }
    fn next(
        &mut self,
        cancel: &ResolverCancellation,
    ) -> Result<Option<DecodedAudioBlock>, MediaError> {
        assert_eq!(thread::current().id(), self.owner);
        event(&self.events, "next");
        self.pulls.fetch_add(1, Ordering::SeqCst);
        if cancel.is_cancelled() {
            return Err(MediaError::Cancelled);
        }
        Ok(self.blocks.pop_front())
    }
}
impl Drop for SyntheticDecoder {
    fn drop(&mut self) {
        event(&self.events, "drop");
    }
}
struct Fixture {
    source: PreparedLocalSource,
    cancel: ResolverCancellation,
    cpu: Arc<Semaphore>,
    owners: Arc<Semaphore>,
    events: Events,
    pulls: Arc<AtomicUsize>,
    caller: ThreadId,
}
impl Fixture {
    fn new(rate: u32, target: PreparedSampleRate, blocks: Vec<DecodedAudioBlock>) -> Self {
        let cancel = ResolverCancellation::default();
        let cpu = Arc::new(Semaphore::new(1));
        let owners = Arc::new(Semaphore::new(1));
        let events: Events = Arc::new(Mutex::new(Vec::new()));
        let pulls = Arc::new(AtomicUsize::new(0));
        let observed_events = events.clone();
        let observed_pulls = pulls.clone();
        let source = PreparedLocalSource::with_pinned_factory(
            identity(),
            target,
            cancel.clone(),
            Box::new(move |_| {
                event(&observed_events, "create");
                Ok(OpenedDecoder {
                    decoder: Box::new(SyntheticDecoder {
                        format: DecodedAudioFormat::new(rate, 1).unwrap(),
                        blocks: blocks.into(),
                        pulls: observed_pulls,
                        events: observed_events,
                        owner: thread::current().id(),
                        _affine: Rc::new(()),
                    }),
                    media: None,
                    clock: true,
                })
            }),
            cpu.clone(),
            owners.clone(),
        )
        .unwrap();
        Self {
            source,
            cancel,
            cpu,
            owners,
            events,
            pulls,
            caller: thread::current().id(),
        }
    }
    async fn pull(&mut self) -> Result<PreparedLocalBatch, PreparedAsrError> {
        tokio::time::timeout(TEST_TIMEOUT, self.source.pull())
            .await
            .expect("synthetic mapped pull exceeded its bounded test timeout")
    }
    async fn released(&self) {
        tokio::time::timeout(TEST_TIMEOUT, async {
            while self.cpu.available_permits() != 1 || self.owners.available_permits() != 1 {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("synthetic mapped owner failed to exit");
    }
    fn affinity(&self) {
        let events = self.events.lock().unwrap();
        let native = events.iter().find(|(name, _)| *name == "create").unwrap().1;
        assert_ne!(native, self.caller);
        assert!(events.iter().all(|(_, owner)| *owner == native));
        assert_eq!(events.iter().filter(|(name, _)| *name == "drop").count(), 1);
    }
}

struct MockBodyRecognizer;
impl PreparedSpeechRecognizer for MockBodyRecognizer {
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
            if cancel.is_cancelled() {
                return Err(PreparedSpeechError::Cancelled);
            }
            let stamp = request.chunk().stamp();
            assert!(request.chunk().source_clock().is_some());
            Ok(PreparedSpeechResponse {
                identity: stamp.identity,
                sequence: stamp.sequence,
                run_id: stamp.run_id,
                language: Some(SpeechLanguage::English),
                cues: vec![PreparedSpeechCue {
                    start_us: stamp.body_start_us - stamp.audio_start_us,
                    duration_us: stamp.body_end_us - stamp.body_start_us,
                    text: format!("synthetic-body-{}", stamp.sequence).into_boxed_str(),
                }],
            })
        })
    }
}
async fn recognize(
    chunks: Vec<PreparedSpeechChunk>,
    rate: PreparedSampleRate,
) -> crate::ai::prepared_transcript::PreparedTranscript {
    let cancel = ResolverCancellation::default();
    let mut session =
        PreparedRecognitionSession::new(identity(), SourceLanguage::English, rate, &cancel)
            .unwrap();
    session
        .recognize_batch(chunks, &MockBodyRecognizer)
        .await
        .unwrap();
    session.finish().unwrap()
}

#[tokio::test]
async fn maximum_eight_kilohertz_blocks_stage_more_than_twelve_seconds_without_losing_evidence() {
    for target in [PreparedSampleRate::Hz16000, PreparedSampleRate::Hz24000] {
        let blocks = (1..=7)
            .map(|sequence| {
                block(
                    8_000,
                    sequence,
                    300 + (sequence - 1) * 4_095_999,
                    MAX_DECODED_BLOCK_SAMPLES,
                )
            })
            .collect();
        let mut fixture = Fixture::new(8_000, target, blocks);
        let first = fixture.pull().await.unwrap();
        assert!(!first.eof);
        assert!(first.media.is_none());
        assert_eq!(first.chunks.len(), 1);
        assert_eq!(first.chunks[0].stamp().body_end_us, 10_000_300);
        assert_eq!(first.chunks[0].audio_end_us(), 10_500_300);
        assert_eq!(fixture.pulls.load(Ordering::SeqCst), 3);
        assert_eq!(fixture.cpu.available_permits(), 1);
        assert_eq!(fixture.owners.available_permits(), 0);
        let second = fixture.pull().await.unwrap();
        assert!(!second.eof);
        assert_eq!(second.chunks.len(), 1);
        assert_eq!(second.chunks[0].stamp().audio_start_us, 9_500_300);
        assert_eq!(second.chunks[0].stamp().body_end_us, 20_000_300);
        assert_eq!(second.chunks[0].audio_end_us(), 20_500_300);
        assert_eq!(fixture.pulls.load(Ordering::SeqCst), 6);
        let last = fixture.pull().await.unwrap();
        assert!(last.eof);
        assert_eq!(last.chunks.len(), 1);
        assert_eq!(last.chunks[0].stamp().audio_start_us, 19_500_300);
        assert_eq!(last.chunks[0].stamp().body_end_us, 28_672_300);
        fixture.released().await;
        fixture.affinity();
        let mut chunks = first.chunks.into_vec();
        chunks.extend(second.chunks.into_vec());
        chunks.extend(last.chunks.into_vec());
        let transcript = recognize(chunks, target).await;
        assert_eq!(transcript.cues().len(), 3);
        assert_eq!(transcript.cues()[0].start_us, 300);
        assert_eq!(transcript.cues()[0].duration_us, 9_999_998);
        assert_eq!(transcript.cues()[1].start_us, 10_000_298);
        assert_eq!(transcript.cues()[1].duration_us, 9_999_998);
        assert_eq!(transcript.cues()[2].start_us, 20_000_296);
        assert_eq!(transcript.cues()[2].duration_us, 8_671_998);
        assert!(!fixture.cancel.is_cancelled());
    }
}

#[tokio::test]
async fn a_native_gap_over_twelve_seconds_flushes_old_tail_before_staging_new_source() {
    let mut sequence = 0;
    let mut blocks = run(16_000, &mut sequence, 3, 161_600);
    sequence += 1;
    blocks.push(block(16_000, sequence, 25_000_003, 1_600));
    let mut fixture = Fixture::new(16_000, PreparedSampleRate::Hz16000, blocks);
    let old = fixture.pull().await.unwrap();
    assert!(!old.eof);
    assert_eq!(old.chunks.len(), 2);
    assert!(old.chunks.iter().all(|chunk| chunk.stamp().run_id == 1));
    assert_eq!(old.chunks[1].stamp().body_end_us, 10_100_003);
    assert_eq!(old.chunks[1].audio_end_us(), 10_100_003);
    assert_eq!(fixture.cpu.available_permits(), 1);
    assert_eq!(fixture.owners.available_permits(), 0);
    let new = fixture.pull().await.unwrap();
    assert!(new.eof);
    assert_eq!(new.chunks.len(), 1);
    let chunk = &new.chunks[0];
    assert_eq!(chunk.stamp().run_id, 2);
    assert!(chunk.stamp().discontinuity);
    assert_eq!(chunk.stamp().audio_start_us, 25_000_003);
    assert_eq!(chunk.source_clock().unwrap().source_run_id(), 2);
    let mut chunks = old.chunks.into_vec();
    chunks.extend(new.chunks.into_vec());
    let transcript = recognize(chunks, PreparedSampleRate::Hz16000).await;
    assert_eq!(transcript.cues()[2].start_us, 25_000_003);
    fixture.released().await;
    fixture.affinity();
}

#[tokio::test]
async fn many_native_runs_without_a_target_frame_leave_no_invented_output_or_old_map() {
    let mut blocks: Vec<_> = (1..=20)
        .map(|sequence| block(192_000, sequence, (sequence - 1) * 1_000_000, 1))
        .collect();
    blocks.push(block(192_000, 21, 20_000_000, 1_920));
    let mut fixture = Fixture::new(192_000, PreparedSampleRate::Hz24000, blocks);
    let batch = fixture.pull().await.unwrap();
    assert!(batch.eof);
    assert_eq!(batch.chunks.len(), 1);
    let chunk = &batch.chunks[0];
    assert_eq!(chunk.stamp().run_id, 1);
    assert!(!chunk.stamp().discontinuity);
    assert_eq!(chunk.stamp().audio_start_us, 20_000_000);
    assert_eq!(chunk.stamp().body_end_us, 20_010_000);
    assert_eq!(chunk.samples().len(), 240);
    assert!(chunk.samples().iter().all(|sample| *sample == 1_000));
    let map = chunk.source_clock().unwrap();
    assert_eq!(map.source_run_id(), 21);
    let interval = map
        .map_interval(identity(), 20_000_000, 20_010_000)
        .unwrap();
    assert_eq!(interval.first_source_sequence(), 21);
    assert_eq!(interval.last_source_sequence(), 21);
    let transcript = recognize(batch.chunks.into_vec(), PreparedSampleRate::Hz24000).await;
    assert_eq!(transcript.cues()[0].start_us, 20_000_000);
    fixture.released().await;
    fixture.affinity();
}

#[tokio::test]
async fn zero_output_source_runs_between_real_runs_do_not_confuse_chunk_run_identity() {
    let mut blocks = vec![block(192_000, 1, 0, 1_920)];
    blocks.extend((2..=21).map(|sequence| block(192_000, sequence, (sequence - 1) * 1_000_000, 1)));
    blocks.push(block(192_000, 22, 21_000_000, 1_920));
    let mut fixture = Fixture::new(192_000, PreparedSampleRate::Hz24000, blocks);
    let first = fixture.pull().await.unwrap();
    assert!(!first.eof);
    assert_eq!(first.chunks.len(), 1);
    assert_eq!(first.chunks[0].source_clock().unwrap().source_run_id(), 1);
    let second = fixture.pull().await.unwrap();
    assert!(second.eof);
    assert_eq!(second.chunks.len(), 1);
    assert_eq!(second.chunks[0].stamp().run_id, 2);
    assert_eq!(second.chunks[0].source_clock().unwrap().source_run_id(), 22);
    assert_eq!(second.chunks[0].stamp().audio_start_us, 21_000_000);
    let mut chunks = first.chunks.into_vec();
    chunks.extend(second.chunks.into_vec());
    let transcript = recognize(chunks, PreparedSampleRate::Hz24000).await;
    assert_eq!(transcript.cues().len(), 2);
    assert_eq!(transcript.cues()[1].start_us, 21_000_000);
    fixture.released().await;
    fixture.affinity();
}

#[tokio::test]
async fn zero_output_native_runs_at_eof_do_not_invalidate_an_already_closed_real_body() {
    let blocks = vec![
        block(192_000, 1, 0, 1_920),
        block(192_000, 2, 1_000_000, 1),
        block(192_000, 3, 2_000_000, 1),
    ];
    let mut fixture = Fixture::new(192_000, PreparedSampleRate::Hz24000, blocks);
    let first = fixture.pull().await.unwrap();
    assert!(!first.eof);
    assert_eq!(first.chunks.len(), 1);
    assert_eq!(first.chunks[0].stamp().body_end_us, 10_000);
    assert_eq!(first.chunks[0].audio_end_us(), 10_000);
    let last = fixture.pull().await.unwrap();
    assert!(last.eof);
    assert!(last.chunks.is_empty());
    assert!(last.media.is_none());
    let transcript = recognize(first.chunks.into_vec(), PreparedSampleRate::Hz24000).await;
    assert_eq!(transcript.cues().len(), 1);
    assert_eq!(transcript.cues()[0].duration_us, 10_000);
    fixture.released().await;
    fixture.affinity();
}

#[tokio::test]
async fn forty_four_point_one_kilohertz_packet_rounding_reaches_asr_with_native_final_time() {
    for target in [PreparedSampleRate::Hz16000, PreparedSampleRate::Hz24000] {
        let blocks = (1..=90)
            .map(|sequence| block(44_100, sequence, 1_000 + (sequence - 1) * 26_122, 1_152))
            .collect();
        let mut fixture = Fixture::new(44_100, target, blocks);
        let batch = fixture.pull().await.unwrap();
        assert!(batch.eof);
        assert_eq!(batch.chunks.len(), 1);
        let chunk = &batch.chunks[0];
        // floor(90 * 1152 * target / 44100) real target frames. The target
        // tail ends before the source-frame endpoint; neither clock is padded.
        assert_eq!(
            chunk.samples().len(),
            (103_680_u64 * u64::from(target.hz()) / 44_100) as usize
        );
        assert_eq!(chunk.stamp().body_end_us, 2_352_000);
        let map = chunk.source_clock().unwrap();
        let native = map.map_interval(identity(), 1_000, 2_352_000).unwrap();
        // Last sample interval: 2324897..2351020, native2324858..2350980.
        // The 20 us target tail exclusion remains within that observed packet.
        assert_eq!(native.end_us(), 2_351_960);
        let transcript = recognize(batch.chunks.into_vec(), target).await;
        assert_eq!(transcript.cues()[0].start_us, 1_000);
        assert_eq!(transcript.cues()[0].duration_us, 2_350_960);
        fixture.released().await;
        fixture.affinity();
    }
}

#[tokio::test]
async fn source_span_capacity_overflow_fails_explicitly_and_releases_native_owner() {
    let count = crate::media::decoded_clock::MAX_CLOCK_WINDOW_SPANS + 1;
    let blocks = (1..=count as u64)
        .map(|sequence| block(16_000, sequence, (sequence - 1) * 1_000_000 / 16_000, 1))
        .collect();
    let mut fixture = Fixture::new(16_000, PreparedSampleRate::Hz16000, blocks);
    assert!(matches!(
        fixture.pull().await,
        Err(PreparedAsrError::Media(MediaError::Overloaded))
            | Err(PreparedAsrError::Speech(PreparedSpeechError::Overloaded))
    ));
    assert!(fixture.cancel.is_cancelled());
    assert_eq!(fixture.pulls.load(Ordering::SeqCst), count);
    fixture.released().await;
    fixture.affinity();
}

#[tokio::test]
async fn cancelling_after_a_returned_mapped_chunk_revokes_evidence_and_reaps_idle_native_owner() {
    let mut sequence = 0;
    let blocks = run(16_000, &mut sequence, 17, 16_000 * 22);
    let mut fixture = Fixture::new(16_000, PreparedSampleRate::Hz16000, blocks);
    let batch = fixture.pull().await.unwrap();
    assert!(!batch.eof);
    assert_eq!(batch.chunks.len(), 1);
    assert_eq!(fixture.cpu.available_permits(), 1);
    assert_eq!(fixture.owners.available_permits(), 0);
    let pulls = fixture.pulls.load(Ordering::SeqCst);
    fixture.cancel.cancel();
    let chunk = &batch.chunks[0];
    assert_eq!(
        chunk.source_clock().unwrap().map_interval(
            identity(),
            chunk.stamp().body_start_us,
            chunk.stamp().body_end_us
        ),
        Err(MediaError::Cancelled)
    );
    fixture.released().await;
    assert_eq!(fixture.pulls.load(Ordering::SeqCst), pulls);
    fixture.affinity();
    assert!(matches!(
        fixture.pull().await,
        Err(PreparedAsrError::Media(MediaError::Cancelled))
    ));
}
