//! Explicit malformed ASR owners are constructed only inside this child test
//! module. No public constructor, publication, file, provider or cache exists.
use super::*;
use crate::{audio::capture::SessionId, resolver::contracts::MAX_CUE_BYTES};

fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(1007),
        generation: 6,
    }
}
fn cue(start_us: u64, duration_us: u64, text: &str) -> PreparedTranscriptCue {
    PreparedTranscriptCue {
        start_us,
        duration_us,
        text: text.into(),
    }
}
fn transcript(cues: Vec<PreparedTranscriptCue>) -> PreparedTranscript {
    PreparedTranscript {
        identity: identity(),
        language: SpeechLanguage::English,
        cues,
    }
}

#[test]
fn validation_borrows_original_owner_and_preserves_getters_for_all_five_languages() {
    for language in SpeechLanguage::ALL {
        let mut original = transcript(vec![cue(137, 201, "one\n\ttwo"), cue(500, 25, "three")]);
        original.language = language;
        let cues_pointer = original.cues().as_ptr();
        let text_pointer = original.cues()[0].text.as_ptr();
        assert_eq!(original.validate(), Ok(()));
        assert_eq!(original.validate(), Ok(()));
        assert_eq!(original.identity(), identity());
        assert_eq!(original.language(), language);
        assert_eq!(original.cues().as_ptr(), cues_pointer);
        assert_eq!(original.cues()[0].text.as_ptr(), text_pointer);
        assert_eq!(original.cues()[0].text.as_ref(), "one\n\ttwo");
        assert_eq!(original.cues()[0].start_us, 137);
        assert_eq!(original.cues()[0].duration_us, 201);
    }
}

#[test]
fn zero_session_or_generation_is_invalid_but_structural_validation_does_not_guess_active_session() {
    for zero_session in [true, false] {
        let mut original = transcript(vec![cue(0, 1, "one")]);
        if zero_session {
            original.identity.session = SessionId(0);
        } else {
            original.identity.generation = 0;
        }
        assert_eq!(original.validate(), Err(PreparedSpeechError::InvalidData));
    }
    // A consumer's expected identity, rather than this structural validator,
    // decides whether a nonzero Session/generation belongs to current work.
    let mut other = transcript(vec![cue(0, 1, "one")]);
    other.identity.session = SessionId(1008);
    other.identity.generation = 7;
    assert_eq!(other.validate(), Ok(()));
}

#[test]
fn empty_transcript_cannot_enter_full_document_translation() {
    assert_eq!(
        transcript(Vec::new()).validate(),
        Err(PreparedSpeechError::InvalidData)
    );
}

#[test]
fn exact_cue_count_is_accepted_and_one_more_is_rejected() {
    let mut original = transcript(
        (0..MAX_ASR_DOCUMENT_CUES)
            .map(|index| cue(index as u64, 1, "x"))
            .collect(),
    );
    assert_eq!(original.validate(), Ok(()));
    original
        .cues
        .push(cue(MAX_ASR_DOCUMENT_CUES as u64, 1, "x"));
    assert_eq!(original.validate(), Err(PreparedSpeechError::InvalidData));
}

#[test]
fn utf8_document_bytes_accept_four_mib_then_reject_one_more_byte() {
    let text = format!("{}ab", "界".repeat(2730));
    assert_eq!(text.len(), MAX_CUE_BYTES);
    let count = MAX_ASR_DOCUMENT_BYTES / MAX_CUE_BYTES;
    let mut original = transcript(
        (0..count)
            .map(|index| cue(index as u64, 1, &text))
            .collect(),
    );
    assert_eq!(
        original
            .cues()
            .iter()
            .map(|cue| cue.text.len())
            .sum::<usize>(),
        MAX_ASR_DOCUMENT_BYTES
    );
    assert_eq!(original.validate(), Ok(()));
    original.cues.push(cue(count as u64, 1, "x"));
    assert_eq!(original.validate(), Err(PreparedSpeechError::InvalidData));
}

#[test]
fn per_cue_limit_counts_utf8_bytes_and_rejects_one_byte_overflow() {
    let exact = format!("{}ab", "界".repeat(2730));
    assert_eq!(transcript(vec![cue(0, 1, &exact)]).validate(), Ok(()));
    let exceeded = "界".repeat(2731);
    assert!(exceeded.chars().count() < MAX_CUE_BYTES);
    assert_eq!(exceeded.len(), MAX_CUE_BYTES + 1);
    assert_eq!(
        transcript(vec![cue(0, 1, &exceeded)]).validate(),
        Err(PreparedSpeechError::InvalidData)
    );
}

#[test]
fn asr_text_rejects_blank_and_controls_including_carriage_return() {
    for text in [
        "",
        " \n\t",
        "one\r\ntwo",
        "one\0two",
        "one\u{7f}two",
        "one\u{1b}two",
        "one\u{85}two",
    ] {
        assert_eq!(
            transcript(vec![cue(0, 1, text)]).validate(),
            Err(PreparedSpeechError::InvalidData)
        );
    }
    assert_eq!(transcript(vec![cue(0, 1, "one\n\ttwo")]).validate(), Ok(()));
}

#[test]
fn zero_duration_is_invalid_at_any_media_position() {
    for start in [0, 137, MAX_DECODED_MEDIA_DURATION_US] {
        assert_eq!(
            transcript(vec![cue(start, 0, "one")]).validate(),
            Err(PreparedSpeechError::InvalidData)
        );
    }
}

#[test]
fn exact_absolute_duration_bound_is_accepted_but_any_end_past_it_is_invalid() {
    assert_eq!(
        transcript(vec![cue(MAX_DECODED_MEDIA_DURATION_US - 1, 1, "one")]).validate(),
        Ok(())
    );
    for (start, duration) in [
        (MAX_DECODED_MEDIA_DURATION_US, 1),
        (MAX_DECODED_MEDIA_DURATION_US - 1, 2),
    ] {
        assert_eq!(
            transcript(vec![cue(start, duration, "one")]).validate(),
            Err(PreparedSpeechError::InvalidData)
        );
    }
}

#[test]
fn checked_timestamp_arithmetic_rejects_overflow_and_unbounded_duration() {
    for (start, duration) in [
        (u64::MAX, 1),
        (u64::MAX - 7, 8),
        (1, u64::MAX),
        (0, u64::MAX),
    ] {
        assert_eq!(
            transcript(vec![cue(start, duration, "one")]).validate(),
            Err(PreparedSpeechError::InvalidData)
        );
    }
}

#[test]
fn adjacent_or_gapped_finals_are_valid_but_overlap_backwards_and_duplicates_are_not() {
    for second_start in [10, 100] {
        assert_eq!(
            transcript(vec![cue(0, 10, "one"), cue(second_start, 5, "two")]).validate(),
            Ok(())
        );
    }
    for cues in [
        vec![cue(0, 10, "one"), cue(9, 5, "one microsecond overlap")],
        vec![cue(10, 10, "one"), cue(0, 5, "backwards")],
        vec![cue(0, 10, "one"), cue(0, 10, "duplicate")],
    ] {
        assert_eq!(
            transcript(cues).validate(),
            Err(PreparedSpeechError::InvalidData)
        );
    }
}
