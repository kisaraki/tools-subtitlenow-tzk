use super::*;
fn atom(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    [
        ((body.len() + 8) as u32).to_be_bytes().to_vec(),
        kind.to_vec(),
        body.to_vec(),
    ]
    .concat()
}
fn item(name: &[u8; 4], kind: u32, data: &[u8]) -> Vec<u8> {
    atom(
        name,
        &atom(
            b"data",
            &[
                kind.to_be_bytes().to_vec(),
                0u32.to_be_bytes().to_vec(),
                data.to_vec(),
            ]
            .concat(),
        ),
    )
}
fn run(data: &[u8]) -> Result<(), MediaError> {
    validate(data, &ResolverCancellation::default())
}

#[test]
fn mp4_metadata_common_utf8_tags_and_url_shaped_text_are_inert() {
    let tags: [&[u8; 4]; 10] = [
        b"\xa9nam", b"\xa9ART", b"aART", b"\xa9alb", b"\xa9gen", b"\xa9wrt", b"\xa9day",
        b"\xa9cmt", b"\xa9too", b"cprt",
    ];
    let mut data = Vec::new();
    for name in tags {
        data.extend(item(
            name,
            1,
            "臺灣、東京、München; https://127.0.0.1/private; C:\\private".as_bytes(),
        ));
    }
    assert_eq!(run(&data), Ok(()));
    assert_eq!(run(&[]), Ok(()));
    assert_eq!(run(&item(b"\xa9nam", 1, &[])), Ok(()));
}
#[test]
fn mp4_metadata_accepts_bounded_png_and_jpeg_headers_without_decoding() {
    for (kind, header) in [
        (13, &[0xff, 0xd8, 0xff][..]),
        (14, &b"\x89PNG\r\n\x1a\n"[..]),
    ] {
        let mut value = vec![0; MAX_COVER_BYTES];
        value[..header.len()].copy_from_slice(header);
        assert_eq!(run(&item(b"covr", kind, &value)), Ok(())); // shape only, not real image payload.
        value.push(0);
        assert_eq!(
            run(&item(b"covr", kind, &value)),
            Err(MediaError::UnsupportedMedia)
        );
    }
}
#[test]
fn mp4_metadata_structured_track_disc_tempo_and_compilation_have_fixed_bounds() {
    let valid = [
        item(b"trkn", 0, &[0, 0, 0, 2, 0, 10, 0, 0]),
        item(b"disk", 0, &[0, 0, 0, 1, 0, 2]),
        item(b"tmpo", 21, &[0, 120]),
        item(b"cpil", 21, &[1]),
    ]
    .concat();
    assert_eq!(run(&valid), Ok(()));
    for bad in [
        item(b"trkn", 0, &[0, 0, 0, 11, 0, 10, 0, 0]),
        item(b"trkn", 0, &[0, 0, 0, 2, 0, 10, 0, 1]),
        item(b"disk", 0, &[0, 0, 0, 3, 0, 2]),
        item(b"tmpo", 21, &[0xff, 0xff]),
        item(b"cpil", 21, &[2]),
        item(b"cpil", 21, &[0, 1]),
        item(b"trkn", 1, b"2/10"),
    ] {
        assert_eq!(run(&bad), Err(MediaError::UnsupportedMedia));
    }
}
#[test]
fn mp4_metadata_unknown_reference_freeform_and_nested_atoms_are_rejected() {
    for name in [b"----", b"uuid", b"iloc", b"url ", b"keys", b"\xa9lyr"] {
        assert_eq!(
            run(&item(name, 1, b"synthetic")),
            Err(MediaError::UnsupportedMedia)
        );
    }
    for nested in [b"url ", b"mean", b"name", b"uuid", b"dref"] {
        assert_eq!(
            run(&atom(
                b"\xa9nam",
                &atom(nested, b"https://127.0.0.1/private")
            )),
            Err(MediaError::UnsupportedMedia)
        );
    }
}
#[test]
fn mp4_metadata_type_version_encoding_and_cover_mismatch_fail_explicitly() {
    for bad in [
        item(b"\xa9nam", 2, &[0, 65]),
        item(b"\xa9nam", 0x01000001, b"title"),
        item(b"\xa9nam", 1, &[0xff]),
        item(b"covr", 1, b"https://127.0.0.1/cover.png"),
        item(b"covr", 13, b"\x89PNG\r\n\x1a\n"),
        item(b"covr", 14, &[0xff, 0xd8, 0xff]),
        item(b"covr", 14, &[]),
    ] {
        assert_eq!(run(&bad), Err(MediaError::UnsupportedMedia));
    }
}
#[test]
fn mp4_metadata_duplicate_items_and_multiple_values_do_not_silently_pick_one() {
    let value = item(b"\xa9nam", 1, b"first");
    assert_eq!(
        run(&[value.clone(), value].concat()),
        Err(MediaError::UnsupportedMedia)
    );
    let value = atom(
        b"data",
        &[vec![0, 0, 0, 1, 0, 0, 0, 0], b"first".to_vec()].concat(),
    );
    assert_eq!(
        run(&atom(b"\xa9nam", &[value.clone(), value].concat())),
        Err(MediaError::UnsupportedMedia)
    );
}
#[test]
fn mp4_metadata_text_and_total_bytes_have_independent_limits() {
    assert_eq!(
        run(&item(b"\xa9nam", 1, &vec![b'a'; MAX_TEXT_BYTES])),
        Ok(())
    );
    assert_eq!(
        run(&item(b"\xa9nam", 1, &vec![b'a'; MAX_TEXT_BYTES + 1])),
        Err(MediaError::UnsupportedMedia)
    );
    assert_eq!(
        run(&vec![0; MAX_METADATA_BYTES + 1]),
        Err(MediaError::UnsupportedMedia)
    );
    let mut cover = vec![0; MAX_COVER_BYTES];
    cover[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
    let mut combined = item(b"covr", 14, &cover);
    for name in [b"\xa9nam", b"\xa9ART", b"aART", b"\xa9alb"] {
        combined.extend(item(name, 1, &vec![b'a'; MAX_TEXT_BYTES]));
    }
    assert!(combined.len() > MAX_METADATA_BYTES);
    assert_eq!(run(&combined), Err(MediaError::UnsupportedMedia));
}
#[test]
fn mp4_metadata_truncated_lengths_overflow_and_byte_mutation_never_panic() {
    let original = item(b"\xa9nam", 1, "臺灣合成資料".as_bytes());
    for length in 1..original.len() {
        assert!(run(&original[..length]).is_err());
    }
    for length in [0u32, 1, 7, u32::MAX] {
        let mut bytes = original.clone();
        bytes[..4].copy_from_slice(&length.to_be_bytes());
        assert!(run(&bytes).is_err());
    }
    for index in 0..original.len() {
        let mut bytes = original.clone();
        bytes[index] ^= 0xff;
        let _ = run(&bytes);
    }
}
#[test]
fn mp4_metadata_cancelled_validation_rejects_even_empty_or_valid_data() {
    let cancel = ResolverCancellation::default();
    cancel.cancel();
    for bytes in [vec![], item(b"\xa9nam", 1, b"synthetic")] {
        assert_eq!(validate(&bytes, &cancel), Err(MediaError::Cancelled));
    }
}
