//! An explicitly named source-language subtitle, shared only for its Session.
//! The catalog compares downloaded cues; it never echoes ASR or searches media.
use super::{
    contracts::*,
    movie::{
        MAX_DIALOGUE_BYTES, MIN_DIALOGUE_CHARACTERS, MOVIE_LOOKUP_TIMEOUT, MovieCatalog,
        MovieCatalogCapabilities, MovieCatalogFuture, MovieCatalogReply, MovieQuery, MovieRecord,
    },
    text_session::validate_bundle_budget,
    timed_subtitle::{SubtitleTrack, TimedSubtitleProvider},
};
use crate::ai::speech::{SpeechLanguage, StreamIdentity};
use futures_util::FutureExt;
use std::{
    panic::AssertUnwindSafe,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

const MAX_CPU_JOBS: usize = 2;

/// The title and track are explicit user input, not an inferred release binding.
/// Signed locations and private names intentionally have no Debug/Serialize.
#[derive(Clone)]
pub struct KnownSubtitleSource {
    title: Box<str>,
    track: SubtitleTrack,
    language: SpeechLanguage,
}
impl KnownSubtitleSource {
    pub fn new(title: Box<str>, track: SubtitleTrack) -> Result<Self, ResolverError> {
        if !metadata(&title) {
            return Err(ResolverError::InvalidData);
        }
        let TextLanguage::Source(language) = track.language() else {
            return Err(ResolverError::InvalidData);
        };
        Ok(Self {
            title,
            track,
            language,
        })
    }
    pub fn start(&self, identity: StreamIdentity) -> Result<SessionKnownSubtitle, ResolverError> {
        SessionKnownSubtitle::new(identity, self.title.clone(), self.track.clone())
    }
    pub fn title(&self) -> &str {
        &self.title
    }
    pub fn language(&self) -> SpeechLanguage {
        self.language
    }
}

/// A single owner retires both catalog evidence and text publications on Drop.
/// Keeping catalog/provider readers alive cannot extend the Session lifetime.
pub struct SessionKnownSubtitle {
    inner: Arc<Shared>,
}
impl SessionKnownSubtitle {
    pub fn new(
        identity: StreamIdentity,
        title: Box<str>,
        track: SubtitleTrack,
    ) -> Result<Self, ResolverError> {
        if !metadata(&title) {
            return Err(ResolverError::InvalidData);
        }
        let TextLanguage::Source(language) = track.language() else {
            return Err(ResolverError::InvalidData);
        };
        let content_id = content_id(identity)?;
        let provider = Arc::new(TimedSubtitleProvider::new(
            identity,
            content_id.clone(),
            track,
            None,
        )?);
        Ok(Self::build(
            identity,
            title,
            language,
            content_id,
            provider,
            workers(),
        ))
    }
    fn build(
        identity: StreamIdentity,
        title: Box<str>,
        language: SpeechLanguage,
        content_id: Box<str>,
        provider: Arc<dyn ExistingTextProvider>,
        workers: Arc<Semaphore>,
    ) -> Self {
        Self {
            inner: Arc::new(Shared {
                identity,
                title,
                language,
                content_id,
                provider,
                document: Mutex::new(None),
                availability: ResolverCancellation::default(),
                requests: Arc::new(Semaphore::new(1)),
                workers,
            }),
        }
    }
    pub fn catalog(&self) -> Arc<dyn MovieCatalog> {
        self.inner.clone()
    }
    pub fn provider(&self) -> Arc<dyn ExistingTextProvider> {
        self.inner.clone()
    }
    pub fn content_id(&self) -> &str {
        &self.inner.content_id
    }
    pub fn title(&self) -> &str {
        &self.inner.title
    }
    pub fn language(&self) -> SpeechLanguage {
        self.inner.language
    }
    pub fn stop(&self) {
        self.inner.availability.cancel();
        self.inner
            .document
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
    }
}
impl Drop for SessionKnownSubtitle {
    fn drop(&mut self) {
        self.stop();
    }
}

fn content_id(identity: StreamIdentity) -> Result<Box<str>, ResolverError> {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let serial = NEXT
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            value.checked_add(1)
        })
        .map_err(|_| ResolverError::InvalidData)?;
    Ok(format!(
        "known-subtitle:{}:{}:{serial}",
        identity.session.0, identity.generation
    )
    .into())
}
fn workers() -> Arc<Semaphore> {
    static WORKERS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    WORKERS
        .get_or_init(|| {
            let cores = std::thread::available_parallelism().map_or(1, usize::from);
            Arc::new(Semaphore::new(cores.min(MAX_CPU_JOBS)))
        })
        .clone()
}

struct Shared {
    identity: StreamIdentity,
    title: Box<str>,
    language: SpeechLanguage,
    content_id: Box<str>,
    provider: Arc<dyn ExistingTextProvider>,
    document: Mutex<Option<Arc<ExistingTextBundle>>>,
    availability: ResolverCancellation,
    requests: Arc<Semaphore>,
    workers: Arc<Semaphore>,
}
struct Check {
    query: TextQuery,
    request: ResolverCancellation,
    external: ResolverCancellation,
    owner: ResolverCancellation,
    deadline: Instant,
    query_deadline: Instant,
}
impl Check {
    fn run(&self) -> Result<(), ResolverError> {
        if self.request.is_cancelled() || self.external.is_cancelled() || self.owner.is_cancelled()
        {
            return Err(ResolverError::Cancelled);
        }
        self.query.validate(Instant::now())?;
        if Instant::now() >= self.deadline {
            return Err(if Instant::now() >= self.query_deadline {
                ResolverError::Stale
            } else {
                ResolverError::Timeout
            });
        }
        Ok(())
    }
}
struct CancelOnDrop(ResolverCancellation);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}
struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);
impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}
struct CpuResult<T> {
    document: Arc<ExistingTextBundle>,
    value: T,
    // A started CPU job retains both permits after future Drop. Returning this
    // permit also prevents a second fetch before the first cache publication.
    _request: OwnedSemaphorePermit,
}
impl Shared {
    fn validate(&self, query: &TextQuery) -> Result<(), ResolverError> {
        query.stamp.validate(self.identity, Instant::now())?;
        query.validate(Instant::now())?;
        if query.source != SubtitleSource::ExistingSubtitle {
            return Err(ResolverError::UnsupportedCapability);
        }
        if query.content_id != self.content_id {
            return Err(ResolverError::ContentMismatch);
        }
        Ok(())
    }
    async fn run<T, F>(
        &self,
        query: &TextQuery,
        cancel: ResolverCancellation,
        operation: F,
    ) -> Result<Option<T>, ResolverError>
    where
        T: Send + 'static,
        F: FnOnce(&ExistingTextBundle, &Check) -> Result<T, ResolverError> + Send + 'static,
    {
        if self.availability.is_cancelled() || cancel.is_cancelled() {
            return Err(ResolverError::Cancelled);
        }
        self.validate(query)?;
        let permit = self
            .requests
            .clone()
            .try_acquire_owned()
            .map_err(|_| ResolverError::TextOverloaded)?;
        let query_deadline = query
            .stamp
            .ended_at
            .checked_add(MAX_WINDOW_AGE)
            .ok_or(ResolverError::Stale)?;
        let deadline = Instant::now()
            .checked_add(MOVIE_LOOKUP_TIMEOUT)
            .ok_or(ResolverError::Timeout)?
            .min(query_deadline);
        let request = ResolverCancellation::default();
        let _guard = CancelOnDrop(request.clone());
        let check = Check {
            query: TextQuery {
                stamp: query.stamp,
                source: query.source,
                content_id: query.content_id.clone(),
            },
            request: request.clone(),
            external: cancel.clone(),
            owner: self.availability.clone(),
            deadline,
            query_deadline,
        };
        let cached = self
            .document
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        let work = async {
            let document = match cached {
                Some(document) => document,
                None => {
                    // Keep trait calls inside catch_unwind too: a failing adapter
                    // must not poison the cache or strand the admission permit.
                    let fetched = AssertUnwindSafe(async {
                        self.provider.lookup_bundle(query, request.clone()).await
                    })
                    .catch_unwind()
                    .await
                    .map_err(|_| ResolverError::WorkerPanicked)??;
                    let Some(document) = fetched else {
                        return Ok(None);
                    };
                    Arc::new(document)
                }
            };
            let cpu = self
                .workers
                .clone()
                .try_acquire_owned()
                .map_err(|_| ResolverError::TextOverloaded)?;
            let language = self.language;
            let mut worker = AbortOnDrop(tokio::task::spawn_blocking(move || {
                let _cpu = cpu;
                check.run()?;
                validate_bundle_budget(&document.text, document.source_reference.as_ref())?;
                document.validate(&check.query)?;
                // This first entry accepts only one declared source track.
                if document.text.language != TextLanguage::Source(language)
                    || document.source_reference.is_some()
                {
                    return Err(ResolverError::ContentMismatch);
                }
                check.run()?;
                let value = operation(&document, &check)?;
                check.run()?;
                Ok::<_, ResolverError>(Some(CpuResult {
                    document,
                    value,
                    _request: permit,
                }))
            }));
            (&mut worker.0)
                .await
                .map_err(|_| ResolverError::WorkerPanicked)?
        };
        let result = tokio::select! { biased;
            _ = self.availability.cancelled() => return Err(ResolverError::Cancelled),
            _ = cancel.cancelled() => return Err(ResolverError::Cancelled),
            _ = tokio::time::sleep_until(deadline.into()) => return Err(if Instant::now() >= query_deadline { ResolverError::Stale } else { ResolverError::Timeout }),
            result = work => result,
        }?;
        if self.availability.is_cancelled() || cancel.is_cancelled() {
            return Err(ResolverError::Cancelled);
        }
        self.validate(query)?;
        let Some(result) = result else {
            return Ok(None);
        };
        {
            let mut cache = self
                .document
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if self.availability.is_cancelled() || cancel.is_cancelled() {
                return Err(ResolverError::Cancelled);
            }
            *cache = Some(result.document);
        }
        Ok(Some(result.value))
    }
}
impl ExistingTextProvider for Shared {
    fn supports(&self, source: SubtitleSource) -> bool {
        source == SubtitleSource::ExistingSubtitle
    }
    fn publication_guard(&self) -> Option<ResolverCancellation> {
        Some(self.availability.clone())
    }
    fn lookup<'a>(&'a self, query: &'a TextQuery, cancel: ResolverCancellation) -> TextFuture<'a> {
        Box::pin(async move {
            Ok(self
                .lookup_bundle(query, cancel)
                .await?
                .map(|bundle| bundle.text))
        })
    }
    fn lookup_bundle<'a>(
        &'a self,
        query: &'a TextQuery,
        cancel: ResolverCancellation,
    ) -> TextBundleFuture<'a> {
        Box::pin(self.run(query, cancel, clone_bundle))
    }
}
impl MovieCatalog for Shared {
    fn capabilities(&self) -> MovieCatalogCapabilities {
        MovieCatalogCapabilities {
            cancellation: true,
            dialogue_references: true,
        }
    }
    fn lookup<'a>(
        &'a self,
        query: &'a MovieQuery,
        cancel: ResolverCancellation,
    ) -> MovieCatalogFuture<'a> {
        Box::pin(async move {
            if self.availability.is_cancelled() || cancel.is_cancelled() {
                return Err(ResolverError::Cancelled);
            }
            query.validate(self.identity, Instant::now())?;
            let Some(dialogue) = query.dialogue.as_deref() else {
                return Ok(MovieCatalogReply {
                    stamp: query.stamp,
                    records: Vec::new(),
                });
            };
            if query.language != Some(self.language)
                || !query
                    .window_title
                    .as_deref()
                    .into_iter()
                    .chain(query.filename.as_deref())
                    .any(|hint| title_matches(hint, &self.title))
                || dialogue.chars().filter(|ch| ch.is_alphanumeric()).count()
                    < MIN_DIALOGUE_CHARACTERS
            {
                return Ok(MovieCatalogReply {
                    stamp: query.stamp,
                    records: Vec::new(),
                });
            }
            let dialogue = dialogue.to_owned();
            let title = self.title.clone();
            let language = self.language;
            let text_query = TextQuery {
                stamp: query.stamp,
                source: SubtitleSource::ExistingSubtitle,
                content_id: self.content_id.clone(),
            };
            let records = self
                .run(&text_query, cancel, move |document, check| {
                    let wanted = normalize(&dialogue);
                    let mut matched = None;
                    for cue in &document.text.cues {
                        check.run()?;
                        if normalize(&cue.text) != wanted {
                            continue;
                        }
                        if matched.is_some() {
                            return Ok(Vec::new());
                        }
                        matched = Some(cue);
                    }
                    let Some(cue) = matched else {
                        return Ok(Vec::new());
                    };
                    // An oversized raw cue still participates in uniqueness,
                    // but cannot become a bounded MovieRecord reference.
                    if cue.text.len() > MAX_DIALOGUE_BYTES {
                        return Ok(Vec::new());
                    }
                    // Complete normalized-token equality measures 100% agreement.
                    // This is not a match probability or a catalog search score.
                    let denominator = wanted.chars().filter(|ch| ch.is_alphanumeric()).count();
                    let numerator = normalize(&cue.text)
                        .chars()
                        .filter(|ch| ch.is_alphanumeric())
                        .count();
                    let agreement = numerator
                        .checked_mul(1000)
                        .and_then(|value| value.checked_div(denominator))
                        .ok_or(ResolverError::InvalidData)?;
                    let confidence = Confidence::new(
                        u16::try_from(agreement).map_err(|_| ResolverError::InvalidData)?,
                    )?;
                    Ok(vec![MovieRecord {
                        id: document.text.content_id.clone(),
                        title,
                        aliases: Vec::new(),
                        year: None,
                        language: Some(language),
                        duration_us: None,
                        confidence,
                        dialogue_references: vec![cue.text.clone()],
                    }])
                })
                .await?
                .unwrap_or_default();
            query.validate(self.identity, Instant::now())?;
            Ok(MovieCatalogReply {
                stamp: query.stamp,
                records,
            })
        })
    }
}
fn normalize(value: &str) -> String {
    value
        .chars()
        .flat_map(char::to_lowercase)
        .map(|ch| if ch.is_alphanumeric() { ch } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
fn title_matches(hint: &str, title: &str) -> bool {
    let hint = normalize(hint);
    let title = normalize(title);
    !title.is_empty()
        && hint.match_indices(&title).any(|(start, _)| {
            (start == 0 || hint.as_bytes()[start - 1] == b' ')
                && (start + title.len() == hint.len()
                    || hint.as_bytes()[start + title.len()] == b' ')
        })
}
fn clone_bundle(
    document: &ExistingTextBundle,
    check: &Check,
) -> Result<ExistingTextBundle, ResolverError> {
    let mut cues = Vec::with_capacity(document.text.cues.len());
    for cue in &document.text.cues {
        check.run()?;
        cues.push(TextCue {
            start_us: cue.start_us,
            duration_us: cue.duration_us,
            text: cue.text.clone(),
        });
    }
    Ok(ExistingTextBundle {
        text: ExistingText {
            identity: document.text.identity,
            source: document.text.source,
            content_id: document.text.content_id.clone(),
            language: document.text.language,
            cues,
        },
        source_reference: None,
    })
}

#[cfg(test)]
#[path = "known_subtitle_tests.rs"]
mod tests;
