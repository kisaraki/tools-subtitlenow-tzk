//! Explicit synthetic decoded PCM and mock recognition only. Native packet
//! timestamps are test input, not claims about a codec, cloud or playback.
use super::*;
use crate::{
    ai::{
        prepared_recognition::PreparedRecognitionSession,
        prepared_speech::{
            PreparedSpeechCapabilities, PreparedSpeechChunk, PreparedSpeechCue,
            PreparedSpeechFuture, PreparedSpeechRecognizer,
        },
    },
    audio::capture::SessionId,
    media::{
        decoded::{DecodedAudioBlock, DecodedAudioFormat},
        decoded_clock::{DecodedClockNormalizer, DecodedClockSpan, DecodedClockWindow},
        prepared_audio::PreparedSampleRate,
    },
    resolver::contracts::ResolverCancellation,
};
use std::sync::Mutex;
use tokio::sync::Semaphore;

fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(1307),
        generation: 4,
    }
}

struct Clock {
    cancel: ResolverCancellation,
    format: DecodedAudioFormat,
    normalizer: DecodedClockNormalizer,
    window: DecodedClockWindow,
    sequence: u64,
    native_next: u64,
    sample_end: u64,
    frames_per_block: usize,
}
impl Clock {
    fn new(rate: u32, frames_per_block: usize, origin: u64) -> Self {
        let cancel = ResolverCancellation::default();
        let format = DecodedAudioFormat::new(rate, 1).unwrap();
        Self {
            normalizer: DecodedClockNormalizer::new(identity(), format, &cancel).unwrap(),
            window: DecodedClockWindow::new(identity(), &cancel).unwrap(),
            cancel,
            format,
            sequence: 0,
            native_next: origin,
            sample_end: origin,
            frames_per_block,
        }
    }
    fn push_at(&mut self, frames: usize, native_start: u64) -> DecodedClockSpan {
        self.sequence += 1;
        let block = DecodedAudioBlock::new(
            identity(),
            self.sequence,
            native_start,
            self.format,
            vec![0; frames],
        )
        .unwrap();
        self.native_next = block.end_us();
        let mapped = self.normalizer.normalize(block, &self.cancel).unwrap();
        let span = mapped.span();
        self.sample_end = span.sample_end_us();
        self.window.append(span, &self.cancel).unwrap();
        span
    }
    fn push(&mut self) -> DecodedClockSpan {
        self.push_at(self.frames_per_block, self.native_next)
    }
    fn snapshot(&mut self, start: u64, end: u64) -> DecodedClockSnapshot {
        if self.sequence != 0 {
            self.window
                .prune_before(start.min(self.sample_end), &self.cancel)
                .unwrap();
        }
        while self.sample_end < end {
            self.push();
        }
        self.window
            .snapshot(identity(), start, end, &self.cancel)
            .unwrap()
    }
}

fn bare_chunk(
    sequence: u64,
    run_id: u64,
    audio_start: u64,
    body_start: u64,
    body_end: u64,
    audio_end: u64,
    discontinuity: bool,
) -> PreparedSpeechChunk {
    let frames = (audio_end - audio_start) * 16_000 / 1_000_000;
    PreparedSpeechChunk::new(
        PreparedChunkStamp {
            identity: identity(),
            sequence,
            run_id,
            rate: PreparedSampleRate::Hz16000,
            audio_start_us: audio_start,
            body_start_us: body_start,
            body_end_us: body_end,
            discontinuity,
        },
        vec![0; frames as usize],
    )
    .unwrap()
}
fn chunk(sequence: u64, start: u64, end: u64) -> PreparedSpeechChunk {
    bare_chunk(sequence, 1, start, start, end, end, false)
}
fn request(chunk: PreparedSpeechChunk) -> PreparedSpeechRequest {
    PreparedSpeechRequest::new(chunk, SourceLanguage::English, Vec::new()).unwrap()
}
fn mapped_request(clock: &mut Clock, sequence: u64, start: u64, end: u64) -> PreparedSpeechRequest {
    let map = clock.snapshot(start, end);
    request(chunk(sequence, start, end).with_clock_map(map).unwrap())
}
fn reply(
    request: &PreparedSpeechRequest,
    start: u64,
    duration: u64,
    text: &str,
) -> PreparedSpeechResponse {
    let stamp = request.chunk().stamp();
    PreparedSpeechResponse {
        identity: stamp.identity,
        sequence: stamp.sequence,
        run_id: stamp.run_id,
        language: Some(SpeechLanguage::English),
        cues: vec![PreparedSpeechCue {
            start_us: start,
            duration_us: duration,
            text: text.into(),
        }],
    }
}
fn merger() -> PreparedTranscriptMerger {
    PreparedTranscriptMerger::new(identity(), SourceLanguage::English).unwrap()
}

#[test]
fn a_chunk_requires_exact_audio_and_context_extent_from_its_map() {
    for (map_start, map_end) in [(0, 30_000), (0, 10_000), (5_000, 20_000)] {
        let mut clock = Clock::new(16_000, 480, 0);
        let map = clock.snapshot(map_start, map_end);
        assert!(matches!(
            chunk(1, 0, 20_000).with_clock_map(map),
            Err(PreparedSpeechError::BoundaryMismatch)
        ));
    }
}

#[test]
fn another_generation_cannot_bind_valid_source_clock_evidence() {
    let mut clock = Clock::new(16_000, 160, 0);
    let map = clock.snapshot(0, 10_000);
    let mut stamp = chunk(1, 0, 10_000).stamp();
    stamp.identity.generation += 1;
    let other = PreparedSpeechChunk::new(stamp, vec![0; 160]).unwrap();
    assert!(matches!(
        other.with_clock_map(map),
        Err(PreparedSpeechError::Stale)
    ));
}

#[test]
fn attaching_a_second_map_never_replaces_existing_chunk_provenance() {
    let mut clock = Clock::new(16_000, 160, 0);
    let first = clock.snapshot(0, 10_000);
    let second = clock.snapshot(0, 10_000);
    let chunk = chunk(1, 0, 10_000).with_clock_map(first).unwrap();
    assert!(matches!(
        chunk.with_clock_map(second),
        Err(PreparedSpeechError::InvalidData)
    ));
}

#[test]
fn detached_chunk_map_survives_pruning_and_dropping_the_source_window() {
    let mut clock = Clock::new(16_000, 160, 2_000_000);
    let map = clock.snapshot(2_000_000, 2_010_000);
    let chunk = chunk(1, 2_000_000, 2_010_000).with_clock_map(map).unwrap();
    clock.window.prune_before(2_010_000, &clock.cancel).unwrap();
    drop(clock);
    let native = chunk
        .source_clock()
        .unwrap()
        .map_interval(identity(), 2_001_000, 2_009_000)
        .unwrap();
    assert_eq!((native.start_us(), native.end_us()), (2_001_000, 2_009_000));
    let request = request(chunk);
    let mut merger = merger();
    merger.register(&request).unwrap();
    merger
        .complete(&request, reply(&request, 1_000, 8_000, "detached"))
        .unwrap();
    assert_eq!(merger.finish().unwrap().cues()[0].start_us, 2_001_000);
}

#[test]
fn four_delayed_chunks_keep_native_time_after_forty_seconds_of_window_pruning() {
    let mut clock = Clock::new(44_100, 22_051, 3_000_000);
    let mut requests = Vec::new();
    let mut expected = Vec::new();
    for sequence in 1..=4 {
        let start = 3_000_000 + (sequence - 1) * 10_000_000;
        let request = mapped_request(&mut clock, sequence, start, start + 10_000_000);
        let interval = request
            .chunk()
            .source_clock()
            .unwrap()
            .map_interval(identity(), start + 100_000, start + 300_000)
            .unwrap();
        expected.push((interval.start_us(), interval.end_us() - interval.start_us()));
        requests.push(request);
    }
    assert!(clock.sample_end > 43_000_000);
    drop(clock);
    let mut merger = merger();
    for request in &requests {
        merger.register(request).unwrap();
    }
    for index in [3, 1, 0, 2] {
        let request = &requests[index];
        merger
            .complete(
                request,
                reply(request, 100_000, 200_000, &format!("part-{index}")),
            )
            .unwrap();
        if index == 3 || index == 1 {
            assert!(merger.context(1).is_empty());
        }
    }
    let transcript = merger.finish().unwrap();
    assert_eq!(transcript.cues().len(), 4);
    for (index, cue) in transcript.cues().iter().enumerate() {
        assert_eq!((cue.start_us, cue.duration_us), expected[index]);
        assert_eq!(cue.text.as_ref(), format!("part-{index}"));
    }
    assert!(transcript.cues()[3].start_us < 33_100_000);
}

#[test]
fn partial_native_intervals_retain_observed_packet_shift_without_clipping() {
    let mut clock = Clock::new(16_000, 160, 0);
    clock.push_at(160, 0);
    clock.push_at(160, 9_999);
    clock.push_at(160, 19_998);
    let request = mapped_request(&mut clock, 1, 0, 30_000);
    let mut merger = merger();
    merger.register(&request).unwrap();
    merger
        .complete(&request, reply(&request, 5_000, 20_000, "true endpoints"))
        .unwrap();
    let transcript = merger.finish().unwrap();
    assert_eq!(transcript.cues()[0].start_us, 5_000);
    assert_eq!(transcript.cues()[0].duration_us, 19_998);
}

#[test]
fn full_audio_context_evidence_does_not_make_context_cues_owned_body() {
    let mut clock = Clock::new(16_000, 1_600, 0);
    let map = clock.snapshot(0, 100_000);
    let chunk = bare_chunk(1, 1, 0, 0, 50_000, 100_000, false)
        .with_clock_map(map)
        .unwrap();
    let request = request(chunk);
    let mut merger = merger();
    merger.register(&request).unwrap();
    assert_eq!(
        merger.complete(&request, reply(&request, 60_000, 10_000, "context only")),
        Err(PreparedSpeechError::BoundaryMismatch)
    );
    assert!(merger.context(1).is_empty());
    assert!(matches!(
        merger.finish(),
        Err(PreparedSpeechError::BoundaryMismatch)
    ));
}

#[test]
fn a_sentence_crossing_body_into_real_context_is_rejected_before_mapping() {
    let mut clock = Clock::new(16_000, 1_600, 0);
    let map = clock.snapshot(0, 100_000);
    let request = request(
        bare_chunk(1, 1, 0, 0, 50_000, 100_000, false)
            .with_clock_map(map)
            .unwrap(),
    );
    let mut merger = merger();
    merger.register(&request).unwrap();
    assert_eq!(
        merger.complete(&request, reply(&request, 49_000, 2_000, "crossing")),
        Err(PreparedSpeechError::BoundaryMismatch)
    );
    assert!(merger.cues.is_empty());
}

#[test]
fn left_overlap_context_cannot_reappear_as_owned_text_in_the_next_chunk() {
    let mut clock = Clock::new(44_100, 22_051, 0);
    let first_map = clock.snapshot(0, 10_500_000);
    let first = request(
        bare_chunk(1, 1, 0, 0, 10_000_000, 10_500_000, false)
            .with_clock_map(first_map)
            .unwrap(),
    );
    let second_map = clock.snapshot(9_500_000, 20_000_000);
    let second = request(
        bare_chunk(2, 1, 9_500_000, 10_000_000, 20_000_000, 20_000_000, false)
            .with_clock_map(second_map)
            .unwrap(),
    );
    let mut merger = merger();
    merger.register(&first).unwrap();
    merger.register(&second).unwrap();
    merger
        .complete(&first, reply(&first, 100_000, 200_000, "first body"))
        .unwrap();
    assert_eq!(
        merger.complete(
            &second,
            reply(&second, 10_000, 10_000, "left context duplicate")
        ),
        Err(PreparedSpeechError::BoundaryMismatch)
    );
    assert!(merger.context(1).is_empty());
    assert!(merger.cues.is_empty());
}

#[test]
fn another_equal_snapshot_with_the_same_stamp_cannot_complete_a_registered_request() {
    let mut clock = Clock::new(16_000, 160, 0);
    let first = mapped_request(&mut clock, 1, 0, 10_000);
    let replacement = mapped_request(&mut clock, 1, 0, 10_000);
    let mut merger = merger();
    merger.register(&first).unwrap();
    assert_eq!(
        merger.complete(&replacement, reply(&replacement, 0, 10_000, "replacement")),
        Err(PreparedSpeechError::Stale)
    );
    assert!(merger.pending.is_empty());
    assert!(merger.context(1).is_empty());
}

#[test]
fn direct_and_mapped_requests_cannot_mix_in_either_direction() {
    for first_mapped in [false, true] {
        let mut clock = Clock::new(16_000, 160, 0);
        let first = if first_mapped {
            mapped_request(&mut clock, 1, 0, 10_000)
        } else {
            request(chunk(1, 0, 10_000))
        };
        let second = if first_mapped {
            request(chunk(2, 10_000, 20_000))
        } else {
            mapped_request(&mut clock, 2, 10_000, 20_000)
        };
        let mut merger = merger();
        merger.register(&first).unwrap();
        merger
            .complete(&first, reply(&first, 0, 5_000, "earlier private text"))
            .unwrap();
        assert_eq!(
            merger.register(&second),
            Err(PreparedSpeechError::InvalidData)
        );
        assert!(merger.context(1).is_empty());
        assert!(merger.cues.is_empty());
    }
}

#[test]
fn the_same_session_cannot_swap_source_packet_rows_between_chunks() {
    let mut original = Clock::new(16_000, 160, 0);
    let mut replacement = Clock::new(16_000, 320, 0);
    let first = mapped_request(&mut original, 1, 0, 10_000);
    let second = mapped_request(&mut replacement, 2, 10_000, 20_000);
    let mut merger = merger();
    merger.register(&first).unwrap();
    assert_eq!(
        merger.register(&second),
        Err(PreparedSpeechError::InvalidData)
    );
}

#[test]
fn the_same_session_cannot_swap_source_run_origin_between_chunks() {
    let mut original = Clock::new(16_000, 320, 0);
    let mut replacement = Clock::new(16_000, 320, 5);
    let first = mapped_request(&mut original, 1, 0, 10_000);
    let second = mapped_request(&mut replacement, 2, 10_000, 20_000);
    let mut merger = merger();
    merger.register(&first).unwrap();
    assert_eq!(
        merger.register(&second),
        Err(PreparedSpeechError::InvalidData)
    );
}

#[test]
fn an_unchanged_chunk_run_cannot_hide_a_real_source_run_transition() {
    let mut clock = Clock::new(16_000, 160, 0);
    let first = mapped_request(&mut clock, 1, 0, 10_000);
    clock.push_at(160, 1_000_000);
    let second = mapped_request(&mut clock, 2, 1_000_000, 1_010_000);
    let mut merger = merger();
    merger.register(&first).unwrap();
    assert_eq!(
        merger.register(&second),
        Err(PreparedSpeechError::BoundaryMismatch)
    );
}

#[test]
fn a_new_chunk_run_requires_a_real_increasing_source_run() {
    let mut clock = Clock::new(16_000, 480, 0);
    let first = mapped_request(&mut clock, 1, 0, 10_000);
    let map = clock.snapshot(20_000, 30_000);
    let second = request(
        bare_chunk(2, 2, 20_000, 20_000, 30_000, 30_000, true)
            .with_clock_map(map)
            .unwrap(),
    );
    let mut merger = merger();
    merger.register(&first).unwrap();
    assert_eq!(
        merger.register(&second),
        Err(PreparedSpeechError::BoundaryMismatch)
    );
}

#[test]
fn chunk_runs_can_use_different_source_ids_and_skip_zero_output_source_runs() {
    let mut clock = Clock::new(192_000, 1_920, 0);
    clock.push_at(1, 0);
    clock.push_at(1_920, 1_000_000);
    let first = mapped_request(&mut clock, 1, 1_000_000, 1_010_000);
    assert_eq!(first.chunk().source_clock().unwrap().source_run_id(), 2);
    clock.push_at(1, 2_000_000);
    clock.push_at(1_920, 3_000_000);
    let map = clock.snapshot(3_000_000, 3_010_000);
    assert_eq!(map.source_run_id(), 4);
    let second = request(
        bare_chunk(2, 2, 3_000_000, 3_000_000, 3_010_000, 3_010_000, true)
            .with_clock_map(map)
            .unwrap(),
    );
    let mut merger = merger();
    merger.register(&first).unwrap();
    merger
        .complete(&first, reply(&first, 0, 5_000, "run two"))
        .unwrap();
    merger.register(&second).unwrap();
    assert!(merger.context(2).is_empty());
    merger
        .complete(&second, reply(&second, 0, 5_000, "run four"))
        .unwrap();
    let transcript = merger.finish().unwrap();
    assert_eq!(transcript.cues()[0].start_us, 1_000_000);
    assert_eq!(transcript.cues()[1].start_us, 3_000_000);
}

#[test]
fn native_one_microsecond_cue_overlap_is_rejected_and_never_clipped() {
    let mut clock = Clock::new(16_000, 160, 0);
    clock.push_at(160, 0);
    clock.push_at(160, 9_999);
    let first = mapped_request(&mut clock, 1, 0, 10_000);
    let second = mapped_request(&mut clock, 2, 10_000, 20_000);
    let mut merger = merger();
    merger.register(&first).unwrap();
    merger.register(&second).unwrap();
    merger
        .complete(&first, reply(&first, 0, 10_000, "first"))
        .unwrap();
    assert_eq!(
        merger.complete(&second, reply(&second, 0, 10_000, "overlap")),
        Err(PreparedSpeechError::InvalidData)
    );
    assert!(merger.context(1).is_empty());
    assert!(merger.cues.is_empty());
    assert!(matches!(
        merger.finish(),
        Err(PreparedSpeechError::InvalidData)
    ));
}

#[test]
fn cancellation_after_snapshot_creation_revokes_binding_and_pending_completion() {
    let mut clock = Clock::new(16_000, 320, 0);
    let request = mapped_request(&mut clock, 1, 0, 10_000);
    let spare = clock.snapshot(10_000, 20_000);
    let mut merger = merger();
    merger.register(&request).unwrap();
    clock.cancel.cancel();
    assert!(matches!(
        chunk(2, 10_000, 20_000).with_clock_map(spare),
        Err(PreparedSpeechError::Cancelled)
    ));
    assert_eq!(
        merger.complete(&request, reply(&request, 0, 5_000, "late")),
        Err(PreparedSpeechError::Cancelled)
    );
    assert!(merger.pending.is_empty());
    assert!(merger.context(1).is_empty());
    assert!(matches!(
        merger.finish(),
        Err(PreparedSpeechError::Cancelled)
    ));
}

#[test]
fn cancellation_after_a_successful_commit_prevents_final_document_handoff() {
    let mut clock = Clock::new(16_000, 160, 0);
    let request = mapped_request(&mut clock, 1, 0, 10_000);
    let mut merger = merger();
    merger.register(&request).unwrap();
    merger
        .complete(&request, reply(&request, 0, 5_000, "committed"))
        .unwrap();
    clock.cancel.cancel();
    assert!(matches!(
        merger.finish(),
        Err(PreparedSpeechError::Cancelled)
    ));
}

#[test]
fn buffered_silent_response_cannot_commit_revoked_source_evidence() {
    let mut clock = Clock::new(16_000, 160, 0);
    let first = mapped_request(&mut clock, 1, 0, 10_000);
    let creation = ResolverCancellation::default();
    clock.push();
    // Give only the second chunk its own preparation scope; the source window
    // and first request remain active when this completed slot is revoked.
    let second_map = clock
        .window
        .snapshot(identity(), 10_000, 20_000, &creation)
        .unwrap();
    let second = request(chunk(2, 10_000, 20_000).with_clock_map(second_map).unwrap());
    let mut merger = merger();
    merger.register(&first).unwrap();
    merger.register(&second).unwrap();
    let mut silence = reply(&second, 0, 5000, "synthetic context");
    silence.cues.clear();
    merger.complete(&second, silence).unwrap();
    creation.cancel();
    assert_eq!(
        merger.complete(&first, reply(&first, 0, 5000, "first")),
        Err(PreparedSpeechError::Cancelled)
    );
    assert!(merger.pending.is_empty());
    assert!(merger.cues.is_empty());
    assert!(merger.context(1).is_empty());
}

#[test]
fn committed_silence_reports_cancelled_before_no_speech_at_final_handoff() {
    let mut clock = Clock::new(16_000, 160, 0);
    let request = mapped_request(&mut clock, 1, 0, 10_000);
    let mut merger = merger();
    merger.register(&request).unwrap();
    let mut response = reply(&request, 0, 5000, "synthetic silence");
    response.cues.clear();
    merger.complete(&request, response).unwrap();
    clock.cancel.cancel();
    assert!(matches!(
        merger.finish(),
        Err(PreparedSpeechError::Cancelled)
    ));
}

#[test]
fn mapped_eof_body_can_consume_prior_context_without_extending_audio_end() {
    for same_audio_start in [false, true] {
        let mut clock = Clock::new(16_000, 320, 0);
        let first_map = clock.snapshot(0, 20_000);
        let first = request(
            bare_chunk(1, 1, 0, 0, 10_000, 20_000, false)
                .with_clock_map(first_map)
                .unwrap(),
        );
        let audio_start = if same_audio_start { 0 } else { 5_000 };
        let final_map = clock.snapshot(audio_start, 20_000);
        let tail = request(
            bare_chunk(2, 1, audio_start, 10_000, 20_000, 20_000, false)
                .with_clock_map(final_map)
                .unwrap(),
        );
        assert_eq!(first.chunk().audio_end_us(), tail.chunk().audio_end_us());
        let mut merger = merger();
        merger.register(&first).unwrap();
        merger.register(&tail).unwrap();
        merger
            .complete(
                &tail,
                reply(&tail, 10_000 - audio_start, 10_000, "EOF context body"),
            )
            .unwrap();
        merger
            .complete(&first, reply(&first, 0, 10_000, "first body"))
            .unwrap();
        let transcript = merger.finish().unwrap();
        assert_eq!(transcript.cues().len(), 2);
        assert_eq!(
            (
                transcript.cues()[0].start_us,
                transcript.cues()[0].duration_us
            ),
            (0, 10_000)
        );
        assert_eq!(
            (
                transcript.cues()[1].start_us,
                transcript.cues()[1].duration_us
            ),
            (10_000, 10_000)
        );
    }
}

#[test]
fn an_earlier_committed_creation_scope_stays_binding_until_final_handoff() {
    let mut clock = Clock::new(16_000, 160, 0);
    clock.push();
    clock.push();
    let earlier = ResolverCancellation::default();
    let later = ResolverCancellation::default();
    let a = clock
        .window
        .snapshot(identity(), 0, 10_000, &earlier)
        .unwrap();
    let b = clock
        .window
        .snapshot(identity(), 10_000, 20_000, &later)
        .unwrap();
    let first = request(chunk(1, 0, 10_000).with_clock_map(a).unwrap());
    let second = request(chunk(2, 10_000, 20_000).with_clock_map(b).unwrap());
    let mut merger = merger();
    for request in [&first, &second] {
        merger.register(request).unwrap();
        merger
            .complete(request, reply(request, 0, 5000, "committed scoped text"))
            .unwrap();
    }
    earlier.cancel();
    assert!(!later.is_cancelled());
    assert!(!clock.cancel.is_cancelled());
    assert!(merger.context(1).is_empty());
    assert!(matches!(
        merger.finish(),
        Err(PreparedSpeechError::Cancelled)
    ));
}

#[test]
fn distinct_clock_scope_capacity_is_bounded_and_reuse_does_not_grow_it() {
    let mut clock = Clock::new(16_000, 160, 0);
    let mut merger = merger();
    for sequence in 1..=MAX_PREPARED_CLOCK_SCOPES as u64 {
        clock.push();
        let start = (sequence - 1) * 10_000;
        let scope = ResolverCancellation::default();
        let map = clock
            .window
            .snapshot(identity(), start, start + 10_000, &scope)
            .unwrap();
        let request = request(
            chunk(sequence, start, start + 10_000)
                .with_clock_map(map)
                .unwrap(),
        );
        if sequence == MAX_PREPARED_CLOCK_SCOPES as u64 {
            assert_eq!(merger.clock_scopes.len(), MAX_PREPARED_CLOCK_SCOPES);
            assert_eq!(
                merger.register(&request),
                Err(PreparedSpeechError::Overloaded)
            );
            assert!(merger.clock_scopes.is_empty());
            assert!(merger.cues.is_empty());
        } else {
            merger.register(&request).unwrap();
            merger
                .complete(&request, reply(&request, 0, 5000, "bounded scope"))
                .unwrap();
            assert_eq!(merger.clock_scopes.len(), sequence as usize + 1);
        }
    }
    let mut clock = Clock::new(16_000, 160, 0);
    let mut reused =
        super::PreparedTranscriptMerger::new(identity(), SourceLanguage::English).unwrap();
    for sequence in 1..=20 {
        let start = (sequence - 1) * 10_000;
        let request = mapped_request(&mut clock, sequence, start, start + 10_000);
        reused.register(&request).unwrap();
        reused
            .complete(&request, reply(&request, 0, 5000, "reused session scope"))
            .unwrap();
        assert_eq!(reused.clock_scopes.len(), 1);
    }
    assert_eq!(reused.finish().unwrap().cues().len(), 20);
}

#[test]
fn a_later_body_can_extend_left_context_without_rewinding_owned_text() {
    let mut clock = Clock::new(16_000, 160, 0);
    clock.push();
    clock.push();
    clock.push();
    let mut merger = merger();
    for (sequence, audio_start, body_start, body_end) in [
        (1, 0, 0, 10_000),
        (2, 10_000, 10_000, 20_000),
        (3, 0, 20_000, 30_000),
    ] {
        let map = clock
            .window
            .snapshot(identity(), audio_start, body_end, &clock.cancel)
            .unwrap();
        let request = request(
            bare_chunk(
                sequence,
                1,
                audio_start,
                body_start,
                body_end,
                body_end,
                false,
            )
            .with_clock_map(map)
            .unwrap(),
        );
        merger.register(&request).unwrap();
        merger
            .complete(
                &request,
                reply(&request, body_start - audio_start, 10_000, "owned body"),
            )
            .unwrap();
    }
    let transcript = merger.finish().unwrap();
    assert_eq!(
        transcript
            .cues()
            .iter()
            .map(|cue| cue.start_us)
            .collect::<Vec<_>>(),
        vec![0, 10_000, 20_000]
    );
}

struct MockMappedRecognizer {
    observed: Mutex<Vec<(u64, u64, u64)>>,
    completed: Mutex<Vec<u64>>,
    later_finished: Semaphore,
}
impl PreparedSpeechRecognizer for MockMappedRecognizer {
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
            let stamp = request.chunk().stamp();
            self.observed.lock().unwrap().push((
                stamp.sequence,
                stamp.audio_start_us,
                request.chunk().audio_end_us(),
            ));
            assert!(request.chunk().source_clock().is_some());
            if stamp.sequence == 1 {
                tokio::select! {
                    _ = cancel.cancelled() => return Err(PreparedSpeechError::Cancelled),
                    permits = self.later_finished.acquire_many(3) => permits.unwrap().forget(),
                }
            }
            if cancel.is_cancelled() {
                return Err(PreparedSpeechError::Cancelled);
            }
            self.completed.lock().unwrap().push(stamp.sequence);
            if stamp.sequence != 1 {
                self.later_finished.add_permits(1);
            }
            Ok(reply(
                request,
                100_000,
                200_000,
                &format!("mock-{}", stamp.sequence),
            ))
        })
    }
}

#[tokio::test]
async fn recognition_keeps_sample_clock_provider_requests_and_restores_native_final_document() {
    let mut clock = Clock::new(44_100, 22_051, 1_000_000);
    let mut chunks = Vec::new();
    let mut expected = Vec::new();
    for sequence in 1..=4 {
        let start = 1_000_000 + (sequence - 1) * 10_000_000;
        let map = clock.snapshot(start, start + 10_000_000);
        let interval = map
            .map_interval(identity(), start + 100_000, start + 300_000)
            .unwrap();
        expected.push((interval.start_us(), interval.end_us() - interval.start_us()));
        chunks.push(
            chunk(sequence, start, start + 10_000_000)
                .with_clock_map(map)
                .unwrap(),
        );
    }
    drop(clock);
    let provider = MockMappedRecognizer {
        observed: Mutex::new(Vec::new()),
        completed: Mutex::new(Vec::new()),
        later_finished: Semaphore::new(0),
    };
    let cancel = ResolverCancellation::default();
    let mut session = PreparedRecognitionSession::new(
        identity(),
        SourceLanguage::English,
        PreparedSampleRate::Hz16000,
        &cancel,
    )
    .unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        session.recognize_batch(chunks, &provider),
    )
    .await
    .unwrap()
    .unwrap();
    let transcript = session.finish().unwrap();
    let mut observed = provider.observed.lock().unwrap().clone();
    observed.sort_by_key(|row| row.0);
    assert_eq!(
        observed,
        (1..=4)
            .map(|sequence| {
                let start = 1_000_000 + (sequence - 1) * 10_000_000;
                (sequence, start, start + 10_000_000)
            })
            .collect::<Vec<_>>()
    );
    assert_eq!(provider.completed.lock().unwrap().last(), Some(&1));
    for (index, cue) in transcript.cues().iter().enumerate() {
        assert_eq!((cue.start_us, cue.duration_us), expected[index]);
        assert_eq!(cue.text.as_ref(), format!("mock-{}", index + 1));
    }
    assert!(!cancel.is_cancelled());
}
