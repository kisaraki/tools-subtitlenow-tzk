//! Owned child controls; parent HWND destroys them. Keys are provider-specific,
//! masked and RAM-only unless the user explicitly chooses secure storage.
use crate::{
    ai::{
        credentials::{ApiKey, CredentialError, MAX_KEY_BLOB_BYTES},
        provider::SpeechProvider,
    },
    domain::{SourceLanguage, TranslationQuality},
    error::{AppError, Result},
    ui::{
        controls::{ControlPreferences, ControlUpdate, SourceChoice, StartRequest},
        subtitle_source::SubtitleSourceSettings,
    },
};
use std::{
    cell::{Cell, RefCell},
    sync::Arc,
};
use windows::{
    Win32::{
        Foundation::{HINSTANCE, HWND, LPARAM, WPARAM},
        Graphics::Gdi::*,
        UI::{
            Controls::{
                DRAWITEMSTRUCT, EM_SETCUEBANNER, ICC_PROGRESS_CLASS, INITCOMMONCONTROLSEX,
                InitCommonControlsEx, ODS_DISABLED, ODS_FOCUS, ODS_SELECTED, PBM_SETPOS,
                PBM_SETRANGE32,
            },
            HiDpi::GetDpiForWindow,
            Input::KeyboardAndMouse::EnableWindow,
            WindowsAndMessaging::*,
        },
    },
    core::{HSTRING, PCWSTR, w},
};
pub(super) const SOURCE: usize = 401;
pub(super) const PROVIDER: usize = 402;
const PROVIDER_PROMPT: &str = "選擇服務";
pub(super) const LANGUAGE: usize = 403;
pub(super) const QUALITY: usize = 404;
pub(super) const OPENAI_KEY: usize = 405;
pub(super) const START: usize = 406;
pub(super) const STOP_BUTTON: usize = 407;
pub(super) const DETAILS: usize = 408;
pub(super) const METER: usize = 409;
pub(super) const GEMINI_KEY: usize = 410;
pub(super) const SAVE_OPENAI_KEY: usize = 411;
pub(super) const SAVE_GEMINI_KEY: usize = 412;
const OPENAI_LABEL: usize = 413;
const GEMINI_LABEL: usize = 414;
const KEY_NOTICE: usize = 415;
pub(super) const SOURCE_SUBTITLE: usize = 416;
const SUBTITLE_NOTICE: usize = 417;
pub(super) const PREPARED_MEDIA: usize = 418;
pub(super) const CHECK_OPENAI_KEY: usize = 419;
pub(super) const CHECK_GEMINI_KEY: usize = 420;
pub(super) const SAVE_PREFERENCES: usize = 422;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ControlPage {
    Subtitles,
    Sources,
    Credentials,
}
pub(super) const SOURCE_ROW_DIP: i32 = 40;
pub(super) const SOURCE_PANEL_HEIGHT: i32 = 180;
pub(super) const CREDENTIAL_PANEL_HEIGHT: i32 = 300;
const DETAIL_HEIGHT_DIP: i32 = 62;
const METER_HEIGHT_DIP: i32 = 8;
const KEY_LABEL_WIDTH_DIP: i32 = 180;
const KEY_SAVE_WIDTH_DIP: i32 = 60;
const KEY_CHECK_WIDTH_DIP: i32 = 100;
const KEY_NOTICE_HEIGHT_DIP: i32 = 50;
const SUBTITLE_SOURCE_HEIGHT_DIP: i32 = 72;
const SUBTITLE_SOURCE_BUTTON_DIP: i32 = 180;
const KEY_GUIDANCE: &str =
    "儲存後清空欄位；未儲存只供本次啟動。\r\n留空用已儲存金鑰；「檢查本機」不連線。";
const KEY_LIMIT: usize = MAX_KEY_BLOB_BYTES / 2;
const EM_SETLIMITTEXT_VALUE: u32 = 0x00c5;
const EM_EMPTYUNDOBUFFER_VALUE: u32 = 0x00cd;
const DROPDOWN_ROWS: i32 = 6;
pub(super) struct ControlPanel {
    pub source: HWND,
    pub provider: HWND,
    pub language: HWND,
    pub quality: HWND,
    pub openai_key: HWND,
    pub gemini_key: HWND,
    pub save_openai_key: HWND,
    pub save_gemini_key: HWND,
    pub check_openai_key: HWND,
    pub check_gemini_key: HWND,
    pub save_preferences: HWND,
    pub openai_label: HWND,
    pub gemini_label: HWND,
    pub key_notice: HWND,
    pub subtitle_source: HWND,
    pub subtitle_notice: HWND,
    pub prepared_media: HWND,
    pub start: HWND,
    pub stop: HWND,
    pub details: HWND,
    pub meter: HWND,
    sources: RefCell<Arc<Vec<SourceChoice>>>,
    latest: RefCell<Arc<Vec<SourceChoice>>>,
    last_details: RefCell<Option<(String, Option<u16>)>>,
    busy: Cell<bool>,
    auto_language_available: Cell<bool>,
    page: Cell<ControlPage>,
    rebuilding: Cell<bool>,
    subtitle_settings: RefCell<Option<SubtitleSourceSettings>>,
}
struct Rebuilding<'a>(&'a Cell<bool>);
impl Drop for Rebuilding<'_> {
    fn drop(&mut self) {
        self.0.set(false);
    }
}
impl ControlPanel {
    pub fn create(
        parent: HWND,
        instance: HINSTANCE,
        preferences: ControlPreferences,
    ) -> Result<Self> {
        // SAFETY: controls belong to the parent's UI thread; parent owns their lifetime.
        unsafe {
            if !InitCommonControlsEx(&INITCOMMONCONTROLSEX {
                dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_PROGRESS_CLASS,
            })
            .as_bool()
            {
                return Err(AppError::SubtitleWindow);
            }
            let combo = |id| {
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("COMBOBOX"),
                    w!(""),
                    WS_CHILD
                        | WS_VISIBLE
                        | WS_TABSTOP
                        | WS_VSCROLL
                        | WINDOW_STYLE(
                            (CBS_DROPDOWNLIST
                                | if id == SOURCE {
                                    CBS_OWNERDRAWFIXED | CBS_HASSTRINGS
                                } else {
                                    0
                                }) as u32,
                        ),
                    0,
                    0,
                    1,
                    180,
                    Some(parent),
                    Some(HMENU(id as *mut _)),
                    Some(instance),
                    None,
                )
                .map_err(|_| AppError::SubtitleWindow)
            };
            let button = |id, title: PCWSTR| {
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("BUTTON"),
                    title,
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP,
                    0,
                    0,
                    1,
                    1,
                    Some(parent),
                    Some(HMENU(id as *mut _)),
                    Some(instance),
                    None,
                )
                .map_err(|_| AppError::SubtitleWindow)
            };
            let password = |id| {
                CreateWindowExW(
                    WS_EX_CLIENTEDGE,
                    w!("EDIT"),
                    w!(""),
                    WS_CHILD
                        | WS_VISIBLE
                        | WS_TABSTOP
                        | WINDOW_STYLE((ES_PASSWORD | ES_AUTOHSCROLL) as u32),
                    0,
                    0,
                    1,
                    1,
                    Some(parent),
                    Some(HMENU(id as *mut _)),
                    Some(instance),
                    None,
                )
                .map_err(|_| AppError::SubtitleWindow)
            };
            let label = |id, text: &str| {
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("STATIC"),
                    &HSTRING::from(text),
                    WS_CHILD | WS_VISIBLE,
                    0,
                    0,
                    1,
                    1,
                    Some(parent),
                    Some(HMENU(id as *mut _)),
                    Some(instance),
                    None,
                )
                .map_err(|_| AppError::SubtitleWindow)
            };
            let panel = Self {
                source: combo(SOURCE)?,
                provider: combo(PROVIDER)?,
                language: combo(LANGUAGE)?,
                quality: combo(QUALITY)?,
                openai_label: label(OPENAI_LABEL, "OpenAI（ChatGPT）")?,
                openai_key: password(OPENAI_KEY)?,
                save_openai_key: button(SAVE_OPENAI_KEY, w!("儲存"))?,
                check_openai_key: button(CHECK_OPENAI_KEY, w!("檢查本機"))?,
                gemini_label: label(GEMINI_LABEL, "Gemini API")?,
                gemini_key: password(GEMINI_KEY)?,
                save_gemini_key: button(SAVE_GEMINI_KEY, w!("儲存"))?,
                check_gemini_key: button(CHECK_GEMINI_KEY, w!("檢查本機"))?,
                save_preferences: button(SAVE_PREFERENCES, w!("儲存服務與偏好"))?,
                key_notice: label(KEY_NOTICE, KEY_GUIDANCE)?,
                subtitle_source: button(SOURCE_SUBTITLE, w!("字幕來源（本次）"))?,
                subtitle_notice: label(SUBTITLE_NOTICE, "字幕來源：未設定（只供本次）")?,
                prepared_media: button(PREPARED_MEDIA, w!("預先準備（本次）"))?,
                start: button(START, w!("開始翻譯"))?,
                stop: button(STOP_BUTTON, w!("停止"))?,
                details: CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("STATIC"),
                    w!("請先選擇來源；尚未取得音訊活動資料。"),
                    WS_CHILD | WS_VISIBLE,
                    0,
                    0,
                    1,
                    1,
                    Some(parent),
                    Some(HMENU(DETAILS as *mut _)),
                    Some(instance),
                    None,
                )
                .map_err(|_| AppError::SubtitleWindow)?,
                meter: CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("msctls_progress32"),
                    w!("所選音訊工作階段峰值"),
                    WS_CHILD,
                    0,
                    0,
                    1,
                    1,
                    Some(parent),
                    Some(HMENU(METER as *mut _)),
                    Some(instance),
                    None,
                )
                .map_err(|_| AppError::SubtitleWindow)?,
                sources: RefCell::new(Arc::new(Vec::new())),
                latest: RefCell::new(Arc::new(Vec::new())),
                last_details: RefCell::new(None),
                busy: Cell::new(false),
                auto_language_available: Cell::new(true),
                page: Cell::new(ControlPage::Subtitles),
                rebuilding: Cell::new(false),
                subtitle_settings: RefCell::new(None),
            };
            add(panel.source, "請選擇程序音訊來源（含子程序）")?;
            for text in [
                PROVIDER_PROMPT,
                "OpenAI · gpt-live-transcribe",
                "Gemini · Transcribe Live",
                "Gemini 3.5 Live Translate（中文串流）",
            ] {
                add(panel.provider, text)?;
            }
            for text in ["自動偵測", "英文", "日文", "法文", "德文", "韓文"] {
                add(panel.language, text)?;
            }
            for text in ["快速", "標準", "高品質"] {
                add(panel.quality, text)?;
            }
            for (control, index) in [
                (panel.source, 0),
                (
                    panel.provider,
                    if preferences.live_model
                        == Some(crate::ai::provider::LiveSpeechModel::GeminiLiveTranslate)
                    {
                        3
                    } else {
                        match preferences.provider {
                            None => 0,
                            Some(SpeechProvider::OpenAi) => 1,
                            Some(SpeechProvider::Gemini) => 2,
                        }
                    },
                ),
                (
                    panel.language,
                    match preferences.language {
                        SourceLanguage::Auto => 0,
                        SourceLanguage::English => 1,
                        SourceLanguage::Japanese => 2,
                        SourceLanguage::French => 3,
                        SourceLanguage::German => 4,
                        SourceLanguage::Korean => 5,
                    },
                ),
                (
                    panel.quality,
                    match preferences.quality {
                        TranslationQuality::Fast => 0,
                        TranslationQuality::Standard => 1,
                        TranslationQuality::High => 2,
                    },
                ),
            ] {
                SendMessageW(control, CB_SETCURSEL, Some(WPARAM(index)), None);
            }
            for control in [panel.openai_key, panel.gemini_key] {
                SendMessageW(
                    control,
                    EM_SETLIMITTEXT_VALUE,
                    Some(WPARAM(KEY_LIMIT)),
                    None,
                );
                SendMessageW(
                    control,
                    EM_SETCUEBANNER,
                    Some(WPARAM(1)),
                    Some(LPARAM(w!("輸入 API 金鑰（遮蔽）").as_ptr() as isize)),
                );
            }
            panel.sync_language_capability()?;
            panel.enable(false, false);
            SendMessageW(
                panel.meter,
                PBM_SETRANGE32,
                Some(WPARAM(0)),
                Some(LPARAM(1000)),
            );
            Ok(panel)
        }
    }
    pub fn handles(&self) -> [HWND; 21] {
        [
            self.source,
            self.provider,
            self.language,
            self.quality,
            self.openai_label,
            self.openai_key,
            self.save_openai_key,
            self.gemini_label,
            self.gemini_key,
            self.save_gemini_key,
            self.check_openai_key,
            self.check_gemini_key,
            self.save_preferences,
            self.key_notice,
            self.subtitle_source,
            self.subtitle_notice,
            self.prepared_media,
            self.start,
            self.stop,
            self.details,
            self.meter,
        ]
    }
    pub fn buttons(&self) -> [HWND; 9] {
        [
            self.start,
            self.stop,
            self.save_openai_key,
            self.save_gemini_key,
            self.check_openai_key,
            self.check_gemini_key,
            self.save_preferences,
            self.subtitle_source,
            self.prepared_media,
        ]
    }
    pub fn set_visible(&self, page: ControlPage) {
        self.page.set(page);
        let meter_visible = page == ControlPage::Sources
            && self
                .last_details
                .borrow()
                .as_ref()
                .is_some_and(|(_, peak)| peak.is_some());
        // SAFETY: all controls are owned by the same UI thread; no focus or user data is read.
        unsafe {
            for control in self.handles() {
                let source = [
                    self.details,
                    self.subtitle_source,
                    self.subtitle_notice,
                    self.prepared_media,
                ]
                .contains(&control);
                let action = [
                    self.start,
                    self.stop,
                    self.source,
                    self.language,
                    self.quality,
                ]
                .contains(&control);
                let show = if action {
                    true
                } else if control == self.meter {
                    meter_visible
                } else if source {
                    page == ControlPage::Sources
                } else {
                    page == ControlPage::Credentials
                };
                if (GetWindowLongPtrW(control, GWL_STYLE) as u32 & WS_VISIBLE.0 != 0) != show {
                    let _ = ShowWindow(control, if show { SW_SHOWNA } else { SW_HIDE });
                }
            }
        }
    }
    pub fn layout(&self, width: i32, pad: i32, row: i32) -> Result<()> {
        // SAFETY: all HWNDs are owned child controls on this UI thread, sizes clamped.
        unsafe {
            let full = (width - 2 * pad).max(1);
            let gap = pad;
            let third = ((full - 2 * gap) / 3).max(1);
            let dpi = GetDpiForWindow(self.source);
            let source_height = super::subtitle_window::scale(SOURCE_ROW_DIP, dpi);
            let detail_height = super::subtitle_window::scale(DETAIL_HEIGHT_DIP, dpi);
            let meter_height = super::subtitle_window::scale(METER_HEIGHT_DIP, dpi);
            MoveWindow(
                self.source,
                pad,
                pad,
                full,
                source_height * DROPDOWN_ROWS,
                true,
            )
            .map_err(|_| AppError::SubtitleWindow)?;
            let y = pad + source_height + gap;
            MoveWindow(
                self.details,
                pad,
                y,
                full,
                detail_height - meter_height - pad / 2,
                true,
            )
            .map_err(|_| AppError::SubtitleWindow)?;
            MoveWindow(
                self.meter,
                pad,
                y + detail_height - meter_height,
                full,
                meter_height,
                true,
            )
            .map_err(|_| AppError::SubtitleWindow)?;
            let y = y + detail_height + gap;
            for (index, control) in [self.provider, self.language, self.quality]
                .into_iter()
                .enumerate()
            {
                MoveWindow(
                    control,
                    pad + index as i32 * (third + gap),
                    y,
                    third,
                    row * DROPDOWN_ROWS,
                    true,
                )
                .map_err(|_| AppError::SubtitleWindow)?;
            }
            let mut y = y + row + gap;
            let label_width = super::subtitle_window::scale(KEY_LABEL_WIDTH_DIP, dpi).min(full / 2);
            let save_width = super::subtitle_window::scale(KEY_SAVE_WIDTH_DIP, dpi).min(full / 4);
            let check_width = super::subtitle_window::scale(KEY_CHECK_WIDTH_DIP, dpi).min(full / 4);
            let edit_width = (full - label_width - save_width - check_width - 3 * gap).max(1);
            for (label, edit, save, check) in [
                (
                    self.openai_label,
                    self.openai_key,
                    self.save_openai_key,
                    self.check_openai_key,
                ),
                (
                    self.gemini_label,
                    self.gemini_key,
                    self.save_gemini_key,
                    self.check_gemini_key,
                ),
            ] {
                MoveWindow(label, pad, y, label_width, row, true)
                    .map_err(|_| AppError::SubtitleWindow)?;
                MoveWindow(edit, pad + label_width + gap, y, edit_width, row, true)
                    .map_err(|_| AppError::SubtitleWindow)?;
                MoveWindow(
                    save,
                    width - pad - check_width - gap - save_width,
                    y,
                    save_width,
                    row,
                    true,
                )
                .map_err(|_| AppError::SubtitleWindow)?;
                MoveWindow(check, width - pad - check_width, y, check_width, row, true)
                    .map_err(|_| AppError::SubtitleWindow)?;
                y += row + gap;
            }
            let notice_height = super::subtitle_window::scale(KEY_NOTICE_HEIGHT_DIP, dpi);
            MoveWindow(self.key_notice, pad, y, full, notice_height, true)
                .map_err(|_| AppError::SubtitleWindow)?;
            let y = y + notice_height + gap;
            let subtitle_height = super::subtitle_window::scale(SUBTITLE_SOURCE_HEIGHT_DIP, dpi);
            let subtitle_button =
                super::subtitle_window::scale(SUBTITLE_SOURCE_BUTTON_DIP, dpi).min(full / 2);
            MoveWindow(self.subtitle_source, pad, y, subtitle_button, row, true)
                .map_err(|_| AppError::SubtitleWindow)?;
            MoveWindow(
                self.prepared_media,
                pad,
                y + row + gap,
                subtitle_button,
                row,
                true,
            )
            .map_err(|_| AppError::SubtitleWindow)?;
            MoveWindow(
                self.subtitle_notice,
                pad + subtitle_button + gap,
                y,
                (full - subtitle_button - gap).max(1),
                subtitle_height,
                true,
            )
            .map_err(|_| AppError::SubtitleWindow)?;
            let y = y + subtitle_height + gap;
            let stop_width = third;
            MoveWindow(
                self.start,
                pad,
                y,
                (full - stop_width - gap).max(1),
                row,
                true,
            )
            .map_err(|_| AppError::SubtitleWindow)?;
            MoveWindow(
                self.stop,
                width - pad - stop_width,
                y,
                stop_width,
                row,
                true,
            )
            .map_err(|_| AppError::SubtitleWindow)?;
        }
        Ok(())
    }
    pub fn layout_header(&self, width: i32, pad: i32, row: i32) -> Result<i32> {
        // SAFETY: shared same-thread source/language controls, all sizes bounded.
        unsafe {
            let full = (width - 2 * pad).max(1);
            let source_height =
                super::subtitle_window::scale(SOURCE_ROW_DIP, GetDpiForWindow(self.source));
            MoveWindow(
                self.source,
                pad,
                pad,
                full,
                source_height * DROPDOWN_ROWS,
                true,
            )
            .map_err(|_| AppError::SubtitleWindow)?;
            let half = ((full - pad) / 2).max(1);
            for (index, control) in [self.language, self.quality].into_iter().enumerate() {
                MoveWindow(
                    control,
                    pad + index as i32 * (half + pad),
                    pad + source_height + pad,
                    half,
                    row * DROPDOWN_ROWS,
                    true,
                )
                .map_err(|_| AppError::SubtitleWindow)?;
            }
            Ok(source_height + row + 2 * pad)
        }
    }
    pub fn layout_page(&self, width: i32, pad: i32, row: i32, page: ControlPage) -> Result<i32> {
        if page != ControlPage::Subtitles {
            self.layout(width, pad, row)?;
        }
        let header = self.layout_header(width, pad, row)?;
        if page == ControlPage::Subtitles {
            return Ok(header);
        }
        // SAFETY: page bodies use our own child HWNDs; keys are never read by layout.
        unsafe {
            let full = (width - 2 * pad).max(1);
            let dpi = GetDpiForWindow(self.source);
            let body = header + pad;
            if page == ControlPage::Sources {
                let detail = super::subtitle_window::scale(DETAIL_HEIGHT_DIP, dpi);
                let meter = super::subtitle_window::scale(METER_HEIGHT_DIP, dpi);
                MoveWindow(
                    self.details,
                    pad,
                    body,
                    full,
                    detail - meter - pad / 2,
                    true,
                )
                .map_err(|_| AppError::SubtitleWindow)?;
                MoveWindow(self.meter, pad, body + detail - meter, full, meter, true)
                    .map_err(|_| AppError::SubtitleWindow)?;
                let y = body + detail + pad;
                let button =
                    super::subtitle_window::scale(SUBTITLE_SOURCE_BUTTON_DIP, dpi).min(full / 2);
                MoveWindow(self.subtitle_source, pad, y, button, row, true)
                    .map_err(|_| AppError::SubtitleWindow)?;
                MoveWindow(self.prepared_media, pad, y + row + pad, button, row, true)
                    .map_err(|_| AppError::SubtitleWindow)?;
                MoveWindow(
                    self.subtitle_notice,
                    pad + button + pad,
                    y,
                    (full - button - pad).max(1),
                    super::subtitle_window::scale(SUBTITLE_SOURCE_HEIGHT_DIP, dpi),
                    true,
                )
                .map_err(|_| AppError::SubtitleWindow)?;
                Ok(header + super::subtitle_window::scale(SOURCE_PANEL_HEIGHT, dpi))
            } else {
                MoveWindow(self.provider, pad, body, full, row * DROPDOWN_ROWS, true)
                    .map_err(|_| AppError::SubtitleWindow)?;
                let label = super::subtitle_window::scale(KEY_LABEL_WIDTH_DIP, dpi).min(full / 2);
                let save = super::subtitle_window::scale(KEY_SAVE_WIDTH_DIP, dpi).min(full / 4);
                let check = super::subtitle_window::scale(KEY_CHECK_WIDTH_DIP, dpi).min(full / 4);
                let edit = (full - label - save - check - 3 * pad).max(1);
                let mut y = body + row + pad;
                for (label_hwnd, edit_hwnd, save_hwnd, check_hwnd) in [
                    (
                        self.openai_label,
                        self.openai_key,
                        self.save_openai_key,
                        self.check_openai_key,
                    ),
                    (
                        self.gemini_label,
                        self.gemini_key,
                        self.save_gemini_key,
                        self.check_gemini_key,
                    ),
                ] {
                    for (control, x, size) in [
                        (label_hwnd, pad, label),
                        (edit_hwnd, pad + label + pad, edit),
                        (save_hwnd, width - pad - check - pad - save, save),
                        (check_hwnd, width - pad - check, check),
                    ] {
                        MoveWindow(control, x, y, size, row, true)
                            .map_err(|_| AppError::SubtitleWindow)?;
                    }
                    y += row + pad;
                }
                let notice = super::subtitle_window::scale(KEY_NOTICE_HEIGHT_DIP, dpi);
                MoveWindow(self.key_notice, pad, y, full, notice, true)
                    .map_err(|_| AppError::SubtitleWindow)?;
                y += notice + pad;
                MoveWindow(self.save_preferences, pad, y, full, row, true)
                    .map_err(|_| AppError::SubtitleWindow)?;
                Ok(header + super::subtitle_window::scale(CREDENTIAL_PANEL_HEIGHT, dpi))
            }
        }
    }
    pub fn update(&self, update: &ControlUpdate, can_stop: bool) -> Result<bool> {
        if self.busy.get() && !update.busy {
            self.clear_subtitle_source()?;
        }
        let previous = self.selected_source();
        let metadata_changed = !Arc::ptr_eq(&self.latest.borrow(), &update.sources);
        *self.latest.borrow_mut() = update.sources.clone();
        // SAFETY: synchronous, owned combo. No RefCell borrow crosses reentrant messages.
        unsafe {
            let current = self.sources.borrow().clone();
            if !update.busy
                && (current.len() != update.sources.len()
                    || !current
                        .iter()
                        .zip(update.sources.iter())
                        .all(|(old, next)| old.same_option(next)))
                && SendMessageW(self.source, CB_GETDROPPEDSTATE, None, None).0 == 0
            {
                let chosen = selected(self.source)
                    .checked_sub(1)
                    .and_then(|index| current.get(index as usize))
                    .cloned();
                self.rebuilding.set(true);
                let _rebuilding = Rebuilding(&self.rebuilding);
                SendMessageW(self.source, CB_RESETCONTENT, None, None);
                add(self.source, "請選擇程序音訊來源（含子程序）")?;
                for source in update.sources.iter() {
                    add(self.source, &source.label)?;
                }
                let selected = chosen
                    .and_then(|chosen| {
                        update.sources.iter().position(|source| {
                            source.id == chosen.id
                                && source.pid == chosen.pid
                                && source.created == chosen.created
                        })
                    })
                    .map_or(0, |index| index + 1);
                *self.sources.borrow_mut() = update.sources.clone();
                SendMessageW(self.source, CB_SETCURSEL, Some(WPARAM(selected)), None);
            }
        }
        self.update_details(&update.sources)?;
        // SAFETY: invalidate only our source selector; owner-draw reads fresh metadata.
        unsafe {
            if metadata_changed {
                let _ = InvalidateRect(Some(self.source), None, false);
            }
        }
        self.enable(update.busy, can_stop);
        let next = self.selected_source();
        let changed = match (previous, next) {
            (Some(old), Some(next)) => !old.same_source(&next),
            (None, None) => false,
            _ => true,
        };
        if changed && !update.busy {
            self.clear_subtitle_source()?;
        }
        Ok(changed)
    }
    pub fn update_details(&self, latest: &[SourceChoice]) -> Result<()> {
        let chosen = self.selected_source();
        let metadata = chosen
            .as_ref()
            .and_then(|chosen| latest.iter().find(|source| source.same_source(chosen)));
        let text = match (chosen.as_ref(), metadata) {
            (None, _) => "請先選擇來源；尚未取得音訊活動資料。".into(),
            (Some(_), None) => "所選工作階段已消失或未取得最新資料。\r\n不會改選其他來源。".into(),
            (_, Some(source)) => source.activity.text(source.icon.is_some()),
        };
        let peak = metadata.and_then(|source| source.activity.peak_permille);
        if self
            .last_details
            .borrow()
            .as_ref()
            .is_some_and(|previous| previous.0 == text && previous.1 == peak)
        {
            return Ok(());
        }
        *self.last_details.borrow_mut() = Some((text.clone(), peak));
        // SAFETY: private static/progress controls; text lives through the copied call.
        unsafe {
            SetWindowTextW(self.details, &HSTRING::from(text))
                .map_err(|_| AppError::SubtitleWindow)?;
            SendMessageW(
                self.meter,
                PBM_SETPOS,
                Some(WPARAM(usize::from(peak.unwrap_or(0)))),
                None,
            );
            let _ = ShowWindow(
                self.meter,
                if peak.is_some() && self.page.get() == ControlPage::Sources {
                    SW_SHOWNA
                } else {
                    SW_HIDE
                },
            );
        }
        Ok(())
    }
    pub fn draw_source(&self, draw: &DRAWITEMSTRUCT, latest: &[SourceChoice]) -> Result<()> {
        let choice = if self.rebuilding.get() {
            None
        } else {
            draw.itemID
                .checked_sub(1)
                .and_then(|index| self.sources.borrow().get(index as usize).cloned())
        };
        let source = choice
            .as_ref()
            .and_then(|choice| latest.iter().find(|source| source.same_source(choice)));
        let label = choice
            .as_ref()
            .map(|choice| choice.label.as_str())
            .unwrap_or("請選擇程序音訊來源（含子程序）");
        // SAFETY: callback borrows a live destination DC on UI thread; SaveDC/guard restores
        // all drawing state, and metadata/label borrows contain no native resource ownership.
        unsafe {
            let saved = SaveDC(draw.hDC);
            if saved == 0 {
                return Err(AppError::SubtitleWindow);
            }
            struct Restore(HDC, i32);
            impl Drop for Restore {
                fn drop(&mut self) {
                    // SAFETY: restores the matching SaveDC on this callback's borrowed DC.
                    unsafe {
                        let _ = RestoreDC(self.0, self.1);
                    }
                }
            }
            let _restore = Restore(draw.hDC, saved);
            IntersectClipRect(
                draw.hDC,
                draw.rcItem.left,
                draw.rcItem.top,
                draw.rcItem.right,
                draw.rcItem.bottom,
            );
            let selected = draw.itemState.0 & ODS_SELECTED.0 != 0;
            let background = if selected {
                COLOR_HIGHLIGHT
            } else {
                COLOR_WINDOW
            };
            let color = if draw.itemState.0 & ODS_DISABLED.0 != 0 {
                COLOR_GRAYTEXT
            } else if selected {
                COLOR_HIGHLIGHTTEXT
            } else {
                COLOR_WINDOWTEXT
            };
            FillRect(draw.hDC, &draw.rcItem, GetSysColorBrush(background));
            SetBkMode(draw.hDC, TRANSPARENT);
            SetTextColor(
                draw.hDC,
                windows::Win32::Foundation::COLORREF(GetSysColor(color)),
            );
            let font = SendMessageW(self.source, WM_GETFONT, None, None).0;
            if font != 0 {
                SelectObject(draw.hDC, HGDIOBJ(font as *mut _));
            }
            let dpi = GetDpiForWindow(self.source);
            let pad = super::subtitle_window::scale(4, dpi);
            let size = super::subtitle_window::scale(28, dpi)
                .min(draw.rcItem.bottom - draw.rcItem.top)
                .max(1);
            let top = draw.rcItem.top + (draw.rcItem.bottom - draw.rcItem.top - size) / 2;
            if let Some(icon) = source.and_then(|source| source.icon.as_ref()) {
                super::subtitle_render::draw_icon(
                    draw.hDC,
                    windows::Win32::Foundation::RECT {
                        left: draw.rcItem.left + pad,
                        top,
                        right: draw.rcItem.left + pad + size,
                        bottom: top + size,
                    },
                    icon,
                )?;
            }
            let mut rect = draw.rcItem;
            rect.left += if choice.is_some() {
                size + 2 * pad
            } else {
                pad
            };
            rect.right -= pad;
            let mut text: Vec<u16> = label.encode_utf16().collect();
            DrawTextW(
                draw.hDC,
                &mut text,
                &mut rect,
                DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
            );
            if draw.itemState.0 & ODS_FOCUS.0 != 0 {
                let _ = DrawFocusRect(draw.hDC, &draw.rcItem);
            }
        }
        Ok(())
    }
    pub fn update_row_height(&self) {
        // SAFETY: own fixed-height combo; -1 denotes closed selection, 0 all list rows.
        unsafe {
            let height =
                super::subtitle_window::scale(SOURCE_ROW_DIP, GetDpiForWindow(self.source));
            for item in [usize::MAX, 0] {
                SendMessageW(
                    self.source,
                    CB_SETITEMHEIGHT,
                    Some(WPARAM(item)),
                    Some(LPARAM(height as isize)),
                );
            }
        }
    }
    pub fn enable(&self, busy: bool, can_stop: bool) {
        self.busy.set(busy);
        // SAFETY: EnableWindow can return zero for a previously enabled control.
        unsafe {
            for control in [
                self.source,
                self.provider,
                self.language,
                self.quality,
                self.openai_key,
                self.gemini_key,
                self.save_openai_key,
                self.save_gemini_key,
                self.save_preferences,
                self.check_openai_key,
                self.check_gemini_key,
                self.subtitle_source,
                self.prepared_media,
            ] {
                let _ = EnableWindow(control, !busy);
            }
            let _ = EnableWindow(self.start, !busy && self.request().is_ok());
            let _ = EnableWindow(self.stop, can_stop);
        }
    }
    pub fn options_changed(&self, id: usize) -> Result<()> {
        if self.busy.get() {
            return Ok(());
        }
        if id == SOURCE {
            self.clear_subtitle_source()?;
        }
        // Separate fixed fields never remap a key when the provider changes.
        self.sync_language_capability()?;
        self.enable(false, false);
        Ok(())
    }
    fn sync_language_capability(&self) -> Result<()> {
        let preferences = self.preferences()?;
        let supported = preferences
            .live_model
            .is_none_or(|model| model.supports_auto_detection());
        if self.auto_language_available.get() == supported {
            return Ok(());
        }
        // SAFETY: replace only our own non-secret combo's first static string.
        // Programmatic combo changes do not emit CBN_SELCHANGE. Preserve every
        // language index and the user's selection; never silently choose a language.
        unsafe {
            let current = selected(self.language);
            if current > 5 {
                return Err(AppError::InvalidArguments);
            }
            if SendMessageW(self.language, CB_DELETESTRING, Some(WPARAM(0)), None).0 < 0 {
                return Err(AppError::SubtitleWindow);
            }
            let label = if supported {
                w!("自動偵測")
            } else {
                w!("請手選語言")
            };
            if SendMessageW(
                self.language,
                CB_INSERTSTRING,
                Some(WPARAM(0)),
                Some(LPARAM(label.as_ptr() as isize)),
            )
            .0 != 0
            {
                return Err(AppError::SubtitleWindow);
            }
            self.auto_language_available.set(supported);
            SendMessageW(
                self.language,
                CB_SETCURSEL,
                Some(WPARAM(current as usize)),
                None,
            );
        }
        Ok(())
    }
    pub fn live_selection_notice(&self) -> Option<String> {
        let preferences = self.preferences().ok()?;
        let model = preferences.live_model?;
        if model == crate::ai::provider::LiveSpeechModel::GeminiTranscribeLive
            && preferences.language == SourceLanguage::Auto
            && model
                .validate_live(preferences.language, preferences.quality)
                .is_ok()
        {
            return Some("自動偵測：服務缺少語言標記時，會在本機依辨識文字估計來源語言；不確定段落將明示略過。可手選來源語言以避免估計。".into());
        }
        let error = model
            .validate_live(preferences.language, preferences.quality)
            .err()?;
        Some(
            if error == crate::ai::speech::SpeechError::ExplicitLanguageRequired {
                format!(
                    "請先手選語言：英、日、法、德或韓。\r\n{} 目前需手選語言；若要自動偵測，請至服務設定選 Gemini · Transcribe Live。",
                    model.model()
                )
            } else {
                format!("選項需調整：{error}\r\n模型：{}", model.model())
            },
        )
    }
    pub fn preferences(&self) -> Result<ControlPreferences> {
        // SAFETY: scalar reads of our own three non-secret combo selections.
        unsafe {
            let provider = match selected(self.provider) {
                0 => None,
                1 => Some(SpeechProvider::OpenAi),
                2 | 3 => Some(SpeechProvider::Gemini),
                _ => return Err(AppError::InvalidArguments),
            };
            let language = match selected(self.language) {
                0 => SourceLanguage::Auto,
                1 => SourceLanguage::English,
                2 => SourceLanguage::Japanese,
                3 => SourceLanguage::French,
                4 => SourceLanguage::German,
                5 => SourceLanguage::Korean,
                _ => return Err(AppError::InvalidArguments),
            };
            let quality = match selected(self.quality) {
                0 => TranslationQuality::Fast,
                1 => TranslationQuality::Standard,
                2 => TranslationQuality::High,
                _ => return Err(AppError::InvalidArguments),
            };
            Ok(ControlPreferences {
                provider,
                live_model: match selected(self.provider) {
                    1 => Some(crate::ai::provider::LiveSpeechModel::GptLiveTranscribe),
                    2 => Some(crate::ai::provider::LiveSpeechModel::GeminiTranscribeLive),
                    3 => Some(crate::ai::provider::LiveSpeechModel::GeminiLiveTranslate),
                    _ => None,
                },
                language,
                quality,
            })
        }
    }
    pub fn request(&self) -> Result<StartRequest> {
        // SAFETY: owned controls, returned indexes validated before use.
        unsafe {
            let index = selected(self.source)
                .checked_sub(1)
                .ok_or(AppError::InvalidArguments)?;
            let source = self
                .sources
                .borrow()
                .get(index as usize)
                .cloned()
                .ok_or(AppError::InvalidArguments)?;
            let provider = match selected(self.provider) {
                1 => SpeechProvider::OpenAi,
                2 | 3 => SpeechProvider::Gemini,
                _ => return Err(crate::ai::speech::SpeechError::NotConfigured.into()),
            };
            let language = match selected(self.language) {
                0 => SourceLanguage::Auto,
                1 => SourceLanguage::English,
                2 => SourceLanguage::Japanese,
                3 => SourceLanguage::French,
                4 => SourceLanguage::German,
                5 => SourceLanguage::Korean,
                _ => return Err(AppError::InvalidArguments),
            };
            let quality = match selected(self.quality) {
                0 => TranslationQuality::Fast,
                1 => TranslationQuality::Standard,
                2 => TranslationQuality::High,
                _ => return Err(AppError::InvalidArguments),
            };
            let live_model = self.preferences()?.live_model;
            live_model
                .unwrap_or_else(|| crate::ai::provider::LiveSpeechModel::default_for(provider))
                .validate_live(language, quality)?;
            Ok(StartRequest {
                source,
                provider,
                live_model,
                language,
                quality,
                subtitle_source: self.current_subtitle_source(),
            })
        }
    }
    pub fn selected_source(&self) -> Option<crate::ui::controls::SourceChoice> {
        // SAFETY: query our own combo, then validate the index against its current mapping.
        let index = unsafe { selected(self.source) }.checked_sub(1)?;
        self.sources.borrow().get(index as usize).cloned()
    }
    fn key_control(&self, provider: SpeechProvider) -> HWND {
        match provider {
            SpeechProvider::OpenAi => self.openai_key,
            SpeechProvider::Gemini => self.gemini_key,
        }
    }
    pub fn clear_keys(&self) -> Result<()> {
        let first = clear_password(self.openai_key);
        let second = clear_password(self.gemini_key);
        first.and(second) // Always attempt to clear both, including after an error.
    }
    pub fn take_key(&self, provider: SpeechProvider) -> Result<Option<ApiKey>> {
        let result = read_password(self.key_control(provider));
        self.clear_keys()?;
        result
    }
    pub fn take_key_to_save(&self, provider: SpeechProvider) -> Result<ApiKey> {
        let control = self.key_control(provider);
        let result = read_password(control);
        clear_password(control)?;
        result?.ok_or_else(|| CredentialError::Missing.into())
    }
    pub fn key_notice(&self, text: &str) -> Result<()> {
        // SAFETY: static text is a safe status string, copied synchronously; never a key.
        unsafe { SetWindowTextW(self.key_notice, &HSTRING::from(text)) }
            .map_err(|_| AppError::SubtitleWindow)
    }
    pub fn edit_subtitle_source(&self, parent: HWND, instance: HINSTANCE) -> Result<()> {
        if self.busy.get() {
            return Ok(());
        }
        // SAFETY: verify the caller supplied this control's owner, so the modal
        // dialog can never disable or send messages to an unrelated window.
        if unsafe { GetParent(self.subtitle_source) }.ok() != Some(parent) {
            return Err(AppError::SubtitleWindow);
        }
        let current = self.current_subtitle_source();
        let outcome = super::subtitle_source_dialog::edit(parent, instance, current.as_ref())?;
        self.apply_subtitle_outcome(outcome)
    }
    pub(super) fn apply_subtitle_outcome(
        &self,
        outcome: super::subtitle_source_dialog::Outcome,
    ) -> Result<()> {
        if self.busy.get() {
            return Ok(());
        }
        match outcome {
            super::subtitle_source_dialog::Outcome::Apply(settings) => {
                *self.subtitle_settings.borrow_mut() = Some(settings);
            }
            super::subtitle_source_dialog::Outcome::Clear => {
                self.subtitle_settings.borrow_mut().take();
            }
            super::subtitle_source_dialog::Outcome::Cancel => return Ok(()),
        }
        self.refresh_subtitle_notice()
    }
    pub fn current_subtitle_source(&self) -> Option<SubtitleSourceSettings> {
        self.subtitle_settings.borrow().clone()
    }
    pub fn clear_subtitle_source(&self) -> Result<()> {
        self.subtitle_settings.borrow_mut().take();
        self.refresh_subtitle_notice()
    }
    fn refresh_subtitle_notice(&self) -> Result<()> {
        let text = self.subtitle_settings.borrow().as_ref().map_or_else(
            || "字幕來源：未設定（只供本次）".to_owned(),
            |settings| {
                format!(
                    "字幕下載：{}\r\n只供本次；停止後清除。",
                    settings.destination()
                )
            },
        );
        // SAFETY: only a host disclosure or static guidance reaches our own label;
        // no signed URL path, query, fragment, or credential is displayed.
        unsafe { SetWindowTextW(self.subtitle_notice, &HSTRING::from(text)) }
            .map_err(|_| AppError::SubtitleWindow)
    }
}
fn clear_password(control: HWND) -> Result<()> {
    // SAFETY: UI-thread-owned password EDIT; the empty value is copied and its undo
    // history discarded synchronously. No external window or buffer is accessed.
    unsafe {
        let result = SetWindowTextW(control, w!("")).map_err(|_| AppError::SubtitleWindow);
        SendMessageW(control, EM_EMPTYUNDOBUFFER_VALUE, None, None);
        result
    }
}
fn read_password(control: HWND) -> Result<Option<ApiKey>> {
    // SAFETY: same-thread owned EDIT. Check actual length as SetWindowText can
    // bypass the interactive limit; oversized values must not become truncated keys.
    let expected = unsafe { GetWindowTextLengthW(control) }.max(0) as usize;
    if expected > KEY_LIMIT {
        return Err(CredentialError::InvalidEncoding.into());
    }
    if expected == 0 {
        return Ok(None);
    }
    let mut buffer = zeroize::Zeroizing::new(vec![0u16; KEY_LIMIT + 1]);
    // SAFETY: bounded writable UTF-16 slice stays alive through this synchronous call.
    let len = unsafe { GetWindowTextW(control, &mut buffer[..]) }.max(0) as usize;
    if len != expected {
        return Err(CredentialError::Unavailable.into());
    }
    let mut bytes = zeroize::Zeroizing::new(Vec::with_capacity(len * 2));
    bytes.extend(buffer[..len].iter().flat_map(|v| v.to_le_bytes()));
    Ok(Some(ApiKey::from_utf16le(&bytes)?))
}
unsafe fn selected(control: HWND) -> isize {
    // SAFETY: caller supplies its own same-thread combo; no pointer parameters.
    unsafe { SendMessageW(control, CB_GETCURSEL, None, None).0 }
}
unsafe fn add(control: HWND, text: &str) -> Result<()> {
    let text = HSTRING::from(text);
    // SAFETY: same-thread combo copies text synchronously before HSTRING is dropped.
    let result = unsafe {
        SendMessageW(
            control,
            CB_ADDSTRING,
            None,
            Some(LPARAM(text.as_ptr() as isize)),
        )
    }
    .0;
    if result < 0 {
        Err(AppError::SubtitleWindow)
    } else {
        Ok(())
    }
}
