//! Current lookup -> bounded preparation -> explicit evidence -> shared Live arbiter.
//! No service selection, alignment guess, renderer or automatic cloud request.
use crate::{
    ai::{speech::StreamIdentity, translation::Translator},
    domain::TranslationQuality,
    resolver::{
        contracts::{
            Confidence, ExistingText, ResolverCancellation, ResolverError, TextCue, TextLanguage,
            TextQuery, WindowStamp,
        },
        text_session::ExistingTextLease,
    },
    session::live_translation::LiveTranslation,
    subtitle::{
        exact_alignment::{ExactAlignmentError, ExactCueMatch},
        existing_text::LocalizedText,
        source_arbiter::{ArbitrationError, CandidateEvidence},
        source_translation::{SourceTextTranslator, SourceTranslationError, TranslatedTextWindow},
    },
};
use std::{ops::Range, time::Instant};
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TextPreparationError {
    #[error("{0}")]
    Lookup(#[from] ResolverError),
    #[error("{0}")]
    Preparation(#[from] SourceTranslationError),
    #[error("{0}")]
    Arbitration(#[from] ArbitrationError),
    #[error("{0}")]
    Alignment(#[from] ExactAlignmentError),
    #[error("來源字幕須明確選取翻譯範圍；中文候選使用完整本地化。")]
    Selection,
}
enum PreparedBody {
    Chinese(LocalizedText),
    Source(TranslatedTextWindow),
}
/// Sealed, prepared RAM candidate; no Debug/Serialize and no raw-string constructor.
pub struct PreparedTextCandidate {
    identity: StreamIdentity,
    stamp: WindowStamp,
    parent: ResolverCancellation,
    preparation: ResolverCancellation,
    confirmed: Option<Confidence>,
    body: PreparedBody,
}
impl PreparedTextCandidate {
    /// Consume sealed ASR evidence, checking the current timeline revision before
    /// submitting. Playback selection still requires an actual observation.
    pub fn submit_matched(
        self,
        flow: &mut LiveTranslation<'_>,
        matched: ExactCueMatch,
    ) -> Result<crate::subtitle::timeline::TimelineBinding, TextPreparationError> {
        let version = flow.sources_mut().version();
        let (binding, evidence) = matched.evidence(
            self.localized(),
            self.document_range(),
            flow.timeline(),
            version,
            &self.parent,
        )?;
        self.submit(flow, evidence)?;
        Ok(binding)
    }
    pub fn localized(&self) -> &LocalizedText {
        match &self.body {
            PreparedBody::Chinese(text) => text,
            PreparedBody::Source(window) => window.localized(),
        }
    }
    pub fn document_range(&self) -> Range<usize> {
        match &self.body {
            PreparedBody::Chinese(text) => 0..text.original().cues.len(),
            PreparedBody::Source(window) => window.range(),
        }
    }
    /// Caller supplies actual alignment evidence. The lookup parent is mandatory
    /// and cannot be replaced by the caller's independent candidate token.
    pub fn submit(
        self,
        flow: &mut LiveTranslation<'_>,
        mut evidence: CandidateEvidence,
    ) -> Result<(), TextPreparationError> {
        self.stamp
            .validate(flow.timeline().identity(), Instant::now())?;
        if self.identity != flow.timeline().identity() {
            return Err(ResolverError::InvalidData.into());
        }
        if self.parent.is_cancelled() || self.preparation.is_cancelled() {
            return Err(ResolverError::Cancelled.into());
        }
        evidence.lookup_availability = Some(self.parent);
        evidence.preparation_availability = Some(self.preparation);
        if let Some(confirmed) = self.confirmed {
            evidence.content_confidence = evidence.content_confidence.min(confirmed);
        }
        match self.body {
            PreparedBody::Chinese(text) => {
                flow.sources_mut()
                    .submit_localized(text, evidence, Instant::now())?
            }
            PreparedBody::Source(window) => {
                flow.sources_mut()
                    .submit_translated(window, evidence, Instant::now())?
            }
        }
        Ok(())
    }
}
/// One caller-owned CPU lane and one mutable request; reuse the selected client.
pub struct TextPreparationSession<'a> {
    identity: StreamIdentity,
    processor: SourceTextTranslator<'a>,
}
impl<'a> TextPreparationSession<'a> {
    pub fn new(
        identity: StreamIdentity,
        quality: TranslationQuality,
        translator: &'a dyn Translator,
    ) -> Result<Self, TextPreparationError> {
        Ok(Self {
            identity,
            processor: SourceTextTranslator::new(identity, quality, translator)?,
        })
    }
    pub fn cancellation(&self) -> ResolverCancellation {
        self.processor.cancellation()
    }
    pub fn stop(&mut self) {
        self.processor.stop();
    }
    /// Withdraw source-specific context without reopening the CPU lane or
    /// forgetting the sequence watermark that prevents replaying old queries.
    pub fn clear_context(&mut self) {
        self.processor.clear();
    }
    /// Verify before any cloud call and cancel preparation when its verifier
    /// retires. The returned pair still requires submit_matched and observation.
    pub async fn prepare_matched(
        &mut self,
        matched: ExactCueMatch,
        cancel: ResolverCancellation,
    ) -> Result<(PreparedTextCandidate, ExactCueMatch), TextPreparationError> {
        matched.validate(self.identity)?;
        let owner = matched.cancellation();
        let range = matches!(matched.input().text().language, TextLanguage::Source(_))
            .then(|| matched.source_range());
        let result = tokio::select! { biased;
            _ = owner.cancelled() => Err(ResolverError::Cancelled.into()),
            result = self.prepare(matched.input(), range, cancel) => result,
        };
        if let Err(error) = matched.validate(self.identity) {
            self.processor.clear();
            return Err(error.into());
        }
        Ok((result?, matched))
    }
    pub async fn prepare(
        &mut self,
        input: ExistingTextLease,
        source_range: Option<Range<usize>>,
        cancel: ResolverCancellation,
    ) -> Result<PreparedTextCandidate, TextPreparationError> {
        input.validate(self.identity)?;
        if input.content_confidence().is_some_and(|confidence| {
            confidence.permille() < crate::subtitle::source_arbiter::MIN_CONTENT_CONFIDENCE
        }) {
            return Err(ArbitrationError::WeakEvidence.into());
        }
        let stopped = self.processor.cancellation();
        if stopped.is_cancelled() || cancel.is_cancelled() {
            return Err(ResolverError::Cancelled.into());
        }
        let is_source = matches!(input.text().language, TextLanguage::Source(_));
        if is_source != source_range.is_some() {
            return Err(TextPreparationError::Selection);
        }
        let parent = input.availability();
        let query = input.query();
        let stamp = query.stamp;
        let query_copy = TextQuery {
            stamp,
            source: query.source,
            content_id: query.content_id.clone(),
        };
        let work = async {
            if let Some(range) = source_range {
                self.processor
                    .translate_window(query_copy, input.text(), range, cancel.clone())
                    .await
                    .map(PreparedBody::Source)
            } else {
                let text = input.text();
                let owned = ExistingText {
                    identity: text.identity,
                    source: text.source,
                    content_id: text.content_id.clone(),
                    language: text.language,
                    cues: text
                        .cues
                        .iter()
                        .map(|cue| TextCue {
                            start_us: cue.start_us,
                            duration_us: cue.duration_us,
                            text: cue.text.clone(),
                        })
                        .collect(),
                };
                self.processor
                    .localize_chinese(query_copy, owned, cancel.clone())
                    .await
                    .map(PreparedBody::Chinese)
            }
        };
        let result: Result<PreparedBody, TextPreparationError> = tokio::select! { biased;
            _ = parent.cancelled() => Err(ResolverError::Cancelled.into()),
            _ = stopped.cancelled() => Err(ResolverError::Cancelled.into()),
            _ = cancel.cancelled() => Err(ResolverError::Cancelled.into()),
            result = work => result.map_err(TextPreparationError::from),
        };
        let body = match result {
            Ok(body) => body,
            Err(error) => {
                if parent.is_cancelled() || stopped.is_cancelled() || cancel.is_cancelled() {
                    self.processor.clear();
                }
                return Err(error);
            }
        };
        if let Err(error) = input.validate(self.identity) {
            self.processor.clear();
            return Err(error.into());
        }
        if stopped.is_cancelled() || cancel.is_cancelled() {
            self.processor.clear();
            return Err(ResolverError::Cancelled.into());
        }
        Ok(PreparedTextCandidate {
            identity: self.identity,
            stamp,
            parent,
            preparation: stopped,
            confirmed: input.content_confidence(),
            body,
        })
    }
}
impl Drop for TextPreparationSession<'_> {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
#[path = "text_preparation_tests.rs"]
mod tests;
