//! Explicit HTTPS subtitle modes on hidden owned controls; no network or cloud.
use super::*;

const SUBTITLE_URL: &str =
    "https://subtitles.synthetic.invalid/private?ticket=SYNTHETIC_SUBTITLE_QUERY";
const MEDIA_URL: &str = "https://media.synthetic.invalid/direct.wav?token=SYNTHETIC_MEDIA_QUERY";

fn input(dialog: &Dialog, format: isize, language: usize, provider: usize, remote_media: bool) {
    fill(dialog, language, provider);
    select(dialog.format.get(), format as usize);
    select(dialog.audio_language.get(), 0);
    set_edit(dialog.subtitle.get(), SUBTITLE_URL).unwrap();
    if remote_media {
        set_edit(dialog.media.get(), MEDIA_URL).unwrap();
    }
    notify(dialog, MEDIA, EN_CHANGE_VALUE, dialog.media.get());
    notify(dialog, FORMAT, CBN_SELCHANGE as usize, dialog.format.get());
}

fn private_values_stay_in_fields(dialog: &Dialog) {
    for control in [dialog.destination.get(), dialog.status.get()] {
        let value = text(control);
        for private in [
            SUBTITLE_URL,
            MEDIA_URL,
            "subtitles.synthetic.invalid",
            "media.synthetic.invalid",
            "SYNTHETIC_SUBTITLE_QUERY",
            "SYNTHETIC_MEDIA_QUERY",
            "ticket=",
            "token=",
            r"C:\Synthetic",
        ] {
            assert!(!value.contains(private));
        }
    }
}

fn assert_text_disclosure(dialog: &Dialog, remote_media: bool) -> String {
    let value = text(dialog.destination.get());
    for expected in [
        "連線指定 HTTPS 字幕",
        "讀取全文",
        "不探索內嵌軌",
        "本次記憶體",
        "不存檔或播放",
    ] {
        assert!(value.contains(expected));
    }
    assert!(!value.contains("短片段媒體音訊"));
    if remote_media {
        assert!(value.contains("先讀遠端WAVE標頭"));
    } else {
        assert!(value.contains("先唯讀持有本機媒體"));
    }
    private_values_stay_in_fields(dialog);
    value
}

#[test]
fn https_native_explicit_modes_keep_formats_languages_quality_and_no_audio_fallback() {
    let owner = Owner::new();
    let (dialog, backend, _) = owner.dialog();
    for remote_media in [false, true] {
        for index in [HTTPS_SRT_FORMAT_INDEX, HTTPS_WEBVTT_FORMAT_INDEX] {
            for (language_index, language) in [
                TextLanguage::TaiwanTraditional,
                TextLanguage::OtherTraditional,
                TextLanguage::SimplifiedChinese,
                TextLanguage::Source(SpeechLanguage::English),
                TextLanguage::Source(SpeechLanguage::Japanese),
                TextLanguage::Source(SpeechLanguage::French),
                TextLanguage::Source(SpeechLanguage::German),
                TextLanguage::Source(SpeechLanguage::Korean),
            ]
            .into_iter()
            .enumerate()
            {
                let provider = usize::from(language_index >= 3) * 2;
                input(&dialog, index, language_index + 1, provider, remote_media);
                select(dialog.quality.get(), 2);
                let (request, options, cloud) = dialog.read_input().unwrap();
                assert_eq!(request.media.is_remote(), remote_media);
                assert_eq!(request.plan(), Ok(PreparationPlan::RemoteSubtitle(0)));
                assert!(request.text_language().unwrap() == Some(language));
                assert!(request.subtitles.is_empty());
                assert!(request.embedded.is_none());
                assert_eq!(request.remote_subtitles.len(), 1);
                assert_eq!(options.source_language, SourceLanguage::Auto);
                assert_eq!(options.quality, TranslationQuality::High);
                assert_eq!(cloud, language_index >= 3);
            }
        }
    }
    assert_eq!(backend.borrow().starts, 0);
}

#[test]
fn https_native_chinese_needs_no_provider_key_or_audio_language_and_uses_explicit_entry() {
    let owner = Owner::new();
    for remote_media in [false, true] {
        for language in 1..=3 {
            let (dialog, backend, _) = owner.dialog();
            input(
                &dialog,
                HTTPS_WEBVTT_FORMAT_INDEX,
                language,
                0,
                remote_media,
            );
            set_edit(dialog.openai_key.get(), &"X".repeat(KEY_LIMIT + 1)).unwrap();
            set_edit(dialog.gemini_key.get(), "invalid unused key").unwrap();
            let disclosure = assert_text_disclosure(&dialog, remote_media);
            assert!(disclosure.contains("不使用 AI 或金鑰"));
            assert!(disclosure.contains("中文本機轉台灣用語"));
            assert!(!disclosure.contains("AI 目的地"));
            assert_eq!(backend.borrow().starts, 0);
            click(&dialog, PREPARE);
            assert_eq!(backend.borrow().starts, 1);
            assert_eq!(backend.borrow().https_starts, 1);
            assert_eq!(
                backend.borrow().plan,
                Some(PreparationPlan::RemoteSubtitle(0))
            );
            assert_eq!(backend.borrow().supplied_key, None);
            assert_eq!(
                backend.borrow().options.unwrap().source_language,
                SourceLanguage::Auto
            );
            assert_keys_cleared(&dialog);
            private_values_stay_in_fields(&dialog);
        }
    }
}

#[test]
fn https_native_source_discloses_chosen_endpoint_and_moves_only_its_masked_key() {
    let owner = Owner::new();
    for remote_media in [false, true] {
        for format in [HTTPS_SRT_FORMAT_INDEX, HTTPS_WEBVTT_FORMAT_INDEX] {
            for (provider, endpoint, key, expected_key) in [
                (1, "https://api.openai.com", "SYNTHETIC_OPENAI", "openai"),
                (
                    2,
                    "https://generativelanguage.googleapis.com",
                    "SYNTHETIC_GEMINI",
                    "gemini",
                ),
            ] {
                let (dialog, backend, _) = owner.dialog();
                input(&dialog, format, 4, provider, remote_media);
                let (chosen, unused) = if provider == 1 {
                    (dialog.openai_key.get(), dialog.gemini_key.get())
                } else {
                    (dialog.gemini_key.get(), dialog.openai_key.get())
                };
                set_edit(chosen, key).unwrap();
                set_edit(unused, &"X".repeat(KEY_LIMIT + 1)).unwrap();
                let disclosure = assert_text_disclosure(&dialog, remote_media);
                for expected in [
                    endpoint,
                    "原文",
                    "全文摘要",
                    "專名／詞彙",
                    "翻譯前後文",
                    "同一所選服務與金鑰",
                    "不傳音訊或任何來源網址",
                ] {
                    assert!(disclosure.contains(expected));
                }
                for control in [dialog.openai_key.get(), dialog.gemini_key.get()] {
                    // SAFETY: scalar style query of this fixture's owned EDIT.
                    assert_ne!(
                        unsafe { GetWindowLongW(control, GWL_STYLE) } as u32 & ES_PASSWORD as u32,
                        0
                    );
                }
                click(&dialog, PREPARE);
                assert_eq!(backend.borrow().starts, 1);
                assert_eq!(backend.borrow().https_starts, 1);
                assert_eq!(backend.borrow().supplied_key, Some(expected_key));
                assert_eq!(
                    backend.borrow().plan,
                    Some(PreparationPlan::RemoteSubtitle(0))
                );
                assert_keys_cleared(&dialog);
                private_values_stay_in_fields(&dialog);
            }
        }
    }
}

#[test]
fn https_native_missing_or_invalid_url_rejects_before_password_instead_of_audio() {
    let owner = Owner::new();
    let (dialog, backend, _) = owner.dialog();
    let too_long = format!("https://synthetic.invalid/{}", "x".repeat(PATH_LIMIT));
    for format in [HTTPS_SRT_FORMAT_INDEX, HTTPS_WEBVTT_FORMAT_INDEX] {
        for subtitle in [
            "",
            " ",
            "\t",
            "\r\n",
            "relative.srt",
            r"C:\Synthetic\text.srt",
            "http://synthetic.invalid/subtitle",
            "ftp://synthetic.invalid/subtitle",
            "https://user:password@synthetic.invalid/subtitle",
            "https://synthetic.invalid/subtitle#fragment",
            "https:///missing-authority",
            " https://synthetic.invalid/subtitle",
            "https://synthetic.invalid/subtitle ",
            "https://synthetic.invalid/subtitle\n",
            "https://synthetic.invalid\\subtitle",
            &too_long,
        ] {
            input(&dialog, format, 4, 1, false);
            set_edit(dialog.subtitle.get(), subtitle).unwrap();
            notify(&dialog, SUBTITLE, EN_CHANGE_VALUE, dialog.subtitle.get());
            assert!(dialog.has_subtitle());
            assert!(!text(dialog.destination.get()).contains("短片段媒體音訊"));
            set_edit(dialog.openai_key.get(), &"X".repeat(KEY_LIMIT + 1)).unwrap();
            set_edit(dialog.gemini_key.get(), "invalid unused key").unwrap();
            click(&dialog, PREPARE);
            assert_eq!(
                dialog.local_error.get(),
                Some(PreparedUiError::Media(MediaError::InvalidInput))
            );
            assert_eq!(backend.borrow().starts, 0);
            assert_eq!(backend.borrow().https_starts, 0);
            assert_eq!(text(dialog.subtitle.get()), subtitle);
            assert_keys_cleared(&dialog);
            private_values_stay_in_fields(&dialog);
        }
    }
}

#[test]
fn https_native_metadata_preflight_and_legacy_modes_do_not_read_password_or_change_sources() {
    let owner = Owner::new();
    let (dialog, backend, _) = owner.dialog();
    for (format, language, provider, expected) in [
        (
            HTTPS_SRT_FORMAT_INDEX,
            0,
            1,
            PreparedUiError::Media(MediaError::InvalidSubtitle),
        ),
        (
            HTTPS_WEBVTT_FORMAT_INDEX,
            4,
            0,
            PreparedUiError::ProviderRequired,
        ),
        (1, 4, 1, PreparedUiError::Media(MediaError::InvalidInput)),
        (2, 4, 1, PreparedUiError::Media(MediaError::InvalidInput)),
    ] {
        input(&dialog, format, language, provider, false);
        set_edit(dialog.openai_key.get(), &"X".repeat(KEY_LIMIT + 1)).unwrap();
        click(&dialog, PREPARE);
        assert_eq!(dialog.local_error.get(), Some(expected));
        assert_eq!(text(dialog.subtitle.get()), SUBTITLE_URL);
        assert_eq!(selected(dialog.format.get()), format);
        assert_keys_cleared(&dialog);
    }
    input(&dialog, HTTPS_SRT_FORMAT_INDEX, 4, 1, false);
    for media in [
        "",
        "relative.wav",
        "https://user:password@synthetic.invalid/media",
    ] {
        set_edit(dialog.media.get(), media).unwrap();
        set_edit(dialog.openai_key.get(), &"X".repeat(KEY_LIMIT + 1)).unwrap();
        click(&dialog, PREPARE);
        assert_eq!(
            dialog.local_error.get(),
            Some(PreparedUiError::Media(MediaError::InvalidInput))
        );
        assert_keys_cleared(&dialog);
    }
    assert_eq!(backend.borrow().starts, 0);
    assert_eq!(backend.borrow().https_starts, 0);
    private_values_stay_in_fields(&dialog);
}

#[test]
fn https_native_format_source_edits_preserve_url_and_discovery_cannot_replace_it() {
    let owner = Owner::new();
    let (dialog, backend, _) = owner.dialog();
    input(&dialog, HTTPS_SRT_FORMAT_INDEX, 1, 0, false);
    let discover = dialog.controls.borrow()[DISCOVER_CONTROL_INDEX];
    // SAFETY: scalar enabled-state query of owned live controls on this thread.
    assert!(!unsafe { IsWindowEnabled(discover) }.as_bool());
    set_edit(dialog.openai_key.get(), "SYNTHETIC_OPENAI").unwrap();
    click(&dialog, DISCOVER);
    assert_eq!(dialog.discovery.borrow().state(), DiscoveryState::Idle);
    assert_eq!(text(dialog.subtitle.get()), SUBTITLE_URL);
    assert_keys_cleared(&dialog);
    for format in [HTTPS_WEBVTT_FORMAT_INDEX, 1, HTTPS_SRT_FORMAT_INDEX] {
        select(dialog.format.get(), format as usize);
        notify(&dialog, FORMAT, CBN_SELCHANGE as usize, dialog.format.get());
        assert_eq!(text(dialog.subtitle.get()), SUBTITLE_URL);
        assert_eq!(selected(dialog.format.get()), format);
        // SAFETY: scalar query of the same owned live discovery button.
        assert_eq!(unsafe { IsWindowEnabled(discover) }.as_bool(), format == 1);
    }
    dialog.discovery_applied.set(true);
    set_edit(dialog.media.get(), MEDIA_URL).unwrap();
    notify(&dialog, MEDIA, EN_CHANGE_VALUE, dialog.media.get());
    assert!(!dialog.discovery_applied.get());
    assert!(dialog.tracks.get().is_empty());
    assert_eq!(text(dialog.subtitle.get()), SUBTITLE_URL);
    assert_text_disclosure(&dialog, true);
    assert_eq!(backend.borrow().starts, 0);
}

#[test]
fn https_native_disclosure_updates_for_owned_field_events_without_starting_work() {
    let owner = Owner::new();
    let (dialog, backend, _) = owner.dialog();
    input(&dialog, HTTPS_SRT_FORMAT_INDEX, 4, 1, false);
    assert!(assert_text_disclosure(&dialog, false).contains("https://api.openai.com"));
    select(dialog.provider.get(), 2);
    notify(
        &dialog,
        PROVIDER,
        CBN_SELCHANGE as usize,
        dialog.provider.get(),
    );
    assert!(
        assert_text_disclosure(&dialog, false)
            .contains("https://generativelanguage.googleapis.com")
    );
    select(dialog.language.get(), 3);
    notify(
        &dialog,
        LANGUAGE,
        CBN_SELCHANGE as usize,
        dialog.language.get(),
    );
    assert!(assert_text_disclosure(&dialog, false).contains("不使用 AI 或金鑰"));
    select(dialog.audio_language.get(), 5);
    notify(
        &dialog,
        AUDIO_LANGUAGE,
        CBN_SELCHANGE as usize,
        dialog.audio_language.get(),
    );
    assert!(assert_text_disclosure(&dialog, false).contains("不使用 AI 或金鑰"));
    set_edit(dialog.subtitle.get(), "").unwrap();
    notify(&dialog, SUBTITLE, EN_CHANGE_VALUE, dialog.subtitle.get());
    assert!(dialog.has_subtitle());
    assert!(assert_text_disclosure(&dialog, false).contains("不使用 AI 或金鑰"));
    // An empty explicit network text input still cannot become ASR.
    assert!(matches!(
        dialog.read_input(),
        Err(PreparedUiError::Media(MediaError::InvalidInput))
    ));
    set_edit(dialog.subtitle.get(), SUBTITLE_URL).unwrap();
    select(dialog.format.get(), HTTPS_WEBVTT_FORMAT_INDEX as usize);
    notify(&dialog, FORMAT, CBN_SELCHANGE as usize, dialog.format.get());
    set_edit(dialog.media.get(), MEDIA_URL).unwrap();
    notify(&dialog, MEDIA, EN_CHANGE_VALUE, dialog.media.get());
    assert!(assert_text_disclosure(&dialog, true).contains("不使用 AI 或金鑰"));
    assert_eq!(backend.borrow().starts, 0);
    assert_eq!(backend.borrow().https_starts, 0);
}

#[test]
fn https_native_ready_locks_stop_clear_undo_and_notification_follow_existing_lifecycle() {
    let owner = Owner::new();
    for remote_media in [false, true] {
        for language in [1, 4] {
            let (dialog, backend, notice) = owner.dialog();
            input(&dialog, HTTPS_SRT_FORMAT_INDEX, language, 1, remote_media);
            click(&dialog, PREPARE);
            for state in [
                PreparedUiState::Preparing,
                PreparedUiState::Stopping,
                PreparedUiState::Ready { cues: 2 },
            ] {
                backend.borrow_mut().state = state;
                dialog.poll().unwrap();
                for control in [
                    dialog.media.get(),
                    dialog.subtitle.get(),
                    dialog.format.get(),
                    dialog.language.get(),
                    dialog.audio_language.get(),
                    dialog.provider.get(),
                    dialog.quality.get(),
                    dialog.openai_key.get(),
                    dialog.gemini_key.get(),
                    dialog.controls.borrow()[DISCOVER_CONTROL_INDEX],
                ] {
                    // SAFETY: scalar enabled-state query of this fixture's controls.
                    assert!(!unsafe { IsWindowEnabled(control) }.as_bool());
                }
                click(&dialog, PREPARE);
                assert_eq!(backend.borrow().starts, 1);
                assert_eq!(backend.borrow().https_starts, 1);
                private_values_stay_in_fields(&dialog);
            }
            dialog.poll().unwrap();
            assert_eq!(notice.borrow().ready, 1);
            backend.borrow_mut().state = PreparedUiState::Preparing;
            set_edit(dialog.openai_key.get(), "SYNTHETIC_OPENAI").unwrap();
            click(&dialog, CANCEL);
            assert_eq!(backend.borrow().state, PreparedUiState::Stopping);
            assert_keys_cleared(&dialog);
            backend.borrow_mut().state = PreparedUiState::Cancelled;
            dialog.poll().unwrap();
            click(&dialog, CLEAR);
            for control in [dialog.media.get(), dialog.subtitle.get()] {
                assert!(text(control).is_empty());
                // SAFETY: own EDIT query proves private source undo was erased.
                assert_eq!(
                    unsafe { SendMessageW(control, EM_CANUNDO, None, None).0 },
                    0
                );
            }
            private_values_stay_in_fields(&dialog);
        }
    }
}

#[test]
fn https_native_close_waits_for_retirement_and_clears_keys_before_destruction() {
    let owner = Owner::new();
    let (dialog, backend, notice) = owner.dialog();
    input(&dialog, HTTPS_WEBVTT_FORMAT_INDEX, 4, 2, true);
    click(&dialog, PREPARE);
    set_edit(dialog.openai_key.get(), "SYNTHETIC_OPENAI").unwrap();
    set_edit(dialog.gemini_key.get(), "SYNTHETIC_GEMINI").unwrap();
    click(&dialog, CLOSE);
    assert_eq!(backend.borrow().state, PreparedUiState::Stopping);
    assert!(dialog.closing.get());
    assert!(!dialog.hwnd.get().0.is_null());
    assert_keys_cleared(&dialog);
    assert_eq!(text(dialog.media.get()), MEDIA_URL);
    assert_eq!(text(dialog.subtitle.get()), SUBTITLE_URL);
    backend.borrow_mut().state = PreparedUiState::Cancelled;
    dialog.poll().unwrap();
    assert!(dialog.hwnd.get().0.is_null());
    assert_eq!(notice.borrow().ready, 0);
}
