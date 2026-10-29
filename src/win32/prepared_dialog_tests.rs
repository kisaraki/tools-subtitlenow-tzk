//! Hidden app-owned native fixtures, synthetic paths/keys, no cloud or toasts.
use super::*;
use crate::media::planning::{PreparationPlan, PreparedRequest};
use std::rc::Rc;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;

const EM_CANUNDO: u32 = 0x00c6;

#[path = "prepared_alignment_dialog_tests.rs"]
mod alignment;
#[path = "prepared_edit_dialog_tests.rs"]
mod edits;
#[path = "prepared_embedded_dialog_tests.rs"]
mod embedded;
#[path = "prepared_https_dialog_tests.rs"]
mod https;
#[path = "prepared_multitrack_dialog_tests.rs"]
mod multiple;
#[path = "prepared_remote_dialog_tests.rs"]
mod remote;
struct Owner(HWND, HINSTANCE);
impl Owner {
    fn new() -> Self {
        // SAFETY: private hidden STATIC owner on this test thread; real user
        // windows and the shell notification area are never operated on.
        unsafe {
            let instance = HINSTANCE(GetModuleHandleW(None).unwrap().0);
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("Synthetic Prepared owner"),
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
    fn dialog(
        &self,
    ) -> (
        Box<Dialog>,
        Rc<RefCell<MockState>>,
        Rc<RefCell<NoticeState>>,
    ) {
        let state = Rc::new(RefCell::new(MockState::default()));
        let notice = Rc::new(RefCell::new(NoticeState::default()));
        let dialog = Dialog::new(
            Box::new(MockBackend(state.clone())),
            Box::new(MockNotification(notice.clone())),
        );
        dialog.create(self.0, self.1, false).unwrap();
        (dialog, state, notice)
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.0);
        }
    }
}
struct MockState {
    state: PreparedUiState,
    starts: usize,
    https_starts: usize,
    stops: usize,
    polls: usize,
    options: Option<PreparedOptions>,
    supplied_key: Option<&'static str>,
    fail_start: bool,
    stopping_polls: Option<usize>,
    plan: Option<PreparationPlan>,
}
impl Default for MockState {
    fn default() -> Self {
        Self {
            state: PreparedUiState::Idle,
            starts: 0,
            https_starts: 0,
            stops: 0,
            polls: 0,
            options: None,
            supplied_key: None,
            fail_start: false,
            stopping_polls: None,
            plan: None,
        }
    }
}
struct MockBackend(Rc<RefCell<MockState>>);
impl PreparedBackend for MockBackend {
    fn start(
        &mut self,
        request: PreparedRequest,
        options: PreparedOptions,
        key: Option<ApiKey>,
    ) -> std::result::Result<(), PreparedUiError> {
        let mut state = self.0.borrow_mut();
        state.starts += 1;
        state.plan = Some(request.plan()?);
        state.options = Some(options);
        state.supplied_key = key.as_ref().map(|key| match key.expose() {
            "SYNTHETIC_OPENAI" => "openai",
            "SYNTHETIC_GEMINI" => "gemini",
            _ => "other",
        });
        if state.fail_start {
            state.state = PreparedUiState::Failed(PreparedUiError::WorkerStart);
            return Err(PreparedUiError::WorkerStart);
        }
        state.state = PreparedUiState::Preparing;
        Ok(())
    }
    fn start_with_https_subtitles(
        &mut self,
        request: PreparedRequest,
        options: PreparedOptions,
        key: Option<ApiKey>,
    ) -> std::result::Result<(), PreparedUiError> {
        self.0.borrow_mut().https_starts += 1;
        self.start(request, options, key)
    }
    fn poll(&mut self) -> PreparedUiState {
        let mut state = self.0.borrow_mut();
        state.polls += 1;
        if state.state == PreparedUiState::Stopping
            && let Some(remaining) = state.stopping_polls
        {
            if remaining == 0 {
                state.state = PreparedUiState::Cancelled;
            } else {
                state.stopping_polls = Some(remaining - 1);
            }
        }
        state.state
    }
    fn stop(&mut self) {
        let mut state = self.0.borrow_mut();
        state.stops += 1;
        state.state = if busy(state.state) {
            PreparedUiState::Stopping
        } else {
            PreparedUiState::Cancelled
        };
    }
    fn state(&self) -> PreparedUiState {
        self.0.borrow().state
    }
}
#[derive(Default)]
struct NoticeState {
    ready: usize,
    clears: usize,
    fail: bool,
}
struct MockNotification(Rc<RefCell<NoticeState>>);
impl CompletionNotification for MockNotification {
    fn ready(&mut self, _: HWND) -> std::result::Result<(), NotificationError> {
        let mut state = self.0.borrow_mut();
        state.ready += 1;
        if state.fail {
            Err(NotificationError::Unavailable)
        } else {
            Ok(())
        }
    }
    fn clear(&mut self) {
        self.0.borrow_mut().clears += 1;
    }
}
fn text(control: HWND) -> String {
    let mut buffer = vec![0u16; PATH_LIMIT + KEY_LIMIT + 1];
    let len = unsafe { GetWindowTextW(control, &mut buffer) }.max(0) as usize;
    String::from_utf16(&buffer[..len]).unwrap()
}
fn select(control: HWND, index: usize) {
    unsafe {
        SendMessageW(control, CB_SETCURSEL, Some(WPARAM(index)), None);
    }
}
fn fill(dialog: &Dialog, language: usize, provider: usize) {
    set_edit(dialog.media.get(), r"C:\Synthetic\media.mkv").unwrap();
    set_edit(dialog.subtitle.get(), r"C:\Synthetic\subtitle.srt").unwrap();
    select(dialog.format.get(), 1);
    select(dialog.language.get(), language);
    select(dialog.provider.get(), provider);
    dialog.refresh_destination().unwrap();
}
fn click(dialog: &Dialog, id: usize) {
    unsafe {
        SendMessageW(dialog.hwnd.get(), WM_COMMAND, Some(WPARAM(id)), None);
    }
    assert!(!dialog.failed.get());
}

#[test]
fn fixed_fields_have_explicit_eight_languages_masked_keys_and_standard_default() {
    let owner = Owner::new();
    let (dialog, _, _) = owner.dialog();
    assert_eq!(dialog.controls.borrow().len(), CONTROL_COUNT);
    for (control, count, initial) in [
        (dialog.format.get(), 6, 0),
        (dialog.language.get(), 9, 0),
        (dialog.provider.get(), 3, 0),
        (dialog.quality.get(), 3, 1),
        (dialog.audio_language.get(), 6, 0),
    ] {
        unsafe {
            assert_eq!(SendMessageW(control, CB_GETCOUNT, None, None).0, count);
            assert_eq!(SendMessageW(control, CB_GETCURSEL, None, None).0, initial);
        }
    }
    for control in [dialog.openai_key.get(), dialog.gemini_key.get()] {
        assert_ne!(
            unsafe { GetWindowLongPtrW(control, GWL_STYLE) } & ES_PASSWORD as isize,
            0
        );
    }
    assert_eq!(text(dialog.controls.borrow()[16]), KEY_NOTICE);
    assert_eq!(text(dialog.controls.borrow()[20]), PLAYBACK_NOTICE);
    assert!(dialog.read_input().is_err());
}

#[test]
fn chinese_preparation_never_reads_either_key_and_cloud_disclosure_stays_local() {
    let owner = Owner::new();
    let (dialog, state, _) = owner.dialog();
    fill(&dialog, 3, 2);
    set_edit(dialog.openai_key.get(), "invalid key").unwrap();
    set_edit(dialog.gemini_key.get(), &"X".repeat(KEY_LIMIT + 1)).unwrap();
    assert!(text(dialog.destination.get()).contains("不使用雲端"));
    click(&dialog, PREPARE);
    assert_eq!(state.borrow().starts, 1);
    assert_eq!(state.borrow().supplied_key, None);
    for control in [dialog.openai_key.get(), dialog.gemini_key.get()] {
        assert!(text(control).is_empty());
        assert_eq!(
            unsafe { SendMessageW(control, EM_CANUNDO, None, None).0 },
            0
        );
    }
}

#[test]
fn source_preparation_moves_only_selected_key_after_validated_paths() {
    let owner = Owner::new();
    let (dialog, state, _) = owner.dialog();
    fill(&dialog, 4, 1);
    set_edit(dialog.openai_key.get(), "SYNTHETIC_OPENAI").unwrap();
    set_edit(dialog.gemini_key.get(), "invalid unused key").unwrap();
    assert!(text(dialog.destination.get()).contains("https://api.openai.com"));
    assert!(!text(dialog.destination.get()).contains("Synthetic"));
    click(&dialog, PREPARE);
    assert_eq!(state.borrow().supplied_key, Some("openai"));
    assert_eq!(
        state.borrow().options.unwrap().quality,
        TranslationQuality::Standard
    );
    assert_eq!(
        state.borrow().options.unwrap().provider,
        Some(SpeechProvider::OpenAi)
    );
    assert!(text(dialog.openai_key.get()).is_empty());
    assert!(text(dialog.gemini_key.get()).is_empty());
}

#[test]
fn blank_selected_key_is_passed_as_none_without_ui_credential_access() {
    let owner = Owner::new();
    let (dialog, state, _) = owner.dialog();
    fill(&dialog, 8, 2);
    select(dialog.quality.get(), 2);
    click(&dialog, PREPARE);
    assert_eq!(state.borrow().starts, 1);
    assert_eq!(state.borrow().supplied_key, None);
    assert_eq!(
        state.borrow().options.unwrap().quality,
        TranslationQuality::High
    );
    assert!(text(dialog.destination.get()).contains("https://generativelanguage.googleapis.com"));
}

#[test]
fn invalid_or_truncated_paths_and_unselected_service_never_start_and_clear_keys() {
    let owner = Owner::new();
    let (dialog, state, _) = owner.dialog();
    for path in [
        r"relative\media.mkv".to_owned(),
        r"\\remote\share\media.mkv".to_owned(),
        "X".repeat(PATH_LIMIT + 1),
    ] {
        fill(&dialog, 4, 1);
        set_edit(dialog.media.get(), &path).unwrap();
        set_edit(dialog.openai_key.get(), "SYNTHETIC_OPENAI").unwrap();
        click(&dialog, PREPARE);
        assert_eq!(state.borrow().starts, 0);
        assert!(text(dialog.openai_key.get()).is_empty());
        assert!(!text(dialog.status.get()).contains(&path));
        let notice = text(dialog.status.get());
        dialog.poll().unwrap();
        assert_eq!(text(dialog.status.get()), notice);
    }
    fill(&dialog, 4, 0);
    click(&dialog, PREPARE);
    assert_eq!(state.borrow().starts, 0);
    assert!(text(dialog.status.get()).contains("需要選擇"));
}

#[test]
fn oversized_or_invalid_selected_key_is_not_truncated_and_both_edits_are_cleared() {
    let owner = Owner::new();
    let (dialog, state, _) = owner.dialog();
    for key in ["invalid key".to_owned(), "X".repeat(KEY_LIMIT + 1)] {
        fill(&dialog, 4, 1);
        set_edit(dialog.openai_key.get(), &key).unwrap();
        set_edit(dialog.gemini_key.get(), "SYNTHETIC_GEMINI").unwrap();
        click(&dialog, PREPARE);
        assert_eq!(state.borrow().starts, 0);
        assert!(text(dialog.openai_key.get()).is_empty());
        assert!(text(dialog.gemini_key.get()).is_empty());
        assert!(!text(dialog.status.get()).contains(&key));
        let notice = text(dialog.status.get());
        dialog.poll().unwrap();
        assert_eq!(text(dialog.status.get()), notice);
    }
}

#[test]
fn all_declared_language_formats_and_qualities_survive_native_request_validation() {
    let owner = Owner::new();
    let (dialog, _, _) = owner.dialog();
    for language in 1..=8 {
        for format in 1..=2 {
            for quality in 0..=2 {
                fill(&dialog, language, 1);
                select(dialog.format.get(), format);
                select(dialog.quality.get(), quality);
                let (_, options, source) = dialog.read_input().unwrap();
                assert_eq!(source, language >= 4);
                assert_eq!(
                    options.quality,
                    [
                        TranslationQuality::Fast,
                        TranslationQuality::Standard,
                        TranslationQuality::High
                    ][quality]
                );
            }
        }
    }
}

#[test]
fn active_work_locks_inputs_and_stale_prepare_or_clear_notifications_do_not_restart() {
    let owner = Owner::new();
    let (dialog, state, _) = owner.dialog();
    fill(&dialog, 1, 0);
    click(&dialog, PREPARE);
    for control in [
        dialog.media.get(),
        dialog.subtitle.get(),
        dialog.provider.get(),
        dialog.quality.get(),
        dialog.openai_key.get(),
        dialog.controls.borrow()[FIRST_ACTION_CONTROL],
        dialog.controls.borrow()[FIRST_ACTION_CONTROL + 2],
    ] {
        assert!(!unsafe { IsWindowEnabled(control) }.as_bool());
    }
    assert!(
        unsafe { IsWindowEnabled(dialog.controls.borrow()[FIRST_ACTION_CONTROL + 1]) }.as_bool()
    );
    click(&dialog, PREPARE);
    click(&dialog, CLEAR);
    assert_eq!(state.borrow().starts, 1);
    assert_eq!(state.borrow().stops, 0);
    click(&dialog, CANCEL);
    assert_eq!(state.borrow().state, PreparedUiState::Stopping);
    state.borrow_mut().state = PreparedUiState::Cancelled;
    dialog.poll().unwrap();
    assert!(unsafe { IsWindowEnabled(dialog.media.get()) }.as_bool());
}

#[test]
fn close_cancels_and_keeps_native_owner_alive_until_backend_reports_reaped() {
    let owner = Owner::new();
    let (dialog, state, notice) = owner.dialog();
    fill(&dialog, 1, 0);
    click(&dialog, PREPARE);
    unsafe {
        SendMessageW(dialog.hwnd.get(), WM_CLOSE, None, None);
    }
    assert!(dialog.closing.get());
    assert!(!dialog.hwnd.get().0.is_null());
    assert_eq!(state.borrow().state, PreparedUiState::Stopping);
    assert!(!unsafe { IsWindowEnabled(dialog.controls.borrow()[FIRST_ACTION_CONTROL]) }.as_bool());
    dialog.poll().unwrap();
    assert!(!dialog.hwnd.get().0.is_null());
    state.borrow_mut().state = PreparedUiState::Cancelled;
    dialog.poll().unwrap();
    assert!(dialog.hwnd.get().0.is_null());
    assert!(notice.borrow().clears >= 1);
}

#[test]
fn completion_notifies_once_per_run_and_failure_keeps_ready_without_auto_play() {
    let owner = Owner::new();
    let (dialog, state, notice) = owner.dialog();
    fill(&dialog, 1, 0);
    click(&dialog, PREPARE);
    notice.borrow_mut().fail = true;
    state.borrow_mut().state = PreparedUiState::Ready { cues: 7 };
    for _ in 0..3 {
        dialog.poll().unwrap();
    }
    assert_eq!(notice.borrow().ready, 1);
    assert_eq!(state.borrow().state, PreparedUiState::Ready { cues: 7 });
    assert!(text(dialog.status.get()).contains("Windows 通知無法送出"));
    assert!(text(dialog.controls.borrow()[20]).contains("不會自動播放"));
    click(&dialog, PREPARE);
    assert_eq!(state.borrow().starts, 1);
    for control in [
        dialog.media.get(),
        dialog.subtitle.get(),
        dialog.provider.get(),
        dialog.openai_key.get(),
        dialog.controls.borrow()[FIRST_ACTION_CONTROL],
    ] {
        assert!(!unsafe { IsWindowEnabled(control) }.as_bool());
    }
    click(&dialog, CLEAR);
    fill(&dialog, 1, 0);
    click(&dialog, PREPARE);
    notice.borrow_mut().fail = false;
    state.borrow_mut().state = PreparedUiState::Ready { cues: 2 };
    dialog.poll().unwrap();
    assert_eq!(notice.borrow().ready, 2);
    assert!(!text(dialog.status.get()).contains(NOTIFICATION_FAILURE));
}

#[test]
fn explicit_clear_resets_fields_undo_choices_and_retired_ready_notification() {
    let owner = Owner::new();
    let (dialog, state, notice) = owner.dialog();
    fill(&dialog, 1, 0);
    click(&dialog, PREPARE);
    state.borrow_mut().state = PreparedUiState::Ready { cues: 1 };
    dialog.poll().unwrap();
    click(&dialog, CLEAR);
    for control in [
        dialog.media.get(),
        dialog.subtitle.get(),
        dialog.openai_key.get(),
        dialog.gemini_key.get(),
    ] {
        assert!(text(control).is_empty());
        assert_eq!(
            unsafe { SendMessageW(control, EM_CANUNDO, None, None).0 },
            0
        );
    }
    assert_eq!(selected(dialog.language.get()), 0);
    assert_eq!(selected(dialog.audio_language.get()), 0);
    assert_eq!(selected(dialog.provider.get()), 0);
    assert_eq!(selected(dialog.quality.get()), 1);
    assert!(!dialog.notified.get());
    assert!(notice.borrow().clears >= 1);
    assert_eq!(state.borrow().state, PreparedUiState::Cancelled);
}

#[test]
fn parent_destruction_cancels_owned_work_and_clears_the_borrowed_native_pointer() {
    let owner = Owner::new();
    let (dialog, state, notice) = owner.dialog();
    fill(&dialog, 1, 0);
    click(&dialog, PREPARE);
    let hwnd = dialog.hwnd.get();
    unsafe {
        DestroyWindow(owner.0).unwrap();
    }
    assert!(dialog.hwnd.get().0.is_null());
    assert!(!unsafe { IsWindow(Some(hwnd)) }.as_bool());
    assert!(state.borrow().stops >= 1);
    assert!(notice.borrow().clears >= 1);
    let polls = state.borrow().polls;
    dialog.poll().unwrap();
    assert_eq!(state.borrow().polls, polls + 1);
    state.borrow_mut().state = PreparedUiState::Cancelled;
    dialog.poll().unwrap();
    assert!(!dialog.failed.get());
}

#[test]
fn notification_payload_is_only_static_guidance_with_terminated_native_buffers() {
    let data = notification_data(HWND::default());
    let decode = |units: &[u16]| {
        String::from_utf16(&units[..units.iter().position(|unit| *unit == 0).unwrap()]).unwrap()
    };
    assert_eq!(decode(&data.szInfoTitle), NOTIFICATION_TITLE);
    assert_eq!(decode(&data.szInfo), NOTIFICATION_BODY);
    assert_eq!(decode(&data.szTip), "SubtitleNow");
    assert_eq!(data.cbSize, std::mem::size_of::<NOTIFYICONDATAW>() as u32);
    for private in ["Synthetic", "http", r"C:\", "API"] {
        assert!(!decode(&data.szInfo).contains(private));
    }
}

#[test]
fn native_layout_and_dpi_rectangle_keep_buttons_and_guidance_in_client_bounds() {
    let owner = Owner::new();
    let (dialog, _, _) = owner.dialog();
    for size in [(WIDTH_DIP, HEIGHT_DIP), (960, 1044)] {
        let mut suggested = RECT {
            left: 12,
            top: 24,
            right: 12 + size.0,
            bottom: 24 + size.1,
        };
        unsafe {
            SendMessageW(
                dialog.hwnd.get(),
                WM_DPICHANGED,
                Some(WPARAM(144 | (144 << 16))),
                Some(LPARAM((&mut suggested as *mut RECT) as isize)),
            );
        }
        assert!(!dialog.failed.get());
        let mut client = RECT::default();
        unsafe {
            GetClientRect(dialog.hwnd.get(), &mut client).unwrap();
        }
        for index in [
            17,
            DISCOVER_CONTROL_INDEX,
            DISCOVER_CONTROL_INDEX + 1,
            FIRST_ACTION_CONTROL,
            FIRST_ACTION_CONTROL + 1,
            FIRST_ACTION_CONTROL + 2,
            FIRST_ACTION_CONTROL + 3,
        ] {
            let mut rect = RECT::default();
            unsafe {
                GetWindowRect(dialog.controls.borrow()[index], &mut rect).unwrap();
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
                let _ = MapWindowPoints(None, Some(dialog.hwnd.get()), &mut points);
                rect.left = points[0].x;
                rect.top = points[0].y;
                rect.right = points[1].x;
                rect.bottom = points[1].y;
            }
            assert!(
                rect.left >= 0
                    && rect.top >= 0
                    && rect.right <= client.right
                    && rect.bottom <= client.bottom,
                "control {index}"
            );
        }
    }
}

#[test]
fn scroll_geometry_covers_common_scaling_and_clipped_200_percent_work_area() {
    let work = RECT {
        left: 0,
        top: 0,
        right: 1920,
        bottom: 1040,
    };
    for dpi in [96, 120, 144, 192] {
        let width = super::super::subtitle_window::scale(WIDTH_DIP, dpi);
        let height = super::super::subtitle_window::scale(HEIGHT_DIP, dpi);
        let bounds = fit_work_area(
            RECT {
                left: 180,
                top: 200,
                right: 180 + width,
                bottom: 200 + height,
            },
            work,
        );
        assert!(bounds.top >= 0 && bounds.bottom <= work.bottom && bounds.right <= work.right);
        let client_height =
            (bounds.bottom - bounds.top - super::super::subtitle_window::scale(40, dpi)).max(1);
        let geometry = scroll_geometry(client_height, dpi, i32::MAX);
        assert!(
            geometry.button_y >= geometry.viewport
                && geometry.button_y + geometry.button_height <= client_height
        );
        assert_eq!(
            geometry.position,
            (geometry.content - geometry.viewport).max(0)
        );
        assert!(
            super::super::subtitle_window::scale(856, dpi) - geometry.position <= geometry.viewport
        );
    }
    let geometry = scroll_geometry(1000, 192, i32::MAX);
    assert_eq!(geometry.position, 1080);
    assert_eq!(geometry.viewport, 680);
    assert_eq!(geometry.button_y, 904);
    assert_eq!(geometry.button_height, 64);
}

#[test]
fn small_native_viewport_scrolls_content_and_keeps_action_buttons_fixed() {
    let owner = Owner::new();
    let (dialog, _, _) = owner.dialog();
    // SAFETY: only resize this app-owned hidden fixture. The scroll container
    // clips fields and preserves the separately parented bottom action buttons.
    unsafe {
        SetWindowPos(
            dialog.hwnd.get(),
            None,
            0,
            0,
            640,
            420,
            SWP_NOZORDER | SWP_NOACTIVATE,
        )
        .unwrap();
    }
    dialog.layout().unwrap();
    let button = dialog.controls.borrow()[FIRST_ACTION_CONTROL];
    let mut before = RECT::default();
    unsafe {
        GetWindowRect(button, &mut before).unwrap();
    }
    dialog.scroll_command(SB_BOTTOM.0 as usize).unwrap();
    assert!(dialog.scroll.get() > 0);
    let mut after = RECT::default();
    unsafe {
        GetWindowRect(button, &mut after).unwrap();
    }
    assert_eq!(
        (before.left, before.top, before.right, before.bottom),
        (after.left, after.top, after.right, after.bottom)
    );
    let mut body = RECT::default();
    let mut label = RECT::default();
    unsafe {
        GetWindowRect(dialog.content.get(), &mut body).unwrap();
        GetWindowRect(dialog.keep_ready.get(), &mut label).unwrap();
    }
    assert!(label.top >= body.top && label.bottom <= body.bottom);
    dialog.scroll_command(SB_TOP.0 as usize).unwrap();
    assert_eq!(dialog.scroll.get(), 0);
}

#[test]
fn worker_start_error_remains_visible_and_does_not_emit_completion_notification() {
    let owner = Owner::new();
    let (dialog, state, notice) = owner.dialog();
    fill(&dialog, 1, 0);
    state.borrow_mut().fail_start = true;
    click(&dialog, PREPARE);
    assert!(text(dialog.status.get()).contains("無法啟動"));
    assert_eq!(notice.borrow().ready, 0);
    assert_eq!(
        state.borrow().state,
        PreparedUiState::Failed(PreparedUiError::WorkerStart)
    );
}

#[test]
fn real_chinese_gui_preparation_holds_ram_ready_until_clear_without_cloud_or_playback() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let media = fixture.path().join("synthetic.media");
    let subtitle = fixture.path().join("synthetic.srt");
    std::fs::write(&media, b"opaque synthetic fixture, not captured media").unwrap();
    std::fs::write(
        &subtitle,
        "1\n01:02:03,004 --> 01:02:05,006\n软件与服务器\n",
    )
    .unwrap();
    let owner = Owner::new();
    let notification = Rc::new(RefCell::new(NoticeState::default()));
    let dialog = Dialog::new(
        Box::new(crate::app::prepared::PreparedController::new()),
        Box::new(MockNotification(notification.clone())),
    );
    dialog.create(owner.0, owner.1, false).unwrap();
    set_edit(dialog.media.get(), media.to_str().unwrap()).unwrap();
    set_edit(dialog.subtitle.get(), subtitle.to_str().unwrap()).unwrap();
    select(dialog.format.get(), 1);
    select(dialog.language.get(), 3);
    select(dialog.provider.get(), 2);
    // Chinese must ignore these deliberately invalid synthetic password edits.
    set_edit(dialog.openai_key.get(), "invalid synthetic key").unwrap();
    set_edit(dialog.gemini_key.get(), &"X".repeat(KEY_LIMIT + 1)).unwrap();
    dialog.refresh_destination().unwrap();
    assert!(text(dialog.destination.get()).contains("不使用雲端"));
    click(&dialog, PREPARE);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while busy(dialog.backend.borrow().state()) {
        dialog.poll().unwrap();
        assert!(
            std::time::Instant::now() < deadline,
            "synthetic GUI worker did not finish"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(
        dialog.backend.borrow().state(),
        PreparedUiState::Ready { cues: 1 }
    );
    assert!(text(dialog.status.get()).contains("共 1 段"));
    assert!(text(dialog.status.get()).contains("可保留返回"));
    assert_eq!(notification.borrow().ready, 1);
    assert!(text(dialog.openai_key.get()).is_empty());
    assert!(text(dialog.gemini_key.get()).is_empty());
    assert!(
        std::fs::OpenOptions::new()
            .write(true)
            .open(&media)
            .is_err()
    );
    // Polling Ready does not notify or start work again. Its file pin remains
    // with the actual PreparedSession until an explicit Clear retires the run.
    dialog.poll().unwrap();
    assert_eq!(notification.borrow().ready, 1);
    click(&dialog, CLEAR);
    assert_eq!(dialog.backend.borrow().state(), PreparedUiState::Cancelled);
    assert!(std::fs::OpenOptions::new().write(true).open(&media).is_ok());
    assert_eq!(std::fs::read_dir(fixture.path()).unwrap().count(), 2);
    click(&dialog, CLOSE);
    assert!(dialog.hwnd.get().0.is_null());
}

#[test]
fn modal_quit_reaps_retiring_worker_and_preserves_the_outer_quit_code() {
    let owner = Owner::new();
    let (dialog, state, _) = owner.dialog();
    fill(&dialog, 1, 0);
    click(&dialog, PREPARE);
    state.borrow_mut().stopping_polls = Some(2);
    // SAFETY: synthetic quit belongs only to this fixture's Rust test/UI thread.
    unsafe {
        PostQuitMessage(37);
    }
    run_modal(&dialog).unwrap();
    assert!(dialog.hwnd.get().0.is_null());
    assert_eq!(state.borrow().state, PreparedUiState::Cancelled);
    assert!(state.borrow().polls >= 3);
    let mut quit = MSG::default();
    // SAFETY: retrieve the reposted quit from this thread only, leaving no quit
    // behind for other app-owned native fixture operations on this thread.
    assert!(unsafe { PeekMessageW(&mut quit, None, WM_QUIT, WM_QUIT, PM_REMOVE) }.as_bool());
    assert_eq!(quit.message, WM_QUIT);
    assert_eq!(quit.wParam.0, 37);
}

#[test]
fn modal_pump_retains_destroyed_native_owner_until_worker_reaping_finishes() {
    let owner = Owner::new();
    let (dialog, state, _) = owner.dialog();
    fill(&dialog, 1, 0);
    click(&dialog, PREPARE);
    state.borrow_mut().stopping_polls = Some(2);
    // SAFETY: destroy this hidden owner only; dialog pointer is synchronously
    // retired, but the stable Rust Box stays alive while modal pumping reaps.
    unsafe {
        DestroyWindow(owner.0).unwrap();
    }
    run_modal(&dialog).unwrap();
    assert_eq!(state.borrow().state, PreparedUiState::Cancelled);
    assert!(state.borrow().polls >= 3);
    assert!(!dialog.failed.get());
}

fn notify(dialog: &Dialog, id: usize, code: usize, control: HWND) {
    // SAFETY: forward a same-thread notification from this fixture's owned
    // viewport/control through the real native callback; no external HWND.
    unsafe {
        SendMessageW(
            dialog.content.get(),
            WM_COMMAND,
            Some(WPARAM(id | (code << 16))),
            Some(LPARAM(control.0 as isize)),
        );
    }
    assert!(!dialog.failed.get());
}
fn audio_input(dialog: &Dialog, language: usize, provider: usize) {
    set_edit(dialog.media.get(), r"C:\Synthetic\voice.wav").unwrap();
    set_edit(dialog.subtitle.get(), "").unwrap();
    select(dialog.audio_language.get(), language);
    select(dialog.provider.get(), provider);
    notify(
        dialog,
        PROVIDER,
        CBN_SELCHANGE as usize,
        dialog.provider.get(),
    );
}
fn assert_keys_cleared(dialog: &Dialog) {
    for control in [dialog.openai_key.get(), dialog.gemini_key.get()] {
        assert!(text(control).is_empty());
        assert_eq!(
            unsafe { SendMessageW(control, EM_CANUNDO, None, None).0 },
            0
        );
    }
}

#[test]
fn unsupported_language_for_explicit_audio_or_subtitle_is_not_relabelled_as_auto() {
    let owner = Owner::new();
    let (dialog, state, _) = owner.dialog();
    let error = PreparedUiError::Speech(PreparedSpeechError::UnsupportedLanguage);
    let generic = format!("媒體準備：{error}");
    audio_input(&dialog, 1, 1);
    // A synchronous failure at the backend interface and an asynchronous
    // provider mismatch both preserve their domain message for selected English.
    dialog.show_error(error).unwrap();
    dialog.poll().unwrap();
    assert_eq!(text(dialog.status.get()), generic);
    assert!(!text(dialog.status.get()).contains(AUTO_NOTICE));
    dialog.local_error.set(None);
    state.borrow_mut().state = PreparedUiState::Failed(error);
    dialog.poll().unwrap();
    assert_eq!(text(dialog.status.get()), generic);
    // An unused Auto preference must not change a sidecar error either.
    fill(&dialog, 1, 0);
    select(dialog.audio_language.get(), 0);
    dialog.show_error(error).unwrap();
    dialog.poll().unwrap();
    assert_eq!(text(dialog.status.get()), generic);
}

#[test]
fn audio_auto_is_typed_before_backend_or_any_key_read_and_preserves_explicit_guidance() {
    let owner = Owner::new();
    let (dialog, state, _) = owner.dialog();
    audio_input(&dialog, 0, 2);
    // Reading either key would produce Credential; Auto must win beforehand.
    set_edit(dialog.openai_key.get(), "invalid unused key").unwrap();
    set_edit(dialog.gemini_key.get(), &"X".repeat(KEY_LIMIT + 1)).unwrap();
    click(&dialog, PREPARE);
    assert_eq!(
        dialog.local_error.get(),
        Some(PreparedUiError::Speech(
            PreparedSpeechError::UnsupportedLanguage
        ))
    );
    assert_eq!(state.borrow().starts, 0);
    assert_eq!(state.borrow().supplied_key, None);
    assert_eq!(
        text(dialog.status.get()),
        format!("媒體準備：{AUTO_NOTICE}")
    );
    assert_keys_cleared(&dialog);
    dialog.poll().unwrap();
    assert!(text(dialog.status.get()).contains("請選擇英文"));
}

#[test]
fn empty_sidecar_uses_independent_five_audio_languages_and_all_qualities() {
    let owner = Owner::new();
    let (dialog, state, _) = owner.dialog();
    for (index, language) in [
        SourceLanguage::English,
        SourceLanguage::Japanese,
        SourceLanguage::French,
        SourceLanguage::German,
        SourceLanguage::Korean,
    ]
    .into_iter()
    .enumerate()
    {
        for provider in 1..=2 {
            for quality in 0..=2 {
                audio_input(&dialog, index + 1, provider);
                select(dialog.format.get(), 0);
                select(dialog.language.get(), 0);
                select(dialog.quality.get(), quality);
                let (request, options, cloud) = dialog.read_input().unwrap();
                assert_eq!(request.plan().unwrap(), PreparationPlan::AsrRequired);
                assert!(request.subtitles.is_empty());
                assert_eq!(options.source_language, language);
                assert_eq!(
                    options.quality,
                    [
                        TranslationQuality::Fast,
                        TranslationQuality::Standard,
                        TranslationQuality::High
                    ][quality]
                );
                assert!(cloud);
            }
        }
    }
    assert_eq!(state.borrow().starts, 0);
}

#[test]
fn audio_prepare_discloses_audio_and_text_before_start_and_moves_only_selected_key() {
    let owner = Owner::new();
    for (provider, service, key, endpoint) in [
        (
            1,
            SpeechProvider::OpenAi,
            "SYNTHETIC_OPENAI",
            "https://api.openai.com",
        ),
        (
            2,
            SpeechProvider::Gemini,
            "SYNTHETIC_GEMINI",
            "https://generativelanguage.googleapis.com",
        ),
    ] {
        let (dialog, state, _) = owner.dialog();
        audio_input(&dialog, 5, provider);
        let (selected, unused) = if provider == 1 {
            (dialog.openai_key.get(), dialog.gemini_key.get())
        } else {
            (dialog.gemini_key.get(), dialog.openai_key.get())
        };
        set_edit(selected, key).unwrap();
        set_edit(unused, &"X".repeat(KEY_LIMIT + 1)).unwrap();
        let disclosure = text(dialog.destination.get());
        assert!(disclosure.contains(endpoint));
        for required in [
            "按準備後",
            "短片段媒體音訊",
            "辨識原文",
            "翻譯前後文",
            "專名／詞彙",
            "全文脈絡摘要",
            "所選服務與金鑰",
        ] {
            assert!(disclosure.contains(required));
        }
        assert!(!disclosure.contains("不傳送媒體音訊"));
        assert!(!disclosure.contains("Synthetic"));
        assert_eq!(state.borrow().starts, 0);
        click(&dialog, PREPARE);
        assert_eq!(state.borrow().starts, 1);
        assert_eq!(state.borrow().plan, Some(PreparationPlan::AsrRequired));
        assert_eq!(state.borrow().options.unwrap().provider, Some(service));
        assert_eq!(
            state.borrow().options.unwrap().source_language,
            SourceLanguage::Korean
        );
        assert_eq!(
            state.borrow().supplied_key,
            Some(if provider == 1 { "openai" } else { "gemini" })
        );
        assert_keys_cleared(&dialog);
    }
}

#[test]
fn sidecar_priority_ignores_audio_auto_and_retains_its_own_format_and_language() {
    let owner = Owner::new();
    let (dialog, _, _) = owner.dialog();
    for language in 1..=8 {
        fill(&dialog, language, 1);
        select(dialog.audio_language.get(), 0);
        select(dialog.format.get(), 2);
        let (request, options, cloud) = dialog.read_input().unwrap();
        assert_eq!(request.plan().unwrap(), PreparationPlan::Subtitle(0));
        assert_eq!(options.source_language, SourceLanguage::Auto);
        assert_eq!(cloud, language >= 4);
        assert!(request.subtitles[0].format == SubtitleFormat::WebVtt);
        assert!(
            request.subtitles[0].language
                == match language {
                    1 => TextLanguage::TaiwanTraditional,
                    2 => TextLanguage::OtherTraditional,
                    3 => TextLanguage::SimplifiedChinese,
                    4 => TextLanguage::Source(SpeechLanguage::English),
                    5 => TextLanguage::Source(SpeechLanguage::Japanese),
                    6 => TextLanguage::Source(SpeechLanguage::French),
                    7 => TextLanguage::Source(SpeechLanguage::German),
                    _ => TextLanguage::Source(SpeechLanguage::Korean),
                }
        );
    }
    // Nonempty whitespace remains a bad path, never an implicit ASR consent.
    set_edit(dialog.subtitle.get(), "   ").unwrap();
    assert_eq!(
        dialog.read_input().err(),
        Some(PreparedUiError::Media(MediaError::InvalidInput))
    );
}

#[test]
fn native_subtitle_change_switches_fields_and_destination_without_starting_work() {
    let owner = Owner::new();
    let (dialog, state, _) = owner.dialog();
    audio_input(&dialog, 1, 1);
    assert!(unsafe { IsWindowEnabled(dialog.audio_language.get()) }.as_bool());
    assert!(unsafe { IsWindowEnabled(dialog.format.get()) }.as_bool());
    assert!(!unsafe { IsWindowEnabled(dialog.language.get()) }.as_bool());
    assert!(text(dialog.destination.get()).contains("短片段媒體音訊"));
    select(dialog.language.get(), 3);
    set_edit(dialog.subtitle.get(), r"C:\Synthetic\text.srt").unwrap();
    notify(&dialog, SUBTITLE, EN_CHANGE_VALUE, dialog.subtitle.get());
    for control in [dialog.format.get(), dialog.language.get()] {
        assert!(unsafe { IsWindowEnabled(control) }.as_bool());
    }
    assert!(!unsafe { IsWindowEnabled(dialog.audio_language.get()) }.as_bool());
    for control in [
        dialog.provider.get(),
        dialog.openai_key.get(),
        dialog.gemini_key.get(),
    ] {
        assert!(!unsafe { IsWindowEnabled(control) }.as_bool());
    }
    assert!(text(dialog.destination.get()).contains("不使用雲端"));
    select(dialog.language.get(), 4);
    notify(
        &dialog,
        LANGUAGE,
        CBN_SELCHANGE as usize,
        dialog.language.get(),
    );
    assert!(unsafe { IsWindowEnabled(dialog.provider.get()) }.as_bool());
    assert!(text(dialog.destination.get()).contains("不傳送媒體音訊"));
    set_edit(dialog.subtitle.get(), "").unwrap();
    notify(&dialog, SUBTITLE, EN_CHANGE_VALUE, dialog.subtitle.get());
    assert!(unsafe { IsWindowEnabled(dialog.audio_language.get()) }.as_bool());
    assert!(text(dialog.destination.get()).contains("短片段媒體音訊"));
    assert_eq!(state.borrow().starts, 0);
}

#[test]
fn audio_requires_explicit_service_and_empty_selected_key_remains_credential_store_request() {
    let owner = Owner::new();
    let (dialog, state, _) = owner.dialog();
    audio_input(&dialog, 2, 0);
    set_edit(dialog.openai_key.get(), "SYNTHETIC_OPENAI").unwrap();
    click(&dialog, PREPARE);
    assert_eq!(
        dialog.local_error.get(),
        Some(PreparedUiError::ProviderRequired)
    );
    assert_eq!(state.borrow().starts, 0);
    assert_keys_cleared(&dialog);
    audio_input(&dialog, 2, 2);
    set_edit(dialog.openai_key.get(), "invalid unused key").unwrap();
    click(&dialog, PREPARE);
    assert_eq!(state.borrow().starts, 1);
    assert_eq!(state.borrow().supplied_key, None);
    assert_eq!(state.borrow().plan, Some(PreparationPlan::AsrRequired));
    assert_keys_cleared(&dialog);
}

#[test]
fn audio_busy_ready_and_close_lock_all_edits_then_clear_keys_before_native_reaping() {
    let owner = Owner::new();
    let (dialog, state, _) = owner.dialog();
    audio_input(&dialog, 3, 1);
    click(&dialog, PREPARE);
    for status in [
        PreparedUiState::Preparing,
        PreparedUiState::Stopping,
        PreparedUiState::Ready { cues: 2 },
    ] {
        state.borrow_mut().state = status;
        dialog.refresh_state(status).unwrap();
        for control in [
            dialog.media.get(),
            dialog.subtitle.get(),
            dialog.format.get(),
            dialog.language.get(),
            dialog.audio_language.get(),
            dialog.provider.get(),
            dialog.quality.get(),
            dialog.openai_key.get(),
            dialog.gemini_key.get(),
            dialog.controls.borrow()[FIRST_ACTION_CONTROL],
        ] {
            assert!(!unsafe { IsWindowEnabled(control) }.as_bool());
        }
        let disclosure = text(dialog.destination.get());
        notify(
            &dialog,
            PROVIDER,
            CBN_SELCHANGE as usize,
            dialog.provider.get(),
        );
        assert_eq!(text(dialog.destination.get()), disclosure);
        click(&dialog, PREPARE);
        assert_eq!(state.borrow().starts, 1);
    }
    state.borrow_mut().state = PreparedUiState::Preparing;
    set_edit(dialog.openai_key.get(), "SYNTHETIC_OPENAI").unwrap();
    set_edit(dialog.gemini_key.get(), "SYNTHETIC_GEMINI").unwrap();
    click(&dialog, CLOSE);
    assert_eq!(state.borrow().state, PreparedUiState::Stopping);
    assert!(!dialog.hwnd.get().0.is_null());
    assert_keys_cleared(&dialog);
    state.borrow_mut().state = PreparedUiState::Cancelled;
    dialog.poll().unwrap();
    assert!(dialog.hwnd.get().0.is_null());
}

#[test]
fn audio_disclosure_height_and_200_percent_scroll_preserve_the_new_language_row() {
    for dpi in [96, 120, 144, 192] {
        let geometry = scroll_geometry(1000, dpi, i32::MAX);
        let scale = |dip| super::super::subtitle_window::scale(dip, dpi);
        assert!(geometry.viewport + scale(8 + DESTINATION_HEIGHT_DIP) <= geometry.button_y);
        assert!(scale(CONTENT_HEIGHT_DIP) - geometry.position <= geometry.viewport);
        assert!(scale(608) - geometry.position <= geometry.viewport);
    }
    let owner = Owner::new();
    let (dialog, _, _) = owner.dialog();
    audio_input(&dialog, 1, 2);
    unsafe {
        SetWindowPos(
            dialog.hwnd.get(),
            None,
            0,
            0,
            640,
            420,
            SWP_NOZORDER | SWP_NOACTIVATE,
        )
        .unwrap();
    }
    dialog.layout().unwrap();
    dialog
        .scroll_to(super::super::subtitle_window::scale(208, unsafe {
            GetDpiForWindow(dialog.hwnd.get())
        }))
        .unwrap();
    let mut viewport = RECT::default();
    let mut row = RECT::default();
    unsafe {
        GetWindowRect(dialog.content.get(), &mut viewport).unwrap();
        GetWindowRect(dialog.controls.borrow()[21], &mut row).unwrap();
    }
    assert!(row.top >= viewport.top && row.bottom <= viewport.bottom);
    let mut destination = RECT::default();
    let mut prepare = RECT::default();
    unsafe {
        GetWindowRect(dialog.destination.get(), &mut destination).unwrap();
        GetWindowRect(dialog.controls.borrow()[FIRST_ACTION_CONTROL], &mut prepare).unwrap();
    }
    assert!(destination.bottom <= prepare.top);
    assert!(
        destination.bottom - destination.top
            >= super::super::subtitle_window::scale(DESTINATION_HEIGHT_DIP, unsafe {
                GetDpiForWindow(dialog.hwnd.get())
            })
    );
}

#[test]
fn native_wrapped_disclosure_uses_real_font_metrics_in_narrow_windows_and_double_height_font() {
    use windows::Win32::Graphics::Gdi::{
        CLIP_DEFAULT_PRECIS, CreateFontW, DEFAULT_CHARSET, DEFAULT_QUALITY, DeleteObject, HFONT,
        OUT_DEFAULT_PRECIS,
    };
    struct Font(HFONT);
    impl Drop for Font {
        fn drop(&mut self) {
            unsafe {
                let _ = DeleteObject(self.0.into());
            }
        }
    }
    let owner = Owner::new();
    // Synthetic 26px font exercises doubled text dimensions even on a 96-DPI
    // test monitor. It is not evidence of a real monitor DPI/user cloud run.
    let font = Font(unsafe {
        CreateFontW(
            -26,
            0,
            0,
            0,
            400,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            DEFAULT_QUALITY,
            0,
            w!("Segoe UI"),
        )
    });
    assert!(!font.0.0.is_null());
    let mut variants = vec![
        (1, false, 0, None),
        (2, false, 0, None),
        (1, true, 0, None),
        (2, true, 0, None),
        (1, true, 4, None),
        (2, true, 4, None),
        (0, true, 1, None),
    ];
    for format in [HTTPS_SRT_FORMAT_INDEX, HTTPS_WEBVTT_FORMAT_INDEX] {
        for remote in [false, true] {
            for (provider, language) in [(0, 1), (1, 4), (2, 4), (0, 4)] {
                variants.push((provider, remote, language, Some(format)));
            }
        }
    }
    for (provider, remote, text_language, https_format) in variants {
        let (dialog, _, _) = owner.dialog();
        audio_input(&dialog, 1, provider);
        if remote {
            set_edit(
                dialog.media.get(),
                "https://synthetic.invalid/direct.wav?private=SYNTHETIC_QUERY",
            )
            .unwrap();
            notify(&dialog, MEDIA, EN_CHANGE_VALUE, dialog.media.get());
        }
        if text_language != 0 {
            select(
                dialog.format.get(),
                https_format.map_or(1, |format| format as usize),
            );
            select(dialog.language.get(), text_language);
            set_edit(
                dialog.subtitle.get(),
                if https_format.is_some() {
                    "https://synthetic-subtitles.invalid/private?token=SYNTHETIC_SUBTITLE_QUERY"
                } else {
                    r"C:\Synthetic\manual.srt"
                },
            )
            .unwrap();
            dialog.fields_changed().unwrap();
        }
        for scaled_font in [false, true] {
            if scaled_font {
                unsafe {
                    SendMessageW(
                        dialog.destination.get(),
                        WM_SETFONT,
                        Some(WPARAM(font.0.0 as usize)),
                        Some(LPARAM(1)),
                    );
                }
            }
            for width in [320, 480, 640] {
                unsafe {
                    SetWindowPos(
                        dialog.hwnd.get(),
                        None,
                        0,
                        0,
                        width,
                        680,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    )
                    .unwrap();
                }
                dialog.layout().unwrap();
                let mut rect = RECT::default();
                let mut button = RECT::default();
                unsafe {
                    GetWindowRect(dialog.destination.get(), &mut rect).unwrap();
                    GetWindowRect(dialog.controls.borrow()[FIRST_ACTION_CONTROL], &mut button)
                        .unwrap();
                }
                let measured = destination_text_height(dialog.destination.get(), unsafe {
                    GetDpiForWindow(dialog.hwnd.get())
                })
                .unwrap();
                assert!(
                    rect.bottom - rect.top >= measured,
                    "clipped disclosure: provider={provider}, remote={remote}, text_language={text_language}, https_format={https_format:?}, width={width}, scaled_font={scaled_font}, actual_height={}, measured={measured}",
                    rect.bottom - rect.top
                );
                assert!(
                    rect.bottom <= button.top,
                    "disclosure overlaps actions: provider={provider}, remote={remote}, text_language={text_language}, https_format={https_format:?}, width={width}, scaled_font={scaled_font}, bottom={}, button_top={}",
                    rect.bottom,
                    button.top
                );
                // The Chinese path has source and local-processing paragraphs;
                // cloud paths also have a separate provider destination. Native
                // wrapping may add lines, while the measured-height and toolbar
                // checks above must hold for every actual line/font/width.
                let minimum_lines = if https_format.is_some() {
                    if text_language <= 3 { 3 } else { 4 }
                } else if provider == 0 {
                    2
                } else {
                    3
                };
                let lines = unsafe {
                    SendMessageW(dialog.destination.get(), EM_GETLINECOUNT_VALUE, None, None).0
                };
                assert!(
                    lines >= minimum_lines,
                    "disclosure lost paragraphs: provider={provider}, remote={remote}, text_language={text_language}, https_format={https_format:?}, width={width}, scaled_font={scaled_font}, actual_lines={lines}"
                );
                let disclosure = text(dialog.destination.get());
                if https_format.is_some() {
                    assert!(disclosure.contains("連線指定 HTTPS 字幕"));
                    assert!(disclosure.contains("讀取全文"));
                    assert!(disclosure.contains("本次記憶體"));
                    assert!(disclosure.contains("不存檔或播放"));
                    assert!(!disclosure.contains("SYNTHETIC_QUERY"));
                    assert!(!disclosure.contains("SYNTHETIC_SUBTITLE_QUERY"));
                    assert!(!disclosure.contains("synthetic-subtitles.invalid"));
                    assert!(!disclosure.contains("短片段媒體音訊"));
                    if remote {
                        assert!(disclosure.contains("先讀遠端WAVE標頭"));
                    } else {
                        assert!(disclosure.contains("先唯讀持有本機媒體"));
                    }
                } else if remote {
                    assert!(disclosure.contains("連線至輸入的 HTTP(S) WAVE 網址"));
                    assert!(!disclosure.contains("SYNTHETIC_QUERY"));
                }
                if https_format.is_some() && text_language >= 4 {
                    assert!(disclosure.contains("不傳音訊或任何來源網址"));
                } else if text_language == 0 {
                    assert!(disclosure.contains("短片段媒體音訊"));
                } else if https_format.is_none() {
                    assert!(disclosure.contains("僅讀取媒體標頭"));
                    assert!(disclosure.contains("不向 AI 傳送媒體音訊或來源網址"));
                    assert!(!disclosure.contains("短片段媒體音訊"));
                }
                if provider == 0 && text_language >= 4 {
                    assert!(disclosure.contains("AI 尚未選擇"));
                } else if provider == 0 {
                    assert!(disclosure.contains("不使用 AI 或金鑰"));
                } else {
                    assert!(disclosure.contains(if provider == 1 {
                        "https://api.openai.com"
                    } else {
                        "https://generativelanguage.googleapis.com"
                    }));
                }
            }
        }
        // Dialog drops (and destroys EDIT) before its borrowed font owner.
    }
}
