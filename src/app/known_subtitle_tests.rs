use super::*;
use crate::{
    ai::speech::SpeechLanguage,
    audio::capture::SessionId,
    resolver::{contracts::Confidence, timed_subtitle::SubtitleFormat},
    ui::subtitle_source::SubtitleSourceDraft,
};
fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(1333),
        generation: 1,
    }
}
fn registry() -> Arc<Registry> {
    Registry::new(
        &SubtitleSourceSettings::from_draft(SubtitleSourceDraft {
            title: "Synthetic film".into(),
            url: "https://example.invalid/private.srt?token=synthetic".into(),
            format: SubtitleFormat::Srt,
            language: SpeechLanguage::English,
        })
        .unwrap(),
    )
    .unwrap()
}
fn movie(session: &SessionKnownSubtitle) -> MovieCandidate {
    MovieCandidate {
        id: session.content_id().into(),
        title: session.title().into(),
        year: None,
        confidence: Confidence::new(900).unwrap(),
        signals: vec![],
    }
}
#[test]
fn catalog_and_binder_share_exactly_one_session_owner_in_both_creation_orders() {
    for catalog_first in [true, false] {
        let registry = registry();
        let (catalog, binder) = if catalog_first {
            let catalog = MovieCatalogFactory::create(registry.as_ref(), identity()).unwrap();
            (
                catalog,
                SubtitleServiceFactory::create(registry.as_ref(), identity()).unwrap(),
            )
        } else {
            let binder = SubtitleServiceFactory::create(registry.as_ref(), identity()).unwrap();
            (
                MovieCatalogFactory::create(registry.as_ref(), identity()).unwrap(),
                binder,
            )
        };
        let session = registry.session(identity()).unwrap();
        assert_eq!(Arc::strong_count(&session), 3);
        let provider = binder.bind(&movie(&session)).unwrap();
        let guard = provider.publication_guard().unwrap();
        drop(session);
        drop(binder);
        assert!(!guard.is_cancelled());
        drop(catalog);
        assert!(guard.is_cancelled());
        assert!(
            registry
                .current
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .1
                .upgrade()
                .is_none()
        );
    }
}
#[test]
fn another_live_identity_cannot_replace_or_retire_current_owner() {
    let registry = registry();
    let session = registry.session(identity()).unwrap();
    let guard = session.provider().publication_guard().unwrap();
    assert!(matches!(
        registry.session(StreamIdentity {
            generation: 2,
            ..identity()
        }),
        Err(ResolverError::RateLimited)
    ));
    assert!(!guard.is_cancelled());
    assert!(Arc::ptr_eq(
        &session,
        &registry.session(identity()).unwrap()
    ));
    let old_id = session.content_id().to_string();
    drop(session);
    let next = registry
        .session(StreamIdentity {
            generation: 2,
            ..identity()
        })
        .unwrap();
    assert!(guard.is_cancelled());
    assert_ne!(next.content_id(), old_id);
}
#[test]
fn binder_rejects_content_facts_without_constructing_new_provider() {
    let registry = registry();
    let session = registry.session(identity()).unwrap();
    let binder = SubtitleServiceFactory::create(registry.as_ref(), identity()).unwrap();
    let known = movie(&session);
    assert_eq!(binder.revision(&known), Some(1));
    for change in 0..3 {
        let mut mismatched = movie(&session);
        match change {
            0 => mismatched.id = "other".into(),
            1 => mismatched.title = "other".into(),
            _ => mismatched.year = Some(2026),
        }
        assert_eq!(binder.revision(&mismatched), None);
        assert!(matches!(
            binder.bind(&mismatched),
            Err(ResolverError::ContentMismatch)
        ));
    }
    let a = binder.bind(&known).unwrap().publication_guard().unwrap();
    let b = binder.bind(&known).unwrap().publication_guard().unwrap();
    assert!(a.same_scope(&b));
}
#[test]
fn disclosure_separates_local_asr_comparison_and_remote_text_download() {
    assert_eq!(LOCAL_DISCLOSURE.transport, ContentTransport::Local);
    assert_eq!(TEXT_DISCLOSURE.transport, ContentTransport::Remote);
    let registry = registry();
    assert_eq!(
        registry.subtitles().unwrap().disclosure().service,
        TEXT_DISCLOSURE.service
    );
    assert!(registry.current.lock().unwrap().is_none());
    #[cfg(windows)]
    {
        let settings = SubtitleSourceSettings::from_draft(SubtitleSourceDraft {
            title: "Synthetic film".into(),
            url: "https://example.invalid/source.srt".into(),
            format: SubtitleFormat::Srt,
            language: SpeechLanguage::English,
        })
        .unwrap();
        let config = configuration(&settings).unwrap();
        assert!(config.movie_dialogue_enabled());
        assert_eq!(config.disclosure().transport, ContentTransport::Local);
        assert_eq!(
            config.subtitles_disclosure().unwrap().transport,
            ContentTransport::Remote
        );
        assert!(config.lyrics_disclosure().is_none());
    }
}
