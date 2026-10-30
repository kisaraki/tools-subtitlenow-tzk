#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() -> std::process::ExitCode {
    #[cfg(windows)]
    {
        if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--capture-helper")) {
            return if subtitlenow::win32::capture_helper::run().is_ok() {
                std::process::ExitCode::SUCCESS
            } else {
                std::process::ExitCode::FAILURE
            };
        }
        subtitlenow::update::cleanup_stale();
        if let Err(error) = subtitlenow::app::run_desktop() {
            subtitlenow::win32::desktop::show_startup_error(&error.to_string());
            return std::process::ExitCode::FAILURE;
        }
        std::process::ExitCode::SUCCESS
    }
    #[cfg(not(windows))]
    std::process::ExitCode::FAILURE
}
