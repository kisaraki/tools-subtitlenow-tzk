//! Explicit one-run subtitle settings. Private titles/signed URLs never serialize.
use crate::{
    ai::speech::SpeechLanguage,
    resolver::{
        contracts::{MAX_METADATA_BYTES, ResolverError, TextLanguage, metadata},
        timed_subtitle::{SubtitleFormat, SubtitleTrack},
    },
};
use std::sync::Arc;
use zeroize::Zeroize;

pub struct SubtitleSourceDraft {
    pub title: String,
    pub url: String,
    pub format: SubtitleFormat,
    pub language: SpeechLanguage,
}
impl Drop for SubtitleSourceDraft {
    fn drop(&mut self) {
        self.url.zeroize();
        self.title.zeroize();
    }
}
struct Settings {
    title: String,
    url: String,
    format: SubtitleFormat,
    language: SpeechLanguage,
}
impl Drop for Settings {
    fn drop(&mut self) {
        self.url.zeroize();
        self.title.zeroize();
    }
}
/// Immutable UI/start snapshot; Clone does not duplicate private URL strings.
#[derive(Clone)]
pub struct SubtitleSourceSettings(Arc<Settings>);
impl SubtitleSourceSettings {
    pub fn from_draft(draft: SubtitleSourceDraft) -> Result<Self, ResolverError> {
        let title = draft.title.trim();
        if !metadata(&draft.title) || draft.title.len() > MAX_METADATA_BYTES {
            return Err(ResolverError::InvalidData);
        }
        SubtitleTrack::new(
            &draft.url,
            draft.format,
            TextLanguage::Source(draft.language),
        )?;
        Ok(Self(Arc::new(Settings {
            title: title.into(),
            url: draft.url.clone(),
            format: draft.format,
            language: draft.language,
        })))
    }
    pub fn title(&self) -> &str {
        &self.0.title
    }
    /// Host and explicit port only. Never display a signed path/query or userinfo.
    pub fn destination(&self) -> String {
        // Constructor validates this URL. Invalid internal data remains undisclosed.
        let Ok(url) = reqwest::Url::parse(&self.0.url) else {
            return "無效的字幕來源".into();
        };
        match (url.host_str(), url.port()) {
            (Some(host), Some(port)) => format!("https://{host}:{port}"),
            (Some(host), None) => format!("https://{host}"),
            _ => "無效的字幕來源".into(),
        }
    }
    pub fn draft(&self) -> SubtitleSourceDraft {
        SubtitleSourceDraft {
            title: self.0.title.clone(),
            url: self.0.url.clone(),
            format: self.0.format,
            language: self.0.language,
        }
    }
    pub(crate) fn track(&self) -> Result<SubtitleTrack, ResolverError> {
        SubtitleTrack::new(
            &self.0.url,
            self.0.format,
            TextLanguage::Source(self.0.language),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn draft() -> SubtitleSourceDraft {
        SubtitleSourceDraft {
            title: "Synthetic film".into(),
            url: "https://example.invalid:8443/private/subtitle.srt?token=synthetic".into(),
            format: SubtitleFormat::Srt,
            language: SpeechLanguage::English,
        }
    }
    #[test]
    fn settings_preserve_explicit_facts_and_disclose_host_without_private_location() {
        let value = SubtitleSourceSettings::from_draft(draft()).unwrap();
        assert_eq!(value.title(), "Synthetic film");
        assert_eq!(value.destination(), "https://example.invalid:8443");
        assert!(!value.destination().contains("synthetic"));
        let copied = value.clone();
        assert!(Arc::ptr_eq(&copied.0, &value.0));
        assert_eq!(copied.draft().url, draft().url);
        assert!(matches!(
            value.track().unwrap().language(),
            TextLanguage::Source(SpeechLanguage::English)
        ));
    }
    #[test]
    fn malformed_private_locations_and_bounded_titles_are_rejected() {
        for title in ["", "  ", "film\0", "film\n"] {
            let mut value = draft();
            value.title = title.into();
            assert!(SubtitleSourceSettings::from_draft(value).is_err());
        }
        let mut value = draft();
        value.title = "x".repeat(MAX_METADATA_BYTES + 1);
        assert!(SubtitleSourceSettings::from_draft(value).is_err());
        for url in [
            "http://example.invalid/subtitle.srt",
            "https://user:synthetic@example.invalid/subtitle.srt",
            "https://example.invalid/subtitle.srt#private",
            "https://example.invalid/subtitle.srt\n",
        ] {
            let mut value = draft();
            value.url = url.into();
            assert!(SubtitleSourceSettings::from_draft(value).is_err());
        }
    }
    #[test]
    fn all_five_languages_and_both_formats_are_explicit() {
        for language in SpeechLanguage::ALL {
            for format in [SubtitleFormat::Srt, SubtitleFormat::WebVtt] {
                let mut value = draft();
                value.language = language;
                value.format = format;
                let settings = SubtitleSourceSettings::from_draft(value).unwrap();
                assert_eq!(settings.draft().language, language);
                assert_eq!(settings.draft().format, format);
            }
        }
    }
}
