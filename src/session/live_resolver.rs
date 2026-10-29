//! Caller-polled ownership of the independent resolver branch. No renderer or
//! automatic provider selection lives here. Dropping it joins the audio CPU lane.
use crate::{
    ai::speech::{SpeechError, SpeechInput, StreamIdentity},
    audio::preprocess::NormalizedAudio,
    resolver::{
        audio_worker::{AudioAssembly, AudioWorkerMetrics, ResolverAudioWorker},
        contracts::{ContentIdentifier, ResolverError},
        session::{ResolverSession, ResolverUpdate},
    },
};
use std::{future::Future, pin::Pin, sync::Arc};

type ResolverRun = Pin<Box<dyn Future<Output = Result<(), ResolverError>> + Send>>;
pub struct LiveResolver {
    audio: ResolverAudioWorker,
    session: ResolverSession,
    run: Option<ResolverRun>,
    epoch: u64,
}
impl LiveResolver {
    /// Provider configuration must be explicit and scoped to this fresh identity.
    pub fn new(
        identity: StreamIdentity,
        provider: Arc<dyn ContentIdentifier>,
    ) -> Result<Self, ResolverError> {
        let (session, task) = ResolverSession::new(identity, provider.capabilities())?;
        let audio = ResolverAudioWorker::new(identity)?;
        Ok(Self {
            audio,
            session,
            epoch: 0,
            run: Some(Box::pin(async move { task.run(provider.as_ref()).await })),
        })
    }
    pub fn try_push(&self, audio: NormalizedAudio) -> Result<(), ResolverError> {
        self.audio.input().try_push(audio)
    }
    pub fn metrics(&self) -> AudioWorkerMetrics {
        self.audio.metrics()
    }
    fn accept(&mut self, assembly: Arc<AudioAssembly>) -> Result<(), ResolverError> {
        if assembly.epoch != self.epoch {
            self.session.invalidate()?;
            self.epoch = assembly.epoch;
        }
        if let Some(window) = &assembly.window {
            self.session.try_submit(Arc::clone(window))?;
        }
        if matches!(
            assembly.error,
            Some(ResolverError::Cancelled | ResolverError::WorkerPanicked)
        ) {
            return Err(assembly.error.unwrap_or(ResolverError::Cancelled));
        }
        Ok(())
    }
    /// Poll along with STT/translation in the owner's select loop. Audio reset is
    /// considered first so old evidence cannot survive while collecting a new window.
    pub async fn changed(&mut self) -> Result<Arc<ResolverUpdate>, ResolverError> {
        loop {
            let Some(run) = self.run.as_mut() else {
                return Err(ResolverError::Cancelled);
            };
            tokio::select! { biased;
                assembly = self.audio.changed() => {
                    if let Err(error) = self.accept(assembly) {
                        let _ = self.stop();
                        return Err(error);
                    }
                },
                result = run => {
                    self.run = None;
                    let _ = self.audio.stop();
                    return Err(result.err().unwrap_or(ResolverError::Cancelled));
                },
                update = self.session.next() => {
                    // A CPU producer can reset between select's first branch poll
                    // and this one. Recheck the publication gate before adoption.
                    let assembly = self.audio.current();
                    if let Some(error @ (ResolverError::Cancelled | ResolverError::WorkerPanicked)) = assembly.error {
                        let _ = self.stop();
                        return Err(error);
                    }
                    if assembly.epoch != self.epoch {
                        let latest = self.audio.latest();
                        if let Err(error) = self.accept(latest) {
                            let _ = self.stop();
                            return Err(error);
                        }
                        continue;
                    }
                    return update.ok_or(ResolverError::Cancelled);
                },
            }
        }
    }
    pub fn stop(&mut self) -> Result<(), ResolverError> {
        self.session.stop();
        self.run = None;
        self.audio.stop()
    }
}
impl Drop for LiveResolver {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

/// STT admission happens first. Only an explicitly configured resolver receives a
/// fixed-size RAM copy; its independent failure is returned without cancelling STT.
/// This is an orchestration boundary, never a native audio callback.
pub fn submit_live_audio(
    speech: &SpeechInput,
    resolver: Option<&LiveResolver>,
    audio: NormalizedAudio,
) -> Result<Option<Result<(), ResolverError>>, SpeechError> {
    let copy = resolver.map(|_| NormalizedAudio {
        stamp: audio.stamp,
        frames: audio.frames,
        samples: audio.samples,
    });
    speech.try_push(audio)?;
    Ok(resolver
        .zip(copy)
        .map(|(resolver, audio)| resolver.try_push(audio)))
}

#[cfg(test)]
#[path = "live_resolver_tests.rs"]
mod tests;
