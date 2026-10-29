//! Versioned preferences only. Content and credentials have no place in this schema.

use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
    domain::{DisplayMode, SourceLanguage, TranslationQuality},
    error::{AppError, Result},
};

pub const CONFIG_SCHEMA_VERSION: u32 = 2;
/// Preferences are tiny; cap reads at 16 KiB so an accidental content file cannot grow RAM.
pub const MAX_CONFIG_BYTES: u64 = 16 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    pub schema_version: u32,
    #[serde(default)]
    pub source_language: SourceLanguage,
    #[serde(default)]
    pub translation_quality: TranslationQuality,
    #[serde(default)]
    pub display_mode: DisplayMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speech_provider: Option<crate::ai::provider::SpeechProvider>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_speech_model: Option<crate::ai::provider::LiveSpeechModel>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema_version: CONFIG_SCHEMA_VERSION,
            source_language: SourceLanguage::Auto,
            translation_quality: TranslationQuality::Standard,
            display_mode: DisplayMode::SideChat,
            speech_provider: None,
            live_speech_model: None,
        }
    }
}

impl AppConfig {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != CONFIG_SCHEMA_VERSION {
            return Err(AppError::UnsupportedConfigVersion {
                found: self.schema_version,
                supported: CONFIG_SCHEMA_VERSION,
            });
        }
        if self
            .live_speech_model
            .is_some_and(|model| self.speech_provider != Some(model.provider()))
        {
            return Err(AppError::InvalidArguments);
        }
        Ok(())
    }

    /// A missing file uses defaults without writing anything. Other failures are explicit.
    pub fn load(path: &Path) -> Result<Self> {
        let file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(source) => return Err(AppError::ConfigRead { source }),
        };
        let mut bytes = Vec::new();
        file.take(MAX_CONFIG_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|source| AppError::ConfigRead { source })?;
        if bytes.len() as u64 > MAX_CONFIG_BYTES {
            return Err(AppError::ConfigTooLarge {
                limit_bytes: MAX_CONFIG_BYTES,
            });
        }
        let mut config: Self =
            serde_json::from_slice(&bytes).map_err(|source| AppError::ConfigInvalid { source })?;
        if config.schema_version == 1 {
            config.schema_version = CONFIG_SCHEMA_VERSION;
        }
        config.validate()?;
        Ok(config)
    }

    /// Replace preferences using a same-directory temporary file, never media or subtitles.
    /// Validation and writing finish before replacement; failed writes clean up via RAII.
    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        let mut bytes =
            serde_json::to_vec_pretty(self).map_err(|source| AppError::ConfigEncode { source })?;
        bytes.push(b'\n');

        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent).map_err(|source| AppError::ConfigWrite { source })?;
        let mut temporary = tempfile::Builder::new()
            .prefix(".subtitlenow-config-")
            .tempfile_in(parent)
            .map_err(|source| AppError::ConfigWrite { source })?;
        temporary
            .write_all(&bytes)
            .map_err(|source| AppError::ConfigWrite { source })?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|source| AppError::ConfigWrite { source })?;
        temporary
            .persist(path)
            .map_err(|error| AppError::ConfigWrite {
                source: error.error,
            })?;
        Ok(())
    }
}

/// Does not create the directory. No fallback to the repository or a roaming location.
#[cfg(windows)]
pub fn default_path() -> Result<PathBuf> {
    let directory = std::env::var_os("LOCALAPPDATA")
        .filter(|value| !value.is_empty())
        .ok_or(AppError::ConfigDirectoryUnavailable)?;
    Ok(PathBuf::from(directory)
        .join("SubtitleNow")
        .join("config.json"))
}

#[cfg(not(windows))]
pub fn default_path() -> Result<PathBuf> {
    Err(AppError::UnsupportedPlatform)
}
