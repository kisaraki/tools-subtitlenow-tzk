//! Synthetic lookup documents and ASR; no fabricated production scores/services.
use super::*;
use crate::{
    ai::speech::SpeechLanguage,
    audio::capture::SessionId,
    resolver::{
        contracts::{
            ExistingText, ExistingTextProvider, SubtitleSource, TextCue, TextFuture, TextQuery,
        },
        text_session::{ExistingTextSession, TextLookupStatus},
    },
};
fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(930),
        generation: 1,
    }
}
fn stamp() -> WindowStamp {
    WindowStamp {
        identity: identity(),
        sequence: 1,
        ended_at: Instant::now(),
        start_us: Some(40_000_000),
        duration_us: 12_000_000,
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
struct MockLookup {
    language: TextLanguage,
    lines: Vec<&'static str>,
    timed: bool,
}
impl ExistingTextProvider for MockLookup {
    fn supports(&self, source: SubtitleSource) -> bool {
        source == SubtitleSource::ExistingLyrics
    }
    fn lookup<'a>(&'a self, query: &'a TextQuery, _: ResolverCancellation) -> TextFuture<'a> {
        Box::pin(async move {
            Ok(Some(ExistingText {
                identity: query.stamp.identity,
                source: query.source,
                content_id: query.content_id.clone(),
                language: self.language,
                cues: self
                    .lines
                    .iter()
                    .enumerate()
                    .map(|(index, text)| TextCue {
                        start_us: self.timed.then_some(5_000_000 + index as u64 * 1_000_000),
                        duration_us: self.timed.then_some(1_000_000),
                        text: (*text).into(),
                    })
                    .collect(),
            }))
        })
    }
}
async fn fixture(
    language: TextLanguage,
    lines: Vec<&'static str>,
    timed: bool,
) -> ExistingTextLease {
    fixture_at(language, lines, timed, stamp()).await
}
async fn fixture_at(
    language: TextLanguage,
    lines: Vec<&'static str>,
    timed: bool,
    initial: WindowStamp,
) -> ExistingTextLease {
    let (mut session, task) = ExistingTextSession::new(identity());
    session
        .select(
            TextQuery {
                stamp: initial,
                source: SubtitleSource::ExistingLyrics,
                content_id: "synthetic work".into(),
            },
            Arc::new(MockLookup {
                language,
                lines,
                timed,
            }),
        )
        .unwrap();
    let work = task.run();
    tokio::pin!(work);
    let mut lease = loop {
        tokio::select! {
            result = &mut work => panic!("mock lookup ended: {result:?}"),
            update = session.next() => {
                let update = update.unwrap();
                if update.status == TextLookupStatus::Available {
                    break ExistingTextLease::from_update(&update).unwrap();
                }
            }
        }
    };
    lease.renew(initial, Confidence::new(900).unwrap()).unwrap();
    session.stop().unwrap();
    work.await.unwrap();
    lease
}
fn verifier() -> ExactCueVerifier {
    // Independent capacity makes policy tests deterministic under cargo parallelism.
    ExactCueVerifier {
        identity: identity(),
        version: version(),
        cancel: ResolverCancellation::default(),
        workers: Arc::new(Semaphore::new(1)),
    }
}
fn version() -> ArbitrationVersion {
    crate::subtitle::source_arbiter::SubtitleSourceArbiter::new(
        identity(),
        crate::domain::TranslationQuality::Standard,
    )
    .version()
}
async fn verify(
    input: ExistingTextLease,
    event: TranscriptEvent,
) -> Result<ExactCueMatch, ExactAlignmentError> {
    verifier()
        .verify(input, event, stamp(), ResolverCancellation::default())
        .await
}
#[tokio::test]
async fn five_languages_unique_full_cue_and_measured_duration() {
    for (language, text) in [
        (SpeechLanguage::English, "Synthetic source one"),
        (SpeechLanguage::Japanese, "これは合成字幕の試験です"),
        (SpeechLanguage::French, "Voici un texte synthétique"),
        (SpeechLanguage::German, "Dies ist ein synthetischer Text"),
        (SpeechLanguage::Korean, "이것은 합성 자막 테스트입니다"),
    ] {
        let input = fixture(
            TextLanguage::Source(language),
            vec!["other synthetic words", text],
            true,
        )
        .await;
        let matched = verify(input, event(language, text)).await.unwrap();
        assert_eq!(matched.source_range(), 1..2);
        assert_eq!(matched.timing_agreement().permille(), 950);
        assert_eq!(matched.input().text().cues.len(), 2);
    }
}
#[tokio::test]
async fn normalization_preserves_word_boundaries_and_punctuation() {
    let input = fixture(
        TextLanguage::Source(SpeechLanguage::English),
        vec!["synthetic source one!"],
        true,
    )
    .await;
    assert!(
        verify(
            input.clone(),
            event(SpeechLanguage::English, "  SYNTHETIC\n source\t one! ")
        )
        .await
        .is_ok()
    );
    for text in [
        "synthetic source one",
        "syntheticsource one!",
        "synthetic source",
    ] {
        assert!(matches!(
            verify(input.clone(), event(SpeechLanguage::English, text)).await,
            Err(ExactAlignmentError::NoUniqueMatch)
        ));
    }
}
#[tokio::test]
async fn repeated_chorus_is_ambiguous_across_entire_document() {
    let input = fixture(
        TextLanguage::Source(SpeechLanguage::English),
        vec![
            "synthetic source one",
            "other synthetic words",
            "SYNTHETIC source one",
        ],
        true,
    )
    .await;
    assert!(matches!(
        verify(
            input,
            event(SpeechLanguage::English, "synthetic source one")
        )
        .await,
        Err(ExactAlignmentError::NoUniqueMatch)
    ));
}
#[tokio::test]
async fn partial_unknown_language_wrong_language_chinese_and_short_text_reject() {
    let input = fixture(
        TextLanguage::Source(SpeechLanguage::English),
        vec!["synthetic source one"],
        true,
    )
    .await;
    let base = event(SpeechLanguage::English, "synthetic source one");
    let mut cases = vec![base.clone(); 5];
    cases[0].kind = TranscriptKind::Partial;
    cases[1].language = None;
    cases[2].language = Some(SpeechLanguage::French);
    cases[3].text = "yes".into();
    cases[4].text = "synthetic\0source one".into();
    for evidence in cases {
        assert!(matches!(
            verify(input.clone(), evidence).await,
            Err(ExactAlignmentError::UnsupportedEvidence)
        ));
    }
    let chinese = fixture(
        TextLanguage::TaiwanTraditional,
        vec!["這是合成字幕測試文字"],
        true,
    )
    .await;
    assert!(matches!(
        verify(chinese, base).await,
        Err(ExactAlignmentError::UnsupportedEvidence)
    ));
}
#[tokio::test]
async fn timing_unknown_overflow_and_length_disagreement_reject() {
    let input = fixture(
        TextLanguage::Source(SpeechLanguage::English),
        vec!["synthetic source one"],
        true,
    )
    .await;
    let base = event(SpeechLanguage::English, "synthetic source one");
    for duration in [
        None,
        Some(0),
        Some(899_999),
        Some(1_000_001),
        Some(u64::MAX),
    ] {
        let mut evidence = base.clone();
        evidence.duration_us = duration;
        assert!(verify(input.clone(), evidence).await.is_err());
    }
    let mut no_start = base.clone();
    no_start.start_us = None;
    assert!(verify(input.clone(), no_start).await.is_err());
    let untimed = fixture(
        TextLanguage::Source(SpeechLanguage::English),
        vec!["synthetic source one"],
        false,
    )
    .await;
    assert!(matches!(
        verify(untimed, base).await,
        Err(ExactAlignmentError::TimingMismatch)
    ));
}
#[tokio::test]
async fn mapped_whole_cue_must_fit_actual_capture_window() {
    let input = fixture(
        TextLanguage::Source(SpeechLanguage::English),
        vec!["synthetic source one"],
        true,
    )
    .await;
    let mut actual = stamp();
    actual.start_us = Some(42_000_000);
    actual.duration_us = 950_000;
    assert!(matches!(
        verifier()
            .verify(
                input.clone(),
                event(SpeechLanguage::English, "synthetic source one"),
                actual,
                ResolverCancellation::default()
            )
            .await,
        Err(ExactAlignmentError::TimingMismatch)
    ));
    actual.start_us = Some(43_000_000);
    actual.duration_us = 1_000_000;
    assert!(
        verifier()
            .verify(
                input,
                event(SpeechLanguage::English, "synthetic source one"),
                actual,
                ResolverCancellation::default()
            )
            .await
            .is_err()
    );
}
#[tokio::test]
async fn foreign_session_stale_and_backwards_proof_reject() {
    let input = fixture(
        TextLanguage::Source(SpeechLanguage::English),
        vec!["synthetic source one"],
        true,
    )
    .await;
    let mut evidence = event(SpeechLanguage::English, "synthetic source one");
    evidence.identity.generation += 1;
    assert!(verify(input.clone(), evidence).await.is_err());
    let mut proof = stamp();
    proof.ended_at -= MAX_WINDOW_AGE + Duration::from_millis(1);
    assert!(matches!(
        verifier()
            .verify(
                input.clone(),
                event(SpeechLanguage::English, "synthetic source one"),
                proof,
                ResolverCancellation::default()
            )
            .await,
        Err(ExactAlignmentError::Invalid(ResolverError::Stale))
    ));
    proof = stamp();
    proof.sequence = 0;
    assert!(
        verifier()
            .verify(
                input,
                event(SpeechLanguage::English, "synthetic source one"),
                proof,
                ResolverCancellation::default()
            )
            .await
            .is_err()
    );
}
#[tokio::test]
async fn cancelled_lookup_external_and_stopped_owner_reject() {
    let input = fixture(
        TextLanguage::Source(SpeechLanguage::English),
        vec!["synthetic source one"],
        true,
    )
    .await;
    let cancel = ResolverCancellation::default();
    cancel.cancel();
    assert!(matches!(
        verifier()
            .verify(
                input.clone(),
                event(SpeechLanguage::English, "synthetic source one"),
                stamp(),
                cancel
            )
            .await,
        Err(ExactAlignmentError::Invalid(ResolverError::Cancelled))
    ));
    let mut owner = verifier();
    owner.stop();
    assert!(
        owner
            .verify(
                input.clone(),
                event(SpeechLanguage::English, "synthetic source one"),
                stamp(),
                ResolverCancellation::default()
            )
            .await
            .is_err()
    );
    input.retire();
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
async fn saturation_has_no_wait_queue_and_preserves_lookup() {
    let input = fixture(
        TextLanguage::Source(SpeechLanguage::English),
        vec!["synthetic source one"],
        true,
    )
    .await;
    let mut owner = verifier();
    let permit = owner.workers.clone().acquire_owned().await.unwrap();
    assert!(matches!(
        owner
            .verify(
                input.clone(),
                event(SpeechLanguage::English, "synthetic source one"),
                stamp(),
                ResolverCancellation::default()
            )
            .await,
        Err(ExactAlignmentError::Overloaded)
    ));
    assert!(input.validate(identity()).is_ok());
    drop(permit);
    assert!(
        owner
            .verify(
                input,
                event(SpeechLanguage::English, "synthetic source one"),
                stamp(),
                ResolverCancellation::default()
            )
            .await
            .is_ok()
    );
}
#[tokio::test]
async fn cooperative_cancel_stops_document_scan() {
    use std::cell::Cell;
    let input = fixture(
        TextLanguage::Source(SpeechLanguage::English),
        vec!["other synthetic words", "synthetic source one"],
        true,
    )
    .await;
    let calls = Cell::new(0);
    let result = match_cue(
        &input,
        &event(SpeechLanguage::English, "synthetic source one"),
        stamp(),
        || {
            calls.set(calls.get() + 1);
            if calls.get() == 2 {
                Err(ResolverError::Cancelled.into())
            } else {
                Ok(())
            }
        },
    );
    assert_eq!(
        result,
        Err(ExactAlignmentError::Invalid(ResolverError::Cancelled))
    );
    assert_eq!(calls.get(), 2);
}
#[test]
fn worker_capacity_is_process_shared_and_uses_available_cores() {
    let first = ExactCueVerifier::new(version());
    let second = ExactCueVerifier::new(version());
    assert!(Arc::ptr_eq(&first.workers, &second.workers));
    assert!(Arc::ptr_eq(
        &first.workers,
        &crate::subtitle::alignment_work::workers()
    ));
    assert_eq!(first.identity, version().identity());
}
#[test]
fn dropped_future_keeps_worker_bounded_until_cancel_checked_and_releases_permit() {
    use std::{future::Future, task::Poll};
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(async {
        let input = fixture(
            TextLanguage::Source(SpeechLanguage::English),
            vec!["synthetic source one"],
            true,
        )
        .await;
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
        let blocker = tokio::task::spawn_blocking(move || {
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        });
        entered_rx.await.unwrap();
        let mut owner = verifier();
        let pending = {
            let request = owner.verify(
                input.clone(),
                event(SpeechLanguage::English, "synthetic source one"),
                stamp(),
                ResolverCancellation::default(),
            );
            tokio::pin!(request);
            std::future::poll_fn(|cx| {
                Poll::Ready(matches!(request.as_mut().poll(cx), Poll::Pending))
            })
            .await
        };
        let capacity = owner.workers.available_permits();
        release_tx.send(()).unwrap();
        blocker.await.unwrap();
        assert!(pending);
        assert_eq!(capacity, 0);
        assert!(input.validate(identity()).is_ok());
        let permit = tokio::time::timeout(
            Duration::from_secs(1),
            owner.workers.clone().acquire_owned(),
        )
        .await
        .unwrap()
        .unwrap();
        drop(permit);
        assert!(
            owner
                .verify(
                    input,
                    event(SpeechLanguage::English, "synthetic source one"),
                    stamp(),
                    ResolverCancellation::default()
                )
                .await
                .is_ok()
        );
    });
}
#[test]
fn original_query_deadline_expires_queued_cpu_job_without_refreshing_lookup() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(async {
        let mut initial = stamp();
        initial.ended_at -= MAX_WINDOW_AGE - Duration::from_millis(300);
        let input = fixture_at(
            TextLanguage::Source(SpeechLanguage::English),
            vec!["synthetic source one"],
            true,
            initial,
        )
        .await;
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
        let blocker = tokio::task::spawn_blocking(move || {
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        });
        entered_rx.await.unwrap();
        let mut owner = verifier();
        let result = owner
            .verify(
                input.clone(),
                event(SpeechLanguage::English, "synthetic source one"),
                stamp(),
                ResolverCancellation::default(),
            )
            .await;
        release_tx.send(()).unwrap();
        blocker.await.unwrap();
        assert!(matches!(
            result,
            Err(ExactAlignmentError::Invalid(ResolverError::Stale))
        ));
        assert_eq!(input.query().stamp.ended_at, initial.ended_at);
        let _permit = tokio::time::timeout(
            Duration::from_secs(1),
            owner.workers.clone().acquire_owned(),
        )
        .await
        .unwrap()
        .unwrap();
    });
}
