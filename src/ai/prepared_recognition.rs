//! Caller-owned offline ASR batches. No Live freshness clock, native worker,
//! credentials, default provider, subtitle provenance or renderer is invented.
use super::{
    prepared_speech::{
        MAX_ASR_BATCH_CHUNKS, PreparedSpeechChunk, PreparedSpeechError, PreparedSpeechRecognizer,
        PreparedSpeechRequest, PreparedSpeechResponse,
    },
    prepared_transcript::{PreparedTranscript, PreparedTranscriptMerger},
    speech::StreamIdentity,
};
use crate::{
    domain::SourceLanguage, media::prepared_audio::PreparedSampleRate,
    resolver::contracts::ResolverCancellation,
};
use futures_util::{FutureExt, StreamExt, stream::FuturesUnordered};
use std::{collections::VecDeque, future::Future, panic::AssertUnwindSafe, time::Duration};

/// A deliberate offline request has its own budget. This never changes Live
/// audio age, provider reconnect, or the document translation timeout.
pub const PREPARED_RECOGNITION_TIMEOUT: Duration = Duration::from_secs(60);

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

struct BatchOwner<'a> {
    session: &'a mut PreparedRecognitionSession,
    success: bool,
}
impl Drop for BatchOwner<'_> {
    fn drop(&mut self) {
        if !self.success {
            self.session.terminate();
        }
    }
}

/// Owns bounded, ordered source text until the caller declares complete EOF.
/// Audio arrives as at most four owned chunks per batch; the producer must pause
/// until this call completes rather than accumulating the whole file in RAM.
///
/// This core has no native-file/GUI adapter. Caller-supplied providers must honor
/// cancellation and own no detached requests; there is no mock/default fallback.
pub struct PreparedRecognitionSession {
    identity: StreamIdentity,
    language: SourceLanguage,
    rate: PreparedSampleRate,
    cancel: ResolverCancellation,
    merger: Option<PreparedTranscriptMerger>,
    terminal: bool,
    cancel_on_drop: bool,
}
impl PreparedRecognitionSession {
    pub fn new(
        identity: StreamIdentity,
        language: SourceLanguage,
        rate: PreparedSampleRate,
        cancel: &ResolverCancellation,
    ) -> Result<Self, PreparedSpeechError> {
        check(cancel)?;
        Ok(Self {
            identity,
            language,
            rate,
            cancel: cancel.clone(),
            merger: Some(PreparedTranscriptMerger::new(identity, language)?),
            terminal: false,
            cancel_on_drop: true,
        })
    }
    /// No provider work starts on construction. Dropping even an unpolled future
    /// cancels this scope. Only a complete successful batch retains source text;
    /// any error makes the session terminal and clears its private merger.
    pub fn recognize_batch<'a>(
        &'a mut self,
        chunks: Vec<PreparedSpeechChunk>,
        provider: &'a dyn PreparedSpeechRecognizer,
    ) -> impl Future<Output = Result<(), PreparedSpeechError>> + Send + 'a {
        self.batch_with_timeout(chunks, provider, PREPARED_RECOGNITION_TIMEOUT)
    }
    fn batch_with_timeout<'a>(
        &'a mut self,
        chunks: Vec<PreparedSpeechChunk>,
        provider: &'a dyn PreparedSpeechRecognizer,
        timeout: Duration,
    ) -> impl Future<Output = Result<(), PreparedSpeechError>> + Send + 'a {
        // Discard oversized PCM and excess Vec capacity before constructing an
        // unpolled future. The future may own only this bounded batch.
        let chunks = bounded_batch(chunks);
        let owner = BatchOwner {
            session: self,
            success: false,
        };
        async move {
            let mut owner = owner;
            let result = match (check(&owner.session.cancel), chunks) {
                (Err(error), _) => Err(error),
                (Ok(()), Ok(chunks)) => {
                    owner
                        .session
                        .execute(chunks.into_vec(), provider, timeout)
                        .await
                }
                (Ok(()), Err(error)) => Err(error),
            };
            owner.success = result.is_ok();
            result
        }
    }
    async fn execute(
        &mut self,
        chunks: Vec<PreparedSpeechChunk>,
        provider: &dyn PreparedSpeechRecognizer,
        timeout: Duration,
    ) -> Result<(), PreparedSpeechError> {
        check(&self.cancel)?;
        if self.terminal {
            return Err(PreparedSpeechError::InvalidData);
        }
        if chunks.is_empty() || chunks.len() > MAX_ASR_BATCH_CHUNKS || timeout.is_zero() {
            return Err(PreparedSpeechError::InvalidData);
        }
        // Validate every input before asking the selected provider to do work.
        for chunk in &chunks {
            if chunk.stamp().identity != self.identity {
                return Err(PreparedSpeechError::Stale);
            }
            if chunk.stamp().rate != self.rate {
                return Err(PreparedSpeechError::InvalidData);
            }
        }
        let capabilities = std::panic::catch_unwind(AssertUnwindSafe(|| provider.capabilities()))
            .map_err(|_| PreparedSpeechError::InvalidData)?;
        capabilities.validate(self.language, self.rate)?;
        check(&self.cancel)?;
        let merger = self
            .merger
            .as_mut()
            .ok_or(PreparedSpeechError::InvalidData)?;
        let mut waiting = VecDeque::with_capacity(chunks.len());
        for chunk in chunks {
            let context = merger.context(chunk.stamp().run_id);
            let request = PreparedSpeechRequest::new(chunk, self.language, context)?;
            merger.register(&request)?;
            waiting.push_back(request);
        }
        let mut active = FuturesUnordered::new();
        while !waiting.is_empty() || !active.is_empty() {
            while active.len() < capabilities.max_concurrent {
                let Some(request) = waiting.pop_front() else {
                    break;
                };
                active.push(run_request(request, provider, &self.cancel, timeout));
            }
            let (request, result) = tokio::select! { biased;
                _=self.cancel.cancelled()=>return Err(PreparedSpeechError::Cancelled),
                result=active.next()=>result.ok_or(PreparedSpeechError::InvalidData)?,
            };
            check(&self.cancel)?;
            merger.complete(&request, result?)?;
        }
        check(&self.cancel)
    }
    /// The producer must have delivered all input and its EOF tail. This is a
    /// source transcript, never an ExistingSubtitle or a Prepared Ready document.
    pub fn finish(mut self) -> Result<PreparedTranscript, PreparedSpeechError> {
        check(&self.cancel)?;
        if self.terminal {
            return Err(PreparedSpeechError::InvalidData);
        }
        let result = self
            .merger
            .take()
            .ok_or(PreparedSpeechError::InvalidData)?
            .finish();
        check(&self.cancel)?;
        if result.is_ok() {
            self.cancel_on_drop = false;
        }
        result
    }
    pub fn cancel(&mut self) {
        self.terminate();
    }
    fn terminate(&mut self) {
        self.terminal = true;
        self.cancel.cancel();
        self.merger = None;
    }
}
fn bounded_batch(
    chunks: Vec<PreparedSpeechChunk>,
) -> Result<Box<[PreparedSpeechChunk]>, PreparedSpeechError> {
    if chunks.is_empty() || chunks.len() > MAX_ASR_BATCH_CHUNKS {
        return Err(PreparedSpeechError::InvalidData);
    }
    Ok(chunks.into_boxed_slice())
}
impl Drop for PreparedRecognitionSession {
    fn drop(&mut self) {
        if self.cancel_on_drop {
            self.cancel.cancel();
        }
    }
}

async fn run_request(
    request: PreparedSpeechRequest,
    provider: &dyn PreparedSpeechRecognizer,
    parent: &ResolverCancellation,
    timeout: Duration,
) -> (
    PreparedSpeechRequest,
    Result<PreparedSpeechResponse, PreparedSpeechError>,
) {
    let result = run_one(&request, provider, parent, timeout).await;
    (request, result)
}
async fn run_one(
    request: &PreparedSpeechRequest,
    provider: &dyn PreparedSpeechRecognizer,
    parent: &ResolverCancellation,
    timeout: Duration,
) -> Result<PreparedSpeechResponse, PreparedSpeechError> {
    check(parent)?;
    let cancel = ResolverCancellation::linked(parent.clone())
        .map_err(|_| PreparedSpeechError::InvalidData)?;
    let _guard = CancelOnDrop {
        cancel: cancel.clone(),
        armed: true,
    };
    let result = tokio::select! { biased;
        _=cancel.cancelled()=>return Err(PreparedSpeechError::Cancelled),
        result=tokio::time::timeout(timeout,AssertUnwindSafe(async { provider.recognize(request,cancel.clone()).await }).catch_unwind())=>{
            result.map_err(|_|PreparedSpeechError::Timeout)?.map_err(|_|PreparedSpeechError::InvalidData)?
        }
    };
    check(&cancel)?;
    check(parent)?;
    result
}
fn check(cancel: &ResolverCancellation) -> Result<(), PreparedSpeechError> {
    if cancel.is_cancelled() {
        Err(PreparedSpeechError::Cancelled)
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "prepared_recognition_tests.rs"]
mod tests;
