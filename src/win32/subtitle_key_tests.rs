//! Fixed key UI regressions. Only test-owned hidden HWNDs, synthetic input and
//! mock Save actions; never access real provider credential targets or networks.
use super::{tests::*, *};
use crate::ai::{
    credentials::{CredentialError, MAX_KEY_BLOB_BYTES},
    provider::SpeechProvider,
};
use windows::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled;

const EM_CANUNDO_TEST: u32 = 0x00c6;
fn set(control: HWND, text: &str) {
    // SAFETY: private EDIT owned by this fixture's UI thread, synchronous copy.
    unsafe {
        SetWindowTextW(control, &windows::core::HSTRING::from(text)).unwrap();
    }
}
fn click(shell: &SubtitleShell, id: usize) {
    // SAFETY: sends a button notification to this fixture's own parent window.
    unsafe {
        SendMessageW(shell.main.get(), WM_COMMAND, Some(WPARAM(id)), None);
    }
    assert!(!shell.failed.get());
}
fn rect(control: HWND) -> RECT {
    let mut value = RECT::default();
    // SAFETY: initialized writable rectangle and same-thread test HWND.
    unsafe {
        GetWindowRect(control, &mut value).unwrap();
    }
    value
}

#[test]
fn fixed_provider_labels_masked_fields_and_save_buttons_fit_without_source_selection() {
    let (shell, _) = controlled_mock();
    shell.command(CREDENTIAL_SETTINGS).unwrap();
    let panel = shell.panel.borrow();
    let panel = panel.as_ref().unwrap();
    assert_eq!(control_text(panel.openai_label), "OpenAI（ChatGPT）");
    assert_eq!(control_text(panel.gemini_label), "Gemini API");
    assert!(control_text(panel.key_notice).contains("未儲存只供本次啟動"));
    assert!(panel.request().is_err());
    for (label, input, save, check) in [
        (
            panel.openai_label,
            panel.openai_key,
            panel.save_openai_key,
            panel.check_openai_key,
        ),
        (
            panel.gemini_label,
            panel.gemini_key,
            panel.save_gemini_key,
            panel.check_gemini_key,
        ),
    ] {
        // SAFETY: scalar style/state and sibling checks on this fixture's own controls.
        unsafe {
            assert!(IsWindowEnabled(input).as_bool());
            assert!(IsWindowEnabled(save).as_bool());
            assert!(IsWindowEnabled(check).as_bool());
            assert!(GetWindowLongPtrW(input, GWL_STYLE) as u32 & ES_PASSWORD as u32 != 0);
            assert!(GetWindowLongPtrW(input, GWL_STYLE) as u32 & WS_VISIBLE.0 != 0);
            assert_eq!(GetWindow(input, GW_HWNDPREV).unwrap(), label);
        }
        assert!(rect(label).right <= rect(input).left);
        assert!(rect(input).right <= rect(save).left);
        assert!(rect(save).right <= rect(check).left);
        assert_eq!(control_text(check), "檢查本機");
        assert!(rect(input).right - rect(input).left > 0);
        let mut size = windows::Win32::Foundation::SIZE::default();
        let text: Vec<u16> = control_text(label).encode_utf16().collect();
        // SAFETY: same-thread label's live font; only borrowed for the private test DC.
        let font = unsafe { SendMessageW(label, WM_GETFONT, None, None).0 };
        super::super::subtitle_render::inspect_test_canvas(1, 1, |dc| {
            // SAFETY: test-owned DIB/DC, live font, valid text slice; restore the
            // original selected font before the DC or its borrowed font is released.
            unsafe {
                let previous = SelectObject(dc, HGDIOBJ(font as *mut _));
                GetTextExtentPoint32W(dc, &text, &mut size).ok().unwrap();
                SelectObject(dc, previous);
            }
            Ok(())
        })
        .unwrap();
        assert!(
            size.cx <= rect(label).right - rect(label).left,
            "provider label '{}' width {} exceeds fixed row width {}",
            control_text(label),
            size.cx,
            rect(label).right - rect(label).left
        );
    }
    assert!(rect(panel.openai_key).bottom <= rect(panel.gemini_key).top);
    assert!(rect(panel.gemini_key).bottom <= rect(panel.key_notice).top);
    assert!(rect(panel.key_notice).bottom <= rect(panel.start).top);
    assert!(rect(panel.save_preferences).bottom <= rect(panel.start).top);
    assert_ne!(
        unsafe { GetWindowLongPtrW(panel.source, GWL_STYLE) as u32 & WS_VISIBLE.0 },
        0
    );
    assert!(rect(panel.start).bottom <= rect(shell.search_label.get()).top);
}

#[test]
fn each_provider_uses_its_own_key_and_start_clears_both_undo_buffers() {
    let (shell, _) = controlled_mock();
    let panel = shell.panel.borrow();
    let panel = panel.as_ref().unwrap();
    for (provider, expected) in [
        (SpeechProvider::OpenAi, "SYNTHETIC_OPENAI_KEY"),
        (SpeechProvider::Gemini, "SYNTHETIC_GEMINI_KEY"),
    ] {
        set(panel.openai_key, "SYNTHETIC_OPENAI_KEY");
        set(panel.gemini_key, "SYNTHETIC_GEMINI_KEY");
        let key = panel.take_key(provider).unwrap().unwrap();
        assert_eq!(key.expose(), expected);
        for field in [panel.openai_key, panel.gemini_key] {
            assert!(control_text(field).is_empty());
            // SAFETY: scalar undo query on an owned EDIT, no secret output.
            assert_eq!(
                unsafe { SendMessageW(field, EM_CANUNDO_TEST, None, None).0 },
                0
            );
        }
    }
}

#[test]
fn invalid_and_programmatically_oversized_input_never_becomes_a_truncated_key() {
    let (shell, _) = controlled_mock();
    let panel = shell.panel.borrow();
    let panel = panel.as_ref().unwrap();
    for text in [
        "bad key".to_owned(),
        "非 API 金鑰".into(),
        "x".repeat(MAX_KEY_BLOB_BYTES / 2 + 1),
    ] {
        set(panel.openai_key, &text);
        set(panel.gemini_key, "SYNTHETIC_OTHER_KEY");
        assert!(panel.take_key(SpeechProvider::OpenAi).is_err());
        assert!(control_text(panel.openai_key).is_empty());
        assert!(control_text(panel.gemini_key).is_empty());
    }
    set(panel.openai_key, &"x".repeat(MAX_KEY_BLOB_BYTES / 2));
    assert_eq!(
        panel
            .take_key(SpeechProvider::OpenAi)
            .unwrap()
            .unwrap()
            .expose()
            .len(),
        MAX_KEY_BLOB_BYTES / 2
    );
}

#[test]
fn save_buttons_route_by_fixed_provider_without_starting_cloud_or_requiring_source() {
    let (shell, state) = controlled_mock();
    {
        let panel = shell.panel.borrow();
        let panel = panel.as_ref().unwrap();
        set(panel.openai_key, "SYNTHETIC_OPENAI_KEY");
        set(panel.gemini_key, "SYNTHETIC_GEMINI_KEY");
        // SAFETY: different selected service must not reroute the OpenAI Save button.
        unsafe {
            SendMessageW(panel.provider, CB_SETCURSEL, Some(WPARAM(2)), None);
        }
    }
    click(&shell, subtitle_controls::SAVE_OPENAI_KEY);
    {
        let panel = shell.panel.borrow();
        let panel = panel.as_ref().unwrap();
        assert!(control_text(panel.openai_key).is_empty());
        assert_eq!(control_text(panel.gemini_key), "SYNTHETIC_GEMINI_KEY");
        assert!(control_text(panel.key_notice).contains("OpenAI 金鑰已儲存"));
        assert!(control_text(panel.key_notice).contains("確認可讀取"));
        assert!(control_text(panel.key_notice).contains("雲端結果請看下方"));
    }
    click(&shell, subtitle_controls::SAVE_GEMINI_KEY);
    assert_eq!(
        state.borrow().saved,
        vec![SpeechProvider::OpenAi, SpeechProvider::Gemini]
    );
    assert!(state.borrow().calls.is_empty());
    let panel = shell.panel.borrow();
    let panel = panel.as_ref().unwrap();
    assert!(control_text(panel.gemini_key).is_empty());
    assert!(control_text(panel.key_notice).contains("Gemini 金鑰已儲存"));
}

#[test]
fn empty_invalid_or_failed_save_reports_safe_error_and_never_false_success() {
    let (shell, state) = controlled_mock();
    for failure in 0..3 {
        {
            let panel = shell.panel.borrow();
            let panel = panel.as_ref().unwrap();
            match failure {
                0 => {}
                1 => set(panel.openai_key, "SYNTHETIC INVALID KEY"),
                _ => {
                    set(panel.openai_key, "SYNTHETIC_VALID_KEY");
                    state.borrow_mut().save_error = Some(CredentialError::Unavailable);
                }
            }
        }
        click(&shell, subtitle_controls::SAVE_OPENAI_KEY);
        let panel = shell.panel.borrow();
        let panel = panel.as_ref().unwrap();
        let notice = control_text(panel.key_notice);
        assert!(notice.contains("本次未儲存"));
        assert!(!notice.contains("金鑰已儲存"));
        assert!(!notice.contains("SYNTHETIC"));
        assert!(control_text(panel.openai_key).is_empty());
        assert!(state.borrow().saved.is_empty());
        assert!(state.borrow().calls.is_empty());
    }
}

#[test]
fn active_session_locks_key_changes_and_ignores_save_notifications() {
    let (shell, state) = controlled_mock();
    {
        let panel = shell.panel.borrow();
        let panel = panel.as_ref().unwrap();
        // SAFETY: explicit synthetic mock source/provider; never runs a real backend.
        unsafe {
            SendMessageW(panel.source, CB_SETCURSEL, Some(WPARAM(1)), None);
            SendMessageW(panel.provider, CB_SETCURSEL, Some(WPARAM(1)), None);
            SendMessageW(panel.language, CB_SETCURSEL, Some(WPARAM(1)), None);
        }
        set(panel.openai_key, "SYNTHETIC_OPENAI_KEY");
    }
    shell.start_from_controls().unwrap();
    {
        let panel = shell.panel.borrow();
        let panel = panel.as_ref().unwrap();
        for control in [
            panel.openai_key,
            panel.gemini_key,
            panel.save_openai_key,
            panel.save_gemini_key,
            panel.check_openai_key,
            panel.check_gemini_key,
        ] {
            // SAFETY: scalar enable-state check on owned controls only.
            assert!(!unsafe { IsWindowEnabled(control) }.as_bool());
        }
    }
    click(&shell, subtitle_controls::SAVE_OPENAI_KEY);
    click(&shell, subtitle_controls::SAVE_GEMINI_KEY);
    click(&shell, subtitle_controls::CHECK_OPENAI_KEY);
    click(&shell, subtitle_controls::CHECK_GEMINI_KEY);
    assert!(state.borrow().saved.is_empty());
    assert!(state.borrow().checked.is_empty());
    assert_eq!(state.borrow().calls.len(), 1);
}

#[test]
fn phase11_local_key_check_routes_only_the_named_provider_and_preserves_unsaved_fields() {
    let (shell, state) = controlled_mock();
    assert!(state.borrow().checked.is_empty());
    {
        let panel = shell.panel.borrow();
        let panel = panel.as_ref().unwrap();
        set(panel.openai_key, "SYNTHETIC_UNSAVED_OPENAI");
        set(panel.gemini_key, "SYNTHETIC_UNSAVED_GEMINI");
        // SAFETY: opposite selected service must not reroute a fixed Check button.
        unsafe {
            SendMessageW(panel.provider, CB_SETCURSEL, Some(WPARAM(2)), None);
        }
    }
    for (id, name) in [
        (subtitle_controls::CHECK_OPENAI_KEY, "OpenAI"),
        (subtitle_controls::CHECK_GEMINI_KEY, "Gemini"),
    ] {
        click(&shell, id);
        let panel = shell.panel.borrow();
        let panel = panel.as_ref().unwrap();
        let notice = control_text(panel.key_notice);
        assert!(notice.contains(name));
        assert!(notice.contains("本機已儲存且可讀取"));
        assert!(notice.contains("雲端連線結果請看下方"));
        assert!(!notice.contains("SYNTHETIC"));
        assert_eq!(control_text(panel.openai_key), "SYNTHETIC_UNSAVED_OPENAI");
        assert_eq!(control_text(panel.gemini_key), "SYNTHETIC_UNSAVED_GEMINI");
    }
    assert_eq!(
        state.borrow().checked,
        vec![SpeechProvider::OpenAi, SpeechProvider::Gemini]
    );
    assert!(state.borrow().saved.is_empty());
    assert!(state.borrow().calls.is_empty());
}

#[test]
fn phase11_local_key_check_distinguishes_missing_invalid_and_unavailable_without_false_success() {
    let (shell, state) = controlled_mock();
    for error in [
        CredentialError::Missing,
        CredentialError::InvalidEncoding,
        CredentialError::Unavailable,
    ] {
        state.borrow_mut().check_error = None;
        click(&shell, subtitle_controls::CHECK_GEMINI_KEY);
        state.borrow_mut().check_error = Some(error);
        click(&shell, subtitle_controls::CHECK_GEMINI_KEY);
        let panel = shell.panel.borrow();
        let notice = control_text(panel.as_ref().unwrap().key_notice);
        assert!(!notice.contains("已儲存且可讀取"));
        assert_eq!(
            notice.contains("本機尚未儲存"),
            error == CredentialError::Missing
        );
        assert_eq!(
            notice.contains("本機金鑰資料無法使用"),
            error == CredentialError::InvalidEncoding
        );
        assert_eq!(
            notice.contains("無法確認本機金鑰狀態"),
            error == CredentialError::Unavailable
        );
        assert!(notice.contains("不連線"));
    }
    assert!(state.borrow().saved.is_empty());
    assert!(state.borrow().calls.is_empty());
}

#[test]
fn phase11_save_with_unconfirmed_readback_never_claims_missing_or_verified() {
    let (shell, state) = controlled_mock();
    state.borrow_mut().save_error = Some(CredentialError::VerificationUnavailable);
    {
        let panel = shell.panel.borrow();
        set(panel.as_ref().unwrap().gemini_key, "SYNTHETIC_KEY");
    }
    click(&shell, subtitle_controls::SAVE_GEMINI_KEY);
    let panel = shell.panel.borrow();
    let panel = panel.as_ref().unwrap();
    let notice = control_text(panel.key_notice);
    assert!(notice.contains("寫入要求已完成"));
    assert!(notice.contains("本機讀取尚未確認"));
    assert!(!notice.contains("本次未儲存"));
    assert!(!notice.contains("本機尚未儲存"));
    assert!(!notice.contains("金鑰已儲存"));
    assert!(!notice.contains("SYNTHETIC"));
    assert!(control_text(panel.gemini_key).is_empty());
    assert!(state.borrow().calls.is_empty());
}
