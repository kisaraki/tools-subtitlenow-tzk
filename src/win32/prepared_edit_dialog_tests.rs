//! Static text edits in synthetic MP4s, hidden owned controls, no cloud/toasts.
use super::*;

#[test]
fn mp4_text_crop_native_real_chinese_ready_and_empty_crop_failure_release_pin() {
    cropped_native_ready(crate::media::local_audio_format::mp4_cropped_subtitle_fixture);
}
#[test]
fn mp4_text_v1_native_real_chinese_ready_and_empty_crop_failure_release_pin() {
    cropped_native_ready(crate::media::local_audio_format::mp4_v1_subtitle_fixture);
}
fn cropped_native_ready(make_fixture: fn(TextLanguage, u32, u32, u32) -> Vec<u8>) {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("synthetic-crop.data");
    let owner = Owner::new();
    let notice = Rc::new(RefCell::new(NoticeState::default()));
    let dialog = Dialog::new(
        Box::new(crate::app::prepared::PreparedController::new()),
        Box::new(MockNotification(notice.clone())),
    );
    dialog.create(owner.0, owner.1, false).unwrap();
    for (start, duration, expected) in [
        (1100, 500, PreparedUiState::Ready { cues: 1 }),
        (
            700,
            400,
            PreparedUiState::Failed(PreparedUiError::Media(MediaError::InvalidSubtitle)),
        ),
    ] {
        let bytes = make_fixture(TextLanguage::SimplifiedChinese, start, duration, 250);
        std::fs::write(&path, &bytes).unwrap();
        set_edit(dialog.media.get(), path.to_str().unwrap()).unwrap();
        discover_delayed_track(&dialog);
        select(dialog.language.get(), 3);
        dialog.fields_changed().unwrap();
        assert!(text(dialog.destination.get()).contains("不使用雲端"));
        click(&dialog, PREPARE);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while busy(dialog.backend.borrow().state()) {
            dialog.poll().unwrap();
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!(dialog.backend.borrow().state(), expected);
        assert_eq!(notice.borrow().ready, 1); // Empty visible interval must not notify Ready.
        assert_keys_cleared(&dialog);
        click(&dialog, CLEAR);
        assert!(std::fs::OpenOptions::new().write(true).open(&path).is_ok());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
    assert_eq!(std::fs::read_dir(fixture.path()).unwrap().count(), 1);
    click(&dialog, CLOSE);
}

fn discover_delayed_track(dialog: &Dialog) {
    click(dialog, DISCOVER);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while dialog.discovery.borrow().state().busy() {
        dialog.poll().unwrap();
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert!(matches!(
        dialog.discovery.borrow().state(),
        DiscoveryState::Found(_)
    ));
    assert_eq!(dialog.tracks.get().len(), 1);
    assert_eq!(selected(dialog.track.get()), 1);
    assert_eq!(selected(dialog.format.get()), EMBEDDED_FORMAT_INDEX);
    assert_eq!(text(dialog.subtitle.get()), "3");
}

#[test]
fn mp4_text_edit_native_chinese_discovery_and_real_ready_keep_pin_until_clear() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("synthetic-delayed-text.data");
    let bytes = crate::media::local_audio_format::mp4_delayed_subtitle_fixture(
        TextLanguage::SimplifiedChinese,
        500,
    );
    std::fs::write(&path, &bytes).unwrap();
    let owner = Owner::new();
    let notice = Rc::new(RefCell::new(NoticeState::default()));
    let dialog = Dialog::new(
        Box::new(crate::app::prepared::PreparedController::new()),
        Box::new(MockNotification(notice.clone())),
    );
    dialog.create(owner.0, owner.1, false).unwrap();
    set_edit(dialog.media.get(), path.to_str().unwrap()).unwrap();
    discover_delayed_track(&dialog);
    assert_eq!(selected(dialog.language.get()), 0); // zho is not a script/region.
    assert_eq!(dialog.backend.borrow().state(), PreparedUiState::Idle);
    assert_eq!(notice.borrow().ready, 0);
    // Metadata discovery finishes without retaining a media pin or Ready owner.
    assert!(std::fs::OpenOptions::new().write(true).open(&path).is_ok());
    select(dialog.language.get(), 3);
    select(dialog.provider.get(), 0); // No service or stored credential is needed.
    notify(
        &dialog,
        LANGUAGE,
        CBN_SELCHANGE as usize,
        dialog.language.get(),
    );
    set_edit(dialog.openai_key.get(), "invalid synthetic unused").unwrap();
    set_edit(dialog.gemini_key.get(), &"X".repeat(KEY_LIMIT + 1)).unwrap();
    assert!(text(dialog.destination.get()).contains("不使用雲端"));
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
    assert!(text(dialog.status.get()).contains("共 2 段"));
    assert!(text(dialog.status.get()).contains("可保留返回"));
    assert_keys_cleared(&dialog);
    assert_eq!(notice.borrow().ready, 1); // Mock completion callback, not a real toast.
    dialog.poll().unwrap();
    assert_eq!(notice.borrow().ready, 1);
    assert!(std::fs::OpenOptions::new().write(true).open(&path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(std::fs::read_dir(fixture.path()).unwrap().count(), 1);
    click(&dialog, CLEAR);
    assert_eq!(dialog.backend.borrow().state(), PreparedUiState::Cancelled);
    assert!(std::fs::OpenOptions::new().write(true).open(&path).is_ok());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(std::fs::read_dir(fixture.path()).unwrap().count(), 1);
    click(&dialog, CLOSE);
    assert!(dialog.hwnd.get().0.is_null());
}

#[test]
fn mp4_text_edit_native_source_discovery_preserves_text_disclosure_and_selected_key() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("synthetic-delayed-source.data");
    let bytes = crate::media::local_audio_format::mp4_delayed_subtitle_fixture(
        TextLanguage::Source(SpeechLanguage::English),
        500,
    );
    std::fs::write(&path, &bytes).unwrap();
    let owner = Owner::new();
    for provider in 1..=2 {
        let (dialog, backend, notice) = owner.dialog();
        set_edit(dialog.media.get(), path.to_str().unwrap()).unwrap();
        set_edit(dialog.openai_key.get(), "invalid synthetic unused").unwrap();
        set_edit(dialog.gemini_key.get(), &"X".repeat(KEY_LIMIT + 1)).unwrap();
        discover_delayed_track(&dialog);
        assert_eq!(selected(dialog.language.get()), 4);
        assert_eq!(backend.borrow().starts, 0);
        assert_eq!(backend.borrow().state, PreparedUiState::Idle);
        assert_eq!(notice.borrow().ready, 0);
        assert_keys_cleared(&dialog);
        assert!(std::fs::OpenOptions::new().write(true).open(&path).is_ok());
        select(dialog.language.get(), 4);
        select(dialog.provider.get(), provider);
        notify(
            &dialog,
            PROVIDER,
            CBN_SELCHANGE as usize,
            dialog.provider.get(),
        );
        let (selected_key, unused, value, destination, expected_key, expected_provider) =
            if provider == 1 {
                (
                    dialog.openai_key.get(),
                    dialog.gemini_key.get(),
                    "SYNTHETIC_OPENAI",
                    "https://api.openai.com",
                    "openai",
                    SpeechProvider::OpenAi,
                )
            } else {
                (
                    dialog.gemini_key.get(),
                    dialog.openai_key.get(),
                    "SYNTHETIC_GEMINI",
                    "https://generativelanguage.googleapis.com",
                    "gemini",
                    SpeechProvider::Gemini,
                )
            };
        let disclosure = text(dialog.destination.get());
        assert!(disclosure.contains(destination));
        assert!(disclosure.contains("不傳送媒體音訊"));
        assert!(disclosure.contains("全文脈絡摘要"));
        assert!(!disclosure.contains("短片段媒體音訊"));
        set_edit(selected_key, value).unwrap();
        set_edit(unused, &"X".repeat(KEY_LIMIT + 1)).unwrap();
        click(&dialog, PREPARE);
        assert_eq!(backend.borrow().starts, 1);
        assert_eq!(backend.borrow().plan, Some(PreparationPlan::EmbeddedMp4));
        assert_eq!(backend.borrow().supplied_key, Some(expected_key));
        assert_eq!(
            backend.borrow().options.unwrap().provider,
            Some(expected_provider)
        );
        assert_eq!(
            backend.borrow().options.unwrap().quality,
            TranslationQuality::Standard
        );
        assert_keys_cleared(&dialog);
        assert_eq!(notice.borrow().ready, 0);
        backend.borrow_mut().stopping_polls = Some(0);
        click(&dialog, CANCEL);
        dialog.poll().unwrap();
        assert_eq!(backend.borrow().state, PreparedUiState::Cancelled);
        assert_eq!(backend.borrow().stops, 1);
        click(&dialog, CLEAR);
        click(&dialog, CLOSE);
        assert!(dialog.hwnd.get().0.is_null());
    }
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(std::fs::read_dir(fixture.path()).unwrap().count(), 1);
}
