//! Read-only process/window metadata. No hooks, remote messages, or HWND capture identity.

use std::{collections::BTreeMap, path::Path};

use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE, HWND, LPARAM},
        System::Threading::{
            OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
            QueryFullProcessImageNameW,
        },
        UI::WindowsAndMessaging::{
            EnumWindows, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
        },
    },
    core::{BOOL, PWSTR},
};

use crate::audio::sessions::{MAX_TITLE_CHARS, MAX_WINDOWS};

struct ProcessHandle(HANDLE);
pub(super) fn creation_time_for(process: HANDLE) -> Option<u64> {
    use windows::Win32::{Foundation::FILETIME, System::Threading::GetProcessTimes};
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: borrowed live process handle with QUERY_LIMITED_INFORMATION, four
    // distinct initialized out parameters; caller retains handle ownership.
    unsafe { GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user) }.ok()?;
    Some((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
}
pub(super) fn creation_time(pid: u32) -> Option<u64> {
    // SAFETY: query-only owning handle; ProcessHandle closes it exactly once.
    let process =
        ProcessHandle(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?);
    creation_time_for(process.0)
}
impl Drop for ProcessHandle {
    fn drop(&mut self) {
        // SAFETY: this is the single owning handle returned by OpenProcess.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

pub(super) fn executable(pid: u32) -> Option<String> {
    const MAX_PATH_UNITS: usize = 32_768;
    // SAFETY: query-only access; zero/terminated/protected PIDs simply return an error.
    let process =
        ProcessHandle(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?);
    let mut path = vec![0_u16; MAX_PATH_UNITS];
    let mut length = path.len() as u32;
    // SAFETY: live handle, correctly sized writable buffer, and initialized in/out length.
    unsafe {
        QueryFullProcessImageNameW(
            process.0,
            PROCESS_NAME_WIN32,
            PWSTR(path.as_mut_ptr()),
            &mut length,
        )
    }
    .ok()?;
    String::from_utf16(path.get(..length as usize)?).ok()
}

pub(super) fn executable_name(path: &str) -> Option<String> {
    Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
}

#[derive(Default)]
pub(super) struct WindowTitles {
    pub titles: BTreeMap<u32, String>,
    pub unavailable: bool,
    pub limit_reached: bool,
    visited: usize,
}

pub(super) fn window_titles() -> WindowTitles {
    let mut result = WindowTitles::default();
    // SAFETY: EnumWindows calls synchronously; the stack context stays live, exclusively
    // borrowed during enumeration. Callback never stores pointers or unwinds across FFI.
    let status = unsafe {
        EnumWindows(
            Some(visit_window),
            LPARAM((&mut result as *mut WindowTitles) as isize),
        )
    };
    result.unavailable = status.is_err() && !result.limit_reached;
    result
}

unsafe extern "system" fn visit_window(window: HWND, context: LPARAM) -> BOOL {
    // SAFETY: only window_titles supplies this pointer, valid through the synchronous call.
    let result = unsafe { &mut *(context.0 as *mut WindowTitles) };
    if result.visited >= MAX_WINDOWS {
        result.limit_reached = true;
        return false.into();
    }
    result.visited += 1;
    // SAFETY: HWND comes from EnumWindows. Races with window closure are handled by failure.
    if !unsafe { IsWindowVisible(window) }.as_bool() {
        return true.into();
    }
    let mut pid = 0;
    let mut title = [0_u16; MAX_TITLE_CHARS + 1];
    // SAFETY: valid out pointer and bounded title buffer. GetWindowText reads captions without
    // SendMessage to another process; no WM_GETTEXT or WM_GETICON calls to hung applications.
    let length = unsafe {
        GetWindowThreadProcessId(window, Some(&mut pid));
        GetWindowTextW(window, &mut title)
    };
    if pid != 0
        && length > 0
        && let Some(units) = title.get(..length as usize)
    {
        result
            .titles
            .entry(pid)
            .or_insert_with(|| String::from_utf16_lossy(units));
    }
    true.into()
}
