//! Synthetic atomic Chinese/reference publications; no providers or media I/O.
use super::*;
use crate::{
    ai::speech::SpeechLanguage,
    audio::capture::SessionId,
    resolver::contracts::{SourceTextReference, SubtitleSource, TextCue, TextQuery},
};

pub(crate) struct Fixture {
    pub scope: AlignmentScope,
    pub bundle: ExistingTextBundle,
    pub event: TranscriptEvent,
    pub stamp: WindowStamp,
    pub cancel: ResolverCancellation,
}
impl Fixture {
    pub(crate) fn new() -> Self {
        let identity = |id| StreamIdentity {
            session: SessionId(id),
            generation: 1,
        };
        let scope = AlignmentScope::new(identity(901), identity(902), 1).unwrap();
        let document = |cues, language| ExistingText {
            identity: scope.document_identity(),
            source: SubtitleSource::ExistingSubtitle,
            content_id: "synthetic-paired-document".into(),
            language,
            cues,
        };
        let mut primary = vec![TextCue {
            start_us: Some(599_000_000),
            duration_us: Some(500_000),
            text: "合成片頭字幕。".into(),
        }];
        primary.extend((0..3).map(|index| TextCue {
            start_us: Some(600_000_000 + index * 1_000_000),
            duration_us: Some(1_000_000),
            text: format!("這是第 {} 段合成中文字幕。", index + 1).into(),
        }));
        let reference = document(
            (0..3)
                .map(|index| TextCue {
                    start_us: Some(600_000_000 + index * 1_000_000),
                    duration_us: Some(1_000_000),
                    text: char::from(b'a' + index as u8).to_string().repeat(32).into(),
                })
                .collect(),
            TextLanguage::Source(SpeechLanguage::English),
        );
        Self {
            scope,
            bundle: ExistingTextBundle {
                text: document(primary, TextLanguage::TaiwanTraditional),
                source_reference: Some(SourceTextReference {
                    text: reference,
                    primary_edition_id: "synthetic-edition".into(),
                    reference_edition_id: "synthetic-edition".into(),
                }),
            },
            event: TranscriptEvent {
                identity: scope.capture_identity(),
                utterance: 1,
                sequence: 1,
                revision: 1,
                start_us: Some(42_000_000),
                duration_us: Some(1_000_000),
                language: Some(SpeechLanguage::English),
                kind: TranscriptKind::Final,
                text: "b".repeat(32).into(),
            },
            stamp: WindowStamp {
                identity: scope.capture_identity(),
                sequence: 20,
                ended_at: Instant::now(),
                start_us: Some(40_000_000),
                duration_us: 12_000_000,
            },
            cancel: ResolverCancellation::default(),
        }
    }
    pub(crate) fn source(&self) -> &ExistingText {
        &self.bundle.source_reference.as_ref().unwrap().text
    }
    fn source_mut(&mut self) -> &mut ExistingText {
        &mut self.bundle.source_reference.as_mut().unwrap().text
    }
    pub(crate) fn pair(&mut self) {
        self.event.text = format!(
            "{} {}",
            self.source().cues[0].text,
            self.source().cues[1].text
        )
        .into();
        self.event.duration_us = Some(2_000_000);
    }
    fn locate(&self) -> Result<FuzzyCueProposal, FuzzyAlignmentError> {
        locate_paired_fuzzy_cue(
            self.scope,
            &self.bundle,
            &self.event,
            self.stamp,
            &self.cancel,
        )
    }
}

#[test]
fn fuzzy_paired_five_languages_and_three_chinese_kinds_preserve_distinct_indices() {
    for chinese in [
        TextLanguage::TaiwanTraditional,
        TextLanguage::OtherTraditional,
        TextLanguage::SimplifiedChinese,
    ] {
        for language in [
            SpeechLanguage::English,
            SpeechLanguage::Japanese,
            SpeechLanguage::French,
            SpeechLanguage::German,
            SpeechLanguage::Korean,
        ] {
            let mut data = Fixture::new();
            data.bundle.text.language = chinese;
            data.source_mut().language = TextLanguage::Source(language);
            data.event.language = Some(language);
            let proposal = data.locate().unwrap();
            assert_eq!(proposal.origin(), FuzzyTextOrigin::PairedSourceReference);
            assert_eq!(proposal.cue_range(), 1..2);
            assert_eq!(proposal.paired_primary_cue_range(), Some(2..3));
            assert_eq!(proposal.media_range_us(), 601_000_000..602_000_000);
            assert_eq!(proposal.capture_range_us(), 42_000_000..43_000_000);
            assert_ne!(
                proposal.scope().document_identity(),
                proposal.scope().capture_identity()
            );
        }
    }
}

#[test]
fn fuzzy_paired_adjacent_reference_span_maps_consecutive_primary_indices() {
    let mut data = Fixture::new();
    data.pair();
    let proposal = data.locate().unwrap();
    assert_eq!(proposal.cue_range(), 0..2);
    assert_eq!(proposal.paired_primary_cue_range(), Some(1..3));
    assert_eq!(proposal.media_range_us(), 600_000_000..602_000_000);
}

#[test]
fn fuzzy_paired_chinese_body_cannot_replace_source_evidence() {
    let mut data = Fixture::new();
    data.event.text = "這段夠長的合成中文譯文不能當作五語原文定位證據".into();
    data.bundle.text.cues[2].text = data.event.text.clone();
    assert_eq!(
        data.locate().err(),
        Some(FuzzyAlignmentError::NoUniqueMatch)
    );
    assert_eq!(
        locate_fuzzy_cue(
            data.scope,
            &data.bundle.text,
            &data.event,
            data.stamp,
            &data.cancel
        )
        .err(),
        Some(FuzzyAlignmentError::UnsupportedEvidence)
    );
}

#[test]
fn fuzzy_paired_requires_chinese_primary_and_explicit_source_reference() {
    let mut data = Fixture::new();
    data.bundle.source_reference = None;
    assert_eq!(
        data.locate().err(),
        Some(FuzzyAlignmentError::UnsupportedEvidence)
    );
    let mut data = Fixture::new();
    data.bundle.text.language = TextLanguage::Source(SpeechLanguage::English);
    assert_eq!(
        data.locate().err(),
        Some(FuzzyAlignmentError::UnsupportedEvidence)
    );
    let mut data = Fixture::new();
    data.source_mut().language = TextLanguage::TaiwanTraditional;
    assert_eq!(data.locate().err(), Some(ResolverError::InvalidData.into()));
}

#[test]
fn fuzzy_paired_provenance_mismatches_and_invalid_edition_metadata_are_rejected() {
    for kind in 0..8 {
        let mut data = Fixture::new();
        let reference = data.bundle.source_reference.as_mut().unwrap();
        match kind {
            0 => reference.text.identity.generation += 1,
            1 => reference.text.source = SubtitleSource::ExistingLyrics,
            2 => reference.text.content_id = "another-content".into(),
            3 => reference.reference_edition_id = "another-edition".into(),
            4 => {
                reference.primary_edition_id = "".into();
                reference.reference_edition_id = "".into();
            }
            5 => {
                reference.primary_edition_id = "bad\nmetadata".into();
                reference.reference_edition_id = "bad\nmetadata".into();
            }
            6 => reference.text.identity.session.0 += 1,
            _ => data.bundle.text.identity.generation += 1,
        }
        assert_eq!(data.locate().err(), Some(ResolverError::InvalidData.into()));
    }
}

#[test]
fn fuzzy_paired_one_microsecond_timing_difference_is_not_rounded_or_forgiven() {
    for duration in [false, true] {
        let mut data = Fixture::new();
        let cue = &mut data.bundle.text.cues[2];
        if duration {
            cue.duration_us = Some(1_000_001);
        } else {
            cue.start_us = Some(601_000_001);
        }
        assert_eq!(
            data.locate().err(),
            Some(FuzzyAlignmentError::TimingMismatch)
        );
    }
}

#[test]
fn fuzzy_paired_full_primary_scan_rejects_duplicate_timing() {
    let mut data = Fixture::new();
    data.bundle.text.cues.insert(
        3,
        TextCue {
            start_us: Some(601_000_000),
            duration_us: Some(1_000_000),
            text: "另一段相同時間字幕。".into(),
        },
    );
    assert_eq!(
        data.locate().err(),
        Some(FuzzyAlignmentError::NoUniqueMatch)
    );
}

#[test]
fn fuzzy_paired_discontinuous_primary_indices_cannot_be_joined() {
    let mut data = Fixture::new();
    data.pair();
    data.bundle.text.cues.insert(
        2,
        TextCue {
            start_us: Some(600_500_000),
            duration_us: Some(100_000),
            text: "中間另有字幕。".into(),
        },
    );
    assert_eq!(
        data.locate().err(),
        Some(FuzzyAlignmentError::TimingMismatch)
    );
}

#[test]
fn fuzzy_paired_far_reference_repeat_defeats_unique_primary_correspondence() {
    let mut data = Fixture::new();
    data.source_mut().cues.push(TextCue {
        start_us: Some(900_000_000),
        duration_us: Some(1_000_000),
        text: "b".repeat(32).into(),
    });
    assert_eq!(
        data.locate().err(),
        Some(FuzzyAlignmentError::NoUniqueMatch)
    );
}

#[test]
fn fuzzy_paired_primary_body_is_not_subject_to_source_normalization_or_copied() {
    let mut data = Fixture::new();
    data.bundle.text.cues[2].text = "漢".repeat(512).into();
    let primary = data.bundle.text.cues[2].text.as_ptr();
    let reference = data.source().cues[1].text.as_ptr();
    assert!(data.locate().is_ok());
    assert_eq!(data.bundle.text.cues[2].text.as_ptr(), primary);
    assert_eq!(data.source().cues[1].text.as_ptr(), reference);
}

#[test]
fn fuzzy_paired_two_track_count_and_byte_budgets_are_aggregate() {
    let mut data = Fixture::new();
    data.bundle.text.cues = (0..MAX_TEXT_CUES)
        .map(|index| TextCue {
            start_us: Some(index as u64),
            duration_us: Some(1),
            text: "合成".into(),
        })
        .collect();
    assert_eq!(
        data.locate().err(),
        Some(FuzzyAlignmentError::BudgetExceeded)
    );
    let mut data = Fixture::new();
    data.bundle.text.cues = (0..MAX_TEXT_BYTES / MAX_CUE_BYTES)
        .map(|index| TextCue {
            start_us: Some(index as u64),
            duration_us: Some(1),
            text: "x".repeat(MAX_CUE_BYTES).into(),
        })
        .collect();
    assert_eq!(
        data.locate().err(),
        Some(FuzzyAlignmentError::BudgetExceeded)
    );
}

#[test]
fn fuzzy_paired_later_oversized_reference_discards_earlier_winner() {
    let mut data = Fixture::new();
    data.source_mut().cues[2].text = "x".repeat(MAX_NORMALIZED_CHARACTERS + 1).into();
    assert_eq!(
        data.locate().err(),
        Some(FuzzyAlignmentError::BudgetExceeded)
    );
}

#[test]
fn fuzzy_paired_invalid_source_evidence_and_capture_identity_are_rejected() {
    for kind in 0..4 {
        let mut data = Fixture::new();
        match kind {
            0 => data.event.language = Some(SpeechLanguage::Japanese),
            1 => data.event.kind = TranscriptKind::Partial,
            2 => data.event.identity.generation += 1,
            _ => data.event.language = None,
        }
        assert_eq!(
            data.locate().err(),
            Some(FuzzyAlignmentError::UnsupportedEvidence)
        );
    }
}

#[test]
fn fuzzy_paired_cancellation_deadline_and_dp_limit_never_return_partial_match() {
    for kind in 0..3 {
        let data = Fixture::new();
        let mut policy = MatchPolicy {
            cell_limit: MAX_DP_CELLS,
            deadline: None,
            check_hook: None,
        };
        let expected = match kind {
            0 => {
                data.cancel.cancel();
                ResolverError::Cancelled.into()
            }
            1 => {
                policy.deadline = Some(Instant::now());
                ResolverError::Stale.into()
            }
            _ => {
                policy.cell_limit = 0;
                FuzzyAlignmentError::BudgetExceeded
            }
        };
        assert_eq!(
            locate_paired_with_policy(
                data.scope,
                &data.bundle,
                &data.event,
                data.stamp,
                &data.cancel,
                policy
            )
            .err(),
            Some(expected)
        );
    }
}

#[test]
fn fuzzy_paired_primary_correspondence_loop_checks_cancellation_and_deadline() {
    let data = Fixture::new();
    let trigger = data.cancel.clone();
    let work = WorkCheck::new(
        &data.cancel,
        Instant::now() + FUZZY_MATCH_TIMEOUT,
        MatchPolicy {
            cell_limit: 0,
            deadline: None,
            check_hook: Some(Box::new(move || trigger.cancel())),
        },
    );
    assert_eq!(
        map_primary(&data.bundle.text, data.source(), 1..2, &work),
        Err(ResolverError::Cancelled.into())
    );
    let cancel = ResolverCancellation::default();
    let work = WorkCheck::new(
        &cancel,
        Instant::now(),
        MatchPolicy {
            cell_limit: 0,
            deadline: None,
            check_hook: None,
        },
    );
    assert_eq!(
        map_primary(&data.bundle.text, data.source(), 1..2, &work),
        Err(ResolverError::Stale.into())
    );
}

#[test]
fn fuzzy_paired_query_validation_retains_source_content_and_identity_checks() {
    let data = Fixture::new();
    for kind in 0..3 {
        let mut stamp = data.stamp;
        stamp.identity = data.scope.document_identity();
        let mut query = TextQuery {
            stamp,
            source: data.bundle.text.source,
            content_id: data.bundle.text.content_id.clone(),
        };
        match kind {
            0 => query.source = SubtitleSource::ExistingLyrics,
            1 => query.content_id = "other".into(),
            _ => query.stamp.identity.generation += 1,
        }
        assert_eq!(
            data.bundle
                .source_reference
                .as_ref()
                .unwrap()
                .validate(&data.bundle.text, &query),
            Err(ResolverError::InvalidData)
        );
    }
}

#[test]
fn fuzzy_paired_proposal_keeps_full_binding_epoch_and_owner_revocation() {
    let data = Fixture::new();
    let proposal = data.locate().unwrap();
    let mut binding = proposal.binding();
    binding.revision += 1;
    assert_eq!(
        proposal.validate(data.scope, binding),
        Err(FuzzyAlignmentError::Obsolete)
    );
    let other = AlignmentScope::new(
        data.scope.document_identity(),
        data.scope.capture_identity(),
        2,
    )
    .unwrap();
    assert_eq!(
        proposal.validate(other, proposal.binding()),
        Err(FuzzyAlignmentError::Obsolete)
    );
    data.cancel.cancel();
    assert_eq!(
        proposal.validate(data.scope, proposal.binding()),
        Err(ResolverError::Cancelled.into())
    );
}
