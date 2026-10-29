//! Explicit real HTTPS subtitle composition using already-known release metadata.
//! No URL/language/edition guessing, credential lookup or network in the factory.
use super::subtitles::{SubtitleConfiguration, SubtitleMovieBinder, SubtitleServiceFactory};
use crate::{
    ai::speech::StreamIdentity,
    resolver::{
        contracts::{
            ExistingTextProvider, MovieCandidate, ResolverCancellation, ResolverError,
            guard_provider,
        },
        timed_subtitle::{ReferenceTrack, SubtitleTrack, TimedSubtitleProvider},
    },
    ui::content::{ContentDisclosure, ContentTransport},
};
use std::sync::Arc;

/// Private Session metadata: signed locations and content identity are not logged
/// or serialized. The provider must supply actual track language and edition IDs.
pub struct SubtitleMetadata {
    pub identity: StreamIdentity,
    pub content_id: Box<str>,
    pub primary: SubtitleTrack,
    pub reference: Option<ReferenceTrack>,
    /// Revoked when this Session metadata is replaced, removed or stopped.
    pub availability: Option<ResolverCancellation>,
}
pub trait SubtitleMetadataSource: Send + Sync {
    /// A Session-local metadata version; this callback never performs I/O.
    fn revision(&self, _identity: StreamIdentity, _movie: &MovieCandidate) -> Option<u64> {
        None
    }
    /// Read existing verified Session metadata only; never synchronous I/O or guessing.
    fn metadata(
        &self,
        identity: StreamIdentity,
        movie: &MovieCandidate,
    ) -> Result<SubtitleMetadata, ResolverError>;
}
struct HttpFactory(Arc<dyn SubtitleMetadataSource>);
struct HttpBinder {
    identity: StreamIdentity,
    metadata: Arc<dyn SubtitleMetadataSource>,
}
/// Disclosure must describe ALL destinations used by the metadata source. Merely
/// constructing configuration neither downloads text nor enables a default service.
pub fn configuration(
    disclosure: ContentDisclosure,
    metadata: Arc<dyn SubtitleMetadataSource>,
) -> Result<SubtitleConfiguration, ResolverError> {
    if disclosure.transport != ContentTransport::Remote {
        return Err(ResolverError::InvalidData);
    }
    SubtitleConfiguration::new(disclosure, Arc::new(HttpFactory(metadata)))
}
impl SubtitleServiceFactory for HttpFactory {
    fn create(
        &self,
        identity: StreamIdentity,
    ) -> Result<Arc<dyn SubtitleMovieBinder>, ResolverError> {
        Ok(Arc::new(HttpBinder {
            identity,
            metadata: self.0.clone(),
        }))
    }
}
impl SubtitleMovieBinder for HttpBinder {
    fn revision(&self, movie: &MovieCandidate) -> Option<u64> {
        self.metadata.revision(self.identity, movie)
    }
    fn bind(&self, movie: &MovieCandidate) -> Result<Arc<dyn ExistingTextProvider>, ResolverError> {
        let metadata = self.metadata.metadata(self.identity, movie)?;
        if metadata.identity != self.identity {
            return Err(ResolverError::InvalidData);
        }
        if metadata.content_id != movie.id {
            return Err(ResolverError::ContentMismatch);
        }
        if metadata
            .availability
            .as_ref()
            .is_some_and(ResolverCancellation::is_cancelled)
        {
            return Err(ResolverError::Cancelled);
        }
        let provider: Arc<dyn ExistingTextProvider> = Arc::new(TimedSubtitleProvider::new(
            self.identity,
            metadata.content_id,
            metadata.primary,
            metadata.reference,
        )?);
        Ok(guard_provider(provider, metadata.availability))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ai::speech::SpeechLanguage,
        audio::capture::SessionId,
        resolver::{
            contracts::{
                Confidence, MovieSignal, SubtitleSource, TextLanguage, TextQuery, WindowStamp,
            },
            timed_subtitle::SubtitleFormat,
        },
    };
    use std::time::Instant;
    struct KnownMetadata {
        mismatch: bool,
    }
    impl SubtitleMetadataSource for KnownMetadata {
        fn metadata(
            &self,
            identity: StreamIdentity,
            movie: &MovieCandidate,
        ) -> Result<SubtitleMetadata, ResolverError> {
            Ok(SubtitleMetadata {
                identity,
                content_id: if self.mismatch {
                    "different synthetic movie".into()
                } else {
                    movie.id.clone()
                },
                primary: SubtitleTrack::new(
                    "https://example.invalid/subtitle.srt?signature=synthetic",
                    SubtitleFormat::Srt,
                    TextLanguage::Source(SpeechLanguage::English),
                )?,
                reference: None,
                availability: None,
            })
        }
    }
    fn identity() -> StreamIdentity {
        StreamIdentity {
            session: SessionId(994),
            generation: 1,
        }
    }
    fn movie() -> MovieCandidate {
        MovieCandidate {
            id: "synthetic movie".into(),
            title: "synthetic title".into(),
            year: None,
            confidence: Confidence::new(900).unwrap(),
            signals: vec![
                MovieSignal::WindowTitle,
                MovieSignal::Dialogue,
                MovieSignal::RemoteMetadata,
            ],
        }
    }
    fn disclosure() -> ContentDisclosure {
        ContentDisclosure {
            service: "已宣告合成 HTTPS 來源",
            transport: ContentTransport::Remote,
        }
    }
    #[test]
    fn real_https_composition_requires_remote_disclosure_and_matching_release_identity() {
        assert!(matches!(
            configuration(
                ContentDisclosure {
                    transport: ContentTransport::Local,
                    ..disclosure()
                },
                Arc::new(KnownMetadata { mismatch: false })
            ),
            Err(ResolverError::InvalidData)
        ));
        let config =
            configuration(disclosure(), Arc::new(KnownMetadata { mismatch: false })).unwrap();
        assert_eq!(config.disclosure().transport, ContentTransport::Remote);
        assert_eq!(config.disclosure().service, disclosure().service);
        let binder = HttpFactory(Arc::new(KnownMetadata { mismatch: true }))
            .create(identity())
            .unwrap();
        assert!(matches!(
            binder.bind(&movie()),
            Err(ResolverError::ContentMismatch)
        ));
    }
    #[tokio::test]
    async fn real_https_factory_has_independent_session_bindings_and_rejects_foreign_queries_before_io()
     {
        let factory = HttpFactory(Arc::new(KnownMetadata { mismatch: false }));
        let first = factory.create(identity()).unwrap();
        let second_id = StreamIdentity {
            generation: 2,
            ..identity()
        };
        let second = factory.create(second_id).unwrap();
        assert!(!Arc::ptr_eq(&first, &second));
        let first_provider = first.bind(&movie()).unwrap();
        let second_provider = second.bind(&movie()).unwrap();
        assert!(!Arc::ptr_eq(&first_provider, &second_provider));
        assert!(first_provider.supports(SubtitleSource::ExistingSubtitle));
        assert!(!first_provider.supports(SubtitleSource::ExistingLyrics));
        let query = TextQuery {
            stamp: WindowStamp {
                identity: second_id,
                sequence: 1,
                ended_at: Instant::now(),
                start_us: Some(0),
                duration_us: 12_000_000,
            },
            source: SubtitleSource::ExistingSubtitle,
            content_id: movie().id,
        };
        assert!(matches!(
            first_provider
                .lookup_bundle(
                    &query,
                    crate::resolver::contracts::ResolverCancellation::default()
                )
                .await,
            Err(ResolverError::InvalidData)
        ));
    }
    #[test]
    fn previous_session_metadata_cannot_build_a_new_session_provider() {
        struct OldMetadata;
        impl SubtitleMetadataSource for OldMetadata {
            fn metadata(
                &self,
                _: StreamIdentity,
                movie: &MovieCandidate,
            ) -> Result<SubtitleMetadata, ResolverError> {
                KnownMetadata { mismatch: false }.metadata(identity(), movie)
            }
        }
        let binder = HttpFactory(Arc::new(OldMetadata))
            .create(StreamIdentity {
                generation: 2,
                ..identity()
            })
            .unwrap();
        assert!(matches!(
            binder.bind(&movie()),
            Err(ResolverError::InvalidData)
        ));
    }

    #[tokio::test]
    async fn http_binding_forwards_session_revision_and_cancellation_without_io() {
        struct ScopedMetadata(ResolverCancellation);
        impl SubtitleMetadataSource for ScopedMetadata {
            fn revision(&self, request: StreamIdentity, selected: &MovieCandidate) -> Option<u64> {
                (request == identity() && selected.id == movie().id).then_some(7)
            }
            fn metadata(
                &self,
                identity: StreamIdentity,
                movie: &MovieCandidate,
            ) -> Result<SubtitleMetadata, ResolverError> {
                let mut metadata = KnownMetadata { mismatch: false }.metadata(identity, movie)?;
                metadata.availability = Some(self.0.clone());
                Ok(metadata)
            }
        }
        let guard = ResolverCancellation::default();
        let factory = HttpFactory(Arc::new(ScopedMetadata(guard.clone())));
        let binder = factory.create(identity()).unwrap();
        assert_eq!(binder.revision(&movie()), Some(7));
        let foreign = factory
            .create(StreamIdentity {
                generation: 2,
                ..identity()
            })
            .unwrap();
        assert_eq!(foreign.revision(&movie()), None);
        let provider = binder.bind(&movie()).unwrap();
        assert!(provider.publication_guard().unwrap().same_scope(&guard));
        guard.cancel();
        let query = TextQuery {
            stamp: WindowStamp {
                identity: identity(),
                sequence: 1,
                ended_at: Instant::now(),
                start_us: Some(0),
                duration_us: 12_000_000,
            },
            source: SubtitleSource::ExistingSubtitle,
            content_id: movie().id,
        };
        assert!(matches!(
            provider
                .lookup_bundle(&query, ResolverCancellation::default())
                .await,
            Err(ResolverError::Cancelled)
        ));
        assert!(matches!(
            binder.bind(&movie()),
            Err(ResolverError::Cancelled)
        ));
    }
}
