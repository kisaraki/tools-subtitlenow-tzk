//! Native decode plus bounded Prepared mono preprocessing on one shared worker.
//! No Live capture stamp, cloud request, playback or persistent PCM exists here.
use super::{
    MediaError,
    decode_worker::{DecodeConsumer, DecodedAudioSummary, decode_consumer},
    decoded::DecodedAudioBlock,
    input::MediaInput,
    prepared_audio::{
        PreparedAudioProcessor, PreparedAudioSummary, PreparedPcmBlock, PreparedSampleRate,
    },
};
use crate::{ai::speech::StreamIdentity, resolver::contracts::ResolverCancellation};
use std::future::Future;

/// The owner must poll this future; even an unpolled Drop cancels the scope.
/// Native creation/destruction, mixing, FIR and EOF flush share the same bounded
/// worker/permit. Only one borrowed <=20 ms mono PCM block reaches `consumer`.
///
/// `consumer` must do short bounded CPU work, must not retain PCM copies, and
/// must not perform network/disk/UI operations or wait for queue capacity. This
/// is an audio preparation entry point, not ASR, a Ready document or playback.
pub fn prepare_local_audio<C>(
    input: MediaInput,
    identity: StreamIdentity,
    rate: PreparedSampleRate,
    cancel: ResolverCancellation,
    consumer: C,
) -> impl Future<Output = Result<PreparedAudioSummary, MediaError>> + Send
where
    C: FnMut(&PreparedPcmBlock, &ResolverCancellation) -> Result<(), MediaError> + Send + 'static,
{
    decode_consumer(
        input,
        identity,
        cancel,
        PreparedConsumer::new(identity, rate, consumer),
    )
}

struct PreparedConsumer<C> {
    identity: StreamIdentity,
    rate: PreparedSampleRate,
    processor: Option<PreparedAudioProcessor>,
    consumer: C,
}
impl<C> PreparedConsumer<C> {
    fn new(identity: StreamIdentity, rate: PreparedSampleRate, consumer: C) -> Self {
        Self {
            identity,
            rate,
            processor: None,
            consumer,
        }
    }
}
impl<C> DecodeConsumer for PreparedConsumer<C>
where
    C: FnMut(&PreparedPcmBlock, &ResolverCancellation) -> Result<(), MediaError> + Send + 'static,
{
    type Output = PreparedAudioSummary;
    fn push(
        &mut self,
        block: &DecodedAudioBlock,
        cancel: &ResolverCancellation,
    ) -> Result<(), MediaError> {
        if self.processor.is_none() {
            self.processor = Some(PreparedAudioProcessor::new(
                self.identity,
                block.format(),
                self.rate,
                cancel,
            )?);
        }
        self.processor
            .as_mut()
            .ok_or(MediaError::InvalidAudio)?
            .push(block, cancel, &mut self.consumer)
    }
    fn finish(
        mut self,
        decoded: DecodedAudioSummary,
        cancel: &ResolverCancellation,
    ) -> Result<Self::Output, MediaError> {
        if decoded.identity() != self.identity {
            return Err(MediaError::Stale);
        }
        self.processor
            .as_mut()
            .ok_or(MediaError::InvalidAudio)?
            .finish(cancel, &mut self.consumer)
    }
}

#[cfg(test)]
#[path = "prepared_worker_tests.rs"]
mod tests;
