//! Shared transcript timeline, stabilization, and alignment boundary.
//! Future renderers consume the same timeline; content is bounded and session-scoped.
pub(crate) mod alignment_text;
pub mod alignment_verification;
mod alignment_work;
pub mod exact_alignment;
pub mod existing_text;
pub mod fuzzy_alignment;
pub mod fuzzy_track;
pub mod fuzzy_worker;
pub mod localization;
pub mod source_arbiter;
pub mod source_translation;
pub mod timeline;
