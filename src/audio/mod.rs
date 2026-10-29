//! Audio discovery, process capture, and streaming preprocessing boundary (Phases 1–3).
//! Capture only timestamps and copies audio into a bounded buffer.
//! No provider requests, disk I/O, UI work, or blocking waits belong on that thread.

pub mod capture;
pub mod discovery;
pub mod preprocess;
pub mod resample;
pub mod sessions;
