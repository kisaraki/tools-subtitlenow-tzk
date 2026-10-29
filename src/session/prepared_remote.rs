//! Explicit remote WAVE preparation shared by callers and the Prepared worker.
use super::*;
use crate::media::{
    remote_http::{RemoteHttpMedia, RemoteMediaError},
    remote_prepared::{RemotePreparedError, recognize_remote_wave},
    remote_probe::{RemoteProbeError, probe_remote_audio},
    remote_wave::{RemoteWaveDecoder, RemoteWaveError},
};
use std::panic::AssertUnwindSafe;

impl PreparedTask {
    /// Explicit local SRT/WebVTT alongside a disclosed remote PCM16 WAVE.
    /// Only headers are read from the media source; selected text never falls
    /// through to audio recognition. Chinese text needs no provider or key.
    /// The caller supplies the sole analyzed translator for source-language text.
    pub async fn run_with_remote_wave_subtitle(
        mut self,
        provider: Option<&dyn PreparedTranslator>,
        quality: TranslationQuality,
        glossary: Option<&SessionGlossary>,
    ) -> PreparedCompletion {
        let result = self.remote_subtitle(provider, quality, glossary).await;
        self.complete(result)
    }
    pub(super) async fn remote_subtitle(
        &mut self,
        provider: Option<&dyn PreparedTranslator>,
        quality: TranslationQuality,
        glossary: Option<&SessionGlossary>,
    ) -> Result<PreparedDocument, MediaError> {
        check(&self.cancellation)?;
        let request = self.request.as_ref().ok_or(MediaError::InvalidInput)?;
        // Preserve ambiguity precedence and reject every remote embedded choice,
        // even if a higher-priority sidecar would otherwise win the plan.
        let plan = request.plan()?;
        if !request.remote_subtitles.is_empty() {
            return Err(MediaError::UnsupportedUrl);
        }
        if !request.media.is_remote() {
            return Err(MediaError::InvalidInput);
        }
        if request.embedded.is_some() {
            return Err(MediaError::UnsupportedUrl);
        }
        let PreparationPlan::Subtitle(index) = plan else {
            return Err(MediaError::UnsupportedUrl);
        };
        let language = request
            .text_language()?
            .ok_or(MediaError::InvalidSubtitle)?;
        if matches!(language, TextLanguage::Source(_)) && provider.is_none() {
            return Err(MediaError::TranslationRequired);
        }
        if let Some(glossary) = glossary {
            glossary
                .entries_for(self.identity)
                .map_err(translation_error)?;
        }
        let request = self.request.take().ok_or(MediaError::InvalidInput)?;
        let identity = self.identity;
        let sequence = self.sequence;
        // Malformed/unreadable selected text fails before media connection. Only
        // the chosen file is read on the existing bounded CPU/disk worker.
        let (input, original, reference) = self
            .blocking(move |cancel| {
                let original = load_sidecar(&request, index, identity, sequence, cancel, true)?;
                let reference = load_reference(request.reference.as_ref(), &original, cancel)?;
                Ok((request.media, original, reference))
            })
            .await?;
        let scope = ResolverCancellation::linked(self.cancellation.clone())
            .map_err(|_| MediaError::InvalidInput)?;
        let media = open_remote_wave(input, self.identity, scope).await?;
        let loaded = LoadedDocument {
            reference,
            quality: None,
            media: PreparedMediaOwner::Remote(Box::new(media)),
            original,
            subtitle_lease: None,
        };
        match provider {
            Some(provider) => {
                self.translate_loaded(
                    loaded,
                    PreparedTextServices::analyzed(provider),
                    quality,
                    glossary,
                )
                .await
            }
            None => {
                self.blocking(move |cancel| finish(loaded, Vec::new(), identity, sequence, cancel))
                    .await
            }
        }
    }
    /// Explicit remote audio fallback, with the caller's disclosed recognizer
    /// and analyzed translator. No credential/default provider or GUI is chosen.
    /// Only an HTTP(S) request without selected subtitles is supported here;
    /// existing text is never ignored or replaced by an audio upload. The usual
    /// Local-only task entry points still reject remote media; the controller
    /// explicitly chooses this method for a disclosed HTTP(S) audio request.
    ///
    /// Construction starts no I/O. Task Drop, even unpolled, retires the Session.
    pub async fn run_with_remote_wave_recognizer(
        mut self,
        recognizer: &dyn PreparedSpeechRecognizer,
        translator: &dyn PreparedTranslator,
        language: SourceLanguage,
        rate: PreparedSampleRate,
        quality: TranslationQuality,
        glossary: Option<&SessionGlossary>,
    ) -> PreparedCompletion {
        let result = self
            .recognize_remote(recognizer, translator, language, rate, quality, glossary)
            .await;
        self.complete(result)
    }
    async fn recognize_remote(
        &mut self,
        recognizer: &dyn PreparedSpeechRecognizer,
        translator: &dyn PreparedTranslator,
        language: SourceLanguage,
        rate: PreparedSampleRate,
        quality: TranslationQuality,
        glossary: Option<&SessionGlossary>,
    ) -> Result<PreparedDocument, MediaError> {
        check(&self.cancellation)?;
        let request = self.request.as_ref().ok_or(MediaError::InvalidInput)?;
        // Preserve the text-first plan, including ambiguity errors, without
        // opening a URL, reading a sidecar, or asking either provider to work.
        let plan = request.plan()?;
        if !request.remote_subtitles.is_empty() {
            return Err(MediaError::UnsupportedUrl);
        }
        if !request.media.is_remote() {
            return Err(MediaError::InvalidInput);
        }
        if plan != PreparationPlan::AsrRequired {
            return Err(MediaError::UnsupportedUrl);
        }
        let empty = SessionGlossary::new(self.identity);
        let glossary = glossary.unwrap_or(&empty);
        glossary
            .entries_for(self.identity)
            .map_err(translation_error)?;
        let capabilities = std::panic::catch_unwind(AssertUnwindSafe(|| recognizer.capabilities()))
            .map_err(|_| MediaError::Speech(PreparedSpeechError::InvalidData))?;
        capabilities
            .validate(language, rate)
            .map_err(MediaError::Speech)?;
        check(&self.cancellation)?;
        let request = self.request.take().ok_or(MediaError::InvalidInput)?;
        // Lower failures cancel a child, preserving their typed cause rather
        // than accidentally replacing SourceChanged with parent cancellation.
        let scope = ResolverCancellation::linked(self.cancellation.clone())
            .map_err(|_| MediaError::InvalidInput)?;
        let decoder = open_remote_wave(request.media, self.identity, scope.clone()).await?;
        let recognized = recognize_remote_wave(decoder, language, rate, recognizer)
            .await
            .map_err(prepared_error)?;
        let (source, transcript) = recognized.into_parts().map_err(prepared_error)?;
        self.finish_recognized(
            PreparedMediaOwner::Remote(Box::new(source)),
            transcript,
            PreparedTextServices::analyzed(translator),
            quality,
            glossary,
            scope,
        )
        .await
    }
}
/// The same source/client/version gate is shared by audio fallback and selected
/// text. Opening validates bounded headers, without pulling a PCM block.
pub(super) async fn open_remote_wave(
    input: crate::media::input::MediaInput,
    identity: StreamIdentity,
    scope: ResolverCancellation,
) -> Result<RemoteWaveDecoder, MediaError> {
    let media = RemoteHttpMedia::open(input, identity, scope)
        .await
        .map_err(remote_media_error)?;
    let media = probe_remote_audio(media)
        .await
        .map_err(|error| match error {
            RemoteProbeError::Remote(error) => remote_media_error(error),
            RemoteProbeError::Media(error) => error,
        })?;
    RemoteWaveDecoder::open(media).await.map_err(wave_error)
}
fn remote_media_error(error: RemoteMediaError) -> MediaError {
    match error {
        RemoteMediaError::Cancelled => MediaError::Cancelled,
        RemoteMediaError::Stale => MediaError::Stale,
        error => MediaError::Remote(error),
    }
}
fn wave_error(error: RemoteWaveError) -> MediaError {
    match error {
        RemoteWaveError::Remote(error) => remote_media_error(error),
        RemoteWaveError::Media(error) => error,
    }
}
fn prepared_error(error: RemotePreparedError) -> MediaError {
    match error {
        RemotePreparedError::Media(error) => wave_error(error),
        RemotePreparedError::Speech(PreparedSpeechError::Cancelled) => MediaError::Cancelled,
        RemotePreparedError::Speech(error) => MediaError::Speech(error),
    }
}

#[cfg(test)]
#[path = "prepared_remote_tests.rs"]
mod tests;
