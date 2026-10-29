//! Explicit movie-catalog composition. Only bounded current Final ASR and
//! declared media hints enter MovieQuery; captured PCM never enters the catalog.
use super::content::{ContentConfiguration, ContentServiceFactory};
use crate::{
    ai::speech::StreamIdentity,
    resolver::{
        contracts::*,
        movie::{MovieCatalog, MovieMetadataResolver, MovieQuery, dialogue_eligible},
        movie_hints::MovieHintBuffer,
    },
    ui::content::ContentDisclosure,
};
use std::{sync::Arc, time::Instant};

/// Each capture/reconnect receives a fresh, explicitly configured catalog.
/// Construction performs no network, credential lookup, or long CPU work.
pub trait MovieCatalogFactory: Send + Sync {
    fn create(&self, identity: StreamIdentity) -> Result<Arc<dyn MovieCatalog>, ResolverError>;
}

/// Current source metadata supplied by the configured owner. Never derive a
/// filename from a private path, a title from a display label, or runtime from PCM.
pub struct MovieHintMetadata {
    pub stamp: WindowStamp,
    pub window_title: Option<Box<str>>,
    pub filename: Option<Box<str>>,
    pub year: Option<u16>,
    pub duration_us: Option<u64>,
    /// Optional owner lease for dynamically observed source metadata. It retains
    /// the original observation deadline; refresh never renews an issued lease.
    pub validity: Option<MovieMetadataValidity>,
}
pub struct MovieMetadataValidity {
    pub cancellation: ResolverCancellation,
    pub expires_at: Instant,
}
pub trait MovieMetadataSource: Send + Sync {
    /// Synchronous bounded RAM lookup only; no IO or inference here. The source
    /// verifies the selected process identity and freshness before returning hints.
    /// The returned stamp must equal the requested current capture stamp.
    fn metadata(&self, stamp: WindowStamp) -> Result<MovieHintMetadata, ResolverError>;
}

/// The disclosure describes all destinations for Final ASR and media hints.
/// Nothing is enabled by default and there is no cross-provider fallback.
pub fn configuration(
    disclosure: ContentDisclosure,
    catalog: Arc<dyn MovieCatalogFactory>,
    metadata: Arc<dyn MovieMetadataSource>,
) -> Result<ContentConfiguration, ResolverError> {
    ContentConfiguration::new(disclosure, Arc::new(MovieFactory { catalog, metadata }))
        .map(ContentConfiguration::with_movie_dialogue)
}
/// Explicit Windows composition: read only a unique caption directly owned by
/// the selected, creation-verified process. No provider is chosen automatically.
#[cfg(windows)]
pub fn windows_configuration(
    disclosure: ContentDisclosure,
    catalog: Arc<dyn MovieCatalogFactory>,
) -> Result<ContentConfiguration, ResolverError> {
    ContentConfiguration::new(
        disclosure,
        Arc::new(WindowsMovieFactory {
            catalog,
            selected: None,
        }),
    )
    .map(ContentConfiguration::with_movie_dialogue)
}
#[cfg(windows)]
struct WindowsMovieFactory {
    catalog: Arc<dyn MovieCatalogFactory>,
    selected: Option<super::content::SelectedProcess>,
}
#[cfg(windows)]
impl ContentServiceFactory for WindowsMovieFactory {
    fn bind_source(
        &self,
        selected: super::content::SelectedProcess,
    ) -> Result<Option<Arc<dyn ContentServiceFactory>>, ResolverError> {
        if selected.pid == 0 || selected.created.is_none_or(|created| created == 0) {
            return Err(ResolverError::SourceMetadataUnavailable);
        }
        Ok(Some(Arc::new(Self {
            catalog: self.catalog.clone(),
            selected: Some(selected),
        })))
    }
    fn create(
        &self,
        identity: StreamIdentity,
    ) -> Result<Arc<dyn ContentIdentifier>, ResolverError> {
        self.create_with_movie_hints(identity, Arc::new(MovieHintBuffer::new(identity)))
    }
    fn create_with_movie_hints(
        &self,
        identity: StreamIdentity,
        hints: Arc<MovieHintBuffer>,
    ) -> Result<Arc<dyn ContentIdentifier>, ResolverError> {
        let selected = self
            .selected
            .ok_or(ResolverError::SourceMetadataUnavailable)?;
        let metadata = crate::win32::movie_metadata::start(identity, selected)?;
        Ok(Arc::new(MovieIdentifier::new(
            identity,
            self.catalog.create(identity)?,
            hints,
            metadata,
        )?))
    }
}
struct MovieFactory {
    catalog: Arc<dyn MovieCatalogFactory>,
    metadata: Arc<dyn MovieMetadataSource>,
}
impl ContentServiceFactory for MovieFactory {
    fn create(
        &self,
        identity: StreamIdentity,
    ) -> Result<Arc<dyn ContentIdentifier>, ResolverError> {
        self.create_with_movie_hints(identity, Arc::new(MovieHintBuffer::new(identity)))
    }
    fn create_with_movie_hints(
        &self,
        identity: StreamIdentity,
        hints: Arc<MovieHintBuffer>,
    ) -> Result<Arc<dyn ContentIdentifier>, ResolverError> {
        Ok(Arc::new(MovieIdentifier::new(
            identity,
            self.catalog.create(identity)?,
            hints,
            self.metadata.clone(),
        )?))
    }
}
struct MovieIdentifier {
    identity: StreamIdentity,
    resolver: MovieMetadataResolver,
    hints: Arc<MovieHintBuffer>,
    metadata: Arc<dyn MovieMetadataSource>,
}
impl MovieIdentifier {
    fn new(
        identity: StreamIdentity,
        catalog: Arc<dyn MovieCatalog>,
        hints: Arc<MovieHintBuffer>,
        metadata: Arc<dyn MovieMetadataSource>,
    ) -> Result<Self, ResolverError> {
        Ok(Self {
            identity,
            resolver: MovieMetadataResolver::new(identity, catalog)?,
            hints,
            metadata,
        })
    }
}
impl ContentIdentifier for MovieIdentifier {
    fn capabilities(&self) -> ResolverCapabilities {
        ResolverCapabilities {
            classification: false,
            songs: false,
            movies: true,
            cancellation: true,
        }
    }
    fn identify<'a>(
        &'a self,
        window: &'a AudioWindow,
        cancel: ResolverCancellation,
    ) -> IdentifyFuture<'a> {
        Box::pin(async move {
            if cancel.is_cancelled() {
                return Err(ResolverError::Cancelled);
            }
            window.stamp.validate(self.identity, Instant::now())?;
            let Some(hint) = self.hints.query(window.stamp)? else {
                return unknown();
            };
            if !dialogue_eligible(&hint.dialogue) {
                return unknown();
            }
            let mut deadline = window
                .stamp
                .ended_at
                .checked_add(MAX_WINDOW_AGE)
                .ok_or(ResolverError::InvalidData)?
                .min(hint.expires_at);
            let metadata = self.metadata.metadata(window.stamp)?;
            if let Some(validity) = &metadata.validity {
                deadline = deadline.min(validity.expires_at);
                if validity.cancellation.is_cancelled() || Instant::now() > deadline {
                    return Err(ResolverError::Stale);
                }
            }
            let metadata_cancel = metadata
                .validity
                .as_ref()
                .map(|validity| validity.cancellation.clone());
            let stamp = metadata.stamp;
            if stamp.identity != window.stamp.identity
                || stamp.sequence != window.stamp.sequence
                || stamp.ended_at != window.stamp.ended_at
                || stamp.start_us != window.stamp.start_us
                || stamp.duration_us != window.stamp.duration_us
            {
                return Err(ResolverError::InvalidData);
            }
            let query = MovieQuery {
                stamp: window.stamp,
                window_title: metadata.window_title,
                filename: metadata.filename,
                dialogue: Some(hint.dialogue),
                language: hint.language,
                year: metadata.year,
                duration_us: metadata.duration_us,
            };
            query.validate(self.identity, Instant::now())?;
            let result = tokio::select! { biased;
                _ = cancel.cancelled() => return Err(ResolverError::Cancelled),
                _ = hint.cancellation.cancelled() => return Err(ResolverError::Stale),
                _ = async { match &metadata_cancel { Some(cancel) => cancel.cancelled().await, None => std::future::pending().await } } => return Err(ResolverError::Stale),
                _ = tokio::time::sleep_until(deadline.into()) => return Err(ResolverError::Stale),
                result = self.resolver.resolve(&query, cancel.clone()) => result,
            }?;
            if cancel.is_cancelled() {
                return Err(ResolverError::Cancelled);
            }
            if hint.cancellation.is_cancelled()
                || metadata_cancel
                    .as_ref()
                    .is_some_and(ResolverCancellation::is_cancelled)
            {
                return Err(ResolverError::Stale);
            }
            if Instant::now() > deadline {
                return Err(ResolverError::Stale);
            }
            window.stamp.validate(self.identity, Instant::now())?;
            Ok(result)
        })
    }
}
fn unknown() -> Result<ContentEvidence, ResolverError> {
    Ok(ContentEvidence {
        kind: ContentType::Unknown,
        confidence: Confidence::new(0)?,
        songs: Vec::new(),
        movies: Vec::new(),
    })
}

#[cfg(test)]
#[path = "movie_tests.rs"]
mod tests;
