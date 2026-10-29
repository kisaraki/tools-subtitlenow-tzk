//! Session-only confirmed-song lookup composition. No renderer, localization,
//! source arbitration, provider fallback or automatic service selection.
use crate::{
    ai::speech::StreamIdentity,
    resolver::{
        candidates::{Resolution, SONG_CONFIDENCE, SongState},
        contracts::*,
        text_session::{ExistingTextLease, ExistingTextSession, TextLookupStatus},
    },
    ui::{
        content::ContentDisclosure,
        lyrics::{UiLyricsState, UiLyricsStatus},
    },
};
use futures_util::FutureExt;
use std::{future::Future, panic::AssertUnwindSafe, pin::Pin, sync::Arc, time::Instant};

/// Create one binder/service for each new Live Session. No synchronous network,
/// long CPU work or credential lookup is allowed in create or bind.
pub trait LyricsServiceFactory: Send + Sync {
    fn create(&self, identity: StreamIdentity) -> Result<Arc<dyn LyricsSongBinder>, ResolverError>;
}
pub trait LyricsSongBinder: Send + Sync {
    /// Bind only actual confirmed metadata. The returned provider must honor the
    /// Session, cancellation, response budgets and the explicitly disclosed service.
    fn bind(&self, resolution: &Resolution)
    -> Result<Arc<dyn ExistingTextProvider>, ResolverError>;
    /// Read a Session-local metadata revision only. No synchronous I/O is allowed.
    /// A revision change allows a formerly missing or replaced binding to retry.
    fn revision(&self, _resolution: &Resolution) -> Option<u64> {
        None
    }
}
#[derive(Clone)]
pub struct LyricsConfiguration {
    disclosure: ContentDisclosure,
    factory: Arc<dyn LyricsServiceFactory>,
}
impl LyricsConfiguration {
    pub fn new(
        disclosure: ContentDisclosure,
        factory: Arc<dyn LyricsServiceFactory>,
    ) -> Result<Self, ResolverError> {
        disclosure.validate()?;
        Ok(Self {
            disclosure,
            factory,
        })
    }
    pub fn disclosure(&self) -> ContentDisclosure {
        self.disclosure
    }
}
struct Binding {
    song: Arc<SongCandidate>,
    revision: Option<u64>,
    provider_guard: Option<ResolverCancellation>,
    provider: Result<Arc<dyn ExistingTextProvider>, ResolverError>,
}
impl Binding {
    fn matches(&self, song: &SongCandidate, revision: Option<u64>) -> bool {
        self.song.id == song.id
            && self.song.title == song.title
            && self.song.artist == song.artist
            && self.revision == revision
            && self
                .provider_guard
                .as_ref()
                .is_none_or(|guard| !guard.is_cancelled())
    }
}
type TextRun = Pin<Box<dyn Future<Output = Result<(), ResolverError>> + Send>>;
pub struct LiveLyrics {
    identity: StreamIdentity,
    session: Option<ExistingTextSession>,
    run: Option<TextRun>,
    binder: Option<Arc<dyn LyricsSongBinder>>,
    binding: Option<Binding>,
    previous: Option<WindowStamp>,
    status: UiLyricsStatus,
    ready: Option<ExistingTextLease>,
}
impl LiveLyrics {
    pub fn new(identity: StreamIdentity, config: Option<&LyricsConfiguration>) -> Self {
        let mut owner = Self {
            identity,
            session: None,
            run: None,
            binder: None,
            binding: None,
            previous: None,
            status: UiLyricsStatus::default(),
            ready: None,
        };
        if let Some(config) = config {
            owner.status.disclosure = Some(config.disclosure);
            let result =
                std::panic::catch_unwind(AssertUnwindSafe(|| config.factory.create(identity)))
                    .unwrap_or(Err(ResolverError::WorkerPanicked));
            match result {
                Ok(binder) => {
                    let (session, task) = ExistingTextSession::new(identity);
                    owner.session = Some(session);
                    owner.run = Some(Box::pin(task.run()));
                    owner.binder = Some(binder);
                    owner.status.state = UiLyricsState::Waiting;
                }
                Err(error) => owner.status.state = UiLyricsState::Failed(error),
            }
        }
        owner
    }
    pub fn status(&self) -> UiLyricsStatus {
        self.status
    }
    pub fn ready_text(&self) -> Option<ExistingTextLease> {
        self.ready
            .as_ref()
            .filter(|ready| ready.validate(self.identity).is_ok())
            .cloned()
    }
    fn retire_text(&mut self) {
        if let Some(ready) = self.ready.take() {
            ready.retire();
        }
    }
    fn set_state(&mut self, state: UiLyricsState) -> bool {
        let changed = self.status.state != state;
        self.status.state = state;
        changed
    }
    /// No invalid/foreign/out-of-order stamp may clear the current valid selection.
    /// Feed only settled Ready evidence, never an interim Resolving notification.
    pub fn select(
        &mut self,
        stamp: WindowStamp,
        resolution: &Resolution,
    ) -> Result<bool, ResolverError> {
        if self.status.disclosure.is_none() {
            return Ok(false);
        }
        stamp.validate(self.identity, Instant::now())?;
        if self
            .previous
            .is_some_and(|old| stamp.sequence <= old.sequence || stamp.ended_at < old.ended_at)
        {
            return Err(ResolverError::Stale);
        }
        if self.session.is_none() {
            return Ok(false);
        }
        self.previous = Some(stamp);
        if resolution.kind != ContentType::Music || resolution.song_state != SongState::Confirmed {
            self.retire_text();
            let result = self
                .session
                .as_mut()
                .ok_or(ResolverError::Cancelled)?
                .select_confirmed_lyrics(stamp, resolution, None);
            self.binding = None;
            return match result {
                Ok(_) => Ok(self.set_state(UiLyricsState::Waiting)),
                Err(error) => Ok(self.set_state(UiLyricsState::Failed(error))),
            };
        }
        let Some(song) = resolution.song.as_ref() else {
            return Ok(self.clear(Some(ResolverError::InvalidData)));
        };
        if song.confidence.permille() < SONG_CONFIDENCE
            || !metadata(&song.id)
            || !metadata(&song.title)
            || song
                .artist
                .as_deref()
                .is_some_and(|artist| !metadata(artist))
        {
            return Ok(self.clear(Some(ResolverError::InvalidData)));
        }
        let revision = match self.binder.as_ref() {
            Some(binder) => {
                std::panic::catch_unwind(AssertUnwindSafe(|| binder.revision(resolution)))
            }
            None => return Ok(self.clear(Some(ResolverError::NotConfigured))),
        };
        let Ok(revision) = revision else {
            return Ok(self.clear(Some(ResolverError::WorkerPanicked)));
        };
        if !self
            .binding
            .as_ref()
            .is_some_and(|binding| binding.matches(song, revision))
        {
            self.clear(None); // Cancel and remove old text before constructing a new binding.
            let provider = match self.binder.as_ref() {
                Some(binder) => {
                    std::panic::catch_unwind(AssertUnwindSafe(|| binder.bind(resolution)))
                        .unwrap_or(Err(ResolverError::WorkerPanicked))
                }
                None => Err(ResolverError::NotConfigured),
            };
            let (provider, provider_guard) = match provider {
                Ok(provider) => match std::panic::catch_unwind(AssertUnwindSafe(|| {
                    provider.publication_guard()
                })) {
                    Ok(guard) => (Ok(provider), guard),
                    Err(_) => (Err(ResolverError::WorkerPanicked), None),
                },
                Err(error) => (Err(error), None),
            };
            self.binding = Some(Binding {
                song: song.clone(),
                revision,
                provider_guard,
                provider,
            });
        }
        if let Some(binding) = &mut self.binding {
            binding.song = song.clone();
        }
        let provider = match self.binding.as_ref().map(|binding| &binding.provider) {
            Some(Ok(provider)) => provider.clone(),
            Some(Err(error)) => return Ok(self.set_state(UiLyricsState::Failed(*error))),
            None => return Ok(self.clear(Some(ResolverError::NotConfigured))),
        };
        let session = self.session.as_mut().ok_or(ResolverError::Cancelled)?;
        let selected = std::panic::catch_unwind(AssertUnwindSafe(|| {
            session.select_confirmed_lyrics(stamp, resolution, Some(provider))
        }))
        .unwrap_or(Err(ResolverError::WorkerPanicked));
        match selected {
            Ok(true) => {
                self.retire_text();
                Ok(self.set_state(UiLyricsState::LookingUp))
            }
            Ok(false) => {
                if let Some(ready) = &mut self.ready
                    && let Err(error) = ready.renew(stamp, song.confidence)
                {
                    return Ok(self.clear(Some(error)));
                }
                Ok(false) // Explicit confirmed renewal only; no refetch or implicit retry.
            }
            Err(error) => Ok(self.clear(Some(error))),
        }
    }
    /// Source loss, weak/unknown/mixed content, freshness expiry or resolver failure.
    /// Active requests are signalled synchronously; changed/Drop releases their futures.
    pub fn clear(&mut self, error: Option<ResolverError>) -> bool {
        self.retire_text();
        self.binding = None;
        let Some(session) = self.session.as_mut() else {
            return false;
        };
        let result = session.clear();
        let state = match (result, error) {
            (Err(error), _) | (_, Some(error)) => UiLyricsState::Failed(error),
            _ => UiLyricsState::Waiting,
        };
        self.set_state(state)
    }
    pub(crate) fn fail(&mut self, error: ResolverError) -> UiLyricsState {
        self.stop();
        if self.status.disclosure.is_some() {
            self.status.state = UiLyricsState::Failed(error);
        }
        self.status.state
    }
    /// Caller-polled: one lookup task, no detached task or text history. The full
    /// candidate remains only in ExistingTextSession; only scalar status leaves here.
    pub async fn changed(&mut self) -> UiLyricsState {
        loop {
            let (Some(session), Some(run)) = (&mut self.session, &mut self.run) else {
                return std::future::pending().await;
            };
            enum Event {
                Run(Result<(), ResolverError>),
                Text(Option<Arc<crate::resolver::text_session::TextLookupUpdate>>),
            }
            let event = tokio::select! { biased;
                result = AssertUnwindSafe(run.as_mut()).catch_unwind() => Event::Run(result.unwrap_or(Err(ResolverError::WorkerPanicked))),
                update = session.next() => Event::Text(update),
            };
            let state = match event {
                Event::Run(result) => {
                    return self.fail(result.err().unwrap_or(ResolverError::Cancelled));
                }
                Event::Text(None) => return self.fail(ResolverError::Cancelled),
                Event::Text(Some(update)) => {
                    if update.identity != self.identity {
                        return self.fail(ResolverError::InvalidData);
                    }
                    match update.status {
                        TextLookupStatus::LookingUp => {
                            self.retire_text();
                            UiLyricsState::LookingUp
                        }
                        TextLookupStatus::Available => {
                            self.retire_text();
                            match ExistingTextLease::from_update(&update) {
                                Ok(mut ready) => {
                                    let renewed = self
                                        .previous
                                        .zip(self.binding.as_ref())
                                        .ok_or(ResolverError::InvalidData)
                                        .and_then(|(stamp, binding)| {
                                            ready.renew(stamp, binding.song.confidence)
                                        });
                                    match renewed {
                                        Ok(()) => {
                                            self.ready = Some(ready);
                                            UiLyricsState::Available
                                        }
                                        Err(error) => UiLyricsState::Failed(error),
                                    }
                                }
                                Err(error) => UiLyricsState::Failed(error),
                            }
                        }
                        TextLookupStatus::Unavailable => {
                            self.retire_text();
                            UiLyricsState::Unavailable
                        }
                        TextLookupStatus::Cleared => {
                            self.retire_text();
                            // A pending selection may have generated Cleared before
                            // its LookingUp. Do not erase a binder failure or query state.
                            if matches!(
                                self.status.state,
                                UiLyricsState::LookingUp | UiLyricsState::Failed(_)
                            ) {
                                continue;
                            }
                            UiLyricsState::Waiting
                        }
                        TextLookupStatus::Failed(error) => {
                            self.retire_text();
                            UiLyricsState::Failed(error)
                        }
                        TextLookupStatus::Stopped => {
                            self.retire_text();
                            UiLyricsState::Stopped
                        }
                    }
                }
            };
            if self.set_state(state) {
                return state;
            }
        }
    }
    pub fn stop(&mut self) {
        self.retire_text();
        if let Some(session) = &mut self.session {
            let _ = session.stop();
        }
        self.run = None;
        self.session = None;
        self.binder = None;
        self.binding = None;
        self.previous = None;
        self.status.stop();
    }
}
impl Drop for LiveLyrics {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
#[path = "lyrics_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "lyrics_metadata_tests.rs"]
mod metadata_tests;
