//! Hidden app-owned fixtures only. Synthetic URLs, no requests, no credentials.
use super::*;
use crate::{
    domain::{SourceLanguage, TranslationQuality},
    ui::controls::{ControlPreferences, ControlUpdate},
    win32::subtitle_controls::ControlPanel,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;

const EM_CANUNDO: u32 = 0x00c6;
struct Owner(HWND, HINSTANCE);
impl Owner {
    fn new() -> Self {
        // SAFETY: this thread creates a private hidden STATIC owner. It is never
        // shown, and neither real user windows nor providers are accessed.
        unsafe {
            let instance = HINSTANCE(GetModuleHandleW(None).unwrap().0);
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("Synthetic SubtitleNow settings owner"),
                WS_OVERLAPPED,
                0,
                0,
                640,
                480,
                None,
                None,
                Some(instance),
                None,
            )
            .unwrap();
            Self(hwnd, instance)
        }
    }
    fn dialog(&self, current: Option<&SubtitleSourceSettings>) -> Box<Dialog> {
        let dialog = Dialog::new();
        dialog.create(self.0, self.1, current, false).unwrap();
        dialog
    }
    fn panel(&self) -> ControlPanel {
        ControlPanel::create(
            self.0,
            self.1,
            ControlPreferences {
                provider: None,
                live_model: None,
                language: SourceLanguage::Auto,
                quality: TranslationQuality::Standard,
            },
        )
        .unwrap()
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        // SAFETY: synchronously destroy this fixture's private owned HWND.
        unsafe {
            let _ = DestroyWindow(self.0);
        }
    }
}
fn text(control: HWND) -> String {
    let mut buffer = vec![0u16; URL_LIMIT + 1];
    // SAFETY: own fixture control and writable bounded UTF-16 slice.
    let len = unsafe { GetWindowTextW(control, &mut buffer) }.max(0) as usize;
    String::from_utf16(&buffer[..len]).unwrap()
}
fn select(control: HWND, index: usize) {
    // SAFETY: scalar selection on our fixture's owned fixed combo.
    unsafe {
        SendMessageW(control, CB_SETCURSEL, Some(WPARAM(index)), None);
    }
}
fn fill(dialog: &Dialog) {
    set_edit(dialog.title.get(), "Synthetic Movie").unwrap();
    set_edit(
        dialog.url.get(),
        "https://subtitles.example.invalid/track.srt?token=SYNTHETIC_TOKEN",
    )
    .unwrap();
    select(dialog.format.get(), 1);
    select(dialog.language.get(), 1);
    dialog.refresh_destination().unwrap();
}
fn settings() -> SubtitleSourceSettings {
    SubtitleSourceSettings::from_draft(SubtitleSourceDraft {
        title: "Synthetic Movie".into(),
        url: "https://subtitles.example.invalid/track.srt?token=SYNTHETIC_TOKEN".into(),
        format: SubtitleFormat::Srt,
        language: SpeechLanguage::English,
    })
    .unwrap()
}
fn click(dialog: &Dialog, id: usize) {
    // SAFETY: synthetic button notification to this fixture's owned dialog only.
    unsafe {
        SendMessageW(dialog.hwnd.get(), WM_COMMAND, Some(WPARAM(id)), None);
    }
    assert!(!dialog.failed.get());
}

#[test]
fn fixed_fields_mask_url_and_offer_only_declared_formats_and_five_languages() {
    let owner = Owner::new();
    let dialog = owner.dialog(None);
    assert_eq!(dialog.controls.borrow().len(), 14);
    for (control, count) in [(dialog.format.get(), 3), (dialog.language.get(), 6)] {
        // SAFETY: own fixed combos; scalar count and selected index queries.
        unsafe {
            assert_eq!(SendMessageW(control, CB_GETCOUNT, None, None).0, count);
            assert_eq!(SendMessageW(control, CB_GETCURSEL, None, None).0, 0);
        }
    }
    // SAFETY: reads only this fixture's owned edit scalar style.
    assert!(
        unsafe { GetWindowLongPtrW(dialog.url.get(), GWL_STYLE) as u32 } & ES_PASSWORD as u32 != 0
    );
    let guidance = text(dialog.controls.borrow()[9]);
    assert!(guidance.contains("停止或關閉後清除"));
    assert!(guidance.contains("套用不會連線"));
    assert!(dialog.read_settings().is_err());
}

#[test]
fn applying_synthetic_source_returns_ram_settings_with_host_only_disclosure() {
    let owner = Owner::new();
    let dialog = owner.dialog(None);
    fill(&dialog);
    click(&dialog, APPLY);
    assert!(dialog.hwnd.get().0.is_null());
    let outcome = dialog.outcome.borrow_mut().take().unwrap();
    let Outcome::Apply(settings) = outcome else {
        panic!("expected settings")
    };
    assert_eq!(settings.title(), "Synthetic Movie");
    let destination = settings.destination();
    assert!(destination.contains("subtitles.example.invalid"));
    for secret in ["SYNTHETIC_TOKEN", "token", "track.srt", "?"] {
        assert!(!destination.contains(secret));
    }
    let draft = settings.draft();
    assert_eq!(draft.format, SubtitleFormat::Srt);
    assert_eq!(draft.language, SpeechLanguage::English);
}

#[test]
fn invalid_url_unknown_language_and_oversized_programmatic_values_stay_unapplied() {
    let owner = Owner::new();
    let dialog = owner.dialog(None);
    for invalid in [
        "http://subtitles.example.invalid/track.srt".to_owned(),
        "https://user:password@subtitles.example.invalid/track.srt".to_owned(),
        "x".repeat(URL_LIMIT + 1),
    ] {
        fill(&dialog);
        set_edit(dialog.url.get(), &invalid).unwrap();
        click(&dialog, APPLY);
        assert!(dialog.outcome.borrow().is_none());
        assert!(!dialog.hwnd.get().0.is_null());
        assert_eq!(text(dialog.error.get()), INVALID_NOTICE);
        assert!(!text(dialog.error.get()).contains(&invalid));
    }
    fill(&dialog);
    select(dialog.language.get(), 0);
    assert_eq!(
        dialog.read_settings().err(),
        Some(ResolverError::UnknownTextLanguage)
    );
    fill(&dialog);
    set_edit(dialog.title.get(), &"x".repeat(TITLE_LIMIT + 1)).unwrap();
    assert_eq!(
        dialog.read_settings().err(),
        Some(ResolverError::InvalidData)
    );
}

#[test]
fn prior_settings_restore_fixed_fields_and_cancel_returns_no_changes() {
    let owner = Owner::new();
    let prior = settings();
    let dialog = owner.dialog(Some(&prior));
    assert_eq!(text(dialog.title.get()), prior.title());
    assert_eq!(text(dialog.url.get()), prior.draft().url);
    // SAFETY: reads our own native undo state after copying the initial signed URL.
    assert_eq!(
        unsafe { SendMessageW(dialog.url.get(), EM_CANUNDO, None, None).0 },
        0
    );
    set_edit(dialog.title.get(), "Other Synthetic Movie").unwrap();
    click(&dialog, CANCEL);
    assert!(matches!(
        dialog.outcome.borrow().as_ref(),
        Some(Outcome::Cancel)
    ));
    assert_eq!(prior.title(), "Synthetic Movie");
}

#[test]
fn explicit_clear_and_close_cancel_are_distinct_and_clear_native_undo_storage() {
    let owner = Owner::new();
    let clear = owner.dialog(Some(&settings()));
    clear.clear_fields().unwrap();
    for field in [clear.title.get(), clear.url.get()] {
        assert!(text(field).is_empty());
        // SAFETY: reads only our own EDIT undo state after explicit secure cleanup.
        assert_eq!(unsafe { SendMessageW(field, EM_CANUNDO, None, None).0 }, 0);
    }
    click(&clear, CLEAR);
    assert!(matches!(
        clear.outcome.borrow().as_ref(),
        Some(Outcome::Clear)
    ));
    let close = owner.dialog(Some(&settings()));
    // SAFETY: close this fixture-owned hidden dialog, no external UI.
    unsafe { SendMessageW(close.hwnd.get(), WM_CLOSE, None, None) };
    assert!(matches!(
        close.outcome.borrow().as_ref(),
        Some(Outcome::Cancel)
    ));
    assert!(close.hwnd.get().0.is_null());
}

#[test]
fn all_declared_language_and_format_choices_survive_native_reading() {
    let owner = Owner::new();
    let dialog = owner.dialog(None);
    fill(&dialog);
    for language in SpeechLanguage::ALL {
        select(dialog.language.get(), language_index(language));
        for (index, format) in [(1, SubtitleFormat::Srt), (2, SubtitleFormat::WebVtt)] {
            select(dialog.format.get(), index);
            let draft = dialog.read_settings().unwrap().draft();
            assert_eq!(draft.language, language);
            assert_eq!(draft.format, format);
        }
    }
}

#[test]
fn panel_button_disables_while_busy_and_cannot_open_a_dialog_from_stale_notification() {
    let owner = Owner::new();
    let panel = owner.panel();
    assert!(panel.current_subtitle_source().is_none());
    assert!(text(panel.subtitle_notice).contains("未設定"));
    panel.enable(true, true);
    // SAFETY: scalar state read only on this fixture's owned button.
    assert!(!unsafe { IsWindowEnabled(panel.subtitle_source) }.as_bool());
    // Busy notifications must return without opening any modal window.
    panel.edit_subtitle_source(owner.0, owner.1).unwrap();
    assert!(panel.current_subtitle_source().is_none());
    panel.update(&ControlUpdate::default(), false).unwrap();
    // SAFETY: scalar state read only on this fixture's owned button.
    assert!(unsafe { IsWindowEnabled(panel.subtitle_source) }.as_bool());
}

#[test]
fn host_preview_is_readonly_and_never_contains_signed_path_or_query() {
    let owner = Owner::new();
    let dialog = owner.dialog(None);
    fill(&dialog);
    assert!(text(dialog.destination.get()).contains("https://subtitles.example.invalid"));
    for secret in ["SYNTHETIC_TOKEN", "token", "track.srt", "?"] {
        assert!(!text(dialog.destination.get()).contains(secret));
    }
    // SAFETY: reads the style of our own readonly host disclosure.
    assert_ne!(
        unsafe { GetWindowLongPtrW(dialog.destination.get(), GWL_STYLE) } & ES_READONLY as isize,
        0
    );
    set_edit(
        dialog.url.get(),
        "https://private:secret@example.invalid/track.srt",
    )
    .unwrap();
    dialog.refresh_destination().unwrap();
    assert!(!text(dialog.destination.get()).contains("secret"));
    assert!(!text(dialog.destination.get()).contains("example.invalid"));
}

#[test]
fn panel_start_carries_immutable_settings_and_completion_clears_them() {
    let owner = Owner::new();
    let panel = owner.panel();
    let choices = crate::ui::controls::choices(&crate::audio::sessions::ScanReport {
        sources: vec![crate::ui::controls::mock_source(42)],
        ..Default::default()
    });
    let sources = std::sync::Arc::new(choices);
    panel
        .update(
            &ControlUpdate {
                sources: sources.clone(),
                ..Default::default()
            },
            false,
        )
        .unwrap();
    select(panel.source, 1);
    select(panel.provider, 1);
    // GPT requires an explicit source language; this fixture tests snapshot ownership.
    select(panel.language, 1);
    panel
        .apply_subtitle_outcome(Outcome::Apply(settings()))
        .unwrap();
    let request = panel.request().unwrap();
    assert_eq!(request.language, SourceLanguage::English);
    assert_eq!(
        request.subtitle_source.as_ref().unwrap().title(),
        "Synthetic Movie"
    );
    assert!(text(panel.subtitle_notice).contains("subtitles.example.invalid"));
    assert!(!text(panel.subtitle_notice).contains("SYNTHETIC_TOKEN"));
    panel.enable(true, true);
    panel.apply_subtitle_outcome(Outcome::Clear).unwrap(); // Stale apply is ignored.
    assert!(panel.current_subtitle_source().is_some());
    panel
        .update(
            &ControlUpdate {
                sources,
                ..Default::default()
            },
            false,
        )
        .unwrap();
    assert!(panel.current_subtitle_source().is_none());
    // Running request snapshot does not change when the editable owner clears.
    assert_eq!(
        request.subtitle_source.as_ref().unwrap().title(),
        "Synthetic Movie"
    );
}

#[test]
fn source_switch_loss_and_explicit_clear_remove_session_settings() {
    let owner = Owner::new();
    let panel = owner.panel();
    panel
        .apply_subtitle_outcome(Outcome::Apply(settings()))
        .unwrap();
    panel
        .options_changed(crate::win32::subtitle_controls::PROVIDER)
        .unwrap();
    assert!(panel.current_subtitle_source().is_some());
    panel
        .options_changed(crate::win32::subtitle_controls::SOURCE)
        .unwrap();
    assert!(panel.current_subtitle_source().is_none());
    panel
        .apply_subtitle_outcome(Outcome::Apply(settings()))
        .unwrap();
    panel.apply_subtitle_outcome(Outcome::Clear).unwrap();
    assert!(panel.current_subtitle_source().is_none());
    let sources = std::sync::Arc::new(crate::ui::controls::choices(
        &crate::audio::sessions::ScanReport {
            sources: vec![crate::ui::controls::mock_source(42)],
            ..Default::default()
        },
    ));
    panel
        .update(
            &ControlUpdate {
                sources,
                ..Default::default()
            },
            false,
        )
        .unwrap();
    select(panel.source, 1);
    panel
        .apply_subtitle_outcome(Outcome::Apply(settings()))
        .unwrap();
    panel.update(&ControlUpdate::default(), false).unwrap();
    assert!(panel.current_subtitle_source().is_none());
}
