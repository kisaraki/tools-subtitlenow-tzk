use super::*;
use std::io::{self, Cursor, SeekFrom};

pub(super) fn atom(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut bytes = ((body.len() + 8) as u32).to_be_bytes().to_vec();
    bytes.extend_from_slice(kind);
    bytes.extend_from_slice(body);
    bytes
}
fn descriptor_bytes(tag: u8, body: &[u8]) -> Vec<u8> {
    assert!(body.len() < 128);
    [vec![tag, body.len() as u8], body.to_vec()].concat()
}
pub(super) fn words(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|x| x.to_be_bytes()).collect()
}
pub(super) fn fixture() -> Vec<u8> {
    let ftyp = atom(b"ftyp", b"mp42\0\0\0\0isommp41");
    let mdat = atom(b"mdat", &[1; 8]); // Gate fixture only, not playable AAC.
    let offset = (ftyp.len() + 8) as u32;
    let asc = descriptor_bytes(5, &[0x11, 0x90]);
    let mut decoder = vec![0x40, 0x15];
    decoder.extend_from_slice(&[0; 11]);
    decoder.extend(asc);
    let es = [
        vec![0, 1, 0],
        descriptor_bytes(4, &decoder),
        descriptor_bytes(6, &[2]),
    ]
    .concat();
    let esds = atom(b"esds", &[vec![0; 4], descriptor_bytes(3, &es)].concat());
    let mut audio = vec![0; 28];
    audio[7] = 1;
    audio[17] = 2;
    audio[19] = 16;
    audio[24..28].copy_from_slice(&(48_000u32 << 16).to_be_bytes());
    audio.extend(esds);
    let stsd = atom(b"stsd", &[words(&[0, 1]), atom(b"mp4a", &audio)].concat());
    let stbl = atom(
        b"stbl",
        &[
            stsd,
            atom(b"stts", &words(&[0, 1, 2, 1024])),
            atom(b"stsc", &words(&[0, 1, 1, 2, 1])),
            atom(b"stsz", &words(&[0, 0, 2, 4, 4])),
            atom(b"stco", &words(&[0, 1, offset])),
        ]
        .concat(),
    );
    let dref = atom(
        b"dref",
        &[words(&[0, 1]), atom(b"url ", &[0, 0, 0, 1])].concat(),
    );
    let minf = atom(
        b"minf",
        &[atom(b"smhd", &[0; 8]), atom(b"dinf", &dref), stbl].concat(),
    );
    let mut mdhd = vec![0; 24];
    mdhd[12..16].copy_from_slice(&48_000u32.to_be_bytes());
    let mut hdlr = vec![0; 24];
    hdlr[8..12].copy_from_slice(b"soun");
    let mdia = atom(
        b"mdia",
        &[atom(b"mdhd", &mdhd), atom(b"hdlr", &hdlr), minf].concat(),
    );
    let mut tkhd = vec![0; 84];
    tkhd[15] = 1;
    let trak = atom(b"trak", &[atom(b"tkhd", &tkhd), mdia].concat());
    let mut mvhd = vec![0; 100];
    mvhd[15] = 1;
    [
        ftyp,
        mdat,
        atom(b"moov", &[atom(b"mvhd", &mvhd), trak].concat()),
    ]
    .concat()
}
fn run(bytes: &[u8]) -> Result<super::super::LocalAudioFormat, MediaError> {
    super::super::probe(
        &mut Cursor::new(bytes),
        bytes.len() as u64,
        &ResolverCancellation::default(),
    )
}
pub(super) fn body(bytes: &[u8], kind: &[u8; 4]) -> usize {
    bytes.windows(4).position(|w| w == kind).unwrap() + 4
}
pub(super) fn replace_word(bytes: &mut [u8], kind: &[u8; 4], position: usize, value: u32) {
    let offset = body(bytes, kind) + position;
    bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

pub(super) fn replace_atom(bytes: &[u8], path: &[&[u8; 4]], replacement: &[u8]) -> Vec<u8> {
    let mut result = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        let size = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
        let kind: &[u8; 4] = bytes[offset + 4..offset + 8].try_into().unwrap();
        let current = &bytes[offset..offset + size];
        if kind == path[0] {
            if path.len() == 1 {
                result.extend_from_slice(replacement);
            } else {
                result.extend(atom(
                    kind,
                    &replace_atom(&current[8..], &path[1..], replacement),
                ));
            }
        } else {
            result.extend_from_slice(current);
        }
        offset += size;
    }
    result
}

// Structural gate fixture only: payload is deliberately not a playable codec.
// Genuine native H.264/AAC and Ready evidence lives in Windows/app tests.
pub(super) fn video_fixture() -> Vec<u8> {
    let bytes = fixture();
    let roots = children(
        &bytes,
        &[b"ftyp", b"mdat", b"moov"],
        &ResolverCancellation::default(),
    )
    .unwrap();
    let movie = children(
        get(&roots, b"moov").unwrap(),
        &[b"mvhd", b"trak"],
        &ResolverCancellation::default(),
    )
    .unwrap();
    let audio = atom(b"trak", get(&movie, b"trak").unwrap());
    let mut video = audio.clone();
    let mut handler = vec![0; 24];
    handler[8..12].copy_from_slice(b"vide");
    video = replace_atom(
        &video,
        &[b"trak", b"mdia", b"hdlr"],
        &atom(b"hdlr", &handler),
    );
    video = replace_atom(
        &video,
        &[b"trak", b"mdia", b"minf", b"smhd"],
        &atom(b"vmhd", &words(&[1, 0, 0])),
    );
    replace_word(&mut video, b"tkhd", 12, 2);
    let mut entry = vec![0; 78];
    entry[7] = 1;
    entry[25] = 128;
    entry[27] = 96;
    entry[41] = 1;
    entry[75] = 24;
    entry[76..78].fill(255);
    entry.extend(atom(
        b"avcC",
        &[1, 66, 0, 10, 255, 225, 0, 4, 103, 66, 0, 10, 1, 0, 1, 104],
    ));
    video = replace_atom(
        &video,
        &[b"trak", b"mdia", b"minf", b"stbl", b"stsd"],
        &atom(b"stsd", &[words(&[0, 1]), atom(b"avc1", &entry)].concat()),
    );
    let audio_start = u32_at(&bytes, body(&bytes, b"stco") + 8).unwrap();
    replace_word(&mut video, b"stco", 8, audio_start + 8);
    [
        atom(b"ftyp", get(&roots, b"ftyp").unwrap()),
        atom(b"mdat", &[1; 16]),
        atom(
            b"moov",
            &[atom(b"mvhd", get(&movie, b"mvhd").unwrap()), audio, video].concat(),
        ),
    ]
    .concat()
}
fn last_body(bytes: &[u8], kind: &[u8; 4]) -> usize {
    bytes.windows(4).rposition(|value| value == kind).unwrap() + 4
}
fn last_word(bytes: &mut [u8], kind: &[u8; 4], offset: usize, value: u32) {
    let start = last_body(bytes, kind) + offset;
    bytes[start..start + 4].copy_from_slice(&value.to_be_bytes());
}

fn with_video_table(bytes: &[u8], extra: &[u8]) -> Vec<u8> {
    let start = last_body(bytes, b"stbl");
    let size = u32_at(bytes, start - 8).unwrap() as usize;
    let replacement = atom(
        b"stbl",
        &[bytes[start..start + size - 8].to_vec(), extra.to_vec()].concat(),
    );
    let track_start = last_body(bytes, b"trak") - 8;
    let track_size = u32_at(bytes, track_start).unwrap() as usize;
    let track = replace_atom(
        &bytes[track_start..track_start + track_size],
        &[b"trak", b"mdia", b"minf", b"stbl"],
        &replacement,
    );
    let mut changed = bytes.to_vec();
    changed.splice(track_start..track_start + track_size, track);
    let movie = body(&changed, b"moov") - 8;
    let size = (changed.len() - movie) as u32;
    changed[movie..movie + 4].copy_from_slice(&size.to_be_bytes());
    changed
}

#[test]
fn mp4_video_composition_tables_accept_unsigned_and_signed_records() {
    let bytes = video_fixture();
    for (values, valid) in [
        (vec![0, 1, 2, 100], true),
        (vec![0x01000000, 2, 1, 1024, 1, (-1024i32) as u32], true),
        (vec![0, 2, 1, 0, 1, u32::MAX], true),
        (vec![0x02000000, 1, 2, 0], false),
        (vec![1, 1, 2, 0], false),
        (vec![0x01000001, 1, 2, 0], false),
        (vec![0, 0], false),
        (vec![0, 1, 0, 0], false),
        (vec![0, 1, 1, 0], false),
        (vec![0, 1, 3, 0], false),
        (vec![0, u32::MAX], false),
        (vec![0, 1, 2, 0, 0], false),
    ] {
        let changed = with_video_table(&bytes, &atom(b"ctts", &words(&values)));
        assert_eq!(run(&changed).is_ok(), valid);
    }
}

#[test]
fn mp4_video_composition_tables_keep_all_reference_extent_and_cancellation_checks() {
    let bytes = with_video_table(
        &video_fixture(),
        &atom(
            b"ctts",
            &words(&[0x01000000, 2, 1, 1024, 1, (-1024i32) as u32]),
        ),
    );
    for (kind, offset, value) in [(b"url ", 0, 0), (b"stco", 8, 0), (b"stsz", 12, 0)] {
        let mut changed = bytes.clone();
        last_word(&mut changed, kind, offset, value);
        assert_eq!(run(&changed), Err(MediaError::UnsupportedMedia));
    }
    let start = body(&bytes, b"ctts");
    for index in start..start + 24 {
        let mut changed = bytes.clone();
        changed[index] ^= 255;
        assert!(std::panic::catch_unwind(|| run(&changed)).is_ok());
    }
    let cancel = ResolverCancellation::default();
    cancel.cancel();
    assert_eq!(
        super::super::probe(&mut Cursor::new(&bytes), bytes.len() as u64, &cancel),
        Err(MediaError::Cancelled)
    );
}

#[test]
fn mp4_video_two_tracks_accept_adjacent_and_interleaved_ranges() {
    assert_eq!(
        run(&video_fixture()),
        Ok(super::super::LocalAudioFormat::Mp4)
    );
    let mut bytes = video_fixture();
    // One four-byte sample per chunk, alternating audio and video. No alias.
    let start = u32_at(&bytes, body(&bytes, b"stco") + 8).unwrap();
    bytes = replace_atom(
        &bytes,
        &[b"moov", b"trak", b"mdia", b"minf", b"stbl", b"stsc"],
        &atom(b"stsc", &words(&[0, 1, 1, 1, 1])),
    );
    bytes = replace_atom(
        &bytes,
        &[b"moov", b"trak", b"mdia", b"minf", b"stbl", b"stco"],
        &atom(b"stco", &words(&[0, 2, start, start + 8])),
    );
    last_word(&mut bytes, b"stco", 8, start + 4);
    last_word(&mut bytes, b"stco", 12, start + 12);
    assert_eq!(run(&bytes), Ok(super::super::LocalAudioFormat::Mp4));
}

#[test]
fn mp4_video_rejects_cross_track_alias_and_mdat_escape() {
    for displacement in [0, 4, 12, 16, u32::MAX] {
        let mut bytes = video_fixture();
        let start = u32_at(&bytes, body(&bytes, b"stco") + 8).unwrap();
        last_word(&mut bytes, b"stco", 8, start.saturating_add(displacement));
        assert_eq!(run(&bytes), Err(MediaError::UnsupportedMedia));
    }
}

#[test]
fn mp4_video_rejects_external_refs_duplicate_ids_and_other_handlers() {
    for (kind, offset, value) in [
        (b"url ", 0, 0),
        (b"tkhd", 12, 1),
        (b"hdlr", 8, u32::from_be_bytes(*b"soun")),
        (b"hdlr", 8, u32::from_be_bytes(*b"text")),
        (b"vmhd", 0, 0),
    ] {
        let mut bytes = video_fixture();
        last_word(&mut bytes, kind, offset, value);
        assert_eq!(run(&bytes), Err(MediaError::UnsupportedMedia));
    }
    let mut bytes = video_fixture();
    let entry = body(&bytes, b"avc1");
    bytes[entry + 7] = 2;
    assert_eq!(run(&bytes), Err(MediaError::UnsupportedMedia));
}

#[test]
fn mp4_video_rejects_protection_profiles_and_bad_parameter_sets() {
    let mut protected = video_fixture();
    let entry = body(&protected, b"avc1");
    protected[entry - 4..entry].copy_from_slice(b"encv");
    assert_eq!(run(&protected), Err(MediaError::UnsupportedMedia));
    for (offset, value) in [
        (0, 0),
        (1, 77),
        (4, 254),
        (5, 224),
        (8, 0x87),
        (9, 77),
        (12, 0),
        (14, 0),
        (15, 103),
    ] {
        let mut bytes = video_fixture();
        let config = body(&bytes, b"avcC");
        bytes[config + offset] = value;
        assert_eq!(run(&bytes), Err(MediaError::UnsupportedMedia));
    }
}

#[test]
fn mp4_video_rejects_dimensions_and_sync_sample_mismatch() {
    for dimension in [0u16, 8193] {
        let mut bytes = video_fixture();
        let entry = body(&bytes, b"avc1");
        bytes[entry + 24..entry + 26].copy_from_slice(&dimension.to_be_bytes());
        assert_eq!(run(&bytes), Err(MediaError::UnsupportedMedia));
    }
    let bytes = video_fixture();
    let stbl_start = last_body(&bytes, b"stbl");
    let size = u32_at(&bytes, stbl_start - 8).unwrap() as usize;
    for indices in [vec![1, 2], vec![0], vec![3], vec![2, 1], vec![1, 1]] {
        let sync = atom(
            b"stss",
            &[words(&[0, indices.len() as u32]), words(&indices)].concat(),
        );
        let replacement = atom(
            b"stbl",
            &[bytes[stbl_start..stbl_start + size - 8].to_vec(), sync].concat(),
        );
        // Replace the last (video) stbl via its track alone.
        let track_start = last_body(&bytes, b"trak") - 8;
        let track_size = u32_at(&bytes, track_start).unwrap() as usize;
        let track = replace_atom(
            &bytes[track_start..track_start + track_size],
            &[b"trak", b"mdia", b"minf", b"stbl"],
            &replacement,
        );
        let mut changed = bytes.clone();
        changed.splice(track_start..track_start + track_size, track);
        let movie = body(&changed, b"moov") - 8;
        let len = (changed.len() - movie) as u32;
        changed[movie..movie + 4].copy_from_slice(&len.to_be_bytes());
        assert_eq!(run(&changed).is_ok(), indices == [1, 2]);
    }
}

#[test]
fn mp4_video_requires_one_audio_and_at_most_one_video_in_either_order() {
    let bytes = video_fixture();
    let cancel = ResolverCancellation::default();
    let roots = children(&bytes, &[b"ftyp", b"mdat", b"moov"], &cancel).unwrap();
    let movie = children_with_tracks(
        get(&roots, b"moov").unwrap(),
        &[b"mvhd", b"trak"],
        true,
        &cancel,
    )
    .unwrap();
    let tracks = movie
        .iter()
        .filter(|a| a.kind == b"trak")
        .collect::<Vec<_>>();
    for (indices, valid) in [
        (vec![1, 0], true),
        (vec![1], false),
        (vec![0, 1, 1], false),
        (vec![1, 1], false),
        (vec![0, 0], false),
    ] {
        let mut body = atom(b"mvhd", get(&movie, b"mvhd").unwrap());
        for index in indices {
            body.extend(atom(b"trak", tracks[index].body));
        }
        let changed = replace_atom(&bytes, &[b"moov"], &atom(b"moov", &body));
        assert_eq!(run(&changed).is_ok(), valid);
    }
}

#[test]
fn mp4_audio_composition_table_requires_version_zero_and_matching_counts() {
    let bytes = fixture();
    let start = body(&bytes, b"stbl");
    let size = u32_at(&bytes, start - 8).unwrap() as usize;
    for (values, valid) in [
        (vec![0, 1, 2, 100], true),
        (vec![0, 1, 1, 100], false),
        (vec![0, 1, 0, 100], false),
        (vec![0x01000000, 1, 2, 100], false),
        (vec![0, 1, 2, 100, 0], false),
    ] {
        let stbl = atom(
            b"stbl",
            &[
                bytes[start..start + size - 8].to_vec(),
                atom(b"ctts", &words(&values)),
            ]
            .concat(),
        );
        let changed = replace_atom(
            &bytes,
            &[b"moov", b"trak", b"mdia", b"minf", b"stbl"],
            &stbl,
        );
        assert_eq!(run(&changed).is_ok(), valid);
    }
}

#[test]
fn mp4_video_truncation_mutation_and_metadata_cancellation_do_not_panic() {
    let bytes = video_fixture();
    for end in 0..bytes.len() {
        assert_eq!(run(&bytes[..end]), Err(MediaError::UnsupportedMedia));
    }
    for index in 0..bytes.len() {
        let mut changed = bytes.clone();
        changed[index] ^= 255;
        assert!(
            std::panic::catch_unwind(|| run(&changed)).is_ok(),
            "mutation {index}"
        );
    }
    let cancel = ResolverCancellation::default();
    let mut reader = ShortReader {
        cursor: Cursor::new(bytes.clone()),
        cancel: cancel.clone(),
        reads: 0,
        stop_at: Some(700),
        error: false,
    };
    assert_eq!(
        super::super::probe(&mut reader, bytes.len() as u64, &cancel),
        Err(MediaError::Cancelled)
    );
    assert_eq!(reader.reads, 700);
}

#[test]
fn mp4_wide_offsets_constant_sizes_multiple_chunks_and_front_metadata_are_validated() {
    let mut bytes = fixture();
    let start = u32_at(&bytes, body(&bytes, b"stco") + 8).unwrap();
    let offsets = [
        words(&[0, 2]),
        u64::from(start).to_be_bytes().to_vec(),
        (u64::from(start) + 4).to_be_bytes().to_vec(),
    ]
    .concat();
    bytes = replace_atom(
        &bytes,
        &[b"moov", b"trak", b"mdia", b"minf", b"stbl", b"stco"],
        &atom(b"co64", &offsets),
    );
    bytes = replace_atom(
        &bytes,
        &[b"moov", b"trak", b"mdia", b"minf", b"stbl", b"stsz"],
        &atom(b"stsz", &words(&[0, 4, 2])),
    );
    replace_word(&mut bytes, b"stsc", 12, 1);
    assert!(run(&bytes).is_ok());
    // Two runs must be ordered, and every run must use sample description 1.
    bytes = replace_atom(
        &bytes,
        &[b"moov", b"trak", b"mdia", b"minf", b"stbl", b"stsc"],
        &atom(b"stsc", &words(&[0, 2, 1, 1, 1, 2, 1, 1])),
    );
    assert!(run(&bytes).is_ok());
    let mut bad = bytes.clone();
    replace_word(&mut bad, b"stsc", 20, 1);
    assert!(run(&bad).is_err());
    // moov may precede mdat; adjust both real file offsets, never scan media.
    let moov_start = body(&bytes, b"moov") - 8;
    let moov_size = bytes.len() - moov_start;
    let co64 = body(&bytes, b"co64") + 8;
    for index in 0..2 {
        let pos = co64 + index * 8;
        let next = u64_at(&bytes, pos).unwrap() + moov_size as u64;
        bytes[pos..pos + 8].copy_from_slice(&next.to_be_bytes());
    }
    let mdat_start = body(&bytes, b"mdat") - 8;
    let front = [
        &bytes[..mdat_start],
        &bytes[moov_start..],
        &bytes[mdat_start..moov_start],
    ]
    .concat();
    assert!(run(&front).is_ok());
    let mut bad = front.clone();
    let at = body(&bad, b"co64") + 16;
    bad[at..at + 8].copy_from_slice(&u64::MAX.to_be_bytes());
    assert!(run(&bad).is_err());
}

#[test]
fn mp4_extended_top_level_box_sizes_are_checked_without_media_buffering() {
    let mut bytes = fixture();
    let mdat = body(&bytes, b"mdat") - 8;
    let size = u32_at(&bytes, mdat).unwrap() as usize;
    let extended = [
        1u32.to_be_bytes().to_vec(),
        b"mdat".to_vec(),
        (size as u64 + 8).to_be_bytes().to_vec(),
        bytes[mdat + 8..mdat + size].to_vec(),
    ]
    .concat();
    let old_offset = u32_at(&bytes, body(&bytes, b"stco") + 8).unwrap();
    replace_word(&mut bytes, b"stco", 8, old_offset + 8);
    let valid = replace_atom(&bytes, &[b"mdat"], &extended);
    assert!(run(&valid).is_ok());
    for length in [0, 15, u64::MAX] {
        let mut bad = valid.clone();
        bad[mdat + 8..mdat + 16].copy_from_slice(&length.to_be_bytes());
        assert_eq!(run(&bad), Err(MediaError::UnsupportedMedia));
    }
}

#[test]
fn mp4_each_mutated_byte_is_handled_without_panic() {
    let original = fixture();
    for index in 0..original.len() {
        let mut bytes = original.clone();
        bytes[index] ^= 0xff;
        let _ = run(&bytes); // Opaque media/text may change; arbitrary metadata must never panic.
    }
}

#[test]
fn mp4_bounded_gate_accepts_local_tables_and_resets_cursor() {
    let bytes = fixture();
    let mut cursor = Cursor::new(&bytes);
    assert_eq!(
        super::super::probe(
            &mut cursor,
            bytes.len() as u64,
            &ResolverCancellation::default()
        ),
        Ok(super::super::LocalAudioFormat::Mp4)
    );
    assert_eq!(cursor.position(), 0);
    assert_eq!(super::super::LocalAudioFormat::Mp4.mime(), "audio/mp4");
    assert!(super::super::LocalAudioFormat::Mp4.uses_packet_clock());
}

#[test]
fn mp4_metadata_cannot_mask_external_audio_references_or_change_table_rules() {
    let mut handler = vec![0; 24];
    handler[8..12].copy_from_slice(b"mdir");
    let data = atom(
        b"data",
        &[vec![0, 0, 0, 1, 0, 0, 0, 0], b"synthetic title".to_vec()].concat(),
    );
    let udta = atom(
        b"udta",
        &atom(
            b"meta",
            &[
                vec![0; 4],
                atom(b"hdlr", &handler),
                atom(b"ilst", &atom(b"\xa9nam", &data)),
            ]
            .concat(),
        ),
    );
    let bytes = fixture();
    let at = body(&bytes, b"moov") - 8;
    let mut tagged = [
        &bytes[..at],
        &atom(b"moov", &[bytes[at + 8..].to_vec(), udta].concat()),
    ]
    .concat();
    assert!(run(&tagged).is_ok());
    replace_word(&mut tagged, b"url ", 0, 0);
    assert_eq!(run(&tagged), Err(MediaError::UnsupportedMedia));
}
#[test]
fn mp4_rejects_external_references_multiple_descriptions_and_non_audio_tracks() {
    for (kind, offset, value) in [
        (b"url ", 0, 0),
        (b"url ", 0, 3),
        (b"dref", 4, 2),
        (b"stsd", 4, 2),
        (b"mp4a", 4, 2),
        (b"hdlr", 8, u32::from_be_bytes(*b"vide")),
    ] {
        let mut bytes = fixture();
        replace_word(&mut bytes, kind, offset, value);
        assert_eq!(run(&bytes), Err(MediaError::UnsupportedMedia), "{kind:?}");
    }
    // URL/dependency bits live in the ES descriptor, separate from dref.
    for flags in [0x20, 0x40, 0x80] {
        let mut bytes = fixture();
        let es = body(&bytes, b"esds");
        bytes[es + 8] = flags;
        assert_eq!(run(&bytes), Err(MediaError::UnsupportedMedia));
    }
}
#[test]
fn mp4_rejects_non_lc_implicit_channel_and_mismatched_rate() {
    for asc in [[0x29, 0x90], [0x11, 0x80], [0x12, 0x10], [0x11, 0x94]] {
        let mut bytes = fixture();
        let at = bytes
            .windows(4)
            .position(|w| w == [5, 2, 0x11, 0x90])
            .unwrap();
        bytes[at + 2..at + 4].copy_from_slice(&asc);
        assert_eq!(run(&bytes), Err(MediaError::UnsupportedMedia));
    }
    let mut bytes = fixture();
    let at = body(&bytes, b"mp4a") - 4;
    bytes[at..at + 4].copy_from_slice(b"enca");
    assert_eq!(run(&bytes), Err(MediaError::UnsupportedMedia));
}
#[test]
fn mp4_checks_every_sample_extent_and_table_consistency() {
    for (kind, offset, value) in [
        (b"stco", 8, 0),
        (b"stco", 8, u32::MAX),
        (b"stsz", 12, 9),
        (b"stsz", 16, 0),
        (b"stts", 8, 3),
        (b"stts", 12, 0),
        (b"stsc", 8, 2),
        (b"stsc", 12, 3),
        (b"stsc", 16, 2),
        (b"stsz", 8, u32::MAX),
        (b"stco", 4, u32::MAX),
    ] {
        let mut bytes = fixture();
        replace_word(&mut bytes, kind, offset, value);
        assert_eq!(
            run(&bytes),
            Err(MediaError::UnsupportedMedia),
            "{kind:?}/{offset}"
        );
    }
}
#[test]
fn mp4_rejects_fragments_duplicate_boxes_and_unknown_extensions() {
    let bytes = fixture();
    let track_start = body(&bytes, b"trak") - 8;
    let track_size = u32_at(&bytes, track_start).unwrap() as usize;
    let track = &bytes[track_start..track_start + track_size];
    let multiple_tracks = replace_atom(&bytes, &[b"moov", b"trak"], &[track, track].concat());
    assert_eq!(run(&multiple_tracks), Err(MediaError::UnsupportedMedia));
    for name in [b"moof", b"mfra", b"uuid", b"junk"] {
        let mut bytes = fixture();
        bytes.extend(atom(name, &[0; 16]));
        assert_eq!(run(&bytes), Err(MediaError::UnsupportedMedia));
    }
    let bytes = fixture();
    let start = body(&bytes, b"moov") - 8;
    let mut duplicate = bytes.clone();
    duplicate.extend_from_slice(&bytes[start..]);
    assert_eq!(run(&duplicate), Err(MediaError::UnsupportedMedia));
    for kind in [b"trak", b"stsd", b"dref"] {
        let mut bytes = fixture();
        let pos = body(&bytes, kind) - 4;
        bytes[pos..pos + 4].copy_from_slice(b"mvex");
        assert_eq!(run(&bytes), Err(MediaError::UnsupportedMedia));
    }
}
#[test]
fn mp4_rejects_truncation_overflow_and_oversized_metadata() {
    let bytes = fixture();
    for size in 0..bytes.len() {
        assert_eq!(run(&bytes[..size]), Err(MediaError::UnsupportedMedia));
    }
    for size in [0u32, 1, 7, u32::MAX] {
        let mut bytes = fixture();
        bytes[..4].copy_from_slice(&size.to_be_bytes());
        assert_eq!(run(&bytes), Err(MediaError::UnsupportedMedia));
    }
    let mut bytes = fixture();
    let start = body(&bytes, b"moov") - 8;
    bytes[start..start + 4].copy_from_slice(&((MAX_MOOV_BYTES + 9) as u32).to_be_bytes());
    bytes.resize(start + MAX_MOOV_BYTES + 9, 0);
    assert_eq!(run(&bytes), Err(MediaError::UnsupportedMedia));
    let mut bytes = fixture();
    for _ in 0..MAX_TOP_BOXES {
        bytes.extend(atom(b"free", &[]));
    }
    assert_eq!(run(&bytes), Err(MediaError::UnsupportedMedia));
}

struct ShortReader {
    cursor: Cursor<Vec<u8>>,
    cancel: ResolverCancellation,
    reads: usize,
    stop_at: Option<usize>,
    error: bool,
}
impl Read for ShortReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.reads += 1;
        if self.stop_at == Some(self.reads) {
            self.cancel.cancel();
        }
        if self.error {
            return Err(io::Error::other("synthetic I/O failure"));
        }
        let count = out.len().min(1);
        self.cursor.read(&mut out[..count])
    }
}
impl Seek for ShortReader {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.cursor.seek(position)
    }
}
#[test]
fn mp4_short_reads_io_failure_and_mid_metadata_cancellation_are_bounded() {
    let bytes = fixture();
    let len = bytes.len() as u64;
    for (stop_at, error) in [(None, false), (Some(100), false), (None, true)] {
        let cancel = ResolverCancellation::default();
        let mut reader = ShortReader {
            cursor: Cursor::new(bytes.clone()),
            cancel: cancel.clone(),
            reads: 0,
            stop_at,
            error,
        };
        let result = super::super::probe(&mut reader, len, &cancel);
        if error {
            assert_eq!(result, Err(MediaError::Unavailable));
        } else if stop_at.is_some() {
            assert_eq!(result, Err(MediaError::Cancelled));
            assert_eq!(reader.reads, 100);
        } else {
            assert_eq!(result, Ok(super::super::LocalAudioFormat::Mp4));
            assert!(reader.reads < bytes.len() + 40);
            assert_eq!(reader.cursor.position(), 0);
        }
    }
}
