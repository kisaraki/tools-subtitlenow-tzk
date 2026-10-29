//! Session-only document translation. This lifetime is independent of Live audio
//! freshness; bounded batches preserve context and restore subtitle order.
use super::{
    prepared_context::{PreparedDocumentContext, PreparedTranslationContext},
    prepared_transcript::PreparedTranscript,
    speech::{SpeechLanguage, TranscriptEvent, TranscriptKind},
    translation::{
        GlossaryEntry, PreparedContext, PreparedDraftContext, QualityPolicy, StableContext,
        TranslationCancellation, TranslationEmitter, TranslationError, TranslationPass,
        TranslationRequest, Translator,
    },
};
use crate::{
    domain::TranslationQuality,
    resolver::contracts::{
        ExistingText, MAX_CUE_BYTES, MAX_TEXT_BYTES, ResolverCancellation, SubtitleSource,
        TextLanguage,
    },
};
use futures_util::{FutureExt, StreamExt, stream::FuturesUnordered};
use std::{panic::AssertUnwindSafe, sync::Arc, time::Duration};

/// Prepared requests may wait for deliberate full-context processing. There is
/// no whole-document expiry; cancellation still bounds every owned future.
pub(crate) const PREPARED_TRANSLATION_TIMEOUT: Duration = Duration::from_secs(30);

struct CancelOnDrop(TranslationCancellation);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

/// Only the explicitly selected provider is invoked. No credentials, detached
/// tasks, file writes, playback guesses, or additional timeline are introduced.
pub(crate) async fn translate_document(
    original: &ExistingText,
    quality: TranslationQuality,
    glossary: &dyn PreparedTranslationContext,
    provider: &dyn Translator,
    cancel: &ResolverCancellation,
) -> Result<Vec<Box<str>>, TranslationError> {
    translate_input(
        PreparedTranslationInput::Existing(original),
        quality,
        glossary,
        provider,
        cancel,
    )
    .await
}

/// ASR source text remains a sealed recognized transcript. The shared engine
/// does not fabricate an existing-subtitle publication or alignment evidence.
pub(crate) async fn translate_transcript(
    original: &PreparedTranscript,
    quality: TranslationQuality,
    glossary: &dyn PreparedTranslationContext,
    provider: &dyn Translator,
    cancel: &ResolverCancellation,
) -> Result<Vec<Box<str>>, TranslationError> {
    translate_input(
        PreparedTranslationInput::Asr(original),
        quality,
        glossary,
        provider,
        cancel,
    )
    .await
}

async fn translate_input(
    original: PreparedTranslationInput<'_>,
    quality: TranslationQuality,
    glossary: &dyn PreparedTranslationContext,
    provider: &dyn Translator,
    cancel: &ResolverCancellation,
) -> Result<Vec<Box<str>>, TranslationError> {
    check_cancel(cancel)?;
    let language = original.validate()?;
    let entries = glossary.glossary().entries_for(original.identity())?;
    let context = glossary.document_context();
    if let Some(context) = context {
        context.validate_for(original.identity(), language, quality, original.len())?;
    }
    let capabilities = std::panic::catch_unwind(AssertUnwindSafe(|| provider.capabilities()))
        .map_err(|_| TranslationError::InvalidData)?;
    capabilities.validate()?;
    let settings = PassSettings {
        quality,
        language,
        concurrency: capabilities.max_concurrent,
        context: QualityPolicy::for_quality(quality).context_sentences,
    };
    let drafts = run_pass(original, settings, entries, provider, cancel, None, context).await?;
    // Standard and High review the entire first pass, including following
    // translations across batch boundaries. Fast deliberately makes one pass.
    if quality == TranslationQuality::Fast {
        check_cancel(cancel)?;
        return Ok(drafts);
    }
    run_pass(
        original,
        settings,
        entries,
        provider,
        cancel,
        Some(&drafts),
        context,
    )
    .await
}

/// Borrow only the validated input. No full document text/timing copy, fake
/// content id, or common publication owner is introduced for recognized ASR.
#[derive(Clone, Copy)]
pub(crate) enum PreparedTranslationInput<'a> {
    Existing(&'a ExistingText),
    Asr(&'a PreparedTranscript),
}
pub(crate) struct PreparedCueView<'a> {
    start_us: Option<u64>,
    duration_us: Option<u64>,
    pub(crate) text: &'a str,
}
impl<'a> PreparedTranslationInput<'a> {
    pub(crate) fn validate(self) -> Result<SpeechLanguage, TranslationError> {
        match self {
            Self::Existing(original) => {
                original
                    .validate_document(
                        original.identity,
                        SubtitleSource::ExistingSubtitle,
                        &original.content_id,
                    )
                    .map_err(|_| TranslationError::InvalidData)?;
                match original.language {
                    TextLanguage::Source(language) => Ok(language),
                    _ => Err(TranslationError::UnsupportedCapability),
                }
            }
            Self::Asr(original) => {
                original
                    .validate()
                    .map_err(|_| TranslationError::InvalidData)?;
                Ok(original.language())
            }
        }
    }
    pub(crate) fn identity(self) -> super::speech::StreamIdentity {
        match self {
            Self::Existing(original) => original.identity,
            Self::Asr(original) => original.identity(),
        }
    }
    pub(crate) fn len(self) -> usize {
        match self {
            Self::Existing(original) => original.cues.len(),
            Self::Asr(original) => original.cues().len(),
        }
    }
    pub(crate) fn cue(self, index: usize) -> PreparedCueView<'a> {
        // Callers index only validated len()/context ranges. The document is
        // immutably borrowed for the complete pass and cannot change mid-batch.
        match self {
            Self::Existing(original) => {
                let cue = &original.cues[index];
                PreparedCueView {
                    start_us: cue.start_us,
                    duration_us: cue.duration_us,
                    text: &cue.text,
                }
            }
            Self::Asr(original) => {
                let cue = &original.cues()[index];
                PreparedCueView {
                    start_us: Some(cue.start_us),
                    duration_us: Some(cue.duration_us),
                    text: &cue.text,
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
struct PassSettings {
    quality: TranslationQuality,
    language: SpeechLanguage,
    concurrency: usize,
    context: usize,
}

async fn run_pass(
    original: PreparedTranslationInput<'_>,
    settings: PassSettings,
    glossary: &[GlossaryEntry],
    provider: &dyn Translator,
    cancel: &ResolverCancellation,
    drafts: Option<&[Box<str>]>,
    context: Option<&Arc<PreparedDocumentContext>>,
) -> Result<Vec<Box<str>>, TranslationError> {
    let mut translated = Vec::with_capacity(original.len());
    let mut total_bytes = 0usize;
    for batch_start in (0..original.len()).step_by(settings.concurrency) {
        check_cancel(cancel)?;
        let batch_end = (batch_start + settings.concurrency).min(original.len());
        let mut pending = FuturesUnordered::new();
        for index in batch_start..batch_end {
            let request = make_request(
                original,
                settings,
                glossary,
                &translated,
                drafts,
                index,
                context,
            );
            pending.push(run_request(
                index,
                request,
                provider,
                cancel,
                PREPARED_TRANSLATION_TIMEOUT,
            ));
        }
        // This buffer contains at most provider.max_concurrent entries. No
        // provider completion is allowed to determine document ordering.
        let mut completed: Vec<Option<Box<str>>> = (batch_start..batch_end).map(|_| None).collect();
        while !pending.is_empty() {
            let (index, result) = tokio::select! { biased;
                _ = cancel.cancelled() => return Err(TranslationError::Cancelled),
                result = pending.next() => result.ok_or(TranslationError::InvalidData)?,
            };
            check_cancel(cancel)?;
            let text = result?;
            validate_result(&text)?;
            total_bytes = total_bytes
                .checked_add(text.len())
                .filter(|bytes| *bytes <= MAX_TEXT_BYTES)
                .ok_or(TranslationError::InvalidData)?;
            let slot = completed
                .get_mut(index - batch_start)
                .ok_or(TranslationError::InvalidData)?;
            if slot.replace(text).is_some() {
                return Err(TranslationError::InvalidData);
            }
        }
        for text in completed {
            translated.push(text.ok_or(TranslationError::InvalidData)?);
        }
        // Ready synthetic/local providers must not monopolize an executor while
        // traversing a maximum-sized document. This also observes new stops.
        tokio::task::yield_now().await;
    }
    check_cancel(cancel)?;
    Ok(translated)
}

fn make_request(
    original: PreparedTranslationInput<'_>,
    settings: PassSettings,
    glossary: &[GlossaryEntry],
    translated: &[Box<str>],
    drafts: Option<&[Box<str>]>,
    index: usize,
    context: Option<&Arc<PreparedDocumentContext>>,
) -> TranslationRequest {
    let cue = original.cue(index);
    let before = index.saturating_sub(settings.context);
    let after = (index + 1 + settings.context).min(original.len());
    let stable_start = translated.len().saturating_sub(settings.context);
    TranslationRequest {
        source: Arc::new(TranscriptEvent {
            identity: original.identity(),
            sequence: index as u64,
            utterance: index as u64 + 1,
            revision: 1,
            start_us: cue.start_us,
            duration_us: cue.duration_us,
            language: Some(settings.language),
            kind: TranscriptKind::Final,
            text: cue.text.into(),
        }),
        quality: settings.quality,
        pass: if drafts.is_some() {
            TranslationPass::Review
        } else {
            TranslationPass::Draft
        },
        previous_stable: (stable_start..translated.len())
            .map(|previous| StableContext {
                sequence: previous as u64,
                source: original.cue(previous).text.into(),
                translation: translated[previous].clone(),
            })
            .collect(),
        glossary: glossary.to_vec(),
        draft: drafts.map(|drafts| drafts[index].clone()),
        committed_prefix: "".into(),
        prepared_context: Some(PreparedContext {
            document: context.cloned(),
            previous_source: (before..index)
                .map(|previous| original.cue(previous).text.into())
                .collect(),
            following_source: (index + 1..after)
                .map(|following| original.cue(following).text.into())
                .collect(),
            neighboring_drafts: drafts
                .map(|drafts| {
                    (before..after)
                        .filter(|neighbor| *neighbor != index)
                        .map(|neighbor| PreparedDraftContext {
                            sequence: neighbor as u64,
                            source: original.cue(neighbor).text.into(),
                            translation: drafts[neighbor].clone(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
        }),
    }
}

async fn run_request(
    index: usize,
    request: TranslationRequest,
    provider: &dyn Translator,
    cancel: &ResolverCancellation,
    timeout: Duration,
) -> (usize, Result<Box<str>, TranslationError>) {
    let token = TranslationCancellation::new();
    let _cancel_on_drop = CancelOnDrop(token.clone());
    let mut output = TranslationEmitter::final_only(&request, token.clone());
    let started_at = std::time::Instant::now();
    let work = AssertUnwindSafe(async {
        provider
            .translate(&request, &mut output, token.clone())
            .await
    })
    .catch_unwind();
    let result = tokio::select! { biased;
        _ = cancel.cancelled() => Err(TranslationError::Cancelled),
        reason = token.cancelled() => Err(reason),
        result = tokio::time::timeout(timeout, work) => match result {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(TranslationError::InvalidData),
            Err(_) => Err(TranslationError::Timeout),
        },
    };
    let result = if cancel.is_cancelled() {
        Err(TranslationError::Cancelled)
    } else if let Some(reason) = token.reason() {
        Err(reason)
    } else if started_at.elapsed() >= timeout {
        // An adapter that blocks while being polled cannot evade the budget by
        // returning Ready before Tokio can poll its timer again.
        Err(TranslationError::Timeout)
    } else {
        result
    };
    (index, result)
}

fn check_cancel(cancel: &ResolverCancellation) -> Result<(), TranslationError> {
    if cancel.is_cancelled() {
        Err(TranslationError::Cancelled)
    } else {
        Ok(())
    }
}

fn validate_result(text: &str) -> Result<(), TranslationError> {
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

#[cfg(test)]
#[path = "prepared_translation_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "prepared_translation_asr_tests.rs"]
mod asr_tests;
