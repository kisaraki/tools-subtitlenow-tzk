//! Until native disclosure is wired, the generic controller never fetches URLs.
use super::*;
use crate::media::remote_subtitle::PreparedSubtitleUrl;

fn remote(language: TextLanguage) -> PreparedSubtitleUrl {
    PreparedSubtitleUrl::new(
        "https://synthetic.invalid/private-subtitle?ticket=SYNTHETIC_PRIVATE",
        SubtitleFormat::Srt,
        language,
    )
    .unwrap()
}

#[test]
fn https_choices_require_disclosed_entry_before_generic_controller_keys_or_worker() {
    let _serial = OWNERS.lock().unwrap();
    let fixture = Fixture::new();
    let factory = Factory::new(Behavior::Panic);
    let mut controller = PreparedController::with_factory(factory.clone());
    for scenario in 0..3 {
        let request = match scenario {
            0 => PreparedRequest::new(MediaInput::local(fixture.media.clone()).unwrap(), vec![])
                .unwrap()
                .with_remote_subtitle(remote(TextLanguage::TaiwanTraditional))
                .unwrap(),
            1 => PreparedRequest::new(
                MediaInput::remote("https://synthetic.invalid/media").unwrap(),
                vec![],
            )
            .unwrap()
            .with_remote_subtitle(remote(TextLanguage::Source(SpeechLanguage::English)))
            .unwrap(),
            _ => fixture
                .chinese()
                .with_remote_subtitle(remote(TextLanguage::Source(SpeechLanguage::Japanese)))
                .unwrap(),
        };
        assert_eq!(
            controller.start(request, options(None), key()),
            Err(PreparedUiError::Media(MediaError::UnsupportedUrl))
        );
        assert!(controller.worker.is_none());
        assert!(controller.session.identity().is_none());
        assert!(controller.ready().is_none());
    }
    assert_eq!(factory.builds.load(Ordering::SeqCst), 0);
    assert_eq!(factory.explicit_keys.load(Ordering::SeqCst), 0);
}

#[test]
fn https_generic_controller_preserves_ambiguity_before_unsupported_or_provider_errors() {
    let _serial = OWNERS.lock().unwrap();
    let fixture = Fixture::new();
    let factory = Factory::new(Behavior::Panic);
    let mut controller = PreparedController::with_factory(factory.clone());
    let request = fixture
        .chinese()
        .with_remote_subtitle(remote(TextLanguage::SimplifiedChinese))
        .unwrap();
    assert_eq!(
        controller.start(request, options(None), None),
        Err(PreparedUiError::Media(MediaError::AmbiguousSubtitle))
    );
    assert_eq!(factory.builds.load(Ordering::SeqCst), 0);
    assert!(controller.worker.is_none());
    assert!(controller.session.identity().is_none());
}
