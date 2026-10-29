//! Original synthetic catalog metadata only. No audio, keys, cloud or text lookup.
use super::*;
use crate::{
    ai::speech::SpeechLanguage,
    audio::capture::SessionId,
    resolver::{
        contracts::{Confidence, MovieSignal},
        timed_subtitle::{ReferenceTrack, SubtitleFormat, SubtitleTrack},
    },
};
use std::time::Duration;
fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(740),
        generation: 1,
    }
}
fn stamp(sequence: u64) -> WindowStamp {
    WindowStamp {
        identity: identity(),
        sequence,
        ended_at: Instant::now(),
        start_us: Some(0),
        duration_us: 12_000_000,
    }
}
fn song() -> SongCandidate {
    SongCandidate {
        id: "synthetic:track".into(),
        title: "Synthetic Lantern".into(),
        artist: Some("Fixture Artist".into()),
        confidence: Confidence::new(920).unwrap(),
        match_offset_us: None,
    }
}
fn movie() -> MovieCandidate {
    MovieCandidate {
        id: "synthetic:feature".into(),
        title: "Synthetic Feature".into(),
        year: Some(2020),
        confidence: Confidence::new(910).unwrap(),
        signals: vec![
            MovieSignal::Dialogue,
            MovieSignal::WindowTitle,
            MovieSignal::RemoteMetadata,
        ],
    }
}
fn lyrics(language: Option<TextLanguage>) -> LyricsMetadata {
    LyricsMetadata {
        identity: identity(),
        content_id: song().id,
        language,
        album: Some("Fixture Album".into()),
        duration_us: Some(120_000_000),
        availability: None,
    }
}
fn subtitle(location: &str) -> SubtitleMetadata {
    SubtitleMetadata {
        identity: identity(),
        content_id: movie().id,
        primary: SubtitleTrack::new(
            location,
            SubtitleFormat::Srt,
            TextLanguage::SimplifiedChinese,
        )
        .unwrap(),
        reference: Some(
            ReferenceTrack::new(
                SubtitleTrack::new(
                    "https://example.invalid/source.vtt",
                    SubtitleFormat::WebVtt,
                    TextLanguage::Source(SpeechLanguage::English),
                )
                .unwrap(),
                "fixture:edition".into(),
                "fixture:edition".into(),
            )
            .unwrap(),
        ),
        availability: None,
    }
}
fn song_metadata(reader: &SessionTextMetadataReader) -> LyricsMetadata {
    LyricsMetadataSource::metadata(reader, identity(), &song()).unwrap()
}
fn movie_metadata(reader: &SessionTextMetadataReader) -> SubtitleMetadata {
    SubtitleMetadataSource::metadata(reader, identity(), &movie()).unwrap()
}
const FIRST_URL: &str = "https://example.invalid/primary.srt?signature=synthetic";
#[test]
fn real_metadata_sources_return_declared_facts_and_atomic_reference_without_inference() {
    let owner = SessionTextMetadata::new(identity());
    let reader = owner.reader();
    assert!(
        owner
            .publish_lyrics(stamp(1), &song(), lyrics(None))
            .unwrap()
    );
    let actual = song_metadata(&reader);
    assert!(actual.language.is_none());
    assert_eq!(actual.album.as_deref(), Some("Fixture Album"));
    assert_eq!(actual.duration_us, Some(120_000_000));
    assert_eq!(actual.content_id, song().id);
    assert!(
        owner
            .publish_subtitle(stamp(1), &movie(), subtitle(FIRST_URL))
            .unwrap()
    );
    let actual = movie_metadata(&reader);
    assert_eq!(actual.content_id, movie().id);
    assert!(actual.primary == subtitle(FIRST_URL).primary);
    assert!(actual.reference == subtitle(FIRST_URL).reference);
    assert!(!actual.availability.unwrap().is_cancelled());
}
#[test]
fn readers_require_exact_session_and_complete_song_or_movie_identity() {
    let owner = SessionTextMetadata::new(identity());
    let reader = owner.reader();
    owner
        .publish_lyrics(stamp(1), &song(), lyrics(None))
        .unwrap();
    owner
        .publish_subtitle(stamp(1), &movie(), subtitle(FIRST_URL))
        .unwrap();
    let foreign = StreamIdentity {
        generation: 2,
        ..identity()
    };
    assert!(matches!(
        LyricsMetadataSource::metadata(&*reader, foreign, &song()),
        Err(ResolverError::InvalidData)
    ));
    assert!(matches!(
        SubtitleMetadataSource::metadata(&*reader, foreign, &movie()),
        Err(ResolverError::InvalidData)
    ));
    for case in 0..3 {
        let mut other = song();
        match case {
            0 => other.id = "different".into(),
            1 => other.title = "different".into(),
            _ => other.artist = None,
        }
        assert!(matches!(
            LyricsMetadataSource::metadata(&*reader, identity(), &other),
            Err(ResolverError::ContentMismatch)
        ));
        let mut other = movie();
        match case {
            0 => other.id = "different".into(),
            1 => other.title = "different".into(),
            _ => other.year = None,
        }
        assert!(matches!(
            SubtitleMetadataSource::metadata(&*reader, identity(), &other),
            Err(ResolverError::ContentMismatch)
        ));
    }
}
#[test]
fn duplicate_or_late_publications_cannot_overwrite_current_or_revive_cleared_metadata() {
    let owner = SessionTextMetadata::new(identity());
    let reader = owner.reader();
    let latest = stamp(5);
    owner.publish_lyrics(latest, &song(), lyrics(None)).unwrap();
    let original = song_metadata(&reader).availability.unwrap();
    for old in [
        latest,
        stamp(4),
        WindowStamp {
            sequence: 6,
            ended_at: latest.ended_at - Duration::from_millis(1),
            ..latest
        },
    ] {
        assert_eq!(
            owner.publish_lyrics(old, &song(), lyrics(Some(TextLanguage::TaiwanTraditional))),
            Err(ResolverError::Stale)
        );
        assert!(!original.is_cancelled());
    }
    owner.clear_lyrics(stamp(7)).unwrap();
    assert!(original.is_cancelled());
    assert_eq!(
        owner.publish_lyrics(stamp(6), &song(), lyrics(None)),
        Err(ResolverError::Stale)
    );
    assert!(matches!(
        LyricsMetadataSource::metadata(&*reader, identity(), &song()),
        Err(ResolverError::NotConfigured)
    ));
}
#[test]
fn identical_verified_metadata_preserves_binding_revision_and_parent_token() {
    let owner = SessionTextMetadata::new(identity());
    let reader = owner.reader();
    owner
        .publish_lyrics(stamp(1), &song(), lyrics(None))
        .unwrap();
    owner
        .publish_subtitle(stamp(1), &movie(), subtitle(FIRST_URL))
        .unwrap();
    let song_token = song_metadata(&reader).availability.unwrap();
    let movie_token = movie_metadata(&reader).availability.unwrap();
    assert!(
        !owner
            .publish_lyrics(stamp(2), &song(), lyrics(None))
            .unwrap()
    );
    assert!(
        !owner
            .publish_subtitle(stamp(2), &movie(), subtitle(FIRST_URL))
            .unwrap()
    );
    assert_eq!(
        LyricsMetadataSource::revision(&*reader, identity(), &song()),
        Some(1)
    );
    assert_eq!(
        SubtitleMetadataSource::revision(&*reader, identity(), &movie()),
        Some(1)
    );
    assert!(song_token.same_scope(&song_metadata(&reader).availability.unwrap()));
    assert!(movie_token.same_scope(&movie_metadata(&reader).availability.unwrap()));
}
#[test]
fn language_album_duration_location_and_edition_changes_retire_the_previous_entry() {
    let owner = SessionTextMetadata::new(identity());
    let reader = owner.reader();
    owner
        .publish_lyrics(stamp(1), &song(), lyrics(None))
        .unwrap();
    let old = song_metadata(&reader).availability.unwrap();
    let mut changed = lyrics(Some(TextLanguage::Source(SpeechLanguage::English)));
    changed.album = None;
    changed.duration_us = Some(121_000_000);
    assert!(owner.publish_lyrics(stamp(2), &song(), changed).unwrap());
    assert!(old.is_cancelled());
    assert_eq!(
        LyricsMetadataSource::revision(&*reader, identity(), &song()),
        Some(2)
    );
    owner
        .publish_subtitle(stamp(1), &movie(), subtitle(FIRST_URL))
        .unwrap();
    let old = movie_metadata(&reader).availability.unwrap();
    assert!(
        owner
            .publish_subtitle(
                stamp(2),
                &movie(),
                subtitle("https://example.invalid/new.srt")
            )
            .unwrap()
    );
    assert!(old.is_cancelled());
    let old = movie_metadata(&reader).availability.unwrap();
    let mut changed = subtitle("https://example.invalid/new.srt");
    changed.reference = None;
    assert!(owner.publish_subtitle(stamp(3), &movie(), changed).unwrap());
    assert!(old.is_cancelled());
    assert_eq!(
        SubtitleMetadataSource::revision(&*reader, identity(), &movie()),
        Some(3)
    );
}
#[test]
fn malformed_or_cross_session_data_is_rejected_without_retiring_valid_metadata() {
    let owner = SessionTextMetadata::new(identity());
    let reader = owner.reader();
    owner
        .publish_lyrics(stamp(1), &song(), lyrics(None))
        .unwrap();
    owner
        .publish_subtitle(stamp(1), &movie(), subtitle(FIRST_URL))
        .unwrap();
    let lyric_token = song_metadata(&reader).availability.unwrap();
    let subtitle_token = movie_metadata(&reader).availability.unwrap();
    for case in 0..6 {
        let mut wrong = lyrics(None);
        match case {
            0 => wrong.identity.generation = 2,
            1 => wrong.content_id = "wrong".into(),
            2 => wrong.album = Some("a".repeat(MAX_METADATA_BYTES + 1).into()),
            3 => wrong.duration_us = Some(0),
            4 => wrong.duration_us = Some(MAX_TRACK_DURATION_US + 1),
            _ => wrong.availability = Some(ResolverCancellation::default()),
        }
        assert!(owner.publish_lyrics(stamp(2), &song(), wrong).is_err());
        assert!(!lyric_token.is_cancelled());
    }
    let mut invalid = subtitle(FIRST_URL);
    invalid.primary = SubtitleTrack::new(
        "https://example.invalid/source.srt",
        SubtitleFormat::Srt,
        TextLanguage::Source(SpeechLanguage::English),
    )
    .unwrap();
    assert_eq!(
        owner.publish_subtitle(stamp(2), &movie(), invalid),
        Err(ResolverError::InvalidData)
    );
    let mut invalid = subtitle(FIRST_URL);
    invalid.identity.generation = 2;
    assert_eq!(
        owner.publish_subtitle(stamp(2), &movie(), invalid),
        Err(ResolverError::ContentMismatch)
    );
    assert!(!subtitle_token.is_cancelled());
}
#[test]
fn replacing_one_content_type_keeps_the_other_and_never_inherits_identification_confidence() {
    let owner = SessionTextMetadata::new(identity());
    let reader = owner.reader();
    owner
        .publish_lyrics(stamp(1), &song(), lyrics(None))
        .unwrap();
    owner
        .publish_subtitle(stamp(1), &movie(), subtitle(FIRST_URL))
        .unwrap();
    let movie_token = movie_metadata(&reader).availability.unwrap();
    for sequence in 2..=128 {
        let mut current = song();
        current.id = format!("synthetic:track:{sequence}").into();
        let mut value = lyrics(None);
        value.content_id = current.id.clone();
        owner
            .publish_lyrics(stamp(sequence), &current, value)
            .unwrap();
        assert!(matches!(
            LyricsMetadataSource::metadata(&*reader, identity(), &song()),
            Err(ResolverError::ContentMismatch)
        ));
        let state = owner.inner.state.lock().unwrap();
        assert!(state.lyrics.as_ref().unwrap().key.id == current.id);
        assert!(state.subtitles.is_some());
    }
    assert!(!movie_token.is_cancelled());
    assert_eq!(song().confidence.permille(), 920);
}
#[test]
fn clear_stop_and_owner_drop_retire_metadata_even_with_readers_retained() {
    let owner = SessionTextMetadata::new(identity());
    let reader = owner.reader();
    owner
        .publish_lyrics(stamp(1), &song(), lyrics(None))
        .unwrap();
    owner
        .publish_subtitle(stamp(1), &movie(), subtitle(FIRST_URL))
        .unwrap();
    let lyric_token = song_metadata(&reader).availability.unwrap();
    let subtitle_token = movie_metadata(&reader).availability.unwrap();
    owner.clear_subtitle(stamp(2)).unwrap();
    assert!(subtitle_token.is_cancelled());
    assert!(!lyric_token.is_cancelled());
    drop(owner);
    assert!(lyric_token.is_cancelled());
    assert!(matches!(
        LyricsMetadataSource::metadata(&*reader, identity(), &song()),
        Err(ResolverError::Cancelled)
    ));
    assert!(matches!(
        SubtitleMetadataSource::metadata(&*reader, identity(), &movie()),
        Err(ResolverError::Cancelled)
    ));
    let owner = SessionTextMetadata::new(identity());
    owner.stop();
    owner.stop();
    assert_eq!(
        owner.publish_lyrics(stamp(1), &song(), lyrics(None)),
        Err(ResolverError::Cancelled)
    );
    assert_eq!(
        owner.clear_subtitle(stamp(1)),
        Err(ResolverError::Cancelled)
    );
}
#[test]
fn explicit_production_configurations_are_read_only_and_do_not_activate_lookup() {
    let owner = SessionTextMetadata::new(identity());
    let lyrics = owner.lyrics_configuration().unwrap();
    assert_eq!(lyrics.disclosure().service, "LRCLIB");
    let remote = ContentDisclosure {
        service: "明確字幕測試位置",
        transport: crate::ui::content::ContentTransport::Remote,
    };
    assert!(owner.subtitle_configuration(remote).unwrap().disclosure() == remote);
    assert!(
        owner
            .subtitle_configuration(ContentDisclosure {
                transport: crate::ui::content::ContentTransport::Local,
                ..remote
            })
            .is_err()
    );
}
#[test]
fn registered_language_can_retry_real_lrc_binding_without_a_cloud_lookup() {
    use crate::{
        app::lyrics::LiveLyrics,
        resolver::candidates::{Resolution, SongState},
        resolver::contracts::ContentType,
        ui::lyrics::UiLyricsState,
    };
    let owner = SessionTextMetadata::new(identity());
    let config = owner.lyrics_configuration().unwrap();
    let mut live = LiveLyrics::new(identity(), Some(&config));
    let resolution = Resolution {
        kind: ContentType::Music,
        song_state: SongState::Confirmed,
        song: Some(Arc::new(song())),
        movies: Vec::new(),
    };
    live.select(stamp(1), &resolution).unwrap();
    assert_eq!(
        live.status().state,
        UiLyricsState::Failed(ResolverError::NotConfigured)
    );
    owner
        .publish_lyrics(stamp(2), &song(), lyrics(None))
        .unwrap();
    live.select(stamp(2), &resolution).unwrap();
    assert_eq!(
        live.status().state,
        UiLyricsState::Failed(ResolverError::UnknownTextLanguage)
    );
    owner
        .publish_lyrics(
            stamp(3),
            &song(),
            lyrics(Some(TextLanguage::Source(SpeechLanguage::English))),
        )
        .unwrap();
    live.select(stamp(3), &resolution).unwrap();
    assert_eq!(live.status().state, UiLyricsState::LookingUp);
    live.stop(); // Never poll the remote lookup task.
}
