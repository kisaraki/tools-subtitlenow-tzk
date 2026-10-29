//! Explicit composition of the real LRCLIB adapter. No language/duration guessing.
use super::lyrics::{LyricsConfiguration, LyricsServiceFactory, LyricsSongBinder};
use crate::{
    ai::speech::StreamIdentity,
    resolver::{
        candidates::{Resolution, SongState},
        contracts::{
            ContentType, ExistingTextProvider, ResolverCancellation, ResolverError, SongCandidate,
            TextLanguage, guard_provider,
        },
        lyrics::{LrcLibService, LyricsTrack},
    },
    ui::content::{ContentDisclosure, ContentTransport},
};
use std::sync::Arc;

pub struct LyricsMetadata {
    pub identity: StreamIdentity,
    pub content_id: Box<str>,
    pub language: Option<TextLanguage>,
    pub album: Option<Box<str>>,
    /// Full recording duration from verified metadata, never the capture window.
    pub duration_us: Option<u64>,
    /// Cancellation of the metadata registration invalidates derived publications.
    pub availability: Option<ResolverCancellation>,
}
pub trait LyricsMetadataSource: Send + Sync {
    /// Read already-known Session metadata only. No synchronous I/O or guessing.
    fn metadata(
        &self,
        identity: StreamIdentity,
        song: &SongCandidate,
    ) -> Result<LyricsMetadata, ResolverError>;
    fn revision(&self, _identity: StreamIdentity, _song: &SongCandidate) -> Option<u64> {
        None
    }
}
struct LrcLibFactory(Arc<dyn LyricsMetadataSource>);
struct LrcLibBinder {
    identity: StreamIdentity,
    service: Arc<LrcLibService>,
    metadata: Arc<dyn LyricsMetadataSource>,
}
/// Merely constructs explicit configuration; default startup does not call it.
pub fn configuration(
    metadata: Arc<dyn LyricsMetadataSource>,
) -> Result<LyricsConfiguration, ResolverError> {
    LyricsConfiguration::new(
        ContentDisclosure {
            service: "LRCLIB",
            transport: ContentTransport::Remote,
        },
        Arc::new(LrcLibFactory(metadata)),
    )
}
impl LyricsServiceFactory for LrcLibFactory {
    fn create(&self, identity: StreamIdentity) -> Result<Arc<dyn LyricsSongBinder>, ResolverError> {
        Ok(Arc::new(LrcLibBinder {
            identity,
            service: LrcLibService::new(identity)?,
            metadata: self.0.clone(),
        }))
    }
}
impl LyricsSongBinder for LrcLibBinder {
    fn bind(
        &self,
        resolution: &Resolution,
    ) -> Result<Arc<dyn ExistingTextProvider>, ResolverError> {
        if resolution.kind != ContentType::Music || resolution.song_state != SongState::Confirmed {
            return Err(ResolverError::ContentMismatch);
        }
        let song = resolution.song.as_ref().ok_or(ResolverError::InvalidData)?;
        let metadata = self.metadata.metadata(self.identity, song)?;
        if metadata.identity != self.identity {
            return Err(ResolverError::InvalidData);
        }
        if metadata.content_id != song.id {
            return Err(ResolverError::ContentMismatch);
        }
        if metadata
            .availability
            .as_ref()
            .is_some_and(ResolverCancellation::is_cancelled)
        {
            return Err(ResolverError::Cancelled);
        }
        let track = LyricsTrack::from_resolution(
            resolution,
            metadata
                .language
                .ok_or(ResolverError::UnknownTextLanguage)?,
            metadata.album,
            metadata.duration_us,
        )?;
        Ok(guard_provider(
            Arc::new(self.service.for_song(track)),
            metadata.availability,
        ))
    }
    fn revision(&self, resolution: &Resolution) -> Option<u64> {
        resolution
            .song
            .as_ref()
            .and_then(|song| self.metadata.revision(self.identity, song))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{audio::capture::SessionId, resolver::contracts::Confidence};
    struct Known(bool);
    impl LyricsMetadataSource for Known {
        fn metadata(
            &self,
            identity: StreamIdentity,
            song: &SongCandidate,
        ) -> Result<LyricsMetadata, ResolverError> {
            Ok(LyricsMetadata {
                identity,
                content_id: song.id.clone(),
                language: self.0.then_some(TextLanguage::TaiwanTraditional),
                album: None,
                duration_us: None,
                availability: None,
            })
        }
    }
    fn identity() -> StreamIdentity {
        StreamIdentity {
            session: SessionId(812),
            generation: 1,
        }
    }
    fn confirmed() -> Resolution {
        Resolution {
            kind: ContentType::Music,
            song_state: SongState::Confirmed,
            song: Some(Arc::new(SongCandidate {
                id: "synthetic".into(),
                title: "synthetic title".into(),
                artist: Some("synthetic artist".into()),
                confidence: Confidence::new(900).unwrap(),
                match_offset_us: None,
            })),
            movies: vec![],
        }
    }
    #[test]
    fn unknown_language_refuses_binding_without_lookup() {
        let binder = LrcLibFactory(Arc::new(Known(false)))
            .create(identity())
            .unwrap();
        assert!(matches!(
            binder.bind(&confirmed()),
            Err(ResolverError::UnknownTextLanguage)
        ));
        assert!(matches!(
            binder.bind(&Resolution::default()),
            Err(ResolverError::ContentMismatch)
        ));
    }
    #[test]
    fn configured_sessions_have_independent_services_and_explicit_disclosure() {
        let factory = LrcLibFactory(Arc::new(Known(true)));
        let a = factory.create(identity()).unwrap();
        let b = factory
            .create(StreamIdentity {
                generation: 2,
                ..identity()
            })
            .unwrap();
        assert!(!Arc::ptr_eq(&a, &b));
        assert!(
            a.bind(&confirmed())
                .unwrap()
                .supports(crate::resolver::contracts::SubtitleSource::ExistingLyrics)
        );
        let config = configuration(Arc::new(Known(true))).unwrap();
        assert_eq!(config.disclosure().transport, ContentTransport::Remote);
        assert_eq!(config.disclosure().service, "LRCLIB");
    }
    struct FixedMetadata {
        identity: StreamIdentity,
        content_id: Box<str>,
        guard: ResolverCancellation,
    }
    impl LyricsMetadataSource for FixedMetadata {
        fn metadata(
            &self,
            _: StreamIdentity,
            _: &SongCandidate,
        ) -> Result<LyricsMetadata, ResolverError> {
            Ok(LyricsMetadata {
                identity: self.identity,
                content_id: self.content_id.clone(),
                language: Some(TextLanguage::TaiwanTraditional),
                album: None,
                duration_us: None,
                availability: Some(self.guard.clone()),
            })
        }
        fn revision(&self, identity: StreamIdentity, song: &SongCandidate) -> Option<u64> {
            (identity == self.identity && song.id == self.content_id).then_some(11)
        }
    }
    fn fixed(identity: StreamIdentity, content_id: &str) -> Arc<FixedMetadata> {
        Arc::new(FixedMetadata {
            identity,
            content_id: content_id.into(),
            guard: ResolverCancellation::default(),
        })
    }
    #[test]
    fn foreign_session_and_foreign_content_are_rejected_before_provider_construction() {
        let foreign = StreamIdentity {
            generation: 2,
            ..identity()
        };
        let foreign_session = LrcLibFactory(fixed(foreign, "synthetic"))
            .create(identity())
            .unwrap();
        assert!(matches!(
            foreign_session.bind(&confirmed()),
            Err(ResolverError::InvalidData)
        ));
        let foreign_content = LrcLibFactory(fixed(identity(), "foreign synthetic song"))
            .create(identity())
            .unwrap();
        assert!(matches!(
            foreign_content.bind(&confirmed()),
            Err(ResolverError::ContentMismatch)
        ));
    }
    #[test]
    fn a_new_session_cannot_reuse_prior_metadata_and_its_revision() {
        let factory = LrcLibFactory(fixed(identity(), "synthetic"));
        let old = factory.create(identity()).unwrap();
        let fresh = factory
            .create(StreamIdentity {
                generation: 2,
                ..identity()
            })
            .unwrap();
        assert!(old.bind(&confirmed()).is_ok());
        assert_eq!(old.revision(&confirmed()), Some(11));
        assert!(matches!(
            fresh.bind(&confirmed()),
            Err(ResolverError::InvalidData)
        ));
        assert_eq!(fresh.revision(&confirmed()), None);
    }
    #[test]
    fn cancellation_guard_is_preserved_and_cancelled_registration_refuses_rebinding() {
        let metadata = fixed(identity(), "synthetic");
        let binder = LrcLibFactory(metadata.clone()).create(identity()).unwrap();
        let provider = binder.bind(&confirmed()).unwrap();
        let guard = provider.publication_guard().unwrap();
        assert!(guard.same_scope(&metadata.guard));
        metadata.guard.cancel();
        assert!(guard.is_cancelled());
        assert!(matches!(
            binder.bind(&confirmed()),
            Err(ResolverError::Cancelled)
        ));
    }
}
