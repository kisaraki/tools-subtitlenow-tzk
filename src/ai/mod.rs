//! Phase 3 provider-neutral streaming speech boundary. No implicit network access.
pub mod credentials;
mod input_level;
mod language_detection;
mod live_translate_wire;
pub mod native_caption;
mod openai_early;
pub mod prepared_context;
pub mod prepared_glossary;
pub mod prepared_recognition;
pub mod prepared_speech;
pub mod prepared_speech_http;
pub(crate) mod prepared_speech_wire;
pub mod prepared_transcript;
pub mod prepared_transcript_translation;
pub(crate) mod prepared_translation;
mod preprocess_worker;
pub mod provider;
pub(crate) mod service_check;
pub mod speech;
pub mod speech_progress;
pub mod streaming;
pub mod transcript_adapter;
pub mod translation;
pub mod translation_http;

#[cfg(test)]
mod prepared_pipeline_tests;
