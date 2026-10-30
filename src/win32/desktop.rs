use crate::error::{AppError, Result};
use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::Threading::CreateMutexW,
        UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW},
    },
    core::{HSTRING, w},
};

pub struct RunningGuard(HANDLE);
impl RunningGuard {
    pub fn new() -> Result<Self> {
        // SAFETY: named kernel object has no pointers retained and is only a
        // running-instance marker for Setup, not a global synchronization lock.
        unsafe { CreateMutexW(None, false, w!("Local\\SubtitleNow.Running")) }
            .map(Self)
            .map_err(|_| AppError::SubtitleWindow)
    }
}
impl Drop for RunningGuard {
    fn drop(&mut self) {
        // SAFETY: this guard uniquely owns the handle returned by CreateMutexW.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
pub fn show_startup_error(text: &str) {
    // SAFETY: synchronous native dialog copies sanitized AppError text.
    unsafe {
        MessageBoxW(
            None,
            &HSTRING::from(text),
            w!("SubtitleNow 無法啟動"),
            MB_OK | MB_ICONERROR,
        );
    }
}
