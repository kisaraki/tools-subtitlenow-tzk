//! Hidden native controls, synthetic text/keys and explicit notice fixtures.
use super::*;

fn wait_discovery(dialog: &Dialog) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while dialog.discovery.borrow().state().busy() {
        dialog.poll().unwrap();
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
}
#[test]
fn mp4_discovery_native_chinese_scan_requires_declaration_then_real_prepared_ready() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("synthetic.data");
    let bytes =
        crate::media::local_audio_format::mp4_prepared_fixture(TextLanguage::SimplifiedChinese);
    std::fs::write(&path, &bytes).unwrap();
    let owner = Owner::new();
    let notice = Rc::new(RefCell::new(NoticeState::default()));
    let dialog = Dialog::new(
        Box::new(crate::app::prepared::PreparedController::new()),
        Box::new(MockNotification(notice.clone())),
    );
    dialog.create(owner.0, owner.1, false).unwrap();
    set_edit(dialog.media.get(), path.to_str().unwrap()).unwrap();
    select(dialog.language.get(), 1); // Do not carry a previous Chinese declaration forward.
    set_edit(dialog.openai_key.get(), "SYNTHETIC_UNUSED").unwrap();
    click(&dialog, DISCOVER);
    assert!(dialog.discovery.borrow().state().busy());
    assert!(text(dialog.destination.get()).contains("不使用金鑰或雲端"));
    assert_keys_cleared(&dialog);
    click(&dialog, PREPARE); // Disabled and guarded, no competing preparation.
    assert_eq!(dialog.backend.borrow().state(), PreparedUiState::Idle);
    wait_discovery(&dialog);
    assert_eq!(text(dialog.subtitle.get()), "3");
    assert_eq!(selected(dialog.format.get()), EMBEDDED_FORMAT_INDEX);
    assert_eq!(selected(dialog.language.get()), 0);
    assert!(text(dialog.status.get()).contains("請指定繁簡類型"));
    assert_eq!(notice.borrow().ready, 0);
    assert!(std::fs::OpenOptions::new().write(true).open(&path).is_ok());
    click(&dialog, PREPARE);
    assert_eq!(dialog.backend.borrow().state(), PreparedUiState::Idle);
    select(dialog.language.get(), 3);
    dialog.fields_changed().unwrap();
    click(&dialog, PREPARE);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while busy(dialog.backend.borrow().state()) {
        dialog.poll().unwrap();
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert_eq!(
        dialog.backend.borrow().state(),
        PreparedUiState::Ready { cues: 2 }
    );
    assert_eq!(notice.borrow().ready, 1);
    assert!(std::fs::OpenOptions::new().write(true).open(&path).is_err());
    click(&dialog, CLEAR);
    assert_eq!(dialog.discovery.borrow().state(), DiscoveryState::Idle);
    assert!(std::fs::OpenOptions::new().write(true).open(&path).is_ok());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(std::fs::read_dir(fixture.path()).unwrap().count(), 1);
    click(&dialog, CLOSE);
}
#[test]
fn mp4_discovery_native_source_hint_failure_clear_and_close_do_not_start_provider() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("synthetic.data");
    let owner = Owner::new();
    let (dialog, backend, notice) = owner.dialog();
    for (language, index) in [
        (SpeechLanguage::English, 4),
        (SpeechLanguage::Japanese, 5),
        (SpeechLanguage::French, 6),
        (SpeechLanguage::German, 7),
        (SpeechLanguage::Korean, 8),
    ] {
        std::fs::write(
            &path,
            crate::media::local_audio_format::mp4_prepared_fixture(TextLanguage::Source(language)),
        )
        .unwrap();
        set_edit(dialog.media.get(), path.to_str().unwrap()).unwrap();
        click(&dialog, DISCOVER);
        wait_discovery(&dialog);
        assert_eq!(selected(dialog.language.get()), index);
        assert_eq!(text(dialog.subtitle.get()), "3");
        assert!(text(dialog.destination.get()).contains("尚未選擇"));
        assert_eq!(backend.borrow().starts, 0);
        assert_eq!(notice.borrow().ready, 0);
        // Editing the source retires the prior metadata label.
        notify(&dialog, MEDIA, EN_CHANGE_VALUE, dialog.media.get());
        assert_eq!(dialog.discovery.borrow().state(), DiscoveryState::Idle);
        assert!(text(dialog.subtitle.get()).is_empty());
        assert_eq!(selected(dialog.language.get()), 0);
    }
    std::fs::write(&path, b"invalid synthetic media").unwrap();
    click(&dialog, DISCOVER);
    wait_discovery(&dialog);
    assert!(matches!(
        dialog.discovery.borrow().state(),
        DiscoveryState::Failed(MediaError::UnsupportedMedia)
    ));
    assert!(text(dialog.status.get()).contains("字幕軌探索"));
    click(&dialog, CLEAR);
    assert!(text(dialog.subtitle.get()).is_empty());
    std::fs::write(
        &path,
        crate::media::local_audio_format::mp4_subtitle_fixture(),
    )
    .unwrap();
    set_edit(dialog.media.get(), path.to_str().unwrap()).unwrap();
    click(&dialog, DISCOVER);
    click(&dialog, CLOSE);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !dialog.hwnd.get().0.is_null() {
        dialog.poll().unwrap();
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert_eq!(backend.borrow().starts, 0);
    assert_eq!(notice.borrow().ready, 0);
    std::fs::remove_file(path).unwrap();
}
#[test]
fn mp4_discovery_native_path_change_cancels_pending_result_before_any_form_update() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("synthetic.data");
    std::fs::write(
        &path,
        crate::media::local_audio_format::mp4_subtitle_fixture(),
    )
    .unwrap();
    let owner = Owner::new();
    let (dialog, backend, notice) = owner.dialog();
    set_edit(dialog.media.get(), path.to_str().unwrap()).unwrap();
    click(&dialog, DISCOVER);
    // No poll yet: either pending or buffered success must be retired. Source
    // controls are disabled for real input; emulate a programmatic edit.
    set_edit(
        dialog.media.get(),
        fixture.path().join("missing.data").to_str().unwrap(),
    )
    .unwrap();
    notify(&dialog, MEDIA, EN_CHANGE_VALUE, dialog.media.get());
    wait_discovery(&dialog);
    assert_eq!(dialog.discovery.borrow().state(), DiscoveryState::Cancelled);
    assert!(text(dialog.subtitle.get()).is_empty());
    assert_eq!(selected(dialog.format.get()), 0);
    assert_eq!(backend.borrow().starts, 0);
    assert_eq!(notice.borrow().ready, 0);
    click(&dialog, DISCOVER);
    wait_discovery(&dialog);
    assert!(matches!(
        dialog.discovery.borrow().state(),
        DiscoveryState::Failed(MediaError::Unavailable)
    ));
    assert!(text(dialog.subtitle.get()).is_empty());
    click(&dialog, CLEAR);
    click(&dialog, CLOSE);
    std::fs::remove_file(path).unwrap();
}
fn embedded_input(dialog: &Dialog, language: usize, provider: usize) {
    fill(dialog, language, provider);
    select(dialog.format.get(), EMBEDDED_FORMAT_INDEX as usize);
    set_edit(dialog.subtitle.get(), "3").unwrap();
    notify(dialog, FORMAT, CBN_SELCHANGE as usize, dialog.format.get());
}
#[test]
fn embedded_prepared_native_mode_can_be_selected_with_empty_text_without_audio_upload() {
    let owner = Owner::new();
    let (dialog, state, _) = owner.dialog();
    set_edit(dialog.media.get(), r"C:\Synthetic\media.mp4").unwrap();
    select(dialog.language.get(), 1);
    select(dialog.format.get(), EMBEDDED_FORMAT_INDEX as usize);
    notify(&dialog, FORMAT, CBN_SELCHANGE as usize, dialog.format.get());
    assert!(dialog.has_subtitle());
    // SAFETY: these scalar queries use this test thread's still-live, owned
    // child HWNDs; no pointer, resource ownership or borrowed buffer escapes.
    assert!(unsafe { IsWindowEnabled(dialog.format.get()) }.as_bool());
    assert!(unsafe { IsWindowEnabled(dialog.language.get()) }.as_bool());
    assert!(!unsafe { IsWindowEnabled(dialog.audio_language.get()) }.as_bool());
    assert!(text(dialog.destination.get()).contains("不使用雲端"));
    assert!(!text(dialog.destination.get()).contains("短片段媒體音訊"));
    click(&dialog, PREPARE);
    assert_eq!(state.borrow().starts, 0);
    assert!(!text(dialog.status.get()).contains(AUTO_NOTICE));
    set_edit(dialog.subtitle.get(), "3").unwrap();
    assert_eq!(
        dialog.read_input().unwrap().0.plan(),
        Ok(PreparationPlan::EmbeddedMp4)
    );
    select(dialog.format.get(), 1);
    set_edit(dialog.subtitle.get(), "").unwrap();
    notify(&dialog, FORMAT, CBN_SELCHANGE as usize, dialog.format.get());
    assert!(!dialog.has_subtitle());
    assert!(text(dialog.destination.get()).contains("短片段媒體音訊"));
}
#[test]
fn embedded_prepared_native_exact_track_id_and_explicit_language_are_required() {
    let owner = Owner::new();
    for value in [
        "",
        "0",
        "-3",
        "3 ",
        " 3",
        "３",
        "4294967296",
        r"C:\Synthetic\text.srt",
    ] {
        let (dialog, state, _) = owner.dialog();
        embedded_input(&dialog, 1, 1);
        set_edit(dialog.subtitle.get(), value).unwrap();
        set_edit(dialog.openai_key.get(), "invalid synthetic key").unwrap();
        click(&dialog, PREPARE);
        assert_eq!(state.borrow().starts, 0);
        assert_keys_cleared(&dialog);
        assert!(!text(dialog.status.get()).contains(AUTO_NOTICE));
    }
    let (dialog, state, _) = owner.dialog();
    embedded_input(&dialog, 0, 1);
    click(&dialog, PREPARE);
    assert_eq!(state.borrow().starts, 0);
}
#[test]
fn embedded_prepared_native_all_languages_and_qualities_keep_audio_auto_independent() {
    let owner = Owner::new();
    let (dialog, state, _) = owner.dialog();
    for language in 1..=8 {
        for quality in 0..=2 {
            embedded_input(&dialog, language, 1);
            select(dialog.quality.get(), quality);
            select(dialog.audio_language.get(), 0);
            let (request, options, cloud) = dialog.read_input().unwrap();
            assert_eq!(request.plan(), Ok(PreparationPlan::EmbeddedMp4));
            assert_eq!(request.embedded.unwrap().track_id(), 3);
            assert!(request.subtitles.is_empty());
            assert_eq!(options.source_language, SourceLanguage::Auto);
            assert_eq!(
                options.quality,
                [
                    TranslationQuality::Fast,
                    TranslationQuality::Standard,
                    TranslationQuality::High
                ][quality]
            );
            assert_eq!(cloud, language >= 4);
        }
    }
    assert_eq!(state.borrow().starts, 0);
}
#[test]
fn embedded_prepared_native_chinese_never_reads_passwords_and_source_moves_only_selected_key() {
    let owner = Owner::new();
    for language in 1..=3 {
        let (dialog, state, _) = owner.dialog();
        embedded_input(&dialog, language, 0);
        set_edit(dialog.openai_key.get(), "invalid synthetic key").unwrap();
        set_edit(dialog.gemini_key.get(), &"X".repeat(KEY_LIMIT + 1)).unwrap();
        click(&dialog, PREPARE);
        assert_eq!(state.borrow().starts, 1);
        assert_eq!(state.borrow().plan, Some(PreparationPlan::EmbeddedMp4));
        assert_eq!(state.borrow().supplied_key, None);
        assert_keys_cleared(&dialog);
    }
    for provider in 1..=2 {
        let (dialog, state, _) = owner.dialog();
        embedded_input(&dialog, 4, provider);
        let (selected_key, unused, value) = if provider == 1 {
            (
                dialog.openai_key.get(),
                dialog.gemini_key.get(),
                "SYNTHETIC_OPENAI",
            )
        } else {
            (
                dialog.gemini_key.get(),
                dialog.openai_key.get(),
                "SYNTHETIC_GEMINI",
            )
        };
        set_edit(selected_key, value).unwrap();
        set_edit(unused, &"X".repeat(KEY_LIMIT + 1)).unwrap();
        assert!(text(dialog.destination.get()).contains("不傳送媒體音訊"));
        assert!(!text(dialog.destination.get()).contains("短片段媒體音訊"));
        click(&dialog, PREPARE);
        assert_eq!(state.borrow().plan, Some(PreparationPlan::EmbeddedMp4));
        assert_eq!(
            state.borrow().supplied_key,
            Some(if provider == 1 { "openai" } else { "gemini" })
        );
        assert_keys_cleared(&dialog);
    }
}
#[test]
fn embedded_prepared_native_real_chinese_controller_reaches_ready_and_clear_releases_pin() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("synthetic-embedded.data");
    let bytes =
        crate::media::local_audio_format::mp4_prepared_fixture(TextLanguage::SimplifiedChinese);
    std::fs::write(&path, &bytes).unwrap();
    let owner = Owner::new();
    let notice = Rc::new(RefCell::new(NoticeState::default()));
    let dialog = Dialog::new(
        Box::new(crate::app::prepared::PreparedController::new()),
        Box::new(MockNotification(notice.clone())),
    );
    dialog.create(owner.0, owner.1, false).unwrap();
    set_edit(dialog.media.get(), path.to_str().unwrap()).unwrap();
    select(dialog.format.get(), EMBEDDED_FORMAT_INDEX as usize);
    select(dialog.language.get(), 3);
    select(dialog.provider.get(), 0); // Real controller cannot construct a service.
    set_edit(dialog.subtitle.get(), "3").unwrap();
    set_edit(dialog.openai_key.get(), "invalid synthetic key").unwrap();
    set_edit(dialog.gemini_key.get(), &"X".repeat(KEY_LIMIT + 1)).unwrap();
    dialog.fields_changed().unwrap();
    assert!(text(dialog.destination.get()).contains("不使用雲端"));
    click(&dialog, PREPARE);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while busy(dialog.backend.borrow().state()) {
        dialog.poll().unwrap();
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(
        dialog.backend.borrow().state(),
        PreparedUiState::Ready { cues: 2 }
    );
    assert!(text(dialog.status.get()).contains("共 2 段"));
    assert!(text(dialog.status.get()).contains("可保留返回"));
    assert_eq!(notice.borrow().ready, 1);
    dialog.poll().unwrap();
    assert_eq!(notice.borrow().ready, 1);
    assert_keys_cleared(&dialog);
    assert!(std::fs::OpenOptions::new().write(true).open(&path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    click(&dialog, CLEAR);
    assert_eq!(dialog.backend.borrow().state(), PreparedUiState::Cancelled);
    assert!(std::fs::OpenOptions::new().write(true).open(&path).is_ok());
    assert_eq!(std::fs::read_dir(fixture.path()).unwrap().count(), 1);
    // A wrong explicit track must fail locally, then permit a corrected run.
    set_edit(dialog.media.get(), path.to_str().unwrap()).unwrap();
    select(dialog.format.get(), EMBEDDED_FORMAT_INDEX as usize);
    select(dialog.language.get(), 3);
    select(dialog.provider.get(), 0);
    set_edit(dialog.subtitle.get(), "4").unwrap();
    click(&dialog, PREPARE);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while busy(dialog.backend.borrow().state()) {
        dialog.poll().unwrap();
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(
        dialog.backend.borrow().state(),
        PreparedUiState::Failed(PreparedUiError::Media(MediaError::UnsupportedMedia))
    );
    assert_eq!(notice.borrow().ready, 1);
    assert!(std::fs::OpenOptions::new().write(true).open(&path).is_ok());
    set_edit(dialog.subtitle.get(), "3").unwrap();
    click(&dialog, PREPARE);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while busy(dialog.backend.borrow().state()) {
        dialog.poll().unwrap();
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(
        dialog.backend.borrow().state(),
        PreparedUiState::Ready { cues: 2 }
    );
    assert_eq!(notice.borrow().ready, 2);
    click(&dialog, CLEAR);
    click(&dialog, CLOSE);
    assert!(dialog.hwnd.get().0.is_null());
}
