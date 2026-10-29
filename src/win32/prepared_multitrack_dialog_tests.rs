//! Real metadata discovery, owned hidden controls, synthetic text, no cloud.
use super::*;
fn scan(dialog: &Dialog) {
    click(dialog, DISCOVER);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while dialog.discovery.borrow().state().busy() {
        dialog.poll().unwrap();
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
}
fn choose(dialog: &Dialog, index: usize) {
    select(dialog.track.get(), index);
    notify(dialog, TRACK, CBN_SELCHANGE as usize, dialog.track.get());
}
fn count(dialog: &Dialog) -> isize {
    // SAFETY: scalar count query of this test thread's owned live combo.
    unsafe { SendMessageW(dialog.track.get(), CB_GETCOUNT, None, None).0 }
}
#[test]
fn mp4_multitrack_native_requires_choice_then_second_chinese_track_reaches_real_ready() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("synthetic.data");
    let bytes = crate::media::local_audio_format::mp4_multi_subtitle_fixture(&[
        TextLanguage::Source(SpeechLanguage::English),
        TextLanguage::SimplifiedChinese,
    ]);
    std::fs::write(&path, &bytes).unwrap();
    let owner = Owner::new();
    let notice = Rc::new(RefCell::new(NoticeState::default()));
    let dialog = Dialog::new(
        Box::new(crate::app::prepared::PreparedController::new()),
        Box::new(MockNotification(notice.clone())),
    );
    dialog.create(owner.0, owner.1, false).unwrap();
    set_edit(dialog.media.get(), path.to_str().unwrap()).unwrap();
    scan(&dialog);
    assert_eq!(count(&dialog), 3);
    assert_eq!(selected(dialog.track.get()), 0);
    assert!(text(dialog.subtitle.get()).is_empty());
    assert_eq!(selected(dialog.format.get()), EMBEDDED_FORMAT_INDEX);
    assert!(text(dialog.status.get()).contains("請從清單明確選擇"));
    assert!(!text(dialog.destination.get()).contains("傳送短片段媒體音訊"));
    click(&dialog, PREPARE);
    assert_eq!(dialog.backend.borrow().state(), PreparedUiState::Idle);
    assert_eq!(notice.borrow().ready, 0);
    choose(&dialog, 2);
    assert_eq!(text(dialog.subtitle.get()), "4");
    assert_eq!(selected(dialog.language.get()), 0); // zho is not a script/region.
    select(dialog.language.get(), 3);
    dialog.fields_changed().unwrap();
    assert_eq!(dialog.tracks.get().len(), 2); // Declaration does not lose catalog.
    set_edit(dialog.openai_key.get(), "invalid synthetic unused").unwrap();
    set_edit(dialog.gemini_key.get(), "invalid synthetic unused").unwrap();
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
    assert_keys_cleared(&dialog);
    assert_eq!(notice.borrow().ready, 1);
    assert!(std::fs::OpenOptions::new().write(true).open(&path).is_err());
    click(&dialog, CLEAR);
    assert_eq!(dialog.tracks.get().len(), 0);
    assert_eq!(count(&dialog), 1);
    assert!(std::fs::OpenOptions::new().write(true).open(&path).is_ok());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(std::fs::read_dir(fixture.path()).unwrap().count(), 1);
    click(&dialog, CLOSE);
}
#[test]
fn mp4_multitrack_native_all_choices_retain_catalog_manual_id_and_path_changes_retire_selection() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("synthetic.data");
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
    std::fs::write(
        &path,
        crate::media::local_audio_format::mp4_multi_subtitle_fixture(&languages),
    )
    .unwrap();
    let owner = Owner::new();
    let (dialog, backend, notice) = owner.dialog();
    set_edit(dialog.media.get(), path.to_str().unwrap()).unwrap();
    scan(&dialog);
    assert_eq!(count(&dialog), 9);
    assert_eq!(selected(dialog.track.get()), 0);
    for (index, expected_language) in [0, 0, 0, 4, 5, 6, 7, 8].into_iter().enumerate() {
        choose(&dialog, index + 1);
        assert_eq!(text(dialog.subtitle.get()), (index + 3).to_string());
        assert_eq!(selected(dialog.language.get()), expected_language);
        assert_eq!(dialog.tracks.get().len(), 8);
    }
    choose(&dialog, 0);
    assert!(text(dialog.subtitle.get()).is_empty());
    assert_eq!(selected(dialog.language.get()), 0);
    choose(&dialog, 2);
    set_edit(dialog.subtitle.get(), "10").unwrap();
    notify(&dialog, SUBTITLE, EN_CHANGE_VALUE, dialog.subtitle.get());
    assert_eq!(selected(dialog.track.get()), 0);
    assert_eq!(dialog.tracks.get().len(), 8);
    choose(&dialog, 8);
    set_edit(
        dialog.media.get(),
        fixture.path().join("other.data").to_str().unwrap(),
    )
    .unwrap();
    notify(&dialog, MEDIA, EN_CHANGE_VALUE, dialog.media.get());
    assert!(text(dialog.subtitle.get()).is_empty());
    assert_eq!(count(&dialog), 1);
    assert!(dialog.tracks.get().is_empty());
    assert_eq!(backend.borrow().starts, 0);
    assert_eq!(notice.borrow().ready, 0);
    click(&dialog, CLOSE);
}
#[test]
fn mp4_multitrack_native_failed_rescan_never_leaves_old_choices_or_partial_catalog() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("synthetic.data");
    let owner = Owner::new();
    let (dialog, backend, _) = owner.dialog();
    std::fs::write(
        &path,
        crate::media::local_audio_format::mp4_multi_subtitle_fixture(
            &[TextLanguage::TaiwanTraditional; 2],
        ),
    )
    .unwrap();
    set_edit(dialog.media.get(), path.to_str().unwrap()).unwrap();
    scan(&dialog);
    choose(&dialog, 2);
    std::fs::write(
        &path,
        crate::media::local_audio_format::mp4_multi_subtitle_fixture(
            &[TextLanguage::TaiwanTraditional; 9],
        ),
    )
    .unwrap();
    scan(&dialog);
    assert_eq!(
        dialog.discovery.borrow().state(),
        DiscoveryState::Failed(MediaError::TooLarge)
    );
    assert!(dialog.tracks.get().is_empty());
    assert!(text(dialog.subtitle.get()).is_empty());
    assert_eq!(count(&dialog), 1);
    assert_eq!(backend.borrow().starts, 0);
    click(&dialog, CLOSE);
    std::fs::remove_file(path).unwrap();
}
