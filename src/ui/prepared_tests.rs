use super::*;
use crate::{
    ai::speech::SpeechLanguage,
    media::{input::MAX_PREPARED_PATH_BYTES, planning::PreparationPlan},
};
fn reference_draft() -> PreparedReferenceDraft {
    PreparedReferenceDraft {
        path: local_path("explicit-reference.srt"),
        edition: "explicit-synthetic-edition".into(),
        format: Some(SubtitleFormat::Srt),
        language: Some(SpeechLanguage::English),
    }
}
fn chinese_request(language: TextLanguage) -> PreparedRequest {
    PreparedInputDraft {
        media: local_path("media.wave"),
        subtitle: local_path("primary.srt"),
        format: SubtitleFormat::Srt,
        language,
    }
    .into_request()
    .unwrap()
}
#[test]
fn alignment_reference_draft_is_passive_and_does_not_compete_with_the_chinese_primary() {
    let request = reference_draft()
        .apply(chinese_request(TextLanguage::TaiwanTraditional))
        .unwrap();
    assert_eq!(request.plan().unwrap(), PreparationPlan::Subtitle(0));
    let reference = request.reference.as_ref().unwrap();
    assert_eq!(reference.edition.as_ref(), "explicit-synthetic-edition");
    assert!(reference.input.language == TextLanguage::Source(SpeechLanguage::English));
}
#[test]
fn alignment_reference_draft_requires_path_format_language_and_an_explicit_edition() {
    for missing in 0..4 {
        let mut draft = reference_draft();
        match missing {
            0 => draft.path.clear(),
            1 => draft.format = None,
            2 => draft.language = None,
            _ => draft.edition.clear(),
        }
        assert!(
            draft
                .apply(chinese_request(TextLanguage::TaiwanTraditional))
                .is_err()
        );
    }
    let request = PreparedReferenceDraft {
        path: String::new(),
        edition: String::new(),
        format: None,
        language: None,
    }
    .apply(chinese_request(TextLanguage::TaiwanTraditional))
    .unwrap();
    assert!(request.reference.is_none());
}
#[test]
fn alignment_reference_draft_rejects_foreign_primaries_and_remote_reference_paths() {
    assert!(
        reference_draft()
            .apply(chinese_request(TextLanguage::Source(
                SpeechLanguage::Japanese
            )))
            .is_err()
    );
    let mut draft = reference_draft();
    draft.path = "https://synthetic.invalid/reference.srt".into();
    assert!(
        draft
            .apply(chinese_request(TextLanguage::TaiwanTraditional))
            .is_err()
    );
}
#[test]
fn alignment_reference_editions_are_bounded_and_never_inferred_from_a_filename() {
    for edition in [
        "\nsynthetic".to_string(),
        " ".into(),
        "x".repeat(crate::resolver::contracts::MAX_METADATA_BYTES + 1),
    ] {
        let mut draft = reference_draft();
        draft.edition = edition;
        assert!(
            draft
                .apply(chinese_request(TextLanguage::SimplifiedChinese))
                .is_err()
        );
    }
}
#[test]
fn alignment_reference_publication_cannot_silently_switch_to_another_primary_after_binding() {
    let request = reference_draft()
        .apply(chinese_request(TextLanguage::SimplifiedChinese))
        .unwrap()
        .with_remote_subtitle(
            crate::media::remote_subtitle::PreparedSubtitleUrl::new(
                "https://synthetic.invalid/other.srt",
                SubtitleFormat::Srt,
                TextLanguage::TaiwanTraditional,
            )
            .unwrap(),
        )
        .unwrap();
    assert!(request.plan().is_err());
}

fn local_path(name: &str) -> String {
    #[cfg(windows)]
    {
        format!(r"C:\Synthetic Prepared Inputs\{name}")
    }
    #[cfg(not(windows))]
    {
        format!("/synthetic-prepared-inputs/{name}")
    }
}
fn draft() -> PreparedInputDraft {
    PreparedInputDraft {
        media: local_path("media with spaces.mp4"),
        subtitle: local_path("subtitle with spaces.srt"),
        format: SubtitleFormat::Srt,
        language: TextLanguage::TaiwanTraditional,
    }
}

#[test]
fn explicit_sidecar_request_is_built_without_opening_synthetic_files() {
    let request = draft().into_request().unwrap();
    assert_eq!(request.plan(), Ok(PreparationPlan::Subtitle(0)));
    assert_eq!(request.subtitles.len(), 1);
    assert_eq!(request.subtitles[0].format, SubtitleFormat::Srt);
    assert!(request.subtitles[0].language == TextLanguage::TaiwanTraditional);
}

#[test]
fn all_declared_languages_and_both_formats_survive_draft_validation() {
    let languages = [
        TextLanguage::TaiwanTraditional,
        TextLanguage::OtherTraditional,
        TextLanguage::SimplifiedChinese,
        TextLanguage::Source(SpeechLanguage::English),
        TextLanguage::Source(SpeechLanguage::Japanese),
        TextLanguage::Source(SpeechLanguage::French),
        TextLanguage::Source(SpeechLanguage::German),
        TextLanguage::Source(SpeechLanguage::Korean),
    ];
    for language in languages {
        for format in [SubtitleFormat::Srt, SubtitleFormat::WebVtt] {
            let mut input = draft();
            input.language = language;
            input.format = format;
            let request = input.into_request().unwrap();
            assert!(request.subtitles[0].language == language);
            assert_eq!(request.subtitles[0].format, format);
        }
    }
}

#[test]
fn invalid_media_and_nonempty_sidecar_inputs_are_rejected_without_io() {
    for invalid in [
        "".to_owned(),
        "relative.mp4".into(),
        local_path("bad\0name.mp4"),
        local_path("bad\nname.mp4"),
        "ftp://example.invalid/private/movie.mp4".into(),
    ] {
        let mut input = draft();
        input.media = invalid.clone();
        assert!(matches!(
            input.into_request(),
            Err(MediaError::InvalidInput)
        ));
        if !invalid.is_empty() {
            let mut input = draft();
            input.subtitle = invalid;
            assert!(matches!(
                input.into_request(),
                Err(MediaError::InvalidInput)
            ));
        }
    }
}

#[test]
fn local_path_capacity_is_enforced_for_each_private_field() {
    for media_field in [false, true] {
        let mut input = draft();
        if media_field {
            input.media = local_path(&"a".repeat(MAX_PREPARED_PATH_BYTES));
        } else {
            input.subtitle = local_path(&"a".repeat(MAX_PREPARED_PATH_BYTES));
        }
        assert!(matches!(
            input.into_request(),
            Err(MediaError::InvalidInput)
        ));
    }
}

#[test]
fn exactly_empty_sidecar_requests_asr_without_inferred_language_or_file_io() {
    // Format/language belong to an explicit subtitle only. Clearing that path
    // does not construct a Chinese sidecar or silently select a speech language.
    for language in [
        TextLanguage::TaiwanTraditional,
        TextLanguage::SimplifiedChinese,
        TextLanguage::Source(SpeechLanguage::Korean),
    ] {
        let mut input = draft();
        input.subtitle.clear();
        input.language = language;
        let request = input.into_request().unwrap();
        assert_eq!(request.plan(), Ok(PreparationPlan::AsrRequired));
        assert!(request.subtitles.is_empty());
    }
}

#[test]
fn whitespace_sidecar_is_invalid_instead_of_implicitly_triggering_audio_upload() {
    for path in [" ", "\t", "\r\n"] {
        let mut input = draft();
        input.subtitle = path.into();
        assert!(matches!(
            input.into_request(),
            Err(MediaError::InvalidInput)
        ));
    }
}

#[test]
fn embedded_prepared_draft_requires_exact_nonzero_ascii_track_id_without_io() {
    for id in [
        "",
        "0",
        "00",
        " 3",
        "3 ",
        "+3",
        "-3",
        "3\n",
        "３",
        "3.0",
        "4294967296",
        "00000000003",
        "text.srt",
    ] {
        let mut input = draft();
        input.subtitle = id.into();
        assert!(
            matches!(input.into_embedded_request(), Err(MediaError::InvalidInput)),
            "accepted invalid synthetic ID {id}"
        );
    }
    for id in ["1", "3", "0000000003", "4294967295"] {
        let mut input = draft();
        input.subtitle = id.into();
        let request = input.into_embedded_request().unwrap();
        assert_eq!(request.plan(), Ok(PreparationPlan::EmbeddedMp4));
        assert_eq!(
            request.embedded.unwrap().track_id(),
            id.parse::<u32>().unwrap()
        );
        assert!(request.subtitles.is_empty());
    }
}

#[test]
fn embedded_prepared_draft_preserves_all_explicit_text_languages_and_path_guards() {
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
        let mut input = draft();
        input.subtitle = "3".into();
        input.language = language;
        assert!(
            input
                .into_embedded_request()
                .unwrap()
                .text_language()
                .unwrap()
                == Some(language)
        );
    }
    let mut input = draft();
    input.subtitle = "3".into();
    input.media = "https://example.invalid/media.mp4".into();
    assert!(matches!(
        input.into_embedded_request(),
        Err(MediaError::UnsupportedUrl)
    ));
}

#[cfg(windows)]
#[test]
fn windows_path_guards_remain_in_force_at_the_ui_boundary() {
    for invalid in [
        r"C:\Synthetic Prepared Inputs\..\media.mp4",
        r"\\server\share\media.mp4",
        r"\\?\C:\media.mp4",
        r"C:\media.mp4:private",
        r"C:\Synthetic Prepared Inputs\CON.mp4",
        r"C:\Synthetic Prepared Inputs\media.mp4 ",
        r#""C:\Synthetic Prepared Inputs\media.mp4""#,
    ] {
        let mut input = draft();
        input.media = invalid.into();
        assert!(matches!(
            input.into_request(),
            Err(MediaError::InvalidInput)
        ));
    }
}

#[test]
fn ready_label_reports_document_readiness_and_unavailable_playback() {
    let label = PreparedUiState::Ready { cues: 42 }.label();
    assert!(label.contains("42"));
    assert!(label.contains("字幕準備完成"));
    assert!(label.contains("可保留返回"));
}

#[test]
fn explicit_http_scheme_builds_remote_asr_draft_without_extension_or_io() {
    for location in [
        "http://example.invalid/direct.wav?token=SYNTHETIC_PRIVATE",
        "HTTPS://example.invalid/no-extension",
        "HtTp://127.0.0.1:9/synthetic.bin",
    ] {
        let mut input = draft();
        input.media = location.into();
        input.subtitle.clear();
        assert!(is_remote_media_text(&input.media));
        let request = input.into_request().unwrap();
        assert!(request.media.is_remote());
        assert_eq!(request.plan(), Ok(PreparationPlan::AsrRequired));
    }
    let mut input = draft();
    input.media = local_path("https-name.wav");
    input.subtitle.clear();
    assert!(!is_remote_media_text(&input.media));
    assert!(!input.into_request().unwrap().media.is_remote());
}

#[test]
fn remote_draft_never_discards_nonempty_selected_text_to_enable_audio() {
    for subtitle in [" ", "\t", "https://example.invalid/sub.srt", "27"] {
        let mut input = draft();
        input.media = "https://example.invalid/direct.wav".into();
        input.subtitle = subtitle.into();
        assert!(matches!(
            input.into_request(),
            Err(MediaError::InvalidInput)
        ));
    }
    for track in ["", "1", "bad"] {
        let mut input = draft();
        input.media = "https://example.invalid/direct.wav".into();
        input.subtitle = track.into();
        assert!(matches!(
            input.into_embedded_request(),
            Err(MediaError::UnsupportedUrl)
        ));
    }
}

#[test]
fn remote_draft_preserves_local_sidecar_format_and_declared_language_without_io() {
    for format in [SubtitleFormat::Srt, SubtitleFormat::WebVtt] {
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
            let mut input = draft();
            input.media = "https://example.invalid/direct.wav?token=SYNTHETIC_PRIVATE".into();
            input.format = format;
            input.language = language;
            let request = input.into_request().unwrap();
            assert!(request.media.is_remote());
            assert_eq!(request.plan(), Ok(PreparationPlan::Subtitle(0)));
            assert_eq!(request.subtitles.len(), 1);
            assert_eq!(request.subtitles[0].format, format);
            assert!(request.text_language().unwrap() == Some(language));
            assert!(request.embedded.is_none());
        }
    }
}

#[cfg(windows)]
#[test]
fn remote_draft_keeps_original_local_sidecar_path_guards() {
    for subtitle in [
        r"C:\Synthetic\..\sidecar.srt",
        r"\\server\share\sidecar.srt",
        r"\\?\C:\sidecar.srt",
        r"C:\sidecar.srt:private",
        r"C:\Synthetic\CON.srt",
        r"C:\Synthetic\sidecar.srt ",
        r#""C:\Synthetic\sidecar.srt""#,
    ] {
        let mut input = draft();
        input.media = "https://example.invalid/direct.wav".into();
        input.subtitle = subtitle.into();
        assert!(matches!(
            input.into_request(),
            Err(MediaError::InvalidInput)
        ));
    }
}

#[test]
fn invalid_url_drafts_are_not_trimmed_repaired_or_treated_as_local_files() {
    for location in [
        "https://user:password@example.invalid/source.wav",
        "https://example.invalid/source.wav#part",
        "https://example.invalid/source.wav ",
        "https://example.invalid/source.wav\n",
        "https:///missing-authority",
        "https://example.invalid\\source.wav",
        " https://example.invalid/source.wav",
        "file:///C:/source.wav",
        "ftp://example.invalid/source.wav",
        "https:/example.invalid/source.wav",
        "日文🙂https://example.invalid/source.wav",
    ] {
        let mut input = draft();
        input.media = location.into();
        input.subtitle.clear();
        assert!(matches!(
            input.into_request(),
            Err(MediaError::InvalidInput)
        ));
    }
    let mut input = draft();
    input.media = format!(
        "https://example.invalid/{}",
        "a".repeat(crate::media::input::MAX_PREPARED_URL_BYTES)
    );
    input.subtitle.clear();
    assert!(matches!(
        input.into_request(),
        Err(MediaError::InvalidInput)
    ));
}

#[test]
fn remote_failure_labels_do_not_expose_url_authorization_or_private_query() {
    use crate::media::remote_http::RemoteMediaError;
    for cause in [
        RemoteMediaError::AccessDenied,
        RemoteMediaError::Redirected,
        RemoteMediaError::SourceChanged,
        RemoteMediaError::UnsupportedRange,
    ] {
        let label =
            PreparedUiState::Failed(PreparedUiError::Media(MediaError::Remote(cause))).label();
        assert!(label.starts_with("媒體準備："));
        assert!(!label.contains("example.invalid"));
        assert!(!label.contains("SYNTHETIC_PRIVATE"));
        assert!(!label.contains("password"));
    }
}

#[test]
fn failed_labels_keep_typed_errors_and_never_include_private_input() {
    let errors = [
        PreparedUiError::Media(MediaError::Unavailable),
        PreparedUiError::Credential(CredentialError::Missing),
        PreparedUiError::Translation(TranslationError::Credential),
        PreparedUiError::Speech(PreparedSpeechError::UnsupportedLanguage),
        PreparedUiError::ProviderRequired,
        PreparedUiError::Busy,
        PreparedUiError::WorkerStart,
        PreparedUiError::WorkerFailed,
    ];
    for error in errors {
        let label = PreparedUiState::Failed(error).label();
        assert!(label.starts_with("媒體準備："));
        assert!(!label.contains("Synthetic"));
        assert!(!label.contains("SYNTHETIC_TEST_KEY"));
        assert!(!label.contains("example.invalid"));
    }
}

#[test]
fn cancellation_and_idle_statuses_do_not_suggest_automatic_playback() {
    for state in [
        PreparedUiState::Idle,
        PreparedUiState::Preparing,
        PreparedUiState::Stopping,
        PreparedUiState::Cancelled,
    ] {
        let label = state.label();
        assert!(!label.is_empty());
        assert!(!label.contains("播放"));
    }
}
