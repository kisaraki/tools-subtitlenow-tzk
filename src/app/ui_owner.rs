//! Own one backend thread. Native views run on the caller's main thread.
use crate::{
    error::{AppError, Result},
    ui::bridge::{self, UiCancellation, UiInbox, UiPublisher, UiStatus},
};
use std::thread::JoinHandle;

pub struct UiWorker {
    thread: Option<JoinHandle<Result<()>>>,
    cancel: UiCancellation,
}
impl UiWorker {
    pub fn request_stop(&self) {
        self.cancel.cancel();
    }
    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }
    pub fn start(
        publisher: UiPublisher,
        work: impl FnOnce(&mut UiPublisher) -> Result<()> + Send + 'static,
    ) -> Result<Self> {
        let cancel = publisher.cancellation();
        let thread = std::thread::Builder::new()
            .name("subtitle-live".into())
            .spawn(move || {
                let mut publisher = publisher;
                let result =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&mut publisher)))
                        .unwrap_or(Err(AppError::LiveWorker));
                let cancelled = publisher.cancellation().is_cancelled();
                let result = match result {
                    Err(AppError::UiNotification(bridge::UiBridgeError::Closed)) if cancelled => {
                        Ok(())
                    }
                    result => result,
                };
                let _ = publisher.finish(if result.is_ok() {
                    UiStatus::Stopped
                } else {
                    UiStatus::Error
                });
                result
            })
            .map_err(|_| AppError::LiveWorker)?;
        Ok(Self {
            thread: Some(thread),
            cancel,
        })
    }
    pub fn stop_join(&mut self) -> Result<()> {
        self.cancel.cancel();
        match self.thread.take() {
            Some(thread) => thread.join().map_err(|_| AppError::LiveWorker)?,
            None => Ok(()),
        }
    }
}

#[cfg(windows)]
pub(super) struct LiveController {
    prepared_source: Option<crate::subtitle::fuzzy_worker::AlignmentSource>,
    discovery: crate::audio::discovery::DiscoveryMonitor,
    sources: std::sync::Arc<Vec<crate::ui::controls::SourceChoice>>,
    worker: Option<UiWorker>,
    notice: Option<String>,
    discovery_notice: Option<String>,
    content: Option<super::content::ContentConfiguration>,
}
#[cfg(windows)]
impl LiveController {
    fn new(content: Option<super::content::ContentConfiguration>) -> Result<Self> {
        Ok(Self {
            prepared_source: None,
            discovery: crate::audio::discovery::DiscoveryMonitor::start(
                crate::win32::audio::WindowsSessionScanner::new,
            )?,
            sources: std::sync::Arc::new(Vec::new()),
            worker: None,
            notice: Some("請明確選擇程序及服務；啟動後音訊與文字將送至所選雲端。".into()),
            discovery_notice: None,
            content,
        })
    }
    fn reap(&mut self) {
        if self.worker.as_ref().is_some_and(UiWorker::is_finished)
            && let Some(mut worker) = self.worker.take()
        {
            let result = worker.stop_join();
            if let Err(error) = result {
                self.notice = Some(error.to_string());
            }
        }
    }
}
#[cfg(windows)]
impl crate::ui::controls::UiController for LiveController {
    fn start_prepared(
        &mut self,
        request: crate::ui::controls::StartRequest,
        key: Option<crate::ai::credentials::ApiKey>,
        source: crate::subtitle::fuzzy_worker::AlignmentSource,
    ) -> Result<UiInbox> {
        if self.worker.is_some() || request.subtitle_source.is_some() || self.content.is_some() {
            return Err(bridge::UiBridgeError::InvalidState.into());
        }
        self.prepared_source = Some(source);
        let result = self.start(request, key);
        self.prepared_source = None;
        result
    }
    fn poll(&mut self) -> crate::ui::controls::ControlUpdate {
        self.reap();
        match self.discovery.poll() {
            Ok(Some(update)) => match &update.result {
                Ok(report) => {
                    let next = crate::ui::controls::choices(report);
                    if self.sources.as_ref() != &next {
                        self.sources = std::sync::Arc::new(next);
                    }
                    self.discovery_notice = if report.is_partial()
                        || report.sources.iter().any(|source| {
                            source.process_scope
                                == crate::audio::sessions::ProcessScope::SingleProcess
                                && source.process_id != std::process::id()
                                && source.process_created.is_none_or(|created| created == 0)
                        }) {
                        Some("部分來源無法獨立驗證，未列入清單；不會自動改抓其他來源。".into())
                    } else {
                        None
                    };
                }
                Err(error) => {
                    self.sources = std::sync::Arc::new(Vec::new());
                    self.discovery_notice = Some(error.to_string());
                }
            },
            Err(error) => {
                self.sources = std::sync::Arc::new(Vec::new());
                self.discovery_notice = Some(error.to_string());
            }
            Ok(None) => {}
        }
        crate::ui::controls::ControlUpdate {
            sources: self.sources.clone(),
            busy: self.worker.is_some(),
            notice: self
                .discovery_notice
                .clone()
                .or_else(|| self.notice.clone()),
        }
    }
    fn start(
        &mut self,
        request: crate::ui::controls::StartRequest,
        key: Option<crate::ai::credentials::ApiKey>,
    ) -> Result<UiInbox> {
        self.reap();
        if self.worker.is_some()
            || !self.sources.iter().any(|source| {
                source.id == request.source.id
                    && source.pid == request.source.pid
                    && source.created == request.source.created
            })
        {
            return Err(bridge::UiBridgeError::InvalidState.into());
        }
        let pinned = crate::win32::capture::ProcessTarget::open(request.source.pid)?;
        pinned.verify_creation_time(request.source.created)?;
        let content = match request.subtitle_source.as_ref() {
            Some(_) if self.content.is_some() => {
                // An explicitly configured library service cannot be silently
                // replaced by another destination from the GUI.
                return Err(
                    crate::resolver::contracts::ResolverError::UnsupportedCapability.into(),
                );
            }
            Some(settings) => Some(super::known_subtitle::configuration(settings)?),
            None => self.content.clone(),
        };
        let source_notice = request.subtitle_source.as_ref().map(|settings| {
            format!(
                "字幕下載：{}。辨識文字在本機核對；音訊與翻譯仍使用所選 AI 服務。",
                settings.destination()
            )
        });
        let options = super::live::LiveOptions {
            pid: request.source.pid,
            provider: request.provider,
            live_model: request.live_model,
            language: request.language,
            quality: Some(request.quality),
            content,
        };
        let prepared_source = self.prepared_source.take();
        let (publisher, inbox) = bridge::channel();
        self.worker = Some(UiWorker::start(publisher, move |publisher| {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|_| AppError::RuntimeInitialization)?;
            runtime.block_on(async {
                publisher.prepare_source_with_creation(
                    bridge::UiSource {
                        pid: options.pid,
                        provider: options.provider,
                    },
                    pinned.creation_time(),
                )?;
                publisher.prepare_live_model(options.live_model.unwrap_or_else(|| {
                    crate::ai::provider::LiveSpeechModel::default_for(options.provider)
                }))?;
                publisher.prepare_content_with_movie_hints(
                    options.content.as_ref().map(|config| config.disclosure()),
                    options
                        .content
                        .as_ref()
                        .is_some_and(super::content::ContentConfiguration::movie_dialogue_enabled),
                )?;
                publisher.prepare_lyrics(
                    options
                        .content
                        .as_ref()
                        .and_then(|config| config.lyrics_disclosure()),
                )?;
                publisher.prepare_subtitles(
                    options
                        .content
                        .as_ref()
                        .and_then(|config| config.subtitles_disclosure()),
                )?;
                if prepared_source.is_some() {
                    publisher.prepare_alignment()?;
                }
                publisher.wait_until_visible().await?;
                match prepared_source {
                    Some(source) => {
                        super::live::run_to_window_prepared(options, pinned, key, publisher, source)
                            .await
                    }
                    None => super::live::run_to_window(options, pinned, key, publisher).await,
                }
            })
        })?);
        self.notice = source_notice;
        Ok(inbox)
    }
    fn save_preferences(
        &mut self,
        preferences: crate::ui::controls::ControlPreferences,
    ) -> Result<()> {
        persist_control_preferences(&super::config::default_path()?, preferences)
    }
    fn save_key(
        &mut self,
        provider: crate::ai::provider::SpeechProvider,
        key: crate::ai::credentials::ApiKey,
    ) -> Result<()> {
        // Only reached from the named provider's explicit Save button. WinCred
        // copies the value securely; this owned RAM copy is wiped on return.
        crate::ai::credentials::WindowsCredentialStore.write_verified(provider, &key)?;
        Ok(())
    }
    fn check_key(&mut self, provider: crate::ai::provider::SpeechProvider) -> Result<()> {
        use crate::ai::credentials::{CredentialStore, WindowsCredentialStore};
        // Only the named Check button reads this provider's key. The short-lived
        // validated copy is wiped on drop; UI receives only success/typed failure.
        drop(WindowsCredentialStore.read(provider)?);
        Ok(())
    }
    fn request_stop(&mut self) {
        if let Some(worker) = &self.worker {
            worker.request_stop();
        }
    }
}

#[cfg(windows)]
fn persist_control_preferences(
    path: &std::path::Path,
    preferences: crate::ui::controls::ControlPreferences,
) -> Result<()> {
    if preferences.provider.is_none() {
        return Err(crate::ai::speech::SpeechError::NotConfigured.into());
    }
    let mut config = super::config::AppConfig::load(path)?;
    config.speech_provider = preferences.provider;
    config.live_speech_model = preferences.live_model;
    config.source_language = preferences.language;
    config.translation_quality = preferences.quality;
    config.save(path)
}
impl Drop for UiWorker {
    fn drop(&mut self) {
        let _ = self.stop_join();
    }
}
/// Even view initialization failure cancels and joins the already-owned backend.
pub fn drive_owned_view(
    work: impl FnOnce(&mut UiPublisher) -> Result<()> + Send + 'static,
    view: impl FnOnce(UiInbox) -> Result<()>,
) -> Result<()> {
    let (publisher, inbox) = bridge::channel();
    let mut worker = UiWorker::start(publisher, work)?;
    let view_result = view(inbox);
    let worker_result = worker.stop_join();
    view_result.and(worker_result)
}

/// Explicit composition entry for a configured content service. The ordinary
/// --ui command passes None; selecting OpenAI/Gemini does not configure a resolver.
#[cfg(windows)]
pub fn run_controlled_with_content(
    mode: crate::domain::DisplayMode,
    preferences: crate::ui::controls::ControlPreferences,
    content: Option<super::content::ContentConfiguration>,
) -> Result<()> {
    crate::win32::subtitle_window::run_controlled(
        mode,
        preferences,
        Box::new(LiveController::new(content)?),
    )
}

#[cfg(windows)]
pub(super) fn run(
    options: super::live::LiveOptions,
    interactive: bool,
    mode: crate::domain::DisplayMode,
) -> Result<()> {
    use std::io::IsTerminal;
    // Prompt on the caller before creating windows/workers: no hidden blocking key
    // prompt remains after UI closure, and no prompt is started by default.
    if interactive && (!std::io::stdout().is_terminal() || !std::io::stdin().is_terminal()) {
        return Err(AppError::ConsoleOutput);
    }
    let pinned = crate::win32::capture::ProcessTarget::open(options.pid)?;
    let key = if interactive {
        Some(super::live::prompt_interactive_key(options.provider)?)
    } else {
        None
    };
    drive_owned_view(
        move |publisher| {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|_| AppError::RuntimeInitialization)?;
            runtime.block_on(async {
                publisher.prepare_source_with_creation(
                    bridge::UiSource {
                        pid: options.pid,
                        provider: options.provider,
                    },
                    pinned.creation_time(),
                )?;
                publisher.prepare_live_model(options.live_model.unwrap_or_else(|| {
                    crate::ai::provider::LiveSpeechModel::default_for(options.provider)
                }))?;
                publisher.prepare_content_with_movie_hints(
                    options.content.as_ref().map(|config| config.disclosure()),
                    options
                        .content
                        .as_ref()
                        .is_some_and(super::content::ContentConfiguration::movie_dialogue_enabled),
                )?;
                publisher.prepare_lyrics(
                    options
                        .content
                        .as_ref()
                        .and_then(|config| config.lyrics_disclosure()),
                )?;
                publisher.prepare_subtitles(
                    options
                        .content
                        .as_ref()
                        .and_then(|config| config.subtitles_disclosure()),
                )?;
                publisher.wait_until_visible().await?;
                super::live::run_to_window(options, pinned, key, publisher).await
            })
        },
        move |inbox| crate::win32::subtitle_window::run_with_inbox(mode, inbox),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    #[cfg(windows)]
    #[test]
    fn phase11_saved_service_preferences_round_trip_without_keys_or_source_identity() {
        use crate::{
            ai::provider::SpeechProvider,
            domain::{DisplayMode, SourceLanguage, TranslationQuality},
            ui::controls::ControlPreferences,
        };
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preferences.json");
        let original = super::super::config::AppConfig {
            display_mode: DisplayMode::Dual,
            ..Default::default()
        };
        original.save(&path).unwrap();
        let preferences = ControlPreferences {
            provider: Some(SpeechProvider::Gemini),
            live_model: None,
            language: SourceLanguage::English,
            quality: TranslationQuality::Fast,
        };
        persist_control_preferences(&path, preferences).unwrap();
        let saved = super::super::config::AppConfig::load(&path).unwrap();
        assert_eq!(saved.speech_provider, preferences.provider);
        assert_eq!(saved.source_language, preferences.language);
        assert_eq!(saved.translation_quality, preferences.quality);
        assert_eq!(saved.display_mode, DisplayMode::Dual);
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(json.as_object().unwrap().len(), 5);
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
        let previous = std::fs::read(&path).unwrap();
        assert!(
            persist_control_preferences(
                &path,
                ControlPreferences {
                    provider: None,
                    ..preferences
                }
            )
            .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), previous);
        std::fs::write(&path, b"{\"schema_version\":999}").unwrap();
        let unsupported = std::fs::read(&path).unwrap();
        assert!(persist_control_preferences(&path, preferences).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), unsupported);
    }
    struct Released(Arc<AtomicBool>);
    impl Drop for Released {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    #[test]
    fn view_failure_cancels_and_joins_pending_owner() {
        let released = Arc::new(AtomicBool::new(false));
        let done = released.clone();
        let result = drive_owned_view(
            move |publisher| {
                let _guard = Released(done);
                let cancel = publisher.cancellation();
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(cancel.cancelled());
                Ok(())
            },
            |_inbox| Err(AppError::SubtitleWindow),
        );
        assert!(matches!(result, Err(AppError::SubtitleWindow)));
        assert!(released.load(Ordering::SeqCst));
    }
    #[test]
    fn owner_drop_joins_and_clears_pending_resources() {
        let (publisher, _inbox) = bridge::channel();
        let released = Arc::new(AtomicBool::new(false));
        let done = released.clone();
        let owner = UiWorker::start(publisher, move |publisher| {
            let _guard = Released(done);
            let cancel = publisher.cancellation();
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(cancel.cancelled());
            Ok(())
        })
        .unwrap();
        drop(owner);
        assert!(released.load(Ordering::SeqCst));
    }
    #[test]
    fn backend_panic_has_safe_error_and_visible_error_state() {
        let (publisher, mut inbox) = bridge::channel();
        let mut owner = UiWorker::start(publisher, |_| panic!("synthetic worker panic")).unwrap();
        assert!(matches!(
            owner.thread.take().unwrap().join().unwrap(),
            Err(AppError::LiveWorker)
        ));
        assert!(inbox.poll().unwrap().status == UiStatus::Error);
    }
}
