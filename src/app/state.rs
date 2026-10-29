//! Lifecycle vocabulary from SPEC.md. Live orchestration emits its transitions;
//! Phase 8's document owner also exposes PreparingMedia/PreparedReady; playback
//! and native notification wiring remain separate from this state vocabulary.

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AppState {
    #[default]
    Idle,
    SourceSelected,
    StartingLive,
    LiveRunning,
    PreparingMedia,
    PreparedReady,
    PreparedPlaying,
    Stopping,
    Recovering,
    Error,
}
