//! Caller-owned local WAVE/MP3 -> bounded offline recognition. The caller selects
//! the recognizer explicitly; this library entry point supplies no credentials,
//! default provider, GUI, Ready document, playback or persistent media content.
use super::{
    MediaError,
    input::{MediaInput, OpenedMedia},
    prepared_audio::PreparedSampleRate,
    prepared_producer::{MAX_LOCAL_BATCH_CHUNKS, PreparedLocalSource},
};
use crate::{
    ai::{
        prepared_recognition::PreparedRecognitionSession,
        prepared_speech::{MAX_ASR_BATCH_CHUNKS, PreparedSpeechError, PreparedSpeechRecognizer},
        prepared_transcript::PreparedTranscript,
        speech::StreamIdentity,
    },
    domain::SourceLanguage,
    resolver::contracts::ResolverCancellation,
};
use std::{future::Future, panic::AssertUnwindSafe};

/// Messages delegate only to the safe domain errors, never private native or
/// provider payloads, paths, audio, credentials or recognized source text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PreparedAsrError {
    #[error("{0}")]
    Media(#[from] MediaError),
    #[error("{0}")]
    Speech(#[from] PreparedSpeechError),
}

/// The original readonly file object and its complete recognized source stay
/// together. No path reopen, publication provenance, playback or Ready is made.
pub(crate) struct RecognizedLocalMedia {
    pub(crate) media: OpenedMedia,
    pub(crate) transcript: PreparedTranscript,
}

struct CancelOnDrop {
    cancel: ResolverCancellation,
    armed: bool,
}
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if self.armed {
            self.cancel.cancel();
        }
    }
}

/// The owner must poll the future. Construction starts no I/O or provider work;
/// dropping even an unpolled future cancels the entire preparation scope.
///
/// Only validated source transcript EOF can succeed. Every native pull yields
/// at most two chunks; only a batch of zero to two chunks may pull again. Three
/// or four chunks are recognized before any further media is decoded, keeping
/// PCM bounded while the provider waits and releasing the shared CPU lane.
pub fn recognize_local_media<'a>(
    input: MediaInput,
    identity: StreamIdentity,
    language: SourceLanguage,
    rate: PreparedSampleRate,
    cancel: ResolverCancellation,
    provider: &'a dyn PreparedSpeechRecognizer,
) -> impl Future<Output = Result<PreparedTranscript, PreparedAsrError>> + Send + 'a {
    // Construct the guarded future now, so this mapping cannot delay the
    // original cancellation guard until its first poll.
    let work = recognize_pinned_local_media(input, identity, language, rate, cancel, provider);
    async move { work.await.map(|recognized| recognized.transcript) }
}

/// Private preparation path: the same original file pin survives native EOF,
/// the final recognizer wait and ownership transfer to the Session composer.
pub(crate) fn recognize_pinned_local_media<'a>(
    input: MediaInput,
    identity: StreamIdentity,
    language: SourceLanguage,
    rate: PreparedSampleRate,
    cancel: ResolverCancellation,
    provider: &'a dyn PreparedSpeechRecognizer,
) -> impl Future<Output = Result<RecognizedLocalMedia, PreparedAsrError>> + Send + 'a {
    let guard = CancelOnDrop {
        cancel: cancel.clone(),
        armed: true,
    };
    async move {
        let mut guard = guard;
        let result = run(input, identity, language, rate, &cancel, provider).await;
        if result.is_ok() {
            guard.armed = false;
        }
        result
    }
}

async fn run(
    input: MediaInput,
    identity: StreamIdentity,
    language: SourceLanguage,
    rate: PreparedSampleRate,
    cancel: &ResolverCancellation,
    provider: &dyn PreparedSpeechRecognizer,
) -> Result<RecognizedLocalMedia, PreparedAsrError> {
    check(cancel)?;
    let mut recognition = PreparedRecognitionSession::new(identity, language, rate, cancel)?;
    // Validate before constructing or pulling the local source. No input I/O
    // is needed to discover an unsupported or panicking provider capability.
    let capabilities = std::panic::catch_unwind(AssertUnwindSafe(|| provider.capabilities()))
        .map_err(|_| PreparedSpeechError::InvalidData)?;
    capabilities.validate(language, rate)?;
    check(cancel)?;
    let mut source = PreparedLocalSource::new(input, identity, rate, cancel.clone())?;
    let mut chunks = Vec::with_capacity(MAX_ASR_BATCH_CHUNKS);
    let media = loop {
        check(cancel)?;
        if chunks.len() > MAX_ASR_BATCH_CHUNKS - MAX_LOCAL_BATCH_CHUNKS {
            recognition
                .recognize_batch(std::mem::take(&mut chunks), provider)
                .await?;
            check(cancel)?;
        }
        let batch = source.pull().await?;
        check(cancel)?;
        if batch.chunks.len() > MAX_LOCAL_BATCH_CHUNKS
            || (batch.chunks.is_empty() && !batch.eof)
            || chunks.len() + batch.chunks.len() > MAX_ASR_BATCH_CHUNKS
            || (!batch.eof && batch.media.is_some())
        {
            return Err(PreparedSpeechError::InvalidData.into());
        }
        chunks.extend(batch.chunks.into_vec());
        if batch.eof {
            break batch.media.ok_or(MediaError::InvalidAudio)?;
        }
    };
    if !chunks.is_empty() {
        recognition.recognize_batch(chunks, provider).await?;
    }
    check(cancel)?;
    let transcript = recognition.finish()?;
    check(cancel)?;
    Ok(RecognizedLocalMedia { media, transcript })
}

fn check(cancel: &ResolverCancellation) -> Result<(), PreparedAsrError> {
    if cancel.is_cancelled() {
        Err(PreparedSpeechError::Cancelled.into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "prepared_asr_tests.rs"]
mod tests;
