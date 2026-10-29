//! Original synthetic text and explicitly synthetic alignment evidence only.
use super::*;
use crate::{
    ai::{speech::SpeechLanguage, translation::*},
    audio::capture::SessionId,
    resolver::contracts::{ExistingText, TextCue, TextQuery},
    subtitle::{existing_text::ChineseTextLocalizer, source_translation::SourceTextTranslator},
};
use std::time::Duration;

const CAPTURE_START: u64 = 42_000_000;
const MEDIA_START: u64 = 5_000_000;
const CUE_DURATION: u64 = 1_000_000;
fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(908),
        generation: 1,
    }
}
fn arbiter(quality: TranslationQuality) -> SubtitleSourceArbiter {
    SubtitleSourceArbiter::new(identity(), quality)
}
fn fixture(
    sequence: u64,
    language: TextLanguage,
    source: SubtitleSource,
) -> (TextQuery, ExistingText) {
    let query = TextQuery {
        stamp: WindowStamp {
            identity: identity(),
            sequence,
            ended_at: Instant::now(),
            start_us: Some(40_000_000),
            duration_us: 12_000_000,
        },
        source,
        content_id: "synthetic work".into(),
    };
    let text = ExistingText {
        identity: identity(),
        source,
        content_id: query.content_id.clone(),
        language,
        cues: vec![TextCue {
            start_us: Some(MEDIA_START),
            duration_us: Some(CUE_DURATION),
            text: match language {
                TextLanguage::TaiwanTraditional => "原生臺灣軟體",
                TextLanguage::OtherTraditional => "軟件",
                TextLanguage::SimplifiedChinese => "软件",
                TextLanguage::Source(_) => "synthetic software",
            }
            .into(),
        }],
    };
    (query, text)
}
async fn localized(sequence: u64, language: TextLanguage, source: SubtitleSource) -> LocalizedText {
    let (query, text) = fixture(sequence, language, source);
    ChineseTextLocalizer::new(identity())
        .unwrap()
        .normalize(query, text, ResolverCancellation::default())
        .await
        .unwrap()
}
fn evidence(version: ArbitrationVersion, text: &LocalizedText) -> CandidateEvidence {
    let original = text.original();
    CandidateEvidence {
        content_confidence: Confidence::new(MIN_CONTENT_CONFIDENCE).unwrap(),
        alignment: CueAlignment {
            version,
            stamp: text.stamp(),
            text_sequence: text.stamp().sequence,
            content_id: original.content_id.clone(),
            source: original.source,
            matched_original: original.cues[0].text.clone(),
            cue_index: 0,
            media_start_us: original.cues[0].start_us.unwrap_or(MEDIA_START),
            capture_start_us: CAPTURE_START,
            confidence: Confidence::new(MIN_ALIGNMENT_CONFIDENCE).unwrap(),
        },
        availability: ResolverCancellation::default(),
        lookup_availability: None,
        preparation_availability: None,
    }
}
fn observation(
    owner: &SubtitleSourceArbiter,
    sequence: u64,
    position: u64,
    now: Instant,
) -> PlaybackObservation {
    PlaybackObservation {
        version: owner.version(),
        sequence,
        observed_at: now,
        capture_position_us: position,
    }
}
fn select_source(
    owner: &mut SubtitleSourceArbiter,
    sequence: u64,
    position: u64,
    now: Instant,
) -> SubtitleSource {
    let obs = observation(owner, sequence, position, now);
    owner.select(obs, now).unwrap().source()
}
struct MockTranslator;
impl Translator for MockTranslator {
    fn capabilities(&self) -> TranslationCapabilities {
        TranslationCapabilities {
            streaming: true,
            cancellation: true,
            supported_languages: [true; 5],
            max_concurrent: 1,
        }
    }
    fn translate<'a>(
        &'a self,
        _: &'a TranslationRequest,
        _: &'a mut TranslationEmitter,
        _: TranslationCancellation,
    ) -> TranslationFuture<'a> {
        Box::pin(async { Ok("translated synthetic 軟體".into()) })
    }
}
async fn translated(sequence: u64, quality: TranslationQuality) -> TranslatedTextWindow {
    let (query, text) = fixture(
        sequence,
        TextLanguage::Source(SpeechLanguage::English),
        SubtitleSource::ExistingSubtitle,
    );
    SourceTextTranslator::new(identity(), quality, &MockTranslator)
        .unwrap()
        .translate_window(query, &text, 0..1, ResolverCancellation::default())
        .await
        .unwrap()
}
#[tokio::test]
async fn translated_subset_keeps_document_index_and_original_media_time() {
    let mut owner = arbiter(TranslationQuality::Standard);
    let (query, mut text) = fixture(
        1,
        TextLanguage::Source(SpeechLanguage::English),
        SubtitleSource::ExistingSubtitle,
    );
    for index in 1..3 {
        text.cues.push(TextCue {
            start_us: Some(MEDIA_START + index * CUE_DURATION),
            duration_us: Some(CUE_DURATION),
            text: format!("synthetic source cue {index}").into(),
        });
    }
    let window =
        SourceTextTranslator::new(identity(), TranslationQuality::Standard, &MockTranslator)
            .unwrap()
            .translate_window(query, &text, 2..3, ResolverCancellation::default())
            .await
            .unwrap();
    let proof = evidence(owner.version(), window.localized());
    owner
        .submit_translated(window, proof, Instant::now())
        .unwrap();
    let now = Instant::now();
    let obs = observation(&owner, 1, CAPTURE_START, now);
    let SourceSelection::Existing { cue, .. } = owner.select(obs, now).unwrap() else {
        panic!("expected translated subset")
    };
    assert_eq!(cue.cue_index, 0);
    assert_eq!(cue.document_cue_index, 2);
    assert_eq!(cue.media_start_us, MEDIA_START + 2 * CUE_DURATION);
    assert_eq!(cue.original, "synthetic source cue 2");
}
#[test]
fn no_evidence_uses_live_ai_and_never_infers_position() {
    let mut owner = arbiter(TranslationQuality::Standard);
    assert_eq!(
        select_source(&mut owner, 1, CAPTURE_START, Instant::now()),
        SubtitleSource::LiveAi
    );
    assert_eq!(owner.candidate_count(), 0);
}
#[tokio::test]
async fn only_the_explicit_capture_interval_is_adopted_with_original_and_target() {
    let mut owner = arbiter(TranslationQuality::Standard);
    let text = localized(
        1,
        TextLanguage::SimplifiedChinese,
        SubtitleSource::ExistingSubtitle,
    )
    .await;
    let proof = evidence(owner.version(), &text);
    owner.submit_localized(text, proof, Instant::now()).unwrap();
    assert_eq!(
        select_source(&mut owner, 1, CAPTURE_START - 1, Instant::now()),
        SubtitleSource::LiveAi
    );
    let now = Instant::now();
    let obs = observation(&owner, 2, CAPTURE_START, now);
    let SourceSelection::Existing { source, cue } = owner.select(obs, now).unwrap() else {
        panic!("expected explicit cue")
    };
    assert_eq!(source, SubtitleSource::ExistingSubtitle);
    assert_eq!(cue.original, "软件");
    assert_eq!(cue.translated, "軟體");
    assert_eq!(
        (cue.capture_start_us, cue.capture_end_us),
        (CAPTURE_START, CAPTURE_START + CUE_DURATION)
    );
    assert_eq!(
        select_source(
            &mut owner,
            3,
            CAPTURE_START + CUE_DURATION - 1,
            Instant::now()
        ),
        SubtitleSource::ExistingSubtitle
    );
    assert_eq!(
        select_source(&mut owner, 4, CAPTURE_START + CUE_DURATION, Instant::now()),
        SubtitleSource::LiveAi
    );
}
#[tokio::test]
async fn language_preference_follows_spec_and_returns_actual_prepared_text() {
    let mut owner = arbiter(TranslationQuality::Standard);
    let window = translated(1, TranslationQuality::Standard).await;
    let proof = evidence(owner.version(), window.localized());
    owner
        .submit_translated(window, proof, Instant::now())
        .unwrap();
    for (index, language) in [
        TextLanguage::SimplifiedChinese,
        TextLanguage::OtherTraditional,
        TextLanguage::TaiwanTraditional,
    ]
    .into_iter()
    .enumerate()
    {
        let text = localized(index as u64 + 2, language, SubtitleSource::ExistingSubtitle).await;
        let proof = evidence(owner.version(), &text);
        owner.submit_localized(text, proof, Instant::now()).unwrap();
        let now = Instant::now();
        let obs = observation(&owner, index as u64 + 1, CAPTURE_START, now);
        let SourceSelection::Existing { cue, .. } = owner.select(obs, now).unwrap() else {
            panic!("expected preferred candidate")
        };
        assert_eq!(
            cue.translated,
            if index == 2 {
                "原生臺灣軟體"
            } else {
                "軟體"
            }
        );
        assert_eq!(cue.quality, None);
    }
    assert_eq!(owner.candidate_count(), MAX_CANDIDATES);
}
#[tokio::test]
async fn weak_evidence_does_not_replace_valid_active_source() {
    let mut owner = arbiter(TranslationQuality::Standard);
    let text = localized(
        1,
        TextLanguage::SimplifiedChinese,
        SubtitleSource::ExistingSubtitle,
    )
    .await;
    let proof = evidence(owner.version(), &text);
    owner.submit_localized(text, proof, Instant::now()).unwrap();
    for weak_alignment in [false, true] {
        let text = localized(
            2,
            TextLanguage::TaiwanTraditional,
            SubtitleSource::ExistingSubtitle,
        )
        .await;
        let mut proof = evidence(owner.version(), &text);
        if weak_alignment {
            proof.alignment.confidence = Confidence::new(MIN_ALIGNMENT_CONFIDENCE - 1).unwrap();
        } else {
            proof.content_confidence = Confidence::new(MIN_CONTENT_CONFIDENCE - 1).unwrap();
        }
        assert_eq!(
            owner.submit_localized(text, proof, Instant::now()),
            Err(ArbitrationError::WeakEvidence)
        );
    }
    assert_eq!(owner.candidate_count(), 1);
    assert_eq!(
        select_source(&mut owner, 1, CAPTURE_START, Instant::now()),
        SubtitleSource::ExistingSubtitle
    );
}
#[tokio::test]
async fn alignment_is_bound_to_query_document_source_cue_and_capture_window() {
    for case in 0..10 {
        let mut owner = arbiter(TranslationQuality::Standard);
        let text = localized(
            1,
            TextLanguage::SimplifiedChinese,
            SubtitleSource::ExistingSubtitle,
        )
        .await;
        let mut proof = evidence(owner.version(), &text);
        match case {
            0 => proof.alignment.content_id = "different work".into(),
            1 => proof.alignment.source = SubtitleSource::ExistingLyrics,
            2 => proof.alignment.matched_original = "different cue".into(),
            3 => proof.alignment.text_sequence += 1,
            4 => proof.alignment.cue_index = 1,
            5 => proof.alignment.media_start_us += 1,
            6 => proof.alignment.capture_start_us = 39_999_999,
            7 => proof.alignment.capture_start_us = 52_000_000,
            8 => proof.alignment.capture_start_us = u64::MAX,
            _ => proof.alignment.stamp.start_us = None,
        }
        assert_eq!(
            owner.submit_localized(text, proof, Instant::now()),
            Err(ArbitrationError::InvalidAlignment)
        );
        assert_eq!(owner.candidate_count(), 0);
    }
}
#[tokio::test]
async fn translation_strategy_must_meet_requested_quality() {
    for required in [TranslationQuality::Standard, TranslationQuality::High] {
        for supplied in [
            TranslationQuality::Fast,
            TranslationQuality::Standard,
            TranslationQuality::High,
        ] {
            let mut owner = arbiter(required);
            let window = translated(1, supplied).await;
            let proof = evidence(owner.version(), window.localized());
            let result = owner.submit_translated(window, proof, Instant::now());
            if quality_rank(supplied) < quality_rank(required) {
                assert_eq!(result, Err(ArbitrationError::InsufficientQuality));
            } else {
                result.unwrap();
                let now = Instant::now();
                let obs = observation(&owner, 1, CAPTURE_START, now);
                let SourceSelection::Existing { cue, .. } = owner.select(obs, now).unwrap() else {
                    panic!("expected translated cue")
                };
                assert_eq!(cue.quality, Some(supplied));
                assert_eq!(cue.translated, "translated synthetic 軟體");
            }
        }
    }
}
#[tokio::test]
async fn equal_priority_keeps_active_source_and_gap_fills_from_live_ai() {
    let mut owner = arbiter(TranslationQuality::Standard);
    let text = localized(
        1,
        TextLanguage::TaiwanTraditional,
        SubtitleSource::ExistingSubtitle,
    )
    .await;
    let proof = evidence(owner.version(), &text);
    owner.submit_localized(text, proof, Instant::now()).unwrap();
    assert_eq!(
        select_source(&mut owner, 1, CAPTURE_START, Instant::now()),
        SubtitleSource::ExistingSubtitle
    );
    let text = localized(
        2,
        TextLanguage::TaiwanTraditional,
        SubtitleSource::ExistingLyrics,
    )
    .await;
    let mut proof = evidence(owner.version(), &text);
    proof.alignment.capture_start_us += CUE_DURATION / 2;
    owner.submit_localized(text, proof, Instant::now()).unwrap();
    assert_eq!(
        select_source(
            &mut owner,
            2,
            CAPTURE_START + CUE_DURATION / 2,
            Instant::now()
        ),
        SubtitleSource::ExistingSubtitle
    );
    assert_eq!(
        select_source(&mut owner, 3, CAPTURE_START + CUE_DURATION, Instant::now()),
        SubtitleSource::ExistingLyrics
    );
    assert_eq!(
        select_source(
            &mut owner,
            4,
            CAPTURE_START + CUE_DURATION * 2,
            Instant::now()
        ),
        SubtitleSource::LiveAi
    );
}
#[tokio::test]
async fn cancelled_candidate_is_retired_and_live_ai_resumes() {
    let mut owner = arbiter(TranslationQuality::Standard);
    let text = localized(
        1,
        TextLanguage::TaiwanTraditional,
        SubtitleSource::ExistingLyrics,
    )
    .await;
    let proof = evidence(owner.version(), &text);
    let cancel = proof.availability.clone();
    owner.submit_localized(text, proof, Instant::now()).unwrap();
    assert_eq!(
        select_source(&mut owner, 1, CAPTURE_START, Instant::now()),
        SubtitleSource::ExistingLyrics
    );
    cancel.cancel();
    assert_eq!(
        select_source(&mut owner, 2, CAPTURE_START, Instant::now()),
        SubtitleSource::LiveAi
    );
    assert_eq!(owner.candidate_count(), 0);
}
#[tokio::test]
async fn original_query_freshness_is_not_renewed_by_later_alignment() {
    let mut owner = arbiter(TranslationQuality::Standard);
    let text = localized(
        1,
        TextLanguage::TaiwanTraditional,
        SubtitleSource::ExistingSubtitle,
    )
    .await;
    let stamp = text.stamp();
    let mut proof = evidence(owner.version(), &text);
    proof.alignment.stamp.ended_at = stamp.ended_at + Duration::from_secs(2);
    let cancel = proof.availability.clone();
    let now = stamp.ended_at + Duration::from_secs(2);
    owner.submit_localized(text, proof, now).unwrap();
    assert_eq!(
        select_source(&mut owner, 1, CAPTURE_START, now),
        SubtitleSource::ExistingSubtitle
    );
    let now = stamp.ended_at + MAX_WINDOW_AGE + Duration::from_nanos(1);
    assert_eq!(
        select_source(&mut owner, 2, CAPTURE_START, now),
        SubtitleSource::LiveAi
    );
    assert!(cancel.is_cancelled());
    assert_eq!(owner.candidate_count(), 0);
}
#[tokio::test]
async fn foreign_session_generation_future_and_old_alignment_are_rejected() {
    for case in 0..4 {
        let mut owner = arbiter(TranslationQuality::Standard);
        let text = localized(
            1,
            TextLanguage::TaiwanTraditional,
            SubtitleSource::ExistingSubtitle,
        )
        .await;
        let mut proof = evidence(owner.version(), &text);
        match case {
            0 => proof.alignment.stamp.identity.session = SessionId(999),
            1 => proof.alignment.stamp.identity.generation += 1,
            2 => proof.alignment.stamp.ended_at = Instant::now() + Duration::from_secs(1),
            _ => proof.alignment.stamp.ended_at -= MAX_WINDOW_AGE + Duration::from_secs(1),
        }
        assert!(owner.submit_localized(text, proof, Instant::now()).is_err());
        assert_eq!(owner.candidate_count(), 0);
    }
}
#[tokio::test]
async fn rejected_observations_do_not_change_active_candidate_or_clock() {
    let mut owner = arbiter(TranslationQuality::Standard);
    let text = localized(
        1,
        TextLanguage::TaiwanTraditional,
        SubtitleSource::ExistingSubtitle,
    )
    .await;
    let proof = evidence(owner.version(), &text);
    owner.submit_localized(text, proof, Instant::now()).unwrap();
    let now = Instant::now();
    assert_eq!(
        select_source(&mut owner, 2, CAPTURE_START, now),
        SubtitleSource::ExistingSubtitle
    );
    for case in 0..6 {
        let mut obs = observation(&owner, 3, CAPTURE_START, now);
        match case {
            0 => obs.sequence = 2,
            1 => obs.capture_position_us -= 1,
            2 => obs.observed_at -= Duration::from_secs(1),
            3 => obs.observed_at += Duration::from_secs(1),
            4 => obs.version.identity.generation += 1,
            _ => obs.observed_at -= MAX_WINDOW_AGE + Duration::from_secs(1),
        }
        assert!(matches!(
            owner.select(obs, now),
            Err(ArbitrationError::Obsolete)
        ));
    }
    assert_eq!(
        select_source(&mut owner, 3, CAPTURE_START, now),
        SubtitleSource::ExistingSubtitle
    );
}
#[tokio::test]
async fn invalidation_cancels_candidates_and_rejects_late_previous_epoch_results() {
    let mut owner = arbiter(TranslationQuality::Standard);
    let text = localized(
        3,
        TextLanguage::TaiwanTraditional,
        SubtitleSource::ExistingSubtitle,
    )
    .await;
    let proof = evidence(owner.version(), &text);
    let old_version = owner.version();
    let cancel = proof.availability.clone();
    owner.submit_localized(text, proof, Instant::now()).unwrap();
    owner.invalidate().unwrap();
    assert!(cancel.is_cancelled());
    for (sequence, version) in [(4, old_version), (3, owner.version())] {
        let text = localized(
            sequence,
            TextLanguage::TaiwanTraditional,
            SubtitleSource::ExistingSubtitle,
        )
        .await;
        let proof = evidence(version, &text);
        assert_eq!(
            owner.submit_localized(text, proof, Instant::now()),
            Err(ArbitrationError::Obsolete)
        );
    }
    let text = localized(
        4,
        TextLanguage::TaiwanTraditional,
        SubtitleSource::ExistingSubtitle,
    )
    .await;
    let proof = evidence(owner.version(), &text);
    owner.submit_localized(text, proof, Instant::now()).unwrap();
    assert_eq!(
        select_source(&mut owner, 1, CAPTURE_START, Instant::now()),
        SubtitleSource::ExistingSubtitle
    );
}
#[tokio::test]
async fn replacement_and_expiry_watermarks_prevent_late_old_result_reentry() {
    let mut owner = arbiter(TranslationQuality::Standard);
    let text = localized(
        2,
        TextLanguage::TaiwanTraditional,
        SubtitleSource::ExistingSubtitle,
    )
    .await;
    let proof = evidence(owner.version(), &text);
    let old_cancel = proof.availability.clone();
    owner.submit_localized(text, proof, Instant::now()).unwrap();
    let text = localized(
        3,
        TextLanguage::TaiwanTraditional,
        SubtitleSource::ExistingSubtitle,
    )
    .await;
    let proof = evidence(owner.version(), &text);
    let current_cancel = proof.availability.clone();
    owner.submit_localized(text, proof, Instant::now()).unwrap();
    assert!(old_cancel.is_cancelled());
    assert_eq!(owner.candidate_count(), 1);
    current_cancel.cancel();
    assert_eq!(
        select_source(&mut owner, 1, CAPTURE_START, Instant::now()),
        SubtitleSource::LiveAi
    );
    for seq in [2, 3] {
        let text = localized(
            seq,
            TextLanguage::TaiwanTraditional,
            SubtitleSource::ExistingSubtitle,
        )
        .await;
        let proof = evidence(owner.version(), &text);
        assert_eq!(
            owner.submit_localized(text, proof, Instant::now()),
            Err(ArbitrationError::Obsolete)
        );
    }
}
#[tokio::test]
async fn capacity_is_bounded_and_overload_does_not_evict_valid_text() {
    let mut owner = arbiter(TranslationQuality::Standard);
    for (index, source) in [
        SubtitleSource::ExistingSubtitle,
        SubtitleSource::ExistingLyrics,
    ]
    .into_iter()
    .enumerate()
    {
        for (offset, lang) in [
            TextLanguage::TaiwanTraditional,
            TextLanguage::OtherTraditional,
        ]
        .into_iter()
        .enumerate()
        {
            let text = localized((index * 2 + offset + 1) as u64, lang, source).await;
            let proof = evidence(owner.version(), &text);
            owner.submit_localized(text, proof, Instant::now()).unwrap();
        }
    }
    let text = localized(
        5,
        TextLanguage::SimplifiedChinese,
        SubtitleSource::ExistingSubtitle,
    )
    .await;
    let proof = evidence(owner.version(), &text);
    assert_eq!(
        owner.submit_localized(text, proof, Instant::now()),
        Err(ArbitrationError::Overloaded)
    );
    assert_eq!(owner.candidate_count(), MAX_CANDIDATES);
    assert_ne!(
        select_source(&mut owner, 1, CAPTURE_START, Instant::now()),
        SubtitleSource::LiveAi
    );
}
#[tokio::test]
async fn untimed_lyrics_cannot_gain_fake_timing_from_evidence() {
    let mut owner = arbiter(TranslationQuality::Standard);
    let (query, mut text) = fixture(
        1,
        TextLanguage::TaiwanTraditional,
        SubtitleSource::ExistingLyrics,
    );
    text.cues[0].start_us = None;
    text.cues[0].duration_us = None;
    let text = ChineseTextLocalizer::new(identity())
        .unwrap()
        .normalize(query, text, ResolverCancellation::default())
        .await
        .unwrap();
    let proof = evidence(owner.version(), &text);
    assert_eq!(
        owner.submit_localized(text, proof, Instant::now()),
        Err(ArbitrationError::InvalidAlignment)
    );
}
#[tokio::test]
async fn stop_drop_and_exhausted_version_clear_and_cancel_owned_candidates() {
    for drop_owner in [false, true] {
        let mut owner = arbiter(TranslationQuality::Standard);
        let text = localized(
            1,
            TextLanguage::TaiwanTraditional,
            SubtitleSource::ExistingSubtitle,
        )
        .await;
        let proof = evidence(owner.version(), &text);
        let cancel = proof.availability.clone();
        owner.submit_localized(text, proof, Instant::now()).unwrap();
        if drop_owner {
            drop(owner);
        } else {
            owner.stop();
            owner.stop();
            assert_eq!(owner.candidate_count(), 0);
            assert!(matches!(
                owner.select(
                    observation(&owner, 1, CAPTURE_START, Instant::now()),
                    Instant::now()
                ),
                Err(ArbitrationError::Stopped)
            ));
        }
        assert!(cancel.is_cancelled());
    }
    let mut owner = arbiter(TranslationQuality::Standard);
    owner.version.epoch = u64::MAX;
    assert_eq!(owner.invalidate(), Err(ArbitrationError::Stopped));
    assert!(owner.stopped);
}
