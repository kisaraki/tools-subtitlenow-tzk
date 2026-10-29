//! Caller-owned Prepared document work. No Live capture stamp or automatic playback.
use crate::{
    ai::{
        prepared_context::PreparedTranslationContext,
        prepared_glossary::{PreparedGlossaryAnalyzer, PreparedTranslator, analyze_glossary},
        prepared_speech::{PreparedSpeechError, PreparedSpeechRecognizer},
        prepared_transcript::PreparedTranscript,
        prepared_transcript_translation::{
            PreparedTranslatedTranscript, translate_recognized_transcript,
        },
        prepared_translation::{PreparedTranslationInput, translate_document},
        speech::StreamIdentity,
        translation::{SessionGlossary, TranslationError, Translator},
    },
    app::state::AppState,
    audio::capture::SessionId,
    domain::{SourceLanguage, TARGET_LANGUAGE, TranslationQuality},
    media::{
        MediaError,
        embedded_mp4::read_pinned as read_embedded_subtitle,
        input::{OpenedMedia, open_media, read_subtitle},
        planning::{PreparationPlan, PreparedRequest},
        prepared_asr::{PreparedAsrError, RecognizedLocalMedia, recognize_pinned_local_media},
        prepared_audio::PreparedSampleRate,
        remote_subtitle::PreparedRemoteSubtitleLease,
    },
    resolver::{
        contracts::{
            ExistingText, MAX_CUE_BYTES, MAX_TEXT_BYTES, ResolverCancellation, ResolverError,
            SourceTextReference, TextLanguage,
        },
        timed_subtitle_parse,
    },
    subtitle::{
        alignment_text::AlignmentText,
        fuzzy_alignment::{AlignmentScope, FuzzyAlignmentError},
        fuzzy_worker::FuzzyCueLocator,
        localization::taiwan_phrase_conversion,
    },
};
use std::{
    sync::{Arc, Weak},
    time::Instant,
};
use tokio::sync::Semaphore;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreparedState {
    Idle,
    Preparing,
    Ready,
    Cancelled,
    Failed(MediaError),
}
/// A complete localized document, not an audio buffer or playback clock.
/// Public access is borrowed only; private CPU work may retain text through
/// cancellation cleanup, never media resources. No Clone/Debug/Serialize/export.
pub struct PreparedDocument {
    identity: StreamIdentity,
    sequence: u64,
    _media: PreparedMediaOwner,
    body: Arc<PreparedTextOwner>,
}
/// Text and its retained-HTTP quota have one lifetime, independent of media.
/// Never expose a strong reference outside the private background boundary.
pub(crate) struct PreparedTextOwner {
    body: PreparedBody,
    // Rust drops fields in declaration order: release retained-text capacity
    // only after the original and translated document have been destroyed.
    _subtitle_lease: Option<PreparedRemoteSubtitleLease>,
}
impl PreparedTextOwner {
    pub(crate) fn display(
        &self,
        range: std::ops::Range<usize>,
        reference_range: std::ops::Range<usize>,
    ) -> Result<crate::subtitle::fuzzy_worker::AlignedCueText, FuzzyAlignmentError> {
        let original = self.alignment_source()?;
        let (translated, quality, content_id) = match &self.body {
            PreparedBody::Existing {
                original,
                translated,
                quality,
                ..
            } => (translated.as_slice(), *quality, original.content_id.clone()),
            PreparedBody::Recognized(document) => (
                document.translated(),
                Some(document.quality()),
                format!("prepared:{}", document.identity().session.0).into(),
            ),
        };
        if range.is_empty()
            || range.len() > crate::subtitle::fuzzy_alignment::MAX_MATCH_CUES
            || range.end > original.len()
            || range.end > translated.len()
        {
            return Err(FuzzyAlignmentError::Obsolete);
        }
        let first = original
            .cue(range.start)
            .ok_or(FuzzyAlignmentError::Obsolete)?;
        let last = original
            .cue(range.end - 1)
            .ok_or(FuzzyAlignmentError::Obsolete)?;
        let start = first.start_us.ok_or(FuzzyAlignmentError::TimingMismatch)?;
        let end = last
            .start_us
            .and_then(|start| {
                last.duration_us
                    .and_then(|duration| start.checked_add(duration))
            })
            .ok_or(FuzzyAlignmentError::TimingMismatch)?;
        let source = if let Some((_, reference)) = self.alignment_pair() {
            AlignmentText::Existing(&reference.text)
        } else {
            original
        };
        let source_range = if self.alignment_pair().is_some() {
            reference_range
        } else {
            range.clone()
        };
        if source_range.is_empty()
            || source_range.len() > crate::subtitle::fuzzy_alignment::MAX_MATCH_CUES
            || source_range.end > source.len()
        {
            return Err(FuzzyAlignmentError::Obsolete);
        }
        fn append(output: &mut String, text: &str) -> Result<(), FuzzyAlignmentError> {
            let total = output
                .len()
                .checked_add(text.len())
                .and_then(|value| value.checked_add(usize::from(!output.is_empty())))
                .filter(|value| *value <= MAX_CUE_BYTES)
                .ok_or(FuzzyAlignmentError::BudgetExceeded)?;
            output
                .try_reserve(total - output.len())
                .map_err(|_| FuzzyAlignmentError::Overloaded)?;
            if !output.is_empty() {
                output.push(' ');
            }
            output.push_str(text);
            Ok(())
        }
        let mut source_text = String::new();
        let mut target_text = String::new();
        for index in source_range {
            append(
                &mut source_text,
                source.cue(index).ok_or(FuzzyAlignmentError::Obsolete)?.text,
            )?;
        }
        for text in &translated[range.clone()] {
            append(&mut target_text, text)?;
        }
        Ok(crate::subtitle::fuzzy_worker::AlignedCueText {
            original: source_text.into(),
            translated: target_text.into(),
            content_id,
            cue_index: range.start,
            media: start..end,
            quality,
            text_sequence: 0,
        })
    }
    pub(crate) fn alignment_pair(&self) -> Option<(&ExistingText, &SourceTextReference)> {
        match &self.body {
            PreparedBody::Existing {
                original,
                reference: Some(reference),
                ..
            } => Some((original, reference)),
            _ => None,
        }
    }
    pub(crate) fn alignment_source(&self) -> Result<AlignmentText<'_>, FuzzyAlignmentError> {
        if let Some(lease) = &self._subtitle_lease {
            lease.check().map_err(|_| ResolverError::Cancelled)?;
        }
        match &self.body {
            PreparedBody::Existing { original, .. } => Ok(AlignmentText::Existing(original)),
            PreparedBody::Recognized(document) => {
                Ok(AlignmentText::Recognized(document.original()))
            }
        }
    }
}
#[derive(Clone)]
pub(crate) struct PreparedAlignmentLease {
    pub(crate) text: Weak<PreparedTextOwner>,
    pub(crate) publication: ResolverCancellation,
    pub(crate) identity: StreamIdentity,
    pub(crate) sequence: u64,
}
enum PreparedMediaOwner {
    Local(OpenedMedia),
    #[cfg(any(windows, test))]
    Remote(Box<crate::media::remote_wave::RemoteWaveDecoder>),
}
impl PreparedMediaOwner {
    fn byte_len(&self) -> u64 {
        match self {
            Self::Local(media) => media.byte_len(),
            #[cfg(any(windows, test))]
            Self::Remote(media) => media.byte_len(),
        }
    }
}
impl From<OpenedMedia> for PreparedMediaOwner {
    fn from(media: OpenedMedia) -> Self {
        Self::Local(media)
    }
}
enum PreparedBody {
    Existing {
        original: ExistingText,
        translated: Vec<Box<str>>,
        reference: Option<SourceTextReference>,
        quality: Option<TranslationQuality>,
    },
    Recognized(PreparedTranslatedTranscript),
}
/// A temporary borrow retains the genuine source kind. Recognized ASR never
/// masquerades as an existing-subtitle publication or playback alignment proof.
pub enum PreparedOriginal<'a> {
    Existing(&'a ExistingText),
    Recognized(&'a PreparedTranscript),
}
impl<'a> PreparedOriginal<'a> {
    pub const fn existing(self) -> Option<&'a ExistingText> {
        match self {
            Self::Existing(original) => Some(original),
            Self::Recognized(_) => None,
        }
    }
    pub const fn recognized(self) -> Option<&'a PreparedTranscript> {
        match self {
            Self::Recognized(original) => Some(original),
            Self::Existing(_) => None,
        }
    }
}
impl PreparedDocument {
    pub fn identity(&self) -> StreamIdentity {
        self.identity
    }
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
    pub fn original(&self) -> PreparedOriginal<'_> {
        match &self.body.body {
            PreparedBody::Existing { original, .. } => PreparedOriginal::Existing(original),
            PreparedBody::Recognized(document) => PreparedOriginal::Recognized(document.original()),
        }
    }
    pub fn cue_count(&self) -> usize {
        match &self.body.body {
            PreparedBody::Existing { original, .. } => original.cues.len(),
            PreparedBody::Recognized(document) => document.original().cues().len(),
        }
    }
    pub fn translated(&self) -> &[Box<str>] {
        match &self.body.body {
            PreparedBody::Existing { translated, .. } => translated,
            PreparedBody::Recognized(document) => document.translated(),
        }
    }
    /// Actual source-text translation quality. Native Chinese localization has
    /// no AI quality tier; it does not imply a downgraded translation strategy.
    pub fn quality(&self) -> Option<TranslationQuality> {
        match &self.body.body {
            PreparedBody::Existing { quality, .. } => *quality,
            PreparedBody::Recognized(document) => Some(document.quality()),
        }
    }
    pub const fn target_language(&self) -> &'static str {
        TARGET_LANGUAGE
    }
    pub fn media_byte_len(&self) -> u64 {
        self._media.byte_len()
    }
}
pub struct PreparedCompletion {
    identity: StreamIdentity,
    sequence: u64,
    pub started_at: Instant,
    pub finished_at: Instant,
    result: Result<PreparedDocument, MediaError>,
}
pub struct PreparedSession {
    identity: Option<StreamIdentity>,
    sequence: u64,
    cancellation: ResolverCancellation,
    state: PreparedState,
    document: Option<PreparedDocument>,
    workers: Arc<Semaphore>,
}
/// Explicitly retained active Ready session. Still RAM-only and owns the same
/// media pin; closing its UI owner retires text and every derived alignment.
pub struct PreparedAttachment {
    session: PreparedSession,
}
impl PreparedAttachment {
    pub fn alignment_source(
        &self,
    ) -> Result<crate::subtitle::fuzzy_worker::AlignmentSource, FuzzyAlignmentError> {
        self.session.alignment_source()
    }
    pub fn cue_count(&self) -> usize {
        self.session.ready().map_or(0, PreparedDocument::cue_count)
    }
    pub(crate) fn into_session(self) -> PreparedSession {
        self.session
    }
}
impl Default for PreparedSession {
    fn default() -> Self {
        Self::new()
    }
}
impl PreparedSession {
    pub(crate) fn into_attachment(self) -> PreparedAttachment {
        PreparedAttachment { session: self }
    }
    pub fn new() -> Self {
        let workers = crate::media::work::workers();
        Self {
            identity: None,
            sequence: 0,
            cancellation: ResolverCancellation::default(),
            state: PreparedState::Idle,
            document: None,
            workers,
        }
    }
    pub fn state(&self) -> PreparedState {
        if matches!(self.state, PreparedState::Preparing | PreparedState::Ready)
            && self.cancellation.is_cancelled()
        {
            PreparedState::Cancelled
        } else {
            self.state
        }
    }
    pub fn app_state(&self) -> AppState {
        match self.state() {
            PreparedState::Preparing => AppState::PreparingMedia,
            PreparedState::Ready => AppState::PreparedReady,
            PreparedState::Failed(_) => AppState::Error,
            _ => AppState::Idle,
        }
    }
    pub fn identity(&self) -> Option<StreamIdentity> {
        self.identity
    }
    pub fn ready(&self) -> Option<&PreparedDocument> {
        (self.state() == PreparedState::Ready)
            .then_some(self.document.as_ref())
            .flatten()
    }
    /// Explicit Ready-to-capture proposal bridge. No capture, cloud, playback,
    /// native resource sharing, or text copy occurs here. Stop/replacement of
    /// this Session revokes all derived locators and delivered proposals.
    pub fn fuzzy_locator(
        &self,
        capture_identity: StreamIdentity,
        epoch: u64,
    ) -> Result<FuzzyCueLocator, FuzzyAlignmentError> {
        let ready = self.ready().ok_or(FuzzyAlignmentError::NotReady)?;
        if matches!(ready.original(), PreparedOriginal::Existing(original) if !matches!(original.language, TextLanguage::Source(_)))
            && ready.body.alignment_pair().is_none()
        {
            return Err(FuzzyAlignmentError::UnsupportedEvidence);
        }
        let scope = AlignmentScope::new(ready.identity(), capture_identity, epoch)?;
        FuzzyCueLocator::from_prepared(
            scope,
            PreparedAlignmentLease {
                text: Arc::downgrade(&ready.body),
                publication: self.cancellation.clone(),
                identity: ready.identity(),
                sequence: ready.sequence(),
            },
        )
    }
    /// Passive weak Ready handoff; no capture identity, media handle or strong
    /// document ownership is transferred to the Live worker.
    pub fn alignment_source(
        &self,
    ) -> Result<crate::subtitle::fuzzy_worker::AlignmentSource, FuzzyAlignmentError> {
        let ready = self.ready().ok_or(FuzzyAlignmentError::NotReady)?;
        if matches!(ready.original(), PreparedOriginal::Existing(original) if !matches!(original.language, TextLanguage::Source(_)))
            && ready.body.alignment_pair().is_none()
        {
            return Err(FuzzyAlignmentError::UnsupportedEvidence);
        }
        crate::subtitle::fuzzy_worker::AlignmentSource::prepared(PreparedAlignmentLease {
            text: Arc::downgrade(&ready.body),
            publication: self.cancellation.clone(),
            identity: ready.identity(),
            sequence: ready.sequence(),
        })
    }
    /// New media work always gets a globally unique Session and retires old work.
    /// The task is caller-driven; creating it starts no I/O, task or capture.
    pub fn start(&mut self, request: PreparedRequest) -> Result<PreparedTask, MediaError> {
        let sequence = self
            .sequence
            .checked_add(1)
            .ok_or(MediaError::SessionExhausted)?;
        let identity = StreamIdentity {
            session: SessionId::allocate().map_err(|_| MediaError::SessionExhausted)?,
            generation: 1,
        };
        self.stop();
        self.sequence = sequence;
        self.identity = Some(identity);
        self.cancellation = ResolverCancellation::default();
        self.state = PreparedState::Preparing;
        Ok(PreparedTask {
            identity,
            sequence,
            started_at: Instant::now(),
            request: Some(request),
            cancellation: self.cancellation.clone(),
            cancel_on_drop: true,
            workers: self.workers.clone(),
        })
    }
    /// Only the current exact completion can become Ready; late results leave it intact.
    pub fn apply(&mut self, completion: PreparedCompletion) -> Result<bool, MediaError> {
        if self.identity != Some(completion.identity) || self.sequence != completion.sequence {
            return Err(MediaError::Stale);
        }
        if self.cancellation.is_cancelled() {
            return Err(MediaError::Cancelled);
        }
        if self.state != PreparedState::Preparing {
            return Err(MediaError::Stale);
        }
        match completion.result {
            Ok(document) => {
                self.document = Some(document);
                self.state = PreparedState::Ready;
                Ok(true)
            }
            Err(error) => {
                self.document = None;
                self.cancellation.cancel();
                self.state = if error == MediaError::Cancelled {
                    PreparedState::Cancelled
                } else {
                    PreparedState::Failed(error)
                };
                Err(error)
            }
        }
    }
    pub fn stop(&mut self) {
        self.cancellation.cancel();
        self.document = None;
        self.identity = None;
        self.state = PreparedState::Cancelled;
    }
}
impl Drop for PreparedSession {
    fn drop(&mut self) {
        self.stop();
    }
}
pub struct PreparedTask {
    identity: StreamIdentity,
    sequence: u64,
    started_at: Instant,
    request: Option<PreparedRequest>,
    cancellation: ResolverCancellation,
    cancel_on_drop: bool,
    workers: Arc<Semaphore>,
}
struct LoadedDocument {
    media: PreparedMediaOwner,
    original: ExistingText,
    reference: Option<SourceTextReference>,
    subtitle_lease: Option<PreparedRemoteSubtitleLease>,
    quality: Option<TranslationQuality>,
}
/// Automatic analysis is explicit at the task entry point. The older library
/// entry points continue to accept a caller-managed glossary and translator.
#[derive(Clone, Copy)]
struct PreparedTextServices<'a> {
    translator: &'a dyn Translator,
    analyzer: Option<&'a dyn PreparedGlossaryAnalyzer>,
}
impl<'a> PreparedTextServices<'a> {
    fn manual(translator: &'a dyn Translator) -> Self {
        Self {
            translator,
            analyzer: None,
        }
    }
    fn analyzed(provider: &'a dyn PreparedTranslator) -> Self {
        Self {
            translator: provider,
            analyzer: Some(provider),
        }
    }
}
struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);
impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}
impl PreparedTask {
    /// The application worker checks this before credential/client construction.
    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }
    pub async fn run(self) -> PreparedCompletion {
        self.run_with(prepare).await
    }
    /// The caller supplies its explicitly selected/disclosed provider. Chinese
    /// text uses only the local CPU lane; no keys, client or fallback are chosen.
    pub async fn run_with_translator(
        mut self,
        provider: &dyn Translator,
        quality: TranslationQuality,
        glossary: Option<&SessionGlossary>,
    ) -> PreparedCompletion {
        let result = self
            .translate(PreparedTextServices::manual(provider), quality, glossary)
            .await;
        self.complete(result)
    }
    /// Analyze every source-text window before any translation. Both operations
    /// belong to the same explicitly selected service. Chinese text stays local.
    pub async fn run_with_analyzed_translator(
        mut self,
        provider: &dyn PreparedTranslator,
        quality: TranslationQuality,
        glossary: Option<&SessionGlossary>,
    ) -> PreparedCompletion {
        let result = self
            .translate(PreparedTextServices::analyzed(provider), quality, glossary)
            .await;
        self.complete(result)
    }
    /// Explicit ASR fallback only after the existing-text preference plan. The
    /// caller selects both providers; this domain layer reads no credentials,
    /// chooses no default provider and depends on no GUI. Chinese sidecars
    /// ignore these providers.
    pub async fn run_with_recognizer(
        mut self,
        recognizer: &dyn PreparedSpeechRecognizer,
        translator: &dyn Translator,
        language: SourceLanguage,
        rate: PreparedSampleRate,
        quality: TranslationQuality,
        glossary: Option<&SessionGlossary>,
    ) -> PreparedCompletion {
        let result = self
            .recognize(
                recognizer,
                PreparedTextServices::manual(translator),
                language,
                rate,
                quality,
                glossary,
            )
            .await;
        self.complete(result)
    }
    /// ASR and existing source subtitles share the same automatic glossary pass.
    /// The original media remains pinned while analysis and translation await.
    pub async fn run_with_analyzed_recognizer(
        mut self,
        recognizer: &dyn PreparedSpeechRecognizer,
        translator: &dyn PreparedTranslator,
        language: SourceLanguage,
        rate: PreparedSampleRate,
        quality: TranslationQuality,
        glossary: Option<&SessionGlossary>,
    ) -> PreparedCompletion {
        let result = self
            .recognize(
                recognizer,
                PreparedTextServices::analyzed(translator),
                language,
                rate,
                quality,
                glossary,
            )
            .await;
        self.complete(result)
    }
    async fn recognize(
        &mut self,
        recognizer: &dyn PreparedSpeechRecognizer,
        services: PreparedTextServices<'_>,
        language: SourceLanguage,
        rate: PreparedSampleRate,
        quality: TranslationQuality,
        glossary: Option<&SessionGlossary>,
    ) -> Result<PreparedDocument, MediaError> {
        check(&self.cancellation)?;
        let plan = legacy_text_plan(self.request.as_ref().ok_or(MediaError::InvalidInput)?)?;
        if plan != PreparationPlan::AsrRequired {
            return self.translate(services, quality, glossary).await;
        }
        // Reject stale Session/generation terms before any decoder or ASR work,
        // including a provider capability call that could initiate its own work.
        let empty = SessionGlossary::new(self.identity);
        let glossary = glossary.unwrap_or(&empty);
        glossary
            .entries_for(self.identity)
            .map_err(translation_error)?;
        let request = self.request.take().ok_or(MediaError::InvalidInput)?;
        // Lower owners cancel their own scope on failure/Drop. A child keeps a
        // typed failure distinct from an explicit stop of the Prepared session.
        let scope = ResolverCancellation::linked(self.cancellation.clone())
            .map_err(|_| MediaError::InvalidInput)?;
        let RecognizedLocalMedia { media, transcript } = recognize_pinned_local_media(
            request.media,
            self.identity,
            language,
            rate,
            scope.clone(),
            recognizer,
        )
        .await
        .map_err(asr_error)?;
        self.finish_recognized(media.into(), transcript, services, quality, glossary, scope)
            .await
    }
    /// Local and remote recognition share the exact document analysis,
    /// translation, localization and validation path. The source owner is held
    /// through every wait and moves into Ready only after complete success.
    async fn finish_recognized(
        &self,
        media: PreparedMediaOwner,
        transcript: PreparedTranscript,
        services: PreparedTextServices<'_>,
        quality: TranslationQuality,
        glossary: &SessionGlossary,
        scope: ResolverCancellation,
    ) -> Result<PreparedDocument, MediaError> {
        check(&self.cancellation)?;
        if transcript.identity() != self.identity {
            return Err(MediaError::Stale);
        }
        let generated = match services.analyzer {
            Some(analyzer) => Some(
                analyze_glossary(
                    PreparedTranslationInput::Asr(&transcript),
                    quality,
                    glossary,
                    analyzer,
                    &scope,
                )
                .await
                .map_err(translation_error)?,
            ),
            None => None,
        };
        let glossary: &dyn PreparedTranslationContext = generated
            .as_ref()
            .map(|analysis| analysis as &dyn PreparedTranslationContext)
            .unwrap_or(glossary);
        // Keep the same pinned input alive across all translation/provider waits.
        let translated = translate_recognized_transcript(
            transcript,
            quality,
            glossary,
            services.translator,
            scope,
        )
        .await
        .map_err(translation_error)?;
        check(&self.cancellation)?;
        if translated.identity() != self.identity {
            return Err(MediaError::Stale);
        }
        if translated.translated().len() != translated.original().cues().len() {
            return Err(MediaError::InvalidSubtitle);
        }
        Ok(PreparedDocument {
            identity: self.identity,
            sequence: self.sequence,
            _media: media,
            body: Arc::new(PreparedTextOwner {
                body: PreparedBody::Recognized(translated),
                _subtitle_lease: None,
            }),
        })
    }
    async fn translate(
        &mut self,
        services: PreparedTextServices<'_>,
        quality: TranslationQuality,
        glossary: Option<&SessionGlossary>,
    ) -> Result<PreparedDocument, MediaError> {
        let loaded = self
            .process(|request, identity, sequence, cancel| {
                load(request, identity, sequence, cancel, true)
            })
            .await?;
        self.translate_loaded(loaded, services, quality, glossary)
            .await
    }
    /// Existing text shares the same analysis, translation and finalization
    /// regardless of whether its media owner is a local pin or remote WAVE.
    /// Neither branch requests audio or fabricates playback alignment evidence.
    async fn translate_loaded(
        &self,
        mut loaded: LoadedDocument,
        services: PreparedTextServices<'_>,
        quality: TranslationQuality,
        glossary: Option<&SessionGlossary>,
    ) -> Result<PreparedDocument, MediaError> {
        let translated = if matches!(loaded.original.language, TextLanguage::Source(_)) {
            loaded.quality = Some(quality);
            let empty = SessionGlossary::new(self.identity);
            let glossary = glossary.unwrap_or(&empty);
            let generated = match services.analyzer {
                Some(analyzer) => Some(
                    analyze_glossary(
                        PreparedTranslationInput::Existing(&loaded.original),
                        quality,
                        glossary,
                        analyzer,
                        &self.cancellation,
                    )
                    .await
                    .map_err(translation_error)?,
                ),
                None => None,
            };
            translate_document(
                &loaded.original,
                quality,
                generated
                    .as_ref()
                    .map(|analysis| analysis as &dyn PreparedTranslationContext)
                    .unwrap_or(glossary),
                services.translator,
                &self.cancellation,
            )
            .await
            .map_err(translation_error)?
        } else {
            Vec::new()
        };
        // No CPU permit is retained while awaiting cloud translation. Restore,
        // localize and validate the complete result on the bounded CPU lane.
        let identity = self.identity;
        let sequence = self.sequence;
        self.blocking(move |cancel| finish(loaded, translated, identity, sequence, cancel))
            .await
    }
    async fn run_with<F>(mut self, operation: F) -> PreparedCompletion
    where
        F: FnOnce(
                PreparedRequest,
                StreamIdentity,
                u64,
                &ResolverCancellation,
            ) -> Result<PreparedDocument, MediaError>
            + Send
            + 'static,
    {
        let result = self.process(operation).await;
        self.complete(result)
    }
    fn complete(&mut self, result: Result<PreparedDocument, MediaError>) -> PreparedCompletion {
        self.cancel_on_drop = false;
        PreparedCompletion {
            identity: self.identity,
            sequence: self.sequence,
            started_at: self.started_at,
            finished_at: Instant::now(),
            result,
        }
    }
    async fn process<T, F>(&mut self, operation: F) -> Result<T, MediaError>
    where
        T: Send + 'static,
        F: FnOnce(
                PreparedRequest,
                StreamIdentity,
                u64,
                &ResolverCancellation,
            ) -> Result<T, MediaError>
            + Send
            + 'static,
    {
        check(&self.cancellation)?;
        legacy_text_plan(self.request.as_ref().ok_or(MediaError::InvalidInput)?)?;
        let request = self.request.take().ok_or(MediaError::InvalidInput)?;
        let identity = self.identity;
        let sequence = self.sequence;
        self.blocking(move |cancel| operation(request, identity, sequence, cancel))
            .await
    }
    async fn blocking<T, F>(&self, operation: F) -> Result<T, MediaError>
    where
        T: Send + 'static,
        F: FnOnce(&ResolverCancellation) -> Result<T, MediaError> + Send + 'static,
    {
        check(&self.cancellation)?;
        let permit = self
            .workers
            .clone()
            .try_acquire_owned()
            .map_err(|_| MediaError::Overloaded)?;
        let cancel = self.cancellation.clone();
        let mut worker = AbortOnDrop(tokio::task::spawn_blocking(move || {
            let _permit = permit; // Held by a started worker even after future Drop.
            operation(&cancel)
        }));
        let result = tokio::select! { biased;
            _ = self.cancellation.cancelled() => return Err(MediaError::Cancelled),
            result = &mut worker.0 => result.map_err(|_| MediaError::WorkerFailed)?,
        };
        if self.cancellation.is_cancelled() {
            return Err(MediaError::Cancelled);
        }
        result
    }
}
impl Drop for PreparedTask {
    fn drop(&mut self) {
        if self.cancel_on_drop {
            self.cancellation.cancel();
        }
    }
}
fn resolver_error(error: ResolverError) -> MediaError {
    match error {
        ResolverError::Cancelled => MediaError::Cancelled,
        ResolverError::UnsupportedCapability => MediaError::UnsupportedSubtitle,
        _ => MediaError::InvalidSubtitle,
    }
}
fn translation_error(error: TranslationError) -> MediaError {
    match error {
        TranslationError::Cancelled => MediaError::Cancelled,
        _ => MediaError::Translation(error),
    }
}
fn asr_error(error: PreparedAsrError) -> MediaError {
    match error {
        PreparedAsrError::Media(error) => error,
        PreparedAsrError::Speech(PreparedSpeechError::Cancelled) => MediaError::Cancelled,
        PreparedAsrError::Speech(error) => MediaError::Speech(error),
    }
}
fn check(cancel: &ResolverCancellation) -> Result<(), MediaError> {
    if cancel.is_cancelled() {
        Err(MediaError::Cancelled)
    } else {
        Ok(())
    }
}
fn prepare(
    request: PreparedRequest,
    identity: StreamIdentity,
    sequence: u64,
    cancel: &ResolverCancellation,
) -> Result<PreparedDocument, MediaError> {
    let loaded = load(request, identity, sequence, cancel, false)?;
    finish(loaded, Vec::new(), identity, sequence, cancel)
}
fn load(
    request: PreparedRequest,
    identity: StreamIdentity,
    sequence: u64,
    cancel: &ResolverCancellation,
    allow_source: bool,
) -> Result<LoadedDocument, MediaError> {
    check(cancel)?;
    let plan = legacy_text_plan(&request)?;
    let media = open_media(&request.media, cancel)?;
    let index = match plan {
        PreparationPlan::Subtitle(index) => index,
        PreparationPlan::EmbeddedMp4 => {
            let selection = request.embedded.ok_or(MediaError::InvalidInput)?;
            if !allow_source && matches!(selection.language(), TextLanguage::Source(_)) {
                return Err(MediaError::TranslationRequired);
            }
            let document = read_embedded_subtitle(media, identity, selection, cancel)?;
            check(cancel)?;
            return Ok(LoadedDocument {
                reference: load_reference(request.reference.as_ref(), &document.text, cancel)?,
                quality: None,
                media: document.media.into(),
                original: document.text,
                subtitle_lease: None,
            });
        }
        PreparationPlan::AsrRequired => return Err(MediaError::AsrUnavailable),
        PreparationPlan::RemoteSubtitle(_) => return Err(MediaError::UnsupportedUrl),
    };
    let original = load_sidecar(&request, index, identity, sequence, cancel, allow_source)?;
    Ok(LoadedDocument {
        reference: load_reference(request.reference.as_ref(), &original, cancel)?,
        quality: None,
        media: media.into(),
        original,
        subtitle_lease: None,
    })
}
/// Read only the chosen local sidecar. The media owner is acquired separately;
/// this helper never interprets subtitle paths as remote media or falls to ASR.
fn load_sidecar(
    request: &PreparedRequest,
    index: usize,
    identity: StreamIdentity,
    sequence: u64,
    cancel: &ResolverCancellation,
    allow_source: bool,
) -> Result<ExistingText, MediaError> {
    check(cancel)?;
    let subtitle = request
        .subtitles
        .get(index)
        .ok_or(MediaError::InvalidInput)?;
    if !allow_source && matches!(subtitle.language, TextLanguage::Source(_)) {
        return Err(MediaError::TranslationRequired);
    }
    let bytes = read_subtitle(subtitle, cancel)?;
    let content_id = format!("prepared:{}:{sequence}", identity.session.0);
    let original = timed_subtitle_parse::parse_prepared(
        subtitle.format,
        &bytes,
        identity,
        &content_id,
        subtitle.language,
        cancel,
    )
    .map_err(resolver_error)?;
    drop(bytes); // Parsed cues own their text; release the raw read buffer now.
    check(cancel)?;
    Ok(original)
}
fn load_reference(
    selection: Option<&crate::media::planning::PreparedSourceReference>,
    primary: &ExistingText,
    cancel: &ResolverCancellation,
) -> Result<Option<SourceTextReference>, MediaError> {
    let Some(selection) = selection else {
        return Ok(None);
    };
    check(cancel)?;
    let bytes = read_subtitle(&selection.input, cancel)?;
    let text = timed_subtitle_parse::parse_prepared(
        selection.input.format,
        &bytes,
        primary.identity,
        &primary.content_id,
        selection.input.language,
        cancel,
    )
    .map_err(resolver_error)?;
    drop(bytes);
    let reference = SourceTextReference {
        text,
        primary_edition_id: selection.edition.clone(),
        reference_edition_id: selection.edition.clone(),
    };
    reference
        .validate_document(primary, primary.identity)
        .map_err(resolver_error)?;
    crate::subtitle::fuzzy_alignment::paired::validate_paired_parts(primary, &reference).map_err(
        |error| match error {
            FuzzyAlignmentError::BudgetExceeded => MediaError::TooLarge,
            FuzzyAlignmentError::Invalid(error) => resolver_error(error),
            _ => MediaError::InvalidSubtitle,
        },
    )?;
    check(cancel)?;
    Ok(Some(reference))
}
fn finish(
    loaded: LoadedDocument,
    translated_source: Vec<Box<str>>,
    identity: StreamIdentity,
    sequence: u64,
    cancel: &ResolverCancellation,
) -> Result<PreparedDocument, MediaError> {
    check(cancel)?;
    if let Some(lease) = &loaded.subtitle_lease {
        lease
            .check()
            .map_err(crate::media::remote_subtitle::RemoteSubtitleError::into_media)?;
    }
    // Keep original text and its lease co-owned through every fallible step.
    // Early returns must destroy retained text before releasing its capacity.
    let original = &loaded.original;
    original
        .validate_document(identity, original.source, &original.content_id)
        .map_err(resolver_error)?;
    let source = matches!(original.language, TextLanguage::Source(_));
    if (source && translated_source.len() != original.cues.len())
        || (!source && !translated_source.is_empty())
    {
        return Err(MediaError::InvalidSubtitle);
    }
    let mut translated = Vec::with_capacity(original.cues.len());
    let mut total = 0usize;
    for (index, cue) in original.cues.iter().enumerate() {
        check(cancel)?;
        let text = if source {
            translated_source[index].as_ref()
        } else {
            cue.text.as_ref()
        };
        if invalid_output(text) {
            return Err(MediaError::InvalidSubtitle);
        }
        let localized = taiwan_phrase_conversion(text).map_err(|_| MediaError::InvalidSubtitle)?;
        total = total
            .checked_add(localized.len())
            .filter(|total| *total <= MAX_TEXT_BYTES)
            .ok_or(MediaError::TooLarge)?;
        if invalid_output(&localized) {
            return Err(MediaError::InvalidSubtitle);
        }
        translated.push(localized.into_boxed_str());
    }
    check(cancel)?;
    Ok(PreparedDocument {
        identity,
        sequence,
        _media: loaded.media,
        body: Arc::new(PreparedTextOwner {
            body: PreparedBody::Existing {
                original: loaded.original,
                translated,
                reference: loaded.reference,
                quality: loaded.quality,
            },
            _subtitle_lease: loaded.subtitle_lease,
        }),
    })
}
fn invalid_output(text: &str) -> bool {
    text.trim().is_empty()
        || text.len() > MAX_CUE_BYTES
        || text
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
}

/// Existing task methods do not opt in to a network subtitle destination.
/// Preserve ambiguity before rejecting any explicit remote choice, including
/// lower-priority candidates that would otherwise be silently ignored.
fn legacy_text_plan(request: &PreparedRequest) -> Result<PreparationPlan, MediaError> {
    let plan = request.plan()?;
    if !request.remote_subtitles.is_empty() {
        return Err(MediaError::UnsupportedUrl);
    }
    Ok(plan)
}

#[cfg(test)]
#[path = "prepared_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "prepared_translation_tests.rs"]
mod translation_tests;

#[cfg(test)]
#[path = "prepared_asr_tests.rs"]
mod asr_tests;

#[cfg(all(test, windows))]
#[path = "prepared_embedded_tests.rs"]
mod embedded_tests;

#[cfg(any(windows, test))]
#[path = "prepared_remote.rs"]
mod remote;

#[cfg(any(windows, test))]
#[path = "prepared_https.rs"]
mod https;
