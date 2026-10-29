//! Real Windows MP3 encoder/decoder, synthesized tone only. No cloud or output.
use super::*;
use crate::{
    audio::capture::SessionId,
    media::{decoded::DecodedAudioGate, local_audio_format::LocalAudioFormat},
};
use std::{fs, fs::OpenOptions, path::PathBuf};

struct Fixture {
    dir: tempfile::TempDir,
    path: PathBuf,
}
impl Fixture {
    fn new(bytes: &[u8]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        // Deliberately not an .mp3 extension: content selects the native handler.
        let path = dir.path().join("synthetic-compressed.bin");
        fs::write(&path, bytes).unwrap();
        Self { dir, path }
    }
    fn input(&self) -> MediaInput {
        MediaInput::local(self.path.clone()).unwrap()
    }
    fn writable(&self) -> bool {
        OpenOptions::new().write(true).open(&self.path).is_ok()
    }
}
fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(329),
        generation: 1,
    }
}
// MPEG-1 codec delay/padding, measured through native time rather than trimmed.
const MAX_CODEC_EDGE_FRAMES: usize = 4 * 1152;
fn decode(fixture: &Fixture, rate: u32, channels: u16) -> (usize, u64) {
    let cancel = ResolverCancellation::default();
    let (mut decoder, pin) =
        open_native_pinned(fixture.input(), identity(), cancel.clone()).unwrap();
    assert_eq!(decoder.format().sample_rate(), rate);
    assert_eq!(decoder.format().channels(), channels);
    let mut gate = DecodedAudioGate::new(identity(), decoder.format()).unwrap();
    let mut pcm = Vec::new();
    let mut first = None;
    let mut last = 0;
    let mut gap_us = 0;
    let mut blocks = 0;
    while let Some(block) = decoder.next(&cancel).unwrap() {
        if gate.validate(&block).unwrap() {
            gap_us += block.timestamp_us().checked_sub(last).unwrap();
        }
        blocks += 1;
        assert_eq!(block.identity(), identity());
        assert!(block.samples().len() * 2 <= MAX_DECODED_BLOCK_BYTES);
        first.get_or_insert(block.timestamp_us());
        last = block.end_us();
        pcm.extend_from_slice(block.samples());
    }
    assert_eq!(first, Some(0));
    let frames = pcm.len() / usize::from(channels);
    // MP3 has codec delay/padding. Never assert PCM identity or invent a trim;
    // retain MF time and require a real, nonempty tone within a documented bound.
    assert!(
        frames >= rate as usize - MAX_CODEC_EDGE_FRAMES
            && frames <= rate as usize + MAX_CODEC_EDGE_FRAMES,
        "rate {rate}, channels {channels}, frames {frames}, end {last}, gap {gap_us}, blocks {blocks}"
    );
    // MF removes initial decoder delay from PCM but retains packet time. Its
    // resulting real reported gap must survive the gate; never insert silence
    // or reset timestamps to make compressed audio pretend to be continuous.
    assert!(gap_us > 0 && gap_us <= MAX_CODEC_EDGE_FRAMES as u64 * 1_000_000 / u64::from(rate));
    assert!(last.abs_diff(frames as u64 * 1_000_000 / u64::from(rate) + gap_us) <= blocks);
    let mono: Vec<i16> = pcm
        .chunks_exact(usize::from(channels))
        .map(|frame| frame[0])
        .collect();
    let interior = &mono[rate as usize / 5..4 * rate as usize / 5];
    let rms = (interior
        .iter()
        .map(|sample| f64::from(*sample).powi(2))
        .sum::<f64>()
        / interior.len() as f64)
        .sqrt();
    assert!(rms > 3000.0 && rms < 5500.0, "synthetic tone RMS {rms}");
    let crossings = interior
        .windows(2)
        .filter(|pair| pair[0] < 0 && pair[1] >= 0)
        .count();
    let frequency = crossings as f64 * f64::from(rate) / interior.len() as f64;
    assert!(
        (430.0..450.0).contains(&frequency),
        "synthetic tone frequency {frequency}"
    );
    assert!(decoder.next(&cancel).unwrap().is_none());
    drop(decoder);
    assert!(!fixture.writable());
    assert!(fs::remove_file(&fixture.path).is_err());
    drop(pin);
    assert!(fixture.writable());
    (frames, last)
}

#[test]
fn native_mpeg1_mp3_three_rates_mono_stereo_preserve_tone_time_and_pin() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    for rate in [32_000, 44_100, 48_000] {
        for channels in [1, 2] {
            let bytes =
                crate::win32::media_mp3_fixture::synthetic_mp3(rate, channels, rate as usize)
                    .unwrap();
            assert_eq!(
                probe(
                    &mut std::io::Cursor::new(&bytes),
                    bytes.len() as u64,
                    &ResolverCancellation::default()
                ),
                Ok(LocalAudioFormat::Mp3)
            );
            let fixture = Fixture::new(&bytes);
            let (_, end) = decode(&fixture, rate, channels);
            let (expected_end, precision) = encoded_end_us(&bytes, rate);
            assert!(
                end.abs_diff(expected_end) <= precision,
                "rate {rate}, channels {channels}, native end {end}, frame end {expected_end}, precision {precision}"
            );
            assert_eq!(fs::read(&fixture.path).unwrap(), bytes);
            assert_eq!(fs::read_dir(fixture.dir.path()).unwrap().count(), 1);
        }
    }
}

fn without_id3(bytes: Vec<u8>) -> Vec<u8> {
    if !bytes.starts_with(b"ID3") {
        return bytes;
    }
    let len = bytes[6..10]
        .iter()
        .fold(0usize, |n, b| (n << 7) | usize::from(*b));
    let start = 10
        + len
        + if bytes[3] == 4 && bytes[5] & 0x10 != 0 {
            10
        } else {
            0
        };
    bytes[start..].to_vec()
}
fn encoded_end_us(bytes: &[u8], rate: u32) -> (u64, u64) {
    // Independent complete-frame accounting for the test encoder's declared
    // 128 kbps MPEG-1 output, not an assumed duration of its source PCM.
    let raw = without_id3(bytes.to_vec());
    let mut offset = 0;
    let mut count = 0u64;
    while offset < raw.len() {
        if raw.len() - offset == 128 && raw[offset..].starts_with(b"TAG") {
            break;
        }
        assert!(raw.len() - offset >= 4);
        let header = u32::from_be_bytes(raw[offset..offset + 4].try_into().unwrap());
        assert_eq!(header >> 21, 0x7ff);
        assert_eq!((header >> 19) & 3, 3);
        assert_eq!((header >> 17) & 3, 1);
        assert_eq!((header >> 12) & 15, 9);
        let index = ((header >> 10) & 3) as usize;
        assert!(index < 3);
        assert_eq!([44_100, 48_000, 32_000][index], rate);
        offset += (144_000 * 128 / rate + ((header >> 9) & 1)) as usize;
        assert!(offset <= raw.len());
        count += 1;
    }
    // Native MF time uses 100 ns units. At most one unit of independent
    // truncation per compressed frame, plus the final microsecond conversion.
    assert!(count >= 2);
    (
        count * 1152 * 1_000_000 / u64::from(rate),
        count.div_ceil(10) + 1,
    )
}

#[test]
fn actual_mp3_native_time_cannot_enter_sample_clock_preprocessing() {
    use crate::media::{prepared_audio::PreparedSampleRate, prepared_worker::prepare_local_audio};
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    let bytes = crate::win32::media_mp3_fixture::synthetic_mp3(44_100, 2, 88_200).unwrap();
    let fixture = Fixture::new(&bytes);
    let cancel = ResolverCancellation::default();
    let mut decoder = open(fixture.input(), identity(), cancel.clone()).unwrap();
    let mut gate = DecodedAudioGate::new(identity(), decoder.format()).unwrap();
    let mut runs = vec![0u64];
    let mut native_end = 0;
    while let Some(block) = decoder.next(&cancel).unwrap() {
        if gate.validate(&block).unwrap() {
            runs.push(0);
        }
        *runs.last_mut().unwrap() += block.frames() as u64;
        native_end = block.end_us();
    }
    drop(decoder);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    for rate in [PreparedSampleRate::Hz16000, PreparedSampleRate::Hz24000] {
        let result = runtime.block_on(prepare_local_audio(
            fixture.input(),
            identity(),
            rate,
            ResolverCancellation::default(),
            |_, _| {
                panic!("unsupported compressed clock must not deliver PCM to DSP");
            },
        ));
        assert_eq!(result.err(), Some(MediaError::UnsupportedAudioClock));
        assert!(fixture.writable());
    }
    assert!(runs.len() > 1 && native_end > 1_000_000);
    assert_eq!(fs::read(&fixture.path).unwrap(), bytes);
}
fn tagged(raw: &[u8], version: u8, footer: bool) -> Vec<u8> {
    // Legal opaque zero padding, no private metadata or URLs supplied to MF.
    let header = [
        b'I',
        b'D',
        b'3',
        version,
        0,
        if footer { 0x10 } else { 0 },
        0,
        0,
        0,
        32,
    ];
    let mut bytes = header.to_vec();
    bytes.extend_from_slice(&[0; 32]);
    if footer {
        bytes.extend_from_slice(b"3DI");
        bytes.extend_from_slice(&header[3..]);
    }
    bytes.extend_from_slice(raw);
    bytes
}
#[test]
fn actual_raw_and_id3_prefixed_mp3_use_only_the_selected_pinned_file() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    let raw =
        without_id3(crate::win32::media_mp3_fixture::synthetic_mp3(44_100, 1, 44_100).unwrap());
    for bytes in [
        raw.clone(),
        tagged(&raw, 2, false),
        tagged(&raw, 3, false),
        tagged(&raw, 4, false),
        tagged(&raw, 4, true),
    ] {
        let fixture = Fixture::new(&bytes);
        decode(&fixture, 44_100, 1);
        assert_eq!(fs::read(&fixture.path).unwrap(), bytes);
    }
}
#[test]
fn mp3_cancel_releases_native_pin_and_a_fresh_session_can_decode_again() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    let bytes = crate::win32::media_mp3_fixture::synthetic_mp3(48_000, 2, 48_000).unwrap();
    let fixture = Fixture::new(&bytes);
    let cancel = ResolverCancellation::default();
    let mut decoder = open(fixture.input(), identity(), cancel.clone()).unwrap();
    assert!(decoder.next(&cancel).unwrap().is_some());
    cancel.cancel();
    assert_eq!(decoder.next(&cancel).err(), Some(MediaError::Cancelled));
    drop(decoder);
    assert!(fixture.writable());
    decode(&fixture, 48_000, 2);
    assert_eq!(fs::read(&fixture.path).unwrap(), bytes);
}
#[test]
fn truncated_mp3_never_transfers_a_pin_or_creates_content_files() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    let raw =
        without_id3(crate::win32::media_mp3_fixture::synthetic_mp3(44_100, 1, 44_100).unwrap());
    for bytes in [raw[..12].to_vec(), b"ID3\x04\0\0\x7f\x7f\x7f\x7f".to_vec()] {
        let fixture = Fixture::new(&bytes);
        assert_eq!(
            open(fixture.input(), identity(), ResolverCancellation::default()).err(),
            Some(MediaError::UnsupportedMedia)
        );
        assert!(fixture.writable());
        assert_eq!(fs::read(&fixture.path).unwrap(), bytes);
        assert_eq!(fs::read_dir(fixture.dir.path()).unwrap().count(), 1);
    }
}

#[test]
fn actual_mp3_public_decode_summary_retains_native_time_and_reclaims_workers() {
    use crate::media::decode_worker::decode_local_audio;
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    let bytes = crate::win32::media_mp3_fixture::synthetic_mp3(44_100, 2, 88_200).unwrap();
    let fixture = Fixture::new(&bytes);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let summary = runtime
        .block_on(decode_local_audio(
            fixture.input(),
            identity(),
            ResolverCancellation::default(),
            |block, _| {
                assert_eq!(block.identity(), identity());
                assert!(block.samples().len() * 2 <= MAX_DECODED_BLOCK_BYTES);
                Ok(())
            },
        ))
        .unwrap();
    assert_eq!(summary.identity(), identity());
    assert_eq!(summary.format().sample_rate(), 44_100);
    assert_eq!(summary.format().channels(), 2);
    assert_eq!(summary.first_timestamp_us(), 0);
    assert!(summary.frames() > 44_100 && summary.blocks() > 1 && summary.gaps() > 0);
    let (end, precision) = encoded_end_us(&bytes, 44_100);
    assert!(summary.last_end_us().abs_diff(end) <= precision);
    assert!(fixture.writable());
    assert_eq!(
        crate::media::work::workers().available_permits(),
        crate::media::work::worker_limit()
    );
    assert!(crate::media::work::native_owners().available_permits() > 0);
    assert_eq!(fs::read(&fixture.path).unwrap(), bytes);
    assert_eq!(fs::read_dir(fixture.dir.path()).unwrap().count(), 1);
}
