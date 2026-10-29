//! Session-only verified-movie subtitle lookup composition. This owner never
//! localizes, aligns, adopts or renders text, and selects no default service.
use crate::{
    ai::speech::StreamIdentity,
    resolver::{
        candidates::{AMBIGUITY_MARGIN, Resolution},
        contracts::*,
        text_session::{ExistingTextLease, ExistingTextSession, TextLookupStatus},
    },
    subtitle::source_arbiter::MIN_CONTENT_CONFIDENCE,
    ui::{
        content::ContentDisclosure,
        subtitles::{UiSubtitleState, UiSubtitleStatus},
    },
};
use futures_util::FutureExt;
use std::{future::Future, panic::AssertUnwindSafe, pin::Pin, sync::Arc, time::Instant};

/// Create a fresh Session binder. Neither create nor bind may perform synchronous
/// network, credential lookup or long CPU work.
pub trait SubtitleServiceFactory: Send + Sync {
    fn create(
        &self,
        identity: StreamIdentity,
    ) -> Result<Arc<dyn SubtitleMovieBinder>, ResolverError>;
}
pub trait SubtitleMovieBinder: Send + Sync {
    /// Version of the currently available Session metadata, without synchronous
    /// I/O. An unchanged version retains the existing provider and publication.
    fn revision(&self, _movie: &MovieCandidate) -> Option<u64> {
        None
    }
    /// Bind this selected movie only, using actual, explicitly configured document
    /// language and release metadata. URLs, ASR preferences and signal labels are
    /// not evidence of a subtitle document's language or edition.
    fn bind(&self, movie: &MovieCandidate) -> Result<Arc<dyn ExistingTextProvider>, ResolverError>;
}
#[derive(Clone)]
pub struct SubtitleConfiguration {
    disclosure: ContentDisclosure,
    factory: Arc<dyn SubtitleServiceFactory>,
}
impl SubtitleConfiguration {
    pub fn new(
        disclosure: ContentDisclosure,
        factory: Arc<dyn SubtitleServiceFactory>,
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
    movie: Arc<MovieCandidate>,
    revision: Option<u64>,
    availability: Option<ResolverCancellation>,
    provider: Result<Arc<dyn ExistingTextProvider>, ResolverError>,
}
impl Binding {
    fn matches(&self, movie: &MovieCandidate, revision: Option<u64>) -> bool {
        self.movie.id == movie.id
            && self.movie.title == movie.title
            && self.movie.year == movie.year
            && self.revision == revision
            && !self
                .availability
                .as_ref()
                .is_some_and(ResolverCancellation::is_cancelled)
    }
}

/// Signals are required in addition to the provider's unchanged confidence. They
/// permit lookup only; exact ASR/cue alignment still gates subtitle adoption.
fn selected_movie(resolution: &Resolution) -> Result<Option<Arc<MovieCandidate>>, ResolverError> {
    if resolution.movies.len() > MAX_CANDIDATES
        || (!resolution.movies.is_empty()
            && !matches!(
                resolution.kind,
                ContentType::MovieOrShow | ContentType::Mixed
            ))
    {
        return Err(ResolverError::InvalidData);
    }
    for (index, movie) in resolution.movies.iter().enumerate() {
        if !metadata(&movie.id)
            || !metadata(&movie.title)
            || movie.year.is_some_and(|year| !(1..=9999).contains(&year))
            || movie.signals.is_empty()
            || movie.signals.len() > MAX_MOVIE_SIGNALS
            || movie
                .signals
                .iter()
                .enumerate()
                .any(|(index, signal)| movie.signals[..index].contains(signal))
            || resolution.movies[..index]
                .iter()
                .any(|previous| previous.id == movie.id)
        {
            return Err(ResolverError::InvalidData);
        }
    }
    if resolution.kind != ContentType::MovieOrShow {
        return Ok(None);
    }
    let Some(top) = resolution
        .movies
        .iter()
        .max_by_key(|movie| movie.confidence)
    else {
        return Ok(None);
    };
    if top.confidence.permille() < MIN_CONTENT_CONFIDENCE
        || !top.signals.contains(&MovieSignal::Dialogue)
        || !top.signals.contains(&MovieSignal::RemoteMetadata)
        || !(top.signals.contains(&MovieSignal::WindowTitle)
            || top.signals.contains(&MovieSignal::Filename))
        || resolution.movies.iter().any(|other| {
            other.id != top.id
                && top
                    .confidence
                    .permille()
                    .saturating_sub(other.confidence.permille())
                    < AMBIGUITY_MARGIN
        })
    {
        return Ok(None);
    }
    Ok(Some(top.clone()))
}

type TextRun = Pin<Box<dyn Future<Output = Result<(), ResolverError>> + Send>>;
pub struct LiveSubtitles {
    identity: StreamIdentity,
    session: Option<ExistingTextSession>,
    run: Option<TextRun>,
    binder: Option<Arc<dyn SubtitleMovieBinder>>,
    binding: Option<Binding>,
    previous: Option<WindowStamp>,
    status: UiSubtitleStatus,
    ready: Option<ExistingTextLease>,
}
impl LiveSubtitles {
    pub fn new(identity: StreamIdentity, config: Option<&SubtitleConfiguration>) -> Self {
        let mut owner = Self {
            identity,
            session: None,
            run: None,
            binder: None,
            binding: None,
            previous: None,
            status: UiSubtitleStatus::default(),
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
                    owner.status.state = UiSubtitleState::Waiting;
                }
                Err(error) => owner.status.state = UiSubtitleState::Failed(error),
            }
        }
        owner
    }
    pub fn status(&self) -> UiSubtitleStatus {
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
    fn set_state(&mut self, state: UiSubtitleState) -> bool {
        let changed = self.status.state != state;
        self.status.state = state;
        changed
    }
    /// Feed settled Ready results only. Foreign, old or out-of-order stamps cannot
    /// clear a valid selection. Unknown, weak or ambiguous content retires it now.
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
        let movie = match selected_movie(resolution) {
            Ok(Some(movie)) => movie,
            Ok(None) => return Ok(self.clear(None)),
            Err(error) => return Ok(self.clear(Some(error))),
        };
        let revision = match self.binder.as_ref() {
            Some(binder) => {
                match std::panic::catch_unwind(AssertUnwindSafe(|| binder.revision(&movie))) {
                    Ok(revision) => revision,
                    Err(_) => return Ok(self.clear(Some(ResolverError::WorkerPanicked))),
                }
            }
            None => return Ok(self.clear(Some(ResolverError::NotConfigured))),
        };
        if !self
            .binding
            .as_ref()
            .is_some_and(|binding| binding.matches(&movie, revision))
        {
            self.clear(None); // Retire the previous publication before binding a new movie.
            let provider = match self.binder.as_ref() {
                Some(binder) => std::panic::catch_unwind(AssertUnwindSafe(|| binder.bind(&movie)))
                    .unwrap_or(Err(ResolverError::WorkerPanicked)),
                None => Err(ResolverError::NotConfigured),
            };
            let (provider, availability) = match provider {
                Ok(provider) => match std::panic::catch_unwind(AssertUnwindSafe(|| {
                    provider.publication_guard()
                })) {
                    Ok(availability) => {
                        let result = if availability
                            .as_ref()
                            .is_some_and(ResolverCancellation::is_cancelled)
                        {
                            Err(ResolverError::Cancelled)
                        } else {
                            Ok(provider)
                        };
                        (result, availability)
                    }
                    Err(_) => (Err(ResolverError::WorkerPanicked), None),
                },
                Err(error) => (Err(error), None),
            };
            self.binding = Some(Binding {
                movie: movie.clone(),
                revision,
                availability,
                provider,
            });
        }
        if let Some(binding) = &mut self.binding {
            binding.movie = movie.clone();
        }
        let provider = match self.binding.as_ref().map(|binding| &binding.provider) {
            Some(Ok(provider)) => provider.clone(),
            Some(Err(error)) => return Ok(self.set_state(UiSubtitleState::Failed(*error))),
            None => return Ok(self.clear(Some(ResolverError::NotConfigured))),
        };
        let session = self.session.as_mut().ok_or(ResolverError::Cancelled)?;
        let selected = std::panic::catch_unwind(AssertUnwindSafe(|| {
            session.select(
                TextQuery {
                    stamp,
                    source: SubtitleSource::ExistingSubtitle,
                    content_id: movie.id.clone(),
                },
                provider,
            )
        }))
        .unwrap_or(Err(ResolverError::WorkerPanicked));
        match selected {
            Ok(true) => {
                self.retire_text();
                Ok(self.set_state(UiSubtitleState::LookingUp))
            }
            Ok(false) => {
                if let Some(ready) = &mut self.ready
                    && let Err(error) = ready.renew(stamp, movie.confidence)
                {
                    return Ok(self.clear(Some(error)));
                }
                Ok(false) // Explicit renewal only; no refetch or implicit retry.
            }
            Err(error) => {
                self.retire_text();
                if let Some(binding) = &mut self.binding {
                    binding.provider = Err(error);
                }
                let cleared = self
                    .session
                    .as_mut()
                    .ok_or(ResolverError::Cancelled)?
                    .clear();
                Ok(self.set_state(UiSubtitleState::Failed(cleared.err().unwrap_or(error))))
            }
        }
    }
    /// Retire the publication synchronously; outstanding requests are cancelled
    /// without waiting for the lookup task to be polled again.
    pub fn clear(&mut self, error: Option<ResolverError>) -> bool {
        self.retire_text();
        self.binding = None;
        let Some(session) = self.session.as_mut() else {
            return false;
        };
        let result = session.clear();
        let state = match (result, error) {
            (Err(error), _) | (_, Some(error)) => UiSubtitleState::Failed(error),
            _ => UiSubtitleState::Waiting,
        };
        self.set_state(state)
    }
    pub(crate) fn fail(&mut self, error: ResolverError) -> UiSubtitleState {
        self.stop();
        if self.status.disclosure.is_some() {
            self.status.state = UiSubtitleState::Failed(error);
        }
        self.status.state
    }
    /// One caller-polled lookup task, one publication, no detached task or history.
    /// Only scalar status leaves this owner for the UI.
    pub async fn changed(&mut self) -> UiSubtitleState {
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
                            UiSubtitleState::LookingUp
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
                                            ready.renew(stamp, binding.movie.confidence)
                                        });
                                    match renewed {
                                        Ok(()) => {
                                            self.ready = Some(ready);
                                            UiSubtitleState::Available
                                        }
                                        Err(error) => UiSubtitleState::Failed(error),
                                    }
                                }
                                Err(error) => UiSubtitleState::Failed(error),
                            }
                        }
                        TextLookupStatus::Unavailable => {
                            self.retire_text();
                            UiSubtitleState::Unavailable
                        }
                        TextLookupStatus::Cleared => {
                            self.retire_text();
                            // Cleared can precede a selected lookup notification.
                            if matches!(
                                self.status.state,
                                UiSubtitleState::LookingUp | UiSubtitleState::Failed(_)
                            ) {
                                continue;
                            }
                            UiSubtitleState::Waiting
                        }
                        TextLookupStatus::Failed(error) => {
                            self.retire_text();
                            UiSubtitleState::Failed(error)
                        }
                        TextLookupStatus::Stopped => {
                            self.retire_text();
                            UiSubtitleState::Stopped
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
impl Drop for LiveSubtitles {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
#[path = "subtitles_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "subtitle_scope_tests.rs"]
mod scope_tests;
