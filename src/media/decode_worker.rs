//! Caller-owned local decoding on the shared bounded Prepared CPU lane.
//! Only one borrowed PCM block reaches the synchronous consumer at a time.
use super::{
    MediaError,
    decoded::{DecodedAudioBlock, DecodedAudioFormat, DecodedAudioGate, MediaDecoder},
    input::MediaInput,
};
use crate::{ai::speech::StreamIdentity, resolver::contracts::ResolverCancellation};
use std::{future::Future, sync::Arc};
use tokio::sync::Semaphore;

/// Engineering limit for one Prepared media timeline, not a decoder capability
/// claim. Refuse longer media before handing out a block past the limit.
pub const MAX_DECODED_MEDIA_DURATION_US: u64 = 24 * 60 * 60 * 1_000_000;

/// Complete validated decode metadata. No audio, text, path, serialization,
/// Debug, or playback state is retained in this result.
pub struct DecodedAudioSummary {
    identity: StreamIdentity,
    format: DecodedAudioFormat,
    blocks: u64,
    frames: u64,
    first_timestamp_us: u64,
    last_end_us: u64,
    gaps: u64,
}
impl DecodedAudioSummary {
    pub const fn identity(&self) -> StreamIdentity {
        self.identity
    }
    pub const fn format(&self) -> DecodedAudioFormat {
        self.format
    }
    pub const fn blocks(&self) -> u64 {
        self.blocks
    }
    pub const fn frames(&self) -> u64 {
        self.frames
    }
    pub const fn first_timestamp_us(&self) -> u64 {
        self.first_timestamp_us
    }
    pub const fn last_end_us(&self) -> u64 {
        self.last_end_us
    }
    pub const fn gaps(&self) -> u64 {
        self.gaps
    }
}

/// The returned future must be polled by its owner. Dropping even an unpolled
/// future cancels this scope. Native COM creation, decode and destruction all
/// happen on one blocking worker; no PCM queue or detached async task exists.
///
/// `consumer` must do short, bounded CPU work only. It must not perform network,
/// disk, UI, or blocking queue operations, and must not retain copies of PCM.
/// A callback error stops pulling immediately and never produces a summary.
pub fn decode_local_audio<C>(
    input: MediaInput,
    identity: StreamIdentity,
    cancel: ResolverCancellation,
    consumer: C,
) -> impl Future<Output = Result<DecodedAudioSummary, MediaError>> + Send
where
    C: FnMut(&DecodedAudioBlock, &ResolverCancellation) -> Result<(), MediaError> + Send + 'static,
{
    decode_with(
        identity,
        cancel,
        consumer,
        move |cancel| open_local(input, identity, cancel),
        super::work::workers(),
    )
}

/// Private EOF-aware processing on the same native worker. Implementations may
/// retain only their explicitly bounded FIR/output state, never a media archive
/// or an async/network consumer. Finish runs only after validated, nonempty EOF.
pub(super) trait DecodeConsumer: Send + 'static {
    type Output: Send + 'static;
    fn push(
        &mut self,
        block: &DecodedAudioBlock,
        cancel: &ResolverCancellation,
    ) -> Result<(), MediaError>;
    fn finish(
        self,
        summary: DecodedAudioSummary,
        cancel: &ResolverCancellation,
    ) -> Result<Self::Output, MediaError>;
}

struct BorrowedConsumer<C>(C);
impl<C> DecodeConsumer for BorrowedConsumer<C>
where
    C: FnMut(&DecodedAudioBlock, &ResolverCancellation) -> Result<(), MediaError> + Send + 'static,
{
    type Output = DecodedAudioSummary;
    fn push(
        &mut self,
        block: &DecodedAudioBlock,
        cancel: &ResolverCancellation,
    ) -> Result<(), MediaError> {
        self.0(block, cancel)
    }
    fn finish(
        self,
        summary: DecodedAudioSummary,
        _: &ResolverCancellation,
    ) -> Result<Self::Output, MediaError> {
        Ok(summary)
    }
}

pub(super) fn decode_consumer<C: DecodeConsumer>(
    input: MediaInput,
    identity: StreamIdentity,
    cancel: ResolverCancellation,
    consumer: C,
) -> impl Future<Output = Result<C::Output, MediaError>> + Send {
    decode_with_consumer(
        identity,
        cancel,
        consumer,
        move |cancel| open_sample_clock(input, identity, cancel),
        super::work::workers(),
    )
}

fn open_local(
    input: MediaInput,
    identity: StreamIdentity,
    cancel: &ResolverCancellation,
) -> Result<Box<dyn MediaDecoder>, MediaError> {
    open_with_clock(input, identity, cancel, DecoderClock::Native)
}
fn open_sample_clock(
    input: MediaInput,
    identity: StreamIdentity,
    cancel: &ResolverCancellation,
) -> Result<Box<dyn MediaDecoder>, MediaError> {
    open_with_clock(input, identity, cancel, DecoderClock::SampleAccurate)
}
enum DecoderClock {
    Native,
    SampleAccurate,
}
fn open_with_clock(
    input: MediaInput,
    identity: StreamIdentity,
    cancel: &ResolverCancellation,
    clock: DecoderClock,
) -> Result<Box<dyn MediaDecoder>, MediaError> {
    #[cfg(windows)]
    {
        // The legacy whole-decode entry point shares the native-owner bound
        // with demand-driven actors. This permit belongs to the decoder, not
        // its async caller, and survives cancellation until actual destruction.
        let owner = super::work::native_owners()
            .try_acquire_owned()
            .map_err(|_| MediaError::Overloaded)?;
        check(cancel)?;
        let decoder = match clock {
            DecoderClock::Native => {
                crate::win32::media_decode::open(input, identity, cancel.clone())?
            }
            DecoderClock::SampleAccurate => {
                crate::win32::media_decode::open_prepared_audio(input, identity, cancel.clone())?
            }
        };
        Ok(Box::new(OwnedDecoder {
            inner: Box::new(decoder),
            _owner: owner,
        }) as Box<dyn MediaDecoder>)
    }
    #[cfg(not(windows))]
    {
        let _ = (input, identity, cancel, clock);
        Err(MediaError::DecoderUnavailable)
    }
}

/// Field order destroys the thread-affine decoder (including MF/COM and the
/// input pin) before releasing its native-owner capacity. MediaDecoder is not
/// Send, so this wrapper cannot move the native owner to a different thread.
#[cfg(any(windows, test))]
struct OwnedDecoder {
    inner: Box<dyn MediaDecoder>,
    _owner: tokio::sync::OwnedSemaphorePermit,
}
#[cfg(any(windows, test))]
impl MediaDecoder for OwnedDecoder {
    fn format(&self) -> DecodedAudioFormat {
        self.inner.format()
    }
    fn next(
        &mut self,
        cancel: &ResolverCancellation,
    ) -> Result<Option<DecodedAudioBlock>, MediaError> {
        self.inner.next(cancel)
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
struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);
impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

// Only tests supply an alternate factory or semaphore. The production factory
// opens the explicitly supplied local input using Windows Media Foundation.
fn decode_with<C, F>(
    identity: StreamIdentity,
    cancel: ResolverCancellation,
    consumer: C,
    factory: F,
    workers: Arc<Semaphore>,
) -> impl Future<Output = Result<DecodedAudioSummary, MediaError>> + Send
where
    C: FnMut(&DecodedAudioBlock, &ResolverCancellation) -> Result<(), MediaError> + Send + 'static,
    F: FnOnce(&ResolverCancellation) -> Result<Box<dyn MediaDecoder>, MediaError> + Send + 'static,
{
    decode_with_consumer(
        identity,
        cancel,
        BorrowedConsumer(consumer),
        factory,
        workers,
    )
}

pub(super) fn decode_with_consumer<C, F>(
    identity: StreamIdentity,
    cancel: ResolverCancellation,
    consumer: C,
    factory: F,
    workers: Arc<Semaphore>,
) -> impl Future<Output = Result<C::Output, MediaError>> + Send
where
    C: DecodeConsumer,
    F: FnOnce(&ResolverCancellation) -> Result<Box<dyn MediaDecoder>, MediaError> + Send + 'static,
{
    // Construct before returning the future so cancellation also covers Drop
    // before its first poll. The native decoder is not created here.
    let guard = CancelOnDrop {
        cancel: cancel.clone(),
        armed: true,
    };
    async move {
        let mut guard = guard;
        let result = dispatch(identity, cancel, consumer, factory, workers).await;
        if result.is_ok() {
            guard.armed = false;
        }
        result
    }
}

async fn dispatch<C, F>(
    identity: StreamIdentity,
    cancel: ResolverCancellation,
    consumer: C,
    factory: F,
    workers: Arc<Semaphore>,
) -> Result<C::Output, MediaError>
where
    C: DecodeConsumer,
    F: FnOnce(&ResolverCancellation) -> Result<Box<dyn MediaDecoder>, MediaError> + Send + 'static,
{
    check(&cancel)?;
    if identity.session.0 == 0 || identity.generation == 0 {
        return Err(MediaError::InvalidAudio);
    }
    let permit = workers
        .try_acquire_owned()
        .map_err(|_| MediaError::Overloaded)?;
    let worker_cancel = cancel.clone();
    let mut worker = AbortOnDrop(tokio::task::spawn_blocking(move || {
        // A started worker retains capacity through native destruction even if
        // its owner stops polling. Aborting never implies this thread exited.
        let _permit = permit;
        check(&worker_cancel)?;
        let mut decoder = factory(&worker_cancel)?;
        check(&worker_cancel)?;
        consume(identity, &worker_cancel, decoder.as_mut(), consumer)
    }));
    let result = tokio::select! { biased;
        _ = cancel.cancelled() => return Err(MediaError::Cancelled),
        result = &mut worker.0 => result.map_err(|_| MediaError::WorkerFailed)?,
    };
    check(&cancel)?;
    result
}

fn consume<C>(
    identity: StreamIdentity,
    cancel: &ResolverCancellation,
    decoder: &mut dyn MediaDecoder,
    mut consumer: C,
) -> Result<C::Output, MediaError>
where
    C: DecodeConsumer,
{
    let format = decoder.format();
    let mut gate = DecodedAudioGate::new(identity, format)?;
    let mut blocks = 0_u64;
    let mut frames = 0_u64;
    let mut gaps = 0_u64;
    let mut first_timestamp_us = None;
    let mut last_end_us = 0;
    loop {
        check(cancel)?;
        let next = decoder.next(cancel)?;
        check(cancel)?;
        let Some(block) = next else {
            break;
        };
        let discontinuity = gate.validate(&block)?;
        if block.end_us() > MAX_DECODED_MEDIA_DURATION_US {
            return Err(MediaError::TooLarge);
        }
        let next_blocks = blocks.checked_add(1).ok_or(MediaError::InvalidAudio)?;
        let next_frames = frames
            .checked_add(u64::try_from(block.frames()).map_err(|_| MediaError::InvalidAudio)?)
            .ok_or(MediaError::InvalidAudio)?;
        let next_gaps = gaps
            .checked_add(u64::from(discontinuity))
            .ok_or(MediaError::InvalidAudio)?;
        check(cancel)?;
        consumer.push(&block, cancel)?;
        check(cancel)?;
        blocks = next_blocks;
        frames = next_frames;
        gaps = next_gaps;
        first_timestamp_us.get_or_insert(block.timestamp_us());
        last_end_us = block.end_us();
        // Release this input before the next pull. Dedicated processors retain
        // only their documented bounded filter state, never input block copies.
    }
    check(cancel)?;
    let summary = DecodedAudioSummary {
        identity,
        format,
        blocks,
        frames,
        first_timestamp_us: first_timestamp_us.ok_or(MediaError::InvalidAudio)?,
        last_end_us,
        gaps,
    };
    let result = consumer.finish(summary, cancel);
    check(cancel)?;
    result
}

fn check(cancel: &ResolverCancellation) -> Result<(), MediaError> {
    if cancel.is_cancelled() {
        Err(MediaError::Cancelled)
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "decode_worker_tests.rs"]
mod tests;
