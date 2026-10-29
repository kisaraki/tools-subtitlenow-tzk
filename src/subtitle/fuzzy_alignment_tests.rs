//! Synthetic text only; these proposals do not establish playback or adoption.
use super::*;
use crate::{
    ai::speech::SpeechLanguage,
    audio::capture::SessionId,
    resolver::contracts::{
        MAX_CUE_BYTES, MAX_TEXT_BYTES, MAX_TEXT_CUES, SubtitleSource, TextCue, TextLanguage,
    },
};
use std::{cell::Cell, rc::Rc};

#[path = "fuzzy_span_tests.rs"]
mod span_tests;

const LINE: &str = "the synthetic subtitle belongs to this document";
fn identity(session: u64) -> StreamIdentity {
    StreamIdentity {
        session: SessionId(session),
        generation: 1,
    }
}
struct Fixture {
    scope: AlignmentScope,
    document: ExistingText,
    event: TranscriptEvent,
    stamp: WindowStamp,
    cancel: ResolverCancellation,
}
impl Fixture {
    fn new(lines: &[&str]) -> Self {
        let scope = AlignmentScope::new(identity(901), identity(902), 1).unwrap();
        Self {
            scope,
            document: ExistingText {
                identity: scope.document_identity(),
                source: SubtitleSource::ExistingSubtitle,
                content_id: "synthetic-document".into(),
                language: TextLanguage::Source(SpeechLanguage::English),
                cues: lines
                    .iter()
                    .enumerate()
                    .map(|(index, text)| TextCue {
                        start_us: Some(600_000_000 + index as u64 * 10_000_000),
                        duration_us: Some(2_000_000),
                        text: (*text).into(),
                    })
                    .collect(),
            },
            event: TranscriptEvent {
                identity: scope.capture_identity(),
                utterance: 3,
                sequence: 4,
                revision: 1,
                start_us: Some(42_000_000),
                duration_us: Some(2_000_000),
                language: Some(SpeechLanguage::English),
                kind: TranscriptKind::Final,
                text: LINE.into(),
            },
            stamp: WindowStamp {
                identity: scope.capture_identity(),
                sequence: 8,
                ended_at: Instant::now(),
                start_us: Some(40_000_000),
                duration_us: 12_000_000,
            },
            cancel: ResolverCancellation::default(),
        }
    }
    fn locate(&self) -> Result<FuzzyCueProposal, FuzzyAlignmentError> {
        locate_fuzzy_cue(
            self.scope,
            &self.document,
            &self.event,
            self.stamp,
            &self.cancel,
        )
    }
    fn policy(&self, policy: MatchPolicy) -> Result<FuzzyCueProposal, FuzzyAlignmentError> {
        locate_with_policy(
            self.scope,
            &self.document,
            &self.event,
            self.stamp,
            &self.cancel,
            policy,
        )
    }
}
fn policy(cell_limit: usize) -> MatchPolicy {
    MatchPolicy {
        cell_limit,
        deadline: None,
        check_hook: None,
    }
}
fn work(cancel: &ResolverCancellation, cell_limit: usize) -> WorkCheck<'_> {
    WorkCheck::new(
        cancel,
        Instant::now() + FUZZY_MATCH_TIMEOUT,
        policy(cell_limit),
    )
}
fn error(fixture: &Fixture, expected: FuzzyAlignmentError) {
    assert_eq!(fixture.locate().err(), Some(expected));
}

#[test]
fn separate_document_and_capture_sessions_retain_original_ranges() {
    let fixture = Fixture::new(&["unrelated alternative text has no shared content", LINE]);
    let proposal = fixture.locate().unwrap();
    assert_eq!(proposal.scope(), fixture.scope);
    assert_ne!(
        proposal.scope().document_identity(),
        proposal.scope().capture_identity()
    );
    assert_eq!(proposal.scope().epoch(), 1);
    assert_eq!(
        proposal.binding(),
        TimelineBinding::from_event(&fixture.event)
    );
    assert_eq!(proposal.cue_index(), 1);
    assert_eq!(proposal.capture_range_us(), 42_000_000..44_000_000);
    assert_eq!(proposal.media_range_us(), 610_000_000..612_000_000);
    assert_eq!(proposal.edit_distance(), 0);
    assert_eq!(proposal.text_agreement_permille(), 1000);
    assert_eq!(proposal.runner_up_agreement_permille(), None);
    proposal
        .validate(fixture.scope, proposal.binding())
        .unwrap();
}

#[test]
fn five_languages_use_scalar_edits_and_explicit_source_language() {
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
        let mut fixture = Fixture::new(&[line]);
        fixture.document.language = TextLanguage::Source(language);
        fixture.event.language = Some(language);
        fixture.event.text = format!("{line}!").into();
        let proposal = fixture.locate().unwrap();
        assert_eq!(proposal.edit_distance(), 1);
        assert_eq!(
            proposal.text_agreement_permille(),
            (line.chars().count() * 1000 / (line.chars().count() + 1)) as u16
        );
    }
}

#[test]
fn ascii_case_and_whitespace_normalize_without_erasing_boundaries() {
    let mut fixture = Fixture::new(&[LINE]);
    fixture.event.text = " \tTHE synthetic\nsubtitle belongs to this document\r ".into();
    assert_eq!(fixture.locate().unwrap().edit_distance(), 0);
    let check = work(&fixture.cancel, MAX_DP_CELLS);
    for (left, right) in [
        ("école", "ecole"),
        ("ab cd", "abcd"),
        ("well-known", "wellknown"),
        ("É", "é"),
        ("é", "e\u{301}"),
    ] {
        assert_ne!(
            normalize(left, &check).unwrap().characters,
            normalize(right, &check).unwrap().characters
        );
    }
}

#[test]
fn weak_and_short_text_are_not_unique_evidence() {
    let mut fixture = Fixture::new(&["this other line has entirely different content"]);
    error(&fixture, FuzzyAlignmentError::NoUniqueMatch);
    fixture.event.text = "short phrase".into();
    error(&fixture, FuzzyAlignmentError::UnsupportedEvidence);
    fixture.event.text = "......................".into();
    error(&fixture, FuzzyAlignmentError::UnsupportedEvidence);
    fixture.event.text = "aaaaaaaaaaaaaaaa".into();
    fixture.document.cues[0].text = "aaaaaaaaaaaaaaa".into();
    error(&fixture, FuzzyAlignmentError::UnsupportedEvidence);
}

#[test]
fn partial_unknown_wrong_language_and_chinese_are_rejected() {
    let mut fixture = Fixture::new(&[LINE]);
    fixture.event.kind = TranscriptKind::Partial;
    error(&fixture, FuzzyAlignmentError::UnsupportedEvidence);
    fixture.event.kind = TranscriptKind::Final;
    fixture.event.language = None;
    error(&fixture, FuzzyAlignmentError::UnsupportedEvidence);
    fixture.event.language = Some(SpeechLanguage::French);
    error(&fixture, FuzzyAlignmentError::UnsupportedEvidence);
    fixture.event.language = Some(SpeechLanguage::English);
    for language in [
        TextLanguage::TaiwanTraditional,
        TextLanguage::OtherTraditional,
        TextLanguage::SimplifiedChinese,
    ] {
        fixture.document.language = language;
        error(&fixture, FuzzyAlignmentError::UnsupportedEvidence);
    }
}

#[test]
fn invalid_scope_and_zero_binding_fields_are_rejected() {
    for (document, capture, epoch) in [
        (identity(0), identity(2), 1),
        (identity(1), identity(0), 1),
        (identity(1), identity(2), 0),
        (
            StreamIdentity {
                generation: 0,
                ..identity(1)
            },
            identity(2),
            1,
        ),
        (
            identity(1),
            StreamIdentity {
                generation: 0,
                ..identity(2)
            },
            1,
        ),
    ] {
        assert_eq!(
            AlignmentScope::new(document, capture, epoch).err(),
            Some(FuzzyAlignmentError::Invalid(ResolverError::InvalidData))
        );
    }
    for field in 0..4 {
        let mut fixture = Fixture::new(&[LINE]);
        match field {
            0 => fixture.event.utterance = 0,
            1 => fixture.event.sequence = 0,
            2 => fixture.event.revision = 0,
            _ => fixture.stamp.sequence = 0,
        }
        error(&fixture, FuzzyAlignmentError::UnsupportedEvidence);
    }
}

#[test]
fn document_event_and_window_identities_are_independently_checked() {
    for field in 0..3 {
        let mut fixture = Fixture::new(&[LINE]);
        match field {
            0 => fixture.document.identity.generation += 1,
            1 => fixture.event.identity.generation += 1,
            _ => fixture.stamp.identity.generation += 1,
        }
        assert!(fixture.locate().is_err());
    }
}

#[test]
fn incomplete_overflowing_or_outside_capture_intervals_fail() {
    for (start, duration) in [
        (None, Some(1)),
        (Some(42_000_000), None),
        (Some(42_000_000), Some(0)),
        (Some(u64::MAX), Some(1)),
        (Some(39_999_999), Some(2_000_000)),
        (Some(51_000_000), Some(2_000_000)),
    ] {
        let mut fixture = Fixture::new(&[LINE]);
        fixture.event.start_us = start;
        fixture.event.duration_us = duration;
        error(&fixture, FuzzyAlignmentError::TimingMismatch);
    }
    let mut fixture = Fixture::new(&[LINE]);
    fixture.stamp.start_us = None;
    error(&fixture, FuzzyAlignmentError::TimingMismatch);
    fixture.stamp.start_us = Some(u64::MAX);
    error(
        &fixture,
        FuzzyAlignmentError::Invalid(ResolverError::InvalidData),
    );
}

#[test]
fn window_edges_are_inclusive_and_duration_is_not_used_to_choose_text() {
    let mut fixture = Fixture::new(&[LINE]);
    fixture.event.start_us = fixture.stamp.start_us;
    fixture.event.duration_us = Some(fixture.stamp.duration_us);
    assert_eq!(
        fixture.locate().unwrap().capture_range_us(),
        40_000_000..52_000_000
    );
}

#[test]
fn distant_and_untimed_repetitions_defeat_whole_document_uniqueness() {
    let mut fixture = Fixture::new(&[LINE, LINE]);
    fixture.document.cues[1].start_us = Some(9_000_000_000);
    error(&fixture, FuzzyAlignmentError::NoUniqueMatch);
    fixture.document.source = SubtitleSource::ExistingLyrics;
    fixture.document.cues[1].start_us = None;
    fixture.document.cues[1].duration_us = None;
    error(&fixture, FuzzyAlignmentError::NoUniqueMatch);
    fixture.document.cues.swap(0, 1);
    error(&fixture, FuzzyAlignmentError::NoUniqueMatch);
}

#[test]
fn below_threshold_runner_still_defeats_margin() {
    let query = "a".repeat(100);
    let best = format!("{}{}", "a".repeat(90), "b".repeat(10));
    let runner = format!("{}{}", "a".repeat(89), "b".repeat(11));
    for lines in [
        [best.as_str(), runner.as_str()],
        [runner.as_str(), best.as_str()],
    ] {
        let mut fixture = Fixture::new(&lines);
        fixture.event.text = query.clone().into();
        error(&fixture, FuzzyAlignmentError::NoUniqueMatch);
    }
}

#[test]
fn exact_threshold_and_margin_are_accepted_with_measured_runner() {
    let best = format!("{}{}", "a".repeat(90), "b".repeat(10));
    let runner = format!("{}{}", "a".repeat(85), "b".repeat(15));
    let mut fixture = Fixture::new(&[&runner, &best]);
    fixture.event.text = "a".repeat(100).into();
    let proposal = fixture.locate().unwrap();
    assert_eq!(proposal.cue_index(), 1);
    assert_eq!(proposal.edit_distance(), 10);
    assert_eq!(proposal.text_agreement_permille(), 900);
    assert_eq!(proposal.runner_up_agreement_permille(), Some(850));
}

#[test]
fn untimed_winner_and_malformed_document_timing_fail() {
    let mut fixture = Fixture::new(&[LINE]);
    fixture.document.source = SubtitleSource::ExistingLyrics;
    fixture.document.cues[0].start_us = None;
    fixture.document.cues[0].duration_us = None;
    error(&fixture, FuzzyAlignmentError::TimingMismatch);
    fixture.document.source = SubtitleSource::ExistingSubtitle;
    error(
        &fixture,
        FuzzyAlignmentError::Invalid(ResolverError::InvalidData),
    );
    for (start, duration) in [
        (Some(1), Some(0)),
        (Some(u64::MAX), Some(1)),
        (Some(1), None),
    ] {
        fixture.document.cues[0].start_us = start;
        fixture.document.cues[0].duration_us = duration;
        error(
            &fixture,
            FuzzyAlignmentError::Invalid(ResolverError::InvalidData),
        );
    }
}

#[test]
fn normalized_limit_is_inclusive_and_later_overflow_discards_best() {
    let line = "a".repeat(MAX_NORMALIZED_CHARACTERS);
    let mut fixture = Fixture::new(&[&line]);
    fixture.event.text = line.clone().into();
    assert_eq!(fixture.locate().unwrap().edit_distance(), 0);
    fixture.event.text = format!("{line}a").into();
    error(&fixture, FuzzyAlignmentError::BudgetExceeded);
    let too_long = format!("{line}a");
    fixture = Fixture::new(&[LINE, &too_long]);
    error(&fixture, FuzzyAlignmentError::BudgetExceeded);
}

#[test]
fn document_byte_cue_and_count_limits_are_not_bypassed() {
    let invalid = FuzzyAlignmentError::Invalid(ResolverError::InvalidData);
    let mut fixture = Fixture::new(&[LINE]);
    fixture.document.cues[0].text = "a".repeat(MAX_CUE_BYTES + 1).into();
    error(&fixture, invalid);
    fixture.document.cues = (0..=MAX_TEXT_CUES)
        .map(|_| TextCue {
            start_us: Some(1),
            duration_us: Some(1),
            text: LINE.into(),
        })
        .collect();
    error(&fixture, invalid);
    fixture.document.cues = (0..=MAX_TEXT_BYTES / MAX_CUE_BYTES)
        .map(|_| TextCue {
            start_us: Some(1),
            duration_us: Some(1),
            text: "a".repeat(MAX_CUE_BYTES).into(),
        })
        .collect();
    error(&fixture, invalid);
}

#[test]
fn invalid_event_bytes_controls_and_blank_document_fail() {
    for text in [
        "a".repeat(MAX_TRANSCRIPT_BYTES + 1),
        format!("{LINE}\0"),
        String::new(),
    ] {
        let mut fixture = Fixture::new(&[LINE]);
        fixture.event.text = text.into();
        error(&fixture, FuzzyAlignmentError::UnsupportedEvidence);
    }
    let mut fixture = Fixture::new(&["   "]);
    error(
        &fixture,
        FuzzyAlignmentError::Invalid(ResolverError::InvalidData),
    );
    fixture.document.cues.clear();
    error(
        &fixture,
        FuzzyAlignmentError::Invalid(ResolverError::InvalidData),
    );
}

#[test]
fn exact_dp_budget_is_allowed_and_one_cell_less_fails() {
    let fixture = Fixture::new(&[LINE]);
    let left: Vec<_> = LINE.chars().collect();
    let mut check = work(&fixture.cancel, MAX_DP_CELLS);
    assert_eq!(edit_distance(&left, &left, &mut check).unwrap(), Some(0));
    let cells = check.cells;
    assert!(cells > 0);
    assert!(fixture.policy(policy(cells)).is_ok());
    assert_eq!(
        fixture.policy(policy(cells - 1)).err(),
        Some(FuzzyAlignmentError::BudgetExceeded)
    );
    let fixture = Fixture::new(&[LINE, LINE]);
    assert_eq!(
        fixture.policy(policy(cells)).err(),
        Some(FuzzyAlignmentError::BudgetExceeded)
    );
}

#[test]
fn public_whole_document_dp_budget_errors_instead_of_partial_best() {
    let line = "a".repeat(MAX_NORMALIZED_CHARACTERS);
    let mut fixture = Fixture::new(&vec![line.as_str(); 200]);
    fixture.event.text = line.into();
    error(&fixture, FuzzyAlignmentError::BudgetExceeded);
}

#[test]
fn cancellation_before_during_normalization_and_during_dp_is_observed() {
    let fixture = Fixture::new(&[LINE]);
    fixture.cancel.cancel();
    error(
        &fixture,
        FuzzyAlignmentError::Invalid(ResolverError::Cancelled),
    );
    for checkpoint in [5, 12] {
        let fixture = Fixture::new(&[LINE]);
        let cancel = fixture.cancel.clone();
        let calls = Rc::new(Cell::new(0));
        let counter = calls.clone();
        let mut limits = policy(MAX_DP_CELLS);
        limits.check_hook = Some(Box::new(move || {
            let count = counter.get() + 1;
            counter.set(count);
            if count == checkpoint {
                cancel.cancel();
            }
        }));
        assert_eq!(
            fixture.policy(limits).err(),
            Some(FuzzyAlignmentError::Invalid(ResolverError::Cancelled))
        );
        assert_eq!(calls.get(), checkpoint);
    }
}

#[test]
fn deadline_and_stale_or_future_capture_window_fail() {
    let fixture = Fixture::new(&[LINE]);
    let mut limits = policy(MAX_DP_CELLS);
    limits.deadline = Some(Instant::now() - Duration::from_millis(1));
    assert_eq!(
        fixture.policy(limits).err(),
        Some(FuzzyAlignmentError::Invalid(ResolverError::Stale))
    );
    for ended_at in [
        Instant::now() - MAX_WINDOW_AGE - Duration::from_secs(1),
        Instant::now() + Duration::from_secs(1),
    ] {
        let mut fixture = Fixture::new(&[LINE]);
        fixture.stamp.ended_at = ended_at;
        error(&fixture, FuzzyAlignmentError::Invalid(ResolverError::Stale));
    }
}

#[test]
fn proposal_rejects_changed_epoch_document_capture_and_full_asr_binding() {
    let fixture = Fixture::new(&[LINE]);
    let proposal = fixture.locate().unwrap();
    for scope in [
        AlignmentScope::new(identity(901), identity(902), 2).unwrap(),
        AlignmentScope::new(identity(903), identity(902), 1).unwrap(),
        AlignmentScope::new(identity(901), identity(903), 1).unwrap(),
        AlignmentScope::new(
            identity(901),
            StreamIdentity {
                generation: 2,
                ..identity(902)
            },
            1,
        )
        .unwrap(),
    ] {
        assert_eq!(
            proposal.validate(scope, proposal.binding()),
            Err(FuzzyAlignmentError::Obsolete)
        );
    }
    for field in 0..4 {
        let mut binding = proposal.binding();
        match field {
            0 => binding.utterance += 1,
            1 => binding.sequence += 1,
            2 => binding.revision += 1,
            _ => binding.identity = identity(903),
        }
        assert_eq!(
            proposal.validate(fixture.scope, binding),
            Err(FuzzyAlignmentError::Obsolete)
        );
    }
}

#[test]
fn proposal_rechecks_cancellation_and_publication_age() {
    let fixture = Fixture::new(&[LINE]);
    let mut proposal = fixture.locate().unwrap();
    proposal.stamp.ended_at = Instant::now() - MAX_WINDOW_AGE - Duration::from_secs(1);
    assert_eq!(
        proposal.validate(fixture.scope, proposal.binding()),
        Err(FuzzyAlignmentError::Invalid(ResolverError::Stale))
    );
    fixture.cancel.cancel();
    assert_eq!(
        proposal.validate(fixture.scope, proposal.binding()),
        Err(FuzzyAlignmentError::Invalid(ResolverError::Cancelled))
    );
}

fn oracle(left: &[char], right: &[char]) -> usize {
    let mut previous: Vec<_> = (0..=right.len()).collect();
    for (row, character) in left.iter().enumerate() {
        let mut current = vec![row + 1; right.len() + 1];
        for (column, other) in right.iter().enumerate() {
            current[column + 1] = (previous[column + 1] + 1)
                .min(current[column] + 1)
                .min(previous[column] + usize::from(character != other));
        }
        previous = current;
    }
    previous[right.len()]
}

#[test]
fn banded_distance_matches_unrestricted_oracle_at_cutoff_and_unicode_edits() {
    let cancel = ResolverCancellation::default();
    // Cover unequal lengths, substitutions/insertions/deletions, scalar widths,
    // zero-width band, rounding at 850, and maximum-size rows.
    for length in [0, 1, 6, 7, 16, 19, 20, 99, 100, 255, 256] {
        let left: Vec<_> = (0..length).map(|i| ['a', 'é', '한', '本'][i % 4]).collect();
        for edits in [0, 1, length * 15 / 100, length * 15 / 100 + 1] {
            for kind in 0..3 {
                let mut right = left.clone();
                match kind {
                    0 => {
                        for character in right.iter_mut().take(edits) {
                            *character = '!';
                        }
                    }
                    1 => {
                        right.truncate(right.len().saturating_sub(edits));
                    }
                    _ => {
                        right.extend(std::iter::repeat_n('!', edits.min(256 - length)));
                    }
                }
                let distance = oracle(&left, &right);
                let maximum = left.len().max(right.len());
                let expected = (distance <= maximum * 150 / 1000).then_some(distance);
                assert_eq!(
                    edit_distance(&left, &right, &mut work(&cancel, MAX_DP_CELLS)).unwrap(),
                    expected
                );
            }
        }
    }
}
