//! Windows capture, streaming STT/translation and shared native subtitle views.
//! Phase 7 adds session-only Chinese subtitle normalization; source arbitration,
//! real-provider and player acceptance remain pending.

pub mod ai;
pub mod app;
pub mod audio;
pub mod domain;
pub mod error;
pub mod logging;
pub mod media;
pub mod performance;
pub mod resolver;
pub mod session;
pub mod subtitle;
pub mod ui;
pub mod update;
pub mod win32;
