//! One modeless, resizable view of the existing status text. No subtitle or AI pipeline.
use super::*;

pub(super) const CLASS: PCWSTR = w!("SubtitleNow.StatusDetails.Phase11");
const WIDTH_DIP: i32 = 640;
const HEIGHT_DIP: i32 = 420;
const MIN_WIDTH_DIP: i32 = 240;
const MIN_HEIGHT_DIP: i32 = 160;
const PAD_DIP: i32 = 8;

impl SubtitleShell {
    pub(super) fn open_status_window(&self, visible: bool) -> Result<()> {
        // SAFETY: only this shell's same-thread owner; modal owners cannot be bypassed.
        if !unsafe { windows::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled(self.main.get()) }
            .as_bool()
        {
            return Ok(());
        }
        // SAFETY: all HWNDs/fonts are owned by this UI-thread shell. The stable
        // shell Box outlives synchronous destruction of the modeless owned window.
        unsafe {
            if !IsWindow(Some(self.status_window.get())).as_bool() {
                let created = (|| -> Result<()> {
                    let instance = HINSTANCE(
                        GetModuleHandleW(None)
                            .map_err(|_| AppError::SubtitleWindow)?
                            .0,
                    );
                    let work = subtitle_monitor::window_work_area(self.main.get())?;
                    let dpi = GetDpiForWindow(self.main.get());
                    let width = scale(WIDTH_DIP, dpi).min((work.right - work.left).max(1));
                    let height = scale(HEIGHT_DIP, dpi).min((work.bottom - work.top).max(1));
                    self.status_window.set(
                        CreateWindowExW(
                            WS_EX_TOOLWINDOW,
                            CLASS,
                            w!("SubtitleNow — 系統狀態與錯誤（可調整大小）"),
                            WS_OVERLAPPEDWINDOW,
                            work.left + (work.right - work.left - width) / 2,
                            work.top + (work.bottom - work.top - height) / 2,
                            width,
                            height,
                            Some(self.main.get()),
                            None,
                            Some(instance),
                            Some(self as *const Self as *const std::ffi::c_void),
                        )
                        .map_err(|_| AppError::SubtitleWindow)?,
                    );
                    self.status_window_edit.set(
                        CreateWindowExW(
                            WS_EX_CLIENTEDGE,
                            w!("EDIT"),
                            w!(""),
                            WS_CHILD
                                | WS_VISIBLE
                                | WS_VSCROLL
                                | WS_TABSTOP
                                | WINDOW_STYLE(
                                    (ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL) as u32,
                                ),
                            0,
                            0,
                            1,
                            1,
                            Some(self.status_window.get()),
                            None,
                            Some(instance),
                            None,
                        )
                        .map_err(|_| AppError::SubtitleWindow)?,
                    );
                    self.update_status_window_font()?;
                    self.layout_status_window()
                })();
                if let Err(error) = created {
                    self.destroy_status_window();
                    return Err(error);
                }
            }
            self.update_status_window_text(&self.status_text.borrow())?;
            if visible {
                let show = if IsIconic(self.status_window.get()).as_bool() {
                    SW_RESTORE
                } else {
                    SW_SHOW
                };
                let _ = ShowWindow(self.status_window.get(), show);
            }
        }
        Ok(())
    }
    pub(super) fn update_status_window_text(&self, text: &str) -> Result<()> {
        // SAFETY: edit copies the bounded, existing status string synchronously.
        // No activation, capture, credentials, transcript reads or persistence.
        unsafe {
            if IsWindow(Some(self.status_window_edit.get())).as_bool() {
                SetWindowTextW(
                    self.status_window_edit.get(),
                    &windows::core::HSTRING::from(text),
                )
                .map_err(|_| AppError::SubtitleWindow)?;
            }
        }
        Ok(())
    }
    fn update_status_window_font(&self) -> Result<()> {
        // SAFETY: message font is owned here. Switch the only child to the new
        // live HFONT before deleting the previous font on the same UI thread.
        unsafe {
            let font = make_font(GetDpiForWindow(self.status_window.get()));
            if font.0.is_null() {
                return Err(AppError::SubtitleWindow);
            }
            SendMessageW(
                self.status_window_edit.get(),
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(1)),
            );
            let old = self.status_window_font.replace(font);
            if !old.0.is_null() {
                let _ = DeleteObject(HGDIOBJ(old.0));
            }
        }
        Ok(())
    }
    fn layout_status_window(&self) -> Result<()> {
        // SAFETY: same-thread HWNDs and initialized out RECT; dimensions clamped.
        unsafe {
            if self.status_window_edit.get().0.is_null() {
                return Ok(());
            }
            let mut rect = RECT::default();
            GetClientRect(self.status_window.get(), &mut rect)
                .map_err(|_| AppError::SubtitleWindow)?;
            let pad = scale(PAD_DIP, GetDpiForWindow(self.status_window.get()));
            MoveWindow(
                self.status_window_edit.get(),
                pad,
                pad,
                (rect.right - 2 * pad).max(1),
                (rect.bottom - 2 * pad).max(1),
                true,
            )
            .map_err(|_| AppError::SubtitleWindow)?;
        }
        Ok(())
    }
    pub(super) fn destroy_status_window(&self) {
        // SAFETY: synchronous destruction clears callback pointers before the
        // shell Box/font is released; an owner may already have destroyed it.
        unsafe {
            if IsWindow(Some(self.status_window.get())).as_bool() {
                let _ = DestroyWindow(self.status_window.get());
            }
            let font = self.status_window_font.replace(HFONT::default());
            if !font.0.is_null() {
                let _ = DeleteObject(HGDIOBJ(font.0));
            }
        }
    }
}

pub(super) unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || -> Result<Option<LRESULT>> {
            // SAFETY: WM_NCCREATE carries our stable shell pointer. No callback
            // reference escapes; NCDESTROY clears it while the Box is still alive.
            unsafe {
                let pointer = state(hwnd, message, lparam);
                if pointer.is_null() {
                    return Ok(None);
                }
                let shell = &*pointer;
                if let Some(value) = shell.theme_message(hwnd, message, wparam, lparam) {
                    return Ok(Some(value));
                }
                match message {
                    WM_CLOSE => {
                        let _ = ShowWindow(hwnd, SW_HIDE);
                        return Ok(Some(LRESULT(0)));
                    }
                    WM_SIZE => {
                        shell.layout_status_window()?;
                        return Ok(Some(LRESULT(0)));
                    }
                    WM_GETMINMAXINFO => {
                        let bounds = &mut *(lparam.0 as *mut MINMAXINFO);
                        let dpi = GetDpiForWindow(hwnd);
                        bounds.ptMinTrackSize.x = scale(MIN_WIDTH_DIP, dpi);
                        bounds.ptMinTrackSize.y = scale(MIN_HEIGHT_DIP, dpi);
                        return Ok(Some(LRESULT(0)));
                    }
                    WM_DPICHANGED if !shell.status_window_edit.get().0.is_null() => {
                        shell.update_status_window_font()?;
                        let rect = &*(lparam.0 as *const RECT);
                        SetWindowPos(
                            hwnd,
                            None,
                            rect.left,
                            rect.top,
                            (rect.right - rect.left).max(1),
                            (rect.bottom - rect.top).max(1),
                            SWP_NOZORDER | SWP_NOACTIVATE,
                        )
                        .map_err(|_| AppError::SubtitleWindow)?;
                        shell.layout_status_window()?;
                        return Ok(Some(LRESULT(0)));
                    }
                    WM_NCDESTROY => {
                        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                        shell.status_window.set(HWND::default());
                        shell.status_window_edit.set(HWND::default());
                    }
                    _ => {}
                }
            }
            Ok(None)
        },
    ));
    match result {
        Ok(Ok(Some(value))) => value,
        Ok(Ok(None)) => {
            // SAFETY: unhandled native message, delegated exactly once.
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        _ => {
            // SAFETY: no unwind crosses the ABI. A failed optional status view
            // is hidden and reported on the main view, never cancels translation.
            unsafe {
                let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const SubtitleShell;
                if let Some(shell) = pointer.as_ref()
                    && let Ok(mut notice) = shell.notice.try_borrow_mut()
                {
                    *notice = Some("無法更新狀態視窗；可繼續使用主頁狀態框。".into());
                }
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
            LRESULT(0)
        }
    }
}
