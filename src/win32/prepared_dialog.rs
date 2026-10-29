//! Owned Prepared controls. Worker polling never performs disk or network I/O.
//! Private paths and password edits are cleared before native destruction.
use crate::{
    ai::{
        credentials::{ApiKey, CredentialError, MAX_KEY_BLOB_BYTES},
        prepared_speech::PreparedSpeechError,
        provider::SpeechProvider,
        speech::SpeechLanguage,
    },
    app::prepared_discovery::{DiscoveryState, PreparedDiscovery},
    domain::{SourceLanguage, TranslationQuality},
    error::{AppError, Result},
    media::{
        MediaError,
        embedded_mp4::{Mp4SubtitleTrack, Mp4SubtitleTracks},
    },
    resolver::{contracts::TextLanguage, timed_subtitle::SubtitleFormat},
    ui::prepared::{
        PreparedBackend, PreparedInputDraft, PreparedOptions, PreparedUiError, PreparedUiState,
        is_remote_media_text,
    },
};
use std::cell::{Cell, RefCell};
use windows::{
    Win32::{
        Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::Gdi::{
            COLOR_WINDOW, DEFAULT_GUI_FONT, GetDC, GetMonitorInfoW, GetStockObject,
            GetTextMetricsW, HDC, HGDIOBJ, MONITOR_DEFAULTTONEAREST, MONITORINFO, MapWindowPoints,
            MonitorFromWindow, ReleaseDC, SelectObject, TEXTMETRICW,
        },
        UI::{
            Controls::SetScrollInfo,
            HiDpi::GetDpiForWindow,
            Input::KeyboardAndMouse::{EnableWindow, GetFocus, IsWindowEnabled, SetFocus},
            Shell::{
                NIF_ICON, NIF_INFO, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE, NIM_MODIFY,
                NOTIFYICONDATAW, Shell_NotifyIconW,
            },
            WindowsAndMessaging::*,
        },
    },
    core::{HSTRING, PCWSTR, w},
};

const CLASS: PCWSTR = w!("SubtitleNow.Prepared.Phase8");
const CONTENT_CLASS: PCWSTR = w!("SubtitleNow.PreparedContent.Phase8");
const MEDIA: usize = 801;
const SUBTITLE: usize = 802;
const FORMAT: usize = 803;
const EMBEDDED_FORMAT_INDEX: isize = 3;
const HTTPS_SRT_FORMAT_INDEX: isize = 4;
const HTTPS_WEBVTT_FORMAT_INDEX: isize = 5;
const LANGUAGE: usize = 804;
const QUALITY: usize = 805;
const PROVIDER: usize = 806;
const OPENAI_KEY: usize = 807;
const GEMINI_KEY: usize = 808;
const PREPARE: usize = 809;
const CANCEL: usize = 810;
const CLEAR: usize = 811;
const CLOSE: usize = 812;
const DESTINATION: usize = 813;
const AUDIO_LANGUAGE: usize = 814;
const DISCOVER: usize = 815;
const TRACK: usize = 816;
const REFERENCE_PATH: usize = 817;
const REFERENCE_FORMAT: usize = 818;
const REFERENCE_LANGUAGE: usize = 819;
const REFERENCE_EDITION: usize = 820;
const KEEP_READY: usize = 821;
const POLL_TIMER: usize = 1;
const POLL_INTERVAL_MS: u32 = 100;
const PATH_LIMIT: usize = 4096;
const KEY_LIMIT: usize = MAX_KEY_BLOB_BYTES / 2;
const WIDTH_DIP: i32 = 640;
const HEIGHT_DIP: i32 = 800;
const CONTENT_HEIGHT_DIP: i32 = 880;
// Destination disclosure stays visible with the actions even when the form
// needs scrolling at 200% scaling or on a monitor with a small work area.
const BUTTON_BAND_DIP: i32 = 160;
const DESTINATION_HEIGHT_DIP: i32 = 88;
const FIRST_ACTION_CONTROL: usize = 34;
const DISCOVER_CONTROL_INDEX: usize = 23;
const CONTROL_COUNT: usize = 38;
const SCROLL_LINE_DIP: i32 = 28;
const EM_SETLIMITTEXT: u32 = 0x00c5;
const EM_EMPTYUNDOBUFFER: u32 = 0x00cd;
const EN_CHANGE_VALUE: usize = 0x0300;
const EM_GETLINECOUNT_VALUE: u32 = 0x00ba;
const NOTICE: &str = "本機：WAVE／MPEG-1 MP3／ADTS 48 kHz；MP4：AAC-LC 44.1／48 kHz。\r\n網址：限直接 PCM16 WAVE（單／雙聲道）；不支援網頁、登入或重新導向。";
const AUTO_NOTICE: &str = "目前不支援 Auto 音訊辨識，請選擇英文、日文、法文、德文或韓文。";
const KEY_NOTICE: &str = "API 金鑰留空使用 Windows 已儲存憑證；網頁登入不適用。\r\n本次輸入不會儲存；探索、準備、取消、清除或關閉時清除輸入欄。";
const PLAYBACK_NOTICE: &str = "不會自動播放；保留後在主視窗手動啟動音訊定位。";
const NOTIFICATION_TITLE: &str = "SubtitleNow 字幕準備完成";
const NOTIFICATION_BODY: &str = "本次字幕已準備完成。請回到準備視窗查看狀態；不會自動播放。";
const NOTIFICATION_FAILURE: &str = "Windows 通知無法送出；字幕仍已準備完成。";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NotificationError {
    Unavailable,
}
trait CompletionNotification {
    fn ready(&mut self, hwnd: HWND) -> std::result::Result<(), NotificationError>;
    fn clear(&mut self);
}
#[derive(Default)]
struct ShellNotification {
    owner: Option<HWND>,
}
impl CompletionNotification for ShellNotification {
    fn ready(&mut self, hwnd: HWND) -> std::result::Result<(), NotificationError> {
        self.clear();
        let mut data = notification_data(hwnd);
        // SAFETY: initialized native payload contains only static copied text and
        // this owned HWND; the shared stock icon is not owned or destroyed here.
        unsafe {
            data.hIcon =
                LoadIconW(None, IDI_INFORMATION).map_err(|_| NotificationError::Unavailable)?;
            data.uFlags = NIF_ICON | NIF_TIP;
            if !Shell_NotifyIconW(NIM_ADD, &data).as_bool() {
                return Err(NotificationError::Unavailable);
            }
            self.owner = Some(hwnd);
            data.uFlags = NIF_INFO;
            if !Shell_NotifyIconW(NIM_MODIFY, &data).as_bool() {
                self.clear();
                return Err(NotificationError::Unavailable);
            }
        }
        Ok(())
    }
    fn clear(&mut self) {
        if let Some(hwnd) = self.owner.take() {
            let data = notification_data(hwnd);
            // SAFETY: deletes only this app-owned icon identified by HWND/uID;
            // shell owns the copied data and no borrowed buffer escapes the call.
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &data);
            }
        }
    }
}
impl Drop for ShellNotification {
    fn drop(&mut self) {
        self.clear();
    }
}
fn notification_data(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut data = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        dwInfoFlags: NIIF_INFO,
        ..Default::default()
    };
    copy_notification_text(&mut data.szTip, "SubtitleNow");
    copy_notification_text(&mut data.szInfoTitle, NOTIFICATION_TITLE);
    copy_notification_text(&mut data.szInfo, NOTIFICATION_BODY);
    data
}
fn copy_notification_text(buffer: &mut [u16], text: &str) {
    // Static guidance is shorter than all declared native buffers. Leave a NUL.
    let maximum = buffer.len().saturating_sub(1);
    for (slot, unit) in buffer.iter_mut().take(maximum).zip(text.encode_utf16()) {
        *slot = unit;
    }
}

struct Dialog {
    reference_path: Cell<HWND>,
    reference_format: Cell<HWND>,
    reference_language: Cell<HWND>,
    reference_edition: Cell<HWND>,
    attachment: RefCell<Option<crate::session::prepared::PreparedAttachment>>,
    keep_ready: Cell<HWND>,
    hwnd: Cell<HWND>,
    content: Cell<HWND>,
    scroll: Cell<i32>,
    media: Cell<HWND>,
    subtitle: Cell<HWND>,
    track: Cell<HWND>,
    tracks: Cell<Mp4SubtitleTracks>,
    format: Cell<HWND>,
    language: Cell<HWND>,
    audio_language: Cell<HWND>,
    quality: Cell<HWND>,
    provider: Cell<HWND>,
    openai_key: Cell<HWND>,
    gemini_key: Cell<HWND>,
    destination: Cell<HWND>,
    status: Cell<HWND>,
    controls: RefCell<Vec<HWND>>,
    backend: RefCell<Box<dyn PreparedBackend>>,
    notification: RefCell<Box<dyn CompletionNotification>>,
    notified: Cell<bool>,
    notification_failed: Cell<bool>,
    closing: Cell<bool>,
    failed: Cell<bool>,
    local_error: Cell<Option<PreparedUiError>>,
    discovery: RefCell<PreparedDiscovery>,
    discovery_applied: Cell<bool>,
    applying_discovery: Cell<bool>,
}
impl Dialog {
    fn new(
        backend: Box<dyn PreparedBackend>,
        notification: Box<dyn CompletionNotification>,
    ) -> Box<Self> {
        Box::new(Self {
            reference_path: Cell::default(),
            reference_format: Cell::default(),
            reference_language: Cell::default(),
            reference_edition: Cell::default(),
            attachment: RefCell::new(None),
            keep_ready: Cell::default(),
            hwnd: Cell::default(),
            content: Cell::default(),
            scroll: Cell::new(0),
            media: Cell::default(),
            subtitle: Cell::default(),
            track: Cell::default(),
            tracks: Cell::new(Mp4SubtitleTracks::default()),
            format: Cell::default(),
            language: Cell::default(),
            audio_language: Cell::default(),
            quality: Cell::default(),
            provider: Cell::default(),
            openai_key: Cell::default(),
            gemini_key: Cell::default(),
            destination: Cell::default(),
            status: Cell::default(),
            controls: RefCell::new(Vec::with_capacity(CONTROL_COUNT)),
            backend: RefCell::new(backend),
            notification: RefCell::new(notification),
            notified: Cell::new(false),
            notification_failed: Cell::new(false),
            closing: Cell::new(false),
            failed: Cell::new(false),
            local_error: Cell::new(None),
            discovery: RefCell::new(PreparedDiscovery::default()),
            discovery_applied: Cell::new(false),
            applying_discovery: Cell::new(false),
        })
    }
    fn create(&self, parent: HWND, instance: HINSTANCE, visible: bool) -> Result<()> {
        // SAFETY: caller retains this stable Box on the UI thread until native
        // destruction clears its borrowed pointer. Parent belongs to this app.
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
            let content_class = WNDCLASSW {
                lpfnWndProc: Some(content_proc),
                lpszClassName: CONTENT_CLASS,
                ..class
            };
            if RegisterClassW(&content_class) == 0
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
            let desired = RECT {
                left: owner.left + (owner.right - owner.left - width) / 2,
                top: owner.top + (owner.bottom - owner.top - height) / 2,
                right: owner.left + (owner.right - owner.left - width) / 2 + width,
                bottom: owner.top + (owner.bottom - owner.top - height) / 2 + height,
            };
            let bounds = fit_work_area(desired, work_area(parent)?);
            let hwnd = CreateWindowExW(
                WS_EX_DLGMODALFRAME | WS_EX_CONTROLPARENT,
                CLASS,
                w!("媒體字幕準備（本次）"),
                WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN,
                bounds.left,
                bounds.top,
                bounds.right - bounds.left,
                bounds.bottom - bounds.top,
                Some(parent),
                None,
                Some(instance),
                Some(self as *const Self as *const _),
            )
            .map_err(|_| AppError::SubtitleWindow)?;
            self.hwnd.set(hwnd);
            self.content.set(
                CreateWindowExW(
                    WS_EX_CONTROLPARENT,
                    CONTENT_CLASS,
                    w!(""),
                    WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN | WS_VSCROLL,
                    0,
                    0,
                    1,
                    1,
                    Some(hwnd),
                    None,
                    Some(instance),
                    Some(self as *const Self as *const _),
                )
                .map_err(|_| AppError::SubtitleWindow)?,
            );
            self.create_controls(instance)?;
            self.layout()?;
            self.refresh_destination()?;
            let state = self.backend.borrow().state();
            self.refresh_state(state)?;
            if SetTimer(Some(hwnd), POLL_TIMER, POLL_INTERVAL_MS, None) == 0 {
                return Err(AppError::SubtitleWindow);
            }
            if visible {
                let _ = ShowWindow(hwnd, SW_SHOW);
                let _ = SetFocus(Some(self.media.get()));
            }
            Ok(())
        }
    }
    unsafe fn create_controls(&self, instance: HINSTANCE) -> Result<()> {
        // SAFETY: controls belong to this UI-thread-owned dialog, native calls
        // synchronously copy the live HSTRING. Parent destroys all child handles.
        unsafe {
            let child = |class: PCWSTR, text: &str, style: WINDOW_STYLE, id: usize| {
                let parent = if [PREPARE, CANCEL, CLEAR, CLOSE, DESTINATION].contains(&id) {
                    self.hwnd.get()
                } else {
                    self.content.get()
                };
                let control = CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    class,
                    &HSTRING::from(text),
                    WS_CHILD | WS_VISIBLE | style,
                    0,
                    0,
                    1,
                    1,
                    Some(parent),
                    Some(HMENU(id as *mut _)),
                    Some(instance),
                    None,
                )
                .map_err(|_| AppError::SubtitleWindow)?;
                SendMessageW(
                    control,
                    WM_SETFONT,
                    Some(WPARAM(GetStockObject(DEFAULT_GUI_FONT).0 as usize)),
                    None,
                );
                self.controls.borrow_mut().push(control);
                Ok::<_, AppError>(control)
            };
            let label = |text| child(w!("STATIC"), text, WINDOW_STYLE::default(), 0);
            let edit = WS_TABSTOP | WS_BORDER | WINDOW_STYLE(ES_AUTOHSCROLL as u32);
            let combo = WS_TABSTOP | WS_VSCROLL | WINDOW_STYLE(CBS_DROPDOWNLIST as u32);
            label("本機完整路徑／HTTP(S) WAVE 網址")?;
            self.media.set(child(w!("EDIT"), "", edit, MEDIA)?);
            label("字幕路徑／HTTPS 網址／內嵌軌 ID")?;
            self.subtitle.set(child(w!("EDIT"), "", edit, SUBTITLE)?);
            label("字幕來源／格式（僅本機外掛留空可辨識音訊）")?;
            self.format.set(child(w!("COMBOBOX"), "", combo, FORMAT)?);
            label("字幕原文語言（請明確指定）")?;
            self.language
                .set(child(w!("COMBOBOX"), "", combo, LANGUAGE)?);
            label("翻譯品質")?;
            self.quality.set(child(w!("COMBOBOX"), "", combo, QUALITY)?);
            label("本次辨識／翻譯服務")?;
            self.provider
                .set(child(w!("COMBOBOX"), "", combo, PROVIDER)?);
            label("ChatGPT / OpenAI API 金鑰")?;
            self.openai_key.set(child(
                w!("EDIT"),
                "",
                edit | WINDOW_STYLE(ES_PASSWORD as u32),
                OPENAI_KEY,
            )?);
            label("Gemini API 金鑰")?;
            self.gemini_key.set(child(
                w!("EDIT"),
                "",
                edit | WINDOW_STYLE(ES_PASSWORD as u32),
                GEMINI_KEY,
            )?);
            label(KEY_NOTICE)?;
            self.destination.set(child(
                w!("EDIT"),
                "",
                WS_TABSTOP | WS_BORDER | WINDOW_STYLE((ES_READONLY | ES_MULTILINE) as u32),
                DESTINATION,
            )?);
            label(NOTICE)?;
            self.status.set(label("")?);
            label(PLAYBACK_NOTICE)?;
            label("音訊來源語言（字幕留空時）")?;
            self.audio_language
                .set(child(w!("COMBOBOX"), "", combo, AUDIO_LANGUAGE)?);
            child(w!("BUTTON"), "探索 MP4 字幕軌", WS_TABSTOP, DISCOVER)?;
            self.track.set(child(w!("COMBOBOX"), "", combo, TRACK)?);
            self.reset_tracks()?;
            label("同版本五語參考軌（本機，可留空）")?;
            self.reference_path
                .set(child(w!("EDIT"), "", edit, REFERENCE_PATH)?);
            label("參考軌格式")?;
            self.reference_format
                .set(child(w!("COMBOBOX"), "", combo, REFERENCE_FORMAT)?);
            label("參考軌來源語言")?;
            self.reference_language
                .set(child(w!("COMBOBOX"), "", combo, REFERENCE_LANGUAGE)?);
            label("兩軌同版本識別碼（明確指定）")?;
            self.reference_edition
                .set(child(w!("EDIT"), "", edit, REFERENCE_EDITION)?);
            self.keep_ready.set(child(
                w!("BUTTON"),
                "保留字幕並返回主視窗",
                WS_TABSTOP,
                KEEP_READY,
            )?);
            for (id, text) in [
                (PREPARE, "準備字幕"),
                (CANCEL, "取消工作"),
                (CLEAR, "清除"),
                (CLOSE, "關閉"),
            ] {
                child(w!("BUTTON"), text, WS_TABSTOP, id)?;
            }
            for (control, choices) in [
                (
                    self.format.get(),
                    &[
                        "請選擇格式",
                        "本機 SRT",
                        "本機 WebVTT",
                        "MP4 內嵌 tx3g（填軌 ID）",
                        "HTTPS SRT（填字幕網址）",
                        "HTTPS WebVTT（填字幕網址）",
                    ][..],
                ),
                (
                    self.language.get(),
                    &[
                        "請選擇語言",
                        "臺灣繁體中文",
                        "其他繁體中文",
                        "簡體中文",
                        "英文",
                        "日文",
                        "法文",
                        "德文",
                        "韓文",
                    ][..],
                ),
                (self.quality.get(), &["快速", "標準", "高品質"][..]),
                (
                    self.reference_format.get(),
                    &["未選擇", "SRT", "WebVTT"][..],
                ),
                (
                    self.reference_language.get(),
                    &["未選擇", "英文", "日文", "法文", "德文", "韓文"][..],
                ),
                (
                    self.audio_language.get(),
                    &["Auto（目前未支援）", "英文", "日文", "法文", "德文", "韓文"][..],
                ),
                (
                    self.provider.get(),
                    &["未選擇（中文本地化不需雲端）", "ChatGPT / OpenAI", "Gemini"][..],
                ),
            ] {
                for text in choices {
                    add(control, text)?;
                }
                SendMessageW(
                    control,
                    CB_SETCURSEL,
                    Some(WPARAM(usize::from(control == self.quality.get()))),
                    None,
                );
            }
            for (control, limit) in [
                (self.media.get(), PATH_LIMIT),
                (self.subtitle.get(), PATH_LIMIT),
                (self.reference_path.get(), PATH_LIMIT),
                (
                    self.reference_edition.get(),
                    crate::resolver::contracts::MAX_METADATA_BYTES,
                ),
                (self.openai_key.get(), KEY_LIMIT),
                (self.gemini_key.get(), KEY_LIMIT),
            ] {
                SendMessageW(control, EM_SETLIMITTEXT, Some(WPARAM(limit)), None);
            }
            Ok(())
        }
    }
    fn layout(&self) -> Result<()> {
        if self.controls.borrow().len() != CONTROL_COUNT {
            return Ok(());
        }
        // SAFETY: layout only repositions the dialog's own child handles according
        // to its current DPI and client width. Combo height includes drop-down rows.
        unsafe {
            let scale =
                |value| super::subtitle_window::scale(value, GetDpiForWindow(self.hwnd.get()));
            let mut client = RECT::default();
            GetClientRect(self.hwnd.get(), &mut client).map_err(|_| AppError::SubtitleWindow)?;
            let dpi = GetDpiForWindow(self.hwnd.get());
            let pad = scale(16);
            let destination_width = (client.right - 2 * pad).max(1);
            // Size without repaint first so EDIT computes real wrapped lines,
            // including long endpoint words. Final positioning follows below.
            MoveWindow(
                self.destination.get(),
                pad,
                0,
                destination_width,
                scale(DESTINATION_HEIGHT_DIP),
                false,
            )
            .map_err(|_| AppError::SubtitleWindow)?;
            let destination_height = destination_text_height(self.destination.get(), dpi)?;
            let geometry = scroll_geometry_with_destination(
                client.bottom,
                dpi,
                self.scroll.get(),
                destination_height,
            );
            self.scroll.set(geometry.position);
            MoveWindow(
                self.content.get(),
                0,
                0,
                client.right.max(1),
                geometry.viewport,
                true,
            )
            .map_err(|_| AppError::SubtitleWindow)?;
            let info = SCROLLINFO {
                cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
                nMin: 0,
                nMax: geometry.content - 1,
                nPage: geometry.viewport as u32,
                nPos: geometry.position,
                ..Default::default()
            };
            SetScrollInfo(self.content.get(), SB_VERT, &info, true);
            let mut body = RECT::default();
            GetClientRect(self.content.get(), &mut body).map_err(|_| AppError::SubtitleWindow)?;
            let width = (body.right - 2 * pad).max(1);
            let gap = scale(8);
            let half = ((width - gap) / 2).max(1);
            let key_label = scale(188).min(width / 2);
            let places = [
                (pad, 16, width, 24),
                (pad, 40, (width - scale(160) - gap).max(1), 28),
                (pad, 80, width, 24),
                (pad, 104, half, 28),
                (pad, 144, half, 24),
                (pad, 168, half, 280),
                (pad + half + gap, 144, half, 24),
                (pad + half + gap, 168, half, 280),
                (pad, 272, half, 24),
                (pad, 296, half, 180),
                (pad + half + gap, 272, half, 24),
                (pad + half + gap, 296, half, 180),
                (pad, 336, key_label, 28),
                (
                    pad + key_label + gap,
                    336,
                    (width - key_label - gap).max(1),
                    28,
                ),
                (pad, 374, key_label, 28),
                (
                    pad + key_label + gap,
                    374,
                    (width - key_label - gap).max(1),
                    28,
                ),
                (pad, 416, width, 42),
                (pad, 0, width, DESTINATION_HEIGHT_DIP),
                (pad, 464, width, 48),
                (pad, 520, width, 48),
                (pad, 580, width, 28),
                (pad, 208, width, 24),
                (pad, 232, width, 280),
                (pad + (width - scale(160)).max(0), 40, scale(160), 28),
                (pad + half + gap, 104, half, 280),
                (pad, 620, width, 24),
                (pad, 644, width, 28),
                (pad, 684, half, 24),
                (pad, 708, half, 280),
                (pad + half + gap, 684, half, 24),
                (pad + half + gap, 708, half, 280),
                (pad, 748, width, 24),
                (pad, 772, width, 28),
                (pad, 824, width, 32),
            ];
            let controls = self.controls.borrow().clone();
            for (control, (x, y, w, h)) in controls.iter().copied().zip(places) {
                if control != self.destination.get() {
                    MoveWindow(
                        control,
                        x,
                        scale(y) - geometry.position,
                        w.max(1),
                        scale(h).max(1),
                        true,
                    )
                    .map_err(|_| AppError::SubtitleWindow)?;
                }
            }
            MoveWindow(
                self.destination.get(),
                pad,
                geometry.viewport + scale(8),
                destination_width,
                destination_height,
                true,
            )
            .map_err(|_| AppError::SubtitleWindow)?;
            let button_width = ((client.right - 2 * pad - 3 * gap) / 4).max(1);
            for (index, control) in controls
                .iter()
                .copied()
                .skip(FIRST_ACTION_CONTROL)
                .enumerate()
            {
                MoveWindow(
                    control,
                    pad + index as i32 * (button_width + gap),
                    geometry.button_y,
                    button_width,
                    geometry.button_height,
                    true,
                )
                .map_err(|_| AppError::SubtitleWindow)?;
            }
            Ok(())
        }
    }
    fn scroll_to(&self, position: i32) -> Result<()> {
        self.scroll.set(position);
        self.layout()
    }
    fn scroll_command(&self, command: usize) -> Result<()> {
        let mut info = SCROLLINFO {
            cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
            fMask: SIF_ALL,
            ..Default::default()
        };
        // SAFETY: read only this viewport's native scrollbar, writing into an
        // initialized callback-local SCROLLINFO. Native position is bounded.
        unsafe {
            GetScrollInfo(self.content.get(), SB_VERT, &mut info)
                .map_err(|_| AppError::SubtitleWindow)?;
        }
        let line = super::subtitle_window::scale(SCROLL_LINE_DIP, unsafe {
            GetDpiForWindow(self.hwnd.get())
        });
        let position = match SCROLLBAR_COMMAND(command as i32) {
            SB_LINEUP => info.nPos - line,
            SB_LINEDOWN => info.nPos + line,
            SB_PAGEUP => info.nPos - info.nPage as i32,
            SB_PAGEDOWN => info.nPos + info.nPage as i32,
            SB_TOP => 0,
            SB_BOTTOM => info.nMax,
            SB_THUMBPOSITION | SB_THUMBTRACK => info.nTrackPos,
            _ => return Ok(()),
        };
        self.scroll_to(position)
    }
    fn reveal_focus(&self) -> Result<()> {
        if self.hwnd.get().0.is_null() {
            return Ok(());
        }
        // SAFETY: inspect only whether current UI-thread focus is our viewport's
        // child. Rectangle buffers remain valid for synchronous native reads.
        unsafe {
            let focus = GetFocus();
            if !IsChild(self.content.get(), focus).as_bool() {
                return Ok(());
            }
            let mut rect = RECT::default();
            GetWindowRect(focus, &mut rect).map_err(|_| AppError::SubtitleWindow)?;
            let mut points = [
                windows::Win32::Foundation::POINT {
                    x: rect.left,
                    y: rect.top,
                },
                windows::Win32::Foundation::POINT {
                    x: rect.right,
                    y: rect.bottom,
                },
            ];
            MapWindowPoints(None, Some(self.content.get()), &mut points);
            let mut viewport = RECT::default();
            GetClientRect(self.content.get(), &mut viewport)
                .map_err(|_| AppError::SubtitleWindow)?;
            if points[0].y < 0 {
                self.scroll_to(self.scroll.get() + points[0].y)?;
            } else if points[1].y > viewport.bottom {
                self.scroll_to(self.scroll.get() + points[1].y - viewport.bottom)?;
            }
        }
        Ok(())
    }
    fn selected_provider(&self) -> Option<SpeechProvider> {
        match selected(self.provider.get()) {
            1 => Some(SpeechProvider::OpenAi),
            2 => Some(SpeechProvider::Gemini),
            _ => None,
        }
    }
    fn has_subtitle(&self) -> bool {
        if selected(self.format.get()) == EMBEDDED_FORMAT_INDEX || self.has_https_subtitle_mode() {
            return true; // Missing ID/URL must not enable audio disclosure/upload.
        }
        // SAFETY: length inspection of our same-thread EDIT is enough to switch
        // form state; private path text is not copied or inspected here.
        unsafe { GetWindowTextLengthW(self.subtitle.get()) > 0 }
    }
    fn has_https_subtitle_mode(&self) -> bool {
        matches!(
            selected(self.format.get()),
            HTTPS_SRT_FORMAT_INDEX | HTTPS_WEBVTT_FORMAT_INDEX
        )
    }
    fn has_remote_media(&self) -> bool {
        read_optional_edit(self.media.get(), PATH_LIMIT)
            .is_ok_and(|text| is_remote_media_text(&zeroize::Zeroizing::new(text)))
    }
    fn selected_audio_language(&self) -> SourceLanguage {
        match selected(self.audio_language.get()) {
            1 => SourceLanguage::English,
            2 => SourceLanguage::Japanese,
            3 => SourceLanguage::French,
            4 => SourceLanguage::German,
            5 => SourceLanguage::Korean,
            _ => SourceLanguage::Auto,
        }
    }
    fn read_input(
        &self,
    ) -> std::result::Result<
        (
            crate::media::planning::PreparedRequest,
            PreparedOptions,
            bool,
        ),
        PreparedUiError,
    > {
        let mut media = zeroize::Zeroizing::new(read_edit(self.media.get(), PATH_LIMIT)?);
        let asr = !self.has_subtitle();
        if is_remote_media_text(&media) && selected(self.format.get()) == EMBEDDED_FORMAT_INDEX {
            return Err(MediaError::UnsupportedUrl.into());
        }
        // Sidecar metadata is independent of the audio language. These unused
        // defaults never manufacture a subtitle when its path is truly empty.
        let format = if asr {
            SubtitleFormat::Srt
        } else {
            match selected(self.format.get()) {
                1 | EMBEDDED_FORMAT_INDEX | HTTPS_SRT_FORMAT_INDEX => SubtitleFormat::Srt,
                2 | HTTPS_WEBVTT_FORMAT_INDEX => SubtitleFormat::WebVtt,
                _ => return Err(MediaError::InvalidSubtitle.into()),
            }
        };
        let language = if asr {
            TextLanguage::TaiwanTraditional
        } else {
            match selected(self.language.get()) {
                1 => TextLanguage::TaiwanTraditional,
                2 => TextLanguage::OtherTraditional,
                3 => TextLanguage::SimplifiedChinese,
                4 => TextLanguage::Source(SpeechLanguage::English),
                5 => TextLanguage::Source(SpeechLanguage::Japanese),
                6 => TextLanguage::Source(SpeechLanguage::French),
                7 => TextLanguage::Source(SpeechLanguage::German),
                8 => TextLanguage::Source(SpeechLanguage::Korean),
                _ => return Err(MediaError::InvalidSubtitle.into()),
            }
        };
        let quality = match selected(self.quality.get()) {
            0 => TranslationQuality::Fast,
            1 => TranslationQuality::Standard,
            2 => TranslationQuality::High,
            _ => return Err(MediaError::InvalidInput.into()),
        };
        let source = asr || matches!(language, TextLanguage::Source(_));
        let options = PreparedOptions {
            provider: self.selected_provider(),
            quality,
            source_language: self.selected_audio_language(),
        };
        let draft = PreparedInputDraft {
            media: std::mem::take(&mut *media),
            subtitle: read_optional_edit(self.subtitle.get(), PATH_LIMIT)?,
            format,
            language,
        };
        let request = if self.has_https_subtitle_mode() {
            draft.into_https_subtitle_request()?
        } else if selected(self.format.get()) == EMBEDDED_FORMAT_INDEX {
            draft.into_embedded_request()?
        } else {
            draft.into_request()?
        };
        let reference = crate::ui::prepared::PreparedReferenceDraft {
            path: read_optional_edit(self.reference_path.get(), PATH_LIMIT)?,
            edition: read_optional_edit(
                self.reference_edition.get(),
                crate::resolver::contracts::MAX_METADATA_BYTES,
            )?,
            format: match selected(self.reference_format.get()) {
                1 => Some(SubtitleFormat::Srt),
                2 => Some(SubtitleFormat::WebVtt),
                _ => None,
            },
            language: match selected(self.reference_language.get()) {
                1 => Some(SpeechLanguage::English),
                2 => Some(SpeechLanguage::Japanese),
                3 => Some(SpeechLanguage::French),
                4 => Some(SpeechLanguage::German),
                5 => Some(SpeechLanguage::Korean),
                _ => None,
            },
        };
        let request = reference.apply(request)?;
        if asr && options.source_language == SourceLanguage::Auto {
            return Err(PreparedSpeechError::UnsupportedLanguage.into());
        }
        if source && options.provider.is_none() {
            return Err(PreparedUiError::ProviderRequired);
        }
        Ok((request, options, source))
    }
    fn discover(&self) -> Result<()> {
        let state = self.backend.borrow().state();
        if busy(state)
            || matches!(state, PreparedUiState::Ready { .. })
            || self.discovery.borrow().state().busy()
            || self.closing.get()
        {
            return Ok(());
        }
        self.clear_keys()?;
        if self.has_https_subtitle_mode() {
            return self.show_error(MediaError::UnsupportedUrl.into());
        }
        let input = read_edit(self.media.get(), PATH_LIMIT).and_then(|path| {
            if is_remote_media_text(&path) {
                Err(MediaError::UnsupportedUrl)
            } else {
                crate::media::input::MediaInput::local(path.into())
            }
        });
        let input = match input {
            Ok(input) => input,
            Err(error) => return self.show_error(error.into()),
        };
        self.media_changed()?; // Retire a previous scan/auto-filled ID before rescanning.
        self.discovery_applied.set(false);
        self.local_error.set(None);
        self.reset_notification();
        let started = self.discovery.borrow_mut().start(input);
        if let Err(error) = started {
            return self.show_error(error.into());
        }
        self.refresh_destination()?;
        self.refresh_state(state)
    }
    fn begin(&self) -> Result<()> {
        let state = self.backend.borrow().state();
        if busy(state)
            || self.discovery.borrow().state().busy()
            || matches!(state, PreparedUiState::Ready { .. })
            || self.closing.get()
        {
            return Ok(());
        }
        let (request, options, source) = match self.read_input() {
            Ok(value) => value,
            Err(error) => {
                self.clear_keys()?;
                return self.show_error(error);
            }
        };
        let key = if source {
            let control = match options.provider {
                Some(SpeechProvider::OpenAi) => self.openai_key.get(),
                Some(SpeechProvider::Gemini) => self.gemini_key.get(),
                None => return self.show_error(PreparedUiError::ProviderRequired),
            };
            read_password(control)
        } else {
            Ok(None)
        };
        self.clear_keys()?;
        let key = match key {
            Ok(key) => key,
            Err(error) => return self.show_error(error.into()),
        };
        self.reset_notification();
        self.discovery.borrow_mut().stop();
        self.local_error.set(None);
        let started = if self.has_https_subtitle_mode() {
            self.backend
                .borrow_mut()
                .start_with_https_subtitles(request, options, key)
        } else {
            self.backend.borrow_mut().start(request, options, key)
        };
        if let Err(error) = started {
            return self.show_error(error);
        }
        let state = self.backend.borrow().state();
        self.refresh_state(state)
    }
    fn show_error(&self, error: PreparedUiError) -> Result<()> {
        self.local_error.set(Some(error));
        let audio_auto =
            !self.has_subtitle() && self.selected_audio_language() == SourceLanguage::Auto;
        set_text(self.status.get(), &error_label(error, audio_auto))
    }
    fn refresh_destination(&self) -> Result<()> {
        if self.destination.get().0.is_null() {
            return Ok(());
        }
        if self.discovery.borrow().state().busy() {
            set_text(
                self.destination.get(),
                "字幕軌探索：僅在本機讀取所選 MP4 的結構，不使用金鑰或雲端；不辨識媒體音訊。",
            )?;
            return self.layout();
        }
        let asr = !self.has_subtitle();
        let chinese = !asr && matches!(selected(self.language.get()), 1..=3);
        let remote = self.has_remote_media();
        if self.has_https_subtitle_mode() {
            return self.refresh_https_destination(remote, chinese);
        }
        let text = if remote && selected(self.format.get()) == EMBEDDED_FORMAT_INDEX {
            "此網址目前不支援內嵌字幕；請保留原選擇並改用本機 MP4，或明確選擇本機 SRT／WebVTT 字幕。按準備不會連線或傳送音訊。"
        } else if remote && chinese {
            "媒體來源：按準備後連線至輸入的 HTTP(S) WAVE 網址，僅讀取媒體標頭。\r\n所選本機中文字幕在本機進行台灣中文本地化；不使用 AI 或金鑰，不向 AI 傳送媒體音訊或來源網址。"
        } else if remote && !asr {
            match self.selected_provider() {
                Some(SpeechProvider::OpenAi) => {
                    "媒體來源：按準備後連線至輸入的 HTTP(S) WAVE 網址，僅讀取媒體標頭。\r\nAI 目的地：https://api.openai.com\r\n僅將所選本機原文字幕、全文脈絡摘要、專名／詞彙及翻譯前後文送至同一個所選服務與金鑰；不向 AI 傳送媒體音訊或來源網址。"
                }
                Some(SpeechProvider::Gemini) => {
                    "媒體來源：按準備後連線至輸入的 HTTP(S) WAVE 網址，僅讀取媒體標頭。\r\nAI 目的地：https://generativelanguage.googleapis.com\r\n僅將所選本機原文字幕、全文脈絡摘要、專名／詞彙及翻譯前後文送至同一個所選服務與金鑰；不向 AI 傳送媒體音訊或來源網址。"
                }
                None => {
                    "媒體來源：按準備後連線至輸入的 HTTP(S) WAVE 網址，僅讀取媒體標頭；AI 目的地尚未選擇。\r\n所選本機原文字幕須明確選擇本次翻譯服務；僅傳送原文字幕、全文脈絡摘要、專名／詞彙及翻譯前後文，不向 AI 傳送媒體音訊或來源網址。"
                }
            }
        } else if remote {
            match self.selected_provider() {
                Some(SpeechProvider::OpenAi) => {
                    "媒體來源：按準備後連線至輸入的 HTTP(S) WAVE 網址。\r\nAI 目的地：https://api.openai.com\r\n僅將短片段媒體音訊、辨識原文、全文脈絡摘要、專名／詞彙及翻譯前後文送至同一個所選服務與金鑰；音訊不存檔。"
                }
                Some(SpeechProvider::Gemini) => {
                    "媒體來源：按準備後連線至輸入的 HTTP(S) WAVE 網址。\r\nAI 目的地：https://generativelanguage.googleapis.com\r\n僅將短片段媒體音訊、辨識原文、全文脈絡摘要、專名／詞彙及翻譯前後文送至同一個所選服務與金鑰；音訊不存檔。"
                }
                None => {
                    "媒體來源：按準備後連線至輸入的 HTTP(S) WAVE 網址；AI 目的地尚未選擇。\r\n須選擇音訊來源語言與本次服務；僅向所選服務傳送短片段媒體音訊、辨識原文、全文脈絡摘要、專名／詞彙及翻譯前後文。"
                }
            }
        } else if chinese {
            "傳送目的地：不使用雲端；中文本地化在本機處理。"
        } else if asr {
            match self.selected_provider() {
                Some(SpeechProvider::OpenAi) => {
                    "傳送目的地：https://api.openai.com\r\n按準備後將短片段媒體音訊送至此服務辨識，再傳送辨識原文與翻譯前後文，整理專名／詞彙與全文脈絡摘要，再進行翻譯。\r\n使用同一個所選服務與金鑰；音訊不存檔。"
                }
                Some(SpeechProvider::Gemini) => {
                    "傳送目的地：https://generativelanguage.googleapis.com\r\n按準備後將短片段媒體音訊送至此服務辨識，再傳送辨識原文與翻譯前後文，整理專名／詞彙與全文脈絡摘要，再進行翻譯。\r\n使用同一個所選服務與金鑰；音訊不存檔。"
                }
                None => {
                    "傳送目的地：尚未選擇。\r\n字幕留空時，須選擇音訊來源語言與本次辨識／翻譯服務。\r\n按準備會傳送短片段媒體音訊、辨識原文及翻譯前後文，整理專名／詞彙與全文脈絡摘要，再進行翻譯。"
                }
            }
        } else {
            match self.selected_provider() {
                Some(SpeechProvider::OpenAi) => {
                    "傳送目的地：https://api.openai.com\r\n按準備後傳送所選原文字幕及翻譯前後文，整理專名／詞彙與全文脈絡摘要，再進行翻譯；不傳送媒體音訊。"
                }
                Some(SpeechProvider::Gemini) => {
                    "傳送目的地：https://generativelanguage.googleapis.com\r\n按準備後傳送所選原文字幕及翻譯前後文，整理專名／詞彙與全文脈絡摘要，再進行翻譯；不傳送媒體音訊。"
                }
                None => "傳送目的地：尚未選擇；來源字幕須明確選擇本次翻譯服務。",
            }
        };
        set_text(self.destination.get(), text)?;
        self.layout()
    }
    fn refresh_https_destination(&self, remote_media: bool, chinese: bool) -> Result<()> {
        // All components are static. Private source locations and signed query
        // values stay only in their owned EDITs; disclosure changes perform no I/O.
        let media = if remote_media {
            "先讀遠端WAVE標頭。"
        } else {
            "先唯讀持有本機媒體。"
        };
        let processing = if chinese {
            "中文本機轉台灣用語，不使用 AI 或金鑰；僅本次記憶體，不存檔或播放。"
        } else {
            match self.selected_provider() {
                Some(SpeechProvider::OpenAi) => {
                    "AI：https://api.openai.com\r\n同一所選服務與金鑰：僅傳原文、全文摘要、專名／詞彙、翻譯前後文；不傳音訊或任何來源網址。僅本次記憶體，不存檔或播放。"
                }
                Some(SpeechProvider::Gemini) => {
                    "AI：https://generativelanguage.googleapis.com\r\n同一所選服務與金鑰：僅傳原文、全文摘要、專名／詞彙、翻譯前後文；不傳音訊或任何來源網址。僅本次記憶體，不存檔或播放。"
                }
                None => {
                    "AI 尚未選擇，請選翻譯服務。\r\n僅傳原文、全文摘要、專名／詞彙、翻譯前後文；不傳音訊或任何來源網址。僅本次記憶體，不存檔或播放。"
                }
            }
        };
        set_text(
            self.destination.get(),
            &format!(
                "準備時：連線指定 HTTPS 字幕讀取全文；不探索內嵌軌。\r\n{media}\r\n{processing}"
            ),
        )?;
        self.layout()
    }
    fn fields_changed(&self) -> Result<()> {
        if self.controls.borrow().len() != CONTROL_COUNT
            || self.hwnd.get().0.is_null()
            || self.applying_discovery.get()
        {
            return Ok(());
        }
        let state = self.backend.borrow().state();
        if busy(state)
            || self.discovery.borrow().state().busy()
            || matches!(state, PreparedUiState::Ready { .. })
            || self.closing.get()
        {
            return Ok(());
        }
        self.discovery.borrow_mut().stop(); // Edits invalidate displayed discovery metadata.
        self.local_error.set(None);
        self.refresh_destination()?;
        self.refresh_state(state)
    }
    fn media_changed(&self) -> Result<()> {
        if self.controls.borrow().len() != CONTROL_COUNT || self.applying_discovery.get() {
            return Ok(());
        }
        // An external/programmatic edit must retire even a pending scan. Real
        // user edits are disabled while busy. Never reuse its ID for a new path.
        self.discovery.borrow_mut().stop();
        self.reset_tracks()?;
        if self.discovery_applied.replace(false)
            && selected(self.format.get()) == EMBEDDED_FORMAT_INDEX
        {
            clear_edit(self.subtitle.get())?;
            // SAFETY: reset only our owned fixed language combo on its UI thread.
            unsafe {
                SendMessageW(self.language.get(), CB_SETCURSEL, Some(WPARAM(0)), None);
            }
        }
        self.fields_changed()
    }
    fn reset_tracks(&self) -> Result<()> {
        self.tracks.set(Mp4SubtitleTracks::default());
        if self.track.get().0.is_null() {
            return Ok(());
        }
        // SAFETY: reset our owned combo on its UI thread; add copies its static
        // label synchronously. No file content, pointers or item-data ownership.
        unsafe {
            SendMessageW(self.track.get(), CB_RESETCONTENT, None, None);
            add(self.track.get(), "探索後請選字幕軌")?;
            SendMessageW(self.track.get(), CB_SETCURSEL, Some(WPARAM(0)), None);
        }
        Ok(())
    }
    fn apply_track(&self, info: Option<Mp4SubtitleTrack>) -> Result<()> {
        use crate::media::embedded_mp4::Mp4SubtitleLanguage;
        let language = match info.map(|track| track.language) {
            Some(Mp4SubtitleLanguage::Source(SpeechLanguage::English)) => 4,
            Some(Mp4SubtitleLanguage::Source(SpeechLanguage::Japanese)) => 5,
            Some(Mp4SubtitleLanguage::Source(SpeechLanguage::French)) => 6,
            Some(Mp4SubtitleLanguage::Source(SpeechLanguage::German)) => 7,
            Some(Mp4SubtitleLanguage::Source(SpeechLanguage::Korean)) => 8,
            _ => 0,
        };
        // SAFETY: fixed scalar indices for owned same-thread combos.
        unsafe {
            SendMessageW(
                self.format.get(),
                CB_SETCURSEL,
                Some(WPARAM(EMBEDDED_FORMAT_INDEX as usize)),
                None,
            );
            SendMessageW(
                self.language.get(),
                CB_SETCURSEL,
                Some(WPARAM(language)),
                None,
            );
        }
        self.applying_discovery.set(true);
        let result = set_text(
            self.subtitle.get(),
            &info.map_or_else(String::new, |track| track.track_id.to_string()),
        );
        self.applying_discovery.set(false);
        result
    }
    fn track_changed(&self) -> Result<()> {
        let state = self.backend.borrow().state();
        if self.tracks.get().is_empty()
            || selected(self.format.get()) != EMBEDDED_FORMAT_INDEX
            || busy(state)
            || matches!(state, PreparedUiState::Ready { .. })
            || self.discovery.borrow().state().busy()
            || self.closing.get()
        {
            return Ok(());
        }
        let index = selected(self.track.get());
        let info = usize::try_from(index)
            .ok()
            .and_then(|n| n.checked_sub(1))
            .and_then(|n| self.tracks.get().get(n));
        self.apply_track(info)?;
        self.fields_changed()
    }
    fn subtitle_changed(&self) -> Result<()> {
        if !self.applying_discovery.get() && !self.tracks.get().is_empty() {
            // SAFETY: manual ID/path edits retire only the owned combo's scalar
            // selection; metadata stays bound to this unchanged source path.
            unsafe {
                SendMessageW(self.track.get(), CB_SETCURSEL, Some(WPARAM(0)), None);
            }
        }
        self.fields_changed()
    }
    fn refresh_state(&self, state: PreparedUiState) -> Result<()> {
        if self.hwnd.get().0.is_null() {
            return Ok(());
        }
        let discovery = self.discovery.borrow().state();
        // SAFETY: owned same-thread control. Only a complete Ready may hand off.
        unsafe {
            let _ = EnableWindow(
                self.keep_ready.get(),
                matches!(state, PreparedUiState::Ready { .. })
                    && !self.closing.get()
                    && !discovery.busy(),
            );
        }
        let active = busy(state) || discovery.busy();
        let locked = active || matches!(state, PreparedUiState::Ready { .. });
        let available = !locked && !self.closing.get();
        let sidecar = self.has_subtitle();
        let chinese = sidecar && matches!(selected(self.language.get()), 1..=3);
        let remote = self.has_remote_media();
        let controls = self.controls.borrow().clone();
        // SAFETY: scalar enabled states affect only our own controls; active work
        // cannot mutate source, provider, quality, or native password fields.
        unsafe {
            for control in [
                self.media.get(),
                self.reference_path.get(),
                self.reference_format.get(),
                self.reference_language.get(),
                self.reference_edition.get(),
                self.subtitle.get(),
                self.quality.get(),
                controls[FIRST_ACTION_CONTROL],
            ] {
                let _ = EnableWindow(control, available);
            }
            let _ = EnableWindow(
                controls[DISCOVER_CONTROL_INDEX],
                available && !remote && !self.has_https_subtitle_mode(),
            );
            let _ = EnableWindow(self.format.get(), available);
            let _ = EnableWindow(
                self.track.get(),
                available
                    && !self.tracks.get().is_empty()
                    && selected(self.format.get()) == EMBEDDED_FORMAT_INDEX,
            );
            let _ = EnableWindow(self.language.get(), available && sidecar);
            let _ = EnableWindow(self.audio_language.get(), available && !sidecar);
            for control in [
                self.provider.get(),
                self.openai_key.get(),
                self.gemini_key.get(),
            ] {
                let _ = EnableWindow(control, available && !chinese);
            }
            let _ = EnableWindow(
                controls[FIRST_ACTION_CONTROL + 2],
                !active && !self.closing.get(),
            );
            let _ = EnableWindow(
                controls[FIRST_ACTION_CONTROL + 1],
                active
                    && !self.closing.get()
                    && state != PreparedUiState::Stopping
                    && discovery != DiscoveryState::Stopping,
            );
        }
        let mut label = self.local_error.get().map_or_else(
            || {
                if discovery == DiscoveryState::Idle {
                    state.label()
                } else {
                    discovery.label()
                }
            },
            |error| {
                let audio_auto =
                    !self.has_subtitle() && self.selected_audio_language() == SourceLanguage::Auto;
                error_label(error, audio_auto)
            },
        );
        if self.notification_failed.get() && matches!(state, PreparedUiState::Ready { .. }) {
            label.push_str("\r\n");
            label.push_str(NOTIFICATION_FAILURE);
        }
        set_text(self.status.get(), &label)
    }
    fn poll(&self) -> Result<()> {
        let previous_discovery = self.discovery.borrow().state();
        let discovery = self.discovery.borrow_mut().poll();
        let state = self.backend.borrow_mut().poll();
        if self.hwnd.get().0.is_null() {
            return Ok(());
        }
        if let DiscoveryState::Found(info) = discovery
            && !self.discovery_applied.replace(true)
            && !self.closing.get()
        {
            self.reset_tracks()?;
            self.tracks.set(info);
            // SAFETY: bounded metadata labels are synchronously copied into our
            // owned same-thread combo; no provider/file content or item pointers.
            unsafe {
                for index in 0..info.len() {
                    if let Some(track) = info.get(index) {
                        let hint = match track.language {
                            crate::media::embedded_mp4::Mp4SubtitleLanguage::Chinese => {
                                "中文（請指定繁簡）"
                            }
                            crate::media::embedded_mp4::Mp4SubtitleLanguage::Undetermined => {
                                "語言未定"
                            }
                            crate::media::embedded_mp4::Mp4SubtitleLanguage::Source(language) => {
                                match language {
                                    SpeechLanguage::English => "英文",
                                    SpeechLanguage::Japanese => "日文",
                                    SpeechLanguage::French => "法文",
                                    SpeechLanguage::German => "德文",
                                    SpeechLanguage::Korean => "韓文",
                                }
                            }
                        };
                        add(self.track.get(), &format!("軌 {} · {hint}", track.track_id))?;
                    }
                }
                SendMessageW(
                    self.track.get(),
                    CB_SETCURSEL,
                    Some(WPARAM(usize::from(info.len() == 1))),
                    None,
                );
            }
            self.apply_track(if info.len() == 1 { info.get(0) } else { None })?;
            self.refresh_destination()?;
        }
        if previous_discovery.busy() && !discovery.busy() {
            self.refresh_destination()?;
        }
        if matches!(state, PreparedUiState::Ready { .. }) && !self.notified.replace(true) {
            self.notification_failed.set(
                self.notification
                    .borrow_mut()
                    .ready(self.hwnd.get())
                    .is_err(),
            );
        }
        self.refresh_state(state)?;
        if self.closing.get() && !busy(state) && !discovery.busy() {
            self.finish()?;
        }
        Ok(())
    }
    fn reset_notification(&self) {
        self.notification.borrow_mut().clear();
        self.notified.set(false);
        self.notification_failed.set(false);
    }
    fn stop(&self) -> Result<()> {
        self.local_error.set(None);
        let cleared = if !self.hwnd.get().0.is_null() {
            self.clear_keys()
        } else {
            Ok(())
        };
        self.reset_notification();
        self.discovery.borrow_mut().stop();
        self.backend.borrow_mut().stop();
        let tracks_cleared = self.reset_tracks();
        let state = self.backend.borrow().state();
        let refreshed = self.refresh_state(state);
        cleared.and(tracks_cleared).and(refreshed)
    }
    fn clear(&self) -> Result<()> {
        if busy(self.backend.borrow().state()) || self.discovery.borrow().state().busy() {
            return Ok(());
        }
        self.stop()?;
        self.clear_fields()?;
        // SAFETY: reset fixed combos on this owned dialog only. Never infer a
        // subtitle language from the filename or previously processed content.
        unsafe {
            for (control, index) in [
                (self.format.get(), 0),
                (self.language.get(), 0),
                (self.audio_language.get(), 0),
                (self.provider.get(), 0),
                (self.quality.get(), 1),
                (self.reference_format.get(), 0),
                (self.reference_language.get(), 0),
            ] {
                SendMessageW(control, CB_SETCURSEL, Some(WPARAM(index)), None);
            }
        }
        self.fields_changed()
    }
    fn close(&self) -> Result<()> {
        self.closing.set(true);
        self.stop()?;
        let state = self.backend.borrow().state();
        if !busy(state) && !self.discovery.borrow().state().busy() {
            self.finish()?;
        }
        Ok(())
    }
    fn keep_ready(&self) -> Result<()> {
        if !matches!(self.backend.borrow().state(), PreparedUiState::Ready { .. })
            || self.discovery.borrow().state().busy()
        {
            return Ok(());
        }
        let result = self.backend.borrow_mut().take_ready();
        let attachment = match result {
            Ok(attachment) => attachment,
            Err(error) => return self.show_error(error),
        };
        *self.attachment.borrow_mut() = Some(attachment);
        self.close()
    }
    fn clear_keys(&self) -> Result<()> {
        let first = clear_edit(self.openai_key.get());
        let second = clear_edit(self.gemini_key.get());
        first.and(second)
    }
    fn clear_fields(&self) -> Result<()> {
        let keys = self.clear_keys();
        let media = clear_edit(self.media.get());
        let subtitle = clear_edit(self.subtitle.get());
        let reference = clear_edit(self.reference_path.get());
        let edition = clear_edit(self.reference_edition.get());
        keys.and(media).and(subtitle).and(reference).and(edition)
    }
    fn finish(&self) -> Result<()> {
        if self.hwnd.get().0.is_null() {
            return Ok(());
        }
        self.reset_notification();
        let cleared = self.clear_fields();
        // SAFETY: synchronous destruction of this dialog and its owned children;
        // WM_NCDESTROY removes the borrowed Box pointer before caller drops it.
        let destroyed =
            unsafe { DestroyWindow(self.hwnd.get()) }.map_err(|_| AppError::SubtitleWindow);
        cleared.and(destroyed)
    }
}
impl Drop for Dialog {
    fn drop(&mut self) {
        self.backend.get_mut().stop();
        self.notification.get_mut().clear();
        let _ = self.finish();
    }
}
fn busy(state: PreparedUiState) -> bool {
    matches!(
        state,
        PreparedUiState::Preparing | PreparedUiState::Stopping
    )
}
fn error_label(error: PreparedUiError, audio_auto: bool) -> String {
    if audio_auto && error == PreparedUiError::Speech(PreparedSpeechError::UnsupportedLanguage) {
        format!("媒體準備：{AUTO_NOTICE}")
    } else {
        format!("媒體準備：{error}")
    }
}

struct ScrollGeometry {
    content: i32,
    viewport: i32,
    position: i32,
    button_y: i32,
    button_height: i32,
}
#[cfg(test)]
fn scroll_geometry(client_height: i32, dpi: u32, position: i32) -> ScrollGeometry {
    scroll_geometry_with_destination(
        client_height,
        dpi,
        position,
        super::subtitle_window::scale(DESTINATION_HEIGHT_DIP, dpi),
    )
}
fn scroll_geometry_with_destination(
    client_height: i32,
    dpi: u32,
    position: i32,
    destination_height: i32,
) -> ScrollGeometry {
    let scale = |value| super::subtitle_window::scale(value, dpi);
    let content = scale(CONTENT_HEIGHT_DIP).max(1);
    let band = scale(BUTTON_BAND_DIP - DESTINATION_HEIGHT_DIP) + destination_height;
    let viewport = (client_height - band).max(1);
    let button_height = scale(32).min((client_height - 2).max(1));
    let button_y = (client_height - scale(16) - button_height).max(0);
    ScrollGeometry {
        content,
        viewport,
        position: position.clamp(0, (content - viewport).max(0)),
        button_y,
        button_height,
    }
}
fn destination_text_height(control: HWND, dpi: u32) -> Result<i32> {
    struct Canvas {
        control: HWND,
        dc: HDC,
        old: HGDIOBJ,
    }
    impl Drop for Canvas {
        fn drop(&mut self) {
            // SAFETY: restore this borrowed control font before returning its
            // same-thread window DC. Neither the font nor HWND is owned here.
            unsafe {
                SelectObject(self.dc, self.old);
                ReleaseDC(Some(self.control), self.dc);
            }
        }
    }
    // SAFETY: the readonly destination EDIT has already received its proposed
    // width. Read its own line count and actual font only; no private content.
    unsafe {
        let lines = SendMessageW(control, EM_GETLINECOUNT_VALUE, None, None).0;
        if !(1..=1024).contains(&lines) {
            return Err(AppError::SubtitleWindow);
        }
        let dc = GetDC(Some(control));
        if dc.0.is_null() {
            return Err(AppError::SubtitleWindow);
        }
        let font = SendMessageW(control, WM_GETFONT, None, None).0;
        let old = SelectObject(dc, HGDIOBJ(font as *mut _));
        if old.0.is_null() || old.0 as isize == -1 {
            ReleaseDC(Some(control), dc);
            return Err(AppError::SubtitleWindow);
        }
        let _canvas = Canvas { control, dc, old };
        let mut metrics = TEXTMETRICW::default();
        GetTextMetricsW(dc, &mut metrics)
            .ok()
            .map_err(|_| AppError::SubtitleWindow)?;
        let height = i32::try_from(lines)
            .ok()
            .and_then(|lines| {
                lines.checked_mul((metrics.tmHeight + metrics.tmExternalLeading).max(1))
            })
            .and_then(|height| height.checked_add(super::subtitle_window::scale(8, dpi)))
            .ok_or(AppError::SubtitleWindow)?;
        Ok(height.max(super::subtitle_window::scale(DESTINATION_HEIGHT_DIP, dpi)))
    }
}
fn fit_work_area(desired: RECT, work: RECT) -> RECT {
    let width = (desired.right - desired.left).clamp(1, (work.right - work.left).max(1));
    let height = (desired.bottom - desired.top).clamp(1, (work.bottom - work.top).max(1));
    let left = desired.left.clamp(work.left, work.right - width);
    let top = desired.top.clamp(work.top, work.bottom - height);
    RECT {
        left,
        top,
        right: left + width,
        bottom: top + height,
    }
}
fn work_area(hwnd: HWND) -> Result<RECT> {
    let mut monitor = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: app-owned owner selects its nearest monitor; the initialized native
    // buffer is writable and no monitor handle is retained or owned by Rust.
    if unsafe {
        GetMonitorInfoW(
            MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST),
            &mut monitor,
        )
    }
    .as_bool()
    {
        Ok(monitor.rcWork)
    } else {
        Err(AppError::SubtitleWindow)
    }
}

pub(super) fn show(
    parent: HWND,
    instance: HINSTANCE,
) -> Result<Option<crate::session::prepared::PreparedAttachment>> {
    let backend = crate::app::prepared::PreparedController::new();
    let dialog = Dialog::new(Box::new(backend), Box::<ShellNotification>::default());
    dialog.create(parent, instance, true)?;
    // SAFETY: only the calling app's UI-thread-owned modal parent is disabled,
    // restored by RAII; parent lifecycle messages continue through this pump.
    let enabled = unsafe { IsWindowEnabled(parent) }.as_bool();
    struct OwnerEnabled(HWND, bool);
    impl Drop for OwnerEnabled {
        fn drop(&mut self) {
            if self.1 {
                unsafe {
                    let _ = EnableWindow(self.0, true);
                }
            }
        }
    }
    let _owner = OwnerEnabled(parent, enabled);
    // SAFETY: own modal parent, with enabled state restored by the guard above.
    unsafe {
        let _ = EnableWindow(parent, false);
    }
    run_modal(&dialog)?;
    let attachment = dialog.attachment.borrow_mut().take();
    Ok(attachment)
}
fn run_modal(dialog: &Dialog) -> Result<()> {
    let mut quit = None;
    while !dialog.hwnd.get().0.is_null() || busy(dialog.backend.borrow().state()) {
        let mut message = MSG::default();
        // Parent destruction removes native timers too. Retain the Box and poll
        // the retiring worker with a short message wait until it is fully reaped.
        // SAFETY: this thread's queue is pumped with an initialized writable MSG;
        // no external HWND or event handle is waited on or altered here.
        unsafe {
            let _ = MsgWaitForMultipleObjects(None, false, POLL_INTERVAL_MS, QS_ALLINPUT);
        }
        // SAFETY: same thread message queue, buffer stays alive for native writes.
        while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
            if message.message == WM_QUIT {
                quit = Some(message.wParam.0 as i32);
                if dialog.close().is_err() {
                    dialog.failed.set(true);
                }
                continue;
            }
            // SAFETY: normal app UI messages continue, including parent lifecycle;
            // use dialog navigation only while its owned HWND still exists.
            unsafe {
                if dialog.hwnd.get().0.is_null()
                    || !IsDialogMessageW(dialog.hwnd.get(), &message).as_bool()
                {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            if dialog.reveal_focus().is_err() {
                dialog.failed.set(true);
                let _ = dialog.close();
            }
        }
        if dialog.poll().is_err() {
            dialog.failed.set(true);
            let _ = dialog.close();
        }
    }
    if let Some(code) = quit {
        unsafe {
            PostQuitMessage(code);
        }
    }
    if dialog.failed.get() {
        Err(AppError::SubtitleWindow)
    } else {
        Ok(())
    }
}
fn selected(control: HWND) -> isize {
    // SAFETY: callers pass only this dialog's fixed same-thread combo handles.
    unsafe { SendMessageW(control, CB_GETCURSEL, None, None).0 }
}
fn set_text(control: HWND, text: &str) -> Result<()> {
    // SAFETY: own control copies static or typed guidance, never private data.
    unsafe { SetWindowTextW(control, &HSTRING::from(text)) }.map_err(|_| AppError::SubtitleWindow)
}
fn clear_edit(control: HWND) -> Result<()> {
    if control.0.is_null() {
        return Ok(());
    }
    // SAFETY: native private contents and undo storage on an owned EDIT are
    // cleared synchronously; all cleanup paths call this before destruction.
    unsafe {
        let cleared = SetWindowTextW(control, w!("")).map_err(|_| AppError::SubtitleWindow);
        SendMessageW(control, EM_EMPTYUNDOBUFFER, None, None);
        cleared
    }
}
#[cfg(test)]
fn set_edit(control: HWND, text: &str) -> Result<()> {
    let mut buffer = zeroize::Zeroizing::new(text.encode_utf16().collect::<Vec<_>>());
    buffer.push(0);
    // SAFETY: owned EDIT copies the live terminated UTF-16 buffer synchronously.
    unsafe { SetWindowTextW(control, PCWSTR(buffer.as_ptr())) }
        .map_err(|_| AppError::SubtitleWindow)
}
fn read_edit(control: HWND, limit: usize) -> std::result::Result<String, MediaError> {
    read_edit_value(control, limit, false)
}
fn read_optional_edit(control: HWND, limit: usize) -> std::result::Result<String, MediaError> {
    read_edit_value(control, limit, true)
}
fn read_edit_value(
    control: HWND,
    limit: usize,
    allow_empty: bool,
) -> std::result::Result<String, MediaError> {
    // SAFETY: query actual owned EDIT length; programmatic oversize must never
    // become truncated accepted filenames. Buffer is bounded and zeroized.
    let expected = unsafe { GetWindowTextLengthW(control) }.max(0) as usize;
    if expected > limit || expected == 0 && !allow_empty {
        return Err(MediaError::InvalidInput);
    }
    if expected == 0 {
        return Ok(String::new());
    }
    let mut buffer = zeroize::Zeroizing::new(vec![0u16; limit + 1]);
    let actual = unsafe { GetWindowTextW(control, &mut buffer[..]) }.max(0) as usize;
    if actual != expected {
        return Err(MediaError::InvalidInput);
    }
    String::from_utf16(&buffer[..actual]).map_err(|_| MediaError::InvalidInput)
}
fn read_password(control: HWND) -> std::result::Result<Option<ApiKey>, CredentialError> {
    // SAFETY: actual-length validation rejects programmatic oversize keys, and
    // every temporary UTF-16 and UTF-16LE copy is zeroized when this returns.
    let expected = unsafe { GetWindowTextLengthW(control) }.max(0) as usize;
    if expected > KEY_LIMIT {
        return Err(CredentialError::InvalidEncoding);
    }
    if expected == 0 {
        return Ok(None);
    }
    let mut buffer = zeroize::Zeroizing::new(vec![0u16; KEY_LIMIT + 1]);
    let actual = unsafe { GetWindowTextW(control, &mut buffer[..]) }.max(0) as usize;
    if actual != expected {
        return Err(CredentialError::Unavailable);
    }
    let mut bytes = zeroize::Zeroizing::new(Vec::with_capacity(actual * 2));
    bytes.extend(buffer[..actual].iter().flat_map(|unit| unit.to_le_bytes()));
    Ok(Some(ApiKey::from_utf16le(&bytes)?))
}
unsafe fn add(control: HWND, text: &str) -> Result<()> {
    let text = HSTRING::from(text);
    // SAFETY: owned combo synchronously copies this HSTRING before it is dropped.
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
    // SAFETY: callback-scoped CREATESTRUCTW contains the stable borrowed pointer
    // passed by create. It is cleared on WM_NCDESTROY before the Box is dropped.
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
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        dialog_message(hwnd, message, wparam, lparam)
    }))
    .unwrap_or_else(|_| {
        // SAFETY: callback still holds the borrowed stable pointer; a reentrant
        // cleanup panic is caught too, so no Rust panic crosses the native ABI.
        unsafe {
            if let Some(dialog) = state(hwnd, 0, LPARAM(0)).as_ref() {
                dialog.failed.set(true);
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| dialog.close()));
            }
        }
        LRESULT(0)
    })
}
unsafe fn dialog_message(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: state pointer is borrowed only during an owned HWND callback. No
    // native resource or Rust reference escapes this UI thread callback.
    unsafe {
        if let Some(dialog) = state(hwnd, message, lparam).as_ref() {
            let result = match message {
                WM_COMMAND => match wparam.0 & 0xffff {
                    PREPARE | 1 => dialog.begin(),
                    DISCOVER => dialog.discover(),
                    TRACK if (wparam.0 >> 16) & 0xffff == CBN_SELCHANGE as usize => {
                        dialog.track_changed()
                    }
                    CANCEL => dialog.stop(),
                    CLEAR => dialog.clear(),
                    KEEP_READY => dialog.keep_ready(),
                    CLOSE | 2 => dialog.close(),
                    LANGUAGE | PROVIDER | AUDIO_LANGUAGE | FORMAT | REFERENCE_FORMAT
                    | REFERENCE_LANGUAGE
                        if (wparam.0 >> 16) & 0xffff == CBN_SELCHANGE as usize =>
                    {
                        dialog.fields_changed()
                    }
                    MEDIA if (wparam.0 >> 16) & 0xffff == EN_CHANGE_VALUE => dialog.media_changed(),
                    SUBTITLE if (wparam.0 >> 16) & 0xffff == EN_CHANGE_VALUE => {
                        dialog.subtitle_changed()
                    }
                    REFERENCE_PATH | REFERENCE_EDITION
                        if (wparam.0 >> 16) & 0xffff == EN_CHANGE_VALUE =>
                    {
                        dialog.fields_changed()
                    }
                    _ => return DefWindowProcW(hwnd, message, wparam, lparam),
                },
                WM_TIMER if wparam.0 == POLL_TIMER => dialog.poll(),
                WM_CLOSE => dialog.close(),
                WM_SIZE => dialog.layout(),
                WM_MOUSEWHEEL => {
                    let delta = ((wparam.0 >> 16) as u16 as i16) as i32;
                    let line =
                        super::subtitle_window::scale(SCROLL_LINE_DIP, GetDpiForWindow(hwnd));
                    dialog.scroll_to(dialog.scroll.get() - delta * line / WHEEL_DELTA as i32)
                }
                WM_DPICHANGED if lparam.0 != 0 => work_area(hwnd).and_then(|work| {
                    let rect = fit_work_area(*(lparam.0 as *const RECT), work);
                    SetWindowPos(
                        hwnd,
                        None,
                        rect.left,
                        rect.top,
                        (rect.right - rect.left).max(1),
                        (rect.bottom - rect.top).max(1),
                        SWP_NOACTIVATE | SWP_NOZORDER,
                    )
                    .map_err(|_| AppError::SubtitleWindow)
                    .and_then(|_| dialog.layout())
                }),
                WM_DESTROY => {
                    dialog.discovery.borrow_mut().stop();
                    dialog.backend.borrow_mut().stop();
                    dialog.reset_notification();
                    let _ = KillTimer(Some(hwnd), POLL_TIMER);
                    dialog.clear_fields()
                }
                WM_NCDESTROY => {
                    dialog.hwnd.set(HWND::default());
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    return DefWindowProcW(hwnd, message, wparam, lparam);
                }
                _ => return DefWindowProcW(hwnd, message, wparam, lparam),
            };
            if result.is_err() {
                dialog.failed.set(true);
                let _ = dialog.close();
            }
            return LRESULT(0);
        }
        DefWindowProcW(hwnd, message, wparam, lparam)
    }
}

unsafe extern "system" fn content_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: this app-owned viewport has the same stable borrowed Box pointer as
    // the dialog. Exceptions never cross the Win32 callback ABI.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        if let Some(dialog) = state(hwnd, message, lparam).as_ref() {
            let result = match message {
                WM_COMMAND => {
                    // Controls are nested under a clipping viewport. Forward only
                    // their own notifications to the owning dialog synchronously.
                    SendMessageW(dialog.hwnd.get(), message, Some(wparam), Some(lparam));
                    return LRESULT(0);
                }
                WM_VSCROLL => dialog.scroll_command(wparam.0 & 0xffff),
                WM_MOUSEWHEEL => {
                    let delta = ((wparam.0 >> 16) as u16 as i16) as i32;
                    let line = super::subtitle_window::scale(
                        SCROLL_LINE_DIP,
                        GetDpiForWindow(dialog.hwnd.get()),
                    );
                    dialog.scroll_to(dialog.scroll.get() - delta * line / WHEEL_DELTA as i32)
                }
                WM_NCDESTROY => {
                    dialog.content.set(HWND::default());
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    return DefWindowProcW(hwnd, message, wparam, lparam);
                }
                _ => return DefWindowProcW(hwnd, message, wparam, lparam),
            };
            if result.is_err() {
                dialog.failed.set(true);
                let _ = dialog.close();
            }
            return LRESULT(0);
        }
        DefWindowProcW(hwnd, message, wparam, lparam)
    }))
    .unwrap_or_else(|_| {
        // SAFETY: callback-scoped access to stable borrowed state. Nested cleanup
        // panics are also caught; no borrowed pointer survives native destruction.
        unsafe {
            if let Some(dialog) = state(hwnd, 0, LPARAM(0)).as_ref() {
                dialog.failed.set(true);
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| dialog.close()));
            }
        }
        LRESULT(0)
    })
}

#[cfg(test)]
#[path = "prepared_dialog_tests.rs"]
mod tests;
