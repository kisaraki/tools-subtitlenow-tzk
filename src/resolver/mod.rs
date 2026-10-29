//! Session-only content resolution, independent of STT and rendering.
//! Adapters must be explicitly configured; no production identifier is enabled yet.
pub mod audio_worker;
pub mod candidates;
pub mod catalog;
pub mod contracts;
pub mod fingerprint;
pub mod fingerprint_worker;
pub mod known_subtitle;
mod lrc;
pub mod lyrics;
pub mod movie;
pub mod movie_hints;
pub mod session;
pub mod text_session;
pub mod timed_subtitle;
pub(crate) mod timed_subtitle_parse;
pub mod window;

pub use contracts::{ContentType, ResolverError, SubtitleSource};

#[cfg(test)]
mod tests;
