//! Bounded, generated document hints. Never evidence for identity or alignment.
use super::{
    speech::{SpeechLanguage, StreamIdentity},
    translation::{SessionGlossary, TranslationError},
};
use crate::{domain::TranslationQuality, resolver::contracts::MAX_TEXT_CUES};
use std::sync::Arc;

pub const MAX_PREPARED_SUMMARY_BYTES: usize = 4096;

pub(crate) fn validate_summary(text: &str, allow_empty: bool) -> Result<(), TranslationError> {
    if ((!allow_empty || !text.is_empty()) && text.trim().is_empty())
        || text.len() > MAX_PREPARED_SUMMARY_BYTES
        || text
            .chars()
            .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
    {
        return Err(TranslationError::InvalidData);
    }
    Ok(())
}

/// No Debug, Clone or serialization. A shared immutable hint lives only through
/// this preparation; it cannot grant subtitle adoption or playback evidence.
pub struct PreparedDocumentContext {
    identity: StreamIdentity,
    language: SpeechLanguage,
    quality: TranslationQuality,
    cue_count: usize,
    summary: Box<str>,
}
impl PreparedDocumentContext {
    pub(crate) fn new(
        identity: StreamIdentity,
        language: SpeechLanguage,
        quality: TranslationQuality,
        cue_count: usize,
        summary: Box<str>,
    ) -> Result<Self, TranslationError> {
        validate_summary(&summary, false)?;
        if identity.session.0 == 0
            || identity.generation == 0
            || !(1..=MAX_TEXT_CUES).contains(&cue_count)
        {
            return Err(TranslationError::InvalidData);
        }
        Ok(Self {
            identity,
            language,
            quality,
            cue_count,
            summary,
        })
    }
    pub fn summary(&self) -> &str {
        &self.summary
    }
    pub fn cue_count(&self) -> usize {
        self.cue_count
    }
    pub(crate) fn validate_for(
        &self,
        identity: StreamIdentity,
        language: SpeechLanguage,
        quality: TranslationQuality,
        cue_count: usize,
    ) -> Result<(), TranslationError> {
        if self.identity != identity {
            return Err(TranslationError::Stale);
        }
        if self.language != language || self.quality != quality || self.cue_count != cue_count {
            return Err(TranslationError::InvalidData);
        }
        validate_summary(&self.summary, false)
    }
}

/// Manual library callers explicitly supply only a glossary. Automatic GUI
/// analysis supplies both resources; it must not silently drop a failed summary.
pub trait PreparedTranslationContext: Send + Sync {
    fn glossary(&self) -> &SessionGlossary;
    fn document_context(&self) -> Option<&Arc<PreparedDocumentContext>>;
}
impl PreparedTranslationContext for SessionGlossary {
    fn glossary(&self) -> &SessionGlossary {
        self
    }
    fn document_context(&self) -> Option<&Arc<PreparedDocumentContext>> {
        None
    }
}
pub(crate) struct PreparedTextAnalysis {
    pub(crate) glossary: SessionGlossary,
    pub(crate) context: Arc<PreparedDocumentContext>,
}
impl PreparedTranslationContext for PreparedTextAnalysis {
    fn glossary(&self) -> &SessionGlossary {
        &self.glossary
    }
    fn document_context(&self) -> Option<&Arc<PreparedDocumentContext>> {
        Some(&self.context)
    }
}
