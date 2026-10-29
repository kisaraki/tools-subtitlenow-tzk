//! Isolated, hidden UI resource checks. No source capture, credentials or cloud calls.
use super::*;
use crate::ui::controls::{SourceChoice, StartRequest};
use std::{
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use windows::Win32::System::Threading::{
    GET_GUI_RESOURCES_FLAGS, GR_GDIOBJECTS, GR_USEROBJECTS, GetCurrentProcess, GetGuiResources,
};
use windows::Win32::UI::Controls::{ODS_DISABLED, ODS_FLAGS, ODS_FOCUS, ODS_SELECTED, ODT_BUTTON};

const CHILD_MARKER: &str = "SUBTITLENOW_TEST_UI_RESOURCE_CHILD";
const FIXTURE_NAME: &str = "win32::subtitle_window::validation_tests::isolated_ui_resource_fixture";
const CHILD_TIMEOUT: Duration = Duration::from_secs(60);
const CHILD_POLL: Duration = Duration::from_millis(20);
const RESOURCE_TOLERANCE: u32 = 2;
const WARMUP_CYCLES: usize = 3;
const BATCHES: usize = 3;
const CYCLES_PER_BATCH: usize = 20;

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn repeated_hidden_ui_cycles_keep_gdi_and_user_resources_bounded_in_isolated_process() {
    let mut child = ChildGuard(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                FIXTURE_NAME,
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD_MARKER, "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + CHILD_TIMEOUT;
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "isolated UI resource check exceeded deadline"
        );
        std::thread::sleep(CHILD_POLL);
    };
    use std::io::Read;
    let mut output = String::new();
    child
        .0
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut output)
        .unwrap();
    child
        .0
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut output)
        .unwrap();
    assert!(
        status.success(),
        "isolated UI resource check failed: {output}"
    );
    assert!(
        output.contains("UI_RESOURCE_CHECK_COMPLETE"),
        "child never completed fixture: {output}"
    );
    println!("{output}");
}

struct MockIdleController(Arc<Vec<SourceChoice>>);
impl UiController for MockIdleController {
    fn poll(&mut self) -> ControlUpdate {
        ControlUpdate {
            sources: self.0.clone(),
            ..Default::default()
        }
    }
    fn start(
        &mut self,
        _: StartRequest,
        _: Option<crate::ai::credentials::ApiKey>,
    ) -> Result<UiInbox> {
        Err(crate::ui::bridge::UiBridgeError::InvalidState.into())
    }
    fn request_stop(&mut self) {}
}
fn cycle() {
    let shell = SubtitleShell::allocate(DisplayMode::SideChat, false);
    let mut source = crate::ui::controls::mock_source(42);
    source.icon = Some(Arc::new(crate::audio::sessions::ApplicationIcon {
        width: 1,
        height: 1,
        bgra: vec![0, 0, 255, 255],
    }));
    source.peak = Some(0.5);
    let sources = Arc::new(crate::ui::controls::choices(
        &crate::audio::sessions::ScanReport {
            sources: vec![source],
            ..Default::default()
        },
    ));
    *shell.controller.borrow_mut() = Some(Box::new(MockIdleController(sources)));
    shell.create(false).unwrap();
    shell.poll_controller().unwrap();
    let combo = shell.panel.borrow().as_ref().unwrap().source;
    // SAFETY: private test selector; no backend start or external process operation.
    unsafe {
        SendMessageW(combo, CB_SETCURSEL, Some(WPARAM(1)), None);
    }
    shell.command(DUAL).unwrap();
    shell.command(THROUGH).unwrap();
    shell.command(THROUGH).unwrap();
    shell.update_font().unwrap(); // replace old selected font on the same thread
    // Exercise theme replacement plus normal/pressed/disabled/focused button
    // painting in the existing isolated GDI/USER lifetime check. No real content.
    let _ = shell.theme_message(shell.main.get(), WM_THEMECHANGED, WPARAM(0), LPARAM(0));
    let buttons = shell.panel.borrow().as_ref().unwrap().buttons();
    for button in buttons {
        for state in [ODS_FLAGS(0), ODS_SELECTED, ODS_DISABLED, ODS_FOCUS] {
            super::subtitle_render::inspect_test_canvas(320, 40, |dc| {
                // SAFETY: native handles and DIB belong to this test's UI thread;
                // the stack structure is only borrowed by synchronous WM_DRAWITEM.
                unsafe {
                    let original_color = GetTextColor(dc);
                    let original_brush = GetCurrentObject(dc, OBJ_BRUSH);
                    let draw = DRAWITEMSTRUCT {
                        CtlType: ODT_BUTTON,
                        CtlID: GetDlgCtrlID(button) as u32,
                        hwndItem: button,
                        hDC: dc,
                        itemState: state,
                        rcItem: RECT {
                            left: 0,
                            top: 0,
                            right: 320,
                            bottom: 40,
                        },
                        ..Default::default()
                    };
                    assert_eq!(
                        SendMessageW(
                            shell.main.get(),
                            WM_DRAWITEM,
                            None,
                            Some(LPARAM((&draw as *const DRAWITEMSTRUCT) as isize))
                        )
                        .0,
                        1
                    );
                    assert_eq!(GetTextColor(dc), original_color);
                    assert_eq!(GetCurrentObject(dc, OBJ_BRUSH), original_brush);
                }
                Ok(())
            })
            .unwrap();
        }
    }
    shell.command(REFRESH_MONITORS).unwrap();
    super::subtitle_render::inspect_test_canvas(320, 40, |dc| {
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
        // SAFETY: private stack structure/DIB survive synchronous owner-draw callback.
        unsafe {
            SendMessageW(
                shell.main.get(),
                WM_DRAWITEM,
                Some(WPARAM(subtitle_controls::SOURCE)),
                Some(LPARAM((&draw as *const DRAWITEMSTRUCT) as isize)),
            );
        }
        Ok(())
    })
    .unwrap();
    assert!(!shell.failed.get());
    drop(shell);
    // SAFETY: flush only this test thread's completed native drawing before accounting.
    unsafe {
        GdiFlush().ok().unwrap();
    }
}
fn resource_count(flags: GET_GUI_RESOURCES_FLAGS) -> u32 {
    // SAFETY: borrowed current-process pseudo handle; zero can be valid, so last-error
    // is reset/checked separately. Never closes the pseudo handle or queries another app.
    unsafe {
        windows::Win32::Foundation::SetLastError(windows::Win32::Foundation::WIN32_ERROR(0));
        let count = GetGuiResources(GetCurrentProcess(), flags);
        if count == 0 {
            assert_eq!(windows::Win32::Foundation::GetLastError().0, 0);
        }
        count
    }
}

#[test]
#[ignore = "child-only isolated UI resource fixture; exercised by regular parent test"]
fn isolated_ui_resource_fixture() {
    assert_eq!(std::env::var(CHILD_MARKER).as_deref(), Ok("1"));
    for _ in 0..WARMUP_CYCLES {
        cycle();
    }
    let baseline = (
        resource_count(GR_GDIOBJECTS),
        resource_count(GR_USEROBJECTS),
    );
    for batch in 0..BATCHES {
        for _ in 0..CYCLES_PER_BATCH {
            cycle();
        }
        let current = (
            resource_count(GR_GDIOBJECTS),
            resource_count(GR_USEROBJECTS),
        );
        println!(
            "UI_RESOURCES batch={} baseline_gdi={} baseline_user={} gdi={} user={}",
            batch + 1,
            baseline.0,
            baseline.1,
            current.0,
            current.1
        );
        assert!(
            current.0 <= baseline.0 + RESOURCE_TOLERANCE,
            "GDI objects accumulated"
        );
        assert!(
            current.1 <= baseline.1 + RESOURCE_TOLERANCE,
            "USER objects accumulated"
        );
    }
    println!(
        "UI_RESOURCE_CHECK_COMPLETE cycles={}",
        BATCHES * CYCLES_PER_BATCH
    );
}
