//! Static one-interval edits; every original selected sample is still validated.
use super::edits::{edited, list};
use super::*;
fn read(bytes: &[u8]) -> Result<ExistingText, MediaError> {
    run(bytes)
}
fn scan(bytes: &[u8]) -> Result<Mp4SubtitleTracks, MediaError> {
    discover_subtitles(
        &mut Cursor::new(bytes),
        bytes.len() as u64,
        &ResolverCancellation::default(),
    )
}
#[test]
fn mp4_text_crop_head_and_tail_intersections_preserve_complete_text_and_clear_gap() {
    let bytes = cropped_fixture(TextLanguage::SimplifiedChinese, 200, 1400, 500);
    assert_eq!(scan(&bytes).unwrap().len(), 1);
    let doc = read(&bytes).unwrap();
    assert_eq!(doc.cues.len(), 2);
    assert_eq!(doc.cues[0].text.as_ref(), "软件与内存");
    assert_eq!(doc.cues[1].text.as_ref(), "鼠标与服务器");
    assert_eq!(doc.cues[0].start_us, Some(500_000));
    assert_eq!(doc.cues[0].duration_us, Some(500_000));
    assert_eq!(doc.cues[1].start_us, Some(1_400_000));
    assert_eq!(doc.cues[1].duration_us, Some(500_000));
    assert_eq!(doc.identity, identity());
}
#[test]
fn mp4_text_crop_exact_boundaries_drop_outside_cues_and_clear_only_is_not_ready() {
    for (start, duration, expected) in [
        (0, 700, "软件与内存"),
        (1100, 1000, "鼠标与服务器"),
        (1100, 1, "鼠标与服务器"),
    ] {
        let bytes = cropped_fixture(TextLanguage::TaiwanTraditional, start, duration, 0);
        let doc = read(&bytes).unwrap();
        assert_eq!(doc.cues.len(), 1);
        assert_eq!(doc.cues[0].text.as_ref(), expected);
        assert_eq!(doc.cues[0].start_us, Some(0));
        assert_eq!(doc.cues[0].duration_us, Some(u64::from(duration) * 1000));
    }
    let bytes = cropped_fixture(TextLanguage::TaiwanTraditional, 700, 400, 0);
    assert!(scan(&bytes).is_ok()); // Structure only, not a body/Ready promise.
    assert!(matches!(read(&bytes), Err(MediaError::InvalidSubtitle)));
    let bytes = cropped_fixture(TextLanguage::TaiwanTraditional, 2100, 1, 0);
    assert!(scan(&bytes).is_err());
}
#[test]
fn mp4_text_crop_fractional_end_and_nonzero_media_start_round_absolute_overlap_once() {
    let mut base = prepared_fixture(TextLanguage::TaiwanTraditional);
    replace_word(&mut base, b"mvhd", 12, 7);
    change_word(&mut base, b"mdhd", 12, 9000);
    let bytes = edited(
        &base,
        3,
        &list(&[(1, -1, 0x10000), (1, 100, 0x10000)]),
        2,
        7,
    );
    let doc = read(&bytes).unwrap();
    assert_eq!(doc.cues[0].start_us, Some(142_857));
    assert_eq!(doc.cues[0].duration_us, Some(66_666));
    assert_eq!(doc.cues[1].start_us, Some(253_968));
    assert_eq!(doc.cues[1].duration_us, Some(31_746));
}
#[test]
fn mp4_text_crop_malformed_body_outside_the_edit_still_invalidates_the_selected_track() {
    for (start, duration, first) in [(1100, 1000, true), (0, 700, false)] {
        let mut bytes = cropped_fixture(TextLanguage::TaiwanTraditional, start, duration, 0);
        let table = last_body(&bytes, b"stco");
        let offset = if first {
            u32_at(&bytes, table + 8).unwrap() as usize
        } else {
            u32_at(&bytes, table + 12).unwrap() as usize + 2
        }; // after empty clear sample
        bytes[offset..offset + 2].copy_from_slice(&u16::MAX.to_be_bytes());
        assert!(scan(&bytes).is_ok());
        assert!(matches!(read(&bytes), Err(MediaError::InvalidSubtitle)));
    }
}
#[test]
fn mp4_text_crop_multiple_tracks_keep_distinct_ranges_and_invalid_unselected_edit_rejects() {
    let base = multi_fixture(&[TextLanguage::TaiwanTraditional; 2]);
    let bytes = edited(
        &base,
        4,
        &list(&[(250, -1, 0x10000), (500, 1100, 0x10000)]),
        750,
        5000,
    );
    assert_eq!(scan(&bytes).unwrap().len(), 2);
    let first = read(&bytes).unwrap();
    assert_eq!(first.cues.len(), 2);
    assert_eq!(first.cues[0].start_us, Some(0));
    let last = read_subtitle(
        &mut Cursor::new(&bytes),
        bytes.len() as u64,
        identity(),
        Mp4SubtitleSelection::new(4, TextLanguage::TaiwanTraditional).unwrap(),
        &ResolverCancellation::default(),
    )
    .unwrap();
    assert_eq!(last.cues.len(), 1);
    assert!(last.cues[0].text.contains("4"));
    assert_eq!(last.cues[0].start_us, Some(250_000));
    assert_eq!(last.cues[0].duration_us, Some(500_000));
    let bad = edited(&base, 4, &list(&[(1000, 2000, 0x10000)]), 1000, 5000);
    assert!(scan(&bad).is_err());
    assert!(read(&bad).is_err());
}
#[test]
fn mp4_text_crop_truncation_and_cancel_while_reading_cropped_out_payload_return_no_document() {
    let bytes = cropped_fixture(TextLanguage::TaiwanTraditional, 1100, 1000, 250);
    for end in 0..bytes.len() {
        assert!(read(&bytes[..end]).is_err());
    }
    let mut reader = counted(bytes, true);
    let len = reader.inner.get_ref().len() as u64;
    let cancel = reader.cancel.clone();
    assert!(matches!(
        read_subtitle(&mut reader, len, identity(), selection(), &cancel),
        Err(MediaError::Cancelled)
    ));
    assert!(cancel.is_cancelled());
    assert_eq!(reader.text_reads, 1);
}
