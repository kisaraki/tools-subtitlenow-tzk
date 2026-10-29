//! Fixed-width edit decoding and numeric boundaries, independent of v0 headers.
use super::*;

fn table(entries: &[(u64, i64, u32)]) -> Vec<u8> {
    let mut body = vec![1, 0, 0, 0];
    body.extend((entries.len() as u32).to_be_bytes());
    for (duration, time, rate) in entries {
        body.extend(duration.to_be_bytes());
        body.extend(time.to_be_bytes());
        body.extend(rate.to_be_bytes());
    }
    let mut atom = ((body.len() + 8) as u32).to_be_bytes().to_vec();
    atom.extend(b"elst");
    atom.extend(body);
    atom
}
fn parse(data: &[u8], total: u64) -> Result<TextEdit, MediaError> {
    TextEdit::parse(
        Some(data),
        1000,
        9000,
        18900,
        total,
        total,
        &ResolverCancellation::default(),
    )
}
#[test]
fn mp4_text_v1_helper_crops_with_fractional_movie_media_phase_and_exact_end() {
    let data = table(&[(1, -1, RATE_ONE), (1, 100, RATE_ONE)]);
    let edit = TextEdit::parse(
        Some(&data),
        7,
        9000,
        18900,
        2,
        2,
        &ResolverCancellation::default(),
    )
    .unwrap();
    assert_eq!(edit.sample_time(0, 700).unwrap(), Some((142857, 66666)));
    assert_eq!(edit.sample_time(1100, 1000).unwrap(), Some((253968, 31746)));
    assert_eq!(edit.sample_time(2100, 100).unwrap(), None);
    let data = table(&[(1000, 9900, RATE_ONE)]);
    let edit = parse(&data, 1000).unwrap();
    assert_eq!(edit.sample_time(9900, 9000).unwrap(), Some((0, 1000000)));
}
#[test]
fn mp4_text_v1_helper_preserves_high_words_and_checks_total_overflow() {
    // The helper takes u64 durations; the container still independently requires
    // supported v0 mdhd/tkhd/mvhd. These values must never wrap to low words.
    let high = (1u64 << 32) + 100;
    let data = table(&[(high, -1, RATE_ONE), (high, high as i64, RATE_ONE)]);
    let edit = TextEdit::parse(
        Some(&data),
        1_000_000,
        1_000_000,
        high * 2,
        high * 2,
        high * 2,
        &ResolverCancellation::default(),
    )
    .unwrap();
    assert_eq!(edit.sample_time(high, high).unwrap(), Some((high, high)));
    let overflow = table(&[(u64::MAX, -1, RATE_ONE), (1, 0, RATE_ONE)]);
    assert!(matches!(
        parse(&overflow, 0),
        Err(MediaError::UnsupportedMedia)
    ));
    let too_long = table(&[(86_400_000, -1, RATE_ONE), (1, 0, RATE_ONE)]);
    assert!(matches!(
        parse(&too_long, 86_400_001),
        Err(MediaError::TooLarge)
    ));
}
#[test]
fn mp4_text_v1_helper_rejects_signed_sentinels_shape_flags_rates_and_cancellation() {
    for entries in [
        vec![],
        vec![(1, -1, RATE_ONE)],
        vec![(1, -2, RATE_ONE)],
        vec![(1, i64::MIN, RATE_ONE)],
        vec![(1, i64::MAX, RATE_ONE)],
        vec![(0, 0, RATE_ONE)],
        vec![(1, 0, 0)],
        vec![(1, 0, 0x20000)],
        vec![(1, 0, 0x10001)],
        vec![(1, 0, 0xffff)],
        vec![(1, u32::MAX as i64, RATE_ONE), (1, 0, RATE_ONE)],
        vec![(1, -2, RATE_ONE), (1, 0, RATE_ONE)],
        vec![(0, -1, RATE_ONE), (1, 0, RATE_ONE)],
        vec![(1, -1, 0), (1, 0, RATE_ONE)],
        vec![(1, 0, RATE_ONE), (1, -1, RATE_ONE)],
        vec![(1, -1, RATE_ONE), (1, 0, RATE_ONE), (1, 0, RATE_ONE)],
    ] {
        assert!(parse(&table(&entries), 1).is_err());
    }
    let data = table(&[(2100, 0, RATE_ONE)]);
    assert!(parse(&data, 2100).is_ok());
    for end in 0..data.len() {
        assert!(parse(&data[..end], 2100).is_err());
    }
    for (index, value) in [(8, 0), (8, 2), (8, 255), (9, 1), (10, 1), (11, 1)] {
        let mut changed = data.clone();
        changed[index] = value;
        assert!(parse(&changed, 2100).is_err());
    }
    let mut extra = data.clone();
    extra.push(0);
    let len = extra.len() as u32;
    extra[..4].copy_from_slice(&len.to_be_bytes());
    assert!(parse(&extra, 2100).is_err());
    let cancel = ResolverCancellation::default();
    cancel.cancel();
    assert!(matches!(
        TextEdit::parse(Some(&data), 1000, 9000, 18900, 2100, 2100, &cancel),
        Err(MediaError::Cancelled)
    ));
}
