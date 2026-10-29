//! Shared production/test seam for Prepared alignment orchestration. The
//! caller supplies actual capture metadata and its existing translation flow.
use crate::{
    ai::speech::{StreamIdentity, TranscriptEvent},
    session::{
        aligned_playback::{AlignedPlayback, AlignedPlaybackState, RealignmentReason},
        alignment_control::AlignmentControlError,
        fuzzy_proposals::ProposalState,
        live_translation::LiveTranslation,
        playback_clock::LivePlaybackClock,
    },
    subtitle::fuzzy_worker::AlignmentSource,
    ui::alignment::AlignmentCommand,
};
pub(crate) struct PreparedLiveAlignment {
    engine: AlignedPlayback,
    source: AlignmentSource,
    identity: StreamIdentity,
    command: AlignmentCommand,
}
impl PreparedLiveAlignment {
    async fn finish_action(
        &mut self,
        result: Result<(), AlignmentControlError>,
    ) -> Result<(), AlignmentControlError> {
        if result.is_err() && !self.source.is_available() {
            self.engine.stop().await
        } else {
            result
        }
    }
    pub(crate) async fn new(
        source: AlignmentSource,
        identity: StreamIdentity,
        command: AlignmentCommand,
    ) -> Result<Self, AlignmentControlError> {
        let mut engine = AlignedPlayback::new(&source, identity)?;
        if command.running {
            engine.resume().await?;
        }
        Ok(Self {
            engine,
            source,
            identity,
            command,
        })
    }
    pub(crate) fn running(&self) -> bool {
        self.engine.running()
    }
    pub(crate) fn state(&self) -> AlignedPlaybackState {
        self.engine.state()
    }
    pub(crate) async fn changed(&mut self) -> Result<ProposalState, AlignmentControlError> {
        self.engine.changed().await
    }
    pub(crate) async fn command(
        &mut self,
        command: AlignmentCommand,
    ) -> Result<(), AlignmentControlError> {
        if !self.source.is_available() {
            return self.engine.stop().await;
        }
        let result = if !command.running {
            self.engine.pause().await
        } else if command.reset != self.command.reset {
            self.engine
                .reset(self.identity, RealignmentReason::ExplicitReset)
                .await
        } else if !self.engine.running() {
            self.engine.resume().await
        } else {
            Ok(())
        };
        self.finish_action(result).await?;
        self.command = command;
        Ok(())
    }
    pub(crate) async fn audio_gap(&mut self) -> Result<(), AlignmentControlError> {
        if !self.source.is_available() {
            return self.engine.stop().await;
        }
        if self.engine.running() {
            let result = self
                .engine
                .reset(self.identity, RealignmentReason::AudioGap)
                .await;
            self.finish_action(result).await?;
        }
        Ok(())
    }
    pub(crate) async fn offer(
        &mut self,
        event: TranscriptEvent,
        clock: &LivePlaybackClock,
    ) -> Result<(), AlignmentControlError> {
        if event.kind != crate::ai::speech::TranscriptKind::Final {
            return Ok(());
        }
        if !self.source.is_available() {
            return self.engine.stop().await;
        }
        if !self.engine.running() {
            return Ok(());
        }
        if let Some(stamp) = clock.cursor().and_then(|cursor| cursor.window()) {
            if let Err(error) = self.engine.offer(event, stamp) {
                let AlignmentControlError::Alignment(input_error) = error else {
                    return Err(error);
                };
                let result = self
                    .engine
                    .reset(self.identity, RealignmentReason::Discontinuity)
                    .await;
                self.finish_action(result).await?;
                if self.source.is_available() {
                    self.engine.reject_input(input_error.into());
                }
            }
        } else {
            let result = self
                .engine
                .reset(self.identity, RealignmentReason::Discontinuity)
                .await;
            self.finish_action(result).await?;
            if self.source.is_available() {
                self.engine.missing_capture();
            }
        }
        Ok(())
    }
    pub(crate) async fn reconcile(
        &mut self,
        clock: &LivePlaybackClock,
        flow: Option<&mut LiveTranslation<'_>>,
    ) -> Result<bool, AlignmentControlError> {
        if !self.source.is_available() {
            let changed = self.engine.state() != AlignedPlaybackState::Stopped;
            self.engine.stop().await?;
            return Ok(changed | flow.is_some_and(|flow| flow.reconcile_sources().is_some()));
        }
        let mut changed = if self.engine.running() {
            match self.engine.reconcile(clock.cursor()).await {
                Ok(changed) => changed,
                Err(_) if !self.source.is_available() => {
                    self.engine.stop().await?;
                    true
                }
                Err(error) => return Err(error),
            }
        } else {
            false
        };
        if let Some(flow) = flow {
            if let Some(anchor) = self.engine.take_verified() {
                match flow.adopt_prepared(&self.source, anchor) {
                    Ok(adopted) => changed |= adopted,
                    Err(error) => {
                        self.engine.reject_adoption(error);
                        changed = true;
                    }
                }
            }
            changed |= flow.reconcile_sources().is_some();
        }
        Ok(changed)
    }
    pub(crate) async fn stop(&mut self) -> Result<(), AlignmentControlError> {
        self.engine.stop().await
    }
}

#[cfg(test)]
#[path = "live_alignment_tests.rs"]
mod tests;
