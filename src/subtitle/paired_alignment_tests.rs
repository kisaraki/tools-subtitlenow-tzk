//! Synthetic atomic provider bundles; no real lookup, capture or cloud service.
use super::*;
use crate::{
    ai::speech::SpeechLanguage,
    audio::capture::SessionId,
    resolver::{
        contracts::{
            ExistingText, ExistingTextBundle, ExistingTextProvider, SourceTextReference,
            SubtitleSource, TextCue, TextFuture, TextQuery,
        },
        text_session::{ExistingTextSession, TextLookupStatus},
    },
};
use std::{future::Future, pin::Pin};

fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(953),
        generation: 1,
    }
}
fn stamp(sequence: u64) -> WindowStamp {
    WindowStamp {
        identity: identity(),
        sequence,
        ended_at: Instant::now(),
        start_us: Some(40_000_000),
        duration_us: 12_000_000,
    }
}
fn version() -> ArbitrationVersion {
    crate::subtitle::source_arbiter::SubtitleSourceArbiter::new(
        identity(),
        crate::domain::TranslationQuality::Standard,
    )
    .version()
}
fn verifier() -> ExactCueVerifier {
    ExactCueVerifier {
        identity: identity(),
        version: version(),
        cancel: ResolverCancellation::default(),
        workers: Arc::new(Semaphore::new(1)),
    }
}
fn event(language: SpeechLanguage, text: &str) -> TranscriptEvent {
    TranscriptEvent {
        identity: identity(),
        utterance: 3,
        revision: 1,
        sequence: 1,
        start_us: Some(42_000_000),
        duration_us: Some(950_000),
        language: Some(language),
        kind: TranscriptKind::Final,
        text: text.into(),
    }
}
#[derive(Clone)]
struct CueSpec {
    start: Option<u64>,
    duration: Option<u64>,
    text: &'static str,
}
fn cue(start: u64, text: &'static str) -> CueSpec {
    CueSpec {
        start: Some(start),
        duration: Some(1_000_000),
        text,
    }
}
struct MockPaired {
    target_language: TextLanguage,
    reference_language: SpeechLanguage,
    target: Vec<CueSpec>,
    reference: Vec<CueSpec>,
    paired: bool,
}
impl MockPaired {
    fn new(
        target_language: TextLanguage,
        reference_language: SpeechLanguage,
        text: &'static str,
    ) -> Self {
        Self {
            target_language,
            reference_language,
            target: vec![
                cue(4_000_000, "較早的合成字幕"),
                cue(5_000_000, "軟體字幕測試文字"),
            ],
            reference: vec![
                cue(5_000_000, text),
                cue(6_000_000, "other synthetic words"),
            ],
            paired: true,
        }
    }
    fn document(
        &self,
        query: &TextQuery,
        language: TextLanguage,
        cues: &[CueSpec],
    ) -> ExistingText {
        ExistingText {
            identity: query.stamp.identity,
            source: query.source,
            content_id: query.content_id.clone(),
            language,
            cues: cues
                .iter()
                .map(|cue| TextCue {
                    start_us: cue.start,
                    duration_us: cue.duration,
                    text: cue.text.into(),
                })
                .collect(),
        }
    }
}
impl ExistingTextProvider for MockPaired {
    fn supports(&self, source: SubtitleSource) -> bool {
        source == SubtitleSource::ExistingLyrics
    }
    fn lookup<'a>(&'a self, _: &'a TextQuery, _: ResolverCancellation) -> TextFuture<'a> {
        Box::pin(async { panic!("paired lookup must use one atomic bundle") })
    }
    fn lookup_bundle<'a>(
        &'a self,
        query: &'a TextQuery,
        _: ResolverCancellation,
    ) -> Pin<Box<dyn Future<Output = Result<Option<ExistingTextBundle>, ResolverError>> + Send + 'a>>
    {
        Box::pin(async move {
            Ok(Some(ExistingTextBundle {
                text: self.document(query, self.target_language, &self.target),
                source_reference: self.paired.then(|| SourceTextReference {
                    text: self.document(
                        query,
                        TextLanguage::Source(self.reference_language),
                        &self.reference,
                    ),
                    primary_edition_id: "synthetic edition 1".into(),
                    reference_edition_id: "synthetic edition 1".into(),
                }),
            }))
        })
    }
}
async fn fixture(provider: MockPaired, initial: WindowStamp) -> ExistingTextLease {
    let (mut session, task) = ExistingTextSession::new(identity());
    session
        .select(
            TextQuery {
                stamp: initial,
                source: SubtitleSource::ExistingLyrics,
                content_id: "synthetic work".into(),
            },
            Arc::new(provider),
        )
        .unwrap();
    let worker = task.run();
    tokio::pin!(worker);
    let mut input = loop {
        tokio::select! {
            result = &mut worker => panic!("synthetic lookup ended: {result:?}"),
            update = session.next() => {
                let update = update.unwrap();
                if update.status == TextLookupStatus::Available {
                    break ExistingTextLease::from_update(&update).unwrap();
                }
                if let TextLookupStatus::Failed(error) = update.status {
                    panic!("synthetic bundle rejected: {error:?}");
                }
            }
        }
    };
    input.renew(initial, Confidence::new(900).unwrap()).unwrap();
    session.stop().unwrap();
    worker.await.unwrap();
    input
}
async fn verify(
    input: ExistingTextLease,
    evidence: TranscriptEvent,
) -> Result<ExactCueMatch, ExactAlignmentError> {
    verifier()
        .verify(input, evidence, stamp(10), ResolverCancellation::default())
        .await
}

#[tokio::test]
async fn all_five_source_languages_match_three_chinese_tracks_by_time_not_index() {
    for (language, text) in [
        (SpeechLanguage::English, "Synthetic source one"),
        (SpeechLanguage::Japanese, "これは合成字幕の試験です"),
        (SpeechLanguage::French, "Voici un texte synthétique"),
        (SpeechLanguage::German, "Dies ist ein synthetischer Text"),
        (SpeechLanguage::Korean, "이것은 합성 자막 테스트입니다"),
    ] {
        for target_language in [
            TextLanguage::TaiwanTraditional,
            TextLanguage::OtherTraditional,
            TextLanguage::SimplifiedChinese,
        ] {
            let input = fixture(MockPaired::new(target_language, language, text), stamp(1)).await;
            let matched = verify(input, event(language, text)).await.unwrap();
            assert_eq!(matched.source_range(), 1..2);
            assert_eq!(matched.timing_agreement().permille(), 950);
            assert_eq!(matched.input().text().cues[1].start_us, Some(5_000_000));
            assert_eq!(matched.input().text().cues[1].duration_us, Some(1_000_000));
            assert!(matched.input().source_reference().is_some());
            assert!(matched.covers_capture_position(42_500_000));
            assert!(!matched.covers_capture_position(43_000_000));
        }
    }
}

#[tokio::test]
async fn duplicate_reference_text_is_ambiguous_even_untimed_or_outside_capture() {
    for repeated in [
        cue(99_000_000, "SYNTHETIC source one"),
        CueSpec {
            start: None,
            duration: None,
            text: "synthetic source one",
        },
    ] {
        let mut provider = MockPaired::new(
            TextLanguage::TaiwanTraditional,
            SpeechLanguage::English,
            "synthetic source one",
        );
        provider.reference.push(repeated);
        let input = fixture(provider, stamp(1)).await;
        assert!(matches!(
            verify(
                input,
                event(SpeechLanguage::English, "synthetic source one")
            )
            .await,
            Err(ExactAlignmentError::NoUniqueMatch)
        ));
    }
}

#[tokio::test]
async fn duplicate_chinese_intervals_cannot_choose_one_translation() {
    let mut provider = MockPaired::new(
        TextLanguage::TaiwanTraditional,
        SpeechLanguage::English,
        "synthetic source one",
    );
    provider
        .target
        .push(cue(5_000_000, "另一份同時段的合成譯文"));
    let input = fixture(provider, stamp(1)).await;
    assert!(
        verify(
            input,
            event(SpeechLanguage::English, "synthetic source one")
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn pair_requires_exact_start_and_duration_including_one_microsecond_differences() {
    for (start, duration) in [
        (4_999_999, 1_000_000),
        (5_000_001, 1_000_000),
        (5_000_000, 999_999),
        (5_000_000, 1_000_001),
    ] {
        let mut provider = MockPaired::new(
            TextLanguage::TaiwanTraditional,
            SpeechLanguage::English,
            "synthetic source one",
        );
        provider.target[1].start = Some(start);
        provider.target[1].duration = Some(duration);
        let input = fixture(provider, stamp(1)).await;
        assert!(
            verify(
                input,
                event(SpeechLanguage::English, "synthetic source one")
            )
            .await
            .is_err()
        );
    }
}

#[tokio::test]
async fn chinese_without_reference_rejects_even_if_its_text_equals_asr() {
    let mut provider = MockPaired::new(
        TextLanguage::TaiwanTraditional,
        SpeechLanguage::English,
        "synthetic source one",
    );
    provider.paired = false;
    provider.target[1].text = "synthetic source one";
    let input = fixture(provider, stamp(1)).await;
    assert!(matches!(
        verify(
            input,
            event(SpeechLanguage::English, "synthetic source one")
        )
        .await,
        Err(ExactAlignmentError::UnsupportedEvidence)
    ));
}

#[tokio::test]
async fn language_partial_and_foreign_session_cannot_supply_pair_evidence() {
    let input = fixture(
        MockPaired::new(
            TextLanguage::TaiwanTraditional,
            SpeechLanguage::English,
            "synthetic source one",
        ),
        stamp(1),
    )
    .await;
    let base = event(SpeechLanguage::English, "synthetic source one");
    let mut cases = vec![base.clone(); 4];
    cases[0].language = Some(SpeechLanguage::French);
    cases[1].language = None;
    cases[2].kind = TranscriptKind::Partial;
    cases[3].identity.generation += 1;
    for evidence in cases {
        assert!(verify(input.clone(), evidence).await.is_err());
    }
}

#[tokio::test]
async fn atomic_publication_retirement_invalidates_unsubmitted_pair_match() {
    let input = fixture(
        MockPaired::new(
            TextLanguage::TaiwanTraditional,
            SpeechLanguage::English,
            "synthetic source one",
        ),
        stamp(1),
    )
    .await;
    let mut owner = verifier();
    let matched = owner
        .verify(
            input.clone(),
            event(SpeechLanguage::English, "synthetic source one"),
            stamp(10),
            ResolverCancellation::default(),
        )
        .await
        .unwrap();
    input.retire();
    assert!(matches!(
        matched.validate(identity()),
        Err(ExactAlignmentError::Invalid(ResolverError::Cancelled))
    ));
    assert!(
        verify(
            input,
            event(SpeechLanguage::English, "synthetic source one")
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn dropping_verifier_retires_pair_evidence_without_retiring_lookup() {
    let input = fixture(
        MockPaired::new(
            TextLanguage::TaiwanTraditional,
            SpeechLanguage::English,
            "synthetic source one",
        ),
        stamp(1),
    )
    .await;
    let mut owner = verifier();
    let matched = owner
        .verify(
            input.clone(),
            event(SpeechLanguage::English, "synthetic source one"),
            stamp(10),
            ResolverCancellation::default(),
        )
        .await
        .unwrap();
    drop(owner);
    assert!(matched.validate(identity()).is_err());
    assert!(input.validate(identity()).is_ok());
}

#[tokio::test]
async fn confirmation_renewal_does_not_extend_previously_minted_pair_evidence() {
    let mut initial = stamp(1);
    initial.ended_at -= MAX_WINDOW_AGE - Duration::from_millis(400);
    let mut input = fixture(
        MockPaired::new(
            TextLanguage::TaiwanTraditional,
            SpeechLanguage::English,
            "synthetic source one",
        ),
        initial,
    )
    .await;
    let mut owner = verifier();
    let matched = owner
        .verify(
            input.clone(),
            event(SpeechLanguage::English, "synthetic source one"),
            stamp(10),
            ResolverCancellation::default(),
        )
        .await
        .unwrap();
    input
        .renew(stamp(2), Confidence::new(900).unwrap())
        .unwrap();
    tokio::time::sleep(Duration::from_millis(450)).await;
    assert!(input.validate(identity()).is_ok());
    assert!(matches!(
        matched.validate(identity()),
        Err(ExactAlignmentError::Invalid(ResolverError::Stale))
    ));
}

#[tokio::test]
async fn identical_text_query_and_timing_do_not_allow_cross_publication_proof() {
    let initial = stamp(1);
    let first = fixture(
        MockPaired::new(
            TextLanguage::TaiwanTraditional,
            SpeechLanguage::English,
            "synthetic source one",
        ),
        initial,
    )
    .await;
    let second = fixture(
        MockPaired::new(
            TextLanguage::TaiwanTraditional,
            SpeechLanguage::English,
            "synthetic source one",
        ),
        initial,
    )
    .await;
    let source = event(SpeechLanguage::English, "synthetic source one");
    let mut owner = verifier();
    let matched = owner
        .verify(
            first.clone(),
            source.clone(),
            stamp(10),
            ResolverCancellation::default(),
        )
        .await
        .unwrap();
    let document = second.text();
    let owned = ExistingText {
        identity: document.identity,
        source: document.source,
        content_id: document.content_id.clone(),
        language: document.language,
        cues: document
            .cues
            .iter()
            .map(|cue| TextCue {
                start_us: cue.start_us,
                duration_us: cue.duration_us,
                text: cue.text.clone(),
            })
            .collect(),
    };
    let mut localizer =
        crate::subtitle::existing_text::ChineseTextLocalizer::new(identity()).unwrap();
    let prepared = localizer
        .normalize(
            TextQuery {
                stamp: second.query().stamp,
                source: second.query().source,
                content_id: second.query().content_id.clone(),
            },
            owned,
            ResolverCancellation::default(),
        )
        .await
        .unwrap();
    let mut timeline = TranscriptTimeline::new(identity());
    timeline.ingest(source).unwrap();
    assert!(matches!(
        matched.evidence(
            &prepared,
            0..prepared.original().cues.len(),
            &timeline,
            version(),
            &second.availability(),
        ),
        Err(ExactAlignmentError::Obsolete)
    ));
    assert!(first.validate(identity()).is_ok());
    assert!(second.validate(identity()).is_ok());
}

#[tokio::test]
async fn weaker_confirmation_revokes_both_tracks_before_matching_again() {
    let mut input = fixture(
        MockPaired::new(
            TextLanguage::TaiwanTraditional,
            SpeechLanguage::English,
            "synthetic source one",
        ),
        stamp(1),
    )
    .await;
    let mut owner = verifier();
    let matched = owner
        .verify(
            input.clone(),
            event(SpeechLanguage::English, "synthetic source one"),
            stamp(10),
            ResolverCancellation::default(),
        )
        .await
        .unwrap();
    input
        .renew(stamp(2), Confidence::new(810).unwrap())
        .unwrap();
    assert!(matched.validate(identity()).is_err());
    assert!(matches!(
        verify(
            input,
            event(SpeechLanguage::English, "synthetic source one")
        )
        .await,
        Err(ExactAlignmentError::UnsupportedEvidence)
    ));
}
