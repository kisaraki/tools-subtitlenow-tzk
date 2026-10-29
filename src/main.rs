use std::process::ExitCode;

use subtitlenow::{app, logging};

fn main() -> ExitCode {
    #[cfg(windows)]
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--capture-helper")) {
        return if subtitlenow::win32::capture_helper::run().is_ok() {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    }
    if let Err(error) = logging::init() {
        eprintln!("{error}");
        return ExitCode::FAILURE;
    }

    match app::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // Display is deliberately sanitized; never log the source chain or raw config.
            tracing::error!(code = error.code(), error = %error, "作業失敗");
            ExitCode::FAILURE
        }
    }
}
