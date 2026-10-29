//! Hidden native URL controls with synthetic input, no network or provider I/O.
use super::*;

const REMOTE: &str = "https://synthetic.invalid/direct.wav?private_token=SYNTHETIC_QUERY";

fn remote_input(dialog: &Dialog, provider: usize) {
    audio_input(dialog, 1, provider);
    set_edit(dialog.media.get(), REMOTE).unwrap();
    notify(dialog, MEDIA, EN_CHANGE_VALUE, dialog.media.get());
}

fn assert_no_remote_echo(dialog: &Dialog) {
    for control in [dialog.destination.get(), dialog.status.get()] {
        let value = text(control);
        for private in [
            REMOTE,
            "synthetic.invalid",
            "private_token",
            "SYNTHETIC_QUERY",
        ] {
            assert!(!value.contains(private));
        }
    }
}

#[test]
fn remote_native_input_preserves_explicit_languages_quality_and_remote_request() {
    let owner = Owner::new();
    let (dialog, backend, _) = owner.dialog();
    for url in [
        REMOTE,
        "HTTP://synthetic.invalid/direct.wav",
        "hTtPs://synthetic.invalid/direct.wav",
    ] {
        for (index, language) in [
            SourceLanguage::English,
            SourceLanguage::Japanese,
            SourceLanguage::French,
            SourceLanguage::German,
            SourceLanguage::Korean,
        ]
        .into_iter()
        .enumerate()
        {
            remote_input(&dialog, 2);
            set_edit(dialog.media.get(), url).unwrap();
            select(dialog.audio_language.get(), index + 1);
            select(dialog.quality.get(), 2);
            let (request, options, cloud) = dialog.read_input().unwrap();
            assert!(request.media.is_remote());
            assert!(request.subtitles.is_empty());
            assert!(request.embedded.is_none());
            assert_eq!(request.plan(), Ok(PreparationPlan::AsrRequired));
            assert_eq!(options.source_language, language);
            assert_eq!(options.provider, Some(SpeechProvider::Gemini));
            assert_eq!(options.quality, TranslationQuality::High);
            assert!(cloud);
        }
    }
    assert_eq!(backend.borrow().starts, 0);
}

#[test]
fn remote_native_disclosure_precedes_start_and_only_selected_masked_key_is_moved() {
    let owner = Owner::new();
    for (provider, endpoint, key, expected) in [
        (1, "https://api.openai.com", "SYNTHETIC_OPENAI", "openai"),
        (
            2,
            "https://generativelanguage.googleapis.com",
            "SYNTHETIC_GEMINI",
            "gemini",
        ),
    ] {
        let (dialog, backend, _) = owner.dialog();
        remote_input(&dialog, provider);
        let (selected, unused) = if provider == 1 {
            (dialog.openai_key.get(), dialog.gemini_key.get())
        } else {
            (dialog.gemini_key.get(), dialog.openai_key.get())
        };
        set_edit(selected, key).unwrap();
        set_edit(unused, &"X".repeat(KEY_LIMIT + 1)).unwrap();
        let disclosure = text(dialog.destination.get());
        for expected in [
            endpoint,
            "連線至輸入的 HTTP(S) WAVE 網址",
            "短片段媒體音訊",
            "辨識原文",
            "全文脈絡摘要",
            "專名／詞彙",
            "翻譯前後文",
            "同一個所選服務與金鑰",
            "音訊不存檔",
        ] {
            assert!(disclosure.contains(expected));
        }
        assert_no_remote_echo(&dialog);
        assert_eq!(backend.borrow().starts, 0);
        for control in [dialog.openai_key.get(), dialog.gemini_key.get()] {
            // SAFETY: read the scalar style of this test thread's owned EDIT.
            assert_ne!(
                unsafe { GetWindowLongW(control, GWL_STYLE) } as u32 & ES_PASSWORD as u32,
                0
            );
        }
        click(&dialog, PREPARE);
        assert_eq!(backend.borrow().starts, 1);
        assert_eq!(backend.borrow().plan, Some(PreparationPlan::AsrRequired));
        assert_eq!(backend.borrow().supplied_key, Some(expected));
        assert_keys_cleared(&dialog);
        assert_no_remote_echo(&dialog);
    }
}

#[test]
fn remote_native_invalid_sidecar_is_rejected_without_ignoring_it_or_reading_keys() {
    let owner = Owner::new();
    let (dialog, backend, _) = owner.dialog();
    for (format, subtitle, language) in [
        (1, "https://synthetic.invalid/private.srt", 1),
        (2, "   ", 4),
        (1, "relative.srt", 3),
        (2, r"\\server\share\private.vtt", 4),
    ] {
        remote_input(&dialog, 1);
        select(dialog.format.get(), format);
        select(dialog.language.get(), language);
        set_edit(dialog.subtitle.get(), subtitle).unwrap();
        dialog.fields_changed().unwrap();
        let disclosure = text(dialog.destination.get());
        assert!(disclosure.contains("僅讀取媒體標頭"));
        assert!(disclosure.contains("不向 AI 傳送媒體音訊或來源網址"));
        set_edit(dialog.openai_key.get(), &"X".repeat(KEY_LIMIT + 1)).unwrap();
        set_edit(dialog.gemini_key.get(), "invalid unused key").unwrap();
        click(&dialog, PREPARE);
        assert_eq!(
            dialog.local_error.get(),
            Some(PreparedUiError::Media(MediaError::InvalidInput))
        );
        assert_eq!(backend.borrow().starts, 0);
        assert_eq!(text(dialog.subtitle.get()), subtitle);
        assert_eq!(selected(dialog.format.get()), format as isize);
        assert_keys_cleared(&dialog);
        assert_no_remote_echo(&dialog);
    }
}

#[test]
fn remote_native_embedded_mode_rejects_even_missing_metadata_before_key_read() {
    let owner = Owner::new();
    let (dialog, backend, _) = owner.dialog();
    for (subtitle, language) in [("", 0), ("3", 4), ("invalid", 1)] {
        remote_input(&dialog, 0);
        select(dialog.format.get(), EMBEDDED_FORMAT_INDEX as usize);
        select(dialog.language.get(), language);
        set_edit(dialog.subtitle.get(), subtitle).unwrap();
        dialog.fields_changed().unwrap();
        let disclosure = text(dialog.destination.get());
        assert!(disclosure.contains("不支援內嵌字幕"));
        assert!(disclosure.contains("不會連線或傳送音訊"));
        set_edit(dialog.openai_key.get(), &"X".repeat(KEY_LIMIT + 1)).unwrap();
        set_edit(dialog.gemini_key.get(), &"X".repeat(KEY_LIMIT + 1)).unwrap();
        click(&dialog, PREPARE);
        assert_eq!(
            dialog.local_error.get(),
            Some(PreparedUiError::Media(MediaError::UnsupportedUrl))
        );
        assert_eq!(backend.borrow().starts, 0);
        assert_eq!(text(dialog.subtitle.get()), subtitle);
        assert_keys_cleared(&dialog);
        assert_no_remote_echo(&dialog);
        select(dialog.format.get(), 0);
    }
}

fn remote_sidecar(dialog: &Dialog, language: usize, provider: usize, format: usize) {
    remote_input(dialog, provider);
    select(dialog.format.get(), format);
    select(dialog.language.get(), language);
    select(dialog.audio_language.get(), 0);
    set_edit(dialog.subtitle.get(), r"C:\Synthetic\manual subtitle.srt").unwrap();
    dialog.fields_changed().unwrap();
}

#[test]
fn remote_native_chinese_sidecar_needs_no_provider_audio_language_or_key() {
    let owner = Owner::new();
    for language in 1..=3 {
        let (dialog, backend, _) = owner.dialog();
        remote_sidecar(&dialog, language, 0, 1);
        set_edit(dialog.openai_key.get(), &"X".repeat(KEY_LIMIT + 1)).unwrap();
        set_edit(dialog.gemini_key.get(), &"X".repeat(KEY_LIMIT + 1)).unwrap();
        let disclosure = text(dialog.destination.get());
        for expected in [
            "連線至輸入的 HTTP(S) WAVE 網址",
            "僅讀取媒體標頭",
            "在本機進行台灣中文本地化",
            "不使用 AI 或金鑰",
            "不向 AI 傳送媒體音訊或來源網址",
        ] {
            assert!(disclosure.contains(expected));
        }
        assert!(!disclosure.contains("AI 目的地"));
        let (request, options, cloud) = dialog.read_input().unwrap();
        assert!(request.media.is_remote());
        assert_eq!(request.plan(), Ok(PreparationPlan::Subtitle(0)));
        assert!(request.embedded.is_none());
        assert_eq!(options.provider, None);
        assert_eq!(options.source_language, SourceLanguage::Auto);
        assert!(!cloud);
        click(&dialog, PREPARE);
        assert_eq!(backend.borrow().starts, 1);
        assert_eq!(backend.borrow().plan, Some(PreparationPlan::Subtitle(0)));
        assert_eq!(backend.borrow().supplied_key, None);
        assert_keys_cleared(&dialog);
        assert_no_remote_echo(&dialog);
    }
}

#[test]
fn remote_native_source_sidecar_discloses_text_only_and_moves_only_selected_masked_key() {
    let owner = Owner::new();
    for (provider, endpoint, key, expected) in [
        (1, "https://api.openai.com", "SYNTHETIC_OPENAI", "openai"),
        (
            2,
            "https://generativelanguage.googleapis.com",
            "SYNTHETIC_GEMINI",
            "gemini",
        ),
    ] {
        for format in [1, 2] {
            for language in 4..=8 {
                let (dialog, backend, _) = owner.dialog();
                remote_sidecar(&dialog, language, provider, format);
                select(dialog.quality.get(), 2);
                let (selected_key, unused) = if provider == 1 {
                    (dialog.openai_key.get(), dialog.gemini_key.get())
                } else {
                    (dialog.gemini_key.get(), dialog.openai_key.get())
                };
                set_edit(selected_key, key).unwrap();
                set_edit(unused, &"X".repeat(KEY_LIMIT + 1)).unwrap();
                let disclosure = text(dialog.destination.get());
                for expected in [
                    endpoint,
                    "僅讀取媒體標頭",
                    "所選本機原文字幕",
                    "全文脈絡摘要",
                    "專名／詞彙",
                    "翻譯前後文",
                    "不向 AI 傳送媒體音訊或來源網址",
                ] {
                    assert!(disclosure.contains(expected));
                }
                assert!(!disclosure.contains("短片段媒體音訊"));
                let (request, options, cloud) = dialog.read_input().unwrap();
                assert!(request.media.is_remote());
                assert_eq!(request.plan(), Ok(PreparationPlan::Subtitle(0)));
                assert_eq!(
                    request.subtitles[0].format,
                    if format == 1 {
                        SubtitleFormat::Srt
                    } else {
                        SubtitleFormat::WebVtt
                    }
                );
                assert_eq!(options.source_language, SourceLanguage::Auto);
                assert_eq!(options.quality, TranslationQuality::High);
                assert!(cloud);
                for control in [dialog.openai_key.get(), dialog.gemini_key.get()] {
                    // SAFETY: scalar style query of the fixture's owned EDIT.
                    assert_ne!(
                        unsafe { GetWindowLongW(control, GWL_STYLE) } as u32 & ES_PASSWORD as u32,
                        0
                    );
                }
                click(&dialog, PREPARE);
                assert_eq!(backend.borrow().starts, 1);
                assert_eq!(backend.borrow().plan, Some(PreparationPlan::Subtitle(0)));
                assert_eq!(backend.borrow().supplied_key, Some(expected));
                assert_keys_cleared(&dialog);
                assert_no_remote_echo(&dialog);
            }
        }
    }
}

#[test]
fn remote_native_sidecar_missing_metadata_or_service_rejects_before_key_read() {
    let owner = Owner::new();
    let (dialog, backend, _) = owner.dialog();
    for (language, format, provider, error) in [
        (0, 1, 1, PreparedUiError::Media(MediaError::InvalidSubtitle)),
        (4, 0, 1, PreparedUiError::Media(MediaError::InvalidSubtitle)),
        (4, 1, 0, PreparedUiError::ProviderRequired),
    ] {
        remote_sidecar(&dialog, language, provider, format);
        set_edit(dialog.openai_key.get(), &"X".repeat(KEY_LIMIT + 1)).unwrap();
        set_edit(dialog.gemini_key.get(), &"X".repeat(KEY_LIMIT + 1)).unwrap();
        click(&dialog, PREPARE);
        assert_eq!(dialog.local_error.get(), Some(error));
        assert_eq!(backend.borrow().starts, 0);
        assert_eq!(
            text(dialog.subtitle.get()),
            r"C:\Synthetic\manual subtitle.srt"
        );
        assert_keys_cleared(&dialog);
        assert_no_remote_echo(&dialog);
    }
}

#[test]
fn remote_native_discovery_is_disabled_and_direct_command_does_not_start_scan() {
    let owner = Owner::new();
    let (dialog, backend, _) = owner.dialog();
    remote_input(&dialog, 0);
    let discover = dialog.controls.borrow()[DISCOVER_CONTROL_INDEX];
    // SAFETY: scalar enabled-state queries target this fixture's live controls.
    assert!(!unsafe { IsWindowEnabled(discover) }.as_bool());
    assert!(text(dialog.destination.get()).contains("AI 目的地尚未選擇"));
    set_edit(dialog.openai_key.get(), "SYNTHETIC_OPENAI").unwrap();
    set_edit(dialog.gemini_key.get(), "SYNTHETIC_GEMINI").unwrap();
    click(&dialog, DISCOVER); // Also exercises the direct guarded notification.
    assert_eq!(dialog.discovery.borrow().state(), DiscoveryState::Idle);
    assert_eq!(
        dialog.local_error.get(),
        Some(PreparedUiError::Media(MediaError::UnsupportedUrl))
    );
    assert_eq!(backend.borrow().starts, 0);
    assert!(dialog.tracks.get().is_empty());
    assert_keys_cleared(&dialog);
    assert_no_remote_echo(&dialog);
    set_edit(dialog.media.get(), r"C:\Synthetic\media.mp4").unwrap();
    notify(&dialog, MEDIA, EN_CHANGE_VALUE, dialog.media.get());
    assert!(unsafe { IsWindowEnabled(discover) }.as_bool());
    assert_eq!(dialog.discovery.borrow().state(), DiscoveryState::Idle);
}

#[test]
fn remote_native_source_edit_retires_scanned_metadata_without_removing_manual_subtitle() {
    let owner = Owner::new();
    let (dialog, backend, _) = owner.dialog();
    audio_input(&dialog, 1, 1);
    dialog.discovery_applied.set(true);
    select(dialog.format.get(), EMBEDDED_FORMAT_INDEX as usize);
    set_edit(dialog.subtitle.get(), "3").unwrap();
    set_edit(dialog.media.get(), REMOTE).unwrap();
    notify(&dialog, MEDIA, EN_CHANGE_VALUE, dialog.media.get());
    assert!(!dialog.discovery_applied.get());
    assert!(text(dialog.subtitle.get()).is_empty());
    assert_eq!(selected(dialog.language.get()), 0);
    assert!(dialog.tracks.get().is_empty());
    // The existing embedded choice remains explicit until changed by the user.
    assert!(text(dialog.destination.get()).contains("不支援內嵌字幕"));
    select(dialog.format.get(), 1);
    select(dialog.language.get(), 1);
    set_edit(dialog.subtitle.get(), r"C:\Synthetic\manual.srt").unwrap();
    set_edit(dialog.media.get(), "https://synthetic.invalid/other.wav").unwrap();
    notify(&dialog, MEDIA, EN_CHANGE_VALUE, dialog.media.get());
    assert_eq!(text(dialog.subtitle.get()), r"C:\Synthetic\manual.srt");
    assert!(text(dialog.destination.get()).contains("僅讀取媒體標頭"));
    assert!(text(dialog.destination.get()).contains("不使用 AI 或金鑰"));
    assert_eq!(backend.borrow().starts, 0);
    assert_no_remote_echo(&dialog);
}

#[test]
fn remote_native_busy_ready_stop_and_clear_preserve_locks_and_remove_url_undo() {
    let owner = Owner::new();
    for sidecar_language in [0, 1, 4] {
        let (dialog, backend, notice) = owner.dialog();
        if sidecar_language == 0 {
            remote_input(&dialog, 1);
        } else {
            remote_sidecar(&dialog, sidecar_language, 1, 1);
        }
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
                dialog.audio_language.get(),
                dialog.provider.get(),
                dialog.quality.get(),
                dialog.openai_key.get(),
                dialog.gemini_key.get(),
                dialog.controls.borrow()[DISCOVER_CONTROL_INDEX],
            ] {
                // SAFETY: scalar query of this test thread's owned native controls.
                assert!(!unsafe { IsWindowEnabled(control) }.as_bool());
            }
            click(&dialog, PREPARE);
            assert_eq!(backend.borrow().starts, 1);
            assert_no_remote_echo(&dialog);
        }
        assert_eq!(notice.borrow().ready, 1);
        backend.borrow_mut().state = PreparedUiState::Preparing;
        set_edit(dialog.openai_key.get(), "SYNTHETIC_OPENAI").unwrap();
        click(&dialog, CANCEL);
        assert_eq!(backend.borrow().state, PreparedUiState::Stopping);
        assert_keys_cleared(&dialog);
        backend.borrow_mut().state = PreparedUiState::Cancelled;
        dialog.poll().unwrap();
        click(&dialog, CLEAR);
        assert!(text(dialog.media.get()).is_empty());
        assert!(text(dialog.subtitle.get()).is_empty());
        // SAFETY: owned EDIT query checks that clear also removed private URL undo.
        assert_eq!(
            unsafe { SendMessageW(dialog.media.get(), EM_CANUNDO, None, None).0 },
            0
        );
        assert!(!text(dialog.destination.get()).contains("連線至輸入的 HTTP(S) WAVE 網址"));
        assert_no_remote_echo(&dialog);
    }
}
