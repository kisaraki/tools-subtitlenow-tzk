//! Real text bytes in synthetic containers; static presentation mapping only.
use super::*;
pub(super) fn list(entries: &[(u32, i32, u32)]) -> Vec<u8> {
    let mut data = words(&[0, entries.len() as u32]);
    for (duration, time, rate) in entries {
        data.extend(words(&[*duration, *time as u32, *rate]));
    }
    data
}
pub(super) fn list_v1(entries: &[(u64, i64, u32)]) -> Vec<u8> {
    let mut data = words(&[0x0100_0000, entries.len() as u32]);
    for (duration, time, rate) in entries {
        data.extend(duration.to_be_bytes());
        data.extend(time.to_be_bytes());
        data.extend(rate.to_be_bytes());
    }
    data
}
pub(super) fn edited(
    bytes: &[u8],
    id: u32,
    elst: &[u8],
    track_duration: u32,
    movie_duration: u32,
) -> Vec<u8> {
    let cancel = ResolverCancellation::default();
    let roots = children(bytes, &[b"ftyp", b"mdat", b"moov"], &cancel).unwrap();
    let movie = children_with_tracks(
        get(&roots, b"moov").unwrap(),
        &[b"mvhd", b"trak"],
        true,
        &cancel,
    )
    .unwrap();
    let mut body = Vec::new();
    for entry in movie {
        let mut data = entry.body.to_vec();
        if entry.kind == b"mvhd" {
            data[16..20].copy_from_slice(&movie_duration.to_be_bytes());
        }
        if entry.kind == b"trak" {
            let children = children(&data, &[b"tkhd", b"mdia", b"edts"], &cancel).unwrap();
            if u32_at(get(&children, b"tkhd").unwrap(), 12).unwrap() == id {
                let mut header = get(&children, b"tkhd").unwrap().to_vec();
                header[20..24].copy_from_slice(&track_duration.to_be_bytes());
                data = [
                    atom(b"tkhd", &header),
                    atom(b"mdia", get(&children, b"mdia").unwrap()),
                    atom(b"edts", &atom(b"elst", elst)),
                ]
                .concat();
            }
        }
        body.extend(atom(entry.kind.try_into().unwrap(), &data));
    }
    [
        atom(b"ftyp", get(&roots, b"ftyp").unwrap()),
        atom(b"mdat", get(&roots, b"mdat").unwrap()),
        atom(b"moov", &body),
    ]
    .concat()
}
pub(crate) fn delayed_fixture(language: TextLanguage, delay: u32) -> Vec<u8> {
    let entries = if delay == 0 {
        vec![(2100, 0, 0x10000)]
    } else {
        vec![(delay, -1, 0x10000), (2100, 0, 0x10000)]
    };
    edited(
        &prepared_fixture(language),
        3,
        &list(&entries),
        2100 + delay,
        3000 + delay,
    )
}
pub(crate) fn cropped_fixture(
    language: TextLanguage,
    start: u32,
    duration: u32,
    delay: u32,
) -> Vec<u8> {
    let entries = if delay == 0 {
        vec![(duration, start as i32, 0x10000)]
    } else {
        vec![(delay, -1, 0x10000), (duration, start as i32, 0x10000)]
    };
    edited(
        &prepared_fixture(language),
        3,
        &list(&entries),
        delay + duration,
        5000 + delay,
    )
}
pub(crate) fn cropped_v1_fixture(
    language: TextLanguage,
    start: u32,
    duration: u32,
    delay: u32,
) -> Vec<u8> {
    let active = (u64::from(duration), i64::from(start), 0x10000);
    let entries = if delay == 0 {
        vec![active]
    } else {
        vec![(u64::from(delay), -1, 0x10000), active]
    };
    edited(
        &prepared_fixture(language),
        3,
        &list_v1(&entries),
        delay + duration,
        5000 + delay,
    )
}
fn scan(bytes: &[u8]) -> Result<Mp4SubtitleTracks, MediaError> {
    discover_subtitles(
        &mut Cursor::new(bytes),
        bytes.len() as u64,
        &ResolverCancellation::default(),
    )
}
#[test]
fn mp4_text_edit_identity_and_leading_empty_shift_cues_and_preserve_clear_gap() {
    for delay in [0, 1, 500, 2500] {
        let bytes = delayed_fixture(TextLanguage::SimplifiedChinese, delay);
        assert_eq!(scan(&bytes).unwrap().len(), 1);
        let doc = run(&bytes).unwrap();
        let origin = u64::from(delay) * 1000;
        assert_eq!(doc.cues.len(), 2);
        assert_eq!(doc.cues[0].text.as_ref(), "软件与内存");
        assert_eq!(doc.cues[0].start_us, Some(origin));
        assert_eq!(doc.cues[0].duration_us, Some(700_000));
        assert_eq!(doc.cues[1].start_us, Some(origin + 1_100_000));
        assert_eq!(doc.cues[1].duration_us, Some(1_000_000));
        assert_eq!(doc.identity, identity());
        assert_eq!(
            super::super::super::probe(
                &mut Cursor::new(&bytes),
                bytes.len() as u64,
                &ResolverCancellation::default()
            ),
            Err(MediaError::UnsupportedMedia)
        );
    }
}
#[test]
fn mp4_text_edit_fractional_media_tick_delay_uses_one_absolute_rational_rounding() {
    let mut bytes = fixture();
    // Both the delay and sample endpoint have fractional microseconds. Adding
    // their separately floored values would make the first endpoint 1 us early.
    replace_word(&mut bytes, b"mvhd", 12, 3000);
    change_word(&mut bytes, b"mdhd", 12, 9000);
    let bytes = edited(
        &bytes,
        3,
        &list(&[(1, -1, 0x10000), (700, 0, 0x10000)]),
        701,
        9000,
    );
    let doc = run(&bytes).unwrap();
    assert_eq!(doc.cues[0].start_us, Some(333));
    assert_eq!(doc.cues[0].duration_us, Some(77_778));
    assert_eq!(doc.cues[1].start_us, Some(122_555));
    assert_eq!(doc.cues[1].duration_us, Some(111_111));
}
#[test]
fn mp4_text_edit_unselected_tracks_are_validated_and_each_selected_track_uses_its_own_delay() {
    let languages = [TextLanguage::TaiwanTraditional; 2];
    let base = multi_fixture(&languages);
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
        &list(&[(1500, -1, 0x10000), (2100, 0, 0x10000)]),
        3600,
        5000,
    );
    assert_eq!(scan(&bytes).unwrap().len(), 2);
    for (id, origin) in [(3, 300_000), (4, 1_500_000)] {
        let doc = read_subtitle(
            &mut Cursor::new(&bytes),
            bytes.len() as u64,
            identity(),
            Mp4SubtitleSelection::new(id, TextLanguage::TaiwanTraditional).unwrap(),
            &ResolverCancellation::default(),
        )
        .unwrap();
        assert_eq!(doc.cues[0].start_us, Some(origin));
        assert_eq!(doc.cues[1].start_us, Some(origin + 1_100_000));
    }
    let bad = edited(&first, 4, &list(&[(2100, 1, 0x10000)]), 2100, 5000);
    assert!(scan(&bad).is_err());
    assert!(run(&bad).is_err()); // Selected first, bad edit in last.
}
#[test]
fn mp4_text_edit_rejects_out_of_range_rate_dwell_repeat_version_and_length() {
    let base = prepared_fixture(TextLanguage::TaiwanTraditional);
    let mut cases = vec![
        list(&[]),
        list(&[(2100, 1, 0x10000)]),
        list(&[(2100, -2, 0x10000)]),
        list(&[(2100, 0, 0)]),
        list(&[(2100, 0, 0x20000)]),
        list(&[(2100, 0, 0x10001)]),
        list(&[(2100, 0, 0x10000), (500, -1, 0x10000)]),
        list(&[(0, -1, 0x10000), (2100, 0, 0x10000)]),
        list(&[(500, -1, 0x10000), (2100, 0, 0x10000), (1, 0, 0x10000)]),
        list(&[(2099, 0, 0x10000)]),
        list(&[(2101, 0, 0x10000)]),
    ];
    for index in [0, 3] {
        let mut data = list(&[(2100, 0, 0x10000)]);
        data[index] = 1;
        cases.push(data);
    }
    let valid = list(&[(500, -1, 0x10000), (2100, 0, 0x10000)]);
    for end in 0..valid.len() {
        cases.push(valid[..end].to_vec());
    }
    let mut trailing = valid.clone();
    trailing.push(0);
    cases.push(trailing);
    for elst in cases {
        let bytes = edited(&base, 3, &elst, 2100, 5000);
        assert!(scan(&bytes).is_err());
        assert!(run(&bytes).is_err());
    }
    for (track, movie) in [(2599, 5000), (2601, 5000), (2600, 2599)] {
        let bytes = edited(&base, 3, &valid, track, movie);
        assert!(run(&bytes).is_err());
    }
    // This explicit shorter duration now describes a real tail crop.
    let mut fractional = base.clone();
    replace_word(&mut fractional, b"mvhd", 12, 999);
    let cropped = run(&edited(
        &fractional,
        3,
        &list(&[(2097, 0, 0x10000)]),
        2097,
        5000,
    ))
    .unwrap();
    assert_eq!(cropped.cues[1].start_us, Some(1_100_000));
    assert_eq!(cropped.cues[1].duration_us, Some(999_099));
}
#[test]
fn mp4_text_edit_audio_video_edits_and_overlong_presentation_remain_unsupported() {
    let base = prepared_fixture(TextLanguage::TaiwanTraditional);
    for id in [1, 2] {
        let bytes = edited(&base, id, &list(&[(1, 0, 0x10000)]), 1, 5000);
        assert!(scan(&bytes).is_err());
        assert!(run(&bytes).is_err());
    }
    let bytes = edited(
        &base,
        3,
        &list(&[(86_400_000, -1, 0x10000), (2100, 0, 0x10000)]),
        86_402_100,
        86_402_100,
    );
    assert!(scan(&bytes).is_err());
    assert!(run(&bytes).is_err());
}
#[test]
fn mp4_text_edit_every_file_truncation_and_cancel_return_no_partial_document() {
    let bytes = delayed_fixture(TextLanguage::TaiwanTraditional, 500);
    for end in 0..bytes.len() {
        assert!(run(&bytes[..end]).is_err());
    }
    let cancel = ResolverCancellation::default();
    cancel.cancel();
    assert_eq!(
        discover_subtitles(&mut Cursor::new(&bytes), bytes.len() as u64, &cancel),
        Err(MediaError::Cancelled)
    );
}
