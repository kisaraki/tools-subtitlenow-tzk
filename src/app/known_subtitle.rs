//! Explicit known-title HTTPS subtitle composition, with one owner per Session.
//! The source document is fetched once; ASR/title comparison stays on this device.
use super::{
    content::ContentConfiguration,
    movie::MovieCatalogFactory,
    subtitles::{SubtitleConfiguration, SubtitleMovieBinder, SubtitleServiceFactory},
};
use crate::{
    ai::speech::StreamIdentity,
    resolver::{
        contracts::{ExistingTextProvider, MovieCandidate, ResolverError},
        known_subtitle::{KnownSubtitleSource, SessionKnownSubtitle},
        movie::{MovieCatalog, MovieCatalogCapabilities, MovieCatalogFuture, MovieQuery},
    },
    ui::{
        content::{ContentDisclosure, ContentTransport},
        subtitle_source::SubtitleSourceSettings,
    },
};
use std::sync::{Arc, Mutex, Weak};

const LOCAL_DISCLOSURE: ContentDisclosure = ContentDisclosure {
    service: "本機核對指定字幕",
    transport: ContentTransport::Local,
};
const TEXT_DISCLOSURE: ContentDisclosure = ContentDisclosure {
    service: "指定的 HTTPS 字幕來源（見來源設定）",
    transport: ContentTransport::Remote,
};
struct Registry {
    source: KnownSubtitleSource,
    current: Mutex<Option<(StreamIdentity, Weak<SessionKnownSubtitle>)>>,
}
impl Registry {
    fn new(settings: &SubtitleSourceSettings) -> Result<Arc<Self>, ResolverError> {
        Ok(Arc::new(Self {
            source: KnownSubtitleSource::new(settings.title().into(), settings.track()?)?,
            current: Mutex::new(None),
        }))
    }
    fn session(
        &self,
        identity: StreamIdentity,
    ) -> Result<Arc<SessionKnownSubtitle>, ResolverError> {
        let mut current = self
            .current
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some((existing, weak)) = &*current
            && let Some(session) = weak.upgrade()
        {
            // Do not cancel or replace another live Session via an old callback.
            return if *existing == identity {
                Ok(session)
            } else {
                Err(ResolverError::RateLimited)
            };
        }
        let session = Arc::new(self.source.start(identity)?);
        *current = Some((identity, Arc::downgrade(&session)));
        Ok(session)
    }
    fn subtitles(self: &Arc<Self>) -> Result<SubtitleConfiguration, ResolverError> {
        SubtitleConfiguration::new(TEXT_DISCLOSURE, self.clone())
    }
}
struct CatalogOwner {
    _session: Arc<SessionKnownSubtitle>,
    reader: Arc<dyn MovieCatalog>,
}
impl MovieCatalog for CatalogOwner {
    fn capabilities(&self) -> MovieCatalogCapabilities {
        self.reader.capabilities()
    }
    fn lookup<'a>(
        &'a self,
        query: &'a MovieQuery,
        cancel: crate::resolver::contracts::ResolverCancellation,
    ) -> MovieCatalogFuture<'a> {
        self.reader.lookup(query, cancel)
    }
}
impl MovieCatalogFactory for Registry {
    fn create(&self, identity: StreamIdentity) -> Result<Arc<dyn MovieCatalog>, ResolverError> {
        let session = self.session(identity)?;
        Ok(Arc::new(CatalogOwner {
            reader: session.catalog(),
            _session: session,
        }))
    }
}
struct Binder {
    session: Arc<SessionKnownSubtitle>,
}
impl Binder {
    fn matches(&self, movie: &MovieCandidate) -> bool {
        movie.id.as_ref() == self.session.content_id()
            && movie.title.as_ref() == self.session.title()
            && movie.year.is_none()
    }
}
impl SubtitleMovieBinder for Binder {
    fn revision(&self, movie: &MovieCandidate) -> Option<u64> {
        // Settings/document are immutable for this run. Session identity and
        // the provider guard distinguish generations; there is no URL refresh.
        self.matches(movie).then_some(1)
    }
    fn bind(&self, movie: &MovieCandidate) -> Result<Arc<dyn ExistingTextProvider>, ResolverError> {
        if !self.matches(movie) {
            return Err(ResolverError::ContentMismatch);
        }
        Ok(self.session.provider())
    }
}
impl SubtitleServiceFactory for Registry {
    fn create(
        &self,
        identity: StreamIdentity,
    ) -> Result<Arc<dyn SubtitleMovieBinder>, ResolverError> {
        Ok(Arc::new(Binder {
            session: self.session(identity)?,
        }))
    }
}

/// The GUI first shows the actual host from this immutable settings snapshot.
/// Both the local comparison and remote text GET must pass the existing gate.
#[cfg(windows)]
pub fn configuration(
    settings: &SubtitleSourceSettings,
) -> Result<ContentConfiguration, ResolverError> {
    let registry = Registry::new(settings)?;
    let content = super::movie::windows_configuration(LOCAL_DISCLOSURE, registry.clone())?;
    Ok(content.with_subtitles(registry.subtitles()?))
}

#[cfg(test)]
#[path = "known_subtitle_tests.rs"]
mod tests;
