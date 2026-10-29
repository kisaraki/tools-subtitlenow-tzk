//! v1 elst within the unchanged bounded, v0-header MP4 text container gate.
use super::edits::{edited, list, list_v1};
use super::*;

fn scan(bytes: &[u8]) -> Result<Mp4SubtitleTracks, MediaError> {
    discover_subtitles(
        &mut Cursor::new(bytes),
        bytes.len() as u64,
        &ResolverCancellation::default(),
    )
}
#[test]
fn mp4_text_v1_container_matches_v0_full_delay_crop_and_clear_boundaries() {
    for (start, duration) in [
        (0, 2100),
        (200, 1400),
        (0, 700),
        (1100, 1000),
        (1100, 1),
        (700, 400),
    ] {
        for delay in [0, 1, 250, 2500] {
            let old = cropped_fixture(TextLanguage::SimplifiedChinese, start, duration, delay);
            let new = cropped_v1_fixture(TextLanguage::SimplifiedChinese, start, duration, delay);
            assert_eq!(scan(&old).unwrap(), scan(&new).unwrap());
            if start == 700 {
                assert!(matches!(run(&new), Err(MediaError::InvalidSubtitle)));
                continue;
            }
            let old = run(&old).unwrap();
            let new = run(&new).unwrap();
            assert_eq!(old.identity, new.identity);
            assert_eq!(old.content_id, new.content_id);
            assert_eq!(old.cues.len(), new.cues.len());
            for (a, b) in old.cues.iter().zip(&new.cues) {
                assert_eq!(a.text, b.text);
                assert_eq!(a.start_us, b.start_us);
                assert_eq!(a.duration_us, b.duration_us);
            }
        }
    }
}
#[test]
fn mp4_text_v1_container_accepts_positive_media_start_above_signed_32_bit() {
    let mut base = prepared_fixture(TextLanguage::TaiwanTraditional);
    // v0 mdhd can represent this positive u32 duration. v1 elst can refer to
    // its tail without treating bit 31 as the signed v0 negative media_time.
    change_word(&mut base, b"mdhd", 12, 1_000_000);
    change_word(&mut base, b"mdhd", 16, 2_201_000_400);
    change_word(&mut base, b"stts", 12, 2_200_000_000);
    change_word(&mut base, b"stts", 20, 400);
    change_word(&mut base, b"stts", 28, 1_000_000);
    let bytes = edited(
        &base,
        3,
        &list_v1(&[(250, -1, 0x10000), (500, 2_200_000_400, 0x10000)]),
        750,
        5000,
    );
    assert!(scan(&bytes).is_ok());
    let doc = run(&bytes).unwrap();
    assert_eq!(doc.cues.len(), 1);
    assert_eq!(doc.cues[0].text.as_ref(), "鼠标与服务器");
    assert_eq!(doc.cues[0].start_us, Some(250_000));
    assert_eq!(doc.cues[0].duration_us, Some(500_000));
    let negative_v0 = edited(
        &base,
        3,
        &list(&[(500, 2_200_000_400u32 as i32, 0x10000)]),
        500,
        5000,
    );
    assert!(scan(&negative_v0).is_err());
}
#[test]
fn mp4_text_v1_container_rejects_high_word_aliases_bad_shapes_and_unsupported_headers() {
    let base = prepared_fixture(TextLanguage::TaiwanTraditional);
    let high = 1u64 << 32;
    for entries in [
        vec![(500, (high + 1100) as i64, 0x10000)],
        vec![(high + 500, 1100, 0x10000)],
        vec![(high + 250, -1, 0x10000), (500, 1100, 0x10000)],
        vec![(u64::MAX, -1, 0x10000), (500, 1100, 0x10000)],
        vec![(500, i64::MIN, 0x10000)],
        vec![(500, -2, 0x10000)],
        vec![(250, u32::MAX as i64, 0x10000), (500, 1100, 0x10000)],
    ] {
        let bytes = edited(&base, 3, &list_v1(&entries), 750, 5000);
        assert!(scan(&bytes).is_err());
        assert!(run(&bytes).is_err());
    }
    let valid = list_v1(&[(250, -1, 0x10000), (500, 1100, 0x10000)]);
    for (index, value) in [(0, 0), (0, 2), (0, 255), (1, 1), (2, 1), (3, 1), (7, 3)] {
        let mut table = valid.clone();
        table[index] = value;
        let bytes = edited(&base, 3, &table, 750, 5000);
        assert!(scan(&bytes).is_err());
        assert!(run(&bytes).is_err());
    }
    for id in [1, 2] {
        // Audio/video edit gates remain independent.
        let bytes = edited(&base, id, &valid, 750, 5000);
        assert!(scan(&bytes).is_err());
        assert!(run(&bytes).is_err());
    }
    for name in [b"mdhd", b"tkhd", b"mvhd"] {
        let mut bytes = cropped_v1_fixture(TextLanguage::TaiwanTraditional, 1100, 500, 250);
        let at = if name == b"mvhd" {
            body(&bytes, name)
        } else {
            last_body(&bytes, name)
        };
        bytes[at] = 1;
        assert!(scan(&bytes).is_err());
        assert!(run(&bytes).is_err());
    }
}
#[test]
fn mp4_text_v1_container_mixed_versions_remain_independent_and_unselected_errors_reject() {
    let base = multi_fixture(&[TextLanguage::TaiwanTraditional; 2]);
    let first = edited(
        &base,
        3,
        &list(&[(300, -1, 0x10000), (2100, 0, 0x10000)]),
        2400,
        5000,
    );
    let bytes = edited(
        &first,
        4,
        &list_v1(&[(250, -1, 0x10000), (500, 1100, 0x10000)]),
        750,
        5000,
    );
    assert_eq!(scan(&bytes).unwrap().len(), 2);
    assert_eq!(run(&bytes).unwrap().cues[0].start_us, Some(300000));
    let selected = Mp4SubtitleSelection::new(4, TextLanguage::TaiwanTraditional).unwrap();
    let doc = read_subtitle(
        &mut Cursor::new(&bytes),
        bytes.len() as u64,
        identity(),
        selected,
        &ResolverCancellation::default(),
    )
    .unwrap();
    assert_eq!(doc.cues.len(), 1);
    assert!(doc.cues[0].text.contains('4'));
    assert_eq!(doc.cues[0].start_us, Some(250000));
    let bad = edited(
        &first,
        4,
        &list_v1(&[(500, (1i64 << 32) + 1100, 0x10000)]),
        500,
        5000,
    );
    assert!(scan(&bad).is_err());
    assert!(run(&bad).is_err());
}
#[test]
fn mp4_text_v1_container_validates_hidden_text_truncations_and_mid_read_cancellation() {
    let bytes = cropped_v1_fixture(TextLanguage::TaiwanTraditional, 1100, 500, 250);
    for end in 0..bytes.len() {
        assert!(run(&bytes[..end]).is_err());
    }
    let mut bad = bytes.clone();
    let offset = u32_at(&bad, last_body(&bad, b"stco") + 8).unwrap() as usize;
    bad[offset..offset + 2].copy_from_slice(&u16::MAX.to_be_bytes());
    assert!(scan(&bad).is_ok());
    assert!(matches!(run(&bad), Err(MediaError::InvalidSubtitle)));
    let mut reader = counted(bytes, true);
    let len = reader.inner.get_ref().len() as u64;
    let cancel = reader.cancel.clone();
    assert!(matches!(
        read_subtitle(&mut reader, len, identity(), selection(), &cancel),
        Err(MediaError::Cancelled)
    ));
    assert_eq!(reader.text_reads, 1);
}
