//! Main-thread native views. All content lives in the shared Presentation.
//! No audio, AI, content files, or COM interfaces enter this module.
use super::{
    subtitle_controls::{self, ControlPage, ControlPanel},
    subtitle_monitor, subtitle_render, subtitle_theme,
};
use crate::ui::appearance::{
    BACKGROUND_LEVELS, FONT_SIZES, MAX_LINES, MonitorPreference, SideDock, SubtitleAppearance,
    SubtitleFont,
};
use crate::ui::controls::{ControlPreferences, ControlUpdate, UiController};
use crate::{
    domain::DisplayMode,
    error::{AppError, Result},
    ui::bridge::{UI_REFRESH_INTERVAL, UiBridgeError, UiFrame, UiInbox, UiStatus},
    ui::history::SideHistory,
    ui::presentation::Presentation,
};
use std::{
    cell::{Cell, RefCell},
    marker::PhantomData,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Controls::{DRAWITEMSTRUCT, EM_SETCUEBANNER, MEASUREITEMSTRUCT, ODT_COMBOBOX},
            HiDpi::*,
            Input::KeyboardAndMouse::{VK_DOWN, VK_PRIOR},
            Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
            WindowsAndMessaging::*,
        },
    },
    core::{PCWSTR, w},
};

const CLASS: PCWSTR = w!("SubtitleNow.SideChat.Phase5");
const OVERLAY_CLASS: PCWSTR = w!("SubtitleNow.BottomSubtitle.Phase5");
const SIDE: usize = 101;
const BOTTOM: usize = 102;
const DUAL: usize = 103;
const ORIGINAL: usize = 104;
const LOCK: usize = 105;
const THROUGH: usize = 106;
const LIVE: usize = 107;
const FOLLOW: usize = 108;
const STOP: usize = 109;
const DOCK_LEFT: usize = 110;
const DOCK_RIGHT: usize = 111;
const DOCK_FLOAT: usize = 112;
const ALIGN_PAUSE: usize = 113;
const ALIGN_RESUME: usize = 114;
const ALIGN_RESET: usize = 115;
const CLEAR_PREPARED: usize = 116;
const SETTINGS: usize = 117;
const CLEAR_SUBTITLES: usize = 118;
const CREDENTIAL_SETTINGS: usize = 119;
const STATUS_DETAILS: usize = 120;
const ABOUT: usize = 121;
const HISTORY_SUBCLASS_ID: usize = 1;
const SETTINGS_WIDTH_DIP: i32 = 680;
const SETTINGS_HEIGHT_DIP: i32 = 720;
const ACTIONS_INLINE_MIN_WIDTH_DIP: i32 = 340;
const SEARCH: usize = 201;
const HISTORY: usize = 202;
const SEARCH_LABEL: usize = 203;
const STATUS: usize = 204;
const STATUS_LABEL: usize = 205;
const BRAND: usize = 206;
const BRAND_HEIGHT_DIP: i32 = 40;
const STATUS_HEIGHT_DIP: i32 = 48;
const MIN_STATUS_HEIGHT_DIP: i32 = 32;
const PREFERRED_SUBTITLE_HEIGHT_DIP: i32 = 96;
const SEARCH_LABEL_HEIGHT_DIP: i32 = 24;
const EM_SETLIMITTEXT_VALUE: u32 = 0x00c5;
const EM_SETSEL_VALUE: u32 = 0x00b1;
const EM_GETSEL_VALUE: u32 = 0x00b0;
const EM_LINEINDEX_VALUE: u32 = 0x00bb;
const EM_GETLINECOUNT_VALUE: u32 = 0x00ba;
const EM_LINEFROMCHAR_VALUE: u32 = 0x00c9;
const EM_EMPTYUNDOBUFFER_VALUE: u32 = 0x00cd;
const EN_CHANGE_VALUE: usize = 0x0300;
const EN_VSCROLL_VALUE: usize = 0x0602;
const EM_GETFIRSTVISIBLELINE_VALUE: u32 = 0x00ce;
const EM_LINESCROLL_VALUE: u32 = 0x00b6;
const UPDATE_TIMER: usize = 301;
const FONT_FIRST: usize = 500;
const SIZE_FIRST: usize = 510;
const OPACITY_FIRST: usize = 530;
const LINES_FIRST: usize = 540;
const OUTLINE: usize = 550;
const SHADOW: usize = 551;
const RESET_APPEARANCE: usize = 552;
const PRIMARY_MONITOR: usize = 600;
const FOLLOW_SOURCE_MONITOR: usize = 601;
const REFRESH_MONITORS: usize = 602;
const MONITOR_FIRST: usize = 610;
const FOLLOW_SOURCE_INTERVAL: Duration = Duration::from_millis(500);

struct RefreshFlag<'a>(&'a Cell<bool>, bool);
impl<'a> RefreshFlag<'a> {
    fn set(flag: &'a Cell<bool>) -> Self {
        Self(flag, flag.replace(true))
    }
}
impl Drop for RefreshFlag<'_> {
    fn drop(&mut self) {
        self.0.set(self.1);
    }
}

struct DpiContext(DPI_AWARENESS_CONTEXT);
impl Drop for DpiContext {
    fn drop(&mut self) {
        // SAFETY: restores this same UI thread's context after its windows are destroyed.
        unsafe {
            SetThreadDpiAwarenessContext(self.0);
        }
    }
}
struct Menu(Option<HMENU>);
impl Drop for Menu {
    fn drop(&mut self) {
        if let Some(menu) = self.0.take() {
            // SAFETY: only unattached menus are owned here; attached menus belong to the window.
            unsafe {
                let _ = DestroyMenu(menu);
            }
        }
    }
}
fn append_popup(parent: HMENU, title: &str, items: &[(usize, String)]) -> Result<HMENU> {
    // SAFETY: UI-thread menu owns its children once attached; RAII cleans failed builds.
    // HSTRING inputs survive each synchronous AppendMenu call.
    unsafe {
        let handle = CreatePopupMenu().map_err(|_| AppError::SubtitleWindow)?;
        let mut owner = Menu(Some(handle));
        for (id, label) in items {
            AppendMenuW(handle, MF_STRING, *id, &windows::core::HSTRING::from(label))
                .map_err(|_| AppError::SubtitleWindow)?;
        }
        AppendMenuW(
            parent,
            MF_POPUP,
            handle.0 as usize,
            &windows::core::HSTRING::from(title),
        )
        .map_err(|_| AppError::SubtitleWindow)?;
        owner.0 = None;
        Ok(handle)
    }
}
pub struct SubtitleShell {
    theme: RefCell<Option<subtitle_theme::Theme>>,
    prepared: RefCell<Option<crate::session::prepared::PreparedAttachment>>,
    model: RefCell<Presentation>,
    main: Cell<HWND>,
    overlay: Cell<HWND>,
    history: Cell<HWND>,
    status: Cell<HWND>,
    status_label: Cell<HWND>,
    status_window: Cell<HWND>,
    status_window_edit: Cell<HWND>,
    status_window_font: Cell<HFONT>,
    status_text: RefCell<String>,
    search: Cell<HWND>,
    search_label: Cell<HWND>,
    brand: Cell<HWND>,
    clear_button: Cell<HWND>,
    font: Cell<HFONT>,
    locked: Cell<bool>,
    through: Cell<bool>,
    failed: Cell<bool>,
    visible: Cell<bool>,
    refreshing: Cell<bool>,
    syncing_search: Cell<bool>,
    history_projection: RefCell<Option<SideHistory>>,
    inbox: RefCell<Option<UiInbox>>,
    frame: RefCell<Option<Arc<UiFrame>>>,
    controller: RefCell<Option<Box<dyn UiController>>>,
    panel: RefCell<Option<ControlPanel>>,
    preferences: Cell<ControlPreferences>,
    control_update: RefCell<ControlUpdate>,
    notice: RefCell<Option<String>>,
    appearance: Cell<SubtitleAppearance>,
    monitor_menu: Cell<HMENU>,
    monitors: RefCell<Vec<subtitle_monitor::Monitor>>,
    selected_monitor: RefCell<MonitorPreference>,
    last_follow_check: Cell<Option<Instant>>,
    followed_work: Cell<Option<RECT>>,
    follow_notice: RefCell<Option<&'static str>>,
    side_dock: Cell<SideDock>,
    page: Cell<ControlPage>,
    subtitle_rect: Cell<Option<RECT>>,
    quit_on_destroy: bool,
    _same_thread: PhantomData<Rc<()>>,
}
impl SubtitleShell {
    fn allocate(mode: DisplayMode, quit_on_destroy: bool) -> Box<Self> {
        Box::new(Self {
            theme: RefCell::new(None),
            prepared: RefCell::new(None),
            model: RefCell::new(Presentation::new(mode)),
            main: Cell::default(),
            overlay: Cell::default(),
            history: Cell::default(),
            status: Cell::default(),
            status_label: Cell::default(),
            status_window: Cell::default(),
            status_window_edit: Cell::default(),
            status_window_font: Cell::default(),
            status_text: RefCell::new(String::new()),
            search: Cell::default(),
            search_label: Cell::default(),
            brand: Cell::default(),
            clear_button: Cell::default(),
            font: Cell::default(),
            locked: Cell::new(false),
            through: Cell::new(false),
            failed: Cell::new(false),
            visible: Cell::new(false),
            refreshing: Cell::new(false),
            syncing_search: Cell::new(false),
            history_projection: RefCell::new(None),
            inbox: RefCell::new(None),
            frame: RefCell::new(None),
            controller: RefCell::new(None),
            panel: RefCell::new(None),
            preferences: Cell::new(ControlPreferences {
                provider: None,
                live_model: None,
                language: crate::domain::SourceLanguage::Auto,
                quality: crate::domain::TranslationQuality::Standard,
            }),
            control_update: RefCell::new(ControlUpdate::default()),
            notice: RefCell::new(None),
            appearance: Cell::new(SubtitleAppearance::default()),
            monitor_menu: Cell::default(),
            monitors: RefCell::new(Vec::new()),
            selected_monitor: RefCell::new(MonitorPreference::default()),
            last_follow_check: Cell::new(None),
            followed_work: Cell::new(None),
            follow_notice: RefCell::new(None),
            side_dock: Cell::new(SideDock::default()),
            page: Cell::new(ControlPage::Subtitles),
            subtitle_rect: Cell::new(None),
            quit_on_destroy,
            _same_thread: PhantomData,
        })
    }
    /// Box address stays fixed until both windows have been synchronously destroyed.
    fn create(&self, visible: bool) -> Result<()> {
        *self.theme.borrow_mut() = Some(subtitle_theme::Theme::new()?);
        // SAFETY: caller owns this Box on the UI thread. CREATESTRUCT stores a borrowed
        // pointer, never transfers ownership. Drop destroys HWNDs before freeing this Box.
        unsafe {
            let instance = HINSTANCE(
                GetModuleHandleW(None)
                    .map_err(|_| AppError::SubtitleWindow)?
                    .0,
            );
            for (name, procedure, background) in [
                (
                    status_window::CLASS,
                    Some(
                        status_window::window_proc
                            as unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT,
                    ),
                    GetSysColorBrush(COLOR_WINDOW),
                ),
                (
                    CLASS,
                    Some(
                        main_proc
                            as unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT,
                    ),
                    HBRUSH((COLOR_WINDOW.0 + 1) as *mut _),
                ),
                (
                    OVERLAY_CLASS,
                    Some(
                        overlay_proc
                            as unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT,
                    ),
                    HBRUSH::default(),
                ),
            ] {
                let class = WNDCLASSW {
                    lpfnWndProc: procedure,
                    hInstance: instance,
                    lpszClassName: name,
                    hCursor: LoadCursorW(None, IDC_ARROW).map_err(|_| AppError::SubtitleWindow)?,
                    hbrBackground: background,
                    ..Default::default()
                };
                if RegisterClassW(&class) == 0
                    && windows::Win32::Foundation::GetLastError()
                        != windows::Win32::Foundation::ERROR_CLASS_ALREADY_EXISTS
                {
                    return Err(AppError::SubtitleWindow);
                }
            }
            let pointer = self as *const Self as *const std::ffi::c_void;
            self.main.set(
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    CLASS,
                    w!("SubtitleNow — 即時字幕"),
                    WS_OVERLAPPEDWINDOW,
                    CW_USEDEFAULT,
                    CW_USEDEFAULT,
                    460,
                    640,
                    None,
                    None,
                    Some(instance),
                    Some(pointer),
                )
                .map_err(|_| AppError::SubtitleWindow)?,
            );
            self.overlay.set(
                CreateWindowExW(
                    WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED,
                    OVERLAY_CLASS,
                    w!("SubtitleNow — 底部字幕"),
                    WS_POPUP,
                    0,
                    0,
                    800,
                    140,
                    Some(self.main.get()),
                    None,
                    Some(instance),
                    Some(pointer),
                )
                .map_err(|_| AppError::SubtitleWindow)?,
            );
            let menu_bar = CreateMenu().map_err(|_| AppError::SubtitleWindow)?;
            let mut menu = Menu(Some(menu_bar));
            let menu_handle = CreatePopupMenu().map_err(|_| AppError::SubtitleWindow)?;
            let mut root = Menu(Some(menu_handle));
            AppendMenuW(menu_bar, MF_POPUP, menu_handle.0 as usize, w!("選單"))
                .map_err(|_| AppError::SubtitleWindow)?;
            root.0 = None;
            AppendMenuW(menu_bar, MF_STRING, ABOUT, w!("關於"))
                .map_err(|_| AppError::SubtitleWindow)?;
            AppendMenuW(
                menu_handle,
                MF_STRING,
                SETTINGS,
                w!("字幕與媒體設定／回到字幕主頁"),
            )
            .map_err(|_| AppError::SubtitleWindow)?;
            AppendMenuW(
                menu_handle,
                MF_STRING,
                CREDENTIAL_SETTINGS,
                w!("服務與金鑰設定／回到字幕主頁"),
            )
            .map_err(|_| AppError::SubtitleWindow)?;
            AppendMenuW(
                menu_handle,
                MF_STRING,
                STATUS_DETAILS,
                w!("可調整大小的狀態與錯誤視窗"),
            )
            .map_err(|_| AppError::SubtitleWindow)?;
            for (title, items) in [
                (
                    w!("顯示模式"),
                    vec![
                        (SIDE, w!("側邊紀錄")),
                        (BOTTOM, w!("底部字幕")),
                        (DUAL, w!("雙視窗")),
                    ],
                ),
                (
                    w!("底部字幕"),
                    vec![
                        (ORIGINAL, w!("中文＋原文")),
                        (LOCK, w!("鎖定位置")),
                        (THROUGH, w!("滑鼠穿透")),
                    ],
                ),
                (
                    w!("紀錄"),
                    vec![
                        (FOLLOW, w!("自動跟隨")),
                        (LIVE, w!("回到即時字幕")),
                        (CLEAR_SUBTITLES, w!("清除字幕")),
                    ],
                ),
                (
                    w!("側邊位置"),
                    vec![
                        (DOCK_LEFT, w!("靠左")),
                        (DOCK_RIGHT, w!("靠右")),
                        (DOCK_FLOAT, w!("自由移動")),
                    ],
                ),
                (w!("工作階段"), vec![(STOP, w!("停止辨識與翻譯"))]),
            ] {
                let popup_handle = CreatePopupMenu().map_err(|_| AppError::SubtitleWindow)?;
                let mut popup = Menu(Some(popup_handle));
                for (id, label) in items {
                    AppendMenuW(popup_handle, MF_STRING, id, label)
                        .map_err(|_| AppError::SubtitleWindow)?;
                }
                AppendMenuW(menu_handle, MF_POPUP, popup_handle.0 as usize, title)
                    .map_err(|_| AppError::SubtitleWindow)?;
                popup.0 = None;
            }
            let appearance_menu = append_popup(
                menu_handle,
                "字幕外觀",
                &[
                    (OUTLINE, "描邊".into()),
                    (SHADOW, "陰影".into()),
                    (RESET_APPEARANCE, "還原字幕外觀".into()),
                ],
            )?;
            append_popup(
                menu_handle,
                "字幕定位",
                &[
                    (ALIGN_PAUSE, "暫停字幕定位（即時辨識繼續）".into()),
                    (ALIGN_RESUME, "恢復字幕定位".into()),
                    (ALIGN_RESET, "跳轉／章節／重啟後重新定位".into()),
                    (CLEAR_PREPARED, "清除已保留的準備字幕".into()),
                ],
            )?;
            append_popup(
                appearance_menu,
                "字型",
                &SubtitleFont::ALL
                    .iter()
                    .enumerate()
                    .map(|(index, font)| (FONT_FIRST + index, font.label().into()))
                    .collect::<Vec<_>>(),
            )?;
            append_popup(
                appearance_menu,
                "字級（DIP）",
                &FONT_SIZES
                    .iter()
                    .enumerate()
                    .map(|(index, size)| (SIZE_FIRST + index, size.to_string()))
                    .collect::<Vec<_>>(),
            )?;
            append_popup(
                appearance_menu,
                "背景不透明度",
                &BACKGROUND_LEVELS
                    .iter()
                    .enumerate()
                    .map(|(index, value)| (OPACITY_FIRST + index, format!("{value}%")))
                    .collect::<Vec<_>>(),
            )?;
            append_popup(
                appearance_menu,
                "最大行數（原文與中文合計）",
                &(1..=MAX_LINES)
                    .enumerate()
                    .map(|(index, lines)| (LINES_FIRST + index, format!("{lines} 行")))
                    .collect::<Vec<_>>(),
            )?;
            self.monitor_menu
                .set(append_popup(menu_handle, "字幕螢幕", &[])?);
            self.rebuild_monitors()?;
            SetMenu(self.main.get(), Some(menu_bar)).map_err(|_| AppError::SubtitleWindow)?;
            menu.0 = None;
            if self.controller.borrow().is_some() {
                let panel =
                    ControlPanel::create(self.main.get(), instance, self.preferences.get())?;
                *self.panel.borrow_mut() = Some(panel);
            }
            // A dedicated preceding static label gives the edit a stable native name,
            // instead of inheriting the last source-activity text as its accessibility label.
            self.search_label.set(
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("STATIC"),
                    w!("搜尋字幕紀錄"),
                    WS_CHILD | WS_VISIBLE,
                    0,
                    0,
                    1,
                    1,
                    Some(self.main.get()),
                    Some(HMENU(SEARCH_LABEL as *mut _)),
                    Some(instance),
                    None,
                )
                .map_err(|_| AppError::SubtitleWindow)?,
            );
            self.search.set(
                CreateWindowExW(
                    WS_EX_CLIENTEDGE,
                    w!("EDIT"),
                    w!(""),
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
                    0,
                    0,
                    0,
                    0,
                    Some(self.main.get()),
                    Some(HMENU(SEARCH as *mut _)),
                    Some(instance),
                    None,
                )
                .map_err(|_| AppError::SubtitleWindow)?,
            );
            SendMessageW(
                self.search.get(),
                EM_SETLIMITTEXT_VALUE,
                Some(WPARAM(64)),
                None,
            );
            SendMessageW(
                self.search.get(),
                EM_SETCUEBANNER,
                Some(WPARAM(1)),
                Some(LPARAM(w!("搜尋字幕紀錄").as_ptr() as isize)),
            );
            self.clear_button.set(
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("BUTTON"),
                    w!("清除字幕"),
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP,
                    0,
                    0,
                    1,
                    1,
                    Some(self.main.get()),
                    Some(HMENU(CLEAR_SUBTITLES as *mut _)),
                    Some(instance),
                    None,
                )
                .map_err(|_| AppError::SubtitleWindow)?,
            );
            self.history.set(
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("EDIT"),
                    w!(""),
                    WS_CHILD
                        | WS_VISIBLE
                        | WS_VSCROLL
                        | WS_TABSTOP
                        | WINDOW_STYLE((ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL) as u32),
                    0,
                    0,
                    0,
                    0,
                    Some(self.main.get()),
                    Some(HMENU(HISTORY as *mut _)),
                    Some(instance),
                    None,
                )
                .map_err(|_| AppError::SubtitleWindow)?,
            );
            // SAFETY: the immutable Box address outlives its child HWND. Both the
            // subclass and model are used only on this owning UI thread; removed on NCDESTROY.
            if !SetWindowSubclass(
                self.history.get(),
                Some(history_input_proc),
                HISTORY_SUBCLASS_ID,
                self as *const Self as usize,
            )
            .as_bool()
            {
                return Err(AppError::SubtitleWindow);
            }
            self.status_label.set(
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("BUTTON"),
                    w!("狀態／錯誤（展開）"),
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP,
                    0,
                    0,
                    1,
                    1,
                    Some(self.main.get()),
                    Some(HMENU(STATUS_LABEL as *mut _)),
                    Some(instance),
                    None,
                )
                .map_err(|_| AppError::SubtitleWindow)?,
            );
            self.status.set(
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("EDIT"),
                    w!(""),
                    WS_CHILD
                        | WS_VISIBLE
                        | WS_VSCROLL
                        | WS_TABSTOP
                        | WINDOW_STYLE((ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL) as u32),
                    0,
                    0,
                    1,
                    1,
                    Some(self.main.get()),
                    Some(HMENU(STATUS as *mut _)),
                    Some(instance),
                    None,
                )
                .map_err(|_| AppError::SubtitleWindow)?,
            );
            self.brand.set(
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("STATIC"),
                    w!("KOSMOS TOOLKIT 探真拓知酷"),
                    WS_CHILD | WS_VISIBLE,
                    0,
                    0,
                    1,
                    1,
                    Some(self.main.get()),
                    Some(HMENU(BRAND as *mut _)),
                    Some(instance),
                    None,
                )
                .map_err(|_| AppError::SubtitleWindow)?,
            );
            self.update_font()?;
            subtitle_theme::style_button(self.clear_button.get());
            subtitle_theme::style_button(self.status_label.get());
            if let Some(panel) = self.panel.borrow().as_ref() {
                for button in panel.buttons() {
                    subtitle_theme::style_button(button);
                }
            }
            self.position_initial_side()?;
            self.position_overlay()?;
            self.refresh(visible)?;
            if SetTimer(
                Some(self.main.get()),
                UPDATE_TIMER,
                UI_REFRESH_INTERVAL.as_millis() as u32,
                None,
            ) == 0
            {
                return Err(AppError::SubtitleWindow);
            }
        }
        Ok(())
    }
    fn poll_inbox(&self) -> Result<bool> {
        let frame = { self.inbox.borrow_mut().as_mut().and_then(UiInbox::poll) };
        let Some(frame) = frame else {
            self.acknowledge_source();
            return Ok(false);
        };
        if self
            .inbox
            .borrow()
            .as_ref()
            .is_some_and(|inbox| inbox.cancellation().is_cancelled())
            && !matches!(frame.status, UiStatus::Stopped | UiStatus::Error)
        {
            return Ok(false);
        }
        {
            let mut model = self.model.borrow_mut();
            let switched = model.identity() != frame.identity;
            match frame.identity {
                Some(identity) => {
                    if switched {
                        model.select_session(identity);
                    }
                    if let Some(snapshot) = &frame.snapshot {
                        model.apply_shared(Arc::clone(snapshot));
                    }
                }
                None => model.stop(),
            }
        }
        *self.frame.borrow_mut() = Some(frame);
        self.refresh(self.visible.get())?;
        self.acknowledge_source();
        Ok(true)
    }
    fn poll_controller(&self) -> Result<()> {
        let update = {
            self.controller
                .borrow_mut()
                .as_mut()
                .map(|controller| controller.poll())
        };
        let Some(update) = update else {
            return Ok(());
        };
        let changed = {
            let previous = self.control_update.borrow();
            previous.busy != update.busy || previous.notice != update.notice
        };
        *self.control_update.borrow_mut() = update.clone();
        let selection_changed = if let Some(panel) = self.panel.borrow().as_ref() {
            panel.update(&update, self.can_stop())?
        } else {
            false
        };
        if changed || selection_changed {
            self.refresh(self.visible.get())?;
        }
        Ok(())
    }
    fn can_stop(&self) -> bool {
        self.inbox
            .borrow()
            .as_ref()
            .is_some_and(|inbox| !inbox.cancellation().is_cancelled())
            && self.frame.borrow().as_ref().is_none_or(|frame| {
                !matches!(
                    frame.status,
                    UiStatus::Stopped | UiStatus::Error | UiStatus::Stopping
                )
            })
    }
    fn start_from_controls(&self) -> Result<()> {
        // SAFETY: own UI-thread main window. A modal Prepared/settings dialog
        // disables this owner; posted commands cannot bypass that startup gate.
        if self.control_update.borrow().busy
            || !unsafe {
                windows::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled(self.main.get())
            }
            .as_bool()
        {
            return Ok(());
        }
        if self.page.get() != ControlPage::Subtitles {
            self.toggle_page(self.page.get())?;
        }
        let result: Result<()> = (|| {
            let (request, key) = {
                let panel = self.panel.borrow();
                let panel = panel.as_ref().ok_or(AppError::InvalidArguments)?;
                let request = panel.request()?;
                let key = panel.take_key(request.provider)?;
                (request, key)
            };
            let inbox = {
                let source = self
                    .prepared
                    .borrow()
                    .as_ref()
                    .map(|ready| ready.alignment_source())
                    .transpose()
                    .map_err(crate::session::alignment_control::AlignmentControlError::from)?;
                let mut controller = self.controller.borrow_mut();
                let controller = controller.as_mut().ok_or(AppError::InvalidArguments)?;
                match source {
                    Some(source) => controller.start_prepared(request, key, source)?,
                    None => controller.start(request, key)?,
                }
            };
            *self.inbox.borrow_mut() = Some(inbox);
            self.model.borrow_mut().stop();
            *self.frame.borrow_mut() = None;
            *self.notice.borrow_mut() = None;
            self.poll_controller()?;
            Ok(())
        })();
        if let Err(error) = result {
            *self.notice.borrow_mut() = Some(error.to_string());
        }
        self.refresh(self.visible.get())
    }
    fn save_key_from_controls(&self, provider: crate::ai::provider::SpeechProvider) -> Result<()> {
        if self.control_update.borrow().busy {
            return Ok(());
        }
        let result: Result<()> = (|| {
            let key = self
                .panel
                .borrow()
                .as_ref()
                .ok_or(AppError::InvalidArguments)?
                .take_key_to_save(provider)?;
            self.controller
                .borrow_mut()
                .as_mut()
                .ok_or(AppError::InvalidArguments)?
                .save_key(provider, key)
        })();
        let name = match provider {
            crate::ai::provider::SpeechProvider::OpenAi => "OpenAI",
            crate::ai::provider::SpeechProvider::Gemini => "Gemini",
        };
        let text = match result {
            Ok(()) => {
                format!(
                    "{name} 金鑰已儲存並確認可讀取。欄位留空可使用。\r\n位置：Windows 認證管理員；雲端結果請看下方。"
                )
            }
            Err(AppError::Credential(
                crate::ai::credentials::CredentialError::VerificationUnavailable,
            )) => {
                format!(
                    "{name}：寫入要求已完成，但本機讀取尚未確認。\r\n請按「檢查本機」；此操作不連線。"
                )
            }
            Err(error) => {
                use crate::ai::credentials::CredentialError;
                let reason = match error {
                    AppError::Credential(CredentialError::Missing) => "請先在該列輸入 API 金鑰。",
                    AppError::Credential(CredentialError::InvalidEncoding) => {
                        "金鑰格式無效，請檢查內容與長度。"
                    }
                    AppError::Credential(CredentialError::Unavailable) => {
                        "無法儲存，請確認 Windows 登入狀態。"
                    }
                    _ => "無法儲存此服務的 API 金鑰。",
                };
                format!("{name}：{reason}\r\n本次未儲存，未建立雲端連線。")
            }
        };
        self.panel
            .borrow()
            .as_ref()
            .ok_or(AppError::InvalidArguments)?
            .key_notice(&text)
    }
    fn save_preferences_from_controls(&self) -> Result<()> {
        // SAFETY: explicit action on this UI-thread owner; a modal child disables it.
        if self.control_update.borrow().busy
            || !unsafe {
                windows::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled(self.main.get())
            }
            .as_bool()
        {
            return Ok(());
        }
        let result: Result<()> = (|| {
            let preferences = self
                .panel
                .borrow()
                .as_ref()
                .ok_or(AppError::InvalidArguments)?
                .preferences()?;
            if preferences.provider.is_none() {
                return Err(crate::ai::speech::SpeechError::NotConfigured.into());
            }
            self.controller
                .borrow_mut()
                .as_mut()
                .ok_or(AppError::InvalidArguments)?
                .save_preferences(preferences)?;
            self.preferences.set(preferences);
            Ok(())
        })();
        let message = match result {
            Ok(()) => {
                "已儲存服務、來源語言與翻譯品質；下次開啟沿用。金鑰請使用各列的儲存按鈕。".into()
            }
            Err(error) => error.to_string(),
        };
        if let Some(panel) = self.panel.borrow().as_ref() {
            panel.key_notice(&message)?;
        }
        *self.notice.borrow_mut() = Some(message);
        self.refresh(self.visible.get())
    }
    fn check_key_from_controls(&self, provider: crate::ai::provider::SpeechProvider) -> Result<()> {
        if self.control_update.borrow().busy {
            return Ok(());
        }
        let result = self
            .controller
            .borrow_mut()
            .as_mut()
            .ok_or(AppError::InvalidArguments)?
            .check_key(provider);
        let name = match provider {
            crate::ai::provider::SpeechProvider::OpenAi => "OpenAI",
            crate::ai::provider::SpeechProvider::Gemini => "Gemini",
        };
        use crate::ai::credentials::CredentialError;
        let text = match result {
            Ok(()) => format!(
                "{name}：本機已儲存且可讀取。欄位留空可使用。\r\n僅確認本機；雲端連線結果請看下方狀態。"
            ),
            Err(AppError::Credential(CredentialError::Missing)) => format!(
                "{name}：本機尚未儲存 API 金鑰。\r\n請在該列輸入並按「儲存」；此檢查不連線。"
            ),
            Err(AppError::Credential(CredentialError::InvalidEncoding)) => format!(
                "{name}：本機金鑰資料無法使用，請重新儲存。\r\n此檢查不修改金鑰，也不連線。"
            ),
            Err(_) => format!(
                "{name}：無法確認本機金鑰狀態。\r\n請檢查 Windows 登入狀態後再試；此檢查不連線。"
            ),
        };
        self.panel
            .borrow()
            .as_ref()
            .ok_or(AppError::InvalidArguments)?
            .key_notice(&text)
    }
    fn clear_search_control(&self) -> Result<()> {
        let _syncing = RefreshFlag::set(&self.syncing_search);
        // SAFETY: only our own EDIT; the notification guard prevents recursive refresh
        // while the model/frame transition is in progress. No RefCell borrow survives.
        unsafe {
            SetWindowTextW(self.search.get(), w!("")).map_err(|_| AppError::SubtitleWindow)?;
            SendMessageW(self.search.get(), EM_EMPTYUNDOBUFFER_VALUE, None, None);
        }
        Ok(())
    }
    fn acknowledge_source(&self) {
        // SAFETY: checks visibility of our controller on its owning UI thread.
        // A minimized/hidden view is not acknowledged before provider startup.
        if self.visible.get()
            && self.page.get() == ControlPage::Subtitles
            && unsafe {
                IsWindowVisible(self.main.get()).as_bool() && !IsIconic(self.main.get()).as_bool()
            }
            && let Some(frame) = self.frame.borrow().as_ref()
            && let Some(inbox) = self.inbox.borrow().as_ref()
        {
            inbox.acknowledge_visible(frame.version);
        }
    }
    fn fail(&self) {
        self.failed.set(true);
        // SAFETY: posting to this thread's owned controller; no content pointer is sent.
        unsafe {
            let _ = PostMessageW(Some(self.main.get()), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
    fn update_font(&self) -> Result<()> {
        // SAFETY: font is sent to our own controls, never deleted while selected. The
        // previous font is released only after both controls reference the replacement.
        unsafe {
            let font = make_font(GetDpiForWindow(self.main.get()));
            if font.0.is_null() {
                return Err(AppError::SubtitleWindow);
            }
            let mut controls = vec![
                self.history.get(),
                self.status.get(),
                self.status_label.get(),
                self.search.get(),
                self.search_label.get(),
                self.brand.get(),
                self.clear_button.get(),
            ];
            if let Some(panel) = self.panel.borrow().as_ref() {
                controls.extend(panel.handles());
            }
            for control in controls {
                SendMessageW(
                    control,
                    WM_SETFONT,
                    Some(WPARAM(font.0 as usize)),
                    Some(LPARAM(1)),
                );
            }
            if let Some(panel) = self.panel.borrow().as_ref() {
                panel.update_row_height();
            }
            let old = self.font.replace(font);
            if !old.0.is_null() {
                let _ = DeleteObject(HGDIOBJ(old.0));
            }
        }
        Ok(())
    }
    fn theme_message(
        &self,
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> Option<LRESULT> {
        if matches!(
            message,
            WM_SYSCOLORCHANGE | WM_THEMECHANGED | WM_SETTINGCHANGE
        ) {
            // A failed cosmetic refresh must not stop an active translation session.
            if let Ok(theme) = subtitle_theme::Theme::new() {
                *self.theme.borrow_mut() = Some(theme);
                // SAFETY: owned UI-thread windows, asynchronous repaint only. No
                // RefCell borrow survives child paint callbacks.
                unsafe {
                    for window in [self.main.get(), self.status_window.get()] {
                        if IsWindow(Some(window)).as_bool() {
                            let _ = RedrawWindow(
                                Some(window),
                                None,
                                None,
                                RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN,
                            );
                        }
                    }
                }
            }
            return None;
        }
        let theme = self.theme.borrow();
        let theme = theme.as_ref()?;
        match message {
            WM_ERASEBKGND => Some(theme.erase(hwnd, HDC(wparam.0 as *mut _))),
            WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC | WM_CTLCOLORLISTBOX => {
                let child = HWND(lparam.0 as *mut _);
                let status = [self.status.get(), self.status_window_edit.get()].contains(&child);
                let surface = message != WM_CTLCOLORSTATIC || child == self.history.get();
                Some(theme.control_color(HDC(wparam.0 as *mut _), surface, status))
            }
            WM_DRAWITEM if lparam.0 != 0 => {
                // SAFETY: native callback keeps the structure and destination DC
                // alive throughout this synchronous owner-draw operation.
                let draw = unsafe { &*(lparam.0 as *const DRAWITEMSTRUCT) };
                if draw.CtlType != windows::Win32::UI::Controls::ODT_BUTTON {
                    return None;
                }
                theme.draw_button(
                    draw,
                    draw.CtlID == subtitle_controls::START as u32
                        || draw.CtlID == subtitle_controls::SAVE_PREFERENCES as u32,
                );
                Some(LRESULT(1))
            }
            _ => None,
        }
    }
    fn position_overlay(&self) -> Result<()> {
        if *self.selected_monitor.borrow() == MonitorPreference::FollowSource {
            self.last_follow_check.set(Some(Instant::now()));
            self.apply_follow_result(self.query_source_work(), true)?;
            return Ok(());
        }
        let (work, missing) = {
            let monitors = self.monitors.borrow();
            let selected = self.selected_monitor.borrow();
            let specific = match &*selected {
                MonitorPreference::Specific(name) => {
                    monitors.iter().find(|monitor| &monitor.name == name)
                }
                _ => None,
            };
            let monitor = specific
                .or_else(|| monitors.iter().find(|monitor| monitor.primary))
                .ok_or(AppError::SubtitleWindow)?;
            (
                monitor.work,
                matches!(*selected, MonitorPreference::Specific(_)) && specific.is_none(),
            )
        };
        if missing {
            *self.notice.borrow_mut() =
                Some("選定螢幕已移除，字幕暫置主螢幕；請重新選擇字幕螢幕。".into());
        }
        self.place_overlay(work)
    }
    fn position_initial_side(&self) -> Result<()> {
        let (screen, work) = subtitle_monitor::window_geometry(self.main.get())?;
        let rect = subtitle_monitor::initial_side_placement(screen, work);
        // SAFETY: only the owned UI window is moved; no model borrow crosses callbacks.
        unsafe {
            SetWindowPos(
                self.main.get(),
                None,
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            )
            .map_err(|_| AppError::SubtitleWindow)?;
        }
        Ok(())
    }
    fn toggle_settings(&self) -> Result<()> {
        self.toggle_page(ControlPage::Sources)
    }
    fn toggle_page(&self, requested: ControlPage) -> Result<()> {
        // SAFETY: rectangles and child HWNDs belong to this same UI thread. State
        // changes before SetWindowPos so reentrant layout uses the new mode.
        unsafe {
            let current = self.page.get();
            let rect = if current == requested {
                self.page.set(ControlPage::Subtitles);
                self.subtitle_rect.take().ok_or(AppError::SubtitleWindow)?
            } else {
                let mut previous = RECT::default();
                GetWindowRect(self.main.get(), &mut previous)
                    .map_err(|_| AppError::SubtitleWindow)?;
                if current == ControlPage::Subtitles {
                    self.subtitle_rect.set(Some(previous));
                }
                self.page.set(requested);
                let work = subtitle_monitor::window_work_area(self.main.get())?;
                let dpi = GetDpiForWindow(self.main.get());
                let width = (previous.right - previous.left)
                    .max(scale(SETTINGS_WIDTH_DIP, dpi))
                    .min(work.right - work.left);
                let height = (previous.bottom - previous.top)
                    .max(scale(SETTINGS_HEIGHT_DIP, dpi))
                    .min(work.bottom - work.top);
                let left = previous.left.clamp(work.left, work.right - width);
                let top = previous.top.clamp(work.top, work.bottom - height);
                RECT {
                    left,
                    top,
                    right: left + width,
                    bottom: top + height,
                }
            };
            SetWindowPos(
                self.main.get(),
                None,
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            )
            .map_err(|_| AppError::SubtitleWindow)?;
        }
        self.layout()?;
        self.refresh(self.visible.get())
    }
    fn position_side(&self) -> Result<()> {
        let side = self.side_dock.get();
        if side == SideDock::Floating {
            return Ok(());
        }
        let work = subtitle_monitor::window_work_area(self.main.get())?;
        // SAFETY: move only our controller; same-thread DPI context and bounded work area.
        // No model/mailbox borrow remains across synchronous DPI/size callbacks.
        unsafe {
            let rect =
                subtitle_monitor::side_placement(work, GetDpiForWindow(self.main.get()), side);
            SetWindowPos(
                self.main.get(),
                None,
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            )
            .map_err(|_| AppError::SubtitleWindow)?;
        }
        Ok(())
    }
    fn place_overlay(&self, work: RECT) -> Result<()> {
        // SAFETY: owned overlay, validated work area (including negative origins).
        // First placement moves onto target; the second uses its actual window DPI.
        // No RefCell borrow survives reentrant WM_DPICHANGED / WM_SIZE callbacks.
        unsafe {
            for _ in 0..2 {
                let rect = subtitle_monitor::placement(
                    work,
                    GetDpiForWindow(self.overlay.get()).clamp(96, 384),
                    self.appearance.get(),
                );
                SetWindowPos(
                    self.overlay.get(),
                    Some(HWND_TOPMOST),
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    SWP_NOACTIVATE,
                )
                .map_err(|_| AppError::SubtitleWindow)?;
            }
        }
        Ok(())
    }
    fn query_source_work(&self) -> std::result::Result<RECT, super::source_window::FollowError> {
        use super::source_window::{self, FollowError};
        let active = {
            let frame = self.frame.borrow();
            frame
                .as_ref()
                .filter(|frame| {
                    matches!(
                        frame.status,
                        UiStatus::Starting
                            | UiStatus::Listening
                            | UiStatus::Translating
                            | UiStatus::UsingExistingText
                            | UiStatus::Reconnecting
                    )
                })
                .map(|frame| (frame.source.map(|source| source.pid), frame.process_created))
        };
        let (pid, created) = if let Some((pid, created)) = active {
            (
                pid.ok_or(FollowError::NoSource)?,
                created.ok_or(FollowError::Identity)?,
            )
        } else {
            let selected = self
                .panel
                .borrow()
                .as_ref()
                .and_then(ControlPanel::selected_source)
                .ok_or(FollowError::NoSource)?;
            (selected.pid, selected.created)
        };
        source_window::work_area(pid, created)
    }
    fn apply_follow_result(
        &self,
        result: std::result::Result<RECT, super::source_window::FollowError>,
        force: bool,
    ) -> Result<bool> {
        let previous = *self.follow_notice.borrow();
        match result {
            Ok(work) => {
                if force || self.followed_work.get() != Some(work) {
                    self.place_overlay(work)?;
                }
                self.followed_work.set(Some(work));
                *self.follow_notice.borrow_mut() = None;
            }
            Err(error) => {
                // No substitution of source/window/monitor when association cannot be proven.
                *self.follow_notice.borrow_mut() = Some(error.notice());
                self.followed_work.set(None);
            }
        }
        Ok(previous != *self.follow_notice.borrow())
    }
    fn poll_follow(&self) -> Result<()> {
        if *self.selected_monitor.borrow() != MonitorPreference::FollowSource
            || self
                .last_follow_check
                .get()
                .is_some_and(|last| last.elapsed() < FOLLOW_SOURCE_INTERVAL)
        {
            return Ok(());
        }
        self.last_follow_check.set(Some(Instant::now()));
        if self.apply_follow_result(self.query_source_work(), false)? {
            self.refresh(self.visible.get())?;
        }
        Ok(())
    }
    fn rebuild_monitors(&self) -> Result<()> {
        let monitors = subtitle_monitor::enumerate()?;
        let menu = self.monitor_menu.get();
        // SAFETY: attached submenu belongs to our main window (or construction owner).
        // No user data pointers are stored; bounded command IDs map to this inventory.
        unsafe {
            loop {
                let count = GetMenuItemCount(Some(menu));
                if count < 0 {
                    return Err(AppError::SubtitleWindow);
                }
                if count == 0 {
                    break;
                }
                DeleteMenu(menu, 0, MF_BYPOSITION).map_err(|_| AppError::SubtitleWindow)?;
            }
            for (id, label) in [
                (PRIMARY_MONITOR, "主螢幕"),
                (FOLLOW_SOURCE_MONITOR, "跟隨來源所在螢幕"),
                (REFRESH_MONITORS, "重新整理螢幕清單"),
            ] {
                AppendMenuW(menu, MF_STRING, id, &windows::core::HSTRING::from(label))
                    .map_err(|_| AppError::SubtitleWindow)?;
            }
            for (index, monitor) in monitors.iter().enumerate() {
                let label = format!(
                    "{}{}",
                    monitor.name,
                    if monitor.primary {
                        "（主螢幕）"
                    } else {
                        ""
                    }
                );
                AppendMenuW(
                    menu,
                    MF_STRING,
                    MONITOR_FIRST + index,
                    &windows::core::HSTRING::from(label),
                )
                .map_err(|_| AppError::SubtitleWindow)?;
            }
        }
        *self.monitors.borrow_mut() = monitors;
        Ok(())
    }
    fn render_overlay(&self) -> Result<()> {
        let text = self.model.borrow().overlay_text();
        subtitle_render::render(self.overlay.get(), &text, self.appearance.get())
    }
    fn refresh(&self, visible: bool) -> Result<()> {
        self.visible.set(visible);
        let _refreshing = RefreshFlag::set(&self.refreshing);
        let (mode, mut history, follow, original, searching) = {
            let model = self.model.borrow();
            (
                model.mode,
                SideHistory::from_view(&model),
                model.auto_follow,
                model.original_in_overlay,
                model.searching(),
            )
        };
        let mut status = self
            .frame
            .borrow()
            .as_ref()
            .map(|frame| frame.status_text())
            .unwrap_or_else(|| UiFrame::default().status_text());
        if self.frame.borrow().is_none()
            && let Some(panel) = self.panel.borrow().as_ref()
            && let Some(source) = panel.selected_source()
        {
            let service = panel
                .preferences()
                .ok()
                .and_then(|preferences| preferences.provider)
                .map(|provider| format!("{provider:?}"))
                .unwrap_or_else(|| "尚未選擇服務".into());
            status = format!(
                "已選擇 PID {}（含子程序全部音訊） · {} · 尚未啟動擷取或雲端",
                source.pid, service
            );
        }
        if !self.control_update.borrow().busy
            && let Some(notice) = self
                .panel
                .borrow()
                .as_ref()
                .and_then(ControlPanel::live_selection_notice)
        {
            status = format!("{notice}\r\n{status}");
        }
        // SAFETY: all HWNDs belong to this thread. Strings remain alive for synchronous
        // SetWindowText/SendMessage. No RefCell borrow crosses a reentrant Win32 call.
        unsafe {
            let side_visible = mode != DisplayMode::BottomSubtitle;
            if side_visible && history.text.is_empty() && searching {
                history = SideHistory::placeholder("找不到符合的字幕。".into());
            } else if side_visible && history.text.is_empty() {
                history = SideHistory::placeholder(
                    "本次尚無字幕。\r\n字幕紀錄只保留在本次開啟期間的記憶體中。".into(),
                );
            } else if !side_visible {
                history =
                    SideHistory::placeholder("底部字幕模式\r\n可從上方選單切換顯示方式。".into());
            }
            let mut notice = self
                .notice
                .borrow()
                .clone()
                .or_else(|| self.control_update.borrow().notice.clone())
                .unwrap_or_default();
            if let Some(follow_notice) = *self.follow_notice.borrow() {
                if !notice.is_empty() {
                    notice.push_str("\r\n");
                }
                notice.push_str(follow_notice);
            }
            let header = if notice.is_empty() {
                format!("{status}\r\n\r\n")
            } else {
                format!("{status}\r\n{notice}\r\n\r\n")
            };
            let empty_label = self
                .frame
                .borrow()
                .as_ref()
                .map_or(UiStatus::Idle, |frame| frame.status)
                .empty_subtitle_label();
            let header = if self.model.borrow().rows().next().is_none() {
                format!("{header}{empty_label}")
            } else {
                header
            };
            let changed = *self.status_text.borrow() != header;
            if changed {
                SetWindowTextW(self.status.get(), &windows::core::HSTRING::from(&header))
                    .map_err(|_| AppError::SubtitleWindow)?;
                if self.update_status_window_text(&header).is_err() {
                    *self.notice.borrow_mut() =
                        Some("無法更新狀態視窗；可繼續使用主頁狀態框。".into());
                }
                *self.status_text.borrow_mut() = header;
            }
            self.update_history(history, follow)?;
            let menu = GetMenu(self.main.get());
            let can_stop = self.can_stop();
            if let Some(panel) = self.panel.borrow().as_ref() {
                panel.enable(self.control_update.borrow().busy, can_stop);
                let sources = self.control_update.borrow().sources.clone();
                panel.update_details(&sources)?;
            }
            // Return value is the prior menu state (zero is valid), not a success BOOL.
            let previous_state = EnableMenuItem(
                menu,
                STOP as u32,
                MF_BYCOMMAND | if can_stop { MF_ENABLED } else { MF_GRAYED },
            );
            if previous_state.0 == -1 {
                return Err(AppError::SubtitleWindow);
            }
            for (id, checked) in [
                (SIDE, mode == DisplayMode::SideChat),
                (BOTTOM, mode == DisplayMode::BottomSubtitle),
                (DUAL, mode == DisplayMode::Dual),
                (ORIGINAL, original),
                (LOCK, self.locked.get()),
                (THROUGH, self.through.get()),
                (FOLLOW, follow),
                (OUTLINE, self.appearance.get().outline),
                (SHADOW, self.appearance.get().shadow),
                (DOCK_LEFT, self.side_dock.get() == SideDock::Left),
                (DOCK_RIGHT, self.side_dock.get() == SideDock::Right),
                (DOCK_FLOAT, self.side_dock.get() == SideDock::Floating),
                (SETTINGS, self.page.get() == ControlPage::Sources),
                (
                    CREDENTIAL_SETTINGS,
                    self.page.get() == ControlPage::Credentials,
                ),
            ] {
                CheckMenuItem(
                    menu,
                    id as u32,
                    (MF_BYCOMMAND | if checked { MF_CHECKED } else { MF_UNCHECKED }).0,
                );
            }
            let style = self.appearance.get();
            let mut checks = Vec::new();
            checks.extend(
                SubtitleFont::ALL
                    .iter()
                    .enumerate()
                    .map(|(index, font)| (FONT_FIRST + index, *font == style.font)),
            );
            checks.extend(
                FONT_SIZES
                    .iter()
                    .enumerate()
                    .map(|(index, size)| (SIZE_FIRST + index, *size == style.font_size)),
            );
            checks.extend(
                BACKGROUND_LEVELS.iter().enumerate().map(|(index, value)| {
                    (OPACITY_FIRST + index, *value == style.background_percent)
                }),
            );
            checks.extend(
                (1..=MAX_LINES)
                    .enumerate()
                    .map(|(index, lines)| (LINES_FIRST + index, lines == style.max_lines)),
            );
            for (id, checked) in checks {
                CheckMenuItem(
                    menu,
                    id as u32,
                    (MF_BYCOMMAND | if checked { MF_CHECKED } else { MF_UNCHECKED }).0,
                );
            }
            let selected = self.selected_monitor.borrow().clone();
            CheckMenuItem(
                self.monitor_menu.get(),
                PRIMARY_MONITOR as u32,
                if selected == MonitorPreference::Primary {
                    MF_CHECKED.0
                } else {
                    MF_UNCHECKED.0
                },
            );
            CheckMenuItem(
                self.monitor_menu.get(),
                FOLLOW_SOURCE_MONITOR as u32,
                if selected == MonitorPreference::FollowSource {
                    MF_CHECKED.0
                } else {
                    MF_UNCHECKED.0
                },
            );
            for (index, monitor) in self.monitors.borrow().iter().enumerate() {
                CheckMenuItem(
                    self.monitor_menu.get(),
                    (MONITOR_FIRST + index) as u32,
                    if selected == MonitorPreference::Specific(monitor.name.clone()) {
                        MF_CHECKED.0
                    } else {
                        MF_UNCHECKED.0
                    },
                );
            }
            if mode != DisplayMode::SideChat {
                self.render_overlay()?;
            }
            self.sync_visibility();
            self.layout()?;
            if follow {
                self.follow_history_to_end();
            }
        }
        // SAFETY: same-thread owned HWND queries only, after successful native
        // content/render submission. Hidden, minimized, searched or scrolled-away
        // SideChat updates do not count as current-row presentation.
        let has_ticket = self
            .frame
            .borrow()
            .as_ref()
            .and_then(|frame| frame.render_ticket.as_ref())
            .is_some_and(|ticket| ticket.pending());
        let current_submitted = has_ticket
            && self.model.borrow().latest_caption_visible()
            && unsafe {
                visible
                    && ((mode != DisplayMode::SideChat
                        && IsWindowVisible(self.overlay.get()).as_bool())
                        || (mode != DisplayMode::BottomSubtitle
                            && follow
                            && !searching
                            && IsWindowVisible(self.history.get()).as_bool()
                            && !IsIconic(self.main.get()).as_bool()
                            && RedrawWindow(
                                Some(self.history.get()),
                                None,
                                None,
                                RDW_INVALIDATE | RDW_UPDATENOW,
                            )
                            .as_bool()))
            };
        if current_submitted
            && let Some(frame) = self.frame.borrow().as_ref()
            && let Some(inbox) = self.inbox.borrow().as_ref()
        {
            inbox.render_submitted(frame.version);
        }
        Ok(())
    }
    fn update_history(&self, next: SideHistory, follow: bool) -> Result<()> {
        // Take the old projection out before any reentrant native message. At most
        // two bounded session projections exist transiently, never a history archive.
        let previous = self.history_projection.borrow_mut().take();
        // SAFETY: our same-thread EDIT owns its copied text. Stack DWORD pointers
        // remain valid through synchronous EM_GETSEL; all indices use UTF-16 units.
        unsafe {
            if previous.as_ref().is_none_or(|old| old.text != next.text) {
                let old_line =
                    SendMessageW(self.history.get(), EM_GETFIRSTVISIBLELINE_VALUE, None, None).0;
                let old_top = SendMessageW(
                    self.history.get(),
                    EM_LINEINDEX_VALUE,
                    Some(WPARAM(old_line.max(0) as usize)),
                    None,
                )
                .0
                .max(0) as u32;
                let (mut start, mut end) = (0u32, 0u32);
                SendMessageW(
                    self.history.get(),
                    EM_GETSEL_VALUE,
                    Some(WPARAM((&mut start as *mut u32) as usize)),
                    Some(LPARAM((&mut end as *mut u32) as isize)),
                );
                SetWindowTextW(
                    self.history.get(),
                    &windows::core::HSTRING::from(next.text.as_str()),
                )
                .map_err(|_| AppError::SubtitleWindow)?;
                if !follow {
                    let selected = previous
                        .as_ref()
                        .and_then(|old| {
                            Some((
                                old.retained_position(&next, start)?,
                                old.retained_position(&next, end)?,
                            ))
                        })
                        .unwrap_or((0, 0));
                    SendMessageW(
                        self.history.get(),
                        EM_SETSEL_VALUE,
                        Some(WPARAM(selected.0 as usize)),
                        Some(LPARAM(selected.1 as isize)),
                    );
                    let top = previous
                        .as_ref()
                        .map_or(0, |old| old.scroll_position(&next, old_top));
                    let line = SendMessageW(
                        self.history.get(),
                        EM_LINEFROMCHAR_VALUE,
                        Some(WPARAM(top as usize)),
                        None,
                    )
                    .0;
                    let current =
                        SendMessageW(self.history.get(), EM_GETFIRSTVISIBLELINE_VALUE, None, None)
                            .0;
                    SendMessageW(
                        self.history.get(),
                        EM_LINESCROLL_VALUE,
                        None,
                        Some(LPARAM(line - current)),
                    );
                }
            }
        }
        *self.history_projection.borrow_mut() = Some(next);
        Ok(())
    }
    fn follow_history_to_end(&self) {
        // SAFETY: same-thread owned EDIT, bounded text. Actual visual line count
        // and UTF-16 offsets stay below i32::MAX. No focus, clipboard or content read.
        // This runs after layout/reflow under RefreshFlag, so programmatic scrolling
        // cannot masquerade as manual input or leave the last line offscreen.
        unsafe {
            let end = GetWindowTextLengthW(self.history.get()).max(0) as usize;
            SendMessageW(
                self.history.get(),
                EM_SETSEL_VALUE,
                Some(WPARAM(end)),
                Some(LPARAM(end as isize)),
            );
            let lines = SendMessageW(self.history.get(), EM_GETLINECOUNT_VALUE, None, None).0;
            SendMessageW(
                self.history.get(),
                EM_LINESCROLL_VALUE,
                None,
                Some(LPARAM(lines.max(0))),
            );
        }
    }
    fn resume_history_follow_at_bottom(&self) {
        if self.refreshing.get() || self.model.borrow().searching() {
            return;
        }
        // SAFETY: initialized scalar out structures and DWORDs for synchronous
        // same-thread queries to our owned EDIT; selection content is never read.
        unsafe {
            let mut scroll = SCROLLINFO {
                cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
                ..Default::default()
            };
            if GetScrollInfo(self.history.get(), SB_VERT, &mut scroll).is_err() || scroll.nPage == 0
            {
                return;
            }
            let bottom = (scroll.nMax - scroll.nPage.saturating_sub(1) as i32).max(scroll.nMin);
            let (mut start, mut end) = (0u32, 0u32);
            SendMessageW(
                self.history.get(),
                EM_GETSEL_VALUE,
                Some(WPARAM((&mut start as *mut u32) as usize)),
                Some(LPARAM((&mut end as *mut u32) as isize)),
            );
            if scroll.nPos >= bottom && start == end {
                self.model.borrow_mut().auto_follow = true;
                CheckMenuItem(GetMenu(self.main.get()), FOLLOW as u32, MF_CHECKED.0);
            }
        }
    }
    fn sync_visibility(&self) {
        let visible = self.visible.get();
        // SAFETY: same-thread owned windows, no activation on a background refresh.
        // The explicit first launch is handled separately by run_window.
        unsafe {
            if IsWindowVisible(self.main.get()).as_bool() != visible {
                let _ = ShowWindow(self.main.get(), if visible { SW_SHOWNA } else { SW_HIDE });
            }
            let overlay_visible = visible
                && !IsIconic(self.main.get()).as_bool()
                && self.model.borrow().mode != DisplayMode::SideChat;
            if IsWindowVisible(self.overlay.get()).as_bool() != overlay_visible {
                let _ = ShowWindow(
                    self.overlay.get(),
                    if overlay_visible { SW_SHOWNA } else { SW_HIDE },
                );
            }
        }
    }
    fn layout(&self) -> Result<()> {
        let _refreshing = RefreshFlag::set(&self.refreshing);
        // SAFETY: owned same-thread HWNDs; all child dimensions are clamped.
        unsafe {
            if IsIconic(self.main.get()).as_bool() {
                return Ok(());
            }
            let mut rect = RECT::default();
            GetClientRect(self.main.get(), &mut rect).map_err(|_| AppError::SubtitleWindow)?;
            let dpi = GetDpiForWindow(self.main.get());
            let page = self.page.get();
            let compact = page == ControlPage::Subtitles;
            let pad = scale(if compact { 6 } else { 12 }, dpi);
            let row = scale(30, dpi);
            let label = scale(SEARCH_LABEL_HEIGHT_DIP, dpi);
            let full = (rect.right - 2 * pad).max(1);
            let mut panel_height = 0;
            let mut action_height = row;
            if let Some(panel) = self.panel.borrow().as_ref() {
                panel.set_visible(page);
                panel_height = panel.layout_page(rect.right, pad, row, page)?;
                if page == ControlPage::Sources {
                    panel.update_details(&self.control_update.borrow().sources)?;
                }
                let top = pad + panel_height;
                if rect.right >= scale(ACTIONS_INLINE_MIN_WIDTH_DIP, dpi) {
                    let third = ((full - 2 * pad) / 3).max(1);
                    for (index, control) in [panel.start, panel.stop, self.clear_button.get()]
                        .into_iter()
                        .enumerate()
                    {
                        let x = pad + index as i32 * (third + pad);
                        let width = if index == 2 {
                            (rect.right - pad - x).max(1)
                        } else {
                            third
                        };
                        MoveWindow(control, x, top, width, row, true)
                            .map_err(|_| AppError::SubtitleWindow)?;
                    }
                } else {
                    action_height = 2 * row + pad;
                    let half = ((full - pad) / 2).max(1);
                    MoveWindow(panel.start, pad, top, full, row, true)
                        .map_err(|_| AppError::SubtitleWindow)?;
                    MoveWindow(panel.stop, pad, top + row + pad, half, row, true)
                        .map_err(|_| AppError::SubtitleWindow)?;
                    MoveWindow(
                        self.clear_button.get(),
                        pad + half + pad,
                        top + row + pad,
                        (full - half - pad).max(1),
                        row,
                        true,
                    )
                    .map_err(|_| AppError::SubtitleWindow)?;
                }
            } else {
                MoveWindow(self.clear_button.get(), pad, pad, full, row, true)
                    .map_err(|_| AppError::SubtitleWindow)?;
            }
            let status_top = pad + panel_height + action_height + pad;
            let spare = rect.bottom - status_top - 2 * label - row - 3 * pad;
            let status_height = if compact {
                scale(STATUS_HEIGHT_DIP, dpi)
                    .min(
                        (spare - scale(PREFERRED_SUBTITLE_HEIGHT_DIP, dpi))
                            .max(scale(MIN_STATUS_HEIGHT_DIP, dpi)),
                    )
                    .min((rect.bottom / 4).max(1))
            } else {
                0
            };
            let status_label_height = if compact { label } else { 0 };
            for control in [self.status_label.get(), self.status.get()] {
                if (GetWindowLongPtrW(control, GWL_STYLE) as u32 & WS_VISIBLE.0 != 0) != compact {
                    let _ = ShowWindow(control, if compact { SW_SHOWNA } else { SW_HIDE });
                }
            }
            MoveWindow(self.status_label.get(), pad, status_top, full, label, true)
                .map_err(|_| AppError::SubtitleWindow)?;
            MoveWindow(
                self.status.get(),
                pad,
                status_top + label,
                full,
                status_height,
                true,
            )
            .map_err(|_| AppError::SubtitleWindow)?;
            let search_top =
                status_top + status_label_height + status_height + if compact { pad } else { 0 };
            MoveWindow(self.search_label.get(), pad, search_top, full, label, true)
                .map_err(|_| AppError::SubtitleWindow)?;
            MoveWindow(self.search.get(), pad, search_top + label, full, row, true)
                .map_err(|_| AppError::SubtitleWindow)?;
            let top = search_top + label + row + pad;
            // Keep the full brand visible with wrapping in the narrow SideChat.
            // Extremely short windows prioritize the existing caption viewport.
            let brand_height = scale(BRAND_HEIGHT_DIP, dpi);
            let footer = if rect.bottom - top - 2 * pad
                >= brand_height + scale(PREFERRED_SUBTITLE_HEIGHT_DIP, dpi)
            {
                brand_height + pad
            } else {
                0
            };
            let show_brand = footer > 0;
            if (GetWindowLongPtrW(self.brand.get(), GWL_STYLE) as u32 & WS_VISIBLE.0 != 0)
                != show_brand
            {
                let _ = ShowWindow(
                    self.brand.get(),
                    if show_brand { SW_SHOWNA } else { SW_HIDE },
                );
            }
            MoveWindow(
                self.brand.get(),
                pad,
                rect.bottom - pad - brand_height,
                full,
                brand_height,
                true,
            )
            .map_err(|_| AppError::SubtitleWindow)?;
            MoveWindow(
                self.history.get(),
                pad,
                top,
                full,
                (rect.bottom - top - pad - footer).max(1),
                true,
            )
            .map_err(|_| AppError::SubtitleWindow)?;
            subtitle_theme::inset_edit(self.history.get(), true);
            subtitle_theme::inset_edit(self.status.get(), true);
            subtitle_theme::inset_edit(self.search.get(), false);
            if let Some(panel) = self.panel.borrow().as_ref() {
                for edit in [panel.openai_key, panel.gemini_key] {
                    subtitle_theme::inset_edit(edit, false);
                }
            }
        }
        Ok(())
    }
    fn command(&self, id: usize) -> Result<()> {
        if id == ABOUT {
            if super::about_window::show(self.main.get())? {
                if let Some(controller) = self.controller.borrow_mut().as_mut() {
                    controller.request_stop();
                }
                // SAFETY: post close to our own UI-thread window; normal teardown
                // cancels and joins all session workers before the process exits.
                unsafe { PostMessageW(Some(self.main.get()), WM_CLOSE, WPARAM(0), LPARAM(0)) }
                    .map_err(|_| AppError::SubtitleWindow)?;
            }
            return Ok(());
        }
        if [STATUS_DETAILS, STATUS_LABEL].contains(&id) {
            if let Err(error) = self.open_status_window(self.visible.get()) {
                *self.notice.borrow_mut() = Some(error.to_string());
                return self.refresh(self.visible.get());
            }
            return Ok(());
        }
        if [ALIGN_PAUSE, ALIGN_RESUME, ALIGN_RESET].contains(&id) {
            let action = match id {
                ALIGN_PAUSE => crate::ui::alignment::AlignmentAction::Pause,
                ALIGN_RESUME => crate::ui::alignment::AlignmentAction::Resume,
                _ => crate::ui::alignment::AlignmentAction::Reset,
            };
            let result = self
                .inbox
                .borrow()
                .as_ref()
                .ok_or(UiBridgeError::InvalidState)
                .and_then(|inbox| inbox.alignment_action(action));
            if let Err(error) = result {
                *self.notice.borrow_mut() = Some(error.to_string());
            }
            return self.refresh(self.visible.get());
        }
        if id == CLEAR_PREPARED {
            self.prepared.borrow_mut().take();
            *self.notice.borrow_mut() =
                Some("已清除準備字幕；正在執行的即時辨識仍使用原選定服務。".into());
            return self.refresh(self.visible.get());
        }
        let previous_style = self.appearance.get();
        let mut style = previous_style;
        let mut appearance_changed = true;
        match id {
            CLEAR_SUBTITLES => {
                self.model.borrow_mut().clear_subtitles();
                self.clear_search_control()?;
                self.history_projection.borrow_mut().take();
                *self.notice.borrow_mut() =
                    Some("已清除字幕紀錄。處理狀態不變，後續新段落會接續顯示。".into());
                return self.refresh(self.visible.get());
            }
            SETTINGS => return self.toggle_settings(),
            CREDENTIAL_SETTINGS => return self.toggle_page(ControlPage::Credentials),
            OUTLINE => style.outline = !style.outline,
            SHADOW => style.shadow = !style.shadow,
            RESET_APPEARANCE => style = SubtitleAppearance::default(),
            id if (FONT_FIRST..FONT_FIRST + SubtitleFont::ALL.len()).contains(&id) => {
                style.font = SubtitleFont::ALL[id - FONT_FIRST]
            }
            id if (SIZE_FIRST..SIZE_FIRST + FONT_SIZES.len()).contains(&id) => {
                style.font_size = FONT_SIZES[id - SIZE_FIRST]
            }
            id if (OPACITY_FIRST..OPACITY_FIRST + BACKGROUND_LEVELS.len()).contains(&id) => {
                style.background_percent = BACKGROUND_LEVELS[id - OPACITY_FIRST]
            }
            id if (LINES_FIRST..LINES_FIRST + usize::from(MAX_LINES)).contains(&id) => {
                style.max_lines = (id - LINES_FIRST + 1) as u8
            }
            _ => appearance_changed = false,
        }
        if appearance_changed {
            self.appearance.set(style);
            // Color/effect/font-family changes preserve a manually dragged position.
            if style.font_size != previous_style.font_size
                || style.max_lines != previous_style.max_lines
            {
                self.position_overlay()?;
            }
            return self.refresh(self.visible.get());
        }
        match id {
            DOCK_LEFT | DOCK_RIGHT | DOCK_FLOAT => {
                self.side_dock.set(match id {
                    DOCK_LEFT => SideDock::Left,
                    DOCK_RIGHT => SideDock::Right,
                    _ => SideDock::Floating,
                });
                self.position_side()?;
            }
            PRIMARY_MONITOR => {
                *self.selected_monitor.borrow_mut() = MonitorPreference::Primary;
                *self.follow_notice.borrow_mut() = None;
                *self.notice.borrow_mut() = None;
                self.position_overlay()?;
            }
            FOLLOW_SOURCE_MONITOR => {
                *self.selected_monitor.borrow_mut() = MonitorPreference::FollowSource;
                self.followed_work.set(None);
                *self.notice.borrow_mut() = None;
                self.position_overlay()?;
            }
            REFRESH_MONITORS => {
                self.rebuild_monitors()?;
                self.position_overlay()?;
            }
            id if (MONITOR_FIRST..MONITOR_FIRST + subtitle_monitor::MAX_MONITORS).contains(&id) => {
                let name = self
                    .monitors
                    .borrow()
                    .get(id - MONITOR_FIRST)
                    .map(|monitor| monitor.name.clone());
                if let Some(name) = name {
                    *self.selected_monitor.borrow_mut() = MonitorPreference::Specific(name);
                    *self.follow_notice.borrow_mut() = None;
                    *self.notice.borrow_mut() = None;
                    self.position_overlay()?;
                }
            }
            STOP => {
                if let Some(panel) = self.panel.borrow().as_ref() {
                    panel.clear_subtitle_source()?;
                }
                if self.inbox.borrow().is_none() {
                    return Ok(());
                }
                if let Some(inbox) = self.inbox.borrow().as_ref() {
                    inbox.cancellation().cancel();
                }
                if let Some(controller) = self.controller.borrow_mut().as_mut() {
                    controller.request_stop();
                }
                self.model.borrow_mut().stop();
                let mut frame = self
                    .frame
                    .borrow()
                    .as_ref()
                    .map(|frame| (**frame).clone())
                    .unwrap_or_default();
                frame.identity = None;
                frame.snapshot = None;
                frame.status = UiStatus::Stopping;
                frame.content.stop();
                frame.lyrics.stop();
                frame.subtitles.stop();
                *self.frame.borrow_mut() = Some(Arc::new(frame));
            }
            SIDE | BOTTOM | DUAL => {
                self.model.borrow_mut().mode = match id {
                    SIDE => DisplayMode::SideChat,
                    BOTTOM => DisplayMode::BottomSubtitle,
                    _ => DisplayMode::Dual,
                }
            }
            ORIGINAL => {
                let mut model = self.model.borrow_mut();
                model.original_in_overlay = !model.original_in_overlay;
            }
            FOLLOW => {
                let mut model = self.model.borrow_mut();
                model.auto_follow = !model.auto_follow;
            }
            LIVE => {
                self.model.borrow_mut().return_to_live();
                self.clear_search_control()?;
            }
            LOCK => self.locked.set(!self.locked.get()),
            THROUGH => {
                self.through.set(!self.through.get());
                // SAFETY: changes only this app's overlay extended style. Last-error is
                // reset because SetWindowLongPtr legitimately returns a zero old value.
                unsafe {
                    let old = GetWindowLongPtrW(self.overlay.get(), GWL_EXSTYLE) as u32;
                    let next = if self.through.get() {
                        old | WS_EX_TRANSPARENT.0
                    } else {
                        old & !WS_EX_TRANSPARENT.0
                    };
                    windows::Win32::Foundation::SetLastError(
                        windows::Win32::Foundation::WIN32_ERROR(0),
                    );
                    if SetWindowLongPtrW(self.overlay.get(), GWL_EXSTYLE, next as isize) == 0
                        && windows::Win32::Foundation::GetLastError().0 != 0
                    {
                        return Err(AppError::SubtitleWindow);
                    }
                    SetWindowPos(
                        self.overlay.get(),
                        None,
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
                    )
                    .map_err(|_| AppError::SubtitleWindow)?;
                }
            }
            _ => return Ok(()),
        }
        self.refresh(self.visible.get())
    }
}
impl Drop for SubtitleShell {
    fn drop(&mut self) {
        self.destroy_status_window();
        // SAFETY: owned main HWND is checked before clearing its child controls;
        // WM_CLOSE may already have destroyed the native window before Rust Drop.
        if unsafe { IsWindow(Some(self.main.get())) }.as_bool()
            && let Some(panel) = self.panel.borrow().as_ref()
        {
            let _ = panel.clear_keys();
            let _ = panel.clear_subtitle_source();
        }
        // SAFETY: same UI thread; destroy owned windows synchronously before Box/font
        // release. Destroying a parent may already destroy its owned overlay/children.
        unsafe {
            for window in [self.overlay.get(), self.main.get()] {
                if IsWindow(Some(window)).as_bool() {
                    let _ = DestroyWindow(window);
                }
            }
            let font = self.font.get();
            if !font.0.is_null() {
                let _ = DeleteObject(HGDIOBJ(font.0));
            }
        }
    }
}
pub fn run(mode: DisplayMode) -> Result<()> {
    run_window(mode, None, None)
}
pub fn run_controlled(
    mode: DisplayMode,
    preferences: ControlPreferences,
    controller: Box<dyn UiController>,
) -> Result<()> {
    run_window(mode, None, Some((preferences, controller)))
}
/// The caller owns the single backend and must observe inbox cancellation and join
/// it when this main-thread loop returns. No provider is started by this function.
pub fn run_with_inbox(mode: DisplayMode, inbox: UiInbox) -> Result<()> {
    run_window(mode, Some(inbox), None)
}
fn run_window(
    mode: DisplayMode,
    inbox: Option<UiInbox>,
    controller: Option<(ControlPreferences, Box<dyn UiController>)>,
) -> Result<()> {
    // SAFETY: sets awareness on the UI thread only, before any of its windows exist.
    let previous =
        unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    if previous.0.is_null() {
        return Err(AppError::SubtitleWindow);
    }
    let _dpi = DpiContext(previous);
    let shell = SubtitleShell::allocate(mode, true);
    *shell.inbox.borrow_mut() = inbox;
    if let Some((preferences, controller)) = controller {
        shell.preferences.set(preferences);
        *shell.controller.borrow_mut() = Some(controller);
    }
    shell.create(true)?;
    // SAFETY: --ui explicitly requests a visible window. The first ShowWindow can
    // consume a launcher's STARTUPINFO hidden flag; this call applies our requested view.
    unsafe {
        let _ = ShowWindow(shell.main.get(), SW_SHOW);
    }
    // SAFETY: MSG is stack-owned, loop dispatches only on this window-owning thread.
    unsafe {
        let mut message = MSG::default();
        loop {
            let status = GetMessageW(&mut message, None, 0, 0).0;
            if status == -1 {
                return Err(AppError::SubtitleWindow);
            }
            if status == 0 {
                break;
            }
            if !IsDialogMessageW(shell.main.get(), &message).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
    if shell.failed.get() {
        Err(AppError::SubtitleWindow)
    } else {
        Ok(())
    }
}
pub(super) fn scale(value: i32, dpi: u32) -> i32 {
    (i64::from(value) * i64::from(dpi.max(96)) / 96) as i32
}
pub(super) unsafe fn make_font(dpi: u32) -> HFONT {
    // SAFETY: initialized, correctly sized NONCLIENTMETRICSW is filled synchronously.
    // Clone Windows' message font for this monitor; caller owns the resulting HFONT.
    unsafe {
        let mut metrics = NONCLIENTMETRICSW {
            cbSize: std::mem::size_of::<NONCLIENTMETRICSW>() as u32,
            ..Default::default()
        };
        if SystemParametersInfoForDpi(
            SPI_GETNONCLIENTMETRICS.0,
            metrics.cbSize,
            Some((&mut metrics as *mut NONCLIENTMETRICSW).cast()),
            0,
            dpi,
        )
        .is_err()
        {
            return HFONT::default();
        }
        CreateFontIndirectW(&metrics.lfMessageFont)
    }
}
unsafe fn state(hwnd: HWND, message: u32, lparam: LPARAM) -> *const SubtitleShell {
    // SAFETY: WM_NCCREATE lParam points to CREATESTRUCTW for this synchronous callback.
    // lpCreateParams is the stable Box pointer supplied by create(). It remains alive
    // until WM_NCDESTROY, where we clear GWLP_USERDATA. This reference never escapes a callback.
    unsafe {
        if message == WM_NCCREATE {
            let create = &*(lparam.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const SubtitleShell
    }
}
unsafe extern "system" fn main_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: Win32 owns callback args. Catch panic so no Rust unwind crosses the ABI.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        main_message(hwnd, message, wparam, lparam)
    }))
    .unwrap_or_else(|_| {
        // SAFETY: GWLP_USERDATA follows the same callback-scoped Box protocol.
        unsafe {
            if let Some(shell) = state(hwnd, 0, LPARAM(0)).as_ref() {
                shell.fail();
            }
        }
        LRESULT(0)
    })
}
unsafe fn main_message(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: state() validates the app-owned pointer protocol; all calls run on UI thread.
    unsafe {
        if let Some(shell) = state(hwnd, message, lparam).as_ref() {
            if let Some(result) = shell.theme_message(hwnd, message, wparam, lparam) {
                return result;
            }
            match message {
                WM_MEASUREITEM if lparam.0 != 0 => {
                    // SAFETY: synchronous Win32-owned out structure, retained only for callback.
                    let measure = &mut *(lparam.0 as *mut MEASUREITEMSTRUCT);
                    if measure.CtlID == subtitle_controls::SOURCE as u32
                        && measure.CtlType == ODT_COMBOBOX
                    {
                        measure.itemHeight =
                            scale(subtitle_controls::SOURCE_ROW_DIP, GetDpiForWindow(hwnd)) as u32;
                        return LRESULT(1);
                    }
                }
                WM_DRAWITEM if lparam.0 != 0 => {
                    // SAFETY: callback-scoped Win32 draw structure and borrowed destination DC.
                    let draw = &*(lparam.0 as *const DRAWITEMSTRUCT);
                    if draw.CtlID == subtitle_controls::SOURCE as u32
                        && draw.CtlType == ODT_COMBOBOX
                    {
                        let latest = shell.control_update.borrow().sources.clone();
                        if let Some(panel) = shell.panel.borrow().as_ref()
                            && panel.draw_source(draw, &latest).is_err()
                        {
                            shell.fail();
                        }
                        return LRESULT(1);
                    }
                }
                WM_TIMER if wparam.0 == UPDATE_TIMER => {
                    if shell.poll_controller().is_err() {
                        shell.fail();
                        return LRESULT(0);
                    }
                    if shell.poll_inbox().is_err() {
                        shell.fail();
                    }
                    if shell.poll_follow().is_err() {
                        shell.fail();
                    }
                    return LRESULT(0);
                }
                WM_COMMAND => {
                    let id = wparam.0 & 0xffff;
                    if id == subtitle_controls::PREPARED_MEDIA {
                        // SAFETY: only this owned main window is inspected; a
                        // nested modal owner must not open a second Prepared run.
                        if !shell.control_update.borrow().busy
                            && windows::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled(
                                shell.main.get(),
                            )
                            .as_bool()
                        {
                            let result = (|| {
                                let instance = HINSTANCE(
                                    GetModuleHandleW(None)
                                        .map_err(|_| AppError::SubtitleWindow)?
                                        .0,
                                );
                                // End this borrow before the nested dialog loop:
                                // parent timer/shutdown callbacks may borrow panel.
                                let button = shell
                                    .panel
                                    .borrow()
                                    .as_ref()
                                    .map(|panel| panel.prepared_media)
                                    .ok_or(AppError::SubtitleWindow)?;
                                // SAFETY: validate our owned child before opening
                                // a modal dialog for this UI-thread owner only.
                                if GetParent(button).ok() != Some(shell.main.get()) {
                                    return Err(AppError::SubtitleWindow);
                                }
                                if let Some(attachment) =
                                    super::prepared_dialog::show(shell.main.get(), instance)?
                                {
                                    *shell.prepared.borrow_mut() = Some(attachment);
                                    *shell.notice.borrow_mut() = Some("已保留本次準備字幕。請在所選程式手動播放，按啟動後以語音重新定位；不會自動播放。".into());
                                }
                                Ok(())
                            })();
                            if let Err(error) = result {
                                *shell.notice.borrow_mut() = Some(error.to_string());
                            }
                            if shell.refresh(shell.visible.get()).is_err() {
                                shell.fail();
                            }
                        }
                        return LRESULT(0);
                    }
                    if id == subtitle_controls::SOURCE_SUBTITLE {
                        if !shell.control_update.borrow().busy {
                            let result = (|| {
                                // SAFETY: current executable module is borrowed,
                                // not owned; the dialog creates only UI-thread HWNDs.
                                let instance = HINSTANCE(
                                    GetModuleHandleW(None)
                                        .map_err(|_| AppError::SubtitleWindow)?
                                        .0,
                                );
                                if let Some(panel) = shell.panel.borrow().as_ref() {
                                    panel.edit_subtitle_source(shell.main.get(), instance)?;
                                }
                                Ok::<_, AppError>(())
                            })();
                            if let Err(error) = result {
                                *shell.notice.borrow_mut() = Some(error.to_string());
                            }
                            if shell.refresh(shell.visible.get()).is_err() {
                                shell.fail();
                            }
                        }
                        return LRESULT(0);
                    }
                    if [
                        subtitle_controls::SAVE_OPENAI_KEY,
                        subtitle_controls::SAVE_GEMINI_KEY,
                    ]
                    .contains(&id)
                    {
                        let provider = if id == subtitle_controls::SAVE_OPENAI_KEY {
                            crate::ai::provider::SpeechProvider::OpenAi
                        } else {
                            crate::ai::provider::SpeechProvider::Gemini
                        };
                        if shell.save_key_from_controls(provider).is_err() {
                            shell.fail();
                        }
                        return LRESULT(0);
                    }
                    if [
                        subtitle_controls::CHECK_OPENAI_KEY,
                        subtitle_controls::CHECK_GEMINI_KEY,
                    ]
                    .contains(&id)
                    {
                        let provider = if id == subtitle_controls::CHECK_OPENAI_KEY {
                            crate::ai::provider::SpeechProvider::OpenAi
                        } else {
                            crate::ai::provider::SpeechProvider::Gemini
                        };
                        if shell.check_key_from_controls(provider).is_err() {
                            shell.fail();
                        }
                        return LRESULT(0);
                    }
                    if id == subtitle_controls::SAVE_PREFERENCES {
                        if shell.save_preferences_from_controls().is_err() {
                            shell.fail();
                        }
                        return LRESULT(0);
                    }
                    if id == subtitle_controls::START {
                        if shell.start_from_controls().is_err() {
                            shell.fail();
                        }
                        return LRESULT(0);
                    }
                    if id == subtitle_controls::STOP_BUTTON {
                        if shell.command(STOP).is_err() {
                            shell.fail();
                        }
                        return LRESULT(0);
                    }
                    if [
                        subtitle_controls::SOURCE,
                        subtitle_controls::PROVIDER,
                        subtitle_controls::LANGUAGE,
                        subtitle_controls::QUALITY,
                    ]
                    .contains(&id)
                        && (wparam.0 >> 16) & 0xffff == CBN_SELCHANGE as usize
                    {
                        let result = {
                            shell
                                .panel
                                .borrow()
                                .as_ref()
                                .map(|panel| panel.options_changed(id))
                        };
                        if result.is_some_and(|result| result.is_err())
                            || shell.refresh(shell.visible.get()).is_err()
                        {
                            shell.fail();
                        }
                        shell.last_follow_check.set(None);
                        return LRESULT(0);
                    }
                    if id == HISTORY && (wparam.0 >> 16) & 0xffff == EN_VSCROLL_VALUE {
                        // EDIT notifications also arise from reflow and programmatic
                        // scrolling. Only the input subclass may suspend auto-follow.
                        return LRESULT(0);
                    }
                    if id == SEARCH && (wparam.0 >> 16) & 0xffff == EN_CHANGE_VALUE {
                        if shell.syncing_search.get() {
                            return LRESULT(0);
                        }
                        let mut query = [0u16; 65];
                        let len = GetWindowTextW(shell.search.get(), &mut query).max(0) as usize;
                        shell
                            .model
                            .borrow_mut()
                            .set_search(&String::from_utf16_lossy(&query[..len]));
                        shell.history_projection.borrow_mut().take();
                        if shell.refresh(shell.visible.get()).is_err() {
                            shell.fail();
                        }
                    } else if shell.command(id).is_err() {
                        shell.fail();
                    }
                    return LRESULT(0);
                }
                WM_SIZE if !shell.history.get().0.is_null() => {
                    shell.sync_visibility();
                    if shell.layout().is_err() {
                        shell.fail();
                    }
                    return LRESULT(0);
                }
                WM_DISPLAYCHANGE => {
                    if shell
                        .rebuild_monitors()
                        .and_then(|_| shell.position_side())
                        .and_then(|_| shell.position_overlay())
                        .and_then(|_| shell.refresh(shell.visible.get()))
                        .is_err()
                    {
                        *shell.notice.borrow_mut() =
                            Some("無法更新螢幕清單，請重新整理字幕螢幕。".into());
                        let _ = shell.refresh(shell.visible.get());
                    }
                    return LRESULT(0);
                }
                WM_DPICHANGED => {
                    let rect = &*(lparam.0 as *const RECT);
                    if SetWindowPos(
                        hwnd,
                        None,
                        rect.left,
                        rect.top,
                        rect.right - rect.left,
                        rect.bottom - rect.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    )
                    .is_err()
                        || shell.update_font().is_err()
                        || shell.position_side().is_err()
                    {
                        shell.fail();
                    }
                    return LRESULT(0);
                }
                WM_CLOSE => {
                    if let Some(panel) = shell.panel.borrow().as_ref() {
                        let _ = panel.clear_keys();
                        let _ = panel.clear_subtitle_source();
                    }
                }
                WM_DESTROY => {
                    let _ = KillTimer(Some(hwnd), UPDATE_TIMER);
                    if let Some(inbox) = shell.inbox.borrow().as_ref() {
                        inbox.cancellation().cancel();
                    }
                    if let Some(controller) = shell.controller.borrow_mut().as_mut() {
                        controller.request_stop();
                    }
                    if shell.quit_on_destroy {
                        PostQuitMessage(0);
                    }
                    return LRESULT(0);
                }
                WM_EXITSIZEMOVE => {
                    if shell.position_side().is_err() {
                        shell.fail();
                    }
                    return LRESULT(0);
                }
                WM_NCDESTROY => {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                }
                _ => {}
            }
        }
        DefWindowProcW(hwnd, message, wparam, lparam)
    }
}
unsafe extern "system" fn history_input_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    subclass: usize,
    reference: usize,
) -> LRESULT {
    // SAFETY: SetWindowSubclass receives our stable Box address. It remains valid
    // until synchronous parent/child destruction completes, on this same UI thread.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        if message == WM_NCDESTROY {
            let _ = RemoveWindowSubclass(hwnd, Some(history_input_proc), subclass);
        } else if reference != 0 {
            let shell = &*(reference as *const SubtitleShell);
            let navigation = message == WM_KEYDOWN
                && (usize::from(VK_PRIOR.0)..=usize::from(VK_DOWN.0)).contains(&wparam.0);
            let manual = navigation
                || matches!(
                    message,
                    WM_MOUSEWHEEL | WM_VSCROLL | WM_LBUTTONDOWN | WM_NCLBUTTONDOWN
                );
            if manual && !shell.refreshing.get() {
                shell.model.borrow_mut().auto_follow = false;
                CheckMenuItem(GetMenu(shell.main.get()), FOLLOW as u32, MF_UNCHECKED.0);
            }
        }
    }));
    if result.is_err() && reference != 0 {
        // SAFETY: same Box lifetime as above; Cell does not require a model borrow.
        unsafe {
            (&*(reference as *const SubtitleShell)).failed.set(true);
        }
    }
    // SAFETY: delegate exactly once to the installed EDIT subclass chain. Never
    // unwind across the callback ABI, and do not retain borrowed message pointers.
    let native_result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
    // Reconcile only after EDIT has performed the actual user scroll/selection.
    // NCDESTROY never dereferences the reference after delegating. The stable
    // shell Box outlives these callbacks and all synchronous child destruction.
    if reference != 0
        && (matches!(
            message,
            WM_VSCROLL | WM_MOUSEWHEEL | WM_LBUTTONUP | WM_NCLBUTTONUP
        ) || message == WM_KEYDOWN
            && (usize::from(VK_PRIOR.0)..=usize::from(VK_DOWN.0)).contains(&wparam.0))
    {
        let resumed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
            (&*(reference as *const SubtitleShell)).resume_history_follow_at_bottom();
        }));
        if resumed.is_err() {
            // SAFETY: same callback-scoped Box lifetime; no borrowed model escapes.
            unsafe {
                (&*(reference as *const SubtitleShell)).failed.set(true);
            }
        }
    }
    native_result
}
unsafe extern "system" fn overlay_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: catch unwinds before crossing Win32's callback ABI.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        overlay_message(hwnd, message, wparam, lparam)
    }))
    .unwrap_or_else(|_| {
        // SAFETY: GWLP_USERDATA follows the same callback-scoped Box protocol.
        unsafe {
            if let Some(shell) = state(hwnd, 0, LPARAM(0)).as_ref() {
                shell.fail();
            }
        }
        LRESULT(0)
    })
}
unsafe fn overlay_message(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: callback state belongs to the same UI thread; temporary paint/font resources
    // are restored and released before returning. No data pointer crosses threads.
    unsafe {
        if let Some(shell) = state(hwnd, message, lparam).as_ref() {
            match message {
                WM_NCHITTEST => {
                    return if shell.through.get() {
                        LRESULT(HTTRANSPARENT as isize)
                    } else if shell.locked.get() {
                        LRESULT(HTCLIENT as isize)
                    } else {
                        LRESULT(HTCAPTION as isize)
                    };
                }
                WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
                WM_DPICHANGED => {
                    let rect = &*(lparam.0 as *const RECT);
                    if SetWindowPos(
                        hwnd,
                        None,
                        rect.left,
                        rect.top,
                        rect.right - rect.left,
                        rect.bottom - rect.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    )
                    .is_err()
                    {
                        shell.fail();
                    }
                    return LRESULT(0);
                }
                WM_PAINT => {
                    let mut paint = PAINTSTRUCT::default();
                    BeginPaint(hwnd, &mut paint);
                    let result = shell.render_overlay();
                    let _ = EndPaint(hwnd, &paint);
                    if result.is_err() {
                        shell.fail();
                    }
                    return LRESULT(0);
                }
                WM_SIZE if !shell.overlay.get().0.is_null() => {
                    if shell.render_overlay().is_err() {
                        shell.fail();
                    }
                    return LRESULT(0);
                }
                WM_NCDESTROY => {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                }
                _ => {}
            }
        }
        DefWindowProcW(hwnd, message, wparam, lparam)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::provider::SpeechProvider;
    struct MockController {
        state: Rc<RefCell<MockControlState>>,
        publisher: Option<crate::ui::bridge::UiPublisher>,
    }
    pub(super) struct MockControlState {
        pub(super) prepared_starts: usize,
        update: ControlUpdate,
        pub(super) calls: Vec<(
            u32,
            crate::ai::provider::SpeechProvider,
            crate::domain::SourceLanguage,
            crate::domain::TranslationQuality,
            bool,
        )>,
        pub(super) saved: Vec<crate::ai::provider::SpeechProvider>,
        pub(super) preferences_saved: Vec<ControlPreferences>,
        pub(super) save_error: Option<crate::ai::credentials::CredentialError>,
        pub(super) checked: Vec<crate::ai::provider::SpeechProvider>,
        pub(super) check_error: Option<crate::ai::credentials::CredentialError>,
    }
    impl UiController for MockController {
        fn save_preferences(&mut self, preferences: ControlPreferences) -> Result<()> {
            self.state.borrow_mut().preferences_saved.push(preferences);
            Ok(())
        }
        fn start_prepared(
            &mut self,
            request: crate::ui::controls::StartRequest,
            key: Option<crate::ai::credentials::ApiKey>,
            _source: crate::subtitle::fuzzy_worker::AlignmentSource,
        ) -> Result<UiInbox> {
            self.state.borrow_mut().prepared_starts += 1;
            self.start(request, key)
        }
        fn poll(&mut self) -> ControlUpdate {
            self.state.borrow().update.clone()
        }
        fn start(
            &mut self,
            request: crate::ui::controls::StartRequest,
            key: Option<crate::ai::credentials::ApiKey>,
        ) -> Result<UiInbox> {
            if self.state.borrow().update.busy {
                return Err(crate::ui::bridge::UiBridgeError::InvalidState.into());
            }
            self.state.borrow_mut().calls.push((
                request.source.pid,
                request.provider,
                request.language,
                request.quality,
                key.is_some(),
            ));
            self.state.borrow_mut().update.busy = true;
            let (mut publisher, inbox) = crate::ui::bridge::channel();
            publisher.prepare_source(crate::ui::bridge::UiSource {
                pid: request.source.pid,
                provider: request.provider,
            })?;
            self.publisher = Some(publisher);
            Ok(inbox)
        }
        fn save_key(
            &mut self,
            provider: crate::ai::provider::SpeechProvider,
            _key: crate::ai::credentials::ApiKey,
        ) -> Result<()> {
            let mut state = self.state.borrow_mut();
            if let Some(error) = state.save_error {
                return Err(error.into());
            }
            state.saved.push(provider);
            Ok(())
        }
        fn check_key(&mut self, provider: crate::ai::provider::SpeechProvider) -> Result<()> {
            let mut state = self.state.borrow_mut();
            state.checked.push(provider);
            match state.check_error {
                Some(error) => Err(error.into()),
                None => Ok(()),
            }
        }
        fn request_stop(&mut self) {
            self.state.borrow_mut().update.busy = false;
            if let Some(mut publisher) = self.publisher.take() {
                let _ = publisher.finish(UiStatus::Stopped);
            }
        }
    }
    pub(super) fn controlled_mock() -> (Box<SubtitleShell>, Rc<RefCell<MockControlState>>) {
        controlled_mock_with_preferences(None)
    }
    fn controlled_mock_with_preferences(
        preferences: Option<ControlPreferences>,
    ) -> (Box<SubtitleShell>, Rc<RefCell<MockControlState>>) {
        let source = crate::ui::controls::mock_source(42);
        let state = Rc::new(RefCell::new(MockControlState {
            update: ControlUpdate {
                sources: Arc::new(crate::ui::controls::choices(
                    &crate::audio::sessions::ScanReport {
                        sources: vec![source],
                        ..Default::default()
                    },
                )),
                busy: false,
                notice: None,
            },
            calls: Vec::new(),
            prepared_starts: 0,
            saved: Vec::new(),
            preferences_saved: Vec::new(),
            save_error: None,
            checked: Vec::new(),
            check_error: None,
        }));
        let shell = SubtitleShell::allocate(DisplayMode::Dual, false);
        if let Some(preferences) = preferences {
            shell.preferences.set(preferences);
        }
        *shell.controller.borrow_mut() = Some(Box::new(MockController {
            state: state.clone(),
            publisher: None,
        }));
        shell.create(false).unwrap();
        shell.command(SETTINGS).unwrap();
        shell.poll_controller().unwrap();
        (shell, state)
    }
    #[test]
    fn phase11_auto_language_preflight_blocks_invalid_models_before_keys_or_controller() {
        use crate::{ai::provider::LiveSpeechModel, domain::SourceLanguage};
        use windows::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled;
        let (shell, state) = controlled_mock();
        shell.command(SETTINGS).unwrap();
        let (provider, language, start, gemini_key, openai_key) = {
            let panel = shell.panel.borrow();
            let panel = panel.as_ref().unwrap();
            // SAFETY: scalar selections and synthetic masked inputs on fixture-owned controls.
            unsafe {
                SendMessageW(panel.source, CB_SETCURSEL, Some(WPARAM(1)), None);
                SendMessageW(panel.provider, CB_SETCURSEL, Some(WPARAM(3)), None);
                SendMessageW(panel.language, CB_SETCURSEL, Some(WPARAM(0)), None);
                SetWindowTextW(panel.gemini_key, w!("SYNTHETIC_GEMINI")).unwrap();
                SetWindowTextW(panel.openai_key, w!("SYNTHETIC_OPENAI")).unwrap();
            }
            panel.options_changed(subtitle_controls::PROVIDER).unwrap();
            (
                panel.provider,
                panel.language,
                panel.start,
                panel.gemini_key,
                panel.openai_key,
            )
        };
        shell.refresh(false).unwrap();
        assert_eq!(control_text(language), "請手選語言");
        assert!(control_text(shell.status.get()).starts_with("請先手選語言"));
        assert!(control_text(shell.status.get()).contains("Gemini · Transcribe Live"));
        assert!(!unsafe { IsWindowEnabled(start) }.as_bool());
        shell.start_from_controls().unwrap(); // posted commands cannot bypass the gate
        assert!(state.borrow().calls.is_empty());
        assert_eq!(control_text(gemini_key), "SYNTHETIC_GEMINI");
        assert_eq!(control_text(openai_key), "SYNTHETIC_OPENAI");
        for (index, expected) in [
            (1, SourceLanguage::English),
            (2, SourceLanguage::Japanese),
            (3, SourceLanguage::French),
            (4, SourceLanguage::German),
            (5, SourceLanguage::Korean),
        ] {
            let panel = shell.panel.borrow();
            let panel = panel.as_ref().unwrap();
            unsafe {
                SendMessageW(language, CB_SETCURSEL, Some(WPARAM(index)), None);
            }
            panel.options_changed(subtitle_controls::LANGUAGE).unwrap();
            let request = panel.request().unwrap();
            assert_eq!(request.language, expected);
            assert_eq!(
                request.live_model,
                Some(LiveSpeechModel::GeminiLiveTranslate)
            );
            assert!(panel.live_selection_notice().is_none());
            assert!(unsafe { IsWindowEnabled(start) }.as_bool());
        }
        {
            let panel = shell.panel.borrow();
            let panel = panel.as_ref().unwrap();
            // SAFETY: model change must preserve the previously selected Korean index.
            unsafe {
                SendMessageW(provider, CB_SETCURSEL, Some(WPARAM(2)), None);
            }
            panel.options_changed(subtitle_controls::PROVIDER).unwrap();
            assert_eq!(
                panel.preferences().unwrap().language,
                SourceLanguage::Korean
            );
            assert_eq!(
                unsafe { SendMessageW(language, CB_GETCOUNT, None, None) }.0,
                6
            );
        }
        for (index, expected_enabled, label) in [
            (2, true, "自動偵測"),
            (1, false, "請手選語言"),
            (3, false, "請手選語言"),
        ] {
            let panel = shell.panel.borrow();
            let panel = panel.as_ref().unwrap();
            unsafe {
                SendMessageW(provider, CB_SETCURSEL, Some(WPARAM(index)), None);
                SendMessageW(language, CB_SETCURSEL, Some(WPARAM(0)), None);
            }
            panel.options_changed(subtitle_controls::PROVIDER).unwrap();
            assert_eq!(control_text(language), label);
            assert_eq!(panel.preferences().unwrap().language, SourceLanguage::Auto);
            assert_eq!(
                unsafe { IsWindowEnabled(start) }.as_bool(),
                expected_enabled
            );
            assert_eq!(panel.request().is_ok(), expected_enabled);
            if index == 2 {
                assert!(
                    panel
                        .live_selection_notice()
                        .unwrap()
                        .contains("在本機依辨識文字估計")
                );
            }
        }
        assert_eq!(control_text(gemini_key), "SYNTHETIC_GEMINI");
        assert!(state.borrow().saved.is_empty() && state.borrow().calls.is_empty());
    }
    #[test]
    fn phase11_saved_native_auto_starts_as_an_unselected_language_without_rewriting_preferences() {
        use crate::{
            ai::provider::LiveSpeechModel,
            domain::{SourceLanguage, TranslationQuality},
        };
        let preferences = ControlPreferences {
            provider: Some(SpeechProvider::Gemini),
            live_model: Some(LiveSpeechModel::GeminiLiveTranslate),
            language: SourceLanguage::Auto,
            quality: TranslationQuality::Standard,
        };
        let (shell, state) = controlled_mock_with_preferences(Some(preferences));
        let panel = shell.panel.borrow();
        let panel = panel.as_ref().unwrap();
        assert!(panel.preferences().unwrap() == preferences);
        assert_eq!(control_text(panel.language), "請手選語言");
        assert!(panel.live_selection_notice().is_some());
        assert!(state.borrow().preferences_saved.is_empty() && state.borrow().calls.is_empty());
        assert!(!shell.failed.get());
    }
    #[test]
    fn phase11_source_language_are_visible_and_status_is_only_on_the_main_page() {
        let (shell, state) = controlled_mock();
        for command in [CREDENTIAL_SETTINGS, CREDENTIAL_SETTINGS, SETTINGS, SETTINGS] {
            shell.command(command).unwrap();
            let panel = shell.panel.borrow();
            let panel = panel.as_ref().unwrap();
            // SAFETY: same-thread fixture-owned child handles, no user UI or key reads.
            unsafe {
                for control in [
                    panel.source,
                    panel.language,
                    panel.quality,
                    panel.start,
                    panel.stop,
                    shell.clear_button.get(),
                ] {
                    assert_ne!(
                        GetWindowLongPtrW(control, GWL_STYLE) as u32 & WS_VISIBLE.0,
                        0
                    );
                }
                for control in [shell.status.get(), shell.status_label.get()] {
                    assert_eq!(
                        GetWindowLongPtrW(control, GWL_STYLE) as u32 & WS_VISIBLE.0 != 0,
                        shell.page.get() == ControlPage::Subtitles
                    );
                }
            }
        }
        assert!(state.borrow().calls.is_empty());
    }
    #[test]
    fn phase11_live_model_selection_round_trips_and_diagnostic_returns_to_main() {
        use crate::ai::provider::LiveSpeechModel;
        let (shell, state) = controlled_mock();
        shell.command(CREDENTIAL_SETTINGS).unwrap();
        for (index, model) in [
            (1, LiveSpeechModel::GptLiveTranscribe),
            (2, LiveSpeechModel::GeminiTranscribeLive),
            (3, LiveSpeechModel::GeminiLiveTranslate),
        ] {
            let panel = shell.panel.borrow();
            let panel = panel.as_ref().unwrap();
            // SAFETY: synthetic selections on this hidden fixture's own UI thread.
            unsafe {
                SendMessageW(panel.source, CB_SETCURSEL, Some(WPARAM(1)), None);
                SendMessageW(panel.provider, CB_SETCURSEL, Some(WPARAM(index)), None);
                SendMessageW(panel.language, CB_SETCURSEL, Some(WPARAM(1)), None);
            }
            assert_eq!(panel.preferences().unwrap().live_model, Some(model));
            assert_eq!(panel.request().unwrap().live_model, Some(model));
        }
        assert!(shell.page.get() == ControlPage::Credentials);
        assert!(state.borrow().calls.is_empty() && state.borrow().saved.is_empty());
    }
    #[test]
    fn phase11_independent_credentials_page_saves_selection_and_keeps_main_actions_at_original_location()
     {
        use crate::domain::{SourceLanguage, TranslationQuality};
        let (shell, state) = controlled_mock();
        shell.command(SETTINGS).unwrap();
        let work = subtitle_monitor::window_work_area(shell.main.get()).unwrap();
        // SAFETY: moves our own hidden fixture only; no user's running app is touched.
        unsafe {
            SetWindowPos(
                shell.main.get(),
                None,
                work.left,
                work.top,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
            .unwrap();
        }
        shell.command(SETTINGS).unwrap();
        let mut rect = RECT::default();
        unsafe {
            GetWindowRect(shell.main.get(), &mut rect).unwrap();
        }
        assert_eq!((rect.left, rect.top), (work.left, work.top));
        assert!(shell.page.get() == ControlPage::Sources);
        let handles = {
            let panel = shell.panel.borrow();
            let panel = panel.as_ref().unwrap();
            [
                panel.source,
                panel.provider,
                panel.openai_key,
                panel.start,
                panel.stop,
                panel.save_preferences,
            ]
        };
        fn visible(control: HWND) -> bool {
            // SAFETY: fixture-owned child, queried only on its owning thread.
            unsafe { GetWindowLongPtrW(control, GWL_STYLE) as u32 & WS_VISIBLE.0 != 0 }
        }
        assert!(visible(handles[0]));
        assert!(!visible(handles[1]) && !visible(handles[2]));
        shell.command(CREDENTIAL_SETTINGS).unwrap();
        assert!(shell.page.get() == ControlPage::Credentials);
        assert!(visible(handles[0]));
        assert!(visible(handles[1]) && visible(handles[2]) && visible(handles[5]));
        // SAFETY: synthetic selections and a synthetic masked field in our hidden fixture.
        unsafe {
            SendMessageW(handles[1], CB_SETCURSEL, Some(WPARAM(2)), None);
            let panel = shell.panel.borrow();
            let panel = panel.as_ref().unwrap();
            SendMessageW(panel.language, CB_SETCURSEL, Some(WPARAM(1)), None);
            SendMessageW(panel.quality, CB_SETCURSEL, Some(WPARAM(0)), None);
            SetWindowTextW(handles[2], w!("synthetic-unsaved-key")).unwrap();
        }
        unsafe {
            SendMessageW(handles[5], BM_CLICK, None, None);
        }
        let preferences = state.borrow().preferences_saved[0];
        assert!(preferences.provider == Some(SpeechProvider::Gemini));
        assert!(preferences.language == SourceLanguage::English);
        assert!(preferences.quality == TranslationQuality::Fast);
        assert!(state.borrow().saved.is_empty() && state.borrow().calls.is_empty());
        assert_eq!(unsafe { GetWindowTextLengthW(handles[2]) }, 21);
        shell.command(CREDENTIAL_SETTINGS).unwrap();
        assert!(shell.page.get() == ControlPage::Subtitles);
        assert!(visible(handles[3]) && visible(handles[4]) && visible(shell.clear_button.get()));
        assert!(visible(handles[0]) && !visible(handles[1]) && !visible(handles[2]));
        unsafe {
            GetWindowRect(shell.main.get(), &mut rect).unwrap();
        }
        assert_eq!((rect.left, rect.top), (work.left, work.top));
        assert!(control_text(shell.status.get()).contains("已儲存服務"));
        assert!(!shell.failed.get());
    }
    #[test]
    fn native_controls_require_selection_clear_key_lock_and_restart() {
        use crate::{
            ai::provider::SpeechProvider,
            domain::{SourceLanguage, TranslationQuality},
        };
        use windows::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled;
        let (shell, state) = controlled_mock();
        {
            let panel = shell.panel.borrow();
            let panel = panel.as_ref().unwrap();
            // SAFETY: this test owns all controls on this thread, no external UI automation.
            unsafe {
                assert!(!IsWindowEnabled(panel.start).as_bool());
                assert_eq!(SendMessageW(panel.source, CB_GETCURSEL, None, None).0, 0);
                SendMessageW(panel.source, CB_SETCURSEL, Some(WPARAM(1)), None);
                assert!(panel.request().is_err());
                SendMessageW(panel.provider, CB_SETCURSEL, Some(WPARAM(2)), None);
                SendMessageW(panel.language, CB_SETCURSEL, Some(WPARAM(4)), None);
                SendMessageW(panel.quality, CB_SETCURSEL, Some(WPARAM(2)), None);
                SetWindowTextW(panel.gemini_key, w!("synthetic-test-key")).unwrap();
                assert!(
                    GetWindowLongPtrW(panel.gemini_key, GWL_STYLE) as u32 & ES_PASSWORD as u32 != 0
                );
            }
        }
        shell.start_from_controls().unwrap();
        assert_eq!(
            state.borrow().calls[0],
            (
                42,
                SpeechProvider::Gemini,
                SourceLanguage::German,
                TranslationQuality::High,
                true
            )
        );
        {
            let panel = shell.panel.borrow();
            let panel = panel.as_ref().unwrap();
            assert!(control_text(panel.gemini_key).is_empty());
            // SAFETY: own handles; controls are disabled while one backend is active.
            unsafe {
                assert!(!IsWindowEnabled(panel.source).as_bool());
                assert!(!IsWindowEnabled(panel.provider).as_bool());
            }
        }
        shell.start_from_controls().unwrap();
        assert_eq!(state.borrow().calls.len(), 1);
        shell.command(STOP).unwrap();
        shell.poll_controller().unwrap();
        shell.poll_inbox().unwrap();
        {
            let panel = shell.panel.borrow();
            let panel = panel.as_ref().unwrap();
            // SAFETY: changes only our owned combo; simulated user picks new provider.
            unsafe {
                SetWindowTextW(panel.gemini_key, w!("synthetic-key-to-clear")).unwrap();
                SendMessageW(panel.provider, CB_SETCURSEL, Some(WPARAM(1)), None);
            }
            panel.options_changed(subtitle_controls::PROVIDER).unwrap();
            assert_eq!(control_text(panel.gemini_key), "synthetic-key-to-clear");
            assert!(control_text(panel.openai_key).is_empty());
        }
        shell.start_from_controls().unwrap();
        assert_eq!(state.borrow().calls.len(), 2);
        assert_eq!(state.borrow().calls[1].1, SpeechProvider::OpenAi);
        assert!(!state.borrow().calls[1].4);
    }
    #[test]
    fn native_source_removal_or_reused_pid_never_selects_another_source() {
        let (shell, state) = controlled_mock();
        {
            let panel = shell.panel.borrow();
            let panel = panel.as_ref().unwrap();
            // SAFETY: this test owns the source combo on its UI thread.
            unsafe {
                SendMessageW(panel.source, CB_SETCURSEL, Some(WPARAM(1)), None);
            }
        }
        let mut replacement = crate::ui::controls::mock_source(42);
        replacement.process_created = Some(200);
        state.borrow_mut().update.sources = Arc::new(crate::ui::controls::choices(
            &crate::audio::sessions::ScanReport {
                sources: vec![replacement],
                ..Default::default()
            },
        ));
        shell.poll_controller().unwrap();
        {
            let panel = shell.panel.borrow();
            assert!(panel.as_ref().unwrap().request().is_err());
        }
        assert!(state.borrow().calls.is_empty());
        state.borrow_mut().update.sources = Arc::new(Vec::new());
        shell.poll_controller().unwrap();
        assert!(state.borrow().calls.is_empty());
    }
    #[test]
    fn native_prepared_entry_respects_live_busy_and_modal_startup_gate() {
        use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, IsWindowEnabled};
        let (shell, state) = controlled_mock();
        let prepared = {
            let panel = shell.panel.borrow();
            let panel = panel.as_ref().unwrap();
            // SAFETY: fixture owns these same-thread selectors and main window.
            unsafe {
                SendMessageW(panel.source, CB_SETCURSEL, Some(WPARAM(1)), None);
                SendMessageW(panel.provider, CB_SETCURSEL, Some(WPARAM(1)), None);
                assert!(IsWindowEnabled(panel.prepared_media).as_bool());
                let _ = EnableWindow(shell.main.get(), false);
            }
            panel.prepared_media
        };
        // SAFETY: only the fixture's own parent receives simulated commands.
        // Disabled modal owners cannot start Live or open another Prepared run.
        unsafe {
            SendMessageW(
                shell.main.get(),
                WM_COMMAND,
                Some(WPARAM(subtitle_controls::START)),
                None,
            );
            SendMessageW(
                shell.main.get(),
                WM_COMMAND,
                Some(WPARAM(subtitle_controls::PREPARED_MEDIA)),
                None,
            );
            let _ = EnableWindow(shell.main.get(), true);
        }
        assert!(state.borrow().calls.is_empty());
        state.borrow_mut().update.busy = true;
        shell.poll_controller().unwrap();
        // SAFETY: same owned button/state, without opening any dialog or service.
        unsafe {
            assert!(!IsWindowEnabled(prepared).as_bool());
            SendMessageW(
                shell.main.get(),
                WM_COMMAND,
                Some(WPARAM(subtitle_controls::PREPARED_MEDIA)),
                None,
            );
        }
        state.borrow_mut().update.busy = false;
        shell.poll_controller().unwrap();
        // SAFETY: scalar visibility of fixture's button only.
        assert!(unsafe { IsWindowEnabled(prepared) }.as_bool());
        assert!(state.borrow().calls.is_empty());
        assert!(!shell.failed.get());
    }
    #[test]
    fn native_panel_geometry_keeps_actions_above_search_and_key_undo_empty() {
        const EM_CANUNDO_VALUE: u32 = 0x00c6;
        let (shell, _state) = controlled_mock();
        let panel = shell.panel.borrow();
        let panel = panel.as_ref().unwrap();
        // SAFETY: rectangles and messages reference only controls owned by this test.
        unsafe {
            let mut start = RECT::default();
            let mut subtitle = RECT::default();
            let mut prepared = RECT::default();
            let mut search = RECT::default();
            GetWindowRect(panel.start, &mut start).unwrap();
            GetWindowRect(panel.subtitle_source, &mut subtitle).unwrap();
            GetWindowRect(panel.prepared_media, &mut prepared).unwrap();
            GetWindowRect(shell.search.get(), &mut search).unwrap();
            assert!(start.bottom <= search.top);
            assert!(start.left == search.left);
            assert!(subtitle.bottom <= prepared.top && prepared.bottom <= start.top);
            assert_eq!(prepared.left, search.left);
            SetWindowTextW(panel.openai_key, w!("synthetic-test-key")).unwrap();
            panel.take_key(SpeechProvider::OpenAi).unwrap();
            assert_eq!(
                SendMessageW(panel.openai_key, EM_CANUNDO_VALUE, None, None).0,
                0
            );
        }
    }
    pub(super) fn fixture_timeline(
        id: u64,
        text: &str,
    ) -> crate::subtitle::timeline::TranscriptTimeline {
        use crate::{
            ai::{
                speech::{StreamIdentity, TranscriptEvent, TranscriptKind},
                translation::{TranslationPass, TranslationUpdate},
            },
            audio::capture::SessionId,
            subtitle::timeline::{Stability, TranscriptTimeline},
        };
        let identity = StreamIdentity {
            session: SessionId(id),
            generation: 1,
        };
        let mut timeline = TranscriptTimeline::new(identity);
        timeline
            .ingest(TranscriptEvent {
                identity,
                sequence: 1,
                utterance: 1,
                revision: 1,
                start_us: Some(0),
                duration_us: Some(100000),
                language: None,
                kind: TranscriptKind::Final,
                text: "test source".into(),
            })
            .unwrap();
        timeline
            .apply(&TranslationUpdate {
                job_id: 0,
                identity,
                utterance: 1,
                source_revision: 1,
                pass: TranslationPass::Draft,
                revision: 1,
                text: text.into(),
                stability: Stability::Final,
                stable_prefix_bytes: text.len(),
                complete: true,
            })
            .unwrap();
        timeline
    }
    pub(super) fn control_text(control: HWND) -> String {
        let mut text = vec![0u16; 2048];
        // SAFETY: the test owns the native control; stack buffer capacity is passed by slice.
        let len = unsafe { GetWindowTextW(control, &mut text) }.max(0) as usize;
        String::from_utf16_lossy(&text[..len])
    }
    #[test]
    fn hidden_view_does_not_open_gate_and_stop_rejects_late_frames() {
        use crate::{
            ai::provider::SpeechProvider,
            ui::bridge::{UiSource, UiStatus, channel},
        };
        let (mut publisher, inbox) = channel();
        publisher
            .prepare_source(UiSource {
                pid: 42,
                provider: SpeechProvider::OpenAi,
            })
            .unwrap();
        let shell = SubtitleShell::allocate(DisplayMode::Dual, false);
        *shell.inbox.borrow_mut() = Some(inbox);
        shell.create(false).unwrap();
        shell.poll_inbox().unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        assert!(
            runtime
                .block_on(async {
                    tokio::time::timeout(
                        std::time::Duration::from_millis(5),
                        publisher.wait_until_visible(),
                    )
                    .await
                })
                .is_err()
        );
        let timeline = fixture_timeline(8000, "停止前測試譯文");
        publisher
            .begin(
                timeline.identity(),
                UiSource {
                    pid: 42,
                    provider: SpeechProvider::OpenAi,
                },
            )
            .unwrap();
        publisher.publish(&timeline, true).unwrap();
        // The newest frame remains unread when stop is requested. It must not restore text.
        shell.command(STOP).unwrap();
        assert!(publisher.cancellation().is_cancelled());
        shell.poll_inbox().unwrap();
        assert!(shell.model.borrow().overlay_text().is_empty());
        assert!(control_text(shell.status.get()).contains("正在停止"));
        drop(publisher);
        shell.poll_inbox().unwrap();
        assert!(control_text(shell.status.get()).contains("已停止"));
        assert!(!control_text(shell.history.get()).contains("停止前測試譯文"));
        assert!(shell.frame.borrow().as_ref().unwrap().status == UiStatus::Stopped);
    }
    #[test]
    fn native_stop_joins_owned_backend_before_any_provider_start() {
        use crate::{
            ai::provider::SpeechProvider,
            app::ui_owner::UiWorker,
            ui::bridge::{UiSource, channel},
        };
        use std::sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        };
        struct MockResource(Arc<AtomicBool>);
        impl Drop for MockResource {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let (mut publisher, inbox) = channel();
        publisher
            .prepare_source(UiSource {
                pid: 42,
                provider: SpeechProvider::OpenAi,
            })
            .unwrap();
        let done = Arc::new(AtomicBool::new(false));
        let guard_done = done.clone();
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let mut worker = UiWorker::start(publisher, move |publisher| {
            let _resource = MockResource(guard_done);
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    publisher.wait_until_visible().await?;
                    counter.fetch_add(1, Ordering::SeqCst);
                    publisher.cancellation().cancelled().await;
                    Ok(())
                })
        })
        .unwrap();
        let shell = SubtitleShell::allocate(DisplayMode::Dual, false);
        *shell.inbox.borrow_mut() = Some(inbox);
        shell.create(false).unwrap();
        shell.poll_inbox().unwrap();
        assert!(control_text(shell.status.get()).contains("PID 42"));
        // SAFETY: invoke this test controller's actual pointer-free menu command path.
        unsafe {
            SendMessageW(shell.main.get(), WM_COMMAND, Some(WPARAM(STOP)), None);
        }
        worker.stop_join().unwrap();
        assert!(done.load(Ordering::SeqCst));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        shell.poll_inbox().unwrap();
        assert!(control_text(shell.status.get()).contains("已停止"));
        assert!(shell.model.borrow().overlay_text().is_empty());
    }
    #[test]
    fn native_timer_consumes_worker_snapshot_search_and_stop_atomically() {
        use crate::{
            ai::provider::SpeechProvider,
            ui::bridge::{UiSource, UiStatus, channel},
        };
        let (mut publisher, inbox) = channel();
        let cancel = publisher.cancellation();
        let shell = SubtitleShell::allocate(DisplayMode::Dual, false);
        *shell.inbox.borrow_mut() = Some(inbox);
        shell.create(false).unwrap();
        let timeline = fixture_timeline(7000, "測試視窗譯文");
        let worker = std::thread::spawn(move || {
            publisher
                .begin(
                    timeline.identity(),
                    UiSource {
                        pid: 42,
                        provider: SpeechProvider::Gemini,
                    },
                )
                .unwrap();
            publisher.publish(&timeline, true).unwrap();
            publisher
        });
        let mut publisher = worker.join().unwrap();
        // SAFETY: sends the same pointer-free WM_TIMER used by the native message loop,
        // to this test's controller on its owning thread. No screenshot automation.
        unsafe {
            SendMessageW(shell.main.get(), WM_TIMER, Some(WPARAM(UPDATE_TIMER)), None);
        }
        assert_eq!(shell.model.borrow().rows().count(), 1);
        assert_eq!(shell.model.borrow().overlay_text(), "測試視窗譯文");
        let text = control_text(shell.history.get());
        assert!(text.contains("測試視窗譯文"));
        assert!(control_text(shell.status.get()).contains("PID 42"));
        assert!(control_text(shell.status.get()).contains("Gemini"));
        assert!(!text.contains("PID 42"));
        assert!(!control_text(shell.status.get()).contains("測試視窗譯文"));
        *shell.notice.borrow_mut() = Some("系統測試錯誤".into());
        shell.refresh(false).unwrap();
        assert_eq!(control_text(shell.history.get()), text);
        assert!(control_text(shell.status.get()).contains("系統測試錯誤"));
        let (mut status_bounds, mut search_bounds) = (RECT::default(), RECT::default());
        // SAFETY: this test owns both child controls on the calling UI thread.
        unsafe {
            GetWindowRect(shell.status.get(), &mut status_bounds).unwrap();
            GetWindowRect(shell.search_label.get(), &mut search_bounds).unwrap();
        }
        assert!(status_bounds.bottom <= search_bounds.top);
        *shell.notice.borrow_mut() = None;
        // SAFETY: owned control copies the static search value; triggers real EN_CHANGE.
        unsafe {
            SetWindowTextW(shell.search.get(), w!("missing")).unwrap();
        }
        assert!(control_text(shell.history.get()).contains("找不到符合的字幕"));
        assert_eq!(shell.model.borrow().overlay_text(), "測試視窗譯文");
        shell.command(LIVE).unwrap();
        assert!(control_text(shell.search.get()).is_empty());
        assert!(control_text(shell.history.get()).contains("測試視窗譯文"));
        publisher.clear(UiStatus::Reconnecting).unwrap();
        assert!(shell.poll_inbox().unwrap());
        assert!(shell.model.borrow().overlay_text().contains("測試視窗譯文"));
        assert!(shell.model.borrow().overlay_text().contains("先前字幕"));
        assert!(control_text(shell.status.get()).contains("重新連線"));
        assert!(!control_text(shell.history.get()).contains("等待字幕"));
        let new = fixture_timeline(7001, "新 Session 測試譯文");
        publisher
            .begin(
                new.identity(),
                UiSource {
                    pid: 42,
                    provider: SpeechProvider::Gemini,
                },
            )
            .unwrap();
        publisher.publish(&new, true).unwrap();
        assert!(
            !publisher
                .publish(&fixture_timeline(7000, "舊 Session 譯文"), true)
                .unwrap()
        );
        shell.poll_inbox().unwrap();
        assert_eq!(shell.model.borrow().overlay_text(), "新 Session 測試譯文");
        publisher.clear(UiStatus::Error).unwrap();
        shell.poll_inbox().unwrap();
        assert!(control_text(shell.status.get()).contains("錯誤"));
        assert!(!control_text(shell.history.get()).contains("等待字幕"));
        assert!(
            shell
                .model
                .borrow()
                .overlay_text()
                .contains("新 Session 測試譯文")
        );
        publisher.clear(UiStatus::Stopped).unwrap();
        shell.poll_inbox().unwrap();
        assert!(control_text(shell.history.get()).contains("測試譯文"));
        assert!(control_text(shell.status.get()).contains("已停止"));
        shell.command(CLEAR_SUBTITLES).unwrap();
        assert!(!control_text(shell.history.get()).contains("測試譯文"));
        drop(shell);
        assert!(cancel.is_cancelled());
    }
    #[test]
    fn dpi_scaling_preserves_logical_sizes_at_supported_levels() {
        for (dpi, expected) in [(96, 100), (120, 125), (144, 150), (168, 175), (192, 200)] {
            assert_eq!(scale(100, dpi), expected);
        }
    }
    #[test]
    fn native_views_switch_without_backend_and_release_owned_windows() {
        let shell = SubtitleShell::allocate(DisplayMode::SideChat, false);
        shell.create(false).unwrap();
        let main = shell.main.get();
        let overlay = shell.overlay.get();
        for (command, mode) in [
            (DUAL, DisplayMode::Dual),
            (BOTTOM, DisplayMode::BottomSubtitle),
            (SIDE, DisplayMode::SideChat),
        ] {
            shell.command(command).unwrap();
            assert_eq!(shell.model.borrow().mode, mode);
        }
        shell.command(LOCK).unwrap();
        shell.command(THROUGH).unwrap();
        // SAFETY: test checks only its own live window handles and destroys them on this thread.
        unsafe {
            assert!(IsWindow(Some(main)).as_bool());
            assert!(IsWindow(Some(overlay)).as_bool());
            assert!(GetWindowLongPtrW(overlay, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0 != 0);
            assert!(GetWindowLongPtrW(overlay, GWL_EXSTYLE) as u32 & WS_EX_TRANSPARENT.0 != 0);
            assert_eq!(
                SendMessageW(overlay, WM_NCHITTEST, None, None).0,
                HTTRANSPARENT as isize
            );
        }
        shell.command(THROUGH).unwrap();
        // SAFETY: no pointer payload; this test owns the overlay on this thread.
        unsafe {
            assert_eq!(
                SendMessageW(overlay, WM_NCHITTEST, None, None).0,
                HTCLIENT as isize
            );
        }
        shell.command(LOCK).unwrap();
        // SAFETY: no pointer payload; validates the draggable overlay hit result.
        unsafe {
            assert_eq!(
                SendMessageW(overlay, WM_NCHITTEST, None, None).0,
                HTCAPTION as isize
            );
        }
        drop(shell);
        // SAFETY: IsWindow accepts expired handles; no reuse operation intervenes.
        unsafe {
            assert!(!IsWindow(Some(main)).as_bool());
            assert!(!IsWindow(Some(overlay)).as_bool());
        }
    }
    #[test]
    fn appearance_and_monitor_commands_preserve_session_and_layered_click_through() {
        use crate::ui::bridge::channel;
        let (publisher, inbox) = channel();
        let cancel = publisher.cancellation();
        let shell = SubtitleShell::allocate(DisplayMode::Dual, false);
        *shell.inbox.borrow_mut() = Some(inbox);
        shell.create(false).unwrap();
        let timeline = fixture_timeline(9000, "外觀測試翻譯");
        shell.model.borrow_mut().select_session(timeline.identity());
        shell
            .model
            .borrow_mut()
            .apply(crate::ui::presentation::TimelineSnapshot::from_timeline(
                &timeline, 1,
            ));
        for id in [
            FONT_FIRST + 2,
            SIZE_FIRST + 7,
            LINES_FIRST,
            OPACITY_FIRST,
            OUTLINE,
            SHADOW,
            ORIGINAL,
            THROUGH,
            THROUGH,
            MONITOR_FIRST,
            PRIMARY_MONITOR,
            REFRESH_MONITORS,
        ] {
            shell.command(id).unwrap();
            assert!(!shell.failed.get());
            assert!(!cancel.is_cancelled());
            assert_eq!(shell.model.borrow().identity(), Some(timeline.identity()));
            assert_eq!(shell.model.borrow().rows().count(), 1);
        }
        assert_eq!(shell.appearance.get().font, SubtitleFont::MingLiU);
        assert_eq!(shell.appearance.get().font_size, 72);
        assert_eq!(shell.appearance.get().max_lines, 1);
        assert_eq!(shell.appearance.get().background_percent, 0);
        assert!(
            shell
                .model
                .borrow()
                .overlay_text()
                .starts_with("外觀測試翻譯")
        );
        // SAFETY: own overlay; removing click-through must preserve per-pixel alpha style.
        unsafe {
            assert!(
                GetWindowLongPtrW(shell.overlay.get(), GWL_EXSTYLE) as u32 & WS_EX_LAYERED.0 != 0
            );
        }
        shell.command(RESET_APPEARANCE).unwrap();
        assert_eq!(shell.appearance.get(), SubtitleAppearance::default());
        assert!(!cancel.is_cancelled());
    }
    #[test]
    fn removed_monitor_reports_explicit_relocation_without_changing_preference() {
        let shell = SubtitleShell::allocate(DisplayMode::SideChat, false);
        shell.create(false).unwrap();
        // Explicit synthetic disconnected device, retaining real current monitor inventory.
        *shell.selected_monitor.borrow_mut() =
            MonitorPreference::Specific("test-disconnected-display".into());
        shell.position_overlay().unwrap();
        shell.refresh(false).unwrap();
        assert!(control_text(shell.status.get()).contains("選定螢幕已移除"));
        assert_eq!(
            *shell.selected_monitor.borrow(),
            MonitorPreference::Specific("test-disconnected-display".into())
        );
        shell.command(PRIMARY_MONITOR).unwrap();
        assert_eq!(*shell.selected_monitor.borrow(), MonitorPreference::Primary);
        assert!(!control_text(shell.status.get()).contains("選定螢幕已移除"));
    }
    #[test]
    fn effect_and_background_changes_preserve_manually_adjusted_position() {
        let shell = SubtitleShell::allocate(DisplayMode::Dual, false);
        shell.create(false).unwrap();
        let mut before = RECT::default();
        // SAFETY: move only this hidden test's own overlay; no external app manipulation.
        unsafe {
            SetWindowPos(
                shell.overlay.get(),
                None,
                30,
                40,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
            .unwrap();
            GetWindowRect(shell.overlay.get(), &mut before).unwrap();
        }
        for command in [OUTLINE, SHADOW, OPACITY_FIRST, FONT_FIRST + 1] {
            shell.command(command).unwrap();
            let mut after = RECT::default();
            // SAFETY: initialized out rectangle for own live window.
            unsafe {
                GetWindowRect(shell.overlay.get(), &mut after).unwrap();
            }
            assert_eq!(after, before);
        }
    }
    #[test]
    fn follow_failure_retains_position_and_recovery_preserves_session_without_repeated_moves() {
        use super::super::source_window::FollowError;
        let (publisher, inbox) = crate::ui::bridge::channel();
        let cancel = publisher.cancellation();
        let shell = SubtitleShell::allocate(DisplayMode::Dual, false);
        *shell.inbox.borrow_mut() = Some(inbox);
        shell.create(false).unwrap();
        let timeline = fixture_timeline(9100, "跟隨測試字幕");
        shell.model.borrow_mut().select_session(timeline.identity());
        shell
            .model
            .borrow_mut()
            .apply(crate::ui::presentation::TimelineSnapshot::from_timeline(
                &timeline, 1,
            ));
        let mut before = RECT::default();
        // SAFETY: initialized out rectangle for own window.
        unsafe {
            GetWindowRect(shell.overlay.get(), &mut before).unwrap();
        }
        shell.command(FOLLOW_SOURCE_MONITOR).unwrap();
        assert!(control_text(shell.status.get()).contains("請先選擇來源"));
        let mut after = RECT::default();
        // SAFETY: own window, no external UI operations.
        unsafe {
            GetWindowRect(shell.overlay.get(), &mut after).unwrap();
        }
        assert_eq!(after, before);
        let work = shell.monitors.borrow()[0].work;
        // Explicit mock query result; uses the real primary area for this hidden test view.
        assert!(shell.apply_follow_result(Ok(work), false).unwrap());
        shell.refresh(false).unwrap();
        assert!(!control_text(shell.status.get()).contains("請先選擇來源"));
        // Manually adjusted position stays when the next sample reports the same area.
        // SAFETY: changes and queries only this hidden test's owned overlay.
        unsafe {
            SetWindowPos(
                shell.overlay.get(),
                None,
                35,
                45,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
            .unwrap();
        }
        // SAFETY: initialized out rectangle for the same test-owned overlay.
        unsafe {
            GetWindowRect(shell.overlay.get(), &mut before).unwrap();
        }
        assert!(!shell.apply_follow_result(Ok(work), false).unwrap());
        // SAFETY: initialized out rectangle after simulated unchanged query result.
        unsafe {
            GetWindowRect(shell.overlay.get(), &mut after).unwrap();
        }
        assert_eq!(after, before);
        assert!(
            shell
                .apply_follow_result(Err(FollowError::NoWindow), false)
                .unwrap()
        );
        assert_eq!(shell.model.borrow().identity(), Some(timeline.identity()));
        assert_eq!(shell.model.borrow().overlay_text(), "跟隨測試字幕");
        assert!(!cancel.is_cancelled());
        shell.command(PRIMARY_MONITOR).unwrap();
        assert!(shell.follow_notice.borrow().is_none());
    }
    #[test]
    fn follow_tick_is_throttled_and_rejects_unverified_active_source() {
        use crate::{ai::provider::SpeechProvider, ui::bridge::UiSource};
        let shell = SubtitleShell::allocate(DisplayMode::SideChat, false);
        shell.create(false).unwrap();
        *shell.frame.borrow_mut() = Some(Arc::new(UiFrame {
            source: Some(UiSource {
                pid: std::process::id(),
                provider: SpeechProvider::OpenAi,
            }),
            process_created: Some(0),
            status: UiStatus::Starting,
            ..Default::default()
        }));
        shell.command(FOLLOW_SOURCE_MONITOR).unwrap();
        assert!(control_text(shell.status.get()).contains("無法驗證身分"));
        let previous = shell.last_follow_check.get();
        shell.poll_follow().unwrap();
        assert_eq!(shell.last_follow_check.get(), previous);
        shell.last_follow_check.set(None);
        shell.poll_follow().unwrap();
        assert!(shell.last_follow_check.get().is_some());
        assert!(!shell.failed.get());
    }
    #[test]
    fn native_activity_updates_keep_selection_and_history_and_hide_unknown_meter() {
        use windows::Win32::UI::Controls::PBM_GETPOS;
        let (shell, state) = controlled_mock();
        let mut source = crate::ui::controls::mock_source(42);
        source.peak = Some(0.35);
        source.volume = Some(0.8);
        source.muted = Some(false);
        state.borrow_mut().update.sources = Arc::new(crate::ui::controls::choices(
            &crate::audio::sessions::ScanReport {
                sources: vec![source.clone()],
                ..Default::default()
            },
        ));
        shell.poll_controller().unwrap();
        let (combo, details, meter) = {
            let panel = shell.panel.borrow();
            let panel = panel.as_ref().unwrap();
            (panel.source, panel.details, panel.meter)
        };
        // SAFETY: test selects only its own source combo, no backend start command.
        unsafe {
            SendMessageW(combo, CB_SETCURSEL, Some(WPARAM(1)), None);
        }
        shell.refresh(false).unwrap();
        assert!(control_text(details).contains("串流執行中 · 有訊號 · 峰值 35.0%"));
        assert!(control_text(shell.status.get()).contains("已選擇 PID 42"));
        assert!(control_text(shell.status.get()).contains("尚未選擇服務"));
        let history = control_text(shell.history.get());
        // SAFETY: private progress bar and selector; visibility style is checked under hidden parent.
        unsafe {
            assert_eq!(SendMessageW(meter, PBM_GETPOS, None, None).0, 350);
            assert!(GetWindowLongPtrW(meter, GWL_STYLE) as u32 & WS_VISIBLE.0 != 0);
        }
        source.peak = None;
        state.borrow_mut().update.sources = Arc::new(crate::ui::controls::choices(
            &crate::audio::sessions::ScanReport {
                sources: vec![source.clone()],
                ..Default::default()
            },
        ));
        shell.poll_controller().unwrap();
        assert!(control_text(details).contains("訊號未知 · 峰值 未知"));
        assert_eq!(control_text(shell.history.get()), history);
        // SAFETY: own controls. Metadata-only updates preserve chosen index and list size.
        unsafe {
            assert_eq!(SendMessageW(combo, CB_GETCURSEL, None, None).0, 1);
            assert_eq!(SendMessageW(combo, CB_GETCOUNT, None, None).0, 2);
            assert_eq!(GetWindowLongPtrW(meter, GWL_STYLE) as u32 & WS_VISIBLE.0, 0);
        }
        state.borrow_mut().update.busy = true;
        source.process_created = Some(101);
        source.peak = Some(1.0);
        state.borrow_mut().update.sources = Arc::new(crate::ui::controls::choices(
            &crate::audio::sessions::ScanReport {
                sources: vec![source],
                ..Default::default()
            },
        ));
        shell.poll_controller().unwrap();
        assert!(control_text(details).contains("未取得最新資料"));
        assert!(!control_text(details).contains("100.0%"));
        assert!(state.borrow().calls.is_empty());
    }

    #[test]
    fn native_status_pane_reserves_caption_space_in_a_short_controlled_window() {
        let (shell, _state) = controlled_mock();
        shell.command(SETTINGS).unwrap();
        // SAFETY: only this test's hidden owner is resized, on its UI thread.
        unsafe {
            let dpi = GetDpiForWindow(shell.main.get());
            SetWindowPos(
                shell.main.get(),
                None,
                0,
                0,
                scale(600, dpi),
                scale(760, dpi),
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
            .unwrap();
            shell.layout().unwrap();
            let (mut status, mut caption) = (RECT::default(), RECT::default());
            GetClientRect(shell.status.get(), &mut status).unwrap();
            GetClientRect(shell.history.get(), &mut caption).unwrap();
            assert!(caption.bottom >= scale(48, dpi));
            assert!(status.bottom >= scale(24, dpi));
        }
    }
    #[test]
    fn source_owner_draw_renders_actual_icon_pixels_and_keeps_accessible_label() {
        let (shell, state) = controlled_mock();
        let mut source = crate::ui::controls::mock_source(42);
        source.icon = Some(Arc::new(crate::audio::sessions::ApplicationIcon {
            width: 1,
            height: 1,
            bgra: vec![0, 0, 255, 255],
        }));
        state.borrow_mut().update.sources = Arc::new(crate::ui::controls::choices(
            &crate::audio::sessions::ScanReport {
                sources: vec![source],
                ..Default::default()
            },
        ));
        shell.poll_controller().unwrap();
        let combo = shell.panel.borrow().as_ref().unwrap().source;
        let pixels = super::super::subtitle_render::inspect_test_canvas(320, 80, |dc| {
            let draw = DRAWITEMSTRUCT {
                CtlType: ODT_COMBOBOX,
                CtlID: subtitle_controls::SOURCE as u32,
                itemID: 1,
                hwndItem: combo,
                hDC: dc,
                rcItem: RECT {
                    left: 0,
                    top: 0,
                    right: 320,
                    bottom: 40,
                },
                ..Default::default()
            };
            // SAFETY: stack draw structure and private test DIB live throughout synchronous callback.
            unsafe {
                assert_eq!(
                    SendMessageW(
                        shell.main.get(),
                        WM_DRAWITEM,
                        Some(WPARAM(subtitle_controls::SOURCE)),
                        Some(LPARAM((&draw as *const DRAWITEMSTRUCT) as isize))
                    )
                    .0,
                    1
                );
            }
            Ok(())
        })
        .unwrap();
        assert!(
            pixels[..40 * 320]
                .iter()
                .any(|pixel| pixel & 0x00ffffff == 0x00ff0000)
        );
        assert!(pixels[40 * 320..].iter().all(|pixel| *pixel == 0));
        // SAFETY: HASSTRINGS leaves the real bounded label available to native accessibility.
        unsafe {
            assert!(GetWindowLongPtrW(combo, GWL_STYLE) as u32 & CBS_HASSTRINGS as u32 != 0);
        }
        assert!(!shell.failed.get());
        assert!(state.borrow().calls.is_empty());
    }
    #[test]
    fn side_docking_preserves_session_and_panel_controls_remain_separated() {
        let (shell, state) = controlled_mock();
        let timeline = fixture_timeline(9200, "停靠測試字幕");
        shell.model.borrow_mut().select_session(timeline.identity());
        shell
            .model
            .borrow_mut()
            .apply(crate::ui::presentation::TimelineSnapshot::from_timeline(
                &timeline, 1,
            ));
        assert_eq!(shell.side_dock.get(), SideDock::Floating);
        for (command, expected) in [
            (DOCK_LEFT, SideDock::Left),
            (DOCK_RIGHT, SideDock::Right),
            (DOCK_FLOAT, SideDock::Floating),
        ] {
            shell.command(command).unwrap();
            assert_eq!(shell.side_dock.get(), expected);
            assert_eq!(shell.model.borrow().identity(), Some(timeline.identity()));
            assert_eq!(shell.model.borrow().overlay_text(), "停靠測試字幕");
            if expected != SideDock::Floating {
                let work = subtitle_monitor::window_work_area(shell.main.get()).unwrap();
                let mut rect = RECT::default();
                // SAFETY: own controller, initialized out rectangle.
                unsafe {
                    GetWindowRect(shell.main.get(), &mut rect).unwrap();
                }
                assert_eq!(rect.top, work.top);
                assert_eq!(rect.bottom, work.bottom);
                if expected == SideDock::Left {
                    assert_eq!(rect.left, work.left);
                } else {
                    assert_eq!(rect.right, work.right);
                }
            }
        }
        let panel = shell.panel.borrow();
        let panel = panel.as_ref().unwrap();
        let mut details = RECT::default();
        let mut provider = RECT::default();
        let mut meter = RECT::default();
        // SAFETY: rectangles for owned controls under the same hidden test window.
        unsafe {
            GetWindowRect(panel.details, &mut details).unwrap();
            GetWindowRect(panel.subtitle_source, &mut provider).unwrap();
            GetWindowRect(panel.meter, &mut meter).unwrap();
        }
        assert!(details.bottom <= meter.top && meter.bottom <= provider.top);
        assert!(state.borrow().calls.is_empty());
    }
    #[test]
    fn phase11_compact_settings_toggle_preserves_subtitles_geometry_and_system_font() {
        let (shell, state) = controlled_mock();
        let timeline = fixture_timeline(9300, "完整暫定字幕");
        shell.model.borrow_mut().select_session(timeline.identity());
        shell
            .model
            .borrow_mut()
            .apply(crate::ui::presentation::TimelineSnapshot::from_timeline(
                &timeline, 1,
            ));
        assert!(shell.page.get() == ControlPage::Sources);
        let initial = shell.subtitle_rect.get().unwrap();
        shell.command(SETTINGS).unwrap();
        assert!(shell.page.get() == ControlPage::Subtitles);
        assert_eq!(shell.side_dock.get(), SideDock::Floating);
        // SAFETY: hidden test windows, owned UI-thread handles and initialized outputs.
        unsafe {
            let mut rect = RECT::default();
            GetWindowRect(shell.main.get(), &mut rect).unwrap();
            assert_eq!(
                (rect.left, rect.top, rect.right, rect.bottom),
                (initial.left, initial.top, initial.right, initial.bottom)
            );
            let (screen, work) = subtitle_monitor::window_geometry(shell.main.get()).unwrap();
            let expected = subtitle_monitor::initial_side_placement(screen, work);
            assert_eq!(rect.right - rect.left, expected.right - expected.left);
            assert_eq!(rect.bottom - rect.top, expected.bottom - expected.top);
            assert_eq!(rect.right, work.right);
            assert_eq!(GetMenuItemCount(Some(GetMenu(shell.main.get()))), 2);
            assert_eq!(GetMenuItemID(GetMenu(shell.main.get()), 1), ABOUT as u32);
            for control in shell.panel.borrow().as_ref().unwrap().handles() {
                let panel = shell.panel.borrow();
                let panel = panel.as_ref().unwrap();
                if [
                    panel.start,
                    panel.stop,
                    panel.source,
                    panel.language,
                    panel.quality,
                ]
                .contains(&control)
                {
                    assert_ne!(
                        GetWindowLongPtrW(control, GWL_STYLE) as u32 & WS_VISIBLE.0,
                        0
                    );
                    continue;
                }
                assert_eq!(
                    GetWindowLongPtrW(control, GWL_STYLE) as u32 & WS_VISIBLE.0,
                    0
                );
            }
            let mut font = LOGFONTW::default();
            assert_ne!(
                GetObjectW(
                    HGDIOBJ(shell.font.get().0),
                    std::mem::size_of::<LOGFONTW>() as i32,
                    Some((&mut font as *mut LOGFONTW).cast())
                ),
                0
            );
            let mut metrics = NONCLIENTMETRICSW {
                cbSize: std::mem::size_of::<NONCLIENTMETRICSW>() as u32,
                ..Default::default()
            };
            SystemParametersInfoForDpi(
                SPI_GETNONCLIENTMETRICS.0,
                metrics.cbSize,
                Some((&mut metrics as *mut NONCLIENTMETRICSW).cast()),
                0,
                GetDpiForWindow(shell.main.get()),
            )
            .unwrap();
            assert_eq!(font.lfHeight, metrics.lfMessageFont.lfHeight);
            assert_eq!(font.lfFaceName, metrics.lfMessageFont.lfFaceName);
            let mut status = RECT::default();
            let mut history = RECT::default();
            GetWindowRect(shell.status.get(), &mut status).unwrap();
            GetWindowRect(shell.history.get(), &mut history).unwrap();
            assert!(status.bottom <= history.top && history.bottom > history.top);
        }
        shell.command(SETTINGS).unwrap();
        assert!(shell.page.get() == ControlPage::Sources);
        assert_eq!(shell.model.borrow().identity(), Some(timeline.identity()));
        assert_eq!(shell.model.borrow().overlay_text(), "完整暫定字幕");
        assert!(state.borrow().calls.is_empty());
    }
    #[test]
    fn search_has_a_separate_preceding_native_label_outside_activity_and_edit() {
        let (shell, _state) = controlled_mock();
        assert_eq!(control_text(shell.search_label.get()), "搜尋字幕紀錄");
        assert!(control_text(shell.search.get()).is_empty());
        let mut label = RECT::default();
        let mut edit = RECT::default();
        // SAFETY: only our hidden test controls and their native sibling ordering.
        unsafe {
            assert_eq!(
                GetWindow(shell.search.get(), GW_HWNDPREV).unwrap(),
                shell.search_label.get()
            );
            GetWindowRect(shell.search_label.get(), &mut label).unwrap();
            GetWindowRect(shell.search.get(), &mut edit).unwrap();
        }
        assert!(label.bottom <= edit.top);
    }
    #[test]
    fn native_provider_placeholder_fits_the_default_selection_area() {
        use windows::Win32::UI::Controls::{COMBOBOXINFO, GetComboBoxInfo};
        let (shell, _state) = controlled_mock();
        let provider = shell.panel.borrow().as_ref().unwrap().provider;
        let mut combo = COMBOBOXINFO {
            cbSize: std::mem::size_of::<COMBOBOXINFO>() as u32,
            ..Default::default()
        };
        // SAFETY: query our owned combo and its installed font, initialized out structure.
        let font = unsafe {
            GetComboBoxInfo(provider, &mut combo).unwrap();
            SendMessageW(provider, WM_GETFONT, None, None).0
        };
        let mut size = windows::Win32::Foundation::SIZE::default();
        let label = control_text(provider);
        assert!(!label.is_empty());
        let text: Vec<u16> = label.encode_utf16().collect();
        super::super::subtitle_render::inspect_test_canvas(1, 1, |dc| {
            // SAFETY: private test DIB/DC and borrowed live font, restore selection before return.
            unsafe {
                let old = SelectObject(dc, HGDIOBJ(font as *mut _));
                GetTextExtentPoint32W(dc, &text, &mut size).ok().unwrap();
                SelectObject(dc, old);
            }
            Ok(())
        })
        .unwrap();
        let available = combo.rcItem.right - combo.rcItem.left;
        assert!(
            size.cx <= available,
            "provider text width {} exceeds selection width {}",
            size.cx,
            available
        );
    }
}

#[cfg(test)]
#[path = "subtitle_validation_tests.rs"]
mod validation_tests;

#[path = "subtitle_status_window.rs"]
mod status_window;

#[cfg(test)]
#[path = "subtitle_interaction_tests.rs"]
mod interaction_tests;

#[cfg(test)]
#[path = "subtitle_key_tests.rs"]
mod key_tests;

#[cfg(test)]
#[path = "subtitle_source_tests.rs"]
mod subtitle_source_tests;

#[cfg(test)]
#[path = "subtitle_alignment_tests.rs"]
mod alignment_tests;
