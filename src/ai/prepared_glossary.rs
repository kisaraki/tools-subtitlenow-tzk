//! Grounded, session-only terminology analysis before Prepared translation.
use super::{
    prepared_context::{PreparedDocumentContext, PreparedTextAnalysis, validate_summary},
    prepared_translation::PreparedTranslationInput,
    speech::{SpeechLanguage, StreamIdentity},
    translation::{
        GlossaryEntry, MAX_GLOSSARY_ENTRIES, MAX_TERM_BYTES, SessionGlossary,
        TranslationCancellation, TranslationError, Translator,
    },
};
use crate::{
    domain::TranslationQuality,
    resolver::contracts::{MAX_CUE_BYTES, MAX_TEXT_CUES, ResolverCancellation},
    subtitle::localization::taiwan_phrase_conversion,
};
use futures_util::FutureExt;
use std::{future::Future, panic::AssertUnwindSafe, pin::Pin, time::Duration};

/// Traverse the entire bounded document, never sample or silently truncate it.
pub const MAX_PREPARED_GLOSSARY_WINDOW_CUES: usize = 64;
pub const MAX_PREPARED_GLOSSARY_WINDOW_SOURCE_BYTES: usize = 64 * 1024;
/// 8192 cues / 4 MiB fit even when a byte-full window leaves one cue behind.
pub const MAX_PREPARED_GLOSSARY_WINDOWS: usize = 256;
pub const MAX_PREPARED_GLOSSARY_PROPOSALS: usize = 64;
pub const MAX_PREPARED_GLOSSARY_RESPONSE_BYTES: usize = 64 * 1024;
pub const PREPARED_GLOSSARY_TIMEOUT: Duration = Duration::from_secs(30);

pub struct PreparedGlossaryCue {
    pub cue_index: usize,
    pub text: Box<str>,
}
/// No Debug, Clone or serialization: adapters build only their own wire data.
pub struct PreparedGlossaryRequest {
    identity: StreamIdentity,
    sequence: u64,
    language: SpeechLanguage,
    quality: TranslationQuality,
    cues: Box<[PreparedGlossaryCue]>,
    known_terms: Box<[GlossaryEntry]>,
    prior_summary: Box<str>,
}
impl PreparedGlossaryRequest {
    pub fn new(
        identity: StreamIdentity,
        sequence: u64,
        language: SpeechLanguage,
        quality: TranslationQuality,
        cues: Vec<PreparedGlossaryCue>,
        known_terms: Vec<GlossaryEntry>,
    ) -> Result<Self, TranslationError> {
        let request = Self {
            identity,
            sequence,
            language,
            quality,
            cues: cues.into_boxed_slice(),
            known_terms: known_terms.into_boxed_slice(),
            prior_summary: "".into(),
        };
        request.validate()?;
        Ok(request)
    }
    pub fn identity(&self) -> StreamIdentity {
        self.identity
    }
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
    pub fn language(&self) -> SpeechLanguage {
        self.language
    }
    pub fn quality(&self) -> TranslationQuality {
        self.quality
    }
    pub fn cues(&self) -> &[PreparedGlossaryCue] {
        &self.cues
    }
    pub fn known_terms(&self) -> &[GlossaryEntry] {
        &self.known_terms
    }
    pub fn prior_summary(&self) -> &str {
        &self.prior_summary
    }
    pub(crate) fn with_prior_summary(mut self, summary: &str) -> Result<Self, TranslationError> {
        validate_summary(summary, true)?;
        self.prior_summary = summary.into();
        Ok(self)
    }
    pub fn validate(&self) -> Result<(), TranslationError> {
        validate_summary(&self.prior_summary, true)?;
        if self.identity.session.0 == 0
            || self.identity.generation == 0
            || self.sequence == 0
            || self.sequence > MAX_PREPARED_GLOSSARY_WINDOWS as u64
            || self.cues.is_empty()
            || self.cues.len() > MAX_PREPARED_GLOSSARY_WINDOW_CUES
            || self.known_terms.len() > MAX_GLOSSARY_ENTRIES
        {
            return Err(TranslationError::InvalidData);
        }
        let mut bytes = 0usize;
        let mut previous = None;
        for cue in &self.cues {
            if cue.cue_index >= MAX_TEXT_CUES
                || previous.is_some_and(|index| cue.cue_index != index + 1)
                || cue.text.trim().is_empty()
                || cue.text.len() > MAX_CUE_BYTES
                || cue
                    .text
                    .chars()
                    .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
            {
                return Err(TranslationError::InvalidData);
            }
            bytes = bytes
                .checked_add(cue.text.len())
                .filter(|bytes| *bytes <= MAX_PREPARED_GLOSSARY_WINDOW_SOURCE_BYTES)
                .ok_or(TranslationError::InvalidData)?;
            previous = Some(cue.cue_index);
        }
        for (index, term) in self.known_terms.iter().enumerate() {
            valid_term(&term.source)?;
            valid_term(&term.taiwan_translation)?;
            if self.known_terms[..index]
                .iter()
                .any(|old| old.source == term.source)
            {
                return Err(TranslationError::InvalidData);
            }
        }
        Ok(())
    }
}
pub struct PreparedGlossaryProposal {
    pub cue_index: usize,
    pub source_byte_start: usize,
    pub source_byte_end: usize,
    pub taiwan_translation: Box<str>,
}
pub struct PreparedGlossaryResponse {
    pub identity: StreamIdentity,
    pub sequence: u64,
    pub proposals: Vec<PreparedGlossaryProposal>,
    pub context_summary: Box<str>,
}
impl PreparedGlossaryResponse {
    pub fn validate(&self, request: &PreparedGlossaryRequest) -> Result<(), TranslationError> {
        request.validate()?;
        if self.identity != request.identity || self.sequence != request.sequence {
            return Err(TranslationError::Stale);
        }
        validate_summary(&self.context_summary, false)?;
        if self.proposals.len() > MAX_PREPARED_GLOSSARY_PROPOSALS {
            return Err(TranslationError::Overloaded);
        }
        for proposal in &self.proposals {
            valid_term(source_term(request, proposal)?)?;
            valid_term(&proposal.taiwan_translation)?;
        }
        Ok(())
    }
}
fn source_term<'a>(
    request: &'a PreparedGlossaryRequest,
    proposal: &PreparedGlossaryProposal,
) -> Result<&'a str, TranslationError> {
    let cue = request
        .cues
        .iter()
        .find(|cue| cue.cue_index == proposal.cue_index)
        .ok_or(TranslationError::InvalidData)?;
    cue.text
        .get(proposal.source_byte_start..proposal.source_byte_end)
        .ok_or(TranslationError::InvalidData)
}
fn valid_term(term: &str) -> Result<(), TranslationError> {
    if term.trim().is_empty() || term.len() > MAX_TERM_BYTES || term.chars().any(char::is_control) {
        Err(TranslationError::InvalidData)
    } else {
        Ok(())
    }
}
pub type PreparedGlossaryFuture<'a> =
    Pin<Box<dyn Future<Output = Result<PreparedGlossaryResponse, TranslationError>> + Send + 'a>>;
pub trait PreparedGlossaryAnalyzer: Send + Sync {
    fn analyze<'a>(
        &'a self,
        request: &'a PreparedGlossaryRequest,
        cancel: TranslationCancellation,
    ) -> PreparedGlossaryFuture<'a>;
}
/// GUI services must provide both capabilities. Manual translation APIs remain
/// explicit separate entry points; missing analysis never silently falls back.
pub trait PreparedTranslator: Translator + PreparedGlossaryAnalyzer {}
impl<T: Translator + PreparedGlossaryAnalyzer> PreparedTranslator for T {}

struct CancelOnDrop(TranslationCancellation);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}
struct ScopeOnDrop(ResolverCancellation);
impl Drop for ScopeOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}
struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);
impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}
fn check(cancel: &ResolverCancellation) -> Result<(), TranslationError> {
    if cancel.is_cancelled() {
        Err(TranslationError::Cancelled)
    } else {
        Ok(())
    }
}
pub(crate) async fn analyze_glossary(
    input: PreparedTranslationInput<'_>,
    quality: TranslationQuality,
    seed: &SessionGlossary,
    provider: &dyn PreparedGlossaryAnalyzer,
    cancel: &ResolverCancellation,
) -> Result<PreparedTextAnalysis, TranslationError> {
    analyze_with_timeout(
        input,
        quality,
        seed,
        provider,
        cancel,
        PREPARED_GLOSSARY_TIMEOUT,
    )
    .await
}
async fn analyze_with_timeout(
    input: PreparedTranslationInput<'_>,
    quality: TranslationQuality,
    seed: &SessionGlossary,
    provider: &dyn PreparedGlossaryAnalyzer,
    cancel: &ResolverCancellation,
    timeout: Duration,
) -> Result<PreparedTextAnalysis, TranslationError> {
    check(cancel)?;
    let scope =
        ResolverCancellation::linked(cancel.clone()).map_err(|_| TranslationError::InvalidData)?;
    let _scope_guard = ScopeOnDrop(scope.clone());
    let cancel = &scope;
    let language = input.validate()?;
    let identity = input.identity();
    let seeded = seed.entries_for(identity)?;
    let mut glossary = SessionGlossary::new(identity);
    for entry in seeded {
        valid_term(&entry.source)?;
        valid_term(&entry.taiwan_translation)?;
        glossary.set(identity, &entry.source, &entry.taiwan_translation)?;
    }
    let manual: std::sync::Arc<[Box<str>]> =
        seeded.iter().map(|entry| entry.source.clone()).collect();
    let mut start = 0usize;
    let mut sequence = 0u64;
    let mut summary: Box<str> = "".into();
    while start < input.len() {
        check(cancel)?;
        sequence += 1;
        let mut cues = Vec::new();
        let mut bytes = 0usize;
        while start + cues.len() < input.len() && cues.len() < MAX_PREPARED_GLOSSARY_WINDOW_CUES {
            let index = start + cues.len();
            let text = input.cue(index).text;
            if bytes + text.len() > MAX_PREPARED_GLOSSARY_WINDOW_SOURCE_BYTES {
                break;
            }
            bytes += text.len();
            cues.push(PreparedGlossaryCue {
                cue_index: index,
                text: text.into(),
            });
        }
        let count = cues.len();
        let request = PreparedGlossaryRequest::new(
            identity,
            sequence,
            language,
            quality,
            cues,
            glossary.entries().to_vec(),
        )?
        .with_prior_summary(&summary)?;
        let token = TranslationCancellation::new();
        let _guard = CancelOnDrop(token.clone());
        let started = std::time::Instant::now();
        let deadline = tokio::time::Instant::now() + timeout;
        let response = tokio::select! { biased;
            _ = cancel.cancelled() => return Err(TranslationError::Cancelled),
            reason = token.cancelled() => return Err(reason),
            _ = tokio::time::sleep_until(deadline) => return Err(TranslationError::Timeout),
            result = AssertUnwindSafe(async { provider.analyze(&request, token.clone()).await }).catch_unwind() =>
                result.map_err(|_| TranslationError::InvalidData)??,
        };
        check(cancel)?;
        if let Some(reason) = token.reason() {
            return Err(reason);
        }
        if started.elapsed() >= timeout {
            return Err(TranslationError::Timeout);
        }
        (glossary, summary) = tokio::select! { biased;
            _ = cancel.cancelled() => return Err(TranslationError::Cancelled),
            reason = token.cancelled() => return Err(reason),
            _ = tokio::time::sleep_until(deadline) => return Err(TranslationError::Timeout),
            result = merge(request, response, glossary, manual.clone(), cancel) => result?,
        };
        check(cancel)?;
        if let Some(reason) = token.reason() {
            return Err(reason);
        }
        if started.elapsed() >= timeout {
            return Err(TranslationError::Timeout);
        }
        start += count;
        tokio::task::yield_now().await;
    }
    check(cancel)?;
    Ok(PreparedTextAnalysis {
        glossary,
        context: std::sync::Arc::new(PreparedDocumentContext::new(
            identity,
            language,
            quality,
            input.len(),
            summary,
        )?),
    })
}
async fn merge(
    request: PreparedGlossaryRequest,
    response: PreparedGlossaryResponse,
    mut glossary: SessionGlossary,
    manual: std::sync::Arc<[Box<str>]>,
    cancel: &ResolverCancellation,
) -> Result<(SessionGlossary, Box<str>), TranslationError> {
    check(cancel)?;
    let permit = crate::media::work::workers()
        .try_acquire_owned()
        .map_err(|_| TranslationError::Overloaded)?;
    let worker_cancel = cancel.clone();
    let mut worker = AbortOnDrop(tokio::task::spawn_blocking(move || {
        let _permit = permit; // A started worker owns this until its actual exit.
        check(&worker_cancel)?;
        response.validate(&request)?;
        let summary = taiwan_phrase_conversion(&response.context_summary)?;
        validate_summary(&summary, false)?;
        for proposal in &response.proposals {
            check(&worker_cancel)?;
            let source = source_term(&request, proposal)?;
            if manual.iter().any(|term| term.as_ref() == source) {
                continue;
            }
            let target = taiwan_phrase_conversion(&proposal.taiwan_translation)?;
            valid_term(&target)?;
            if let Some(old) = glossary
                .entries()
                .iter()
                .find(|term| term.source.as_ref() == source)
            {
                if old.taiwan_translation.as_ref() != target {
                    return Err(TranslationError::InvalidData);
                }
            } else {
                glossary.set(request.identity, source, &target)?;
            }
        }
        check(&worker_cancel)?;
        Ok((glossary, summary.into_boxed_str()))
    }));
    let result = tokio::select! { biased;
        _ = cancel.cancelled() => return Err(TranslationError::Cancelled),
        result = &mut worker.0 => result.map_err(|_| TranslationError::InvalidData)?,
    };
    check(cancel)?;
    result
}

#[cfg(test)]
#[path = "prepared_glossary_tests.rs"]
mod tests;
