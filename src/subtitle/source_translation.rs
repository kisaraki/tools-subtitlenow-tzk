//! Bounded, caller-selected source subtitle windows -> zh-TW candidates.
//! No alignment guess, provider selection, renderer, extra timeline or persistence.
use super::existing_text::{
    ChineseTextLocalizer, LocalizedText, TextLocalizationError, validate_budget,
};
use crate::{
    ai::{
        speech::{
            MAX_TRANSCRIPT_BYTES, SpeechLanguage, StreamIdentity, TranscriptEvent, TranscriptKind,
        },
        translation::{
            MAX_TRANSLATION_AGE, QualityPolicy, SessionGlossary, StableContext,
            TranslationCancellation, TranslationEmitter, TranslationError, TranslationPass,
            TranslationRequest, Translator,
        },
    },
    domain::TranslationQuality,
    resolver::contracts::{
        ExistingText, MAX_WINDOW_AGE, ResolverCancellation, ResolverError, SubtitleSource, TextCue,
        TextLanguage, TextQuery, WindowStamp,
    },
};
use futures_util::FutureExt;
use std::{collections::VecDeque, ops::Range, panic::AssertUnwindSafe, sync::Arc, time::Instant};

/// A Live candidate window, not whole-document/Prepared processing. Reject larger
/// ranges explicitly; never silently truncate or assume playback starts at zero.
pub const MAX_TRANSLATION_WINDOW_CUES: usize = 4;
const MAX_CONTEXT_CUES: usize =
    QualityPolicy::for_quality(TranslationQuality::High).context_sentences;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SourceTranslationError {
    #[error("{0}")]
    Invalid(#[from] ResolverError),
    #[error("{0}")]
    Translation(#[from] TranslationError),
    #[error("{0}")]
    Localization(#[from] TextLocalizationError),
    #[error("請選取一至四個來源字幕片段後再翻譯。")]
    WindowSize,
    #[error("既有字幕翻譯工作異常結束。")]
    WorkerPanicked,
}
/// Range is relative to the caller's verified document, never a playback offset.
pub struct TranslatedTextWindow {
    range: Range<usize>,
    quality: TranslationQuality,
    localized: LocalizedText,
}
impl TranslatedTextWindow {
    pub fn range(&self) -> Range<usize> {
        self.range.clone()
    }
    pub fn quality(&self) -> TranslationQuality {
        self.quality
    }
    pub fn localized(&self) -> &LocalizedText {
        &self.localized
    }
    pub(crate) fn into_localized(self) -> LocalizedText {
        self.localized
    }
}
#[derive(Clone)]
struct ContextCue {
    value: StableContext,
    start_us: Option<u64>,
    duration_us: Option<u64>,
}
struct Binding {
    content_id: Box<str>,
    source: SubtitleSource,
    language: SpeechLanguage,
    end: usize,
}
struct CancelOnDrop(TranslationCancellation);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

pub struct SourceTextTranslator<'a> {
    identity: StreamIdentity,
    quality: TranslationQuality,
    provider: &'a dyn Translator,
    cpu: ChineseTextLocalizer,
    cancel: ResolverCancellation,
    previous: Option<WindowStamp>,
    binding: Option<Binding>,
    context: VecDeque<ContextCue>,
    glossary: SessionGlossary,
}
impl<'a> SourceTextTranslator<'a> {
    /// The caller supplies the already-selected/disclosed provider. No credentials
    /// are read, client created, or fallback selected here. One request at a time.
    pub fn new(
        identity: StreamIdentity,
        quality: TranslationQuality,
        provider: &'a dyn Translator,
    ) -> Result<Self, SourceTranslationError> {
        std::panic::catch_unwind(AssertUnwindSafe(|| provider.capabilities().validate()))
            .map_err(|_| SourceTranslationError::WorkerPanicked)??;
        Ok(Self {
            identity,
            quality,
            provider,
            cpu: ChineseTextLocalizer::new(identity)?,
            cancel: ResolverCancellation::default(),
            previous: None,
            binding: None,
            context: VecDeque::new(),
            glossary: SessionGlossary::new(identity),
        })
    }
    pub fn cancellation(&self) -> ResolverCancellation {
        self.cancel.clone()
    }
    /// Reuse the same bounded CPU lane for Chinese publications; no provider call.
    pub(crate) async fn localize_chinese(
        &mut self,
        query: TextQuery,
        text: ExistingText,
        cancel: ResolverCancellation,
    ) -> Result<LocalizedText, SourceTranslationError> {
        self.check_cancel(&cancel)?;
        query.stamp.validate(self.identity, Instant::now())?;
        query.validate(Instant::now())?;
        if self.previous.is_some_and(|old| {
            query.stamp.sequence <= old.sequence || query.stamp.ended_at < old.ended_at
        }) {
            return Err(ResolverError::Stale.into());
        }
        if matches!(text.language, TextLanguage::Source(_)) {
            return Err(TranslationError::UnsupportedCapability.into());
        }
        self.clear();
        self.previous = Some(query.stamp);
        Ok(self.cpu.normalize(query, text, cancel).await?)
    }
    /// Session-only terms; cleared when content/source/language changes or stops.
    pub fn glossary_mut(&mut self) -> &mut SessionGlossary {
        &mut self.glossary
    }
    pub fn clear(&mut self) {
        self.context.clear();
        self.binding = None;
        self.glossary = SessionGlossary::new(self.identity);
    }
    pub fn stop(&mut self) {
        self.cancel.cancel();
        self.clear();
        self.cpu.stop();
    }
    pub async fn translate_window(
        &mut self,
        query: TextQuery,
        text: &ExistingText,
        range: Range<usize>,
        cancel: ResolverCancellation,
    ) -> Result<TranslatedTextWindow, SourceTranslationError> {
        self.check_cancel(&cancel)?;
        query.stamp.validate(self.identity, Instant::now())?;
        query.validate(Instant::now())?;
        if self.previous.is_some_and(|old| {
            query.stamp.sequence <= old.sequence || query.stamp.ended_at < old.ended_at
        }) {
            return Err(ResolverError::Stale.into());
        }
        validate_budget(text)?;
        text.validate(&query)?;
        let TextLanguage::Source(language) = text.language else {
            return Err(TranslationError::UnsupportedCapability.into());
        };
        if range.start >= range.end
            || range.end > text.cues.len()
            || range.len() > MAX_TRANSLATION_WINDOW_CUES
        {
            return Err(SourceTranslationError::WindowSize);
        }
        let same_content = self.binding.as_ref().is_some_and(|binding| {
            binding.content_id == text.content_id
                && binding.source == text.source
                && binding.language == language
        });
        if !same_content && self.binding.is_some() {
            self.clear();
        }
        let adjacent = same_content
            && self
                .binding
                .as_ref()
                .is_some_and(|binding| binding.end == range.start);
        if !adjacent
            || self.context.iter().any(|entry| {
                text.cues
                    .get(entry.value.sequence as usize)
                    .is_none_or(|cue| {
                        cue.text != entry.value.source
                            || cue.start_us != entry.start_us
                            || cue.duration_us != entry.duration_us
                    })
            })
        {
            self.context.clear();
        }
        // Update identity now, but commit the end/context only after the whole
        // window succeeds. Cancelled/failed partial work never becomes context.
        self.binding = Some(Binding {
            content_id: text.content_id.clone(),
            source: text.source,
            language,
            end: range.start,
        });
        self.previous = Some(query.stamp);
        let stamp = query.stamp;
        let deadline = (stamp.ended_at + MAX_WINDOW_AGE).min(Instant::now() + MAX_TRANSLATION_AGE);
        let request_cancel = TranslationCancellation::new();
        let _request = CancelOnDrop(request_cancel.clone());
        let session_cancel = self.cancel.clone();
        let work =
            AssertUnwindSafe(self.prepare(query, text, range.clone(), request_cancel, deadline))
                .catch_unwind();
        let result = tokio::select! { biased;
            _ = session_cancel.cancelled() => return Err(TranslationError::Cancelled.into()),
            _ = cancel.cancelled() => return Err(TranslationError::Cancelled.into()),
            result = tokio::time::timeout_at(deadline.into(), work) => result
                .map_err(|_| SourceTranslationError::Translation(TranslationError::Timeout))?
                .map_err(|_| SourceTranslationError::WorkerPanicked)?,
        }?;
        self.check_cancel(&cancel)?;
        if Instant::now() >= deadline {
            return Err(TranslationError::Timeout.into());
        }
        stamp.validate(self.identity, Instant::now())?;
        // Commit only localized, fully reviewed results. No partial candidates escape.
        for (index, (cue, translated)) in result
            .original()
            .cues
            .iter()
            .zip(result.translated())
            .enumerate()
        {
            self.context.push_back(ContextCue {
                value: StableContext {
                    sequence: (range.start + index) as u64,
                    source: cue.text.clone(),
                    translation: translated.clone(),
                },
                start_us: cue.start_us,
                duration_us: cue.duration_us,
            });
            if self.context.len() > MAX_CONTEXT_CUES {
                self.context.pop_front();
            }
        }
        if let Some(binding) = &mut self.binding {
            binding.end = range.end;
        }
        Ok(TranslatedTextWindow {
            range,
            quality: self.quality,
            localized: result,
        })
    }
    fn check_cancel(&self, cancel: &ResolverCancellation) -> Result<(), SourceTranslationError> {
        if cancel.is_cancelled() || self.cancel.is_cancelled() {
            Err(TranslationError::Cancelled.into())
        } else {
            Ok(())
        }
    }
    async fn prepare(
        &mut self,
        query: TextQuery,
        text: &ExistingText,
        range: Range<usize>,
        cancel: TranslationCancellation,
        deadline: Instant,
    ) -> Result<LocalizedText, SourceTranslationError> {
        self.provider.capabilities().validate()?;
        let policy = QualityPolicy::for_quality(self.quality);
        let mut context = self.context.clone();
        let mut translated = Vec::with_capacity(range.len());
        let mut originals = Vec::with_capacity(range.len());
        let language = match text.language {
            TextLanguage::Source(language) => language,
            _ => return Err(TranslationError::UnsupportedCapability.into()),
        };
        for index in range {
            check_deadline(&cancel, deadline)?;
            let cue = &text.cues[index];
            let source = Arc::new(TranscriptEvent {
                identity: self.identity,
                sequence: index as u64,
                utterance: index as u64 + 1,
                revision: 1,
                start_us: cue.start_us,
                duration_us: cue.duration_us,
                language: Some(language),
                kind: TranscriptKind::Final,
                text: cue.text.clone(),
            });
            let mut request = TranslationRequest {
                source,
                quality: self.quality,
                pass: TranslationPass::Draft,
                previous_stable: context
                    .iter()
                    .skip(context.len().saturating_sub(policy.context_sentences))
                    .map(|entry| entry.value.clone())
                    .collect(),
                glossary: self.glossary.entries().to_vec(),
                draft: None,
                committed_prefix: "".into(),
                prepared_context: None,
            };
            let mut output = TranslationEmitter::final_only(&request, cancel.clone());
            let mut result = self
                .provider
                .translate(&request, &mut output, cancel.clone())
                .await?;
            check_deadline(&cancel, deadline)?;
            validate_result(&result)?;
            if policy.review_final {
                request.pass = TranslationPass::Review;
                request.draft = Some(result);
                let mut output = TranslationEmitter::final_only(&request, cancel.clone());
                result = self
                    .provider
                    .translate(&request, &mut output, cancel.clone())
                    .await?;
                check_deadline(&cancel, deadline)?;
                validate_result(&result)?;
            }
            context.push_back(ContextCue {
                value: StableContext {
                    sequence: index as u64,
                    source: cue.text.clone(),
                    translation: result.clone(),
                },
                start_us: cue.start_us,
                duration_us: cue.duration_us,
            });
            if context.len() > MAX_CONTEXT_CUES {
                context.pop_front();
            }
            translated.push(result);
            originals.push(TextCue {
                start_us: cue.start_us,
                duration_us: cue.duration_us,
                text: cue.text.clone(),
            });
        }
        let original = ExistingText {
            identity: text.identity,
            content_id: text.content_id.clone(),
            source: text.source,
            language: text.language,
            cues: originals,
        };
        Ok(self
            .cpu
            .normalize_translations(query, original, translated, self.cancel.clone())
            .await?)
    }
}
impl Drop for SourceTextTranslator<'_> {
    fn drop(&mut self) {
        self.stop();
    }
}
fn validate_result(text: &str) -> Result<(), TranslationError> {
    if text.trim().is_empty()
        || text.len() > MAX_TRANSCRIPT_BYTES
        || text
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        Err(TranslationError::InvalidData)
    } else {
        Ok(())
    }
}
fn check_deadline(
    cancel: &TranslationCancellation,
    deadline: Instant,
) -> Result<(), TranslationError> {
    if let Some(reason) = cancel.reason() {
        return Err(reason);
    }
    if Instant::now() >= deadline {
        return Err(TranslationError::Timeout);
    }
    Ok(())
}

#[cfg(test)]
#[path = "source_translation_tests.rs"]
mod tests;
