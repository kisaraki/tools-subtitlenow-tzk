//! Explicit Prepared network-subtitle work. No GUI, credentials, provider
//! selection, ASR fallback or invented media/version/alignment evidence.
use super::*;
use crate::media::remote_subtitle::{RemoteSubtitleError, fetch_remote_subtitle};

impl PreparedTask {
    /// This entry explicitly opts in to the selected HTTPS subtitle location.
    /// Chinese text stays local; source text uses only the supplied analyzed
    /// translator. Media is pinned before subtitle HTTP/analysis waits and no
    /// audio is requested.
    pub async fn run_with_https_subtitles(
        mut self,
        provider: Option<&dyn PreparedTranslator>,
        quality: TranslationQuality,
        glossary: Option<&SessionGlossary>,
    ) -> PreparedCompletion {
        let result = self.https_subtitles(provider, quality, glossary).await;
        self.complete(result)
    }

    async fn https_subtitles(
        &mut self,
        provider: Option<&dyn PreparedTranslator>,
        quality: TranslationQuality,
        glossary: Option<&SessionGlossary>,
    ) -> Result<PreparedDocument, MediaError> {
        check(&self.cancellation)?;
        let request = self.request.as_ref().ok_or(MediaError::InvalidInput)?;
        let plan = request.plan()?;
        // Remote media never supports any selected embedded track, even if a
        // higher-priority external subtitle would win the metadata-only plan.
        if request.media.is_remote() && request.embedded.is_some() {
            return Err(MediaError::UnsupportedUrl);
        }
        if request.remote_subtitles.is_empty() {
            return Err(MediaError::InvalidInput);
        }
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
        if let PreparationPlan::RemoteSubtitle(index) = plan {
            let mut request = self.request.take().ok_or(MediaError::InvalidInput)?;
            if index >= request.remote_subtitles.len() {
                return Err(MediaError::InvalidInput);
            }
            // Move only the winner; the remaining private locations are dropped
            // without opening them or copying any subtitle document.
            let selected = request.remote_subtitles.swap_remove(index);
            let reference_selection = request.reference.take();
            request.remote_subtitles.clear();
            // Retain the actual media source before waiting for a subtitle
            // destination. A missing/unsupported source never starts that GET.
            let media = if request.media.is_remote() {
                let scope = ResolverCancellation::linked(self.cancellation.clone())
                    .map_err(|_| MediaError::InvalidInput)?;
                PreparedMediaOwner::Remote(Box::new(
                    super::remote::open_remote_wave(request.media, self.identity, scope).await?,
                ))
            } else {
                self.blocking(move |cancel| open_media(&request.media, cancel))
                    .await?
                    .into()
            };
            let fetched = fetch_remote_subtitle(
                selected,
                self.identity,
                self.sequence,
                self.cancellation.clone(),
            )
            .await
            .map_err(RemoteSubtitleError::into_media)?;
            // No await or fallible step separates this move from rebuilding
            // co-ownership, including original-before-lease cleanup ordering.
            let (original, lease) = fetched
                .into_parts()
                .map_err(RemoteSubtitleError::into_media)?;
            let mut loaded = LoadedDocument {
                reference: None,
                quality: None,
                media,
                original,
                subtitle_lease: Some(lease),
            };
            if reference_selection.is_some() {
                loaded = self
                    .blocking(move |cancel| {
                        loaded.reference =
                            load_reference(reference_selection.as_ref(), &loaded.original, cancel)?;
                        Ok(loaded)
                    })
                    .await?;
            }
            return match provider {
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
                    let identity = self.identity;
                    let sequence = self.sequence;
                    self.blocking(move |cancel| {
                        finish(loaded, Vec::new(), identity, sequence, cancel)
                    })
                    .await
                }
            };
        }
        if !matches!(
            plan,
            PreparationPlan::Subtitle(_) | PreparationPlan::EmbeddedMp4
        ) {
            return Err(MediaError::InvalidSubtitle);
        }
        // The common plan has already selected a higher-priority local/embedded
        // document. Explicitly discard only losing network choices, then reuse
        // the existing text pipeline with its original media and cancellation.
        let request = self.request.as_mut().ok_or(MediaError::InvalidInput)?;
        request.remote_subtitles.clear();
        if request.media.is_remote() {
            self.remote_subtitle(provider, quality, glossary).await
        } else {
            match provider {
                Some(provider) => {
                    self.translate(PreparedTextServices::analyzed(provider), quality, glossary)
                        .await
                }
                None => self.process(prepare).await,
            }
        }
    }
}
