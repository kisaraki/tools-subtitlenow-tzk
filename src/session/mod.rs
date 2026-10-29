//! Session identity, cancellation, and RAM-only ownership boundary.
//! Owns cancellable live translation work; no global content store exists.
pub mod aligned_playback;
pub mod alignment_control;
pub mod fuzzy_proposals;
pub mod live_resolver;
pub mod live_text;
pub mod live_translation;
pub mod playback_clock;
pub mod prepared;
pub mod text_preparation;
