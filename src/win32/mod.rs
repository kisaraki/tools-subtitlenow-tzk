//! Windows integration. Native COM resources stay on their owning discovery/capture worker.

#![allow(unsafe_code)]

#[cfg(windows)]
mod about_window;
#[cfg(windows)]
pub mod desktop;

#[cfg(windows)]
pub mod audio;
#[cfg(windows)]
pub mod capture;
#[cfg(windows)]
pub mod capture_helper;
#[cfg(windows)]
pub mod credentials;
#[cfg(windows)]
mod icon;
#[cfg(windows)]
mod loopback_clock;
#[cfg(windows)]
pub(crate) mod media_decode;
#[cfg(windows)]
pub(crate) mod media_input;
#[cfg(all(test, windows))]
pub(crate) mod media_mp3_fixture;
#[cfg(windows)]
mod media_stream;
#[cfg(windows)]
pub(crate) mod movie_metadata;
#[cfg(windows)]
pub mod performance;
#[cfg(windows)]
mod prepared_dialog;
#[cfg(windows)]
mod process;
#[cfg(windows)]
mod resources;
#[cfg(windows)]
mod source_window;
#[cfg(windows)]
mod subtitle_controls;
#[cfg(windows)]
mod subtitle_monitor;
#[cfg(windows)]
mod subtitle_render;
#[cfg(windows)]
mod subtitle_source_dialog;
#[cfg(windows)]
mod subtitle_theme;
#[cfg(windows)]
pub mod subtitle_window;

#[cfg(all(test, windows))]
mod soak_tests;
#[cfg(all(test, windows))]
mod tests;
