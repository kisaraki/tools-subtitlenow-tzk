//! Demand-driven DSP: one owned state/input/job; no channel or audio read-ahead.
use super::{
    MediaError,
    prepared_audio::{PreparedAudioProcessor, PreparedSampleRate},
    prepared_segments::PreparedSegmentBuilder,
    remote_prepared::{RemotePreparedError, check},
    remote_wave::{RemoteWaveBlock, RemoteWaveDecoder},
};
use crate::{
    ai::prepared_speech::{PreparedSpeechChunk, PreparedSpeechError},
    resolver::contracts::ResolverCancellation,
};
use std::{panic::AssertUnwindSafe, sync::Arc};
use tokio::{sync::Semaphore, task::JoinHandle};

// One input <=32768 frames (<=4.096 s at 8 kHz). Existing 10 s bodies plus
// EOF flush can emit at most two chunks per pull, as on the local path.
pub(super) const MAX_REMOTE_BATCH_CHUNKS: usize = 2;
pub(super) struct Batch {
    pub chunks: Box<[PreparedSpeechChunk]>,
    pub eof: bool,
}
pub(super) struct RemotePreparedSource {
    core: Option<Core>,
    rate: PreparedSampleRate,
    cpu: Arc<Semaphore>,
    terminal: bool,
}
// Aborted queued jobs retain captured permits until the runtime reaps them;
// started jobs retain CPU/PCM permits until their actual checked exit.
struct Job<T>(JoinHandle<T>);
impl<T> Drop for Job<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}
impl RemotePreparedSource {
    pub fn new(rate: PreparedSampleRate, cpu: Arc<Semaphore>) -> Self {
        Self {
            core: None,
            rate,
            cpu,
            terminal: false,
        }
    }
    pub async fn pull(
        &mut self,
        source: &mut RemoteWaveDecoder,
    ) -> Result<Batch, RemotePreparedError> {
        let cancel = source.cancellation();
        check(&cancel)?;
        if self.terminal {
            return Err(MediaError::InvalidAudio.into());
        }
        loop {
            // No CPU permit while awaiting HTTP. A failure keeps its original
            // typed transport error, even though it also cancels the source.
            let input = source.read_next().await?;
            let eof = input.is_none();
            check(&cancel)?;
            let permit = self
                .cpu
                .clone()
                .try_acquire_owned()
                .map_err(|_| MediaError::Overloaded)?;
            let runtime =
                tokio::runtime::Handle::try_current().map_err(|_| MediaError::WorkerFailed)?;
            let core = self.core.take();
            let identity = source.identity();
            let format = source.format();
            let rate = self.rate;
            let scope = cancel.clone();
            let mut job = Job(runtime.spawn_blocking(move || {
                let _permit = permit;
                std::panic::catch_unwind(AssertUnwindSafe(|| {
                    check(&scope)?;
                    let mut core = match core {
                        Some(core) => core,
                        None => Core {
                            processor: PreparedAudioProcessor::new(identity, format, rate, &scope)?,
                            segments: PreparedSegmentBuilder::new(identity, rate)?,
                        },
                    };
                    let chunks = core.step(input, &scope)?;
                    check(&scope)?;
                    Ok((if eof { None } else { Some(core) }, chunks))
                }))
                .unwrap_or_else(|_| Err(RemotePreparedError::from(MediaError::WorkerFailed)))
            }));
            // On ordinary cancellation, wait for real DSP cleanup. Dropped
            // outer futures cancel via the source/guard, with permits retained
            // by any started worker, so replacements cannot bypass capacity.
            let result = tokio::select! { biased;
                _=cancel.cancelled()=>{
                    let _ = (&mut job.0).await;
                    return Err(PreparedSpeechError::Cancelled.into());
                },
                result=&mut job.0=>result.map_err(|_| MediaError::WorkerFailed)?,
            };
            let (core, chunks) = result?;
            check(&cancel)?;
            self.core = core;
            self.terminal = eof;
            if eof || !chunks.is_empty() {
                return Ok(Batch {
                    chunks: chunks.into_boxed_slice(),
                    eof,
                });
            }
        }
    }
}
struct Core {
    processor: PreparedAudioProcessor,
    segments: PreparedSegmentBuilder,
}
impl Core {
    fn step(
        &mut self,
        input: Option<RemoteWaveBlock>,
        cancel: &ResolverCancellation,
    ) -> Result<Vec<PreparedSpeechChunk>, RemotePreparedError> {
        let mut chunks = Vec::with_capacity(MAX_REMOTE_BATCH_CHUNKS);
        let segments = &mut self.segments;
        let mut failure = None;
        let mut emit =
            |block: super::decoded::DecodedAudioBlock, gap, scope: &ResolverCancellation| {
                let gap = gap && block.sequence() != 1;
                let result = segments
                    .push(block, gap, scope)
                    .map_err(RemotePreparedError::from)
                    .and_then(|()| drain(segments, &mut chunks));
                if let Err(error) = result {
                    failure = Some(error);
                    return Err(MediaError::InvalidAudio);
                }
                Ok(())
            };
        let eof = input.is_none();
        let result = if let Some(input) = input.as_ref() {
            self.processor.push_owned(input.block()?, cancel, &mut emit)
        } else {
            self.processor.finish_owned(cancel, &mut emit).map(|_| ())
        };
        if let Some(failure) = failure {
            return Err(failure);
        }
        result?;
        if eof {
            segments.finish(cancel)?;
            drain(segments, &mut chunks)?;
        }
        check(cancel)?;
        Ok(chunks)
    }
}
fn drain(
    segments: &mut PreparedSegmentBuilder,
    chunks: &mut Vec<PreparedSpeechChunk>,
) -> Result<(), RemotePreparedError> {
    while let Some(chunk) = segments.take() {
        if chunks.len() == MAX_REMOTE_BATCH_CHUNKS {
            return Err(PreparedSpeechError::Overloaded.into());
        }
        chunks.push(chunk);
    }
    Ok(())
}
