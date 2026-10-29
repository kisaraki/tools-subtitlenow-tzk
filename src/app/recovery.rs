//! One bounded recovery budget shared by speech and translation. No provider,
//! source selection, network, clock reads or retained content belongs here.
use crate::{
    ai::{speech::SpeechError, translation::TranslationError},
    error::{AppError, Result},
    ui::bridge::UiBridgeError,
};
use std::time::Duration;

const MAX_RETRIES: u32 = 3;
const RETRY_BASE_MS: u64 = 500;
pub(super) const STABLE_RESET: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RecoveryAction {
    Stop,
    Rotate,
    Retry(Duration),
    Fail,
}
pub(super) struct Recovery {
    attempts: u32,
    seed: u32,
}
impl Recovery {
    pub fn new(seed: u32) -> Self {
        Self { attempts: 0, seed }
    }
    pub fn attempts(&self) -> u32 {
        self.attempts
    }
    /// Called only after the old capture, provider jobs and socket are stopped.
    /// Changing failure stage never grants a fresh retry budget.
    pub fn decide(&mut self, result: &Result<()>, running_for: Duration) -> RecoveryAction {
        if running_for >= STABLE_RESET {
            self.attempts = 0;
        }
        match result {
            Ok(())
            | Err(AppError::Speech(SpeechError::Cancelled))
            | Err(AppError::UiNotification(UiBridgeError::Closed)) => return RecoveryAction::Stop,
            Err(AppError::Speech(SpeechError::SessionExpired)) if running_for >= STABLE_RESET => {
                return RecoveryAction::Rotate;
            }
            _ => {}
        }
        let retryable = match result {
            Err(AppError::Speech(error)) => {
                error.retryable()
                    || matches!(
                        error.root_cause(),
                        SpeechError::Overloaded
                            | SpeechError::AudioDiscontinuity
                            | SpeechError::SessionExpired
                    )
            }
            Err(AppError::Translation(error)) => matches!(
                error,
                TranslationError::Network
                    | TranslationError::RateLimited
                    | TranslationError::Timeout
                    | TranslationError::Stale
                    | TranslationError::Overloaded
            ),
            _ => false,
        };
        if !retryable || self.attempts >= MAX_RETRIES {
            return RecoveryAction::Fail;
        }
        let delay = retry_delay(self.attempts, self.seed);
        self.attempts += 1;
        RecoveryAction::Retry(delay)
    }
}
fn retry_delay(attempt: u32, seed: u32) -> Duration {
    // Existing bounded jitter, independent of secrets/content. At most 3 retries.
    Duration::from_millis(
        RETRY_BASE_MS * (1u64 << attempt.min(MAX_RETRIES))
            + (u64::from(seed) * 31 + u64::from(attempt) * 97) % 251,
    )
}

#[cfg(test)]
#[path = "recovery_tests.rs"]
mod tests;
