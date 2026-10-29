//! Session-only translation of recognized source text. It retains ASR
//! provenance and genuine media time, never a subtitle publication or Ready.
use super::{
    prepared_context::PreparedTranslationContext,
    prepared_transcript::PreparedTranscript,
    prepared_translation::translate_transcript,
    speech::StreamIdentity,
    translation::{TranslationError, Translator},
};
use crate::{
    domain::{TARGET_LANGUAGE, TranslationQuality},
    resolver::contracts::{MAX_CUE_BYTES, MAX_TEXT_BYTES, ResolverCancellation},
    subtitle::localization::taiwan_phrase_conversion,
};
use std::{future::Future, sync::Arc};
use tokio::{sync::Semaphore, task::JoinHandle};

/// Original and translated cues share one order and immutable media time.
/// No Clone, Debug, serialization, content export or playback binding exists.
pub struct PreparedTranslatedTranscript {
    original: PreparedTranscript,
    translated: Box<[Box<str>]>,
    quality: TranslationQuality,
}
impl PreparedTranslatedTranscript {
    pub const fn identity(&self) -> StreamIdentity {
        self.original.identity()
    }
    pub const fn original(&self) -> &PreparedTranscript {
        &self.original
    }
    pub fn translated(&self) -> &[Box<str>] {
        &self.translated
    }
    pub const fn quality(&self) -> TranslationQuality {
        self.quality
    }
    pub const fn target_language(&self) -> &'static str {
        TARGET_LANGUAGE
    }
}
struct CancelOnDrop {
    cancel: ResolverCancellation,
    armed: bool,
}
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if self.armed {
            self.cancel.cancel();
        }
    }
}
struct AbortOnDrop<T>(JoinHandle<T>);
impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Caller selects a translator and same-session glossary explicitly. Nothing
/// starts until polled; even an unpolled Drop cancels and releases owned text.
/// Provider waits hold no CPU/native permit. Final Taiwan localization runs on
/// the shared bounded CPU lane and only a complete validated document succeeds.
pub fn translate_recognized_transcript<'a>(
    original: PreparedTranscript,
    quality: TranslationQuality,
    glossary: &'a dyn PreparedTranslationContext,
    provider: &'a dyn Translator,
    cancel: ResolverCancellation,
) -> impl Future<Output = Result<PreparedTranslatedTranscript, TranslationError>> + Send + 'a {
    translate_with_lane(
        original,
        quality,
        glossary,
        provider,
        cancel,
        crate::media::work::workers(),
    )
}
fn translate_with_lane<'a>(
    original: PreparedTranscript,
    quality: TranslationQuality,
    glossary: &'a dyn PreparedTranslationContext,
    provider: &'a dyn Translator,
    cancel: ResolverCancellation,
    cpu: Arc<Semaphore>,
) -> impl Future<Output = Result<PreparedTranslatedTranscript, TranslationError>> + Send + 'a {
    let guard = CancelOnDrop {
        cancel: cancel.clone(),
        armed: true,
    };
    async move {
        let mut guard = guard;
        let result = run(original, quality, glossary, provider, &cancel, cpu).await;
        if result.is_ok() {
            guard.armed = false;
        }
        result
    }
}
async fn run(
    original: PreparedTranscript,
    quality: TranslationQuality,
    glossary: &dyn PreparedTranslationContext,
    provider: &dyn Translator,
    cancel: &ResolverCancellation,
    cpu: Arc<Semaphore>,
) -> Result<PreparedTranslatedTranscript, TranslationError> {
    check(cancel)?;
    let translated = translate_transcript(&original, quality, glossary, provider, cancel).await?;
    check(cancel)?;
    finish_on_worker(cancel, cpu, move |scope| {
        finish(original, translated, quality, scope)
    })
    .await
}
fn finish_on_worker<'a, F>(
    cancel: &'a ResolverCancellation,
    cpu: Arc<Semaphore>,
    operation: F,
) -> impl Future<Output = Result<PreparedTranslatedTranscript, TranslationError>> + Send + 'a
where
    F: FnOnce(&ResolverCancellation) -> Result<PreparedTranslatedTranscript, TranslationError>
        + Send
        + 'static,
{
    let guard = CancelOnDrop {
        cancel: cancel.clone(),
        armed: true,
    };
    async move {
        let mut guard = guard;
        check(cancel)?;
        let permit = cpu
            .try_acquire_owned()
            .map_err(|_| TranslationError::Overloaded)?;
        let runtime =
            tokio::runtime::Handle::try_current().map_err(|_| TranslationError::InvalidData)?;
        let worker_cancel = cancel.clone();
        let mut worker = AbortOnDrop(runtime.spawn_blocking(move || {
            // A started worker owns text and capacity through real cleanup, even
            // when the caller stops polling; abort never implies actual exit.
            let _permit = permit;
            operation(&worker_cancel)
        }));
        let result = tokio::select! { biased;
            _ = cancel.cancelled() => return Err(TranslationError::Cancelled),
            result = &mut worker.0 => result.map_err(|_| TranslationError::InvalidData)?,
        };
        check(cancel)?;
        if result.is_ok() {
            guard.armed = false;
        }
        result
    }
}
fn finish(
    original: PreparedTranscript,
    translated: Vec<Box<str>>,
    quality: TranslationQuality,
    cancel: &ResolverCancellation,
) -> Result<PreparedTranslatedTranscript, TranslationError> {
    check(cancel)?;
    original
        .validate()
        .map_err(|_| TranslationError::InvalidData)?;
    if translated.len() != original.cues().len() {
        return Err(TranslationError::InvalidData);
    }
    let mut localized = Vec::with_capacity(translated.len());
    let mut raw_bytes = 0usize;
    let mut localized_bytes = 0usize;
    for text in translated {
        check(cancel)?;
        validate_text(&text)?;
        raw_bytes = raw_bytes
            .checked_add(text.len())
            .filter(|bytes| *bytes <= MAX_TEXT_BYTES)
            .ok_or(TranslationError::InvalidData)?;
        let text = taiwan_phrase_conversion(&text)?;
        validate_text(&text)?;
        localized_bytes = localized_bytes
            .checked_add(text.len())
            .filter(|bytes| *bytes <= MAX_TEXT_BYTES)
            .ok_or(TranslationError::InvalidData)?;
        localized.push(text.into_boxed_str());
    }
    check(cancel)?;
    Ok(PreparedTranslatedTranscript {
        original,
        translated: localized.into_boxed_slice(),
        quality,
    })
}
fn validate_text(text: &str) -> Result<(), TranslationError> {
    if text.trim().is_empty()
        || text.len() > MAX_CUE_BYTES
        || text
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        Err(TranslationError::InvalidData)
    } else {
        Ok(())
    }
}
fn check(cancel: &ResolverCancellation) -> Result<(), TranslationError> {
    if cancel.is_cancelled() {
        Err(TranslationError::Cancelled)
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "prepared_transcript_translation_tests.rs"]
mod tests;
