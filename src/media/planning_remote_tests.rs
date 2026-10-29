//! Metadata-only mixed choices, no network, credentials or file reads.
use super::*;
use crate::{ai::speech::SpeechLanguage, resolver::timed_subtitle::SubtitleFormat};

fn media() -> MediaInput {
    MediaInput::local(std::env::temp_dir().join("unopened-synthetic.media")).unwrap()
}
fn local(language: TextLanguage) -> LocalSubtitleInput {
    LocalSubtitleInput::new(
        std::env::temp_dir().join("unopened-synthetic.srt"),
        SubtitleFormat::Srt,
        language,
    )
    .unwrap()
}
fn remote(language: TextLanguage) -> PreparedSubtitleUrl {
    PreparedSubtitleUrl::new(
        "https://synthetic.invalid/explicit?ticket=SYNTHETIC_PRIVATE",
        SubtitleFormat::WebVtt,
        language,
    )
    .unwrap()
}
fn languages() -> [TextLanguage; 8] {
    [
        TextLanguage::TaiwanTraditional,
        TextLanguage::OtherTraditional,
        TextLanguage::SimplifiedChinese,
        TextLanguage::Source(SpeechLanguage::English),
        TextLanguage::Source(SpeechLanguage::Japanese),
        TextLanguage::Source(SpeechLanguage::French),
        TextLanguage::Source(SpeechLanguage::German),
        TextLanguage::Source(SpeechLanguage::Korean),
    ]
}

#[test]
fn https_choices_preserve_explicit_language_without_io_or_asr_planning() {
    for language in languages() {
        let request = PreparedRequest::new(media(), vec![])
            .unwrap()
            .with_remote_subtitle(remote(language))
            .unwrap();
        assert_eq!(request.plan(), Ok(PreparationPlan::RemoteSubtitle(0)));
        assert!(request.text_language().unwrap() == Some(language));
        assert!(request.subtitles.is_empty());
        assert!(request.embedded.is_none());
    }
}

#[test]
fn https_and_local_choices_share_all_language_priorities_and_ties() {
    for (left, local_language) in languages().into_iter().enumerate() {
        for (right, remote_language) in languages().into_iter().enumerate() {
            let request = PreparedRequest::new(media(), vec![local(local_language)])
                .unwrap()
                .with_remote_subtitle(remote(remote_language))
                .unwrap();
            let expected = match left.min(3).cmp(&right.min(3)) {
                std::cmp::Ordering::Less => Ok(PreparationPlan::Subtitle(0)),
                std::cmp::Ordering::Greater => Ok(PreparationPlan::RemoteSubtitle(0)),
                std::cmp::Ordering::Equal => Err(MediaError::AmbiguousSubtitle),
            };
            assert_eq!(request.plan(), expected);
            match expected {
                Ok(PreparationPlan::Subtitle(_)) => {
                    assert!(request.text_language().unwrap() == Some(local_language));
                }
                Ok(_) => assert!(request.text_language().unwrap() == Some(remote_language)),
                Err(error) => {
                    assert!(matches!(request.text_language(), Err(found) if found == error))
                }
            }
        }
    }
}

#[test]
fn https_and_embedded_choices_share_priorities_without_opening_a_container() {
    for (left, embedded_language) in languages().into_iter().enumerate() {
        for (right, remote_language) in languages().into_iter().enumerate() {
            let request = PreparedRequest::new(media(), vec![])
                .unwrap()
                .with_remote_subtitle(remote(remote_language))
                .unwrap()
                .with_embedded_subtitle(Mp4SubtitleSelection::new(3, embedded_language).unwrap())
                .unwrap();
            let expected = match left.min(3).cmp(&right.min(3)) {
                std::cmp::Ordering::Less => Ok(PreparationPlan::EmbeddedMp4),
                std::cmp::Ordering::Greater => Ok(PreparationPlan::RemoteSubtitle(0)),
                std::cmp::Ordering::Equal => Err(MediaError::AmbiguousSubtitle),
            };
            assert_eq!(request.plan(), expected);
            if expected.is_err() {
                assert!(matches!(
                    request.text_language(),
                    Err(MediaError::AmbiguousSubtitle)
                ));
            }
        }
    }
}

#[test]
fn mixed_choice_budget_counts_every_local_embedded_and_remote_candidate() {
    let source = TextLanguage::Source(SpeechLanguage::English);
    let mut request =
        PreparedRequest::new(media(), (0..4).map(|_| local(source)).collect()).unwrap();
    for _ in 0..3 {
        request = request.with_remote_subtitle(remote(source)).unwrap();
    }
    request = request
        .with_embedded_subtitle(Mp4SubtitleSelection::new(3, source).unwrap())
        .unwrap();
    assert_eq!(request.choice_count(), MAX_SUBTITLE_CHOICES);
    assert!(matches!(
        request.with_remote_subtitle(remote(source)),
        Err(MediaError::TooLarge)
    ));

    let mut request = PreparedRequest::new(media(), vec![]).unwrap();
    for _ in 0..MAX_SUBTITLE_CHOICES {
        request = request.with_remote_subtitle(remote(source)).unwrap();
    }
    assert!(matches!(
        request.with_embedded_subtitle(Mp4SubtitleSelection::new(3, source).unwrap()),
        Err(MediaError::TooLarge)
    ));
    let mut request = PreparedRequest::new(media(), vec![]).unwrap();
    for _ in 0..MAX_SUBTITLE_CHOICES {
        request = request.with_remote_subtitle(remote(source)).unwrap();
    }
    assert!(matches!(
        request.with_remote_subtitle(remote(source)),
        Err(MediaError::TooLarge)
    ));
}

#[test]
fn winner_keeps_its_original_remote_index_and_worse_ties_do_not_hide_it() {
    let source = TextLanguage::Source(SpeechLanguage::English);
    let request = PreparedRequest::new(media(), vec![local(source), local(source)])
        .unwrap()
        .with_remote_subtitle(remote(TextLanguage::SimplifiedChinese))
        .unwrap()
        .with_remote_subtitle(remote(TextLanguage::TaiwanTraditional))
        .unwrap()
        .with_remote_subtitle(remote(TextLanguage::SimplifiedChinese))
        .unwrap();
    assert_eq!(request.plan(), Ok(PreparationPlan::RemoteSubtitle(1)));
    assert!(request.text_language().unwrap() == Some(TextLanguage::TaiwanTraditional));
}
