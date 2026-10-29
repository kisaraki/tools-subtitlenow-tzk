use super::*;
use crate::audio::capture::CaptureError;

#[test]
fn phase11_safe_connection_diagnostics_never_change_the_shared_recovery_budget() {
    use crate::ai::speech::{ConnectionStage, RemoteStatusKind};
    for (cause, retryable) in [
        (SpeechError::Network, true),
        (SpeechError::ServiceInternal, true),
        (SpeechError::Timeout, true),
        (SpeechError::RateLimited, true),
        (SpeechError::Credential, false),
        (SpeechError::InvalidData, false),
        (SpeechError::SecureTransport, false),
    ] {
        let error = Err(AppError::Speech(SpeechError::Connection {
            stage: ConnectionStage::SetupReceive,
            cause: Box::new(SpeechError::RemoteStatus {
                kind: RemoteStatusKind::WebSocket,
                code: 4000,
                cause: Box::new(cause),
            }),
        }));
        let mut recovery = Recovery::new(0);
        for _ in 0..3 {
            assert_eq!(
                matches!(
                    recovery.decide(&error, Duration::ZERO),
                    RecoveryAction::Retry(_)
                ),
                retryable
            );
        }
        assert_eq!(
            recovery.decide(&error, Duration::ZERO),
            RecoveryAction::Fail
        );
        assert_eq!(recovery.attempts(), if retryable { 3 } else { 0 });
    }
}

#[test]
fn phase11_audio_diagnostics_keep_the_same_three_retry_budget_and_delays() {
    use crate::ai::speech::{AudioIssue, AudioStage};
    for stage in [
        AudioStage::Input,
        AudioStage::Receive,
        AudioStage::Preprocess,
        AudioStage::Protocol,
        AudioStage::SendQueue,
        AudioStage::AwaitingFinal,
    ] {
        let plain = Err(AppError::Speech(SpeechError::AudioDiscontinuity));
        let detailed = Err(AppError::Speech(SpeechError::audio_failure(
            stage,
            AudioIssue::Age,
        )));
        let mut original = Recovery::new(42);
        let mut diagnostic = Recovery::new(42);
        for _ in 0..4 {
            assert_eq!(
                original.decide(&plain, Duration::ZERO),
                diagnostic.decide(&detailed, Duration::ZERO)
            );
        }
        assert_eq!(diagnostic.attempts(), 3);
    }
}

#[test]
fn phase11_named_queue_faults_share_the_original_overload_retry_budget() {
    use crate::ai::speech::SpeechQueue;
    for queue in [
        SpeechQueue::NativeCaptionOutput,
        SpeechQueue::AudioInput,
        SpeechQueue::TranscriptOutput,
        SpeechQueue::PreprocessInput,
        SpeechQueue::PreprocessOutput,
        SpeechQueue::SendAudio,
        SpeechQueue::CurrentTurn,
        SpeechQueue::PendingFinal,
        SpeechQueue::LanguagePartials,
    ] {
        let failure = SpeechError::queue_full(queue, 12);
        assert_eq!(failure.root_cause(), &SpeechError::Overloaded);
        assert!(failure.to_string().contains("上限 12 項"));
        let mut plain = Recovery::new(42);
        let mut detailed = Recovery::new(42);
        for _ in 0..4 {
            assert_eq!(
                plain.decide(
                    &Err(AppError::Speech(SpeechError::Overloaded)),
                    Duration::ZERO
                ),
                detailed.decide(&Err(AppError::Speech(failure.clone())), Duration::ZERO)
            );
        }
        assert_eq!(detailed.attempts(), 3);
    }
}

#[test]
fn phase11_alternating_speech_and_translation_faults_share_one_finite_budget() {
    let mut recovery = Recovery::new(42);
    let mut delays = Vec::new();
    for error in [
        AppError::Speech(SpeechError::Network),
        AppError::Translation(TranslationError::Timeout),
        AppError::Speech(SpeechError::RateLimited),
    ] {
        let RecoveryAction::Retry(delay) = recovery.decide(&Err(error), Duration::from_secs(1))
        else {
            panic!("transient fault must use its remaining retry");
        };
        delays.push(delay);
    }
    assert!(delays.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(recovery.attempts(), 3);
    assert_eq!(
        recovery.decide(
            &Err(AppError::Translation(TranslationError::Network)),
            Duration::from_secs(1)
        ),
        RecoveryAction::Fail
    );
}
#[test]
fn phase11_transient_fault_classes_preserve_the_existing_recovery_policy() {
    for error in [
        AppError::Speech(SpeechError::GenerationInterrupted),
        AppError::Speech(SpeechError::Network),
        AppError::Speech(SpeechError::Timeout),
        AppError::Speech(SpeechError::RateLimited),
        AppError::Speech(SpeechError::AudioDiscontinuity),
        AppError::Speech(SpeechError::Overloaded),
        AppError::Translation(TranslationError::Network),
        AppError::Translation(TranslationError::RateLimited),
        AppError::Translation(TranslationError::Timeout),
        AppError::Translation(TranslationError::Stale),
        AppError::Translation(TranslationError::Overloaded),
    ] {
        assert!(matches!(
            Recovery::new(0).decide(&Err(error), Duration::ZERO),
            RecoveryAction::Retry(_)
        ));
    }
}
#[test]
fn phase11_credentials_tls_source_exit_and_device_failure_are_never_silently_retried() {
    for error in [
        AppError::Speech(SpeechError::Credential),
        AppError::Speech(SpeechError::SecureTransport),
        AppError::Speech(SpeechError::InvalidData),
        AppError::Speech(SpeechError::UnsupportedLanguage),
        AppError::Speech(SpeechError::LanguageNotReported),
        AppError::Translation(TranslationError::Credential),
        AppError::Translation(TranslationError::Incomplete),
        AppError::Translation(TranslationError::Refused),
        AppError::Capture(CaptureError::Disconnected),
        AppError::Capture(CaptureError::SourceExited),
    ] {
        let mut recovery = Recovery::new(0);
        assert_eq!(
            recovery.decide(&Err(error), Duration::ZERO),
            RecoveryAction::Fail
        );
        assert_eq!(recovery.attempts(), 0);
    }
}
#[test]
fn phase11_healthy_run_resets_the_budget_and_planned_rotation_does_not_spin() {
    let mut recovery = Recovery::new(1);
    for _ in 0..3 {
        recovery.decide(&Err(AppError::Speech(SpeechError::Timeout)), Duration::ZERO);
    }
    assert_eq!(recovery.attempts(), 3);
    assert!(matches!(
        recovery.decide(&Err(AppError::Speech(SpeechError::Network)), STABLE_RESET),
        RecoveryAction::Retry(_)
    ));
    assert_eq!(recovery.attempts(), 1);
    assert_eq!(
        recovery.decide(
            &Err(AppError::Speech(SpeechError::SessionExpired)),
            STABLE_RESET
        ),
        RecoveryAction::Rotate
    );
    assert_eq!(recovery.attempts(), 0);
}
#[test]
fn phase11_repeated_early_rotation_exhausts_the_budget() {
    let mut recovery = Recovery::new(0);
    for _ in 0..3 {
        assert!(matches!(
            recovery.decide(
                &Err(AppError::Speech(SpeechError::SessionExpired)),
                STABLE_RESET - Duration::from_nanos(1)
            ),
            RecoveryAction::Retry(_)
        ));
    }
    assert_eq!(
        recovery.decide(
            &Err(AppError::Speech(SpeechError::SessionExpired)),
            Duration::ZERO
        ),
        RecoveryAction::Fail
    );
}
#[test]
fn phase11_user_stop_and_closed_view_do_not_schedule_reconnection() {
    for result in [
        Ok(()),
        Err(AppError::Speech(SpeechError::Cancelled)),
        Err(AppError::UiNotification(UiBridgeError::Closed)),
    ] {
        let mut recovery = Recovery::new(0);
        assert_eq!(
            recovery.decide(&result, Duration::ZERO),
            RecoveryAction::Stop
        );
        assert_eq!(recovery.attempts(), 0);
    }
}
#[test]
fn phase11_retry_jitter_remains_bounded_at_pid_extremes() {
    for seed in [0, 42, u32::MAX] {
        for (attempt, base) in [(0, 500), (1, 1000), (2, 2000)] {
            let delay = retry_delay(attempt, seed);
            assert!(delay >= Duration::from_millis(base));
            assert!(delay <= Duration::from_millis(base + 250));
        }
    }
}
