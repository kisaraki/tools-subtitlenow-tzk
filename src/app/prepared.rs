//! One caller-owned Prepared worker; RAM documents remain on the UI owner.
use crate::{
    ai::{
        credentials::{ApiKey, CredentialStore, WindowsCredentialStore},
        prepared_glossary::PreparedTranslator,
        prepared_speech::{PreparedSpeechError, PreparedSpeechRecognizer},
        prepared_speech_http::HttpPreparedRecognizer,
        provider::SpeechProvider,
        translation_http::HttpTranslator,
    },
    domain::SourceLanguage,
    media::{MediaError, planning::PreparedRequest, prepared_audio::PreparedSampleRate},
    resolver::contracts::TextLanguage,
    session::prepared::{PreparedCompletion, PreparedDocument, PreparedSession},
    ui::prepared::{PreparedBackend, PreparedOptions, PreparedUiError, PreparedUiState},
};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc,
        mpsc::{self, Receiver},
    },
    thread::JoinHandle,
};

/// Construct only the service selected by this explicit Prepare action. The
/// injectable seam never prompts, chooses a fallback, or starts capture.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PreparedServiceKind {
    Translation,
    Recognition,
}
enum PreparedServices {
    Translation(Box<dyn PreparedTranslator>),
    Recognition {
        recognizer: Box<dyn PreparedSpeechRecognizer>,
        translator: Box<dyn PreparedTranslator>,
    },
}
trait SelectedPreparedServiceFactory: Send + Sync {
    fn build(
        &self,
        provider: SpeechProvider,
        key: Option<ApiKey>,
        kind: PreparedServiceKind,
    ) -> Result<PreparedServices, PreparedUiError>;
}
struct HttpPreparedServiceFactory;
impl SelectedPreparedServiceFactory for HttpPreparedServiceFactory {
    fn build(
        &self,
        provider: SpeechProvider,
        key: Option<ApiKey>,
        kind: PreparedServiceKind,
    ) -> Result<PreparedServices, PreparedUiError> {
        Self::build_with_store(provider, key, kind, &WindowsCredentialStore)
    }
}
impl HttpPreparedServiceFactory {
    fn build_with_store(
        provider: SpeechProvider,
        key: Option<ApiKey>,
        kind: PreparedServiceKind,
        store: &dyn CredentialStore,
    ) -> Result<PreparedServices, PreparedUiError> {
        // Resolve the selected key exactly once. Both ASR and translation use
        // this same credential and provider; construction sends no requests.
        let key = match key {
            Some(key) => key,
            None => store.read(provider).map_err(PreparedUiError::Credential)?,
        };
        let translator =
            Box::new(HttpTranslator::new(provider, &key).map_err(PreparedUiError::Translation)?);
        match kind {
            PreparedServiceKind::Translation => Ok(PreparedServices::Translation(translator)),
            PreparedServiceKind::Recognition => {
                let recognizer = Box::new(
                    HttpPreparedRecognizer::new(provider, &key).map_err(PreparedUiError::Speech)?,
                );
                Ok(PreparedServices::Recognition {
                    recognizer,
                    translator,
                })
            }
        }
    }
}
type WorkerResult = Result<PreparedCompletion, PreparedUiError>;
/// One final completion per caller-owned worker, with no history queue.
const COMPLETION_QUEUE_ITEMS: usize = 1;
struct PreparedWorker {
    thread: JoinHandle<()>,
    result: Receiver<WorkerResult>,
}
pub struct PreparedController {
    session: PreparedSession,
    worker: Option<PreparedWorker>,
    state: PreparedUiState,
    factory: Arc<dyn SelectedPreparedServiceFactory>,
}
impl Default for PreparedController {
    fn default() -> Self {
        Self::new()
    }
}
impl PreparedController {
    pub fn from_attachment(attachment: crate::session::prepared::PreparedAttachment) -> Self {
        let cues = attachment.cue_count();
        let mut controller = Self::new();
        controller.session = attachment.into_session();
        controller.state = PreparedUiState::Ready { cues };
        controller
    }
    pub fn new() -> Self {
        Self::with_factory(Arc::new(HttpPreparedServiceFactory))
    }
    fn with_factory(factory: Arc<dyn SelectedPreparedServiceFactory>) -> Self {
        Self {
            session: PreparedSession::new(),
            worker: None,
            state: PreparedUiState::Idle,
            factory,
        }
    }
    /// This borrow never leaves the owner or creates a second timeline.
    pub fn ready(&self) -> Option<&PreparedDocument> {
        self.session.ready()
    }
    /// Explicit metadata/proposal library entry; does not start capture/playback.
    pub fn fuzzy_locator(
        &self,
        capture_identity: crate::ai::speech::StreamIdentity,
        epoch: u64,
    ) -> Result<
        crate::subtitle::fuzzy_worker::FuzzyCueLocator,
        crate::subtitle::fuzzy_alignment::FuzzyAlignmentError,
    > {
        self.session.fuzzy_locator(capture_identity, epoch)
    }
    fn fail(&mut self, error: PreparedUiError) -> PreparedUiError {
        self.session.stop();
        self.state = PreparedUiState::Failed(error);
        error
    }
    fn reap(&mut self) {
        if !self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.thread.is_finished())
        {
            return;
        }
        let Some(worker) = self.worker.take() else {
            return;
        };
        let joined = worker.thread.join();
        // Stop has already retired the identity and cleared the RAM document.
        // A completion buffered just before Stop must not publish Ready.
        if self.state == PreparedUiState::Stopping {
            self.state = PreparedUiState::Cancelled;
            return;
        }
        if joined.is_err() {
            self.fail(PreparedUiError::WorkerFailed);
            return;
        }
        match worker.result.try_recv() {
            Ok(Ok(completion)) => match self.session.apply(completion) {
                Ok(true) => {
                    let Some(document) = self.session.ready() else {
                        self.fail(PreparedUiError::WorkerFailed);
                        return;
                    };
                    self.state = PreparedUiState::Ready {
                        cues: document.cue_count(),
                    };
                }
                Ok(false) => {
                    self.fail(PreparedUiError::WorkerFailed);
                }
                Err(MediaError::Cancelled) => {
                    self.state = PreparedUiState::Cancelled;
                }
                Err(MediaError::Speech(error)) => {
                    self.fail(PreparedUiError::Speech(error));
                }
                Err(MediaError::Translation(error)) => {
                    self.fail(PreparedUiError::Translation(error));
                }
                Err(error) => {
                    self.fail(PreparedUiError::Media(error));
                }
            },
            Ok(Err(error)) => {
                self.fail(error);
            }
            Err(_) => {
                self.fail(PreparedUiError::WorkerFailed);
            }
        }
    }
}
impl PreparedController {
    fn start_internal(
        &mut self,
        request: PreparedRequest,
        options: PreparedOptions,
        key: Option<ApiKey>,
        allow_https_subtitles: bool,
    ) -> Result<(), PreparedUiError> {
        // A stopped or completed thread must first be reaped by Poll. This
        // keeps a controller from owning overlapping runtimes or file workers.
        if self.worker.is_some() {
            return Err(PreparedUiError::Busy);
        }
        let language = request
            .text_language()
            .map_err(|error| self.fail(PreparedUiError::Media(error)))?;
        let remote = request.media.is_remote();
        // The ordinary entry keeps its local/audio destination contract. Only
        // the explicit entry opts into disclosed HTTPS subtitle candidates.
        // Metadata ambiguity above keeps priority over either entry guard.
        let has_https_subtitles = !request.remote_subtitles.is_empty();
        if !allow_https_subtitles && has_https_subtitles {
            return Err(self.fail(PreparedUiError::Media(MediaError::UnsupportedUrl)));
        }
        // Remote WAVE accepts explicit local sidecars, never an embedded MP4
        // choice, even when a preferred sidecar wins. Metadata planning above
        // keeps ambiguity precedence before credentials, factories or I/O.
        if remote && request.embedded.is_some() {
            return Err(self.fail(PreparedUiError::Media(MediaError::UnsupportedUrl)));
        }
        if allow_https_subtitles && !has_https_subtitles {
            return Err(self.fail(PreparedUiError::Media(MediaError::InvalidInput)));
        }
        let kind = match language {
            Some(TextLanguage::Source(_)) => Some(PreparedServiceKind::Translation),
            Some(_) => None,
            None if allow_https_subtitles => {
                return Err(self.fail(PreparedUiError::Media(MediaError::InvalidInput)));
            }
            None => {
                // Official recognizers cannot constrain Auto to the five source
                // languages. Reject before credentials, factory or file I/O.
                if options.source_language == SourceLanguage::Auto {
                    return Err(self.fail(PreparedUiError::Speech(
                        PreparedSpeechError::UnsupportedLanguage,
                    )));
                }
                Some(PreparedServiceKind::Recognition)
            }
        };
        let provider = if kind.is_some() {
            Some(
                options
                    .provider
                    .ok_or_else(|| self.fail(PreparedUiError::ProviderRequired))?,
            )
        } else {
            None
        };
        let key = if kind.is_some() {
            key
        } else {
            drop(key);
            None
        };
        let task = self
            .session
            .start(request)
            .map_err(|error| self.fail(PreparedUiError::Media(error)))?;
        let factory = self.factory.clone();
        let (sender, result) = mpsc::sync_channel(COMPLETION_QUEUE_ITEMS);
        let spawned = std::thread::Builder::new()
            .name("subtitle-prepared".into())
            .spawn(move || {
                let completion = catch_unwind(AssertUnwindSafe(|| {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|_| PreparedUiError::WorkerStart)?;
                    if task.is_cancelled() {
                        return Err(PreparedUiError::Media(MediaError::Cancelled));
                    }
                    let services = match (provider, kind) {
                        (Some(provider), Some(kind)) => {
                            let services = factory.build(provider, key, kind)?;
                            // A faulty injectable factory must not quietly run
                            // translation alone or perform unrequested ASR.
                            let matches = matches!(
                                (&services, kind),
                                (
                                    PreparedServices::Translation(_),
                                    PreparedServiceKind::Translation
                                ) | (
                                    PreparedServices::Recognition { .. },
                                    PreparedServiceKind::Recognition
                                )
                            );
                            if !matches {
                                return Err(PreparedUiError::WorkerFailed);
                            }
                            Some(services)
                        }
                        (None, None) => None,
                        _ => return Err(PreparedUiError::WorkerFailed),
                    };
                    if task.is_cancelled() {
                        return Err(PreparedUiError::Media(MediaError::Cancelled));
                    }
                    Ok(runtime.block_on(async {
                        match services {
                            Some(PreparedServices::Translation(translator)) => {
                                if allow_https_subtitles {
                                    task.run_with_https_subtitles(
                                        Some(translator.as_ref()),
                                        options.quality,
                                        None,
                                    )
                                    .await
                                } else if remote {
                                    task.run_with_remote_wave_subtitle(
                                        Some(translator.as_ref()),
                                        options.quality,
                                        None,
                                    )
                                    .await
                                } else {
                                    task.run_with_analyzed_translator(
                                        translator.as_ref(),
                                        options.quality,
                                        None,
                                    )
                                    .await
                                }
                            }
                            Some(PreparedServices::Recognition {
                                recognizer,
                                translator,
                            }) => {
                                if remote {
                                    task.run_with_remote_wave_recognizer(
                                        recognizer.as_ref(),
                                        translator.as_ref(),
                                        options.source_language,
                                        PreparedSampleRate::Hz16000,
                                        options.quality,
                                        None,
                                    )
                                    .await
                                } else {
                                    task.run_with_analyzed_recognizer(
                                        recognizer.as_ref(),
                                        translator.as_ref(),
                                        options.source_language,
                                        PreparedSampleRate::Hz16000,
                                        options.quality,
                                        None,
                                    )
                                    .await
                                }
                            }
                            None => {
                                if allow_https_subtitles {
                                    task.run_with_https_subtitles(None, options.quality, None)
                                        .await
                                } else if remote {
                                    task.run_with_remote_wave_subtitle(None, options.quality, None)
                                        .await
                                } else {
                                    task.run().await
                                }
                            }
                        }
                    }))
                    // Runtime Drop joins any started bounded blocking work before
                    // this outer thread becomes finished and can be reaped.
                }))
                .unwrap_or(Err(PreparedUiError::WorkerFailed));
                // Exactly one result is produced. Dropping the receiver releases
                // this document instead of leaving a detached result owner.
                let _ = sender.send(completion);
            });
        match spawned {
            Ok(thread) => {
                self.worker = Some(PreparedWorker { thread, result });
                self.state = PreparedUiState::Preparing;
                Ok(())
            }
            Err(_) => Err(self.fail(PreparedUiError::WorkerStart)),
        }
    }
}
impl PreparedBackend for PreparedController {
    fn take_ready(
        &mut self,
    ) -> Result<crate::session::prepared::PreparedAttachment, PreparedUiError> {
        self.reap();
        if self.worker.is_some() {
            return Err(PreparedUiError::Busy);
        }
        self.session.alignment_source().map_err(|error| {
            if error == crate::subtitle::fuzzy_alignment::FuzzyAlignmentError::UnsupportedEvidence {
                PreparedUiError::ReferenceRequired
            } else {
                error.into()
            }
        })?;
        let session = std::mem::take(&mut self.session);
        self.state = PreparedUiState::Idle;
        Ok(session.into_attachment())
    }
    fn start(
        &mut self,
        request: PreparedRequest,
        options: PreparedOptions,
        key: Option<ApiKey>,
    ) -> Result<(), PreparedUiError> {
        self.start_internal(request, options, key, false)
    }
    fn start_with_https_subtitles(
        &mut self,
        request: PreparedRequest,
        options: PreparedOptions,
        key: Option<ApiKey>,
    ) -> Result<(), PreparedUiError> {
        self.start_internal(request, options, key, true)
    }
    fn poll(&mut self) -> PreparedUiState {
        self.reap();
        self.state
    }
    fn stop(&mut self) {
        self.session.stop();
        self.state = if self.worker.is_some() {
            PreparedUiState::Stopping
        } else {
            PreparedUiState::Cancelled
        };
    }
    fn state(&self) -> PreparedUiState {
        self.state
    }
}
impl Drop for PreparedController {
    fn drop(&mut self) {
        self.session.stop();
        if let Some(worker) = self.worker.take() {
            // Native close first polls Stopping, so normal UI destruction only
            // reaches this safety net after the owned thread has already exited.
            let _ = worker.thread.join();
        }
    }
}

#[cfg(test)]
#[path = "prepared_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "prepared_asr_tests.rs"]
mod asr_tests;
