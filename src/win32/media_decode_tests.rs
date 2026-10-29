//! Actual Windows Media Foundation on short caller-created PCM WAV inputs.
//! No user media, capture, speaker output, production export, keys or cloud.
use super::*;
use crate::{
    audio::capture::SessionId,
    media::decoded::{DecodedAudioGate, MAX_DECODED_BLOCK_SAMPLES},
};
use std::{fs, path::PathBuf};

struct Fixture {
    directory: tempfile::TempDir,
    path: PathBuf,
}
impl Fixture {
    fn bytes(bytes: &[u8]) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("synthetic-input.wav");
        fs::write(&path, bytes).unwrap();
        Self { directory, path }
    }
    fn input(&self) -> MediaInput {
        MediaInput::local(self.path.clone()).unwrap()
    }
}
fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(299),
        generation: 1,
    }
}
fn wave(rate: u32, channels: u16, samples: &[i16]) -> Vec<u8> {
    let bytes = u32::try_from(samples.len() * 2).unwrap();
    let mut body = Vec::new();
    body.extend_from_slice(b"RIFF");
    body.extend_from_slice(&(36 + bytes).to_le_bytes());
    body.extend_from_slice(b"WAVEfmt ");
    body.extend_from_slice(&16u32.to_le_bytes());
    body.extend_from_slice(&1u16.to_le_bytes());
    body.extend_from_slice(&channels.to_le_bytes());
    body.extend_from_slice(&rate.to_le_bytes());
    body.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
    body.extend_from_slice(&(channels * 2).to_le_bytes());
    body.extend_from_slice(&16u16.to_le_bytes());
    body.extend_from_slice(b"data");
    body.extend_from_slice(&bytes.to_le_bytes());
    body.extend(samples.iter().flat_map(|sample| sample.to_le_bytes()));
    body
}
fn samples(frames: usize, channels: u16) -> Vec<i16> {
    (0..frames * usize::from(channels))
        .map(|index| ((index % 40_001) as i32 - 20_000) as i16)
        .collect()
}

#[test]
fn actual_native_pcm_wave_preserves_channels_rates_samples_time_and_partial_tail() {
    for rate in [44_100, 48_000] {
        for channels in [1, 2] {
            let expected = samples(rate as usize + 137, channels);
            let fixture = Fixture::bytes(&wave(rate, channels, &expected));
            let before = fs::read(&fixture.path).unwrap();
            let cancel = ResolverCancellation::default();
            let mut decoder = open(fixture.input(), identity(), cancel.clone()).unwrap();
            assert_eq!(decoder.format().sample_rate(), rate);
            assert_eq!(decoder.format().channels(), channels);
            let mut gate = DecodedAudioGate::new(identity(), decoder.format()).unwrap();
            let mut offset = 0usize;
            let mut blocks = 0;
            let mut last_end = 0;
            while let Some(block) = decoder.next(&cancel).unwrap() {
                assert!(!gate.validate(&block).unwrap());
                assert!(block.samples().len() <= MAX_DECODED_BLOCK_SAMPLES);
                assert_eq!(
                    block.samples(),
                    &expected[offset..offset + block.samples().len()]
                );
                assert_eq!(block.identity(), identity());
                if blocks == 0 {
                    assert_eq!(block.timestamp_us(), 0);
                }
                offset += block.samples().len();
                blocks += 1;
                last_end = block.end_us();
            }
            assert_eq!(offset, expected.len());
            assert!(blocks > 1);
            let expected_end = (rate as u64 + 137) * 1_000_000 / rate as u64;
            assert!(last_end.abs_diff(expected_end) <= 1);
            assert!(decoder.next(&cancel).unwrap().is_none());
            drop(decoder);
            assert_eq!(fs::read(&fixture.path).unwrap(), before);
            assert_eq!(fs::read_dir(fixture.directory.path()).unwrap().count(), 1);
        }
    }
}

#[test]
fn actual_open_pinned_preserves_original_file_owner_after_native_destruction() {
    let expected = samples(16_137, 1);
    let fixture = Fixture::bytes(&wave(16_000, 1, &expected));
    let before = fs::read(&fixture.path).unwrap();
    let cancel = ResolverCancellation::default();
    let (mut decoder, media) = open_pinned(fixture.input(), identity(), cancel.clone()).unwrap();
    assert_eq!(media.byte_len(), before.len() as u64);
    let mut offset = 0;
    while let Some(block) = decoder.next(&cancel).unwrap() {
        assert_eq!(block.samples(), &expected[offset..offset + block.frames()]);
        offset += block.frames();
    }
    assert_eq!(offset, expected.len());
    drop(decoder);
    assert!(
        fs::OpenOptions::new()
            .write(true)
            .open(&fixture.path)
            .is_err()
    );
    assert!(fs::remove_file(&fixture.path).is_err());
    assert_eq!(fs::read(&fixture.path).unwrap(), before);
    drop(media);
    assert!(
        fs::OpenOptions::new()
            .write(true)
            .open(&fixture.path)
            .is_ok()
    );
    assert!(fs::remove_file(&fixture.path).is_ok());
}
#[test]
fn native_open_retains_same_readonly_file_pin_until_drop() {
    let fixture = Fixture::bytes(&wave(48_000, 1, &samples(2_000, 1)));
    let cancel = ResolverCancellation::default();
    let decoder = open(fixture.input(), identity(), cancel).unwrap();
    assert!(
        fs::OpenOptions::new()
            .write(true)
            .open(&fixture.path)
            .is_err()
    );
    assert!(fs::remove_file(&fixture.path).is_err());
    drop(decoder);
    assert!(
        fs::OpenOptions::new()
            .write(true)
            .open(&fixture.path)
            .is_ok()
    );
}
#[test]
fn native_cancel_rejects_pcm_and_reopening_a_fresh_run_works() {
    let fixture = Fixture::bytes(&wave(48_000, 2, &samples(100_000, 2)));
    let cancel = ResolverCancellation::default();
    let mut decoder = open(fixture.input(), identity(), cancel.clone()).unwrap();
    assert!(decoder.next(&cancel).unwrap().is_some());
    cancel.cancel();
    assert_eq!(decoder.next(&cancel).err(), Some(MediaError::Cancelled));
    drop(decoder);
    let fresh = ResolverCancellation::default();
    let mut decoder = open(fixture.input(), identity(), fresh.clone()).unwrap();
    assert_eq!(decoder.next(&fresh).unwrap().unwrap().sequence(), 1);
}
#[test]
fn cancelled_or_invalid_identity_never_opens_missing_private_input() {
    let fixture = Fixture::bytes(b"synthetic");
    fs::remove_file(&fixture.path).unwrap();
    let cancel = ResolverCancellation::default();
    cancel.cancel();
    assert_eq!(
        open(fixture.input(), identity(), cancel).err(),
        Some(MediaError::Cancelled)
    );
    let invalid = StreamIdentity {
        session: SessionId(0),
        generation: 0,
    };
    assert_eq!(
        open(fixture.input(), invalid, ResolverCancellation::default()).err(),
        Some(MediaError::InvalidAudio)
    );
}
#[test]
fn unknown_playlist_and_malformed_wave_never_decode_or_modify_input() {
    for body in [
        b"#EXTM3U\nhttps://example.invalid/private\n".to_vec(),
        b"RIFF\x04\0\0\0WAVE".to_vec(),
        b"not audio".to_vec(),
    ] {
        let fixture = Fixture::bytes(&body);
        let cancel = ResolverCancellation::default();
        assert!(matches!(
            open(fixture.input(), identity(), cancel),
            Err(MediaError::UnsupportedMedia | MediaError::InvalidAudio)
        ));
        assert_eq!(fs::read(&fixture.path).unwrap(), body);
        assert_eq!(fs::read_dir(fixture.directory.path()).unwrap().count(), 1);
    }
}
#[test]
fn unsupported_remote_media_does_not_fetch_or_create_native_reader() {
    let input = MediaInput::remote("https://example.invalid/private.wav?synthetic=1").unwrap();
    assert_eq!(
        open(input, identity(), ResolverCancellation::default()).err(),
        Some(MediaError::UnsupportedUrl)
    );
}
#[test]
fn native_header_and_hresult_classification_are_explicit() {
    let cancel = ResolverCancellation::default();
    assert_eq!(
        probe(
            &mut std::io::Cursor::new(b"RIFF\x04\0\0\0WAVE"),
            12,
            &cancel
        ),
        Ok(crate::media::local_audio_format::LocalAudioFormat::Wave)
    );
    assert_eq!(
        probe(&mut std::io::Cursor::new(b"RIFX\0\0\0\0WAVE"), 12, &cancel),
        Err(MediaError::UnsupportedMedia)
    );
    let cancel = ResolverCancellation::default();
    for (code, expected) in [
        (MF_E_LICENSE_REQUIRED, MediaError::ProtectedMedia),
        (
            MF_E_UNSUPPORTED_BYTESTREAM_TYPE,
            MediaError::UnsupportedMedia,
        ),
        (MF_E_INVALIDMEDIATYPE, MediaError::UnsupportedMedia),
    ] {
        assert_eq!(
            native::<()>(Err(windows::core::Error::from_hresult(code)), &cancel),
            Err(expected)
        );
    }
    cancel.cancel();
    assert_eq!(
        native::<()>(
            Err(windows::core::Error::from_hresult(MF_E_LICENSE_REQUIRED)),
            &cancel
        ),
        Err(MediaError::Cancelled)
    );
}

#[test]
fn native_read_flags_deliver_final_sample_and_never_hide_errors_or_format_changes() {
    let end = MF_SOURCE_READERF_ENDOFSTREAM.0 as u32;
    let tick = MF_SOURCE_READERF_STREAMTICK.0 as u32;
    assert_eq!(
        read_disposition(end, true),
        Ok(ReadDisposition::Sample { ended: true })
    );
    assert_eq!(read_disposition(end, false), Ok(ReadDisposition::End));
    assert_eq!(read_disposition(tick, false), Ok(ReadDisposition::Empty));
    assert_eq!(
        read_disposition(0, true),
        Ok(ReadDisposition::Sample { ended: false })
    );
    for has_sample in [false, true] {
        assert_eq!(
            read_disposition(end | MF_SOURCE_READERF_ERROR.0 as u32, has_sample),
            Err(MediaError::InvalidAudio)
        );
        for changed in [
            MF_SOURCE_READERF_NATIVEMEDIATYPECHANGED,
            MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED,
            MF_SOURCE_READERF_NEWSTREAM,
        ] {
            assert_eq!(
                read_disposition(end | changed.0 as u32, has_sample),
                Err(MediaError::UnsupportedMedia)
            );
        }
    }
}

#[test]
fn large_native_pending_splits_on_whole_frames_and_drains_after_end_flag() {
    let fixture = Fixture::bytes(&wave(44_100, 1, &samples(16, 1)));
    let cancel = ResolverCancellation::default();
    let mut decoder = open(fixture.input(), identity(), cancel.clone()).unwrap();
    // Explicit synthetic pending buffer exercises sizes a real WAVE handler
    // need not return. Three channels force 64 KiB rounding to whole frames.
    decoder.format = DecodedAudioFormat::new(44_100, 3).unwrap();
    let frames_per_block = MAX_DECODED_BLOCK_BYTES / 6;
    let expected = samples(frames_per_block * 2 + 137, 3);
    let bytes: Vec<u8> = expected
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect();
    // SAFETY: the decoder initialized MF on this same thread. Allocation is
    // bounded below 1 MiB and remains owned through filling and each pull.
    let buffer = unsafe { MFCreateMemoryBuffer(u32::try_from(bytes.len()).unwrap()) }.unwrap();
    let mut data = std::ptr::null_mut();
    let mut maximum = 0;
    // SAFETY: Lock returns a writable range; the checked allocation capacity
    // covers the entire fixture. Exactly one Unlock follows the copy.
    unsafe {
        buffer.Lock(&mut data, Some(&mut maximum), None).unwrap();
        assert!(!data.is_null());
        assert!(maximum as usize >= bytes.len());
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), data, bytes.len());
        buffer.Unlock().unwrap();
        buffer.SetCurrentLength(bytes.len() as u32).unwrap();
    }
    let base_us = 1_234_567;
    decoder.pending = Some(Pending {
        buffer,
        timestamp_us: base_us,
        length: bytes.len(),
        offset: 0,
    });
    decoder.ended = true;
    let mut gate = DecodedAudioGate::new(identity(), decoder.format()).unwrap();
    let mut offset = 0;
    for sequence in 1..=3 {
        let block = decoder.next(&cancel).unwrap().unwrap();
        assert_eq!(block.sequence(), sequence);
        assert!(!gate.validate(&block).unwrap());
        assert_eq!(
            block.timestamp_us(),
            base_us + (offset / 3) as u64 * 1_000_000 / 44_100
        );
        let length = if sequence < 3 {
            frames_per_block * 3
        } else {
            137 * 3
        };
        assert_eq!(block.samples(), &expected[offset..offset + length]);
        assert_eq!(block.samples().len(), length);
        offset += length;
    }
    assert_eq!(offset, expected.len());
    assert!(decoder.pending.is_none());
    assert!(decoder.next(&cancel).unwrap().is_none());
}

#[test]
fn actual_worker_decodes_short_native_wave_into_borrowed_ram_blocks_and_summary() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let expected = samples(48_137, 2);
    let fixture = Fixture::bytes(&wave(48_000, 2, &expected));
    let mut offset = 0;
    let summary = runtime
        .block_on(crate::media::decode_worker::decode_local_audio(
            fixture.input(),
            identity(),
            ResolverCancellation::default(),
            move |block, _| {
                assert_eq!(
                    block.samples(),
                    &expected[offset..offset + block.samples().len()]
                );
                offset += block.samples().len();
                Ok(())
            },
        ))
        .unwrap();
    assert_eq!(summary.frames(), 48_137);
    assert_eq!(summary.identity(), identity());
    assert!(
        fs::OpenOptions::new()
            .write(true)
            .open(&fixture.path)
            .is_ok()
    );
    assert_eq!(fs::read_dir(fixture.directory.path()).unwrap().count(), 1);
}

#[test]
fn actual_wave_preprocessing_downmixes_and_resamples_with_native_time_and_exact_tail_count() {
    use crate::media::{prepared_audio::PreparedSampleRate, prepared_worker::prepare_local_audio};
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let owner_thread = std::thread::current().id();
    for input_rate in [44_100, 48_000] {
        for channels in [1, 2] {
            for rate in [PreparedSampleRate::Hz16000, PreparedSampleRate::Hz24000] {
                let frames = input_rate as usize / 25 + 137;
                let expected_sample = if channels == 1 { 1357 } else { 0 };
                let samples: Vec<i16> = (0..frames)
                    .flat_map(|_| {
                        if channels == 1 {
                            vec![1357]
                        } else {
                            vec![1357, -1357]
                        }
                    })
                    .collect();
                let fixture = Fixture::bytes(&wave(input_rate, channels, &samples));
                let before = fs::read(&fixture.path).unwrap();
                let mut output_frames = 0u64;
                let mut sequence = 0u64;
                let summary = runtime
                    .block_on(prepare_local_audio(
                        fixture.input(),
                        identity(),
                        rate,
                        ResolverCancellation::default(),
                        move |block, _| {
                            assert_ne!(std::thread::current().id(), owner_thread);
                            assert_eq!(block.identity(), identity());
                            assert_eq!(block.sample_rate(), rate.hz());
                            sequence += 1;
                            assert_eq!(block.sequence(), sequence);
                            assert_eq!(
                                block.timestamp_us(),
                                output_frames * 1_000_000 / u64::from(rate.hz())
                            );
                            assert!(block.frames() <= (rate.hz() / 50) as usize);
                            assert!(
                                block
                                    .samples()
                                    .iter()
                                    .all(|value| *value == expected_sample)
                            );
                            assert!(!block.discontinuity());
                            output_frames += block.frames() as u64;
                            Ok(())
                        },
                    ))
                    .unwrap();
                let expected_frames = frames as u64 * u64::from(rate.hz()) / u64::from(input_rate);
                assert_eq!(summary.frames(), expected_frames);
                assert_eq!(
                    summary.blocks(),
                    expected_frames.div_ceil(u64::from(rate.hz() / 50))
                );
                assert_eq!(summary.identity(), identity());
                assert_eq!(summary.sample_rate(), rate.hz());
                assert_eq!(summary.first_timestamp_us(), 0);
                assert_eq!(summary.gaps(), 0);
                assert!(summary.last_end_us() <= frames as u64 * 1_000_000 / u64::from(input_rate));
                assert!(
                    fs::OpenOptions::new()
                        .write(true)
                        .open(&fixture.path)
                        .is_ok()
                );
                assert_eq!(fs::read(&fixture.path).unwrap(), before);
                assert_eq!(fs::read_dir(fixture.directory.path()).unwrap().count(), 1);
            }
        }
    }
}

#[test]
fn actual_preprocessing_cancel_releases_native_pin_and_fresh_run_restarts_at_first_block() {
    use crate::media::{prepared_audio::PreparedSampleRate, prepared_worker::prepare_local_audio};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let fixture = Fixture::bytes(&wave(48_000, 1, &vec![500; 48_000]));
    let calls = Arc::new(AtomicUsize::new(0));
    let sink = calls.clone();
    let cancel = ResolverCancellation::default();
    let result = runtime.block_on(prepare_local_audio(
        fixture.input(),
        identity(),
        PreparedSampleRate::Hz16000,
        cancel.clone(),
        move |block, cancel| {
            assert_eq!(block.sequence(), 1);
            sink.fetch_add(1, Ordering::SeqCst);
            cancel.cancel();
            Ok(())
        },
    ));
    assert_eq!(result.err(), Some(MediaError::Cancelled));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while fs::OpenOptions::new()
        .write(true)
        .open(&fixture.path)
        .is_err()
    {
        assert!(
            std::time::Instant::now() < deadline,
            "synthetic native worker did not release its pin"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let mut first = true;
    let summary = runtime
        .block_on(prepare_local_audio(
            fixture.input(),
            identity(),
            PreparedSampleRate::Hz24000,
            ResolverCancellation::default(),
            move |block, _| {
                if first {
                    assert_eq!(block.sequence(), 1);
                    assert_eq!(block.timestamp_us(), 0);
                    first = false;
                }
                Ok(())
            },
        ))
        .unwrap();
    assert_eq!(summary.frames(), 24_000);
}

#[test]
fn actual_native_wave_shorter_than_target_frame_is_not_padded_into_prepared_audio() {
    use crate::media::{prepared_audio::PreparedSampleRate, prepared_worker::prepare_local_audio};
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let fixture = Fixture::bytes(&wave(192_000, 1, &[500]));
    let result = runtime.block_on(prepare_local_audio(
        fixture.input(),
        identity(),
        PreparedSampleRate::Hz16000,
        ResolverCancellation::default(),
        |_, _| panic!("sub-frame source cannot create PCM"),
    ));
    assert_eq!(result.err(), Some(MediaError::InsufficientAudio));
    assert!(
        fs::OpenOptions::new()
            .write(true)
            .open(&fixture.path)
            .is_ok()
    );
    assert_eq!(fs::read_dir(fixture.directory.path()).unwrap().count(), 1);
}

#[test]
fn actual_long_wave_preprocessing_keeps_frame_clock_across_native_samples() {
    use crate::media::{prepared_audio::PreparedSampleRate, prepared_worker::prepare_local_audio};
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    // More than the 1 MiB native-sample budget, but never a user/captured file.
    // Exercise repeated native reads, FIR history wraps and the absolute clock.
    let input_frames = 44_100 * 30 + 137;
    let fixture = Fixture::bytes(&wave(44_100, 1, &vec![1234; input_frames]));
    assert!(fs::metadata(&fixture.path).unwrap().len() > MAX_NATIVE_SAMPLE_BYTES as u64);
    let mut output_frames = 0u64;
    let summary = runtime
        .block_on(prepare_local_audio(
            fixture.input(),
            identity(),
            PreparedSampleRate::Hz16000,
            ResolverCancellation::default(),
            move |block, _| {
                assert_eq!(block.timestamp_us(), output_frames * 1_000_000 / 16_000);
                assert!(block.samples().iter().all(|value| *value == 1234));
                assert!(!block.discontinuity());
                output_frames += block.frames() as u64;
                Ok(())
            },
        ))
        .unwrap();
    assert_eq!(summary.frames(), input_frames as u64 * 16_000 / 44_100);
    assert_eq!(summary.blocks(), summary.frames().div_ceil(320));
    assert_eq!(summary.gaps(), 0);
    assert!(summary.last_end_us() <= input_frames as u64 * 1_000_000 / 44_100);
    assert!(
        fs::OpenOptions::new()
            .write(true)
            .open(&fixture.path)
            .is_ok()
    );
    assert_eq!(fs::read_dir(fixture.directory.path()).unwrap().count(), 1);
}
