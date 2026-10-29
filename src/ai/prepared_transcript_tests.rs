//! Synthetic bounded final text; no provider, files, capture or playback.
use super::*;
use crate::{
    ai::prepared_speech::{PreparedSpeechChunk, PreparedSpeechCue},
    audio::capture::SessionId,
    media::prepared_audio::PreparedSampleRate,
};

fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(202),
        generation: 3,
    }
}
fn request_at(
    sequence: u64,
    run_id: u64,
    start: u64,
    discontinuity: bool,
    language: SourceLanguage,
) -> PreparedSpeechRequest {
    PreparedSpeechRequest::new(
        PreparedSpeechChunk::new(
            PreparedChunkStamp {
                identity: identity(),
                sequence,
                run_id,
                rate: PreparedSampleRate::Hz16000,
                audio_start_us: start,
                body_start_us: start,
                body_end_us: start + 10_000,
                discontinuity,
            },
            vec![0; 160],
        )
        .unwrap(),
        language,
        Vec::new(),
    )
    .unwrap()
}
fn request(sequence: u64) -> PreparedSpeechRequest {
    request_at(
        sequence,
        1,
        1_000_000 + (sequence - 1) * 10_000,
        false,
        SourceLanguage::English,
    )
}
fn reply(
    request: &PreparedSpeechRequest,
    language: Option<SpeechLanguage>,
    texts: Vec<Box<str>>,
) -> PreparedSpeechResponse {
    let stamp = request.chunk().stamp();
    PreparedSpeechResponse {
        identity: stamp.identity,
        sequence: stamp.sequence,
        run_id: stamp.run_id,
        language,
        cues: texts
            .into_iter()
            .enumerate()
            .map(|(index, text)| PreparedSpeechCue {
                start_us: index as u64,
                duration_us: 1,
                text,
            })
            .collect(),
    }
}
fn one(request: &PreparedSpeechRequest, text: &str) -> PreparedSpeechResponse {
    reply(request, Some(SpeechLanguage::English), vec![text.into()])
}
fn merger() -> PreparedTranscriptMerger {
    PreparedTranscriptMerger::new(identity(), SourceLanguage::English).unwrap()
}

#[test]
fn complete_document_keeps_true_provenance_and_absolute_media_time() {
    let request = request(1);
    let mut merger = merger();
    merger.register(&request).unwrap();
    let mut response = one(&request, "source text");
    response.cues[0].start_us = 123;
    response.cues[0].duration_us = 321;
    merger.complete(&request, response).unwrap();
    let transcript = merger.finish().unwrap();
    assert_eq!(transcript.identity(), identity());
    assert_eq!(transcript.language(), SpeechLanguage::English);
    assert_eq!(transcript.cues()[0].start_us, 1_000_123);
    assert_eq!(transcript.cues()[0].duration_us, 321);
    assert_eq!(transcript.cues()[0].text.as_ref(), "source text");
}

#[test]
fn out_of_order_batch_is_committed_only_as_a_contiguous_prefix() {
    let requests: Vec<_> = (1..=4).map(request).collect();
    let mut merger = merger();
    for request in &requests {
        merger.register(request).unwrap();
    }
    merger
        .complete(&requests[3], one(&requests[3], "four"))
        .unwrap();
    merger
        .complete(&requests[1], one(&requests[1], "two"))
        .unwrap();
    assert!(merger.context(1).is_empty());
    merger
        .complete(&requests[0], one(&requests[0], "one"))
        .unwrap();
    assert_eq!(
        merger
            .context(1)
            .iter()
            .map(|text| text.as_ref())
            .collect::<Vec<_>>(),
        ["one", "two"]
    );
    merger
        .complete(&requests[2], one(&requests[2], "three"))
        .unwrap();
    let document = merger.finish().unwrap();
    assert_eq!(
        document
            .cues()
            .iter()
            .map(|cue| cue.text.as_ref())
            .collect::<Vec<_>>(),
        ["one", "two", "three", "four"]
    );
    assert_eq!(document.cues()[3].start_us, 1_030_000);
}

#[test]
fn successful_commit_frees_pending_capacity_without_retaining_requests() {
    let mut merger = merger();
    for sequence in 1..=20 {
        let request = request(sequence);
        merger.register(&request).unwrap();
        merger.complete(&request, one(&request, "one")).unwrap();
        assert!(merger.pending.is_empty());
    }
    assert_eq!(merger.finish().unwrap().cues().len(), 20);
}

#[test]
fn saturation_is_terminal_and_clears_even_completed_out_of_order_responses() {
    let requests: Vec<_> = (1..=5).map(request).collect();
    let mut merger = merger();
    for request in &requests[..4] {
        merger.register(request).unwrap();
    }
    merger
        .complete(&requests[3], one(&requests[3], "private pending"))
        .unwrap();
    assert_eq!(
        merger.register(&requests[4]),
        Err(PreparedSpeechError::Overloaded)
    );
    assert!(merger.pending.is_empty());
    assert!(merger.cues.is_empty());
    assert!(merger.context(1).is_empty());
    assert_eq!(
        merger.complete(&requests[0], one(&requests[0], "late")),
        Err(PreparedSpeechError::Overloaded)
    );
    assert!(matches!(
        merger.finish(),
        Err(PreparedSpeechError::Overloaded)
    ));
}

#[test]
fn initial_registration_requires_sequence_one_run_one_and_no_gap_marker() {
    for (sequence, run, discontinuity) in [(2, 1, false), (1, 2, false), (1, 1, true)] {
        let mut merger = merger();
        let request = request_at(
            sequence,
            run,
            1_000_000,
            discontinuity,
            SourceLanguage::English,
        );
        assert_eq!(
            merger.register(&request),
            Err(PreparedSpeechError::InvalidData)
        );
        assert!(matches!(
            merger.finish(),
            Err(PreparedSpeechError::InvalidData)
        ));
    }
}

#[test]
fn sequence_skip_duplicate_and_same_run_gap_are_rejected() {
    for next in [
        request(1),
        request(3),
        request_at(2, 1, 1_010_002, false, SourceLanguage::English),
        request_at(2, 1, 1_009_998, false, SourceLanguage::English),
        request_at(2, 1, 1_010_000, true, SourceLanguage::English),
    ] {
        let mut merger = merger();
        merger.register(&request(1)).unwrap();
        assert_eq!(
            merger.register(&next),
            Err(PreparedSpeechError::InvalidData)
        );
        assert!(merger.pending.is_empty());
    }
}

#[test]
fn one_microsecond_body_rounding_is_tolerated_without_changing_cue_times() {
    let first = request(1);
    for next_start in [1_009_999, 1_010_001] {
        let second = request_at(2, 1, next_start, false, SourceLanguage::English);
        let mut merger = merger();
        merger.register(&first).unwrap();
        merger.register(&second).unwrap();
        merger.complete(&first, one(&first, "one")).unwrap();
        merger.complete(&second, one(&second, "two")).unwrap();
        assert_eq!(merger.finish().unwrap().cues()[1].start_us, next_start);
    }
}

#[test]
fn new_run_requires_increment_gap_flag_and_forward_audio_time() {
    for next in [
        request_at(2, 2, 1_020_000, false, SourceLanguage::English),
        request_at(2, 3, 1_020_000, true, SourceLanguage::English),
        request_at(2, 2, 1_009_999, true, SourceLanguage::English),
    ] {
        let mut merger = merger();
        merger.register(&request(1)).unwrap();
        assert_eq!(
            merger.register(&next),
            Err(PreparedSpeechError::InvalidData)
        );
    }
}

#[test]
fn actual_gap_has_separate_context_and_no_synthetic_silence() {
    let first = request(1);
    let second = request_at(2, 2, 2_000_000, true, SourceLanguage::English);
    let mut merger = merger();
    merger.register(&first).unwrap();
    merger.register(&second).unwrap();
    merger.complete(&first, one(&first, "old run")).unwrap();
    assert_eq!(merger.context(1)[0].as_ref(), "old run");
    assert!(merger.context(2).is_empty());
    merger.complete(&second, one(&second, "new run")).unwrap();
    assert!(merger.context(1).is_empty());
    assert_eq!(merger.context(2)[0].as_ref(), "new run");
    let transcript = merger.finish().unwrap();
    assert_eq!(transcript.cues().len(), 2);
    assert_eq!(transcript.cues()[1].start_us, 2_000_000);
}

#[test]
fn foreign_identity_and_selected_language_registration_cannot_enter_session() {
    let original = request(1);
    let mut stamp = original.chunk().stamp();
    stamp.identity.generation += 1;
    let foreign = PreparedSpeechRequest::new(
        PreparedSpeechChunk::new(stamp, vec![0; 160]).unwrap(),
        SourceLanguage::English,
        Vec::new(),
    )
    .unwrap();
    let different_language = request_at(1, 1, 1_000_000, false, SourceLanguage::Japanese);
    for request in [foreign, different_language] {
        let mut merger = merger();
        assert_eq!(merger.register(&request), Err(PreparedSpeechError::Stale));
    }
    let mut invalid = identity();
    invalid.session.0 = 0;
    assert!(matches!(
        PreparedTranscriptMerger::new(invalid, SourceLanguage::Auto),
        Err(PreparedSpeechError::InvalidData)
    ));
    invalid = identity();
    invalid.generation = 0;
    assert!(matches!(
        PreparedTranscriptMerger::new(invalid, SourceLanguage::Auto),
        Err(PreparedSpeechError::InvalidData)
    ));
}

#[test]
fn completion_must_match_registered_stamp_language_and_audio_extent() {
    for variation in 0..3 {
        let original = request(1);
        let mut stamp = original.chunk().stamp();
        if variation == 0 {
            stamp.body_end_us -= 1;
        }
        let altered = PreparedSpeechRequest::new(
            PreparedSpeechChunk::new(stamp, vec![0; if variation == 2 { 161 } else { 160 }])
                .unwrap(),
            if variation == 1 {
                SourceLanguage::Auto
            } else {
                SourceLanguage::English
            },
            Vec::new(),
        )
        .unwrap();
        let mut merger = merger();
        merger.register(&original).unwrap();
        assert_eq!(
            merger.complete(&altered, one(&altered, "altered")),
            Err(PreparedSpeechError::Stale)
        );
        assert!(merger.pending.is_empty());
    }
}

#[test]
fn duplicate_pending_or_committed_completion_never_returns_a_partial_document() {
    for pending_duplicate in [true, false] {
        let first = request(1);
        let second = request(2);
        let mut merger = merger();
        merger.register(&first).unwrap();
        merger.register(&second).unwrap();
        let target = if pending_duplicate { &second } else { &first };
        merger.complete(target, one(target, "one")).unwrap();
        assert_eq!(
            merger.complete(target, one(target, "duplicate")),
            Err(PreparedSpeechError::Stale)
        );
        assert!(merger.pending.is_empty());
        assert!(merger.cues.is_empty());
        assert!(matches!(merger.finish(), Err(PreparedSpeechError::Stale)));
    }
}

#[test]
fn unregistered_and_response_identity_mismatch_are_terminal() {
    let request = request(1);
    let mut merger = merger();
    assert_eq!(
        merger.complete(&request, one(&request, "unregistered")),
        Err(PreparedSpeechError::Stale)
    );
    let mut merger = PreparedTranscriptMerger::new(identity(), SourceLanguage::English).unwrap();
    merger.register(&request).unwrap();
    let mut response = one(&request, "old session");
    response.identity.session.0 += 1;
    assert_eq!(
        merger.complete(&request, response),
        Err(PreparedSpeechError::Stale)
    );
    assert!(merger.pending.is_empty());
}

#[test]
fn auto_detection_waits_for_nonempty_committed_result_and_rejects_mixed_languages() {
    let first = request_at(1, 1, 1_000_000, false, SourceLanguage::Auto);
    let second = request_at(2, 1, 1_010_000, false, SourceLanguage::Auto);
    let third = request_at(3, 1, 1_020_000, false, SourceLanguage::Auto);
    let mut merger = PreparedTranscriptMerger::new(identity(), SourceLanguage::Auto).unwrap();
    for request in [&first, &second, &third] {
        merger.register(request).unwrap();
    }
    merger
        .complete(
            &first,
            reply(&first, Some(SpeechLanguage::Japanese), Vec::new()),
        )
        .unwrap();
    assert_eq!(merger.detected, None);
    merger
        .complete(
            &third,
            reply(&third, Some(SpeechLanguage::Japanese), vec!["three".into()]),
        )
        .unwrap();
    assert_eq!(merger.detected, None);
    assert_eq!(
        merger.complete(
            &second,
            reply(&second, Some(SpeechLanguage::English), vec!["two".into()])
        ),
        Err(PreparedSpeechError::MixedLanguages)
    );
    assert!(merger.cues.is_empty());
    assert!(merger.pending.is_empty());
    assert!(matches!(
        merger.finish(),
        Err(PreparedSpeechError::MixedLanguages)
    ));
}

#[test]
fn silence_is_a_completed_chunk_but_all_silence_cannot_be_ready() {
    let first = request(1);
    let second = request(2);
    let mut merger = merger();
    merger.register(&first).unwrap();
    merger.register(&second).unwrap();
    merger.complete(&second, one(&second, "speech")).unwrap();
    merger
        .complete(&first, reply(&first, None, Vec::new()))
        .unwrap();
    assert_eq!(merger.finish().unwrap().cues().len(), 1);
    let mut merger = PreparedTranscriptMerger::new(identity(), SourceLanguage::English).unwrap();
    merger.register(&first).unwrap();
    merger
        .complete(&first, reply(&first, None, Vec::new()))
        .unwrap();
    assert!(matches!(
        merger.finish(),
        Err(PreparedSpeechError::NoSpeech)
    ));
    assert!(matches!(
        PreparedTranscriptMerger::new(identity(), SourceLanguage::English)
            .unwrap()
            .finish(),
        Err(PreparedSpeechError::NoSpeech)
    ));
}

#[test]
fn incomplete_batch_cannot_finish_with_a_committed_prefix() {
    let first = request(1);
    let second = request(2);
    let mut merger = merger();
    merger.register(&first).unwrap();
    merger.register(&second).unwrap();
    merger.complete(&first, one(&first, "prefix")).unwrap();
    assert!(matches!(
        merger.finish(),
        Err(PreparedSpeechError::InvalidData)
    ));
}

#[test]
fn context_is_only_last_eight_committed_cues_in_the_current_run() {
    let request = request(1);
    let mut merger = merger();
    merger.register(&request).unwrap();
    merger
        .complete(
            &request,
            reply(
                &request,
                Some(SpeechLanguage::English),
                (0..12)
                    .map(|index| format!("cue {index}").into_boxed_str())
                    .collect(),
            ),
        )
        .unwrap();
    let context = merger.context(1);
    assert_eq!(context.len(), 8);
    assert_eq!(context[0].as_ref(), "cue 4");
    assert_eq!(context[7].as_ref(), "cue 11");
    assert!(merger.context(2).is_empty());
}

#[test]
fn context_exact_byte_bound_is_not_a_full_transcript_copy() {
    let request = request(1);
    let mut merger = merger();
    merger.register(&request).unwrap();
    merger
        .complete(
            &request,
            reply(
                &request,
                Some(SpeechLanguage::English),
                (0..9).map(|_| "x".repeat(8192).into_boxed_str()).collect(),
            ),
        )
        .unwrap();
    let context = merger.context(1);
    assert_eq!(context.len(), 8);
    assert_eq!(
        context.iter().map(|text| text.len()).sum::<usize>(),
        MAX_ASR_CONTEXT_BYTES
    );
    assert_eq!(merger.finish().unwrap().cues().len(), 9);
}

#[test]
fn malformed_completion_clears_private_committed_and_pending_text() {
    let first = request(1);
    let second = request(2);
    let mut merger = merger();
    merger.register(&first).unwrap();
    merger.register(&second).unwrap();
    merger
        .complete(&first, one(&first, "private committed"))
        .unwrap();
    assert_eq!(
        merger.complete(&second, one(&second, "\0")),
        Err(PreparedSpeechError::InvalidData)
    );
    assert!(merger.cues.is_empty());
    assert!(merger.pending.is_empty());
    assert_eq!(merger.bytes, 0);
    assert!(merger.context(1).is_empty());
    assert_eq!(
        merger.register(&request(3)),
        Err(PreparedSpeechError::InvalidData)
    );
    assert!(matches!(
        merger.finish(),
        Err(PreparedSpeechError::InvalidData)
    ));
}

#[test]
fn actual_cross_chunk_cue_overlap_is_rejected_even_when_body_rounding_is_valid() {
    let first = request(1);
    let second = request_at(2, 1, 1_009_999, false, SourceLanguage::English);
    let mut merger = merger();
    merger.register(&first).unwrap();
    merger.register(&second).unwrap();
    let mut response = one(&first, "full body");
    response.cues[0].duration_us = 10_000;
    merger.complete(&first, response).unwrap();
    assert_eq!(
        merger.complete(&second, one(&second, "overlap")),
        Err(PreparedSpeechError::InvalidData)
    );
    assert!(merger.cues.is_empty());
}

#[test]
fn document_cue_limit_accepts_8192_then_rejects_one_more_without_partial_ready() {
    let mut merger = merger();
    for sequence in 1..=16 {
        let request = request(sequence);
        merger.register(&request).unwrap();
        merger
            .complete(
                &request,
                reply(
                    &request,
                    Some(SpeechLanguage::English),
                    vec!["x".into(); 512],
                ),
            )
            .unwrap();
    }
    assert_eq!(merger.cues.len(), MAX_ASR_DOCUMENT_CUES);
    let request = request(17);
    merger.register(&request).unwrap();
    assert_eq!(
        merger.complete(&request, one(&request, "overflow")),
        Err(PreparedSpeechError::InvalidData)
    );
    assert!(merger.cues.is_empty());
    assert!(matches!(
        merger.finish(),
        Err(PreparedSpeechError::InvalidData)
    ));
}

#[test]
fn document_byte_limit_accepts_four_mib_then_rejects_one_more_without_partial_ready() {
    let mut merger = merger();
    for sequence in 1..=32 {
        let request = request(sequence);
        merger.register(&request).unwrap();
        merger
            .complete(
                &request,
                reply(
                    &request,
                    Some(SpeechLanguage::English),
                    vec!["x".repeat(8192).into_boxed_str(); 16],
                ),
            )
            .unwrap();
    }
    assert_eq!(merger.bytes, MAX_ASR_DOCUMENT_BYTES);
    let request = request(33);
    merger.register(&request).unwrap();
    assert_eq!(
        merger.complete(&request, one(&request, "x")),
        Err(PreparedSpeechError::InvalidData)
    );
    assert!(merger.cues.is_empty());
    assert!(merger.pending.is_empty());
    assert!(matches!(
        merger.finish(),
        Err(PreparedSpeechError::InvalidData)
    ));
}

#[test]
fn checked_registration_counters_cannot_wrap_to_accept_another_chunk() {
    let mut merger = merger();
    let mut previous = request(1).chunk().stamp();
    previous.sequence = u64::MAX;
    merger.last_registered = Some(previous);
    merger.last_registered_audio_end_us = Some(previous.body_end_us);
    assert_eq!(
        merger.register(&request(1)),
        Err(PreparedSpeechError::InvalidData)
    );
    let mut merger = PreparedTranscriptMerger::new(identity(), SourceLanguage::English).unwrap();
    previous.sequence = 1;
    previous.run_id = u64::MAX;
    merger.last_registered = Some(previous);
    merger.last_registered_audio_end_us = Some(previous.body_end_us);
    let request = request_at(2, 1, 1_020_000, true, SourceLanguage::English);
    assert_eq!(
        merger.register(&request),
        Err(PreparedSpeechError::InvalidData)
    );
}

#[test]
fn first_chunk_of_each_run_cannot_claim_audio_from_before_its_body() {
    for is_new_run in [false, true] {
        let mut merger = merger();
        if is_new_run {
            let first = request(1);
            merger.register(&first).unwrap();
            merger.complete(&first, one(&first, "old run")).unwrap();
        }
        let original = request_at(
            if is_new_run { 2 } else { 1 },
            if is_new_run { 2 } else { 1 },
            if is_new_run { 2_000_000 } else { 1_000_000 },
            is_new_run,
            SourceLanguage::English,
        );
        let mut stamp = original.chunk().stamp();
        stamp.body_start_us += 500;
        let prefixed = PreparedSpeechRequest::new(
            PreparedSpeechChunk::new(stamp, vec![0; 160]).unwrap(),
            SourceLanguage::English,
            Vec::new(),
        )
        .unwrap();
        assert_eq!(
            merger.register(&prefixed),
            Err(PreparedSpeechError::InvalidData)
        );
        assert!(merger.cues.is_empty());
        assert!(merger.pending.is_empty());
    }
}

#[test]
fn new_run_requires_a_real_gap_larger_than_timestamp_rounding() {
    for gap in [0, 1] {
        let first = request(1);
        let second = request_at(
            2,
            2,
            first.chunk().stamp().body_end_us + gap,
            true,
            SourceLanguage::English,
        );
        let mut merger = merger();
        merger.register(&first).unwrap();
        merger.complete(&first, one(&first, "old context")).unwrap();
        assert_eq!(
            merger.register(&second),
            Err(PreparedSpeechError::InvalidData)
        );
        assert!(merger.context(1).is_empty());
        assert!(merger.pending.is_empty());
    }
    let first = request(1);
    let second = request_at(
        2,
        2,
        first.chunk().stamp().body_end_us + 2,
        true,
        SourceLanguage::English,
    );
    let mut merger = merger();
    merger.register(&first).unwrap();
    merger.register(&second).unwrap();
    merger.complete(&first, one(&first, "old run")).unwrap();
    merger.complete(&second, one(&second, "new run")).unwrap();
    assert_eq!(merger.finish().unwrap().cues()[1].start_us, 1_010_002);
}

#[test]
fn later_same_run_chunk_keeps_valid_overlap_and_owned_body_timing() {
    let first = request(1);
    let mut stamp = request(2).chunk().stamp();
    stamp.audio_start_us -= 1_000;
    let second = PreparedSpeechRequest::new(
        PreparedSpeechChunk::new(stamp, vec![0; 176]).unwrap(),
        SourceLanguage::English,
        Vec::new(),
    )
    .unwrap();
    let mut merger = merger();
    merger.register(&first).unwrap();
    merger.register(&second).unwrap();
    merger.complete(&first, one(&first, "one")).unwrap();
    let mut response = one(&second, "two");
    response.cues[0].start_us = 1_000;
    merger.complete(&second, response).unwrap();
    assert_eq!(merger.finish().unwrap().cues()[1].start_us, 1_010_000);
}

#[test]
fn pending_responses_discard_spare_cue_capacity_including_empty_silence() {
    for count in [0, 1] {
        let first = request(1);
        let second = request(2);
        let mut merger = merger();
        merger.register(&first).unwrap();
        merger.register(&second).unwrap();
        let mut response = if count == 0 {
            reply(&second, None, Vec::new())
        } else {
            one(&second, "pending")
        };
        let mut cues = Vec::with_capacity(4096);
        cues.extend(response.cues);
        assert!(cues.capacity() > cues.len());
        response.cues = cues;
        merger.complete(&second, response).unwrap();
        let stored = merger.pending.back().unwrap().response.as_ref().unwrap();
        assert_eq!(stored.cues.len(), count);
        assert_eq!(stored.cues.capacity(), count);
        merger.complete(&first, one(&first, "first")).unwrap();
        assert_eq!(merger.finish().unwrap().cues().len(), count + 1);
    }
}

#[test]
fn invalid_response_with_large_capacity_does_not_remain_in_failed_merger() {
    let first = request(1);
    let second = request(2);
    let mut merger = merger();
    merger.register(&first).unwrap();
    merger.register(&second).unwrap();
    merger.complete(&first, one(&first, "committed")).unwrap();
    let mut response = one(&second, "\0");
    let mut cues = Vec::with_capacity(4096);
    cues.extend(response.cues);
    response.cues = cues;
    assert_eq!(
        merger.complete(&second, response),
        Err(PreparedSpeechError::InvalidData)
    );
    assert!(merger.pending.is_empty());
    assert!(merger.cues.is_empty());
    assert!(matches!(
        merger.finish(),
        Err(PreparedSpeechError::InvalidData)
    ));
}

fn request_with_right_context() -> PreparedSpeechRequest {
    let stamp = request(1).chunk().stamp();
    PreparedSpeechRequest::new(
        PreparedSpeechChunk::new(stamp, vec![0; 176]).unwrap(),
        SourceLanguage::English,
        Vec::new(),
    )
    .unwrap()
}

#[test]
fn run_switch_cannot_skip_real_audio_retained_as_previous_right_context() {
    let first = request_with_right_context();
    assert_eq!(first.chunk().audio_end_us(), 1_011_000);
    let next_run = request_at(2, 2, 2_000_000, true, SourceLanguage::English);
    let mut merger = merger();
    merger.register(&first).unwrap();
    merger.complete(&first, one(&first, "first body")).unwrap();
    assert_eq!(
        merger.register(&next_run),
        Err(PreparedSpeechError::BoundaryMismatch)
    );
    assert!(merger.cues.is_empty());
    assert!(merger.pending.is_empty());
    assert!(matches!(
        merger.finish(),
        Err(PreparedSpeechError::BoundaryMismatch)
    ));
}

#[test]
fn finish_cannot_publish_a_transcript_when_final_right_context_has_no_body() {
    let first = request_with_right_context();
    let mut merger = merger();
    merger.register(&first).unwrap();
    merger
        .complete(&first, one(&first, "completed body"))
        .unwrap();
    assert!(merger.pending.is_empty());
    assert!(matches!(
        merger.finish(),
        Err(PreparedSpeechError::BoundaryMismatch)
    ));
}

#[test]
fn genuine_eof_tail_closes_context_for_finish_and_a_later_real_gap() {
    for with_gap in [false, true] {
        let first = request_with_right_context();
        let tail = PreparedSpeechRequest::new(
            PreparedSpeechChunk::new(
                PreparedChunkStamp {
                    identity: identity(),
                    sequence: 2,
                    run_id: 1,
                    rate: PreparedSampleRate::Hz16000,
                    audio_start_us: 1_009_000,
                    body_start_us: 1_010_000,
                    body_end_us: 1_011_000,
                    discontinuity: false,
                },
                vec![0; 32],
            )
            .unwrap(),
            SourceLanguage::English,
            Vec::new(),
        )
        .unwrap();
        assert_eq!(
            tail.chunk().audio_end_us(),
            tail.chunk().stamp().body_end_us
        );
        let mut merger = merger();
        merger.register(&first).unwrap();
        merger.register(&tail).unwrap();
        merger.complete(&first, one(&first, "body")).unwrap();
        let mut tail_response = one(&tail, "tail speech");
        tail_response.cues[0].start_us = 1_000;
        tail_response.cues[0].duration_us = 1_000;
        merger.complete(&tail, tail_response).unwrap();
        if with_gap {
            let next_run = request_at(3, 2, 2_000_000, true, SourceLanguage::English);
            merger.register(&next_run).unwrap();
            merger
                .complete(&next_run, one(&next_run, "new run"))
                .unwrap();
        }
        let transcript = merger.finish().unwrap();
        assert_eq!(transcript.cues().len(), if with_gap { 3 } else { 2 });
        assert_eq!(transcript.cues()[1].start_us, 1_010_000);
        assert_eq!(transcript.cues()[1].duration_us, 1_000);
    }
}
