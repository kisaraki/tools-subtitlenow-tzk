//! Sealed synthetic ASR merger output; no files, services, keys or playback.
use super::*;
use crate::{
    ai::{
        prepared_speech::{
            PreparedChunkStamp, PreparedSpeechChunk, PreparedSpeechCue, PreparedSpeechRequest,
            PreparedSpeechResponse,
        },
        prepared_transcript::{PreparedTranscript, PreparedTranscriptMerger},
        speech::SpeechLanguage,
    },
    audio::capture::SessionId,
    domain::SourceLanguage,
    media::prepared_audio::PreparedSampleRate,
};
const LINE: &str = "the synthetic subtitle belongs to this document";
const MEDIA_START_US: u64 = 600_000_000;
const CUES_PER_CHUNK: usize = 512;

#[test]
fn fuzzy_recognized_adjacent_pair_keeps_provenance_and_native_span_in_five_languages() {
    for language in [
        SpeechLanguage::English,
        SpeechLanguage::Japanese,
        SpeechLanguage::French,
        SpeechLanguage::German,
        SpeechLanguage::Korean,
    ] {
        let lines = [
            "synthetic first adjacent phrase",
            "synthetic second adjacent phrase",
        ];
        let document = transcript(language, &lines);
        let input = event(language, &lines.join(" "));
        let proposal = locate(&document, &input).unwrap();
        assert_eq!(proposal.origin(), FuzzyTextOrigin::PreparedRecognition);
        assert_eq!(proposal.cue_range(), 0..2);
        assert_eq!(
            proposal.media_range_us(),
            MEDIA_START_US + 123..MEDIA_START_US + 15_123
        );
        assert_eq!(document.cues().len(), 2);
    }
}

#[test]
fn fuzzy_recognized_far_duplicate_pair_is_not_hidden_by_timing_or_chunk_boundaries() {
    let mut lines = vec!["unrelated synthetic filler"; CUES_PER_CHUNK + 2];
    lines[0] = "first phrase across cues";
    lines[1] = "second phrase across cues";
    lines[CUES_PER_CHUNK] = lines[0];
    lines[CUES_PER_CHUNK + 1] = lines[1];
    let document = transcript(SpeechLanguage::English, &lines);
    let input = event(
        SpeechLanguage::English,
        &format!("{} {}", lines[0], lines[1]),
    );
    assert_eq!(
        locate(&document, &input).err(),
        Some(FuzzyAlignmentError::NoUniqueMatch)
    );
}
fn identity(session: u64) -> StreamIdentity {
    StreamIdentity {
        session: SessionId(session),
        generation: 1,
    }
}
fn scope() -> AlignmentScope {
    AlignmentScope::new(identity(901), identity(902), 1).unwrap()
}
fn transcript(language: SpeechLanguage, lines: &[&str]) -> PreparedTranscript {
    let mut merger = PreparedTranscriptMerger::new(identity(901), SourceLanguage::Auto).unwrap();
    for (chunk_index, chunk) in lines.chunks(CUES_PER_CHUNK).enumerate() {
        let anchor = MEDIA_START_US + chunk_index as u64 * 10_000_000;
        let sequence = chunk_index as u64 + 1;
        let request = PreparedSpeechRequest::new(
            PreparedSpeechChunk::new(
                PreparedChunkStamp {
                    identity: identity(901),
                    sequence,
                    run_id: 1,
                    rate: PreparedSampleRate::Hz16000,
                    audio_start_us: anchor,
                    body_start_us: anchor,
                    body_end_us: anchor + 10_000_000,
                    discontinuity: false,
                },
                vec![0; 160_000],
            )
            .unwrap(),
            SourceLanguage::Auto,
            Vec::new(),
        )
        .unwrap();
        merger.register(&request).unwrap();
        merger
            .complete(
                &request,
                PreparedSpeechResponse {
                    identity: identity(901),
                    sequence,
                    run_id: 1,
                    language: Some(language),
                    cues: chunk
                        .iter()
                        .enumerate()
                        .map(|(index, text)| PreparedSpeechCue {
                            start_us: index as u64 * 10_000 + 123,
                            duration_us: 5_000,
                            text: (*text).into(),
                        })
                        .collect(),
                },
            )
            .unwrap();
    }
    merger.finish().unwrap()
}
fn event(language: SpeechLanguage, text: &str) -> TranscriptEvent {
    TranscriptEvent {
        identity: identity(902),
        utterance: 3,
        sequence: 4,
        revision: 1,
        start_us: Some(42_000_000),
        duration_us: Some(2_000_000),
        language: Some(language),
        kind: TranscriptKind::Final,
        text: text.into(),
    }
}
fn stamp() -> WindowStamp {
    WindowStamp {
        identity: identity(902),
        sequence: 8,
        ended_at: Instant::now(),
        start_us: Some(40_000_000),
        duration_us: 12_000_000,
    }
}
fn policy() -> MatchPolicy {
    MatchPolicy {
        cell_limit: MAX_DP_CELLS,
        deadline: None,
        check_hook: None,
    }
}
fn locate(
    document: &PreparedTranscript,
    input: &TranscriptEvent,
) -> Result<FuzzyCueProposal, FuzzyAlignmentError> {
    locate_text_with_policy(
        scope(),
        AlignmentText::Recognized(document),
        input,
        stamp(),
        &ResolverCancellation::default(),
        policy(),
    )
}

#[test]
fn fuzzy_recognized_view_borrows_original_allocation_and_absolute_time() {
    let document = transcript(SpeechLanguage::English, &[LINE]);
    let view = AlignmentText::Recognized(&document);
    view.validate(identity(901)).unwrap();
    assert_eq!(view.origin(), FuzzyTextOrigin::PreparedRecognition);
    assert_eq!(
        view.cue(0).unwrap().text.as_ptr(),
        document.cues()[0].text.as_ptr()
    );
    assert!(view.cue(1).is_none());
    let input = event(SpeechLanguage::English, LINE);
    let proposal = locate(&document, &input).unwrap();
    assert_eq!(proposal.origin(), FuzzyTextOrigin::PreparedRecognition);
    assert_eq!(
        proposal.media_range_us(),
        MEDIA_START_US + 123..MEDIA_START_US + 5_123
    );
    assert_eq!(proposal.capture_range_us(), 42_000_000..44_000_000);
    assert_eq!(proposal.binding(), TimelineBinding::from_event(&input));
    assert_eq!(proposal.scope(), scope());
}

#[test]
fn fuzzy_recognized_five_languages_keep_unicode_scalar_edits_and_language() {
    for (language, line) in [
        (
            SpeechLanguage::English,
            "a synthetic subtitle demonstrates character edits",
        ),
        (
            SpeechLanguage::Japanese,
            "これは文字の編集距離を検証するための合成字幕です",
        ),
        (
            SpeechLanguage::French,
            "cette réplique synthétique vérifie les caractères",
        ),
        (
            SpeechLanguage::German,
            "diese künstliche untertitelzeile prüft die zeichen",
        ),
        (
            SpeechLanguage::Korean,
            "이것은문자편집거리를검증하기위한합성자막문장입니다",
        ),
    ] {
        let document = transcript(language, &[line]);
        let proposal = locate(&document, &event(language, &format!("{line}!"))).unwrap();
        assert_eq!(proposal.edit_distance(), 1);
        assert_eq!(proposal.origin(), FuzzyTextOrigin::PreparedRecognition);
        assert_eq!(
            proposal.text_agreement_permille(),
            (line.chars().count() * 1000 / (line.chars().count() + 1)) as u16
        );
    }
}

#[test]
fn fuzzy_recognized_document_and_capture_identities_are_independent() {
    let document = transcript(SpeechLanguage::English, &[LINE]);
    for variant in 0..3 {
        let current = match variant {
            0 => AlignmentScope::new(identity(900), identity(902), 1).unwrap(),
            1 => AlignmentScope::new(identity(901), identity(903), 1).unwrap(),
            _ => AlignmentScope::new(
                StreamIdentity {
                    generation: 2,
                    ..identity(901)
                },
                identity(902),
                1,
            )
            .unwrap(),
        };
        assert!(
            locate_text_with_policy(
                current,
                AlignmentText::Recognized(&document),
                &event(SpeechLanguage::English, LINE),
                stamp(),
                &ResolverCancellation::default(),
                policy()
            )
            .is_err()
        );
    }
}

#[test]
fn fuzzy_recognized_rejects_unknown_wrong_language_partial_and_missing_time() {
    let document = transcript(SpeechLanguage::English, &[LINE]);
    for variant in 0..5 {
        let mut input = event(SpeechLanguage::English, LINE);
        match variant {
            0 => input.language = None,
            1 => input.language = Some(SpeechLanguage::French),
            2 => input.kind = TranscriptKind::Partial,
            3 => input.start_us = None,
            _ => input.duration_us = Some(0),
        }
        assert!(locate(&document, &input).is_err());
    }
}

#[test]
fn fuzzy_recognized_far_duplicate_cannot_be_hidden_by_a_local_window() {
    let mut lines = vec!["zzzzzzzzzzzzzzzz"; CUES_PER_CHUNK + 1];
    lines[0] = LINE;
    lines[CUES_PER_CHUNK] = LINE;
    let document = transcript(SpeechLanguage::English, &lines);
    assert_eq!(
        locate(&document, &event(SpeechLanguage::English, LINE)).err(),
        Some(FuzzyAlignmentError::NoUniqueMatch)
    );
}

#[test]
fn fuzzy_recognized_later_oversized_normalization_discards_earlier_match() {
    let oversized = "é".repeat(MAX_NORMALIZED_CHARACTERS + 1);
    let document = transcript(SpeechLanguage::English, &[LINE, &oversized]);
    assert_eq!(
        locate(&document, &event(SpeechLanguage::English, LINE)).err(),
        Some(FuzzyAlignmentError::BudgetExceeded)
    );
}

#[test]
fn fuzzy_recognized_scans_all_8192_sealed_cues_without_copying_a_subtitle_document() {
    let mut lines = vec!["zzzzzzzzzzzzzzzz"; crate::ai::prepared_speech::MAX_ASR_DOCUMENT_CUES];
    let index = lines.len() - 1;
    lines[index] = LINE;
    let document = transcript(SpeechLanguage::English, &lines);
    let proposal = locate(&document, &event(SpeechLanguage::English, LINE)).unwrap();
    assert_eq!(proposal.cue_index(), index);
    assert_eq!(
        proposal.media_range_us().start,
        document.cues()[index].start_us
    );
    assert_eq!(
        proposal.media_range_us().end,
        document.cues()[index].start_us + document.cues()[index].duration_us
    );
}

#[test]
fn fuzzy_recognized_cancel_deadline_and_dp_budget_are_not_bypassed() {
    let document = transcript(SpeechLanguage::English, &[LINE]);
    let input = event(SpeechLanguage::English, LINE);
    for variant in 0..3 {
        let cancel = ResolverCancellation::default();
        let mut limits = policy();
        let expected = match variant {
            0 => {
                cancel.cancel();
                FuzzyAlignmentError::Invalid(ResolverError::Cancelled)
            }
            1 => {
                limits.deadline = Some(Instant::now());
                FuzzyAlignmentError::Invalid(ResolverError::Stale)
            }
            _ => {
                limits.cell_limit = 0;
                FuzzyAlignmentError::BudgetExceeded
            }
        };
        assert_eq!(
            locate_text_with_policy(
                scope(),
                AlignmentText::Recognized(&document),
                &input,
                stamp(),
                &cancel,
                limits
            )
            .err(),
            Some(expected)
        );
    }
}
