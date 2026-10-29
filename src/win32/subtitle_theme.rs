//! UI-thread-only native styling; no content, preferences or backend access.
use crate::error::{AppError, Result};
use std::{marker::PhantomData, rc::Rc};
use windows::Win32::{
    Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::*,
    UI::{
        Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW},
        Controls::{
            DRAWITEMSTRUCT, EM_SETMARGINS, EM_SETRECTNP, ODS_DISABLED, ODS_FOCUS, ODS_NOFOCUSRECT,
            ODS_SELECTED,
        },
        HiDpi::GetDpiForWindow,
        WindowsAndMessaging::*,
    },
};

const BUTTON_CORNER_DIP: i32 = 8;
const BUTTON_TEXT_INSET_DIP: i32 = 4;
const EDIT_HORIZONTAL_INSET_DIP: i32 = 8;
const EDIT_VERTICAL_INSET_DIP: i32 = 4;

const fn rgb(red: u32, green: u32, blue: u32) -> COLORREF {
    COLORREF(red | green << 8 | blue << 16)
}

pub(super) struct Theme {
    background: HBRUSH,
    surface: HBRUSH,
    status: HBRUSH,
    pub text: COLORREF,
    pub muted: COLORREF,
    accent: COLORREF,
    accent_text: COLORREF,
    pressed: COLORREF,
    border: COLORREF,
    background_color: COLORREF,
    surface_color: COLORREF,
    status_color: COLORREF,
    disabled: COLORREF,
    disabled_text: COLORREF,
    high_contrast: bool,
    _ui_thread: PhantomData<Rc<()>>,
}

impl Theme {
    pub fn new() -> Result<Self> {
        let mut theme = Self {
            background: HBRUSH::default(),
            surface: HBRUSH::default(),
            status: HBRUSH::default(),
            text: rgb(25, 42, 62),
            muted: rgb(83, 99, 120),
            accent: rgb(33, 89, 199),
            accent_text: rgb(255, 255, 255),
            pressed: rgb(24, 65, 149),
            border: rgb(212, 222, 235),
            background_color: rgb(243, 246, 250),
            surface_color: rgb(255, 255, 255),
            status_color: rgb(233, 240, 251),
            disabled: rgb(228, 234, 242),
            disabled_text: rgb(97, 110, 128),
            high_contrast: false,
            _ui_thread: PhantomData,
        };
        // SAFETY: initialized correctly sized structure; synchronous system query.
        unsafe {
            let mut contrast = HIGHCONTRASTW {
                cbSize: std::mem::size_of::<HIGHCONTRASTW>() as u32,
                ..Default::default()
            };
            // If the query fails, prefer readable system colors over custom styling.
            theme.high_contrast = SystemParametersInfoW(
                SPI_GETHIGHCONTRAST,
                contrast.cbSize,
                Some((&mut contrast as *mut HIGHCONTRASTW).cast()),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
            .is_err()
                || contrast.dwFlags.0 & HCF_HIGHCONTRASTON.0 != 0;
            if theme.high_contrast {
                theme.text = COLORREF(GetSysColor(COLOR_WINDOWTEXT));
                theme.muted = theme.text;
                theme.accent = COLORREF(GetSysColor(COLOR_HIGHLIGHT));
                theme.accent_text = COLORREF(GetSysColor(COLOR_HIGHLIGHTTEXT));
                theme.pressed = theme.accent;
                theme.border = COLORREF(GetSysColor(COLOR_WINDOWTEXT));
                theme.background_color = COLORREF(GetSysColor(COLOR_BTNFACE));
                theme.surface_color = COLORREF(GetSysColor(COLOR_WINDOW));
                theme.status_color = theme.surface_color;
                theme.disabled = COLORREF(GetSysColor(COLOR_BTNFACE));
                theme.disabled_text = COLORREF(GetSysColor(COLOR_GRAYTEXT));
            }
            theme.background = CreateSolidBrush(theme.background_color);
            theme.surface = CreateSolidBrush(theme.surface_color);
            theme.status = CreateSolidBrush(theme.status_color);
        }
        if [theme.background, theme.surface, theme.status]
            .iter()
            .any(|brush| brush.0.is_null())
        {
            return Err(AppError::SubtitleWindow);
        }
        Ok(theme)
    }

    pub fn erase(&self, hwnd: HWND, dc: HDC) -> LRESULT {
        // SAFETY: borrowed callback DC and live UI-thread HWND; brush outlives paint.
        unsafe {
            let mut rect = RECT::default();
            if GetClientRect(hwnd, &mut rect).is_ok() {
                FillRect(dc, &rect, self.background);
            }
        }
        LRESULT(1)
    }

    pub fn control_color(&self, dc: HDC, surface: bool, status: bool) -> LRESULT {
        let (brush, background, foreground) = if status {
            (self.status, self.status_color, self.text)
        } else if surface {
            (self.surface, self.surface_color, self.text)
        } else {
            (self.background, self.background_color, self.muted)
        };
        // SAFETY: borrowed native child paint DC; the returned brush is owned by
        // this shell until all its HWNDs have been destroyed.
        unsafe {
            SetTextColor(dc, foreground);
            SetBkColor(dc, background);
        }
        LRESULT(brush.0 as isize)
    }

    pub fn draw_button(&self, draw: &DRAWITEMSTRUCT, primary: bool) {
        // SAFETY: callback-scoped draw data/DC and owned child HWND. Save/restore
        // all DC selections; stock objects are borrowed and never deleted.
        unsafe {
            let saved = SaveDC(draw.hDC);
            if saved == 0 {
                return;
            }
            let disabled = draw.itemState.0 & ODS_DISABLED.0 != 0;
            let down = draw.itemState.0 & ODS_SELECTED.0 != 0;
            let (fill, text) = if disabled {
                (self.disabled, self.disabled_text)
            } else if primary {
                (
                    if down { self.pressed } else { self.accent },
                    self.accent_text,
                )
            } else {
                (
                    if down {
                        self.status_color
                    } else {
                        self.surface_color
                    },
                    self.text,
                )
            };
            FillRect(draw.hDC, &draw.rcItem, self.background);
            SelectObject(draw.hDC, GetStockObject(DC_BRUSH));
            SelectObject(draw.hDC, GetStockObject(DC_PEN));
            SetDCBrushColor(draw.hDC, fill);
            SetDCPenColor(
                draw.hDC,
                if primary && !disabled {
                    fill
                } else {
                    self.border
                },
            );
            let radius =
                super::subtitle_window::scale(BUTTON_CORNER_DIP, GetDpiForWindow(draw.hwndItem));
            let _ = RoundRect(
                draw.hDC,
                draw.rcItem.left,
                draw.rcItem.top,
                draw.rcItem.right,
                draw.rcItem.bottom,
                radius,
                radius,
            );
            let font = SendMessageW(draw.hwndItem, WM_GETFONT, None, None).0;
            if font != 0 {
                SelectObject(draw.hDC, HGDIOBJ(font as *mut _));
            }
            SetTextColor(draw.hDC, text);
            SetBkMode(draw.hDC, TRANSPARENT);
            // Only a fixed button label is read; never an EDIT, key or subtitle.
            let mut label = [0u16; 160];
            let count = GetWindowTextW(draw.hwndItem, &mut label).max(0) as usize;
            let mut rect = draw.rcItem;
            let inset = super::subtitle_window::scale(
                BUTTON_TEXT_INSET_DIP,
                GetDpiForWindow(draw.hwndItem),
            );
            rect.left += inset;
            rect.right -= inset;
            DrawTextW(
                draw.hDC,
                &mut label[..count],
                &mut rect,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
            );
            if draw.itemState.0 & ODS_FOCUS.0 != 0 && draw.itemState.0 & ODS_NOFOCUSRECT.0 == 0 {
                let mut focus = draw.rcItem;
                let _ = InflateRect(&mut focus, -inset, -inset);
                let _ = DrawFocusRect(draw.hDC, &focus);
            }
            let _ = RestoreDC(draw.hDC, saved);
        }
    }
}

impl Drop for Theme {
    fn drop(&mut self) {
        // SAFETY: unique brushes, never selected into a DC. The owner destroys
        // its windows before these fields drop; partial construction is safe.
        unsafe {
            for brush in [self.background, self.surface, self.status] {
                if !brush.0.is_null() {
                    let _ = DeleteObject(HGDIOBJ(brush.0));
                }
            }
        }
    }
}

pub(super) fn style_button(hwnd: HWND) {
    // SAFETY: explicit owned push-button handles only. Keep native text, command,
    // focus, keyboard and accessibility behavior; only replace the paint type.
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        SetWindowLongPtrW(
            hwnd,
            GWL_STYLE,
            ((style & !(BS_TYPEMASK as u32)) | BS_OWNERDRAW as u32) as isize,
        );
    }
}

pub(super) fn inset_edit(hwnd: HWND, multiline: bool) {
    // SAFETY: owned EDIT, synchronous formatting only; no text or selection reads.
    unsafe {
        let dpi = GetDpiForWindow(hwnd);
        let pad = super::subtitle_window::scale(EDIT_HORIZONTAL_INSET_DIP, dpi);
        if multiline {
            let mut rect = RECT::default();
            if GetClientRect(hwnd, &mut rect).is_ok() {
                rect.left = pad.min((rect.right / 4).max(0));
                rect.right = (rect.right - rect.left).max(rect.left + 1);
                rect.top = super::subtitle_window::scale(EDIT_VERTICAL_INSET_DIP, dpi)
                    .min(rect.bottom / 4);
                rect.bottom = (rect.bottom - rect.top).max(rect.top + 1);
                SendMessageW(
                    hwnd,
                    EM_SETRECTNP,
                    None,
                    Some(LPARAM((&rect as *const RECT) as isize)),
                );
            }
        } else {
            SendMessageW(
                hwnd,
                EM_SETMARGINS,
                Some(WPARAM((EC_LEFTMARGIN | EC_RIGHTMARGIN) as usize)),
                Some(LPARAM((pad | pad << 16) as isize)),
            );
        }
    }
}
