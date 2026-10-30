//! Application composition; subsystem implementations remain independent.

pub mod config;
pub mod content;
pub mod known_subtitle;
#[cfg(windows)]
mod live;
pub(crate) mod live_alignment;
pub mod lyrics;
pub mod lyrics_lrc;
pub mod movie;
pub(crate) mod performance;
pub mod prepared;
pub mod prepared_discovery;
#[cfg(windows)]
mod recovery;
pub(crate) mod source_metadata;
pub mod state;
pub mod subtitles;
pub mod subtitles_http;
pub mod text_metadata;
pub mod ui_owner;

use crate::error::Result;

#[cfg(windows)]
pub fn run_desktop() -> Result<()> {
    let _running = crate::win32::desktop::RunningGuard::new()?;
    let config = config::AppConfig::load(&config::default_path()?)?;
    let preferences = crate::ui::controls::ControlPreferences {
        provider: config.speech_provider,
        live_model: config.live_speech_model,
        language: config.source_language,
        quality: config.translation_quality,
    };
    ui_owner::run_controlled_with_content(config.display_mode, preferences, None)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Command {
    Status,
    Help,
    Ui,
    ListSources,
    WatchSources,
    Capture(u32),
    Speech(
        u32,
        Option<crate::ai::provider::SpeechProvider>,
        Option<crate::domain::SourceLanguage>,
        bool,
    ),
    Translate(
        u32,
        Option<crate::ai::provider::SpeechProvider>,
        Option<crate::domain::SourceLanguage>,
        Option<crate::domain::TranslationQuality>,
        bool,
    ),
    WindowTranslation(
        u32,
        Option<crate::ai::provider::SpeechProvider>,
        Option<crate::domain::SourceLanguage>,
        Option<crate::domain::TranslationQuality>,
        bool,
    ),
    SetKey(crate::ai::provider::SpeechProvider),
    CheckKey(crate::ai::provider::SpeechProvider),
}

fn provider_argument(
    value: Option<std::ffi::OsString>,
) -> Result<crate::ai::provider::SpeechProvider> {
    use crate::ai::provider::SpeechProvider;
    match value.as_deref().and_then(|value| value.to_str()) {
        Some("openai") => Ok(SpeechProvider::OpenAi),
        Some("gemini") => Ok(SpeechProvider::Gemini),
        _ => Err(crate::error::AppError::InvalidArguments),
    }
}

fn parse_arguments(arguments: impl IntoIterator<Item = std::ffi::OsString>) -> Result<Command> {
    let mut arguments = arguments.into_iter();
    let Some(argument) = arguments.next() else {
        return Ok(Command::Status);
    };
    let command = match argument.to_str() {
        Some("--help" | "-h") => Command::Help,
        Some("--ui") => Command::Ui,
        Some("--list-sources") => Command::ListSources,
        Some("--watch-sources") => Command::WatchSources,
        Some("--set-stt-key") => Command::SetKey(provider_argument(arguments.next())?),
        Some("--check-stt-key") => Command::CheckKey(provider_argument(arguments.next())?),
        Some("--capture-pid" | "--stt-pid" | "--translate-pid" | "--translate-window-pid") => {
            let speech = argument != "--capture-pid";
            let window = argument == "--translate-window-pid";
            let translate = argument == "--translate-pid" || window;
            let pid = arguments
                .next()
                .and_then(|value| {
                    value.to_str().and_then(|value| {
                        if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                            return None;
                        }
                        value.parse::<u32>().ok().filter(|pid| *pid > 0)
                    })
                })
                .ok_or(crate::error::AppError::InvalidArguments)?;
            if speech {
                let mut provider = None;
                let mut language = None;
                let mut interactive_key = false;
                let mut quality = None;
                while let Some(flag) = arguments.next() {
                    match flag.to_str() {
                        Some("--quality") if translate && quality.is_none() => {
                            use crate::domain::TranslationQuality;
                            quality = Some(
                                match arguments.next().as_deref().and_then(|value| value.to_str()) {
                                    Some("fast") => TranslationQuality::Fast,
                                    Some("standard") => TranslationQuality::Standard,
                                    Some("high") => TranslationQuality::High,
                                    _ => return Err(crate::error::AppError::InvalidArguments),
                                },
                            );
                        }
                        Some("--interactive-key") if !interactive_key => interactive_key = true,
                        Some("--provider") if provider.is_none() => {
                            provider = Some(provider_argument(arguments.next())?)
                        }
                        Some("--language") if language.is_none() => {
                            use crate::domain::SourceLanguage;
                            language = Some(
                                match arguments.next().as_deref().and_then(|value| value.to_str()) {
                                    Some("auto") => SourceLanguage::Auto,
                                    Some("en") => SourceLanguage::English,
                                    Some("ja") => SourceLanguage::Japanese,
                                    Some("fr") => SourceLanguage::French,
                                    Some("de") => SourceLanguage::German,
                                    Some("ko") => SourceLanguage::Korean,
                                    _ => return Err(crate::error::AppError::InvalidArguments),
                                },
                            );
                        }
                        _ => return Err(crate::error::AppError::InvalidArguments),
                    }
                }
                if window {
                    Command::WindowTranslation(pid, provider, language, quality, interactive_key)
                } else if translate {
                    Command::Translate(pid, provider, language, quality, interactive_key)
                } else {
                    Command::Speech(pid, provider, language, interactive_key)
                }
            } else {
                Command::Capture(pid)
            }
        }
        Some(_) | None => return Err(crate::error::AppError::InvalidArguments),
    };
    if arguments.next().is_some() {
        return Err(crate::error::AppError::InvalidArguments);
    }
    Ok(command)
}

/// Discovery/capture require explicit commands; default startup only reads preferences.
#[cfg(windows)]
pub fn run() -> Result<()> {
    use crate::{domain::TARGET_LANGUAGE, error::AppError, ui::source_list};
    use std::io::Write;

    let command = parse_arguments(std::env::args_os().skip(1))?;
    if command == Command::Help {
        std::io::stdout()
            .write_all(source_list::HELP.as_bytes())
            .map_err(|_| AppError::ConsoleOutput)?;
        return Ok(());
    }
    if let Command::SetKey(provider) | Command::CheckKey(provider) = command {
        use crate::ai::credentials::{ApiKey, CredentialStore, WindowsCredentialStore};
        if matches!(command, Command::SetKey(_)) {
            let text = zeroize::Zeroizing::new(
                rpassword::prompt_password("輸入所選服務的 API 金鑰（不顯示輸入）：")
                    .map_err(|_| crate::ai::credentials::CredentialError::Unavailable)?,
            );
            let key = ApiKey::from_text(&text)?;
            WindowsCredentialStore.write(provider, &key)?;
            writeln!(
                std::io::stdout(),
                "API 金鑰已儲存至此 Windows 使用者的認證管理員。未建立雲端連線。"
            )
            .map_err(|_| AppError::ConsoleOutput)?;
        } else {
            let _key = WindowsCredentialStore.read(provider)?;
            writeln!(
                std::io::stdout(),
                "API 金鑰已設定且可讀取；尚未驗證服務端權限，未建立雲端連線。"
            )
            .map_err(|_| AppError::ConsoleOutput)?;
        }
        return Ok(());
    }

    let config = config::AppConfig::load(&config::default_path()?)?;
    if command == Command::Ui {
        let preferences = crate::ui::controls::ControlPreferences {
            provider: config.speech_provider,
            live_model: config.live_speech_model,
            language: config.source_language,
            quality: config.translation_quality,
        };
        return ui_owner::run_controlled_with_content(config.display_mode, preferences, None);
    }
    tracing::info!(
        phase = 7,
        state = ?state::AppState::Idle,
        source_language = ?config.source_language,
        target_language = TARGET_LANGUAGE,
        translation_quality = ?config.translation_quality,
        display_mode = ?config.display_mode,
        capture_active = false,
        cloud_processing = false,
        content_resolver_configured = false,
        speech_provider = ?config.speech_provider,
        "Phase 7：既有字幕整合開發中，已加入中文本地化核心；內容服務與字幕來源切換尚未啟用。--ui 開啟待命視窗；--translate-window-pid 將指定來源的翻譯送至視窗，--translate-pid 使用終端機。未指定命令時不擷取、不連線。真實服務與金鑰互動測試留至結案。"
    );
    if let Command::WindowTranslation(pid, provider, language, quality, interactive) = command {
        return ui_owner::run(
            live::LiveOptions {
                pid,
                live_model: config.live_speech_model,
                provider: provider
                    .or(config.speech_provider)
                    .ok_or(crate::ai::speech::SpeechError::NotConfigured)?,
                language: language.unwrap_or(config.source_language),
                quality: Some(quality.unwrap_or(config.translation_quality)),
                content: None,
            },
            interactive,
            config.display_mode,
        );
    }
    match command {
        Command::ListSources
        | Command::WatchSources
        | Command::Capture(_)
        | Command::Speech(..)
        | Command::Translate(..) => {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|_| AppError::RuntimeInitialization)?;
            runtime.block_on(async {
                match command {
                    Command::Capture(pid) => capture(pid).await,
                    Command::Speech(pid, provider, language, interactive_key) => {
                        live::run(
                            pid,
                            provider
                                .or(config.speech_provider)
                                .ok_or(crate::ai::speech::SpeechError::NotConfigured)?,
                            language.unwrap_or(config.source_language),
                            interactive_key,
                            None,
                        )
                        .await
                    }
                    Command::Translate(pid, provider, language, quality, interactive_key) => {
                        live::run(
                            pid,
                            provider
                                .or(config.speech_provider)
                                .ok_or(crate::ai::speech::SpeechError::NotConfigured)?,
                            language.unwrap_or(config.source_language),
                            interactive_key,
                            Some(quality.unwrap_or(config.translation_quality)),
                        )
                        .await
                    }
                    _ => discover(command == Command::WatchSources).await,
                }
            })
        }
        _ => Ok(()),
    }
}

#[cfg(windows)]
async fn capture(pid: u32) -> Result<()> {
    use crate::{
        audio::capture::{CaptureSession, CaptureState},
        error::AppError,
        win32::capture::{ProcessTarget, capture},
    };
    use std::{io::Write, time::Duration};

    // Pin the selected PID. Fresh Core Audio confirmation stays inside the session helper,
    // before native capture; no Core Audio objects are constructed in this parent process.
    let target = ProcessTarget::open(pid)?;
    let stop_signal = tokio::signal::ctrl_c();
    tokio::pin!(stop_signal);
    // Flush visibility before any capture starts; a broken output pipe cancels startup.
    let mut output = std::io::stdout();
    writeln!(output, "準備擷取 PID {pid} 與其子程序在所有播放裝置上的音訊。僅在記憶體內處理，不使用雲端。按 Ctrl+C 停止；切換來源請停止後重新指定 PID。")
        .and_then(|_| output.flush()).map_err(|_| AppError::ConsoleOutput)?;
    let mut session = CaptureSession::start_normalized(move |sink| capture(target, sink))?;
    let mut drain = tokio::time::interval(Duration::from_millis(20));
    drain.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut report = tokio::time::interval(Duration::from_secs(1));
    report.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            biased;
            signal = &mut stop_signal => { signal.map_err(|_| AppError::StopSignal)?; break; }
            state = session.changed() => match state? {
                CaptureState::Starting => {},
                CaptureState::Running => {
                    writeln!(output, "正在擷取 PID {pid}（工作階段 {}）；PCM 48 kHz，已啟用單聲道正規化。", session.id().0)
                        .map_err(|_| AppError::ConsoleOutput)?;
                }
                CaptureState::Stopped(result) => { result?; break; }
            },
            _ = drain.tick() => {
                for _ in 0..crate::audio::capture::RING_BLOCKS {
                    if session.try_next().is_none() { break; }
                }
            }
            _ = report.tick() => {
                let metrics = session.metrics();
                performance::report_capture(session.id(), metrics);
                tracing::debug!(session = session.id().0, captured_frames = metrics.captured_frames,
                    dropped_frames = metrics.dropped_frames, stale_frames = metrics.stale_frames,
                    raw_queue_depth = metrics.raw_queue_depth, normalized_queue_depth = metrics.normalized_queue_depth,
                    owned_capture_workers = metrics.workers_running, "音訊管線統計");
            }
        }
    }
    let stopped = session.stop();
    performance::report_capture(session.id(), session.metrics());
    stopped?;
    writeln!(output, "PID {pid} 音訊擷取已停止，工作階段緩衝區已釋放。")
        .map_err(|_| AppError::ConsoleOutput)?;
    Ok(())
}

#[cfg(windows)]
async fn discover(watch: bool) -> Result<()> {
    use crate::{
        audio::discovery::DiscoveryMonitor, error::AppError, ui::source_list,
        win32::audio::WindowsSessionScanner,
    };

    let mut monitor = DiscoveryMonitor::start(WindowsSessionScanner::new)?;
    let stop_signal = tokio::signal::ctrl_c();
    tokio::pin!(stop_signal);
    loop {
        tokio::select! {
            biased;
            signal = &mut stop_signal => {
                signal.map_err(|_| AppError::StopSignal)?;
                break;
            }
            update = monitor.next() => {
                let update = update?;
                source_list::write_update(&mut std::io::stdout().lock(), &update).map_err(|_| AppError::ConsoleOutput)?;
                if !watch {
                    update.result.clone()?;
                    break;
                }
            }
        }
    }
    monitor.stop()?;
    tracing::info!("音訊來源探索已停止");
    Ok(())
}

#[cfg(not(windows))]
pub fn run() -> Result<()> {
    Err(crate::error::AppError::UnsupportedPlatform)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_explicit_discovery_commands() {
        assert_eq!(parse_arguments([]).unwrap(), Command::Status);
        assert_eq!(
            parse_arguments(["--list-sources".into()]).unwrap(),
            Command::ListSources
        );
        assert_eq!(
            parse_arguments(["--watch-sources".into()]).unwrap(),
            Command::WatchSources
        );
        assert_eq!(parse_arguments(["--help".into()]).unwrap(), Command::Help);
        assert_eq!(parse_arguments(["--ui".into()]).unwrap(), Command::Ui);
        assert!(parse_arguments(["--ui".into(), "--translate-pid".into(), "123".into()]).is_err());
        assert!(parse_arguments(["--record".into()]).is_err());
        assert!(parse_arguments(["--list-sources".into(), "--watch-sources".into()]).is_err());
    }

    #[test]
    fn capture_requires_one_explicit_positive_pid() {
        assert_eq!(
            parse_arguments(["--capture-pid".into(), "123".into()]).unwrap(),
            Command::Capture(123)
        );
        for args in [
            vec!["--capture-pid"],
            vec!["--capture-pid", "0"],
            vec!["--capture-pid", "-1"],
            vec!["--capture-pid", "+1"],
            vec!["--capture-pid", "4294967296"],
            vec!["--capture-pid", "123", "456"],
        ] {
            assert!(parse_arguments(args.into_iter().map(Into::into)).is_err());
        }
    }
    #[test]
    fn speech_requires_one_source_and_only_supported_explicit_options() {
        use crate::{ai::provider::SpeechProvider, domain::SourceLanguage};
        assert_eq!(
            parse_arguments(
                [
                    "--stt-pid",
                    "123",
                    "--provider",
                    "gemini",
                    "--language",
                    "ko"
                ]
                .map(Into::into)
            )
            .unwrap(),
            Command::Speech(
                123,
                Some(SpeechProvider::Gemini),
                Some(SourceLanguage::Korean),
                false
            )
        );
        assert_eq!(
            parse_arguments(["--set-stt-key", "openai"].map(Into::into)).unwrap(),
            Command::SetKey(SpeechProvider::OpenAi)
        );
        assert_eq!(
            parse_arguments(["--stt-pid", "123", "--interactive-key"].map(Into::into)).unwrap(),
            Command::Speech(123, None, None, true)
        );
        for args in [
            vec!["--stt-pid", "0"],
            vec!["--stt-pid", "123", "--language", "zh"],
            vec!["--stt-pid", "123", "--provider", "other"],
            vec![
                "--stt-pid",
                "123",
                "--provider",
                "openai",
                "--provider",
                "gemini",
            ],
            vec!["--set-stt-key", "openai", "SECRET"],
            vec!["--check-stt-key"],
        ] {
            assert!(parse_arguments(args.into_iter().map(Into::into)).is_err());
        }
    }

    #[test]
    fn translation_requires_an_explicit_command_and_supported_quality() {
        use crate::{
            ai::provider::SpeechProvider,
            domain::{SourceLanguage, TranslationQuality},
        };
        assert_eq!(
            parse_arguments(
                [
                    "--translate-pid",
                    "123",
                    "--provider",
                    "openai",
                    "--language",
                    "ja",
                    "--quality",
                    "high",
                    "--interactive-key"
                ]
                .map(Into::into)
            )
            .unwrap(),
            Command::Translate(
                123,
                Some(SpeechProvider::OpenAi),
                Some(SourceLanguage::Japanese),
                Some(TranslationQuality::High),
                true
            )
        );
        for args in [
            vec!["--stt-pid", "123", "--quality", "high"],
            vec!["--translate-pid", "0"],
            vec!["--translate-pid", "123", "--quality", "unknown"],
            vec![
                "--translate-pid",
                "123",
                "--quality",
                "fast",
                "--quality",
                "high",
            ],
        ] {
            assert!(parse_arguments(args.into_iter().map(Into::into)).is_err());
        }
    }
    #[test]
    fn window_translation_requires_explicit_source_and_valid_options() {
        use crate::{
            ai::provider::SpeechProvider,
            domain::{SourceLanguage, TranslationQuality},
        };
        assert_eq!(
            parse_arguments(
                [
                    "--translate-window-pid",
                    "123",
                    "--provider",
                    "gemini",
                    "--language",
                    "ko",
                    "--quality",
                    "high",
                    "--interactive-key"
                ]
                .map(Into::into)
            )
            .unwrap(),
            Command::WindowTranslation(
                123,
                Some(SpeechProvider::Gemini),
                Some(SourceLanguage::Korean),
                Some(TranslationQuality::High),
                true
            )
        );
        for args in [
            vec!["--translate-window-pid"],
            vec!["--translate-window-pid", "0"],
            vec!["--translate-window-pid", "123", "--language", "zh"],
            vec!["--translate-window-pid", "123", "--quality", "other"],
            vec!["--translate-window-pid", "123", "--provider", "other"],
            vec![
                "--translate-window-pid",
                "123",
                "--provider",
                "openai",
                "--provider",
                "gemini",
            ],
            vec![
                "--translate-window-pid",
                "123",
                "--key",
                "PRIVATE_KEY_SENTINEL",
            ],
        ] {
            assert!(parse_arguments(args.into_iter().map(Into::into)).is_err());
        }
    }

    #[cfg(windows)]
    #[test]
    fn invalid_unicode_argument_is_not_treated_as_no_argument() {
        use std::os::windows::ffi::OsStringExt;
        assert!(parse_arguments([std::ffi::OsString::from_wide(&[0xD800])]).is_err());
    }
}
