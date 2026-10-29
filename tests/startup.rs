#![cfg(windows)]

use std::{fs, path::Path, process::Command};

use subtitlenow::{
    app::config::AppConfig,
    domain::{DisplayMode, SourceLanguage, TranslationQuality},
};

fn command(directory: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_subtitlenow"));
    // Each subprocess owns its environment; tests never mutate the shared process environment.
    command
        .env("LOCALAPPDATA", directory)
        .env("SUBTITLENOW_LOG", "INFO");
    command
}

#[test]
fn startup_reports_honest_phase_six_status_without_writing_files() {
    let directory = tempfile::tempdir().unwrap();
    let output = command(directory.path())
        .env_remove("SUBTITLENOW_LOG")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    let event: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(event["level"], "INFO");
    let fields = &event["fields"];
    assert_eq!(fields["phase"], 7);
    assert_eq!(fields["content_resolver_configured"], false);
    assert_eq!(fields["state"], "Idle");
    assert_eq!(fields["display_mode"], "SideChat");
    assert_eq!(fields["translation_quality"], "Standard");
    assert_eq!(fields["source_language"], "Auto");
    assert_eq!(fields["target_language"], "zh-TW");
    assert_eq!(fields["capture_active"], false);
    assert_eq!(fields["cloud_processing"], false);
    assert!(
        fields["message"]
            .as_str()
            .unwrap()
            .contains("--translate-window-pid")
    );
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn startup_reads_saved_preferences() {
    let directory = tempfile::tempdir().unwrap();
    let config = AppConfig {
        source_language: SourceLanguage::Japanese,
        translation_quality: TranslationQuality::High,
        display_mode: DisplayMode::Dual,
        ..AppConfig::default()
    };
    config
        .save(&directory.path().join("SubtitleNow").join("config.json"))
        .unwrap();
    let output = command(directory.path()).output().unwrap();
    assert!(output.status.success());
    let event: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(event["fields"]["source_language"], "Japanese");
    assert_eq!(event["fields"]["translation_quality"], "High");
    assert_eq!(event["fields"]["display_mode"], "Dual");
}

#[test]
fn malformed_settings_fail_without_logging_private_input_or_overwriting() {
    let directory = tempfile::tempdir().unwrap();
    let parent = directory.path().join("SubtitleNow");
    fs::create_dir(&parent).unwrap();
    let path = parent.join("config.json");
    let input = br#"{"schema_version":1,"source_language":"PRIVATE_INPUT_SENTINEL"}"#;
    fs::write(&path, input).unwrap();
    let output = command(directory.path()).output().unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!stderr.contains("PRIVATE_INPUT_SENTINEL"));
    // Match the unique directory name, which remains recognizable in JSON-escaped paths.
    assert!(!stderr.contains(directory.path().file_name().unwrap().to_str().unwrap()));
    let event: serde_json::Value = serde_json::from_str(&stderr).unwrap();
    assert_eq!(event["level"], "ERROR");
    assert_eq!(event["fields"]["code"], "config_invalid");
    assert_eq!(fs::read(path).unwrap(), input);
}

#[test]
fn missing_config_directory_environment_fails_explicitly() {
    let directory = tempfile::tempdir().unwrap();
    let output = command(directory.path())
        .env_remove("LOCALAPPDATA")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let event: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(event["fields"]["code"], "config_directory_unavailable");
}

#[test]
fn log_level_controls_output_and_rejects_unrecognized_values() {
    let directory = tempfile::tempdir().unwrap();
    let output = command(directory.path())
        .env("SUBTITLENOW_LOG", "WARN")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let output = command(directory.path())
        .env("SUBTITLENOW_LOG", "PRIVATE_LOG_SENTINEL")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("日誌層級不正確"));
    assert!(!stderr.contains("PRIVATE_LOG_SENTINEL"));
}

#[test]
fn help_works_without_config_or_audio_and_unknown_arguments_are_not_echoed() {
    let directory = tempfile::tempdir().unwrap();
    let output = command(directory.path())
        .env_remove("LOCALAPPDATA")
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("--watch-sources")
    );
    let output = command(directory.path()).arg("--help").output().unwrap();
    assert!(String::from_utf8(output.stdout).unwrap().contains("--ui"));
    let output = command(directory.path())
        .arg("PRIVATE_ARGUMENT_SENTINEL")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        !String::from_utf8(output.stderr)
            .unwrap()
            .contains("PRIVATE_ARGUMENT_SENTINEL")
    );
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn invalid_process_fails_at_source_validation_without_creating_files() {
    let directory = tempfile::tempdir().unwrap();
    let output = command(directory.path())
        .args(["--capture-pid", "4294967295"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    let events: Vec<serde_json::Value> = stderr
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(events.len(), 2);
    assert_eq!(events[1]["fields"]["code"], "audio_capture");
    let error = events[1]["fields"]["error"].as_str().unwrap();
    assert!(error.contains("OpenProcess"), "{error}");
    assert!(!error.contains("20348"));
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn speech_does_not_choose_a_provider_implicitly_or_accept_key_arguments() {
    let directory = tempfile::tempdir().unwrap();
    let output = command(directory.path())
        .args(["--stt-pid", "123"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let events: Vec<serde_json::Value> = String::from_utf8(output.stderr)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        events.last().unwrap()["fields"]["code"],
        "speech_recognition"
    );
    let output = command(directory.path())
        .args(["--set-stt-key", "openai", "PRIVATE_KEY_SENTINEL"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        !String::from_utf8(output.stderr)
            .unwrap()
            .contains("PRIVATE_KEY_SENTINEL")
    );
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn translation_does_not_choose_a_provider_or_prompt_keys_in_redirected_output() {
    let directory = tempfile::tempdir().unwrap();
    let output = command(directory.path())
        .args(["--translate-pid", "123"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let events: Vec<serde_json::Value> = String::from_utf8(output.stderr)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        events.last().unwrap()["fields"]["code"],
        "speech_recognition"
    );
    let output = command(directory.path())
        .args([
            "--translate-pid",
            "123",
            "--provider",
            "openai",
            "--interactive-key",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let events: Vec<serde_json::Value> = String::from_utf8(output.stderr)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(events.last().unwrap()["fields"]["code"], "console_output");
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
}
#[test]
fn window_translation_rejects_invalid_startup_without_credentials_or_capture() {
    let directory = tempfile::tempdir().unwrap();
    for (args, code) in [
        (vec!["--translate-window-pid", "123"], "speech_recognition"),
        (
            vec![
                "--translate-window-pid",
                "123",
                "--provider",
                "openai",
                "--interactive-key",
            ],
            "console_output",
        ),
        (
            vec![
                "--translate-window-pid",
                "4294967295",
                "--provider",
                "openai",
            ],
            "audio_capture",
        ),
        (
            vec![
                "--translate-window-pid",
                "123",
                "--key",
                "PRIVATE_KEY_SENTINEL",
            ],
            "invalid_arguments",
        ),
    ] {
        let output = command(directory.path()).args(args).output().unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(!stderr.contains("PRIVATE_KEY_SENTINEL"));
        let events: Vec<serde_json::Value> = stderr
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(events.last().unwrap()["fields"]["code"], code);
    }
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
}
