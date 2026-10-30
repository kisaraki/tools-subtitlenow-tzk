//! UI-thread About dialog; the single owned update job never touches native controls.
use crate::{
    error::{AppError, Result},
    update::{self, Outcome, Release, UpdateJob},
};
use std::cell::{Cell, RefCell};
use windows::{
    Win32::{
        Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::Gdi::{COLOR_WINDOW, DeleteObject, HFONT, HGDIOBJ},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            HiDpi::GetDpiForWindow,
            Input::KeyboardAndMouse::{EnableWindow, IsWindowEnabled, SetFocus},
            WindowsAndMessaging::*,
        },
    },
    core::{HSTRING, PCWSTR, w},
};

const CLASS: PCWSTR = w!("SubtitleNow.About");
const CHECK: usize = 1001;
const INSTALL: usize = 1002;
const CLOSE: usize = 1003;
const POLL: usize = 1;
struct About {
    hwnd: Cell<HWND>,
    status: Cell<HWND>,
    check: Cell<HWND>,
    install: Cell<HWND>,
    job: RefCell<Option<UpdateJob>>,
    release: RefCell<Option<Release>>,
    launched: Cell<bool>,
    font: Cell<HFONT>,
}
impl About {
    fn new() -> Box<Self> {
        Box::new(Self {
            hwnd: Cell::new(HWND::default()),
            status: Cell::new(HWND::default()),
            check: Cell::new(HWND::default()),
            install: Cell::new(HWND::default()),
            job: RefCell::new(None),
            release: RefCell::new(None),
            launched: Cell::new(false),
            font: Cell::new(HFONT::default()),
        })
    }
    fn status(&self, text: &str) {
        // SAFETY: owned same-thread STATIC copies the temporary HSTRING synchronously.
        unsafe {
            let _ = SetWindowTextW(self.status.get(), &HSTRING::from(text));
        }
    }
    fn buttons(&self) {
        let busy = self.job.borrow().is_some();
        // SAFETY: these child controls belong to this live UI-thread dialog.
        unsafe {
            let _ = EnableWindow(self.check.get(), !busy);
            let _ = EnableWindow(self.install.get(), !busy && self.release.borrow().is_some());
        }
    }
    fn start(&self, install: bool) {
        if self.job.borrow().is_some() {
            return;
        }
        let release = if install {
            let Some(release) = self.release.borrow().clone() else {
                return;
            };
            // SAFETY: live same-thread owner and static text; explicit consent to exit.
            let confirmed = unsafe {
                MessageBoxW(
                    Some(self.hwnd.get()),
                    w!(
                        "下載並驗證完成後會關閉 SubtitleNow，並開啟安裝精靈。\n目前翻譯會停止，本次字幕紀錄會隨程式關閉清除。\n安裝精靈會先移除舊版再安裝新版。是否繼續？"
                    ),
                    w!("線上更新"),
                    MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2,
                )
            };
            if confirmed != IDYES {
                return;
            }
            Some(release)
        } else {
            self.release.borrow_mut().take();
            None
        };
        match UpdateJob::start(release) {
            Ok(job) => {
                *self.job.borrow_mut() = Some(job);
                self.status(if install {
                    "正在下載並驗證安裝檔…\r\n關閉此視窗可取消下載。"
                } else {
                    "正在向 GitHub 檢查正式版本…"
                });
            }
            Err(error) => self.status(&error.to_string()),
        }
        self.buttons();
    }
    fn poll(&self) {
        let result = self.job.borrow().as_ref().and_then(UpdateJob::poll);
        let Some(result) = result else {
            return;
        };
        self.job.borrow_mut().take();
        match result {
            Ok(Outcome::Checked(Some(release))) => {
                self.status(&format!(
                    "有新版本 {} 可用。\r\n選擇「下載並安裝」開始線上更新。",
                    release.version
                ));
                *self.release.borrow_mut() = Some(release);
            }
            Ok(Outcome::Checked(None)) => self.status("目前已是最新版本。"),
            Ok(Outcome::Downloaded(installer)) => match installer.launch() {
                Ok(()) => {
                    self.launched.set(true);
                    // SAFETY: synchronous destroy while this Box still owns callbacks.
                    unsafe {
                        let _ = DestroyWindow(self.hwnd.get());
                    }
                    return;
                }
                Err(error) => self.status(&error.to_string()),
            },
            Err(error) => self.status(&error.to_string()),
        }
        self.buttons();
    }
    fn create(&self, owner: HWND, visible: bool) -> Result<()> {
        // SAFETY: registered callback holds the stable Box address only until
        // NCDESTROY. Children/fonts are owned on this thread and cleaned by Drop.
        unsafe {
            let instance = HINSTANCE(
                GetModuleHandleW(None)
                    .map_err(|_| AppError::SubtitleWindow)?
                    .0,
            );
            let class = WNDCLASSW {
                hInstance: instance,
                lpszClassName: CLASS,
                lpfnWndProc: Some(window_proc),
                hCursor: LoadCursorW(None, IDC_ARROW).map_err(|_| AppError::SubtitleWindow)?,
                hbrBackground: windows::Win32::Graphics::Gdi::HBRUSH(
                    (COLOR_WINDOW.0 + 1) as *mut _,
                ),
                ..Default::default()
            };
            RegisterClassW(&class); // An earlier About window may have registered it.
            let dpi = GetDpiForWindow(owner).max(96);
            let scale = |value| super::subtitle_window::scale(value, dpi);
            let work = super::subtitle_monitor::window_work_area(owner)?;
            let mut size = RECT {
                right: scale(540),
                bottom: scale(350),
                ..Default::default()
            };
            let style = WS_CAPTION | WS_SYSMENU | WS_POPUP;
            windows::Win32::UI::HiDpi::AdjustWindowRectExForDpi(
                &mut size,
                style,
                false,
                WS_EX_DLGMODALFRAME,
                dpi,
            )
            .map_err(|_| AppError::SubtitleWindow)?;
            let width = size.right - size.left;
            let height = size.bottom - size.top;
            self.hwnd.set(
                CreateWindowExW(
                    WS_EX_DLGMODALFRAME,
                    CLASS,
                    w!("關於 SubtitleNow"),
                    style,
                    work.left + (work.right - work.left - width) / 2,
                    work.top + (work.bottom - work.top - height) / 2,
                    width,
                    height,
                    Some(owner),
                    None,
                    Some(instance),
                    Some(self as *const Self as *const _),
                )
                .map_err(|_| AppError::SubtitleWindow)?,
            );
            self.font.set(super::subtitle_window::make_font(dpi));
            if self.font.get().0.is_null() {
                return Err(AppError::SubtitleWindow);
            }
            let child =
                |class: PCWSTR, text: &str, id: usize, x, y, width, height| -> Result<HWND> {
                    let hwnd = CreateWindowExW(
                        WINDOW_EX_STYLE::default(),
                        class,
                        &HSTRING::from(text),
                        WS_CHILD
                            | WS_VISIBLE
                            | if id > 0 {
                                WS_TABSTOP
                            } else {
                                WINDOW_STYLE::default()
                            },
                        scale(x),
                        scale(y),
                        scale(width),
                        scale(height),
                        Some(self.hwnd.get()),
                        Some(HMENU(id as *mut _)),
                        Some(instance),
                        None,
                    )
                    .map_err(|_| AppError::SubtitleWindow)?;
                    SendMessageW(
                        hwnd,
                        WM_SETFONT,
                        Some(WPARAM(self.font.get().0 as usize)),
                        Some(LPARAM(1)),
                    );
                    Ok(hwnd)
                };
            child(
                w!("STATIC"),
                &format!(
                    "SubtitleNow\r\n版本：{}（Windows x64）\r\n作者：{}",
                    update::VERSION,
                    update::AUTHOR
                ),
                0,
                20,
                18,
                500,
                80,
            )?;
            child(w!("STATIC"), update::WEBSITE, 0, 20, 105, 500, 28)?;
            child(
                w!("STATIC"),
                "只有按下檢查更新才會連線至 GitHub。\r\n更新要求不含音訊、字幕、來源程式名稱或 API 金鑰。",
                0,
                20,
                143,
                500,
                52,
            )?;
            self.status
                .set(child(w!("STATIC"), "尚未檢查更新。", 0, 20, 205, 500, 68)?);
            self.check
                .set(child(w!("BUTTON"), "檢查更新", CHECK, 20, 295, 145, 34)?);
            self.install.set(child(
                w!("BUTTON"),
                "下載並安裝",
                INSTALL,
                175,
                295,
                165,
                34,
            )?);
            child(w!("BUTTON"), "關閉", CLOSE, 395, 295, 125, 34)?;
            self.buttons();
            if SetTimer(Some(self.hwnd.get()), POLL, 100, None) == 0 {
                return Err(AppError::SubtitleWindow);
            }
            if visible {
                let _ = ShowWindow(self.hwnd.get(), SW_SHOW);
                let _ = SetFocus(Some(self.check.get()));
            }
        }
        Ok(())
    }
}
impl Drop for About {
    fn drop(&mut self) {
        self.job.get_mut().take();
        // SAFETY: destruction clears the callback before releasing this Box/font.
        unsafe {
            if IsWindow(Some(self.hwnd.get())).as_bool() {
                let _ = DestroyWindow(self.hwnd.get());
            }
            if !self.font.get().0.is_null() {
                let _ = DeleteObject(HGDIOBJ(self.font.get().0));
            }
        }
    }
}
pub(super) fn show(owner: HWND) -> Result<bool> {
    // SAFETY: caller supplies its own UI-thread owner. Prevent nested modal entry.
    if !unsafe { IsWindowEnabled(owner) }.as_bool() {
        return Ok(false);
    }
    let dialog = About::new();
    dialog.create(owner, true)?;
    // SAFETY: message loop remains on the owning thread, so parent timers and
    // backend cancellation still run. Owner enabled again on every exit path.
    unsafe {
        let _ = EnableWindow(owner, false);
        let mut message = MSG::default();
        let mut failed = false;
        while IsWindow(Some(dialog.hwnd.get())).as_bool() {
            let result = GetMessageW(&mut message, None, 0, 0).0;
            if result <= 0 {
                if result == 0 {
                    PostQuitMessage(message.wParam.0 as i32);
                } else {
                    failed = true;
                }
                break;
            }
            if !IsDialogMessageW(dialog.hwnd.get(), &message).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        let _ = EnableWindow(owner, true);
        let _ = SetForegroundWindow(owner);
        if failed {
            return Err(AppError::SubtitleWindow);
        }
    }
    Ok(dialog.launched.get())
}
unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // SAFETY: only our CreateWindowExW sends this stable Box in CREATESTRUCTW.
        // No mutable reference is held across dispatch; NCDESTROY clears the pointer.
        unsafe {
            if message == WM_NCCREATE {
                let create = &*(lparam.0 as *const CREATESTRUCTW);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
            }
            let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const About;
            if !pointer.is_null() {
                let state = &*pointer;
                match message {
                    WM_COMMAND => {
                        match wparam.0 & 0xffff {
                            CHECK => state.start(false),
                            INSTALL => state.start(true),
                            CLOSE | 2 => {
                                let _ = DestroyWindow(hwnd);
                            }
                            _ => {}
                        }
                        return LRESULT(0);
                    }
                    WM_TIMER if wparam.0 == POLL => {
                        state.poll();
                        return LRESULT(0);
                    }
                    WM_CLOSE => {
                        let _ = DestroyWindow(hwnd);
                        return LRESULT(0);
                    }
                    WM_NCDESTROY => {
                        let _ = KillTimer(Some(hwnd), POLL);
                        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    }
                    _ => {}
                }
            }
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
    }));
    result.unwrap_or(LRESULT(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn about_starts_offline_with_version_and_author_and_escape_only_closes() {
        // SAFETY: private hidden test window on this thread; no user app, key,
        // capture or network access. Both native windows are destroyed here.
        unsafe {
            let owner = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("About test"),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                600,
                400,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            let dialog = About::new();
            dialog.create(owner, false).unwrap();
            assert!(dialog.job.borrow().is_none());
            assert!(dialog.release.borrow().is_none());
            assert!(IsWindowEnabled(dialog.check.get()).as_bool());
            assert!(!IsWindowEnabled(dialog.install.get()).as_bool());
            let first = GetWindow(dialog.hwnd.get(), GW_CHILD).unwrap();
            let mut text = [0u16; 256];
            let length = GetWindowTextW(first, &mut text) as usize;
            let text = String::from_utf16_lossy(&text[..length]);
            assert!(text.contains(update::VERSION));
            assert!(text.contains(update::AUTHOR));
            SendMessageW(dialog.hwnd.get(), WM_COMMAND, Some(WPARAM(2)), None);
            assert!(!IsWindow(Some(dialog.hwnd.get())).as_bool());
            assert!(!dialog.launched.get());
            assert!(dialog.job.borrow().is_none());
            drop(dialog);
            DestroyWindow(owner).unwrap();
        }
    }
}
