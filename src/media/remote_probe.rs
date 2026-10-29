//! One bounded worker bridges HTTP ranges to the existing content format gate.
//! No decoder, PCM, Ready, playback or persistent byte cache is created here.
use super::{
    MediaError,
    local_audio_format::{self, LocalAudioFormat},
    remote_http::{MAX_REMOTE_RANGE_BYTES, RemoteHttpMedia, RemoteMediaError, RemoteRangeRequest},
};
use crate::{ai::speech::StreamIdentity, resolver::contracts::ResolverCancellation};
use std::{
    future::Future,
    io::{self, Read, Seek, SeekFrom},
    sync::Arc,
};
use tokio::{
    runtime::Handle,
    sync::{OwnedSemaphorePermit, Semaphore},
    task::JoinHandle,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RemoteProbeError {
    #[error("{0}")]
    Remote(#[from] RemoteMediaError),
    #[error("{0}")]
    Media(#[from] MediaError),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteAudioFormat {
    Wave,
    Mp3,
    Adts,
    Mp4,
}
impl From<LocalAudioFormat> for RemoteAudioFormat {
    fn from(value: LocalAudioFormat) -> Self {
        match value {
            LocalAudioFormat::Wave => Self::Wave,
            LocalAudioFormat::Mp3 => Self::Mp3,
            LocalAudioFormat::Adts => Self::Adts,
            LocalAudioFormat::Mp4 => Self::Mp4,
        }
    }
}
/// Content preflight only. Owns the original URL/client/validator and request
/// sequence; no independent identity, local file pin or codec proof is invented.
pub struct ProbedRemoteAudio {
    media: RemoteHttpMedia,
    format: RemoteAudioFormat,
}
impl ProbedRemoteAudio {
    pub fn identity(&self) -> StreamIdentity {
        self.media.identity()
    }
    pub fn byte_len(&self) -> u64 {
        self.media.byte_len()
    }
    pub fn format(&self) -> RemoteAudioFormat {
        self.format
    }
    pub fn into_media(self) -> RemoteHttpMedia {
        self.media
    }
}
struct CancelOnDrop(ResolverCancellation, bool);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if self.1 {
            self.0.cancel();
        }
    }
}
struct Worker<T>(JoinHandle<T>);
impl<T> Drop for Worker<T> {
    fn drop(&mut self) {
        // Started blocking jobs stop cooperatively. Their source continues to
        // hold its real owner quota until exit, even if this handle is dropped.
        self.0.abort();
    }
}

/// Passive construction. Poll on an I/O/timer-enabled Tokio runtime, which must
/// stay alive while the blocking job exits. The awaiting runtime drives I/O even
/// with a current-thread scheduler; Handle::block_on is used only on the worker.
/// Drop or failure cancels the source. Success returns the same owned source.
pub fn probe_remote_audio(
    media: RemoteHttpMedia,
) -> impl Future<Output = Result<ProbedRemoteAudio, RemoteProbeError>> + Send {
    run(media, super::work::workers(), |reader, cancel| {
        local_audio_format::probe(reader, reader.media.byte_len(), cancel)
    })
}
#[cfg(test)]
pub(super) fn probe_for_test(
    media: RemoteHttpMedia,
    cpu: Arc<Semaphore>,
) -> impl Future<Output = Result<ProbedRemoteAudio, RemoteProbeError>> + Send {
    run(media, cpu, |reader, cancel| {
        local_audio_format::probe(reader, reader.media.byte_len(), cancel)
    })
}
fn run<F>(
    media: RemoteHttpMedia,
    cpu: Arc<Semaphore>,
    probe: F,
) -> impl Future<Output = Result<ProbedRemoteAudio, RemoteProbeError>> + Send
where
    F: FnOnce(&mut BlockingReader, &ResolverCancellation) -> Result<LocalAudioFormat, MediaError>
        + Send
        + 'static,
{
    let guard = CancelOnDrop(media.cancellation(), true);
    async move {
        let mut guard = guard;
        check(&guard.0)?;
        let runtime = Handle::try_current().map_err(|_| RemoteMediaError::RuntimeUnavailable)?;
        let permit = cpu
            .clone()
            .try_acquire_owned()
            .map_err(|_| MediaError::Overloaded)?;
        let cancel = guard.0.clone();
        let mut worker = Worker(tokio::task::spawn_blocking(move || {
            let mut reader = BlockingReader {
                media,
                runtime,
                cancel,
                cpu,
                permit: Some(permit),
                position: 0,
                failure: None,
            };
            let cancel = reader.cancel.clone();
            check(&cancel)?;
            let result = probe(&mut reader, &cancel);
            if let Some(error) = reader.failure {
                return Err(error);
            }
            check(&cancel)?;
            let format = result?.into();
            Ok(ProbedRemoteAudio {
                media: reader.media,
                format,
            })
        }));
        // Cancellation wakes the range future; join actual worker exit before
        // returning. A range failure also cancels its scope, so selecting that
        // token here would hide SourceChanged/Timeout behind Cancelled.
        let output = (&mut worker.0)
            .await
            .map_err(|_| MediaError::WorkerFailed)??;
        check(&guard.0)?;
        guard.1 = false;
        Ok(output)
    }
}
fn check(cancel: &ResolverCancellation) -> Result<(), RemoteProbeError> {
    if cancel.is_cancelled() {
        Err(RemoteMediaError::Cancelled.into())
    } else {
        Ok(())
    }
}

/// Private synchronous adapter: never exposed to UI/capture or used on an
/// executor thread. One range at a time, copied into the parser's own bounded
/// buffer and immediately dropped. It retains no read-ahead/media cache.
struct BlockingReader {
    media: RemoteHttpMedia,
    runtime: Handle,
    cancel: ResolverCancellation,
    cpu: Arc<Semaphore>,
    permit: Option<OwnedSemaphorePermit>,
    position: u64,
    failure: Option<RemoteProbeError>,
}
impl BlockingReader {
    fn fail(&mut self, error: RemoteProbeError) -> io::Error {
        self.failure.get_or_insert(error);
        io::Error::other(error)
    }
    fn ready(&mut self) -> io::Result<()> {
        if let Some(error) = self.failure {
            return Err(io::Error::other(error));
        }
        check(&self.cancel).map_err(|error| self.fail(error))
    }
}
impl Read for BlockingReader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        self.ready()?;
        let length = (output.len().min(MAX_REMOTE_RANGE_BYTES) as u64)
            .min(self.media.byte_len() - self.position) as usize;
        if length == 0 {
            return Ok(0);
        }
        let request = RemoteRangeRequest::new(
            self.media.identity(),
            self.media.next_sequence(),
            self.position,
            length,
        )
        .map_err(|error| self.fail(error.into()))?;
        // The same process-wide CPU permit covers parser/copy work, but must
        // never be held while the worker waits for asynchronous network I/O.
        self.permit = None;
        let range = self
            .runtime
            .block_on(self.media.read_range(request))
            .map_err(|error| self.fail(error.into()))?;
        self.ready()?;
        let permit = self
            .cpu
            .clone()
            .try_acquire_owned()
            .map_err(|_| self.fail(MediaError::Overloaded.into()))?;
        self.permit = Some(permit);
        let bytes = range.bytes().map_err(|error| self.fail(error.into()))?;
        output[..length].copy_from_slice(bytes);
        self.position += length as u64; // Bounded by validated representation length.
        self.ready()?;
        Ok(length)
    }
}
impl Seek for BlockingReader {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.ready()?;
        let position = match from {
            SeekFrom::Start(offset) => Some(offset),
            SeekFrom::Current(delta) => self.position.checked_add_signed(delta),
            SeekFrom::End(delta) => self.media.byte_len().checked_add_signed(delta),
        }
        .filter(|position| *position <= self.media.byte_len())
        .ok_or_else(|| self.fail(RemoteMediaError::InvalidRequest.into()))?;
        self.position = position;
        Ok(position)
    }
}

#[cfg(test)]
#[path = "remote_probe_tests.rs"]
mod tests;
