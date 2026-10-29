//! Distinct text tracks with real byte references, metadata-only discovery.
use super::*;
fn choices() -> [TextLanguage; MAX_MP4_SUBTITLE_TRACKS] {
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
fn scan(bytes: &[u8]) -> Result<Mp4SubtitleTracks, MediaError> {
    discover_subtitles(
        &mut Cursor::new(bytes),
        bytes.len() as u64,
        &ResolverCancellation::default(),
    )
}
#[test]
fn mp4_multitrack_discovery_is_bounded_ordered_and_never_selects_a_preferred_chinese_track() {
    let bytes = multi_fixture(&choices());
    let mut reader = counted(bytes.clone(), false);
    let catalog = discover_subtitles(
        &mut reader,
        bytes.len() as u64,
        &ResolverCancellation::default(),
    )
    .unwrap();
    assert_eq!(catalog.len(), 8);
    assert_eq!(reader.text_reads, 0);
    for (index, language) in choices().into_iter().enumerate() {
        let hint = match language {
            TextLanguage::Source(source) => Mp4SubtitleLanguage::Source(source),
            _ => Mp4SubtitleLanguage::Chinese,
        };
        assert_eq!(
            catalog.get(index),
            Some(Mp4SubtitleTrack {
                track_id: index as u32 + 3,
                language: hint
            })
        );
    }
    assert_eq!(catalog.get(8), None);
    assert_eq!(catalog.get(usize::MAX), None);
    assert_eq!(
        discover_subtitle(
            &mut Cursor::new(&bytes),
            bytes.len() as u64,
            &ResolverCancellation::default()
        ),
        Err(MediaError::AmbiguousSubtitle)
    );
    assert_eq!(
        scan(&multi_fixture(&[TextLanguage::SimplifiedChinese; 9])),
        Err(MediaError::TooLarge)
    );
    assert!(scan(&multi_fixture(&[])).is_err());
    assert_eq!(
        super::super::super::probe(
            &mut Cursor::new(&bytes),
            bytes.len() as u64,
            &ResolverCancellation::default()
        ),
        Err(MediaError::UnsupportedMedia)
    );
}
#[test]
fn mp4_multitrack_explicit_selection_reads_only_its_text_and_preserves_timing() {
    let languages = choices();
    let bytes = multi_fixture(&languages);
    for (index, language) in languages.into_iter().enumerate() {
        let id = index as u32 + 3;
        let document = read_subtitle(
            &mut Cursor::new(&bytes),
            bytes.len() as u64,
            identity(),
            Mp4SubtitleSelection::new(id, language).unwrap(),
            &ResolverCancellation::default(),
        )
        .unwrap();
        assert_eq!(document.cues.len(), 2);
        assert!(
            document
                .cues
                .iter()
                .all(|cue| cue.text.contains(&format!("{id}")))
        );
        assert_eq!(document.cues[0].start_us, Some(0));
        assert_eq!(document.cues[1].start_us, Some(1_100_000));
        assert_eq!(document.cues[1].duration_us, Some(1_000_000));
        assert!(document.language == language);
        assert_eq!(document.identity, identity());
    }
    for id in [1, 2, 11, u32::MAX] {
        assert!(
            read_subtitle(
                &mut Cursor::new(&bytes),
                bytes.len() as u64,
                identity(),
                Mp4SubtitleSelection::new(id, TextLanguage::TaiwanTraditional).unwrap(),
                &ResolverCancellation::default()
            )
            .is_err()
        );
    }
    assert!(
        read_subtitle(
            &mut Cursor::new(&bytes),
            bytes.len() as u64,
            identity(),
            Mp4SubtitleSelection::new(6, TextLanguage::TaiwanTraditional).unwrap(),
            &ResolverCancellation::default()
        )
        .is_err()
    );
}
#[test]
fn mp4_multitrack_unselected_tables_duplicates_and_cross_track_aliases_still_reject() {
    let base = multi_fixture(&[TextLanguage::TaiwanTraditional; 3]);
    for (kind, offset, value) in [
        (b"tkhd", 12, 3),
        (b"stco", 8, 0),
        (b"stsz", 8, MAX_TX3G_SAMPLES as u32 + 1),
        (b"mdhd", 16, 1),
    ] {
        let mut bytes = base.clone();
        change_word(&mut bytes, kind, offset, value);
        assert!(scan(&bytes).is_err());
        assert!(run(&bytes).is_err()); // Selected first track, error in last.
    }
    let mut bytes = base.clone();
    let offsets: Vec<_> = bytes
        .windows(4)
        .enumerate()
        .filter_map(|(i, b)| (b == b"stco").then_some(i + 4))
        .collect();
    let first_text = u32_at(&bytes, offsets[2] + 8).unwrap();
    change_word(&mut bytes, b"stco", 8, first_text);
    assert!(scan(&bytes).is_err());
    assert!(run(&bytes).is_err());
}
#[test]
fn mp4_multitrack_unselected_payload_is_not_decoded_or_returned() {
    let mut bytes = multi_fixture(&[TextLanguage::TaiwanTraditional; 2]);
    let offset = u32_at(&bytes, last_body(&bytes, b"stco") + 8).unwrap() as usize;
    bytes[offset..offset + 2].copy_from_slice(&u16::MAX.to_be_bytes());
    assert_eq!(scan(&bytes).unwrap().len(), 2);
    assert_eq!(run(&bytes).unwrap().cues.len(), 2); // First intact track.
    assert!(
        read_subtitle(
            &mut Cursor::new(&bytes),
            bytes.len() as u64,
            identity(),
            Mp4SubtitleSelection::new(4, TextLanguage::TaiwanTraditional).unwrap(),
            &ResolverCancellation::default()
        )
        .is_err()
    );
}
#[test]
fn mp4_multitrack_every_truncation_and_cancel_reject_without_partial_catalog() {
    let bytes = multi_fixture(&[TextLanguage::TaiwanTraditional; 2]);
    for end in 0..bytes.len() {
        assert!(scan(&bytes[..end]).is_err());
    }
    let cancel = ResolverCancellation::default();
    cancel.cancel();
    assert_eq!(
        discover_subtitles(&mut Cursor::new(&bytes), bytes.len() as u64, &cancel),
        Err(MediaError::Cancelled)
    );
}
