//! Pull-based HTTP PCM16 WAVE decoding, sharing the original range owner.
//! No MF callback, capture, playback, ASR, Ready or disk buffer is involved.
use super::{
    MediaError,
    decode_worker::MAX_DECODED_MEDIA_DURATION_US,
    decoded::{DecodedAudioBlock, DecodedAudioFormat, DecodedAudioGate, MAX_DECODED_BLOCK_BYTES},
    remote_http::{
        MAX_REMOTE_RANGE_BYTES, MAX_REMOTE_RANGE_REQUESTS, RemoteHttpMedia, RemoteMediaError,
        RemoteRangeRequest,
    },
    remote_probe::{ProbedRemoteAudio, RemoteAudioFormat},
};
use crate::{ai::speech::StreamIdentity, resolver::contracts::ResolverCancellation};
use std::{
    future::Future,
    sync::{Arc, OnceLock},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Includes in-flight and downstream-retained PCM, across retired sources.
pub const MAX_REMOTE_PCM_BLOCKS: usize = 4;
const MAX_WAVE_CHUNKS: usize = 128;
const MAX_WAVE_JUNK_BYTES: u64 = 1024 * 1024;
/// At most 4096 LE samples are converted before cancellation/yield (8 slices
/// per 64 KiB block). This is bounded copying, not a CPU-heavy codec task.
const CONVERSION_SLICE_BYTES: usize = 8192;
const US_PER_SECOND: u64 = 1_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RemoteWaveError {
    #[error("{0}")]
    Remote(#[from] RemoteMediaError),
    #[error("{0}")]
    Media(#[from] MediaError),
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
/// PCM has no Debug/Clone/serialization or ownership escape hatch. Retained
/// blocks keep their global memory permit even after their decoder is retired.
pub struct RemoteWaveBlock {
    block: DecodedAudioBlock,
    cancel: ResolverCancellation,
    _permit: OwnedSemaphorePermit,
}
impl RemoteWaveBlock {
    pub fn block(&self) -> Result<&DecodedAudioBlock, RemoteWaveError> {
        check(&self.cancel)?;
        Ok(&self.block)
    }
}
struct Layout {
    format: DecodedAudioFormat,
    start: u64,
    bytes: u64,
    align: usize,
}
#[derive(PartialEq, Eq)]
enum State {
    Reading,
    Eof,
    Failed,
}
pub struct RemoteWaveDecoder {
    media: RemoteHttpMedia,
    layout: Layout,
    frames: u64,
    sequence: u64,
    state: State,
    gate: DecodedAudioGate,
    cancel: ResolverCancellation,
    cpu: Arc<Semaphore>,
    blocks: Arc<Semaphore>,
}
impl Drop for RemoteWaveDecoder {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
fn blocks() -> Arc<Semaphore> {
    static BLOCKS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    BLOCKS
        .get_or_init(|| Arc::new(Semaphore::new(MAX_REMOTE_PCM_BLOCKS)))
        .clone()
}
impl RemoteWaveDecoder {
    /// Passive construction; only the already selected Wave source is accepted.
    /// All RIFF chunk boundaries are checked before any PCM is returned.
    pub fn open(
        input: ProbedRemoteAudio,
    ) -> impl Future<Output = Result<Self, RemoteWaveError>> + Send {
        Self::open_with(input, super::work::workers(), blocks())
    }
    fn open_with(
        input: ProbedRemoteAudio,
        cpu: Arc<Semaphore>,
        blocks: Arc<Semaphore>,
    ) -> impl Future<Output = Result<Self, RemoteWaveError>> + Send {
        let format = input.format();
        let mut media = input.into_media();
        let cancel = media.cancellation();
        let guard = CancelOnDrop {
            cancel: cancel.clone(),
            armed: true,
        };
        async move {
            let mut guard = guard;
            check(&cancel)?;
            if format != RemoteAudioFormat::Wave {
                return Err(MediaError::UnsupportedMedia.into());
            }
            let layout = headers::parse(&mut media, &cancel).await?;
            let reads = layout.bytes.div_ceil(block_bytes(layout.align) as u64);
            if media
                .next_sequence()
                .checked_add(reads - 1)
                .is_none_or(|last| last > MAX_REMOTE_RANGE_REQUESTS)
            {
                return Err(MediaError::TooLarge.into());
            }
            let gate = DecodedAudioGate::new(media.identity(), layout.format)?;
            check(&cancel)?;
            guard.armed = false;
            Ok(Self {
                media,
                layout,
                frames: 0,
                sequence: 1,
                state: State::Reading,
                gate,
                cancel,
                cpu,
                blocks,
            })
        }
    }
    pub fn identity(&self) -> StreamIdentity {
        self.media.identity()
    }
    pub fn format(&self) -> DecodedAudioFormat {
        self.layout.format
    }
    pub fn total_frames(&self) -> u64 {
        self.layout.bytes / self.layout.align as u64
    }
    pub(crate) fn byte_len(&self) -> u64 {
        self.media.byte_len()
    }
    pub(crate) fn cancellation(&self) -> ResolverCancellation {
        self.cancel.clone()
    }
    /// One serialized pull. No scheduling/read-ahead; any failure or even an
    /// unpolled Future Drop retires this scope and revokes old block access.
    pub fn read_next(
        &mut self,
    ) -> impl Future<Output = Result<Option<RemoteWaveBlock>, RemoteWaveError>> + Send + '_ {
        let guard = CancelOnDrop {
            cancel: self.cancel.clone(),
            armed: true,
        };
        async move {
            let mut guard = guard;
            let result = self.pull().await;
            if result.is_ok() {
                guard.armed = false;
            } else {
                self.state = State::Failed;
            }
            result
        }
    }
    async fn pull(&mut self) -> Result<Option<RemoteWaveBlock>, RemoteWaveError> {
        check(&self.cancel)?;
        match self.state {
            State::Eof => return Ok(None),
            State::Failed => return Err(MediaError::InvalidAudio.into()),
            State::Reading => {}
        }
        let permit = self
            .blocks
            .clone()
            .try_acquire_owned()
            .map_err(|_| MediaError::Overloaded)?;
        let consumed = self
            .frames
            .checked_mul(self.layout.align as u64)
            .ok_or(MediaError::InvalidAudio)?;
        let length =
            (self.layout.bytes - consumed).min(block_bytes(self.layout.align) as u64) as usize;
        let range = read(&mut self.media, self.layout.start + consumed, length).await?;
        // Network never holds a CPU permit. Saturation is an explicit failure.
        let _cpu = self
            .cpu
            .clone()
            .try_acquire_owned()
            .map_err(|_| MediaError::Overloaded)?;
        let bytes = range.bytes()?;
        let mut samples = Vec::new();
        samples
            .try_reserve_exact(length / 2)
            .map_err(|_| MediaError::Overloaded)?;
        for slice in bytes.chunks(CONVERSION_SLICE_BYTES) {
            check(&self.cancel)?;
            let (pairs, remainder) = slice.as_chunks::<2>();
            if !remainder.is_empty() {
                return Err(MediaError::InvalidAudio.into());
            }
            samples.extend(pairs.iter().copied().map(i16::from_le_bytes));
            tokio::task::yield_now().await;
        }
        check(&self.cancel)?;
        let timestamp = self
            .frames
            .checked_mul(US_PER_SECOND)
            .ok_or(MediaError::InvalidAudio)?
            / u64::from(self.layout.format.sample_rate());
        let block = DecodedAudioBlock::new(
            self.identity(),
            self.sequence,
            timestamp,
            self.layout.format,
            samples,
        )?;
        if self.gate.validate(&block)? {
            return Err(MediaError::InvalidAudio.into());
        }
        self.frames += block.frames() as u64;
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or(MediaError::InvalidAudio)?;
        if self.frames == self.total_frames() {
            self.state = State::Eof;
        }
        Ok(Some(RemoteWaveBlock {
            block,
            cancel: self.cancel.clone(),
            _permit: permit,
        }))
    }
}
fn check(cancel: &ResolverCancellation) -> Result<(), RemoteWaveError> {
    if cancel.is_cancelled() {
        Err(RemoteMediaError::Cancelled.into())
    } else {
        Ok(())
    }
}
#[cfg(test)]
#[path = "remote_wave_test_source.rs"]
pub(crate) mod test_source;
fn block_bytes(align: usize) -> usize {
    let limit = MAX_DECODED_BLOCK_BYTES.min(MAX_REMOTE_RANGE_BYTES);
    limit - limit % align
}
async fn read(
    media: &mut RemoteHttpMedia,
    offset: u64,
    length: usize,
) -> Result<super::remote_http::RemoteMediaRange, RemoteWaveError> {
    let request = RemoteRangeRequest::new(media.identity(), media.next_sequence(), offset, length)?;
    Ok(media.read_range(request).await?)
}
#[path = "remote_wave_headers.rs"]
mod headers;
#[cfg(test)]
#[path = "remote_wave_tests.rs"]
mod tests;
