//! Synthetic headers only. Passing this gate does not claim valid decoded audio.
use super::*;
use std::io::{Cursor, Error};

fn frames(header: [u8; 4], bytes: usize) -> Vec<u8> {
    let mut frame = vec![0; bytes];
    frame[..4].copy_from_slice(&header);
    [frame.clone(), frame].concat()
}
fn audio() -> Vec<u8> {
    frames([0xff, 0xfb, 0x90, 0xc0], 417)
}
fn tag(version: u8, flags: u8, payload: &[u8]) -> Vec<u8> {
    let size = payload.len() as u32;
    let header = [
        b'I',
        b'D',
        b'3',
        version,
        0,
        flags,
        ((size >> 21) & 127) as u8,
        ((size >> 14) & 127) as u8,
        ((size >> 7) & 127) as u8,
        (size & 127) as u8,
    ];
    let mut out = header.to_vec();
    out.extend_from_slice(payload);
    if version == 4 && flags & 0x10 != 0 {
        out.extend_from_slice(b"3DI");
        out.extend_from_slice(&header[3..]);
    }
    out.extend(audio());
    out
}
fn run(bytes: Vec<u8>) -> Result<LocalAudioFormat, MediaError> {
    let len = bytes.len() as u64;
    let mut reader = Cursor::new(bytes);
    let result = probe(&mut reader, len, &ResolverCancellation::default());
    if result.is_ok() {
        assert_eq!(reader.position(), 0);
    }
    result
}

#[test]
fn original_wave_header_gate_and_trusted_mime_remain_intact() {
    assert_eq!(
        run(b"RIFF\x04\0\0\0WAVE".to_vec()),
        Ok(LocalAudioFormat::Wave)
    );
    assert_eq!(LocalAudioFormat::Wave.mime(), "audio/wav");
    assert_eq!(LocalAudioFormat::Mp3.mime(), "audio/mpeg");
}
#[test]
fn mpeg1_three_rates_mono_stereo_and_crc_headers_have_checked_frame_lengths() {
    for (rate_bits, expected_rate, bytes) in [(0, 44_100, 417), (4, 48_000, 384), (8, 32_000, 576)]
    {
        for channel in [0, 0x40, 0x80, 0xc0] {
            for protection in [0xfa, 0xfb] {
                let header = [0xff, protection, 0x90 | rate_bits, channel];
                let frame = mp3_frame(header).unwrap();
                assert_eq!(frame.rate, expected_rate);
                assert_eq!(frame.bytes, bytes as u32);
                assert_eq!(frame.channels, if channel == 0xc0 { 1 } else { 2 });
                assert_eq!(run(frames(header, bytes)), Ok(LocalAudioFormat::Mp3));
            }
        }
    }
}
#[test]
fn padding_and_variable_bitrate_do_not_require_identical_headers() {
    let mut bytes = frames([0xff, 0xfb, 0x90, 0], 417);
    bytes.truncate(417);
    let second = frames([0xff, 0xfb, 0xa2, 0x40], 523);
    bytes.extend_from_slice(&second[..523]);
    assert_eq!(run(bytes), Ok(LocalAudioFormat::Mp3));
}

fn adts(rate_index: u8, channels: u8, length: usize, crc: bool) -> Vec<u8> {
    let mut bytes = vec![0; length];
    bytes[..7].copy_from_slice(&[
        0xff,
        if crc { 0xf0 } else { 0xf1 },
        0x40 | (rate_index << 2),
        (channels << 6) | ((length >> 11) as u8),
        (length >> 3) as u8,
        ((length as u8 & 7) << 5) | 0x1f,
        0xfc,
    ]);
    bytes
}
#[test]
fn adts_two_complete_lc_frames_support_checked_rate_channels_and_crc_header_size() {
    for rate in [3, 4] {
        for channels in [1, 2] {
            for crc in [false, true] {
                let mut bytes = adts(rate, channels, 100, crc);
                bytes.extend(adts(rate, channels, 200, !crc));
                assert_eq!(run(bytes), Ok(LocalAudioFormat::Adts));
            }
        }
    }
    assert_eq!(LocalAudioFormat::Adts.mime(), "audio/aac");
    assert!(LocalAudioFormat::Adts.uses_packet_clock());
    assert!(LocalAudioFormat::Mp3.uses_packet_clock());
    assert!(!LocalAudioFormat::Wave.uses_packet_clock());
}
#[test]
fn adts_rejects_mpeg2_other_profiles_rates_pce_multichannel_and_multiple_raw_blocks() {
    for (offset, replacement) in [
        (1, 0xf9),
        (1, 0xf3),
        (2, 0x10),
        (2, 0x90),
        (2, 0xd0),
        (2, 0x4c),
        (2, 0x7c),
        (3, 0),
        (3, 0xc0),
        (6, 0xfd),
    ] {
        let mut bytes = adts(4, 2, 100, false);
        bytes[offset] = replacement;
        bytes.extend(adts(4, 2, 100, false));
        assert_eq!(run(bytes), Err(MediaError::UnsupportedMedia));
    }
}
#[test]
fn adts_truncation_changed_format_empty_payload_and_junk_prefix_do_not_pass() {
    let mut full = adts(4, 2, 100, false);
    full.extend(adts(4, 2, 100, false));
    for length in [0, 6, 12, 100, 106, 199] {
        assert_eq!(
            run(full[..length].to_vec()),
            Err(MediaError::UnsupportedMedia)
        );
    }
    for second in [
        adts(3, 2, 100, false),
        adts(4, 1, 100, false),
        adts(4, 2, 7, false),
        adts(4, 2, 9, true),
    ] {
        let mut bytes = adts(4, 2, 100, false);
        bytes.extend(second);
        assert_eq!(run(bytes), Err(MediaError::UnsupportedMedia));
    }
    let mut junk = b"opaque prefix".to_vec();
    junk.extend(full);
    assert_eq!(run(junk), Err(MediaError::UnsupportedMedia));
}
#[test]
fn adts_probe_reads_twenty_bytes_resets_cursor_and_respects_cancel_and_io_errors() {
    let mut bytes = adts(4, 2, 8191, false);
    bytes.extend(adts(4, 2, 8191, true));
    let mut reader = Observed::new(bytes.clone());
    assert_eq!(reader.probe(), Ok(LocalAudioFormat::Adts));
    assert_eq!(reader.received, 20);
    assert_eq!(reader.bytes.position(), 0);
    reader = Observed::new(bytes.clone());
    reader.max_read = 1;
    assert_eq!(reader.probe(), Ok(LocalAudioFormat::Adts));
    assert_eq!(reader.received, 20);
    reader = Observed::new(bytes.clone());
    reader.cancel_read = true;
    assert_eq!(reader.probe(), Err(MediaError::Cancelled));
    reader = Observed::new(bytes.clone());
    reader.cancel_seek = true;
    assert_eq!(reader.probe(), Err(MediaError::Cancelled));
    reader = Observed::new(bytes);
    reader.fail_read = true;
    assert_eq!(reader.probe(), Err(MediaError::Unavailable));
}
#[test]
fn all_three_id3_versions_skip_opaque_bounded_metadata() {
    for version in [2, 3, 4] {
        assert_eq!(run(tag(version, 0, &[])), Ok(LocalAudioFormat::Mp3));
        assert_eq!(
            run(tag(
                version,
                0,
                b"PRIVATE_SYNTHETIC_METADATA https://example.invalid/"
            )),
            Ok(LocalAudioFormat::Mp3)
        );
    }
}
#[test]
fn id3_v24_footer_is_excluded_from_size_and_must_match_header() {
    let bytes = tag(4, 0x10, &[0; 16]);
    assert_eq!(run(bytes.clone()), Ok(LocalAudioFormat::Mp3));
    for index in [26, 29, 31, 35] {
        let mut wrong = bytes.clone();
        wrong[index] ^= 1;
        assert_eq!(run(wrong), Err(MediaError::UnsupportedMedia));
    }
}
#[test]
fn reserved_id3_flags_unknown_versions_and_v22_compression_are_unsupported() {
    for (version, flags) in [
        (2, 1),
        (2, 0x40),
        (3, 0x10),
        (4, 1),
        (1, 0),
        (5, 0),
        (255, 0),
    ] {
        assert_eq!(
            run(tag(version, flags, &[])),
            Err(MediaError::UnsupportedMedia)
        );
    }
    let mut bytes = tag(3, 0, &[]);
    bytes[4] = 255;
    assert_eq!(run(bytes), Err(MediaError::UnsupportedMedia));
}
#[test]
fn nonsynchsafe_id3_size_and_truncated_declared_tag_are_rejected() {
    for index in 6..10 {
        let mut bytes = tag(3, 0, &[]);
        bytes[index] |= 0x80;
        assert_eq!(run(bytes), Err(MediaError::UnsupportedMedia));
    }
    let mut bytes = tag(3, 0, &[]);
    bytes[8] = 64;
    assert_eq!(run(bytes), Err(MediaError::UnsupportedMedia));
}
#[test]
fn id3_capacity_counts_header_and_footer_not_only_payload() {
    assert_eq!(
        run(tag(3, 0, &vec![0; MAX_ID3_TAG_BYTES as usize - 10])),
        Ok(LocalAudioFormat::Mp3)
    );
    assert_eq!(
        run(tag(3, 0, &vec![0; MAX_ID3_TAG_BYTES as usize - 9])),
        Err(MediaError::UnsupportedMedia)
    );
    assert_eq!(
        run(tag(4, 0x10, &vec![0; MAX_ID3_TAG_BYTES as usize - 20])),
        Ok(LocalAudioFormat::Mp3)
    );
    assert_eq!(
        run(tag(4, 0x10, &vec![0; MAX_ID3_TAG_BYTES as usize - 19])),
        Err(MediaError::UnsupportedMedia)
    );
}
#[test]
fn only_mpeg1_layer3_nonfree_nonreserved_headers_are_supported() {
    for header in [
        [0xff, 0xf3, 0x90, 0],
        [0xff, 0xe3, 0x90, 0],
        [0xff, 0xeb, 0x90, 0],
        [0xff, 0xfd, 0x90, 0],
        [0xff, 0xff, 0x90, 0],
        [0xff, 0xfb, 0, 0],
        [0xff, 0xfb, 0xf0, 0],
        [0xff, 0xfb, 0x9c, 0],
        [0xff, 0xfb, 0x90, 2],
        [0x7f, 0xfb, 0x90, 0],
    ] {
        assert!(mp3_frame(header).is_err());
        assert_eq!(run(frames(header, 417)), Err(MediaError::UnsupportedMedia));
    }
}
#[test]
fn complete_second_frame_and_consistent_rate_channels_are_required() {
    for len in [4, 12, 417, 420, 833] {
        assert_eq!(
            run(audio()[..len].to_vec()),
            Err(MediaError::UnsupportedMedia)
        );
    }
    for (header, size) in [
        ([0xff, 0xfb, 0x94, 0xc0], 384),
        ([0xff, 0xfb, 0x90, 0], 417),
    ] {
        let mut bytes = audio()[..417].to_vec();
        bytes.extend_from_slice(&frames(header, size)[..size]);
        assert_eq!(run(bytes), Err(MediaError::UnsupportedMedia));
    }
}
#[test]
fn playlist_mp4_adts_and_junk_prefix_are_never_scanned_for_embedded_sync() {
    for prefix in [
        b"#EXTM3U\nhttps://example.invalid/".as_slice(),
        b"\0\0\0\x18ftypisom".as_slice(),
        b"\xff\xf1\x50\x80\0\x1f\xfc\0\0\0\0\0".as_slice(),
        b"arbitrary junk".as_slice(),
    ] {
        let mut bytes = prefix.to_vec();
        bytes.extend(audio());
        assert_eq!(run(bytes), Err(MediaError::UnsupportedMedia));
    }
}

struct Observed {
    bytes: Cursor<Vec<u8>>,
    reads: usize,
    seeks: usize,
    received: usize,
    cancel: ResolverCancellation,
    cancel_read: bool,
    cancel_seek: bool,
    max_read: usize,
    fail_read: bool,
    fail_seek: bool,
    interrupted: bool,
}
impl Observed {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes: Cursor::new(bytes),
            reads: 0,
            seeks: 0,
            received: 0,
            cancel: ResolverCancellation::default(),
            cancel_read: false,
            cancel_seek: false,
            max_read: usize::MAX,
            fail_read: false,
            fail_seek: false,
            interrupted: false,
        }
    }
    fn probe(&mut self) -> Result<LocalAudioFormat, MediaError> {
        let len = self.bytes.get_ref().len() as u64;
        let cancel = self.cancel.clone();
        probe(self, len, &cancel)
    }
}
impl Read for Observed {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.reads += 1;
        if self.cancel_read {
            self.cancel.cancel();
        }
        if self.fail_read {
            return Err(Error::other("synthetic failure"));
        }
        if self.interrupted {
            return Err(Error::from(ErrorKind::Interrupted));
        }
        let count = buffer.len().min(self.max_read);
        let read = self.bytes.read(&mut buffer[..count])?;
        self.received += read;
        Ok(read)
    }
}
impl Seek for Observed {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.seeks += 1;
        if self.cancel_seek {
            self.cancel.cancel();
        }
        if self.fail_seek {
            return Err(Error::other("synthetic seek failure"));
        }
        self.bytes.seek(pos)
    }
}
#[test]
fn full_metadata_is_never_read_or_allocated_by_the_gate() {
    let mut reader = Observed::new(tag(4, 0x10, &vec![0; MAX_ID3_TAG_BYTES as usize - 20]));
    assert_eq!(reader.probe(), Ok(LocalAudioFormat::Mp3));
    assert_eq!(reader.received, 31);
    assert_eq!(reader.seeks, 6);
    assert_eq!(reader.bytes.position(), 0);
}
#[test]
fn short_reads_remain_bounded_and_reconstruct_the_same_evidence() {
    let mut reader = Observed::new(audio());
    reader.max_read = 1;
    assert_eq!(reader.probe(), Ok(LocalAudioFormat::Mp3));
    assert_eq!(reader.received, 21);
    assert_eq!(reader.reads, 21);
}
#[test]
fn precancel_does_no_io_and_cancel_during_seek_or_read_wins_over_success() {
    let mut reader = Observed::new(audio());
    reader.cancel.cancel();
    assert_eq!(reader.probe(), Err(MediaError::Cancelled));
    assert_eq!(reader.reads + reader.seeks, 0);
    for seek in [false, true] {
        let mut reader = Observed::new(audio());
        reader.cancel_seek = seek;
        reader.cancel_read = !seek;
        assert_eq!(reader.probe(), Err(MediaError::Cancelled));
    }
}
#[test]
fn io_errors_are_typed_and_repeated_interrupts_cannot_spin_forever() {
    for seek in [false, true] {
        let mut reader = Observed::new(audio());
        reader.fail_seek = seek;
        reader.fail_read = !seek;
        assert_eq!(reader.probe(), Err(MediaError::Unavailable));
    }
    let mut reader = Observed::new(audio());
    reader.interrupted = true;
    assert_eq!(reader.probe(), Err(MediaError::Unavailable));
    assert_eq!(reader.reads, MAX_INTERRUPTED_READS);
}
#[test]
fn declared_length_and_actual_eof_are_both_checked_without_guessing() {
    let cancel = ResolverCancellation::default();
    let mut reader = Cursor::new(audio()[..833].to_vec());
    assert_eq!(
        probe(&mut reader, 834, &cancel),
        Err(MediaError::UnsupportedMedia)
    );
    let mut reader = Cursor::new(audio());
    assert_eq!(
        probe(&mut reader, 833, &cancel),
        Err(MediaError::UnsupportedMedia)
    );
    assert_eq!(
        probe(&mut reader, u64::MAX, &cancel),
        Ok(LocalAudioFormat::Mp3)
    );
}
