//! Explicit development mocks only; no cloud requests, files, or artificial production text.
use super::*;
use crate::ai::provider::SpeechProvider;
use crate::{
    audio::{
        capture::{AudioStamp, BLOCK_FRAMES, SessionId},
        preprocess::NormalizedAudio,
    },
    domain::SourceLanguage,
};
use std::time::{Duration, Instant};

fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(700),
        generation: 2,
    }
}
fn capabilities() -> SpeechCapabilities {
    SpeechCapabilities {
        streaming: true,
        partials: true,
        cancellation: true,
        supported_languages: [true; 5],
        constrained_auto_detection: true,
    }
}
fn audio(sequence: u64) -> NormalizedAudio {
    NormalizedAudio {
        stamp: AudioStamp {
            session: identity().session,
            sequence,
            frame_position: Some(sequence * BLOCK_FRAMES as u64),
            qpc_100ns: None,
            received: Instant::now(),
            discontinuity: false,
        },
        frames: BLOCK_FRAMES,
        samples: [0.0; BLOCK_FRAMES],
    }
}
fn event(utterance: u64, revision: u64, kind: TranscriptKind, text: &str) -> TranscriptEvent {
    TranscriptEvent {
        identity: identity(),
        utterance,
        revision,
        sequence: utterance,
        start_us: Some(utterance * 100_000),
        duration_us: Some(100_000),
        language: Some(SpeechLanguage::English),
        kind,
        text: text.into(),
    }
}
struct MockSpeechRecognizer;
impl SpeechRecognizer for MockSpeechRecognizer {
    fn capabilities(&self) -> SpeechCapabilities {
        capabilities()
    }
    fn run<'a>(
        &'a self,
        _: SourceLanguage,
        mut audio: SpeechAudio,
        output: TranscriptOutput,
        cancellation: Cancellation,
    ) -> SpeechFuture<'a> {
        Box::pin(async move {
            audio.recv().await?;
            output.emit(event(1, 1, TranscriptKind::Partial, "Test hypothesis"))?;
            output.emit(event(1, 2, TranscriptKind::Final, "Test final"))?;
            Err(cancellation.cancelled().await)
        })
    }
}

#[tokio::test]
async fn provider_failure_clears_queued_events_and_keeps_safe_error() {
    struct FailedProvider;
    impl SpeechRecognizer for FailedProvider {
        fn capabilities(&self) -> SpeechCapabilities {
            capabilities()
        }
        fn run<'a>(
            &'a self,
            _: SourceLanguage,
            _: SpeechAudio,
            output: TranscriptOutput,
            _: Cancellation,
        ) -> SpeechFuture<'a> {
            Box::pin(async move {
                output.emit(event(1, 1, TranscriptKind::Partial, "private test text"))?;
                Err(SpeechError::Credential)
            })
        }
    }
    let (mut session, task) =
        SpeechSession::new(identity(), SourceLanguage::English, capabilities()).unwrap();
    assert_eq!(
        task.run(&FailedProvider).await,
        Err(SpeechError::Credential)
    );
    assert_eq!(
        session.cancellation().reason(),
        Some(SpeechError::Credential)
    );
    assert!(session.next().await.is_none());
    assert!(
        !SpeechError::Credential
            .to_string()
            .contains("private test text")
    );
}

#[tokio::test]
async fn mock_stream_yields_partial_final_and_joins_on_stop() {
    let (mut session, task) =
        SpeechSession::new(identity(), SourceLanguage::English, capabilities()).unwrap();
    session.input.try_push(audio(0)).unwrap();
    let cancellation = session.cancellation();
    let consumer = async {
        assert_eq!(session.next().await.unwrap().kind, TranscriptKind::Partial);
        assert_eq!(session.next().await.unwrap().kind, TranscriptKind::Final);
        cancellation.cancel();
        assert!(session.next().await.is_none());
    };
    let (result, ()) = tokio::join!(task.run(&MockSpeechRecognizer), consumer);
    assert_eq!(result, Err(SpeechError::Cancelled));
}

#[tokio::test]
async fn pre_cancelled_task_never_polls_provider() {
    struct NeverStarted;
    impl SpeechRecognizer for NeverStarted {
        fn capabilities(&self) -> SpeechCapabilities {
            capabilities()
        }
        fn run<'a>(
            &'a self,
            _: SourceLanguage,
            _: SpeechAudio,
            _: TranscriptOutput,
            _: Cancellation,
        ) -> SpeechFuture<'a> {
            panic!("provider must not start")
        }
    }
    let (session, task) =
        SpeechSession::new(identity(), SourceLanguage::Auto, capabilities()).unwrap();
    session.cancellation().cancel();
    assert_eq!(task.run(&NeverStarted).await, Err(SpeechError::Cancelled));
}

#[tokio::test]
async fn cancellation_drops_pending_provider_future_without_orphans() {
    struct PendingProvider;
    impl SpeechRecognizer for PendingProvider {
        fn capabilities(&self) -> SpeechCapabilities {
            capabilities()
        }
        fn run<'a>(
            &'a self,
            _: SourceLanguage,
            _: SpeechAudio,
            _: TranscriptOutput,
            _: Cancellation,
        ) -> SpeechFuture<'a> {
            Box::pin(std::future::pending())
        }
    }
    let (session, task) =
        SpeechSession::new(identity(), SourceLanguage::Auto, capabilities()).unwrap();
    let cancellation = session.cancellation();
    let stop = async {
        tokio::task::yield_now().await;
        cancellation.cancel();
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(1), async {
        tokio::join!(task.run(&PendingProvider), stop)
    })
    .await
    .unwrap();
    assert_eq!(result, Err(SpeechError::Cancelled));
}

#[test]
fn audio_saturation_is_bounded_and_requires_explicit_restart() {
    let (session, task) =
        SpeechSession::new(identity(), SourceLanguage::Auto, capabilities()).unwrap();
    for sequence in 0..AUDIO_QUEUE_BLOCKS {
        session.input.try_push(audio(sequence as u64)).unwrap();
    }
    assert_eq!(
        session.input.try_push(audio(12)),
        Err(SpeechError::queue_full(
            SpeechQueue::AudioInput,
            AUDIO_QUEUE_BLOCKS
        ))
    );
    session.cancellation().cancel();
    assert_eq!(
        session.cancellation().reason(),
        Some(SpeechError::queue_full(
            SpeechQueue::AudioInput,
            AUDIO_QUEUE_BLOCKS
        ))
    );
    drop(task);
}

#[tokio::test]
async fn input_rejects_foreign_malformed_stale_and_discontinuous_audio() {
    let (session, mut task) =
        SpeechSession::new(identity(), SourceLanguage::Auto, capabilities()).unwrap();
    let mut foreign = audio(0);
    foreign.stamp.session = SessionId(699);
    assert_eq!(
        session.input.try_push(foreign),
        Err(SpeechError::InvalidData)
    );
    let mut malformed = audio(0);
    malformed.frames = BLOCK_FRAMES + 1;
    assert_eq!(
        session.input.try_push(malformed),
        Err(SpeechError::InvalidData)
    );
    let mut malformed = audio(0);
    malformed.samples[0] = f32::NAN;
    assert_eq!(
        session.input.try_push(malformed),
        Err(SpeechError::InvalidData)
    );
    session.input.try_push(audio(0)).unwrap();
    task.audio.as_mut().unwrap().recv().await.unwrap();
    session.input.try_push(audio(2)).unwrap();
    assert!(matches!(
        task.audio.as_mut().unwrap().recv().await.err(),
        Some(SpeechError::AudioFailure {
            stage: AudioStage::Receive,
            issue: AudioIssue::SequenceGap,
            ..
        })
    ));
    // Keep the receiver alive while verifying age validation.
    let (fresh, _task) =
        SpeechSession::new(identity(), SourceLanguage::Auto, capabilities()).unwrap();
    let mut stale = audio(0);
    stale.stamp.received -= Duration::from_secs(1);
    assert_eq!(
        fresh.input.try_push(stale),
        Err(SpeechError::audio_failure(
            AudioStage::Input,
            AudioIssue::Age
        ))
    );
}

#[tokio::test]
async fn queued_audio_is_age_checked_at_consumer() {
    let (session, mut task) =
        SpeechSession::new(identity(), SourceLanguage::Auto, capabilities()).unwrap();
    session.input.try_push(audio(0)).unwrap();
    tokio::time::sleep(Duration::from_millis(280)).await;
    assert!(matches!(
        task.audio.as_mut().unwrap().recv().await.err(),
        Some(SpeechError::AudioFailure {
            stage: AudioStage::Receive,
            issue: AudioIssue::Age,
            ..
        })
    ));
}

#[tokio::test]
async fn phase11_audio_failure_preserves_exact_boundary_and_first_cancellation_reason() {
    for issue in [
        AudioIssue::DiscontinuityFlag,
        AudioIssue::SequenceGap,
        AudioIssue::PositionGap,
        AudioIssue::TimestampUnavailable,
        AudioIssue::Age,
    ] {
        let (session, mut task) =
            SpeechSession::new(identity(), SourceLanguage::English, capabilities()).unwrap();
        // An initial native discontinuity is accepted as before; a later one is not.
        let mut first = audio(0);
        first.stamp.discontinuity = true;
        session.input.try_push(first).unwrap();
        task.audio.as_mut().unwrap().recv().await.unwrap();
        let mut next = audio(1);
        match issue {
            AudioIssue::DiscontinuityFlag => next.stamp.discontinuity = true,
            AudioIssue::SequenceGap => next.stamp.sequence += 1,
            AudioIssue::PositionGap => next.stamp.frame_position = Some(17),
            AudioIssue::TimestampUnavailable => {
                next.stamp.frame_position = None;
                next.stamp.discontinuity = true;
            }
            // Future local timestamps were already rejected by the age check.
            AudioIssue::Age => next.stamp.received += Duration::from_secs(1),
            _ => unreachable!(),
        }
        if issue == AudioIssue::Age {
            // Input has the same clock check and rejects before the consumer.
            assert_eq!(
                session.input.try_push(next),
                Err(SpeechError::audio_failure(AudioStage::Input, issue))
            );
            assert_eq!(
                session.cancellation().reason(),
                Some(SpeechError::audio_failure(AudioStage::Input, issue))
            );
        } else {
            session.input.try_push(next).unwrap();
            let expected = task.audio.as_mut().unwrap().recv().await.err().unwrap();
            assert!(matches!(&expected, SpeechError::AudioFailure {
                stage: AudioStage::Receive, issue: actual, ..
            } if *actual == issue));
            session.cancellation().cancel();
            assert_eq!(session.cancellation().reason(), Some(expected.clone()));
            assert_eq!(session.input.try_push(audio(2)), Err(expected.clone()));
            assert_eq!(expected.root_cause(), &SpeechError::AudioDiscontinuity);
            assert!(!expected.retryable());
            assert!(expected.to_string().contains("辨識端取出音訊"));
        }
    }
}

#[tokio::test]
async fn phase11_capture_flag_does_not_hide_numeric_queue_or_position_gaps() {
    for sequence_gap in [true, false] {
        let (session, mut task) =
            SpeechSession::new(identity(), SourceLanguage::English, capabilities()).unwrap();
        let mut first = audio(0);
        first.stamp.qpc_100ns = Some(1_000_000);
        session.input.try_push(first).unwrap();
        task.audio.as_mut().unwrap().recv().await.unwrap();
        let mut next = audio(1);
        next.stamp.discontinuity = true;
        next.stamp.qpc_100ns = Some(1_400_000);
        let issue = if sequence_gap {
            next.stamp.sequence = 2;
            AudioIssue::SequenceGap
        } else {
            next.stamp.frame_position = Some(1920);
            AudioIssue::PositionGap
        };
        session.input.try_push(next).unwrap();
        let error = task.audio.as_mut().unwrap().recv().await.err().unwrap();
        let SpeechError::AudioFailure {
            stage,
            issue: actual,
            detail,
            ..
        } = &error
        else {
            panic!("wrong error: {error:?}");
        };
        assert_eq!(*stage, AudioStage::Receive);
        assert_eq!(*actual, issue);
        assert_eq!(
            detail,
            &format!(
                "（序號 0→{}；前包 960 frames；位置差 {} frames；QPC 間隔 400000×100 ns；標記 1）",
                if sequence_gap { 2 } else { 1 },
                if sequence_gap { 0 } else { 960 }
            )
        );
        assert_eq!(session.cancellation().reason(), Some(error));
    }
}

#[test]
fn events_reject_previous_generation_language_and_excessive_text() {
    let (_session, task) =
        SpeechSession::new(identity(), SourceLanguage::English, capabilities()).unwrap();
    let output = task.output.as_ref().unwrap();
    let mut old = event(1, 1, TranscriptKind::Partial, "old");
    old.identity.generation -= 1;
    assert_eq!(output.emit(old), Err(SpeechError::InvalidData));
    let mut wrong = event(1, 1, TranscriptKind::Partial, "wrong");
    wrong.language = Some(SpeechLanguage::Japanese);
    assert_eq!(output.emit(wrong), Err(SpeechError::UnsupportedLanguage));
    assert_eq!(
        output.emit(event(
            1,
            1,
            TranscriptKind::Partial,
            &"x".repeat(MAX_TRANSCRIPT_BYTES + 1)
        )),
        Err(SpeechError::InvalidData)
    );
}

#[tokio::test]
async fn slow_event_consumer_cancels_instead_of_losing_finals() {
    let (mut session, task) =
        SpeechSession::new(identity(), SourceLanguage::English, capabilities()).unwrap();
    for index in 0..EVENT_QUEUE_ITEMS {
        task.output
            .as_ref()
            .unwrap()
            .emit(event(index as u64, 1, TranscriptKind::Final, "test"))
            .unwrap();
    }
    assert_eq!(
        task.output
            .as_ref()
            .unwrap()
            .emit(event(65, 1, TranscriptKind::Final, "overflow")),
        Err(SpeechError::queue_full(
            SpeechQueue::TranscriptOutput,
            EVENT_QUEUE_ITEMS
        ))
    );
    assert!(session.next().await.is_none());
}

#[test]
fn revision_gate_orders_utterances_allows_rollback_and_keeps_final_terminal() {
    let mut ledger = TranscriptLedger::new(identity());
    assert!(ledger.apply(event(2, 1, TranscriptKind::Final, "second")));
    assert!(ledger.apply(event(1, 1, TranscriptKind::Partial, "tentative tail")));
    assert!(ledger.apply(event(1, 2, TranscriptKind::Partial, "tentative")));
    assert!(!ledger.apply(event(1, 1, TranscriptKind::Partial, "late")));
    assert!(ledger.apply(event(1, 3, TranscriptKind::Final, "first")));
    assert!(!ledger.apply(event(
        1,
        4,
        TranscriptKind::Partial,
        "must not replace final"
    )));
    assert_eq!(
        ledger
            .entries()
            .iter()
            .map(|entry| entry.text.as_ref())
            .collect::<Vec<_>>(),
        ["first", "second"]
    );
    let mut old = event(3, 1, TranscriptKind::Final, "old session");
    old.identity.session = SessionId(699);
    assert!(!ledger.apply(old));
}

#[test]
fn bounded_ledger_does_not_resurrect_retired_utterances() {
    let mut ledger = TranscriptLedger::new(identity());
    for index in 0..100 {
        ledger.apply(event(index, 1, TranscriptKind::Final, "test"));
    }
    assert_eq!(ledger.entries().len(), EVENT_QUEUE_ITEMS);
    assert!(!ledger.apply(event(0, 9, TranscriptKind::Partial, "old")));
}

#[test]
fn five_language_capability_gate_and_provider_plans_are_explicit() {
    assert_eq!(SpeechLanguage::from_code("zh-CN"), None);
    assert_eq!(SpeechLanguage::from_code("es"), None);
    let mut limited = capabilities();
    limited.constrained_auto_detection = false;
    assert_eq!(
        limited.validate(SourceLanguage::Auto),
        Err(SpeechError::UnsupportedLanguage)
    );
    limited.supported_languages[SpeechLanguage::Korean as usize] = false;
    assert_eq!(
        limited.validate(SourceLanguage::Korean),
        Err(SpeechError::UnsupportedLanguage)
    );
    for provider in [SpeechProvider::OpenAi, SpeechProvider::Gemini] {
        let plan = provider.plan(SourceLanguage::Auto);
        let codes = match provider {
            SpeechProvider::OpenAi => {
                &plan.setup["session"]["audio"]["input"]["transcription"]["languages"]
            }
            SpeechProvider::Gemini => {
                &plan.setup["setup"]["inputAudioTranscription"]["languageCodes"]
            }
        };
        assert_eq!(
            codes.as_array().unwrap().len(),
            if provider == SpeechProvider::Gemini {
                0
            } else {
                5
            }
        );
        if provider == SpeechProvider::Gemini {
            assert_eq!(
                plan.setup["setup"]["realtimeInputConfig"]["automaticActivityDetection"]["disabled"],
                false
            );
        }
        for code in codes.as_array().unwrap() {
            assert!(SpeechLanguage::from_code(code.as_str().unwrap()).is_some());
        }
        let single = provider.plan(SourceLanguage::Japanese);
        assert!(
            single
                .setup
                .to_string()
                .contains(if provider == SpeechProvider::Gemini {
                    "ja-JP"
                } else {
                    "\"ja\""
                })
        );
    }
    assert!(SpeechError::RateLimited.retryable());
    assert!(!SpeechError::Credential.retryable());
}

#[test]
fn phase11_regular_language_tags_keep_the_five_language_boundary() {
    for code in [
        "en",
        "EN",
        "en-GB",
        "en-Latn-US",
        "en-001",
        "de-1901",
        "fr-CA",
        "ja-Jpan-JP",
        "ko-Kore-KR",
    ] {
        assert!(SpeechLanguage::from_code(code).is_some(), "{code}");
    }
    for code in [
        "",
        "en-",
        "en--US",
        "en_US",
        " en",
        "en ",
        "english",
        "en-usa",
        "en-US-extra!",
        "de-1901-1901",
        "en-US-Latn",
        "es",
        "es-ES",
        "zh-Hant",
        "en-中文",
        "en-US-en-US",
    ] {
        assert_eq!(SpeechLanguage::from_code(code), None, "{code}");
    }
    assert_eq!(
        SpeechLanguage::from_code(&format!("en-{}", "a".repeat(65))),
        None
    );
}
