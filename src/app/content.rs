//! Formal Live composition of the optional resolver branch. Never owns a renderer
//! or subtitle source; STT admission and failures stay independent.
use crate::{
    ai::speech::{SpeechError, SpeechInput, StreamIdentity, TranscriptEvent},
    app::lyrics::{LiveLyrics, LyricsConfiguration},
    app::subtitles::{LiveSubtitles, SubtitleConfiguration},
    audio::preprocess::NormalizedAudio,
    resolver::{
        audio_worker::AudioWorkerMetrics,
        candidates::CONFIRMED_FRESHNESS,
        contracts::{ContentIdentifier, ResolverError, WindowStamp},
        movie_hints::MovieHintBuffer,
        session::{ResolverStatus, ResolverUpdate},
    },
    session::live_resolver::{LiveResolver, submit_live_audio},
    ui::content::{ContentDisclosure, UiContentState, UiContentStatus},
    ui::lyrics::UiLyricsStatus,
    ui::subtitles::UiSubtitleStatus,
};
use futures_util::FutureExt;
use std::{panic::AssertUnwindSafe, sync::Arc, time::Instant};

/// Explicit configuration only. Create a new Session-scoped identifier on every
/// capture/reconnect. This synchronous method must not perform network or long CPU
/// work; the returned identifier owns bounded asynchronous/CPU work and cancellation.
pub trait ContentServiceFactory: Send + Sync {
    fn create(&self, identity: StreamIdentity)
    -> Result<Arc<dyn ContentIdentifier>, ResolverError>;
    /// Optional, explicitly disclosed Final-ASR ingress. Ordinary factories keep
    /// their existing audio-only contract and reject enabling this ingress unless
    /// they implement it. Construction must remain non-blocking.
    fn create_with_movie_hints(
        &self,
        _identity: StreamIdentity,
        _hints: Arc<MovieHintBuffer>,
    ) -> Result<Arc<dyn ContentIdentifier>, ResolverError> {
        Err(ResolverError::UnsupportedCapability)
    }
    /// Bind metadata to the explicitly selected, creation-verified process before
    /// constructing a Session. Pure configuration only; no native or network work.
    fn bind_source(
        &self,
        _selected: SelectedProcess,
    ) -> Result<Option<Arc<dyn ContentServiceFactory>>, ResolverError> {
        Ok(None)
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SelectedProcess {
    pub pid: u32,
    pub created: Option<u64>,
}
struct FailedFactory(ResolverError);
impl ContentServiceFactory for FailedFactory {
    fn create(
        &self,
        _identity: StreamIdentity,
    ) -> Result<Arc<dyn ContentIdentifier>, ResolverError> {
        Err(self.0)
    }
    fn create_with_movie_hints(
        &self,
        identity: StreamIdentity,
        _hints: Arc<MovieHintBuffer>,
    ) -> Result<Arc<dyn ContentIdentifier>, ResolverError> {
        self.create(identity)
    }
}
#[derive(Clone)]
pub struct ContentConfiguration {
    disclosure: ContentDisclosure,
    factory: Arc<dyn ContentServiceFactory>,
    lyrics: Option<LyricsConfiguration>,
    subtitles: Option<SubtitleConfiguration>,
    movie_dialogue: bool,
}
impl ContentConfiguration {
    /// The disclosure describes every destination used by this configured service.
    /// A remote service must never be presented as local. No automatic fallback.
    pub fn new(
        disclosure: ContentDisclosure,
        factory: Arc<dyn ContentServiceFactory>,
    ) -> Result<Self, ResolverError> {
        disclosure.validate()?;
        Ok(Self {
            disclosure,
            factory,
            lyrics: None,
            subtitles: None,
            movie_dialogue: false,
        })
    }
    pub fn disclosure(&self) -> ContentDisclosure {
        self.disclosure
    }
    /// A metadata binding failure disables only this optional branch, preserving
    /// its disclosed destination and the independent Live STT/translation run.
    pub fn for_selected_source(&self, selected: SelectedProcess) -> Self {
        let mut bound = self.clone();
        match std::panic::catch_unwind(AssertUnwindSafe(|| self.factory.bind_source(selected)))
            .unwrap_or(Err(ResolverError::WorkerPanicked))
        {
            Ok(Some(factory)) => bound.factory = factory,
            Ok(None) => {}
            Err(error) => bound.factory = Arc::new(FailedFactory(error)),
        }
        bound
    }
    /// Enable only for a service whose visible disclosure covers current Final
    /// ASR and media hints. This never enables another service or provider fallback.
    pub fn with_movie_dialogue(mut self) -> Self {
        self.movie_dialogue = true;
        self
    }
    pub fn movie_dialogue_enabled(&self) -> bool {
        self.movie_dialogue
    }
    pub fn with_lyrics(mut self, lyrics: LyricsConfiguration) -> Self {
        self.lyrics = Some(lyrics);
        self
    }
    pub fn lyrics_disclosure(&self) -> Option<ContentDisclosure> {
        self.lyrics.as_ref().map(LyricsConfiguration::disclosure)
    }
    pub fn with_subtitles(mut self, subtitles: SubtitleConfiguration) -> Self {
        self.subtitles = Some(subtitles);
        self
    }
    pub fn subtitles_disclosure(&self) -> Option<ContentDisclosure> {
        self.subtitles
            .as_ref()
            .map(SubtitleConfiguration::disclosure)
    }
}
pub struct LiveContent {
    identity: StreamIdentity,
    resolver: Option<LiveResolver>,
    status: UiContentStatus,
    previous: Option<WindowStamp>,
    expires: Option<Instant>,
    lyrics: LiveLyrics,
    subtitles: LiveSubtitles,
    movie_hints: Option<Arc<MovieHintBuffer>>,
}
impl LiveContent {
    pub fn new(identity: StreamIdentity, config: Option<&ContentConfiguration>) -> Self {
        let mut owner = Self {
            identity,
            resolver: None,
            status: UiContentStatus::default(),
            previous: None,
            expires: None,
            lyrics: LiveLyrics::new(identity, config.and_then(|config| config.lyrics.as_ref())),
            subtitles: LiveSubtitles::new(
                identity,
                config.and_then(|config| config.subtitles.as_ref()),
            ),
            movie_hints: config
                .filter(|config| config.movie_dialogue)
                .map(|_| Arc::new(MovieHintBuffer::new(identity))),
        };
        if let Some(config) = config {
            owner.status.disclosure = Some(config.disclosure);
            owner.status.movie_dialogue = config.movie_dialogue;
            let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
                match &owner.movie_hints {
                    Some(hints) => config
                        .factory
                        .create_with_movie_hints(identity, hints.clone()),
                    None => config.factory.create(identity),
                }
                .and_then(|provider| LiveResolver::new(identity, provider))
            }))
            .unwrap_or(Err(ResolverError::WorkerPanicked));
            match result {
                Ok(resolver) => {
                    owner.resolver = Some(resolver);
                    owner.status.state = UiContentState::Collecting;
                }
                Err(error) => {
                    if let Some(hints) = &owner.movie_hints {
                        hints.stop();
                    }
                    owner.status.state = UiContentState::Failed(error);
                    owner.lyrics.fail(error);
                    owner.subtitles.fail(error);
                }
            }
        }
        owner
    }
    pub fn status(&self) -> UiContentStatus {
        self.status
    }
    pub fn movie_dialogue_enabled(&self) -> bool {
        self.movie_hints.is_some() && self.resolver.is_some()
    }
    pub fn lyrics_status(&self) -> UiLyricsStatus {
        self.lyrics.status()
    }
    pub fn ready_lyrics(&self) -> Option<crate::resolver::text_session::ExistingTextLease> {
        self.lyrics.ready_text()
    }
    pub fn subtitles_status(&self) -> UiSubtitleStatus {
        self.subtitles.status()
    }
    pub fn ready_subtitles(&self) -> Option<crate::resolver::text_session::ExistingTextLease> {
        self.subtitles.ready_text()
    }
    /// Settled music/movie selections retire the other source. Both share one
    /// downstream verifier/preparation task and the original transcript timeline.
    pub fn ready_text(&self) -> Option<crate::resolver::text_session::ExistingTextLease> {
        self.ready_subtitles().or_else(|| self.ready_lyrics())
    }
    /// Playback timing is no longer reliable; retire the publication immediately.
    /// A new settled resolver confirmation must select text again.
    pub fn reset_text_for_audio_gap(&mut self) -> bool {
        self.clear_movie_hints();
        self.lyrics.clear(None) | self.subtitles.clear(None)
    }
    fn clear_movie_hints(&self) {
        if let Some(hints) = &self.movie_hints {
            hints.clear();
        }
    }
    /// Route only an event already accepted by the speech/translation ledger.
    /// This bounded RAM copy never calls a catalog or affects STT admission.
    pub fn observe_transcript(&mut self, event: &TranscriptEvent) -> bool {
        let Some(hints) = &self.movie_hints else {
            return false;
        };
        if self.resolver.is_none() {
            return false;
        }
        match hints.observe(event) {
            Ok(changed) => changed,
            Err(ResolverError::Stale) => false,
            Err(_) => false, // Rejected hints cannot retire a newer valid publication.
        }
    }
    pub fn metrics(&self) -> Option<AudioWorkerMetrics> {
        self.resolver.as_ref().map(LiveResolver::metrics)
    }
    fn set_state(&mut self, state: UiContentState) -> bool {
        let changed = self.status.state != state;
        self.status.state = state;
        changed
    }
    /// Called only by the Live orchestration drain, never by capture callbacks.
    /// Returns whether the lightweight UI status changed. STT errors remain fatal
    /// to the speech run; resolver errors never cancel or retry the speech provider.
    pub fn submit_audio(
        &mut self,
        speech: &SpeechInput,
        audio: NormalizedAudio,
    ) -> Result<bool, SpeechError> {
        if audio.stamp.discontinuity {
            self.clear_movie_hints();
        }
        match submit_live_audio(speech, self.resolver.as_ref(), audio)? {
            Some(Err(error)) => {
                self.clear_movie_hints();
                let before = self.lyrics.status();
                let before_subtitles = self.subtitles.status();
                self.expires = None;
                if matches!(
                    error,
                    ResolverError::Cancelled | ResolverError::WorkerPanicked
                ) {
                    if let Some(hints) = &self.movie_hints {
                        hints.stop();
                    }
                    self.resolver.take();
                    self.lyrics.fail(error);
                    self.subtitles.fail(error);
                } else {
                    self.lyrics.clear(Some(error));
                    self.subtitles.clear(Some(error));
                }
                let lyrics_changed = self.lyrics.status() != before;
                Ok(self.set_state(UiContentState::Failed(error))
                    | lyrics_changed
                    | (self.subtitles.status() != before_subtitles))
            }
            _ => Ok(false),
        }
    }
    fn observe(&mut self, update: &ResolverUpdate) -> Result<UiContentState, ResolverError> {
        // A reset may use the preceding window's stamp. It must clear the visible
        // state even while a fresh full window is still being collected.
        if update.stamp.identity != self.identity {
            return Err(ResolverError::InvalidData);
        }
        if matches!(
            update.status,
            ResolverStatus::Failed(ResolverError::AudioDiscontinuity)
        ) {
            self.clear_movie_hints();
            self.expires = None;
            self.lyrics.clear(None);
            self.subtitles.clear(None);
            return Ok(UiContentState::Collecting);
        }
        update.stamp.validate(self.identity, Instant::now())?;
        if self.previous.is_some_and(|old| {
            update.stamp.sequence < old.sequence || update.stamp.ended_at < old.ended_at
        }) {
            return Err(ResolverError::Stale);
        }
        self.previous = Some(update.stamp);
        self.expires = update.stamp.ended_at.checked_add(CONFIRMED_FRESHNESS);
        Ok(match update.status {
            ResolverStatus::Resolving => UiContentState::Identifying,
            ResolverStatus::Ready => {
                // Song lookup failures never invalidate content identification or STT.
                if let Err(error) = self.lyrics.select(update.stamp, &update.resolution)
                    && error != ResolverError::Stale
                {
                    self.lyrics.clear(Some(error));
                }
                if let Err(error) = self.subtitles.select(update.stamp, &update.resolution)
                    && error != ResolverError::Stale
                {
                    self.subtitles.clear(Some(error));
                }
                UiContentState::Ready(update.resolution.kind, update.resolution.song_state)
            }
            ResolverStatus::Failed(error) => {
                self.expires = None;
                self.lyrics.clear(Some(error));
                self.subtitles.clear(Some(error));
                UiContentState::Failed(error)
            }
            ResolverStatus::Stopped => {
                if let Some(hints) = &self.movie_hints {
                    hints.stop();
                }
                self.expires = None;
                self.lyrics.stop();
                self.subtitles.stop();
                UiContentState::Stopped
            }
        })
    }
    /// Caller-polled along with speech and translation. Disabled/failed owners
    /// remain pending instead of producing an always-ready busy loop.
    pub async fn changed(&mut self) -> UiContentState {
        loop {
            let Some(resolver) = &mut self.resolver else {
                return std::future::pending().await;
            };
            let previous_lyrics = self.lyrics.status();
            let previous_subtitles = self.subtitles.status();
            let result = tokio::select! { biased;
                result = AssertUnwindSafe(resolver.changed()).catch_unwind() => Some(result.unwrap_or(Err(ResolverError::WorkerPanicked))),
                _ = async { match self.expires { Some(at) => tokio::time::sleep_until(at.into()).await, None => std::future::pending().await } } => None,
                _ = self.lyrics.changed() => return self.status.state,
                _ = self.subtitles.changed() => return self.status.state,
            };
            let state = match result {
                Some(Ok(update)) => self.observe(&update).unwrap_or_else(|error| {
                    if error == ResolverError::Stale {
                        return self.status.state;
                    }
                    self.expires = None;
                    self.lyrics.clear(Some(error));
                    self.subtitles.clear(Some(error));
                    UiContentState::Failed(error)
                }),
                Some(Err(error)) => {
                    if let Some(hints) = &self.movie_hints {
                        hints.stop();
                    }
                    self.resolver.take(); // cancels, drops requests and joins the CPU lane
                    self.expires = None;
                    self.lyrics.fail(error);
                    self.subtitles.fail(error);
                    UiContentState::Failed(error)
                }
                None => {
                    self.expires = None;
                    self.lyrics.clear(Some(ResolverError::Stale));
                    self.subtitles.clear(Some(ResolverError::Stale));
                    UiContentState::Failed(ResolverError::Stale)
                }
            };
            if self.set_state(state)
                || self.lyrics.status() != previous_lyrics
                || self.subtitles.status() != previous_subtitles
            {
                return state;
            }
        }
    }
    pub fn stop(&mut self) {
        if let Some(hints) = &self.movie_hints {
            hints.stop();
        }
        self.resolver.take();
        self.expires = None;
        self.previous = None;
        self.status.stop();
        self.lyrics.stop();
        self.subtitles.stop();
    }
}
impl Drop for LiveContent {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
#[path = "subtitle_live_tests.rs"]
mod subtitle_tests;
#[cfg(test)]
#[path = "content_tests.rs"]
mod tests;
