//! Explicit UI mode boundaries. Constructors remain passive and private.
use super::*;
use crate::{
    ai::speech::SpeechLanguage,
    media::{input::MAX_PREPARED_URL_BYTES, planning::PreparationPlan},
};

fn draft() -> PreparedInputDraft {
    PreparedInputDraft {
        media: if cfg!(windows) {
            r"C:\Synthetic Prepared Inputs\media.mp4".into()
        } else {
            "/synthetic-prepared-inputs/media.mp4".into()
        },
        subtitle: "https://synthetic.invalid/private?ticket=SYNTHETIC_PRIVATE".into(),
        format: SubtitleFormat::Srt,
        language: TextLanguage::TaiwanTraditional,
    }
}

#[test]
fn https_draft_preserves_all_declared_languages_formats_and_media_kinds_without_io() {
    for language in [
        TextLanguage::TaiwanTraditional,
        TextLanguage::OtherTraditional,
        TextLanguage::SimplifiedChinese,
        TextLanguage::Source(SpeechLanguage::English),
        TextLanguage::Source(SpeechLanguage::Japanese),
        TextLanguage::Source(SpeechLanguage::French),
        TextLanguage::Source(SpeechLanguage::German),
        TextLanguage::Source(SpeechLanguage::Korean),
    ] {
        for format in [SubtitleFormat::Srt, SubtitleFormat::WebVtt] {
            for remote in [false, true] {
                let mut input = draft();
                input.format = format;
                input.language = language;
                if remote {
                    input.media = "http://synthetic.invalid/media?ticket=SYNTHETIC_MEDIA".into();
                }
                let request = input.into_https_subtitle_request().unwrap();
                assert_eq!(request.plan(), Ok(PreparationPlan::RemoteSubtitle(0)));
                assert!(request.text_language().unwrap() == Some(language));
                assert_eq!(request.media.is_remote(), remote);
                assert_eq!(request.remote_subtitles.len(), 1);
                assert!(request.subtitles.is_empty());
                assert!(request.embedded.is_none());
            }
        }
    }
}

#[test]
fn https_draft_requires_nonempty_valid_https_instead_of_local_text_or_asr_fallback() {
    for subtitle in [
        "",
        " ",
        "\t",
        "1",
        "relative.srt",
        r"C:\Synthetic\text.srt",
        "http://synthetic.invalid/sub",
        "ftp://synthetic.invalid/sub",
        "https://user:password@synthetic.invalid/sub",
        "https://@synthetic.invalid/sub",
        "https://synthetic.invalid/sub#private",
        "https:///sub",
        "https:synthetic.invalid/sub",
        "https://synthetic.invalid/a b",
        " https://synthetic.invalid/sub",
        "https://synthetic.invalid/sub\n",
        "https://synthetic.invalid\\sub",
    ] {
        let mut input = draft();
        input.subtitle = subtitle.into();
        assert!(matches!(
            input.into_https_subtitle_request(),
            Err(MediaError::InvalidInput)
        ));
    }
    let mut input = draft();
    input.subtitle = format!(
        "https://synthetic.invalid/{}",
        "a".repeat(MAX_PREPARED_URL_BYTES)
    );
    assert!(matches!(
        input.into_https_subtitle_request(),
        Err(MediaError::InvalidInput)
    ));
}

#[test]
fn https_draft_keeps_media_path_and_url_validation_before_scheduling() {
    for media in [
        "",
        "relative.mp4",
        "ftp://synthetic.invalid/media",
        "https:///media",
        "https://user:password@synthetic.invalid/media",
        "https://synthetic.invalid/a#private",
        "https://synthetic.invalid/a b",
        "https://synthetic.invalid/a\n",
    ] {
        let mut input = draft();
        input.media = media.into();
        assert!(matches!(
            input.into_https_subtitle_request(),
            Err(MediaError::InvalidInput)
        ));
    }
    #[cfg(windows)]
    for media in [
        r"C:\Synthetic\..\media.mp4",
        r"\\server\share\media.mp4",
        r"\\?\C:\media.mp4",
        r"C:\media.mp4:private",
        r"C:\Synthetic\CON.mp4",
        r"C:\Synthetic\media.mp4 ",
    ] {
        let mut input = draft();
        input.media = media.into();
        assert!(matches!(
            input.into_https_subtitle_request(),
            Err(MediaError::InvalidInput)
        ));
    }
}

#[test]
fn https_subtitle_text_does_not_implicitly_select_network_mode_in_legacy_drafts() {
    for subtitle in [
        "https://synthetic.invalid/a.srt?ticket=SYNTHETIC_PRIVATE",
        "HTTPS://synthetic.invalid/no-extension",
    ] {
        let mut input = draft();
        input.subtitle = subtitle.into();
        assert!(matches!(
            input.into_request(),
            Err(MediaError::InvalidInput)
        ));
        let mut input = draft();
        input.subtitle = subtitle.into();
        assert!(matches!(
            input.into_embedded_request(),
            Err(MediaError::InvalidInput)
        ));
    }
}

struct LegacyBackend;
impl PreparedBackend for LegacyBackend {
    fn start(
        &mut self,
        _: PreparedRequest,
        _: PreparedOptions,
        _: Option<ApiKey>,
    ) -> Result<(), PreparedUiError> {
        panic!("explicit HTTPS entry must not fall through to legacy/audio start")
    }
    fn poll(&mut self) -> PreparedUiState {
        PreparedUiState::Idle
    }
    fn state(&self) -> PreparedUiState {
        PreparedUiState::Idle
    }
    fn stop(&mut self) {}
}

#[test]
fn https_backend_default_fails_closed_without_calling_legacy_start() {
    let request = draft().into_https_subtitle_request().unwrap();
    let mut backend = LegacyBackend;
    assert_eq!(
        backend.start_with_https_subtitles(
            request,
            PreparedOptions {
                provider: Some(SpeechProvider::OpenAi),
                quality: TranslationQuality::Standard,
                source_language: SourceLanguage::Auto,
            },
            Some(ApiKey::from_text("SYNTHETIC_TEST_KEY").unwrap())
        ),
        Err(PreparedUiError::Media(MediaError::UnsupportedUrl))
    );
    assert_eq!(backend.state(), PreparedUiState::Idle);
}

#[test]
fn https_failure_labels_keep_transport_cause_without_private_locations() {
    use crate::media::remote_subtitle::RemoteSubtitleFailure;
    for cause in [
        RemoteSubtitleFailure::NotFound,
        RemoteSubtitleFailure::AccessDenied,
        RemoteSubtitleFailure::RateLimited,
        RemoteSubtitleFailure::Redirected,
        RemoteSubtitleFailure::Timeout,
        RemoteSubtitleFailure::Network,
        RemoteSubtitleFailure::TooLarge,
        RemoteSubtitleFailure::Overloaded,
    ] {
        let error = PreparedUiError::Media(MediaError::RemoteSubtitle(cause));
        let label = PreparedUiState::Failed(error).label();
        assert!(label.contains(&cause.to_string()));
        for private in [
            "synthetic.invalid",
            "SYNTHETIC_PRIVATE",
            "ticket=",
            "password",
        ] {
            assert!(!label.contains(private));
        }
    }
}
