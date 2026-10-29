//! Private demand-driven native ownership. No borrowed public PCM callback is
//! retained and no decoder can cross the worker's COM thread boundary.
use super::{
    MediaError,
    decoded::MediaDecoder,
    decoded_clock::{DecodedClockNormalizer, DecodedClockSpan, DecodedClockWindow},
    input::{MediaInput, OpenedMedia},
    prepared_asr::PreparedAsrError,
    prepared_audio::{PreparedAudioProcessor, PreparedSampleRate},
    prepared_segments::PreparedSegmentBuilder,
};
use crate::{
    ai::{
        prepared_speech::{PreparedSpeechChunk, PreparedSpeechError},
        speech::StreamIdentity,
    },
    resolver::contracts::ResolverCancellation,
};
use std::{future::Future, panic::AssertUnwindSafe, sync::Arc};
use tokio::{
    runtime::Handle,
    sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot},
    task::JoinHandle,
};

/// A native block, including gap/EOF flushing, can yield at most two bodies.
/// The minimum native rate and 64 KiB input limit bound each block to 4.096 s.
pub(crate) const MAX_LOCAL_BATCH_CHUNKS: usize = 2;
const COMMAND_CAPACITY: usize = 1;
#[cfg(test)]
type Factory =
    Box<dyn FnOnce(&ResolverCancellation) -> Result<Box<dyn MediaDecoder>, MediaError> + Send>;
struct OpenedDecoder {
    decoder: Box<dyn MediaDecoder>,
    media: Option<OpenedMedia>,
    clock: bool,
}
type PinnedFactory =
    Box<dyn FnOnce(&ResolverCancellation) -> Result<OpenedDecoder, MediaError> + Send>;

pub(crate) struct PreparedLocalBatch {
    pub chunks: Box<[PreparedSpeechChunk]>,
    pub eof: bool,
    /// Only true EOF transfers the original pin; native resources are joined
    /// before this batch reaches its caller. Explicit decoder mocks have None.
    pub(crate) media: Option<OpenedMedia>,
}
struct PullCommand {
    cpu: OwnedSemaphorePermit,
    reply: oneshot::Sender<Result<PreparedLocalBatch, PreparedAsrError>>,
}
pub(crate) struct PreparedLocalSource {
    identity: StreamIdentity,
    rate: PreparedSampleRate,
    cancel: ResolverCancellation,
    factory: Option<PinnedFactory>,
    cpu: Arc<Semaphore>,
    owners: Arc<Semaphore>,
    commands: Option<mpsc::Sender<PullCommand>>,
    worker: Option<JoinHandle<Result<(), PreparedAsrError>>>,
    terminal: bool,
    cancel_on_drop: bool,
}
struct PullOwner<'a> {
    source: &'a mut PreparedLocalSource,
    success: bool,
}
impl Drop for PullOwner<'_> {
    fn drop(&mut self) {
        if !self.success {
            self.source.terminate();
        }
    }
}
impl PreparedLocalSource {
    /// Passive construction; no file, native object, worker or PCM exists yet.
    pub(crate) fn new(
        input: MediaInput,
        identity: StreamIdentity,
        rate: PreparedSampleRate,
        cancel: ResolverCancellation,
    ) -> Result<Self, PreparedAsrError> {
        Self::with_pinned_factory(
            identity,
            rate,
            cancel,
            Box::new(move |cancel| {
                #[cfg(windows)]
                {
                    crate::win32::media_decode::open_native_pinned(input, identity, cancel.clone())
                        .map(|(decoder, media)| {
                            let clock = decoder.uses_packet_clock();
                            OpenedDecoder {
                                decoder: Box::new(decoder),
                                media: Some(media),
                                clock,
                            }
                        })
                }
                #[cfg(not(windows))]
                {
                    let _ = (input, identity, cancel);
                    Err(MediaError::DecoderUnavailable)
                }
            }),
            super::work::workers(),
            super::work::native_owners(),
        )
    }
    #[cfg(test)]
    fn with_factory(
        identity: StreamIdentity,
        rate: PreparedSampleRate,
        cancel: ResolverCancellation,
        factory: Factory,
        cpu: Arc<Semaphore>,
        owners: Arc<Semaphore>,
    ) -> Result<Self, PreparedAsrError> {
        Self::with_pinned_factory(
            identity,
            rate,
            cancel,
            Box::new(move |cancel| {
                factory(cancel).map(|decoder| OpenedDecoder {
                    decoder,
                    media: None,
                    clock: false,
                })
            }),
            cpu,
            owners,
        )
    }
    fn with_pinned_factory(
        identity: StreamIdentity,
        rate: PreparedSampleRate,
        cancel: ResolverCancellation,
        factory: PinnedFactory,
        cpu: Arc<Semaphore>,
        owners: Arc<Semaphore>,
    ) -> Result<Self, PreparedAsrError> {
        check(&cancel)?;
        if identity.session.0 == 0 || identity.generation == 0 {
            return Err(MediaError::InvalidAudio.into());
        }
        Ok(Self {
            identity,
            rate,
            cancel,
            factory: Some(factory),
            cpu,
            owners,
            commands: None,
            worker: None,
            terminal: false,
            cancel_on_drop: true,
        })
    }
    /// Exactly one caller-owned pull. Drop before the first poll also cancels.
    pub(crate) fn pull(
        &mut self,
    ) -> impl Future<Output = Result<PreparedLocalBatch, PreparedAsrError>> + Send + '_ {
        let owner = PullOwner {
            source: self,
            success: false,
        };
        async move {
            let mut owner = owner;
            let result = owner.source.dispatch().await;
            owner.success = result.is_ok();
            result
        }
    }
    async fn dispatch(&mut self) -> Result<PreparedLocalBatch, PreparedAsrError> {
        check(&self.cancel)?;
        if self.terminal {
            return Err(MediaError::InvalidAudio.into());
        }
        // No scheduling queue for CPU work: saturation is an explicit error.
        let cpu = self
            .cpu
            .clone()
            .try_acquire_owned()
            .map_err(|_| MediaError::Overloaded)?;
        if self.worker.is_none() {
            let owner = self
                .owners
                .clone()
                .try_acquire_owned()
                .map_err(|_| MediaError::Overloaded)?;
            let runtime = Handle::try_current().map_err(|_| MediaError::WorkerFailed)?;
            let (sender, receiver) = mpsc::channel(COMMAND_CAPACITY);
            let factory = self.factory.take().ok_or(MediaError::WorkerFailed)?;
            let identity = self.identity;
            let rate = self.rate;
            let cancel = self.cancel.clone();
            let idle_runtime = runtime.clone();
            self.worker = Some(runtime.spawn_blocking(move || {
                // Created before all native locals; held until they are actually
                // destroyed. abort() on a started worker is never exit proof.
                let _owner = owner;
                worker_loop(identity, rate, &cancel, factory, receiver, &idle_runtime)
            }));
            self.commands = Some(sender);
        }
        let (reply, receive) = oneshot::channel();
        self.commands
            .as_ref()
            .ok_or(MediaError::WorkerFailed)?
            .try_send(PullCommand { cpu, reply })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => MediaError::Overloaded,
                mpsc::error::TrySendError::Closed(_) => MediaError::WorkerFailed,
            })?;
        let result = tokio::select! { biased;
            _ = self.cancel.cancelled() => return Err(MediaError::Cancelled.into()),
            result = receive => result,
        };
        check(&self.cancel)?;
        let batch = match result {
            Ok(result) => result?,
            Err(_) => {
                self.join().await?;
                return Err(MediaError::WorkerFailed.into());
            }
        };
        if !batch.eof && batch.media.is_some() {
            return Err(MediaError::InvalidAudio.into());
        }
        if batch.eof {
            // True EOF includes native destruction, not just a queued reply.
            self.commands = None;
            self.join().await?;
            check(&self.cancel)?;
            self.terminal = true;
            self.cancel_on_drop = false;
        }
        Ok(batch)
    }
    async fn join(&mut self) -> Result<(), PreparedAsrError> {
        let worker = self.worker.as_mut().ok_or(MediaError::WorkerFailed)?;
        let result = tokio::select! { biased;
            _ = self.cancel.cancelled() => return Err(MediaError::Cancelled.into()),
            result = worker => result.map_err(|_| MediaError::WorkerFailed)?,
        };
        self.worker = None;
        result
    }
    fn terminate(&mut self) {
        self.terminal = true;
        self.cancel.cancel();
        self.factory = None;
        self.commands = None;
        if let Some(worker) = &self.worker {
            worker.abort();
        }
    }
}
impl Drop for PreparedLocalSource {
    fn drop(&mut self) {
        if self.cancel_on_drop {
            self.terminate();
        }
    }
}

fn worker_loop(
    identity: StreamIdentity,
    rate: PreparedSampleRate,
    cancel: &ResolverCancellation,
    factory: PinnedFactory,
    mut commands: mpsc::Receiver<PullCommand>,
    runtime: &Handle,
) -> Result<(), PreparedAsrError> {
    let mut factory = Some(factory);
    let mut core: Option<NativeCore> = None;
    loop {
        // This is the same native blocking thread, not an async CPU worker. The
        // idle future uses only runtime-independent channels/watch cancellation.
        // No CPU permit, timer, network, relay task or blocking queue holds PCM.
        let command = runtime.block_on(async {
            tokio::select! { biased;
                _ = cancel.cancelled() => None,
                command = commands.recv() => command,
            }
        });
        let Some(command) = command else {
            return check(cancel);
        };
        let PullCommand { cpu, reply } = command;
        let result = match std::panic::catch_unwind(AssertUnwindSafe(|| {
            check(cancel)?;
            if core.is_none() {
                let opened = (factory.take().ok_or(MediaError::WorkerFailed)?)(cancel)?;
                check(cancel)?;
                core = Some(NativeCore::new(identity, rate, opened, cancel)?);
            }
            core.as_mut().ok_or(MediaError::WorkerFailed)?.pull(cancel)
        })) {
            Ok(result) => result,
            Err(_) => Err(MediaError::WorkerFailed.into()),
        };
        let terminal = match &result {
            Ok(batch) => batch.eof,
            Err(_) => true,
        };
        if terminal {
            core = None;
        }
        // Normal EOF/error destroys native resources under this step's CPU
        // permit. Idle cancellation must always allow same-thread RAII cleanup;
        // it remains bounded by the retained native-owner permit.
        drop(cpu);
        let result = check(cancel).and(result);
        if reply.send(result).is_err() {
            return Err(MediaError::Cancelled.into());
        }
        if terminal {
            return Ok(());
        }
    }
}
struct NativeCore {
    decoder: Box<dyn MediaDecoder>,
    processor: PreparedAudioProcessor,
    segments: PreparedSegmentBuilder,
    clock: Option<NativeClock>,
    media: Option<OpenedMedia>,
}
struct NativeClock {
    normalizer: DecodedClockNormalizer,
    window: DecodedClockWindow,
}
impl NativeCore {
    fn new(
        identity: StreamIdentity,
        rate: PreparedSampleRate,
        opened: OpenedDecoder,
        cancel: &ResolverCancellation,
    ) -> Result<Self, PreparedAsrError> {
        let processor =
            PreparedAudioProcessor::new(identity, opened.decoder.format(), rate, cancel)?;
        let segments = PreparedSegmentBuilder::new(identity, rate)?;
        let clock = if opened.clock {
            Some(NativeClock {
                normalizer: DecodedClockNormalizer::new(identity, opened.decoder.format(), cancel)?,
                window: DecodedClockWindow::new(identity, cancel)?,
            })
        } else {
            None
        };
        Ok(Self {
            decoder: opened.decoder,
            processor,
            segments,
            clock,
            media: opened.media,
        })
    }
    fn pull(
        &mut self,
        cancel: &ResolverCancellation,
    ) -> Result<PreparedLocalBatch, PreparedAsrError> {
        let mut chunks = Vec::with_capacity(MAX_LOCAL_BATCH_CHUNKS);
        loop {
            check(cancel)?;
            let next = self.decoder.next(cancel)?;
            check(cancel)?;
            let eof = next.is_none();
            let (next, pending) = match (next, self.clock.as_mut()) {
                (Some(block), Some(clock)) => {
                    let mapped = clock.normalizer.normalize(block, cancel)?;
                    let (block, span) = mapped.into_parts();
                    clock.window.validate_pending(span, cancel)?;
                    (Some(block), Some(span))
                }
                (next, _) => (next, None),
            };
            if pending.is_some_and(|span| span.discontinuity()) {
                // Close the old real run before a long gap enters the window.
                // Zero-output new runs cannot keep an old body/context alive.
                self.emit_step(None, None, true, &mut chunks, cancel)?;
                self.segments.end_source_run(cancel)?;
                drain(
                    &mut self.segments,
                    self.clock.as_mut(),
                    None,
                    &mut chunks,
                    cancel,
                )?;
            }
            self.emit_step(next.as_ref(), pending, false, &mut chunks, cancel)?;
            if eof {
                self.segments.finish(cancel)?;
                drain(
                    &mut self.segments,
                    self.clock.as_mut(),
                    None,
                    &mut chunks,
                    cancel,
                )?;
            } else if let (Some(span), Some(clock)) = (pending, self.clock.as_mut()) {
                // Snapshot ready chunks before discarding original rows. Keep
                // the straddling row for both left context and unflushed DSP.
                let earliest = [
                    self.segments.retained_audio_start()?,
                    self.processor.retained_audio_start()?,
                ]
                .into_iter()
                .flatten()
                .min()
                .unwrap_or(span.sample_end_us());
                clock.window.append_retained(span, earliest, cancel)?;
            }
            check(cancel)?;
            if eof || !chunks.is_empty() {
                return Ok(PreparedLocalBatch {
                    chunks: chunks.into_boxed_slice(),
                    eof,
                    media: if eof { self.media.take() } else { None },
                });
            }
        }
    }

    fn emit_step(
        &mut self,
        block: Option<&super::decoded::DecodedAudioBlock>,
        pending: Option<DecodedClockSpan>,
        flush_run: bool,
        chunks: &mut Vec<PreparedSpeechChunk>,
        cancel: &ResolverCancellation,
    ) -> Result<(), PreparedAsrError> {
        let mut error = None;
        let segments = &mut self.segments;
        let clock = &mut self.clock;
        let mut emit =
            |block: super::decoded::DecodedAudioBlock, gap, scope: &ResolverCancellation| {
                let gap = gap && block.sequence() != 1;
                let result = segments
                    .push(block, gap, scope)
                    .map_err(PreparedAsrError::from)
                    .and_then(|()| drain(segments, clock.as_mut(), pending, chunks, scope));
                if let Err(failure) = result {
                    error = Some(failure);
                    return Err(MediaError::InvalidAudio);
                }
                Ok(())
            };
        let result = if flush_run {
            self.processor.flush_run_owned(cancel, &mut emit)
        } else if let Some(block) = block {
            self.processor.push_owned(block, cancel, &mut emit)
        } else {
            self.processor.finish_owned(cancel, &mut emit).map(|_| ())
        };
        if let Some(error) = error {
            return Err(error);
        }
        result.map_err(PreparedAsrError::from)
    }
}
fn drain(
    segments: &mut PreparedSegmentBuilder,
    mut clock: Option<&mut NativeClock>,
    pending: Option<DecodedClockSpan>,
    chunks: &mut Vec<PreparedSpeechChunk>,
    cancel: &ResolverCancellation,
) -> Result<(), PreparedAsrError> {
    while let Some(chunk) = segments.take() {
        if chunks.len() == MAX_LOCAL_BATCH_CHUNKS {
            return Err(PreparedSpeechError::Overloaded.into());
        }
        let chunk = if let Some(clock) = clock.as_mut() {
            let stamp = chunk.stamp();
            let snapshot = if let Some(span) = pending {
                clock.window.snapshot_with_pending(
                    stamp.identity,
                    stamp.audio_start_us,
                    chunk.audio_end_us(),
                    span,
                    cancel,
                )?
            } else {
                clock.window.snapshot(
                    stamp.identity,
                    stamp.audio_start_us,
                    chunk.audio_end_us(),
                    cancel,
                )?
            };
            chunk.with_clock_map(snapshot)?
        } else {
            chunk
        };
        chunks.push(chunk);
    }
    Ok(())
}
fn check(cancel: &ResolverCancellation) -> Result<(), PreparedAsrError> {
    if cancel.is_cancelled() {
        Err(MediaError::Cancelled.into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "prepared_producer_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "prepared_producer_clock_tests.rs"]
mod clock_tests;
