use std::{fs, path::Path};

use subtitlenow::{
    app::config::{AppConfig, CONFIG_SCHEMA_VERSION, MAX_CONFIG_BYTES},
    domain::{DisplayMode, SourceLanguage, TARGET_LANGUAGE, TranslationQuality},
    error::AppError,
};

fn load_json(directory: &Path, json: &[u8]) -> Result<AppConfig, AppError> {
    let path = directory.join("config.json");
    fs::write(&path, json).unwrap();
    AppConfig::load(&path)
}
#[test]
fn phase11_live_model_profiles_round_trip_and_v1_migration_does_not_write_or_change_vendor() {
    use subtitlenow::ai::provider::LiveSpeechModel;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.json");
    let old = br#"{"schema_version":1,"speech_provider":"Gemini","source_language":"English"}"#;
    fs::write(&path, old).unwrap();
    let migrated = AppConfig::load(&path).unwrap();
    assert_eq!(migrated.schema_version, CONFIG_SCHEMA_VERSION);
    assert_eq!(migrated.live_speech_model, None);
    assert_eq!(fs::read(&path).unwrap(), old);
    for model in [
        LiveSpeechModel::GptLiveTranscribe,
        LiveSpeechModel::GeminiTranscribeLive,
        LiveSpeechModel::GeminiLiveTranslate,
    ] {
        let config = AppConfig {
            speech_provider: Some(model.provider()),
            live_speech_model: Some(model),
            ..AppConfig::default()
        };
        config.save(&path).unwrap();
        assert_eq!(AppConfig::load(&path).unwrap(), config);
    }
    let mut invalid = AppConfig::load(&path).unwrap();
    invalid.speech_provider = Some(subtitlenow::ai::provider::SpeechProvider::OpenAi);
    let previous = fs::read(&path).unwrap();
    assert!(invalid.save(&path).is_err());
    assert_eq!(fs::read(&path).unwrap(), previous);
    assert!(load_json(directory.path(),br#"{"schema_version":2,"speech_provider":"Gemini","live_speech_model":"unapproved-model"}"#).is_err());
}

#[test]
fn speech_provider_preference_is_optional_and_never_contains_credentials() {
    let directory = tempfile::tempdir().unwrap();
    for provider in [
        subtitlenow::ai::provider::SpeechProvider::OpenAi,
        subtitlenow::ai::provider::SpeechProvider::Gemini,
    ] {
        let config = AppConfig {
            speech_provider: Some(provider),
            ..AppConfig::default()
        };
        let path = directory.path().join("config.json");
        config.save(&path).unwrap();
        assert_eq!(AppConfig::load(&path).unwrap(), config);
    }
    assert!(matches!(
        load_json(
            directory.path(),
            br#"{"schema_version":1,"speech_provider":"Unapproved"}"#
        ),
        Err(AppError::ConfigInvalid { .. })
    ));
    assert!(matches!(
        load_json(
            directory.path(),
            br#"{"schema_version":1,"speech_provider":"OpenAi","api_key":"PRIVATE"}"#
        ),
        Err(AppError::ConfigInvalid { .. })
    ));
}

#[test]
fn missing_file_uses_product_defaults_without_creating_files() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("not-created").join("config.json");
    let config = AppConfig::load(&path).unwrap();
    assert_eq!(config.schema_version, CONFIG_SCHEMA_VERSION);
    assert_eq!(config.source_language, SourceLanguage::Auto);
    assert_eq!(config.translation_quality, TranslationQuality::Standard);
    assert_eq!(config.display_mode, DisplayMode::SideChat);
    assert_eq!(TARGET_LANGUAGE, "zh-TW");
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn omitted_preferences_use_defaults_but_version_is_required() {
    let directory = tempfile::tempdir().unwrap();
    assert_eq!(
        load_json(directory.path(), br#"{"schema_version":1}"#).unwrap(),
        AppConfig::default()
    );
    assert!(matches!(
        load_json(directory.path(), b"{}"),
        Err(AppError::ConfigInvalid { .. })
    ));
}

#[test]
fn all_supported_preferences_round_trip_and_replace_existing_file() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("preferences").join("config.json");
    for source_language in [
        SourceLanguage::Auto,
        SourceLanguage::English,
        SourceLanguage::Japanese,
        SourceLanguage::French,
        SourceLanguage::German,
        SourceLanguage::Korean,
    ] {
        for translation_quality in [
            TranslationQuality::Fast,
            TranslationQuality::Standard,
            TranslationQuality::High,
        ] {
            for display_mode in [
                DisplayMode::SideChat,
                DisplayMode::BottomSubtitle,
                DisplayMode::Dual,
            ] {
                let config = AppConfig {
                    source_language,
                    translation_quality,
                    display_mode,
                    ..AppConfig::default()
                };
                config.save(&path).unwrap();
                assert_eq!(AppConfig::load(&path).unwrap(), config);
            }
        }
    }
    assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
    let json: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    let keys: Vec<_> = json
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "display_mode",
            "schema_version",
            "source_language",
            "translation_quality"
        ]
    );
}

#[test]
fn rejects_unknown_versions_without_changing_the_file() {
    let directory = tempfile::tempdir().unwrap();
    for version in [0, CONFIG_SCHEMA_VERSION + 1] {
        let json = format!(r#"{{"schema_version":{version}}}"#);
        assert!(matches!(
            load_json(directory.path(), json.as_bytes()),
            Err(AppError::UnsupportedConfigVersion { found, supported })
                if found == version && supported == CONFIG_SCHEMA_VERSION
        ));
        assert_eq!(
            fs::read(directory.path().join("config.json")).unwrap(),
            json.as_bytes()
        );
    }
}

#[test]
fn invalid_save_leaves_previous_preferences_intact() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.json");
    AppConfig::default().save(&path).unwrap();
    let original = fs::read(&path).unwrap();
    let invalid = AppConfig {
        schema_version: CONFIG_SCHEMA_VERSION + 1,
        ..AppConfig::default()
    };
    assert!(matches!(
        invalid.save(&path),
        Err(AppError::UnsupportedConfigVersion { .. })
    ));
    assert_eq!(fs::read(path).unwrap(), original);
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn rejects_unknown_content_and_credential_fields() {
    let directory = tempfile::tempdir().unwrap();
    for field in [
        "audio",
        "transcript",
        "translation",
        "lyrics",
        "subtitle",
        "fingerprint",
        "metadata",
        "api_key",
        "target_language",
        "telemetry",
        "unexpected_setting",
    ] {
        let json = format!(r#"{{"schema_version":1,"{field}":"private-sentinel"}}"#);
        let error = load_json(directory.path(), json.as_bytes()).unwrap_err();
        assert!(matches!(error, AppError::ConfigInvalid { .. }));
        assert!(!error.to_string().contains("private-sentinel"));
        assert!(!error.to_string().contains(field));
    }
}

#[test]
fn rejects_corruption_invalid_utf8_and_unsupported_preferences() {
    let directory = tempfile::tempdir().unwrap();
    let invalid: &[&[u8]] = &[
        b"",
        b"{",
        b"\xff\xfe",
        br#"{"schema_version":1,"source_language":"Spanish"}"#,
        br#"{"schema_version":1,"source_language":"Chinese"}"#,
        br#"{"schema_version":1,"translation_quality":"Ultra"}"#,
        br#"{"schema_version":1,"display_mode":"Floating"}"#,
        br#"{"schema_version":1,"source_language":null}"#,
        br#"{"schema_version":1,"schema_version":1}"#,
    ];
    for bytes in invalid {
        assert!(matches!(
            load_json(directory.path(), bytes),
            Err(AppError::ConfigInvalid { .. })
        ));
    }
}

#[test]
fn enforces_read_size_limit_at_the_boundary() {
    let directory = tempfile::tempdir().unwrap();
    let mut bytes = br#"{"schema_version":1}"#.to_vec();
    bytes.resize(MAX_CONFIG_BYTES as usize, b' ');
    assert_eq!(
        load_json(directory.path(), &bytes).unwrap(),
        AppConfig::default()
    );
    bytes.push(b' ');
    assert!(matches!(
        load_json(directory.path(), &bytes),
        Err(AppError::ConfigTooLarge { limit_bytes }) if limit_bytes == MAX_CONFIG_BYTES
    ));
}

#[test]
fn read_failure_is_not_treated_as_a_missing_config() {
    let directory = tempfile::tempdir().unwrap();
    assert!(matches!(
        AppConfig::load(directory.path()),
        Err(AppError::ConfigRead { .. })
    ));
}

#[test]
fn failed_replacement_cleans_up_temporary_preferences() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("existing-directory");
    fs::create_dir(&path).unwrap();
    let marker = path.join("keep.txt");
    fs::write(&marker, b"keep").unwrap();
    assert!(matches!(
        AppConfig::default().save(&path),
        Err(AppError::ConfigWrite { .. })
    ));
    assert_eq!(fs::read(marker).unwrap(), b"keep");
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn invalid_parent_reports_write_error_without_modifying_it() {
    let directory = tempfile::tempdir().unwrap();
    let parent = directory.path().join("file-instead-of-directory");
    fs::write(&parent, b"keep").unwrap();
    assert!(matches!(
        AppConfig::default().save(&parent.join("config.json")),
        Err(AppError::ConfigWrite { .. })
    ));
    assert_eq!(fs::read(parent).unwrap(), b"keep");
}
