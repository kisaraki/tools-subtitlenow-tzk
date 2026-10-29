//! Multi-signal metadata verification, never subtitle adoption. No catalog is
//! selected by default; callers explicitly provide a cancellable catalog and
//! current-session hints. Neither PCM nor a universal fingerprint is involved.
use super::{candidates::CLASSIFICATION_CONFIDENCE, contracts::*};
use crate::ai::speech::{SpeechLanguage, StreamIdentity};
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;

pub const MAX_MOVIE_ALIASES: usize = 4;
pub const MAX_DIALOGUE_REFERENCES: usize = 4;
pub const MAX_DIALOGUE_BYTES: usize = 4096;
/// A deliberately conservative exact-phrase gate, not a recognition probability.
pub const MIN_DIALOGUE_CHARACTERS: usize = 24;
pub const MIN_MEDIA_DURATION_US: u64 = 1_000_000;
pub const MAX_MEDIA_DURATION_US: u64 = 24 * 60 * 60 * 1_000_000;
/// Known release runtimes must agree closely; other cuts remain unverified.
pub const MEDIA_DURATION_TOLERANCE_US: u64 = 5 * 1_000_000;
pub const MOVIE_LOOKUP_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone)]
pub struct MovieQuery {
    pub stamp: WindowStamp,
    pub window_title: Option<Box<str>>,
    /// Basename only. Directory paths and URLs must not enter catalog requests.
    pub filename: Option<Box<str>>,
    /// Caller supplies current-window ASR, not session history or an older run.
    pub dialogue: Option<Box<str>>,
    pub language: Option<SpeechLanguage>,
    pub year: Option<u16>,
    /// Actual full media runtime, never the twelve-second capture window.
    pub duration_us: Option<u64>,
}
impl MovieQuery {
    pub fn validate(&self, identity: StreamIdentity, now: Instant) -> Result<(), ResolverError> {
        self.stamp.validate(identity, now)?;
        if self.window_title.as_deref().is_some_and(|v| !metadata(v))
            || self.filename.as_deref().is_some_and(|v| {
                !metadata(v) || v.contains(['/', '\\', ':']) || matches!(v, "." | "..")
            })
            || self.dialogue.as_deref().is_some_and(|v| !dialogue_text(v))
            || self.year.is_some_and(|v| !(1..=9999).contains(&v))
            || self.duration_us.is_some_and(|v| !runtime(v))
        {
            return Err(ResolverError::InvalidData);
        }
        Ok(())
    }
    fn eligible(&self) -> bool {
        (self.window_title.is_some() || self.filename.is_some())
            && self.dialogue.as_deref().is_some_and(dialogue_eligible)
    }
}
pub(crate) fn dialogue_eligible(value: &str) -> bool {
    normalize(value)
        .chars()
        .filter(|c| c.is_alphanumeric())
        .count()
        >= MIN_DIALOGUE_CHARACTERS
}

/// Confidence and references must be actual catalog evidence. Popularity/search
/// position must not be represented as identification confidence by an adapter.
#[derive(Clone)]
pub struct MovieRecord {
    pub id: Box<str>,
    pub title: Box<str>,
    pub aliases: Vec<Box<str>>,
    pub year: Option<u16>,
    pub language: Option<SpeechLanguage>,
    pub duration_us: Option<u64>,
    pub confidence: Confidence,
    /// Bounded excerpts from the candidate itself, not echoed query dialogue.
    pub dialogue_references: Vec<Box<str>>,
}
impl MovieRecord {
    fn validate(&self) -> Result<(), ResolverError> {
        if !metadata(&self.id)
            || !metadata(&self.title)
            || self.aliases.len() > MAX_MOVIE_ALIASES
            || self.aliases.iter().any(|v| !metadata(v))
            || self.year.is_some_and(|v| !(1..=9999).contains(&v))
            || self.duration_us.is_some_and(|v| !runtime(v))
            || self.dialogue_references.len() > MAX_DIALOGUE_REFERENCES
            || self.dialogue_references.iter().any(|v| !dialogue_text(v))
        {
            return Err(ResolverError::InvalidData);
        }
        Ok(())
    }
}
pub struct MovieCatalogReply {
    pub stamp: WindowStamp,
    pub records: Vec<MovieRecord>,
}
#[derive(Clone, Copy)]
pub struct MovieCatalogCapabilities {
    pub cancellation: bool,
    pub dialogue_references: bool,
}
impl MovieCatalogCapabilities {
    fn validate(self) -> Result<(), ResolverError> {
        if self.cancellation && self.dialogue_references {
            Ok(())
        } else {
            Err(ResolverError::UnsupportedCapability)
        }
    }
}
pub type MovieCatalogFuture<'a> =
    Pin<Box<dyn Future<Output = Result<MovieCatalogReply, ResolverError>> + Send + 'a>>;
pub trait MovieCatalog: Send + Sync {
    fn capabilities(&self) -> MovieCatalogCapabilities;
    /// Bound data before allocating/decoding it, use explicitly configured secure
    /// transport, retain it only in Session RAM, and honor cancellation and Drop.
    /// There is no automatic provider, cross-provider fallback or audio argument.
    fn lookup<'a>(
        &'a self,
        query: &'a MovieQuery,
        cancel: ResolverCancellation,
    ) -> MovieCatalogFuture<'a>;
}
struct RequestGuard(ResolverCancellation);
impl Drop for RequestGuard {
    fn drop(&mut self) {
        self.0.cancel();
    }
}
pub struct MovieMetadataResolver {
    identity: StreamIdentity,
    catalog: Arc<dyn MovieCatalog>,
    requests: Semaphore,
    previous: Mutex<Option<(u64, Instant)>>,
}
impl MovieMetadataResolver {
    pub fn new(
        identity: StreamIdentity,
        catalog: Arc<dyn MovieCatalog>,
    ) -> Result<Self, ResolverError> {
        catalog.capabilities().validate()?;
        Ok(Self {
            identity,
            catalog,
            requests: Semaphore::new(1),
            previous: Mutex::new(None),
        })
    }
    /// Owner polls this alongside Live STT. No worker is spawned/detached, no
    /// queued requests accumulate, and failure does not cancel STT or adopt text.
    pub async fn resolve(
        &self,
        query: &MovieQuery,
        cancel: ResolverCancellation,
    ) -> Result<ContentEvidence, ResolverError> {
        if cancel.is_cancelled() {
            return Err(ResolverError::Cancelled);
        }
        query.validate(self.identity, Instant::now())?;
        self.catalog.capabilities().validate()?;
        let _permit = self
            .requests
            .try_acquire()
            .map_err(|_| ResolverError::RateLimited)?;
        {
            let mut previous = self.previous.lock().unwrap_or_else(|e| e.into_inner());
            if previous.is_some_and(|(sequence, at)| {
                query.stamp.sequence <= sequence || query.stamp.ended_at < at
            }) {
                return Err(ResolverError::Stale);
            }
            *previous = Some((query.stamp.sequence, query.stamp.ended_at));
        }
        if !query.eligible() {
            if cancel.is_cancelled() {
                return Err(ResolverError::Cancelled);
            }
            query.validate(self.identity, Instant::now())?;
            return unknown();
        }
        let request_cancel = ResolverCancellation::default();
        let _request = RequestGuard(request_cancel.clone());
        let reply = tokio::select! { biased;
            _ = cancel.cancelled() => return Err(ResolverError::Cancelled),
            result = tokio::time::timeout(MOVIE_LOOKUP_TIMEOUT, self.catalog.lookup(query, request_cancel)) => result.unwrap_or(Err(ResolverError::Timeout))?,
        };
        if cancel.is_cancelled() {
            return Err(ResolverError::Cancelled);
        }
        query.validate(self.identity, Instant::now())?;
        let evidence = verify_reply(query, reply)?;
        if cancel.is_cancelled() {
            return Err(ResolverError::Cancelled);
        }
        query.validate(self.identity, Instant::now())?;
        Ok(evidence)
    }
}
fn verify_reply(
    query: &MovieQuery,
    reply: MovieCatalogReply,
) -> Result<ContentEvidence, ResolverError> {
    let stamp = reply.stamp;
    if stamp.identity != query.stamp.identity
        || stamp.sequence != query.stamp.sequence
        || stamp.ended_at != query.stamp.ended_at
        || stamp.start_us != query.stamp.start_us
        || stamp.duration_us != query.stamp.duration_us
        || reply.records.len() > MAX_CANDIDATES
    {
        return Err(ResolverError::InvalidData);
    }
    let mut movies = Vec::new();
    for (index, record) in reply.records.iter().enumerate() {
        record.validate()?;
        if reply.records[..index].iter().any(|v| v.id == record.id) {
            return Err(ResolverError::InvalidData);
        }
        if record.confidence.permille() < CLASSIFICATION_CONFIDENCE
            || query.year.is_some_and(|v| record.year != Some(v))
            || query.language.is_some_and(|v| record.language != Some(v))
            || query.duration_us.is_some_and(|v| {
                record
                    .duration_us
                    .is_none_or(|r| r.abs_diff(v) > MEDIA_DURATION_TOLERANCE_US)
            })
        {
            continue;
        }
        let matches_title = |hint: &str| {
            std::iter::once(&record.title)
                .chain(&record.aliases)
                .any(|title| phrase_contains(&normalize(hint), &normalize(title)))
        };
        let window_match = query.window_title.as_deref().is_some_and(matches_title);
        let filename_match = query.filename.as_deref().is_some_and(matches_title);
        let dialogue_match = query.dialogue.as_deref().is_some_and(|dialogue| {
            let phrase = normalize(dialogue);
            record
                .dialogue_references
                .iter()
                .any(|reference| phrase_contains(&normalize(reference), &phrase))
        });
        // Window title and filename can be copies of the same metadata. They
        // never substitute for an independent, sufficiently long ASR match.
        if !(window_match || filename_match) || !dialogue_match {
            continue;
        }
        let mut signals = vec![MovieSignal::Dialogue, MovieSignal::RemoteMetadata];
        if window_match {
            signals.push(MovieSignal::WindowTitle);
        }
        if filename_match {
            signals.push(MovieSignal::Filename);
        }
        if query.language.is_some() {
            signals.push(MovieSignal::Language);
        }
        if query.duration_us.is_some() {
            signals.push(MovieSignal::Duration);
        }
        movies.push(MovieCandidate {
            id: record.id.clone(),
            title: record.title.clone(),
            year: record.year,
            confidence: record.confidence,
            signals,
        });
    }
    movies.sort_by(|a, b| {
        b.confidence
            .cmp(&a.confidence)
            .then_with(|| a.id.cmp(&b.id))
    });
    let Some(confidence) = movies.first().map(|v| v.confidence) else {
        return unknown();
    };
    let evidence = ContentEvidence {
        kind: ContentType::MovieOrShow,
        confidence,
        songs: Vec::new(),
        movies,
    };
    evidence.validate()?;
    Ok(evidence)
}
fn unknown() -> Result<ContentEvidence, ResolverError> {
    Ok(ContentEvidence {
        kind: ContentType::Unknown,
        confidence: Confidence::new(0)?,
        songs: Vec::new(),
        movies: Vec::new(),
    })
}
fn runtime(value: u64) -> bool {
    (MIN_MEDIA_DURATION_US..=MAX_MEDIA_DURATION_US).contains(&value)
}
fn dialogue_text(value: &str) -> bool {
    !value.trim().is_empty()
        && value.len() <= MAX_DIALOGUE_BYTES
        && !value
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
}
fn normalize(value: &str) -> String {
    value
        .chars()
        .flat_map(char::to_lowercase)
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
/// Match whole tokens rather than a title such as "Film" inside "FilmTwo".
fn phrase_contains(text: &str, phrase: &str) -> bool {
    !phrase.is_empty()
        && text.match_indices(phrase).any(|(start, _)| {
            (start == 0 || text.as_bytes()[start - 1] == b' ')
                && (start + phrase.len() == text.len()
                    || text.as_bytes()[start + phrase.len()] == b' ')
        })
}

#[cfg(test)]
#[path = "movie_tests.rs"]
mod tests;
