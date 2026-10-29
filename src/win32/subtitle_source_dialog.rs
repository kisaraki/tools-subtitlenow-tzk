//! Owned, Session-only subtitle settings. No requests, disk writes, or catalog
//! claims occur here. Signed URLs are masked and native undo storage is cleared.
use crate::{
    ai::speech::SpeechLanguage,
    error::{AppError, Result},
    resolver::{contracts::ResolverError, timed_subtitle::SubtitleFormat},
    ui::subtitle_source::{SubtitleSourceDraft, SubtitleSourceSettings},
};
use std::cell::{Cell, RefCell};
use windows::{
    Win32::{
        Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::Gdi::{COLOR_WINDOW, DEFAULT_GUI_FONT, GetStockObject},
        UI::{
            HiDpi::GetDpiForWindow,
            Input::KeyboardAndMouse::{EnableWindow, IsWindowEnabled, SetFocus},
            WindowsAndMessaging::*,
        },
    },
    core::{HSTRING, PCWSTR, w},
};

const CLASS: PCWSTR = w!("SubtitleNow.SessionSubtitleSource.Phase7");
const TITLE: usize = 701;
const URL: usize = 702;
const FORMAT: usize = 703;
const LANGUAGE: usize = 704;
const APPLY: usize = 705;
const CLEAR: usize = 706;
const CANCEL: usize = 707;
const DESTINATION: usize = 708;
const TITLE_LIMIT: usize = 1024;
const URL_LIMIT: usize = crate::resolver::timed_subtitle::MAX_SUBTITLE_URL_BYTES;
const WIDTH_DIP: i32 = 640;
const HEIGHT_DIP: i32 = 492;
const EM_SETLIMITTEXT: u32 = 0x00c5;
const EM_EMPTYUNDOBUFFER: u32 = 0x00cd;
const INVALID_NOTICE: &str = "請填寫有效片名與 HTTPS 字幕網址，並選擇格式及來源語言。";

pub(super) enum Outcome {
    Apply(SubtitleSourceSettings),
    Clear,
    Cancel,
}

struct Dialog {
    hwnd: Cell<HWND>,
    title: Cell<HWND>,
    url: Cell<HWND>,
    destination: Cell<HWND>,
    format: Cell<HWND>,
    language: Cell<HWND>,
    error: Cell<HWND>,
    controls: RefCell<Vec<HWND>>,
    outcome: RefCell<Option<Outcome>>,
    failed: Cell<bool>,
}
impl Dialog {
    fn new() -> Box<Self> {
        Box::new(Self {
            hwnd: Cell::default(),
            title: Cell::default(),
            url: Cell::default(),
            destination: Cell::default(),
            format: Cell::default(),
            language: Cell::default(),
            error: Cell::default(),
            controls: RefCell::new(Vec::with_capacity(14)),
            outcome: RefCell::new(None),
            failed: Cell::new(false),
        })
    }
    fn create(
        &self,
        parent: HWND,
        instance: HINSTANCE,
        current: Option<&SubtitleSourceSettings>,
        visible: bool,
    ) -> Result<()> {
        // SAFETY: the caller holds this stable Box on the UI thread until its
        // owned HWND is synchronously destroyed. The native pointer is borrowed.
        unsafe {
            let class = WNDCLASSW {
                lpfnWndProc: Some(dialog_proc),
                hInstance: instance,
                lpszClassName: CLASS,
                hCursor: LoadCursorW(None, IDC_ARROW).map_err(|_| AppError::SubtitleWindow)?,
                hbrBackground: windows::Win32::Graphics::Gdi::HBRUSH(
                    (COLOR_WINDOW.0 + 1) as *mut _,
                ),
                ..Default::default()
            };
            if RegisterClassW(&class) == 0
                && windows::Win32::Foundation::GetLastError()
                    != windows::Win32::Foundation::ERROR_CLASS_ALREADY_EXISTS
            {
                return Err(AppError::SubtitleWindow);
            }
            let dpi = GetDpiForWindow(parent);
            let width = super::subtitle_window::scale(WIDTH_DIP, dpi);
            let height = super::subtitle_window::scale(HEIGHT_DIP, dpi);
            let mut owner = RECT::default();
            GetWindowRect(parent, &mut owner).map_err(|_| AppError::SubtitleWindow)?;
            let hwnd = CreateWindowExW(
                WS_EX_DLGMODALFRAME | WS_EX_CONTROLPARENT,
                CLASS,
                w!("字幕來源（本次）"),
                WS_POPUP | WS_CAPTION | WS_SYSMENU,
                owner.left + (owner.right - owner.left - width) / 2,
                owner.top + (owner.bottom - owner.top - height) / 2,
                width,
                height,
                Some(parent),
                None,
                Some(instance),
                Some(self as *const Self as *const _),
            )
            .map_err(|_| AppError::SubtitleWindow)?;
            self.hwnd.set(hwnd);
            self.create_controls(instance)?;
            self.layout()?;
            if let Some(settings) = current {
                let draft = settings.draft();
                set_edit(self.title.get(), &draft.title)?;
                set_edit(self.url.get(), &draft.url)?;
                SendMessageW(
                    self.format.get(),
                    CB_SETCURSEL,
                    Some(WPARAM(match draft.format {
                        SubtitleFormat::Srt => 1,
                        SubtitleFormat::WebVtt => 2,
                    })),
                    None,
                );
                SendMessageW(
                    self.language.get(),
                    CB_SETCURSEL,
                    Some(WPARAM(language_index(draft.language))),
                    None,
                );
                // Clear edit history even for initial signed URLs copied from RAM.
                SendMessageW(self.url.get(), EM_EMPTYUNDOBUFFER, None, None);
            }
            self.refresh_destination()?;
            if visible {
                let _ = ShowWindow(hwnd, SW_SHOW);
                let _ = SetFocus(Some(self.title.get()));
            }
            Ok(())
        }
    }
    unsafe fn create_controls(&self, instance: HINSTANCE) -> Result<()> {
        // SAFETY: all created controls belong to our own dialog on this thread;
        // parent destruction owns them and each HSTRING survives the copied call.
        unsafe {
            let child = |class: PCWSTR, text: &str, style: WINDOW_STYLE, id: usize| {
                let handle = CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    class,
                    &HSTRING::from(text),
                    WS_CHILD | WS_VISIBLE | style,
                    0,
                    0,
                    1,
                    1,
                    Some(self.hwnd.get()),
                    Some(HMENU(id as *mut _)),
                    Some(instance),
                    None,
                )
                .map_err(|_| AppError::SubtitleWindow)?;
                let font = GetStockObject(DEFAULT_GUI_FONT);
                SendMessageW(handle, WM_SETFONT, Some(WPARAM(font.0 as usize)), None);
                self.controls.borrow_mut().push(handle);
                Ok::<_, AppError>(handle)
            };
            let label = |text| child(w!("STATIC"), text, WINDOW_STYLE::default(), 0);
            let combo_style = WS_TABSTOP | WS_VSCROLL | WINDOW_STYLE(CBS_DROPDOWNLIST as u32);
            label("片名（來源視窗須包含相同片名）")?;
            self.title.set(child(
                w!("EDIT"),
                "",
                WS_TABSTOP | WS_BORDER | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
                TITLE,
            )?);
            label("HTTPS 字幕網址（遮蔽可能包含的授權資訊）")?;
            self.url.set(child(
                w!("EDIT"),
                "",
                WS_TABSTOP | WS_BORDER | WINDOW_STYLE((ES_AUTOHSCROLL | ES_PASSWORD) as u32),
                URL,
            )?);
            self.destination.set(child(
                w!("EDIT"),
                "字幕下載主機：請先完成設定",
                WS_TABSTOP | WS_BORDER | WINDOW_STYLE((ES_READONLY | ES_AUTOHSCROLL) as u32),
                DESTINATION,
            )?);
            label("字幕格式")?;
            self.format
                .set(child(w!("COMBOBOX"), "", combo_style, FORMAT)?);
            label("字幕原文語言")?;
            self.language
                .set(child(w!("COMBOBOX"), "", combo_style, LANGUAGE)?);
            label(
                "僅供本次，停止或關閉後清除。按套用不會連線。\r\n開始後才讀取指定字幕；需與實際對白及時間核對才能採用。",
            )?;
            self.error.set(label("")?);
            child(w!("BUTTON"), "套用", WS_TABSTOP, APPLY)?;
            child(w!("BUTTON"), "清除設定", WS_TABSTOP, CLEAR)?;
            child(w!("BUTTON"), "取消", WS_TABSTOP, CANCEL)?;
            for text in ["請選擇格式", "SRT", "WebVTT"] {
                add(self.format.get(), text)?;
            }
            for text in ["請選擇語言", "英文", "日文", "法文", "德文", "韓文"] {
                add(self.language.get(), text)?;
            }
            for control in [self.format.get(), self.language.get()] {
                SendMessageW(control, CB_SETCURSEL, Some(WPARAM(0)), None);
            }
            for (control, limit) in [(self.title.get(), TITLE_LIMIT), (self.url.get(), URL_LIMIT)] {
                SendMessageW(control, EM_SETLIMITTEXT, Some(WPARAM(limit)), None);
            }
            Ok(())
        }
    }
    fn layout(&self) -> Result<()> {
        if self.controls.borrow().len() != 14 {
            return Ok(());
        }
        // SAFETY: only our dialog controls are repositioned; rectangles are bounded
        // by its actual client size and scaled for its current monitor DPI.
        unsafe {
            let dpi = GetDpiForWindow(self.hwnd.get());
            let scale = |value| super::subtitle_window::scale(value, dpi);
            let mut client = RECT::default();
            GetClientRect(self.hwnd.get(), &mut client).map_err(|_| AppError::SubtitleWindow)?;
            let pad = scale(16);
            let width = (client.right - 2 * pad).max(1);
            let row = scale(28);
            let gap = scale(8);
            let half = ((width - gap) / 2).max(1);
            let controls = self.controls.borrow().clone();
            let placements = [
                (pad, pad, width, row),
                (pad, pad + row, width, row),
                (pad, pad + 2 * row + gap, width, row),
                (pad, pad + 3 * row + gap, width, row),
                (pad, pad + 4 * row + 2 * gap, width, row),
                (pad, pad + 5 * row + 3 * gap, half, row),
                (pad, pad + 6 * row + 3 * gap, half, row * 6),
                (pad + half + gap, pad + 5 * row + 3 * gap, half, row),
                (pad + half + gap, pad + 6 * row + 3 * gap, half, row * 6),
                (pad, pad + 7 * row + 4 * gap, width, 2 * row),
                (pad, pad + 9 * row + 5 * gap, width, 2 * row),
                (pad, pad + 11 * row + 6 * gap, (width - 2 * gap) / 3, row),
                (
                    pad + (width - 2 * gap) / 3 + gap,
                    pad + 11 * row + 6 * gap,
                    (width - 2 * gap) / 3,
                    row,
                ),
                (
                    pad + 2 * ((width - 2 * gap) / 3 + gap),
                    pad + 11 * row + 6 * gap,
                    (width - 2 * gap) / 3,
                    row,
                ),
            ];
            for (control, (x, y, width, height)) in controls.into_iter().zip(placements) {
                MoveWindow(control, x, y, width.max(1), height.max(1), true)
                    .map_err(|_| AppError::SubtitleWindow)?;
            }
            Ok(())
        }
    }
    fn read_settings(&self) -> std::result::Result<SubtitleSourceSettings, ResolverError> {
        // SAFETY: scalar indexes read from our fixed, same-thread combo controls.
        let format = match unsafe { SendMessageW(self.format.get(), CB_GETCURSEL, None, None).0 } {
            1 => SubtitleFormat::Srt,
            2 => SubtitleFormat::WebVtt,
            _ => return Err(ResolverError::InvalidData),
        };
        // SAFETY: scalar indexes read from our fixed, same-thread combo controls.
        let language =
            match unsafe { SendMessageW(self.language.get(), CB_GETCURSEL, None, None).0 } {
                1 => SpeechLanguage::English,
                2 => SpeechLanguage::Japanese,
                3 => SpeechLanguage::French,
                4 => SpeechLanguage::German,
                5 => SpeechLanguage::Korean,
                _ => return Err(ResolverError::UnknownTextLanguage),
            };
        SubtitleSourceSettings::from_draft(SubtitleSourceDraft {
            title: read_edit(self.title.get(), TITLE_LIMIT)?,
            url: read_edit(self.url.get(), URL_LIMIT)?,
            format,
            language,
        })
    }
    fn apply(&self) -> Result<()> {
        match self.read_settings() {
            Ok(settings) => self.finish(Outcome::Apply(settings)),
            Err(_) => {
                // SAFETY: safe static validation guidance, no echoed input or URL.
                unsafe { SetWindowTextW(self.error.get(), &HSTRING::from(INVALID_NOTICE)) }
                    .map_err(|_| AppError::SubtitleWindow)
            }
        }
    }
    fn refresh_destination(&self) -> Result<()> {
        if self.destination.get().0.is_null() {
            return Ok(()); // EDIT notifications can occur during child creation.
        }
        let text = self.read_settings().map_or_else(
            |_| "字幕下載主機：請先完成有效設定".to_owned(),
            |settings| format!("字幕下載主機：{}", settings.destination()),
        );
        // SAFETY: host-only guidance in our readonly EDIT; never a signed URL.
        unsafe { SetWindowTextW(self.destination.get(), &HSTRING::from(text)) }
            .map_err(|_| AppError::SubtitleWindow)
    }
    fn clear_fields(&self) -> Result<()> {
        let title = clear_edit(self.title.get());
        let url = clear_edit(self.url.get());
        title.and(url) // Always clear both, including when one native call fails.
    }
    fn finish(&self, outcome: Outcome) -> Result<()> {
        if self.hwnd.get().0.is_null() {
            return Ok(());
        }
        *self.outcome.borrow_mut() = Some(outcome);
        let cleared = self.clear_fields();
        // SAFETY: this dialog owns the HWND and its children; destruction is
        // synchronous and WM_NCDESTROY clears the borrowed native state pointer.
        let destroyed =
            unsafe { DestroyWindow(self.hwnd.get()) }.map_err(|_| AppError::SubtitleWindow);
        cleared.and(destroyed)
    }
}
impl Drop for Dialog {
    fn drop(&mut self) {
        if !self.hwnd.get().0.is_null() {
            let _ = self.finish(Outcome::Cancel);
        }
    }
}

pub(super) fn edit(
    parent: HWND,
    instance: HINSTANCE,
    current: Option<&SubtitleSourceSettings>,
) -> Result<Outcome> {
    let dialog = Dialog::new();
    dialog.create(parent, instance, current, true)?;
    // SAFETY: ControlPanel validated that parent owns its control. This function
    // runs only on that UI thread and restores its previous enabled state by RAII.
    let enabled = unsafe { IsWindowEnabled(parent) }.as_bool();
    struct OwnerEnabled(HWND, bool);
    impl Drop for OwnerEnabled {
        fn drop(&mut self) {
            if self.1 {
                // SAFETY: own modal owner, still on its UI thread. No unrelated
                // window is enabled, activated, or inspected by this operation.
                unsafe {
                    let _ = EnableWindow(self.0, true);
                }
            }
        }
    }
    let _owner = OwnerEnabled(parent, enabled);
    // SAFETY: own parent only; modal edits cannot trigger live startup by clicking
    // the disabled owner, while its normal lifecycle messages keep being pumped.
    unsafe {
        let _ = EnableWindow(parent, false);
    }
    loop {
        if dialog.outcome.borrow().is_some() || dialog.hwnd.get().0.is_null() {
            break;
        }
        let mut message = MSG::default();
        // SAFETY: initialized writable MSG lives through this thread's message call.
        let status = unsafe { GetMessageW(&mut message, None, 0, 0) }.0;
        if status == -1 {
            return Err(AppError::SubtitleWindow);
        }
        if status == 0 {
            let _ = dialog.finish(Outcome::Cancel);
            // SAFETY: preserve application shutdown for the outer UI loop.
            unsafe { PostQuitMessage(message.wParam.0 as i32) };
            break;
        }
        // SAFETY: message-loop APIs process this thread's existing messages;
        // dialog navigation is restricted to its owned HWND and child controls.
        unsafe {
            if !IsDialogMessageW(dialog.hwnd.get(), &message).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
    if dialog.failed.get() {
        return Err(AppError::SubtitleWindow);
    }
    Ok(dialog
        .outcome
        .borrow_mut()
        .take()
        .unwrap_or(Outcome::Cancel))
}

fn language_index(language: SpeechLanguage) -> usize {
    match language {
        SpeechLanguage::English => 1,
        SpeechLanguage::Japanese => 2,
        SpeechLanguage::French => 3,
        SpeechLanguage::German => 4,
        SpeechLanguage::Korean => 5,
    }
}
fn clear_edit(control: HWND) -> Result<()> {
    if control.0.is_null() {
        return Ok(());
    }
    // SAFETY: this module supplies only its own same-thread EDIT handles. The
    // native contents and undo history are cleared before child destruction.
    unsafe {
        let cleared = SetWindowTextW(control, w!("")).map_err(|_| AppError::SubtitleWindow);
        SendMessageW(control, EM_EMPTYUNDOBUFFER, None, None);
        cleared
    }
}
fn set_edit(control: HWND, text: &str) -> Result<()> {
    let mut buffer = zeroize::Zeroizing::new(text.encode_utf16().collect::<Vec<_>>());
    buffer.push(0);
    // SAFETY: owned EDIT copies a live, NUL-terminated UTF-16 buffer synchronously;
    // the temporary copy is zeroized immediately afterward.
    unsafe { SetWindowTextW(control, PCWSTR(buffer.as_ptr())) }
        .map_err(|_| AppError::SubtitleWindow)
}
fn read_edit(control: HWND, limit: usize) -> std::result::Result<String, ResolverError> {
    // SAFETY: read actual length first, because programmatic SetWindowText can
    // bypass interactive limits. Never accept a truncated signed URL or title.
    let expected = unsafe { GetWindowTextLengthW(control) }.max(0) as usize;
    if expected == 0 || expected > limit {
        return Err(ResolverError::InvalidData);
    }
    let mut buffer = zeroize::Zeroizing::new(vec![0u16; limit + 1]);
    // SAFETY: bounded writable UTF-16 slice remains alive for this copied call.
    let actual = unsafe { GetWindowTextW(control, &mut buffer[..]) }.max(0) as usize;
    if actual != expected {
        return Err(ResolverError::InvalidData);
    }
    String::from_utf16(&buffer[..actual]).map_err(|_| ResolverError::InvalidData)
}
unsafe fn add(control: HWND, text: &str) -> Result<()> {
    let text = HSTRING::from(text);
    // SAFETY: own combo copies the still-live HSTRING synchronously.
    if unsafe {
        SendMessageW(
            control,
            CB_ADDSTRING,
            None,
            Some(LPARAM(text.as_ptr() as isize)),
        )
        .0
    } < 0
    {
        Err(AppError::SubtitleWindow)
    } else {
        Ok(())
    }
}
unsafe fn state(hwnd: HWND, message: u32, lparam: LPARAM) -> *const Dialog {
    // SAFETY: WM_NCCREATE lParam is native CREATESTRUCTW containing the stable,
    // borrowed Box pointer supplied by create(). It is cleared at WM_NCDESTROY.
    unsafe {
        if message == WM_NCCREATE {
            let create = &*(lparam.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Dialog
    }
}
unsafe extern "system" fn dialog_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: Win32 owns callback arguments. No Rust panic crosses the native ABI.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        dialog_message(hwnd, message, wparam, lparam)
    }))
    .unwrap_or_else(|_| {
        // SAFETY: same borrowed state protocol, used only during this callback.
        unsafe {
            if let Some(dialog) = state(hwnd, 0, LPARAM(0)).as_ref() {
                dialog.failed.set(true);
                // A second reentrant RefCell panic must also stay inside Rust.
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    dialog.finish(Outcome::Cancel)
                }));
            }
        }
        LRESULT(0)
    })
}
unsafe fn dialog_message(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: state pointer is valid only in this app-owned callback; it remains
    // alive until Drop synchronously destroys HWND. No reference escapes here.
    unsafe {
        if let Some(dialog) = state(hwnd, message, lparam).as_ref() {
            match message {
                WM_COMMAND => {
                    let id = wparam.0 & 0xffff;
                    if [TITLE, URL, FORMAT, LANGUAGE].contains(&id) {
                        if dialog.refresh_destination().is_err() {
                            dialog.failed.set(true);
                        }
                        return LRESULT(0);
                    }
                    let result = match wparam.0 & 0xffff {
                        APPLY | 1 => dialog.apply(),
                        CLEAR => dialog.finish(Outcome::Clear),
                        CANCEL | 2 => dialog.finish(Outcome::Cancel),
                        _ => return DefWindowProcW(hwnd, message, wparam, lparam),
                    };
                    if result.is_err() {
                        dialog.failed.set(true);
                        let _ = dialog.finish(Outcome::Cancel);
                    }
                    return LRESULT(0);
                }
                WM_CLOSE => {
                    if dialog.finish(Outcome::Cancel).is_err() {
                        dialog.failed.set(true);
                    }
                    return LRESULT(0);
                }
                WM_SIZE => {
                    if dialog.layout().is_err() {
                        dialog.failed.set(true);
                        let _ = dialog.finish(Outcome::Cancel);
                    }
                }
                WM_DPICHANGED if lparam.0 != 0 => {
                    // SAFETY: Win32's callback-scoped DPI recommendation rectangle.
                    let rect = &*(lparam.0 as *const RECT);
                    if SetWindowPos(
                        hwnd,
                        None,
                        rect.left,
                        rect.top,
                        (rect.right - rect.left).max(1),
                        (rect.bottom - rect.top).max(1),
                        SWP_NOACTIVATE | SWP_NOZORDER,
                    )
                    .is_err()
                        || dialog.layout().is_err()
                    {
                        dialog.failed.set(true);
                        let _ = dialog.finish(Outcome::Cancel);
                    }
                    return LRESULT(0);
                }
                WM_NCDESTROY => {
                    dialog.hwnd.set(HWND::default());
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                }
                WM_DESTROY if dialog.clear_fields().is_err() => {
                    // This also covers destruction initiated by the owned parent,
                    // while EDIT children still exist during WM_DESTROY.
                    dialog.failed.set(true);
                }
                _ => {}
            }
        }
        DefWindowProcW(hwnd, message, wparam, lparam)
    }
}

#[cfg(test)]
#[path = "subtitle_source_dialog_tests.rs"]
mod tests;
