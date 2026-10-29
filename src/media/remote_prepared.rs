//! Explicit remote PCM16 WAVE -> existing bounded Prepared recognition.
//! No default provider, credentials, Ready, playback, GUI or persistent content.
use super::{
    MediaError,
    prepared_audio::PreparedSampleRate,
    remote_prepared_source::{MAX_REMOTE_BATCH_CHUNKS, RemotePreparedSource},
    remote_wave::{RemoteWaveDecoder, RemoteWaveError},
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
use std::{future::Future, panic::AssertUnwindSafe, sync::Arc};
use tokio::sync::Semaphore;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RemotePreparedError {
    #[error("{0}")]
    Media(#[from] RemoteWaveError),
    #[error("{0}")]
    Speech(#[from] PreparedSpeechError),
}
impl From<MediaError> for RemotePreparedError {
    fn from(error: MediaError) -> Self {
        Self::Media(error.into())
    }
}

/// EOF source and recognized source text stay together. The original HTTP
/// owner/ETag/sequence survives the final provider wait; this is not a Ready
/// document or an alignment proof. Drop retires the source and its scope.
pub struct RecognizedRemoteWave {
    source: RemoteWaveDecoder,
    transcript: PreparedTranscript,
}
impl RecognizedRemoteWave {
    /// Internal Session handoff only. The same EOF source must be retained by
    /// the consumer through translation and Ready; no public text-only escape.
    pub(crate) fn into_parts(
        self,
    ) -> Result<(RemoteWaveDecoder, PreparedTranscript), RemotePreparedError> {
        check(&self.source.cancellation())?;
        Ok((self.source, self.transcript))
    }
    pub fn identity(&self) -> StreamIdentity {
        self.source.identity()
    }
    /// Session-only borrow. Consumers must also match their active identity;
    /// there is no cloning, serialization or detached transcript escape hatch.
    pub fn transcript(&self) -> Result<&PreparedTranscript, RemotePreparedError> {
        check(&self.source.cancellation())?;
        Ok(&self.transcript)
    }
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

/// Caller supplies an already validated source and explicitly selected provider.
/// Construction performs no further media/provider I/O; even unpolled Drop
/// retires the source. DSP runs on bounded blocking jobs using the existing CPU
/// lane. Provider waits hold no CPU permit and schedule no media read-ahead.
pub fn recognize_remote_wave<'a>(
    source: RemoteWaveDecoder,
    language: SourceLanguage,
    rate: PreparedSampleRate,
    provider: &'a dyn PreparedSpeechRecognizer,
) -> impl Future<Output = Result<RecognizedRemoteWave, RemotePreparedError>> + Send + 'a {
    recognize_with(source, language, rate, provider, super::work::workers())
}
fn recognize_with<'a>(
    mut source: RemoteWaveDecoder,
    language: SourceLanguage,
    rate: PreparedSampleRate,
    provider: &'a dyn PreparedSpeechRecognizer,
    cpu: Arc<Semaphore>,
) -> impl Future<Output = Result<RecognizedRemoteWave, RemotePreparedError>> + Send + 'a {
    let cancel = source.cancellation();
    let guard = CancelOnDrop {
        cancel: cancel.clone(),
        armed: true,
    };
    async move {
        let mut guard = guard;
        check(&cancel)?;
        let mut recognition =
            PreparedRecognitionSession::new(source.identity(), language, rate, &cancel)?;
        let capabilities = std::panic::catch_unwind(AssertUnwindSafe(|| provider.capabilities()))
            .map_err(|_| PreparedSpeechError::InvalidData)?;
        capabilities.validate(language, rate)?;
        check(&cancel)?;
        let mut producer = RemotePreparedSource::new(rate, cpu);
        let mut chunks = Vec::with_capacity(MAX_ASR_BATCH_CHUNKS);
        loop {
            check(&cancel)?;
            if chunks.len() > MAX_ASR_BATCH_CHUNKS - MAX_REMOTE_BATCH_CHUNKS {
                recognition
                    .recognize_batch(std::mem::take(&mut chunks), provider)
                    .await?;
                check(&cancel)?;
            }
            let batch = producer.pull(&mut source).await?;
            if batch.chunks.len() > MAX_REMOTE_BATCH_CHUNKS
                || (batch.chunks.is_empty() && !batch.eof)
                || chunks.len() + batch.chunks.len() > MAX_ASR_BATCH_CHUNKS
            {
                return Err(PreparedSpeechError::InvalidData.into());
            }
            chunks.extend(batch.chunks.into_vec());
            if batch.eof {
                break;
            }
        }
        if !chunks.is_empty() {
            recognition.recognize_batch(chunks, provider).await?;
        }
        check(&cancel)?;
        let transcript = recognition.finish()?;
        check(&cancel)?;
        guard.armed = false;
        Ok(RecognizedRemoteWave { source, transcript })
    }
}
pub(super) fn check(cancel: &ResolverCancellation) -> Result<(), RemotePreparedError> {
    if cancel.is_cancelled() {
        Err(PreparedSpeechError::Cancelled.into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "remote_prepared_tests.rs"]
mod tests;
