//! Synthetic cross-cue candidates; no playback, services or source adoption.
use super::*;

const LEFT: &str = "the synthetic sentence begins here";
const RIGHT: &str = "and continues across subtitle boundaries";

fn pair() -> Fixture {
    let mut fixture = Fixture::new(&[LEFT, RIGHT]);
    fixture.document.cues[0].duration_us = Some(1_000_000);
    fixture.document.cues[1].start_us = Some(601_000_000);
    fixture.document.cues[1].duration_us = Some(1_000_000);
    fixture.event.text = format!("{LEFT} {RIGHT}").into();
    fixture
}

#[test]
fn adjacent_pair_preserves_full_indices_original_intervals_and_binding() {
    let fixture = pair();
    let proposal = fixture.locate().unwrap();
    assert_eq!(proposal.cue_range(), 0..2);
    assert_eq!(proposal.cue_index(), 0);
    assert_eq!(proposal.media_range_us(), 600_000_000..602_000_000);
    assert_eq!(proposal.capture_range_us(), 42_000_000..44_000_000);
    assert_eq!(proposal.edit_distance(), 0);
    assert_eq!(proposal.text_agreement_permille(), 1000);
    assert_eq!(proposal.origin(), FuzzyTextOrigin::ExistingText);
    proposal
        .validate(fixture.scope, TimelineBinding::from_event(&fixture.event))
        .unwrap();
}

#[test]
fn single_candidate_reports_one_cue_and_is_compared_against_pairs() {
    let mut fixture = pair();
    fixture.event.text = LEFT.into();
    assert_eq!(fixture.locate().unwrap().cue_range(), 0..1);
    // A separate single cue ties the complete two-cue wording.
    fixture.document.cues.push(TextCue {
        start_us: Some(700_000_000),
        duration_us: Some(2_000_000),
        text: format!("{LEFT} {RIGHT}").into(),
    });
    fixture.event.text = format!("{LEFT} {RIGHT}").into();
    error(&fixture, FuzzyAlignmentError::NoUniqueMatch);
}

#[test]
fn distant_pair_blocks_uniqueness_even_when_its_times_cannot_be_adopted() {
    for untimed in [false, true] {
        let mut fixture = pair();
        fixture.document.source = SubtitleSource::ExistingLyrics;
        for (index, line) in [LEFT, RIGHT].into_iter().enumerate() {
            fixture.document.cues.push(TextCue {
                start_us: (!untimed).then_some(900_000_000 + index as u64 * 10_000_000),
                duration_us: (!untimed).then_some(1_000_000),
                text: line.into(),
            });
        }
        error(&fixture, FuzzyAlignmentError::NoUniqueMatch);
    }
}

#[test]
fn overlapping_lexical_pairs_are_distinct_competitors() {
    let line = "synthetic identical adjacent wording";
    let mut fixture = Fixture::new(&[line, line, line]);
    fixture.event.text = format!("{line} {line}").into();
    error(&fixture, FuzzyAlignmentError::NoUniqueMatch);
}

#[test]
fn pair_gap_is_inclusive_and_one_microsecond_more_is_rejected() {
    for extra in [0, 1] {
        let mut fixture = pair();
        fixture.document.cues[1].start_us = Some(601_000_000 + MAX_MATCH_GAP_US + extra);
        if extra == 0 {
            assert_eq!(
                fixture.locate().unwrap().media_range_us(),
                600_000_000..602_500_000
            );
        } else {
            error(&fixture, FuzzyAlignmentError::TimingMismatch);
        }
    }
}

#[test]
fn pair_requires_complete_nonoverlapping_positive_times() {
    for kind in 0..3 {
        let mut fixture = pair();
        match kind {
            0 => fixture.document.cues[1].start_us = Some(600_999_999),
            1 => {
                fixture.document.source = SubtitleSource::ExistingLyrics;
                fixture.document.cues[1].start_us = None;
                fixture.document.cues[1].duration_us = None;
            }
            _ => {
                fixture.document.source = SubtitleSource::ExistingLyrics;
                fixture.document.cues[0].start_us = None;
                fixture.document.cues[0].duration_us = None;
            }
        }
        error(&fixture, FuzzyAlignmentError::TimingMismatch);
    }
}

#[test]
fn nonadjacent_and_three_cue_phrases_are_not_silently_joined() {
    let mut fixture = Fixture::new(&[LEFT, "unrelated middle wording", RIGHT]);
    fixture.event.text = format!("{LEFT} {RIGHT}").into();
    error(&fixture, FuzzyAlignmentError::NoUniqueMatch);
    fixture.event.text = format!("{LEFT} unrelated middle wording {RIGHT}").into();
    error(&fixture, FuzzyAlignmentError::NoUniqueMatch);
}

#[test]
fn pair_normalization_preserves_spaces_punctuation_accents_and_scalar_boundaries() {
    for (language, left, right) in [
        (SpeechLanguage::English, LEFT, RIGHT),
        (
            SpeechLanguage::Japanese,
            "字幕境界の前半を確認します",
            "次の文章との一致を検証します",
        ),
        (
            SpeechLanguage::French,
            "la première phrase synthétique",
            "préserve les caractères accentués",
        ),
        (
            SpeechLanguage::German,
            "synthetische Wörter bleiben",
            "über beide Untertitel erhalten",
        ),
        (
            SpeechLanguage::Korean,
            "첫 번째 합성 문장을 검증합니다",
            "다음 자막 문장과 함께 확인합니다",
        ),
    ] {
        let mut fixture = pair();
        fixture.document.language = TextLanguage::Source(language);
        fixture.document.cues[0].text = format!("  {left}\t").into();
        fixture.document.cues[1].text = format!("\n{right}  ").into();
        fixture.event.language = Some(language);
        fixture.event.text = format!("{left} {right}").into();
        assert_eq!(fixture.locate().unwrap().edit_distance(), 0);
        fixture.event.text = format!("{left}{right}").into();
        assert_eq!(fixture.locate().unwrap().edit_distance(), 1);
    }
}

#[test]
fn maximum_join_is_bounded_and_distant_long_pairs_do_not_hide_single_winner() {
    let a = "a".repeat(MAX_NORMALIZED_CHARACTERS);
    let b = "b".repeat(MAX_NORMALIZED_CHARACTERS);
    let fixture = Fixture::new(&[&a, &b, LINE]);
    assert_eq!(fixture.locate().unwrap().cue_range(), 2..3);
    let cancel = ResolverCancellation::default();
    let check = work(&cancel, MAX_DP_CELLS);
    let joined = join_cues(
        &normalize(&a, &check).unwrap(),
        &normalize(&b, &check).unwrap(),
    );
    assert_eq!(joined.characters.len(), MAX_MATCH_CHARACTERS);
    assert_eq!(joined.characters[MAX_NORMALIZED_CHARACTERS], ' ');
}

#[test]
fn later_oversized_cue_still_discards_earlier_pair_winner() {
    let mut fixture = pair();
    fixture.document.cues.push(TextCue {
        start_us: Some(700_000_000),
        duration_us: Some(1_000_000),
        text: "x".repeat(MAX_NORMALIZED_CHARACTERS + 1).into(),
    });
    error(&fixture, FuzzyAlignmentError::BudgetExceeded);
}

#[test]
fn pair_dp_cost_is_shared_and_exact_total_budget_is_inclusive() {
    let fixture = pair();
    let mut check = work(&fixture.cancel, MAX_DP_CELLS);
    let expected = normalize(&fixture.event.text, &check).unwrap();
    let left = normalize(LEFT, &check).unwrap();
    let right = normalize(RIGHT, &check).unwrap();
    for candidate in [&left, &right, &join_cues(&left, &right)] {
        edit_distance(&expected.characters, &candidate.characters, &mut check).unwrap();
    }
    let cost = check.cells;
    assert!(cost > 0);
    assert!(fixture.policy(policy(cost)).is_ok());
    assert_eq!(
        fixture.policy(policy(cost - 1)).err(),
        Some(FuzzyAlignmentError::BudgetExceeded)
    );
}

#[test]
fn expanded_dp_rows_agree_with_unrestricted_oracle_and_enforce_850_floor() {
    let cancel = ResolverCancellation::default();
    let left = vec!['é'; MAX_NORMALIZED_CHARACTERS];
    for extra in [
        1,
        28,
        45,
        46,
        MAX_MATCH_CHARACTERS - MAX_NORMALIZED_CHARACTERS,
    ] {
        let mut right = left.clone();
        right.extend(std::iter::repeat_n('!', extra));
        let distance = oracle(&left, &right);
        let expected = (distance <= right.len() * 150 / 1000).then_some(distance);
        assert_eq!(
            edit_distance(&left, &right, &mut work(&cancel, MAX_DP_CELLS)).unwrap(),
            expected
        );
    }
}

#[test]
fn pair_candidates_still_recheck_full_binding_epoch_cancellation_and_freshness() {
    let fixture = pair();
    let proposal = fixture.locate().unwrap();
    let mut binding = proposal.binding();
    binding.revision += 1;
    assert_eq!(
        proposal.validate(fixture.scope, binding),
        Err(FuzzyAlignmentError::Obsolete)
    );
    let other = AlignmentScope::new(
        fixture.scope.document_identity(),
        fixture.scope.capture_identity(),
        2,
    )
    .unwrap();
    assert_eq!(
        proposal.validate(other, proposal.binding()),
        Err(FuzzyAlignmentError::Obsolete)
    );
    fixture.cancel.cancel();
    assert_eq!(
        proposal.validate(fixture.scope, proposal.binding()),
        Err(ResolverError::Cancelled.into())
    );
    let mut stale = pair();
    stale.stamp.ended_at = Instant::now() - MAX_WINDOW_AGE - Duration::from_millis(1);
    error(&stale, ResolverError::Stale.into());
}
