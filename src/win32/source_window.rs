//! Best-effort monitor association for the explicitly selected PID, never a capture source.
use super::process;
use crate::audio::sessions::MAX_WINDOWS;
use std::os::windows::io::{FromRawHandle, OwnedHandle};
use windows::{
    Win32::{
        Foundation::WAIT_TIMEOUT,
        Foundation::{HWND, LPARAM, RECT},
        Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONULL, MONITORINFO, MonitorFromWindow},
        System::Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
            WaitForSingleObject,
        },
        UI::WindowsAndMessaging::{
            EnumWindows, GWL_EXSTYLE, GetForegroundWindow, GetWindowLongPtrW, GetWindowRect,
            GetWindowThreadProcessId, IsIconic, IsWindowVisible, WS_EX_TOOLWINDOW,
        },
    },
    core::BOOL,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum FollowError {
    NoSource,
    Identity,
    NoWindow,
    Unavailable,
}
impl FollowError {
    pub fn notice(self) -> &'static str {
        match self {
            Self::NoSource => "跟隨來源視窗：請先選擇來源；字幕保留目前位置。",
            Self::Identity => "跟隨來源視窗：來源已結束或無法驗證身分；字幕保留目前位置。",
            Self::NoWindow => "跟隨來源視窗：該程序沒有可見的非最小化視窗；字幕保留目前位置。",
            Self::Unavailable => "跟隨來源視窗：無法查詢視窗或螢幕；字幕保留目前位置。",
        }
    }
}
struct Search {
    pid: u32,
    foreground: HWND,
    best: Option<HWND>,
    score: (bool, i64),
    visited: usize,
    limited: bool,
}
fn rank(rect: RECT, foreground: bool) -> Option<(bool, i64)> {
    let width = i64::from(rect.right) - i64::from(rect.left);
    let height = i64::from(rect.bottom) - i64::from(rect.top);
    (width > 0 && height > 0).then(|| (foreground, width.saturating_mul(height)))
}
pub(super) fn work_area(pid: u32, created: u64) -> std::result::Result<RECT, FollowError> {
    if pid == 0 || created == 0 {
        return Err(FollowError::Identity);
    }
    // SAFETY: query/synchronize-only process handle, uniquely owned and closed on every exit.
    let native = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            false,
            pid,
        )
    }
    .map_err(|_| FollowError::Identity)?;
    // SAFETY: successful OpenProcess returns an owned (non-pseudo) handle.
    let owned = unsafe { OwnedHandle::from_raw_handle(native.0) };
    if process::creation_time_for(native) != Some(created)
        // SAFETY: held live SYNCHRONIZE handle, zero-time query never waits for termination.
        || unsafe { WaitForSingleObject(native, 0) } != WAIT_TIMEOUT
    {
        return Err(FollowError::Identity);
    }
    // SAFETY: query-only foreground metadata, no activation or input injection.
    let mut search = Search {
        pid,
        foreground: unsafe { GetForegroundWindow() },
        best: None,
        score: (false, 0),
        visited: 0,
        limited: false,
    };
    // SAFETY: exclusive stack context survives synchronous bounded enumeration.
    if unsafe { EnumWindows(Some(visit), LPARAM((&mut search as *mut Search) as isize)) }.is_err()
        || search.limited
    {
        return Err(FollowError::Unavailable);
    }
    let window = search.best.ok_or(FollowError::NoWindow)?;
    let mut actual_pid = 0;
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: recheck window ownership after enumeration; only short native queries,
    // no messages to another process. Held process handle prevents matching a reused PID.
    unsafe {
        if GetWindowThreadProcessId(window, Some(&mut actual_pid)) == 0
            || actual_pid != pid
            || !IsWindowVisible(window).as_bool()
            || IsIconic(window).as_bool()
        {
            return Err(FollowError::NoWindow);
        }
        let monitor = MonitorFromWindow(window, MONITOR_DEFAULTTONULL);
        if monitor.0.is_null() || !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return Err(FollowError::Unavailable);
        }
        if WaitForSingleObject(native, 0) != WAIT_TIMEOUT {
            return Err(FollowError::Identity);
        }
        actual_pid = 0;
        if GetWindowThreadProcessId(window, Some(&mut actual_pid)) == 0 || actual_pid != pid {
            return Err(FollowError::NoWindow);
        }
    }
    drop(owned);
    if info.rcWork.right <= info.rcWork.left || info.rcWork.bottom <= info.rcWork.top {
        return Err(FollowError::Unavailable);
    }
    Ok(info.rcWork)
}
unsafe extern "system" fn visit(window: HWND, context: LPARAM) -> BOOL {
    // SAFETY: unique initialized context from work_area; no allocation or panic in callback.
    let search = unsafe { &mut *(context.0 as *mut Search) };
    if search.visited >= MAX_WINDOWS {
        search.limited = true;
        return false.into();
    }
    search.visited += 1;
    let mut pid = 0;
    let mut rect = RECT::default();
    // SAFETY: handles from EnumWindows; races are treated as unavailable. No foreign
    // WM_GETTEXT/WM_GETICON, hooks, child-window traversal or audio operations.
    unsafe {
        if GetWindowThreadProcessId(window, Some(&mut pid)) == 0
            || pid != search.pid
            || !IsWindowVisible(window).as_bool()
            || IsIconic(window).as_bool()
            || GetWindowLongPtrW(window, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 != 0
            || GetWindowRect(window, &mut rect).is_err()
        {
            return true.into();
        }
    }
    if let Some(score) = rank(rect, window == search.foreground)
        && score > search.score
    {
        search.score = score;
        search.best = Some(window);
    }
    true.into()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn foreground_window_precedes_largest_window_and_empty_geometry_is_ineligible() {
        let small = RECT {
            left: -300,
            top: 0,
            right: -200,
            bottom: 100,
        };
        let large = RECT {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };
        assert!(rank(small, true) > rank(large, false));
        assert!(rank(large, false) > rank(small, false));
        assert_eq!(rank(RECT::default(), true), None);
    }
    #[test]
    fn reused_or_unverified_process_is_rejected_before_enumerating_windows() {
        let created = process::creation_time(std::process::id()).unwrap();
        assert_eq!(
            work_area(std::process::id(), created + 1),
            Err(FollowError::Identity)
        );
        assert_eq!(work_area(0, created), Err(FollowError::Identity));
        assert_eq!(work_area(std::process::id(), 0), Err(FollowError::Identity));
    }
    #[test]
    fn native_own_fixture_window_resolves_its_actual_monitor_without_audio_or_activation() {
        use windows::{
            Win32::UI::WindowsAndMessaging::{
                CreateWindowExW, DestroyWindow, WS_EX_NOACTIVATE, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
            },
            core::w,
        };
        struct Fixture(HWND);
        impl Drop for Fixture {
            fn drop(&mut self) {
                // SAFETY: test owns this window and destroys it on the creating thread.
                unsafe {
                    let _ = DestroyWindow(self.0);
                }
            }
        }
        // SAFETY: built-in class, static strings, no borrowed callback data; own non-activated
        // native fixture only, never another app's window and never an audio source.
        let fixture = Fixture(
            unsafe {
                CreateWindowExW(
                    WS_EX_NOACTIVATE,
                    w!("STATIC"),
                    w!("SubtitleNow native follow test"),
                    WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                    40,
                    40,
                    320,
                    180,
                    None,
                    None,
                    None,
                    None,
                )
            }
            .unwrap(),
        );
        let created = process::creation_time(std::process::id()).unwrap();
        let mut expected = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        // SAFETY: query the test-owned window and initialized monitor out structure.
        unsafe {
            GetMonitorInfoW(
                MonitorFromWindow(fixture.0, MONITOR_DEFAULTTONULL),
                &mut expected,
            )
            .ok()
            .unwrap();
        }
        assert_eq!(
            work_area(std::process::id(), created).unwrap(),
            expected.rcWork
        );
    }
}
