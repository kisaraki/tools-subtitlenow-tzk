//! Actual Windows AAC encoder/ADTS source; synthetic tone and RAII files only.
use super::*;
use crate::{
    audio::capture::SessionId,
    media::{decoded::DecodedAudioGate, local_audio_format::LocalAudioFormat},
};
use std::{fs, fs::OpenOptions};
fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(709),
        generation: 1,
    }
}
fn input(path: &std::path::Path) -> MediaInput {
    MediaInput::local(path.to_path_buf()).unwrap()
}
#[test]
fn native_adts_real_encoder_supported_clock_preserves_tone_and_pinned_input() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    assert_supported_adts(2);
}
#[test]
fn native_adts_mono_clock_preserves_tone_and_pinned_input() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    assert_supported_adts(1);
}
fn assert_supported_adts(channels: u16) {
    let rate = 48_000u32;
    let bytes =
        crate::win32::media_mp3_fixture::synthetic_adts(rate, channels, 2 * rate as usize).unwrap();
    assert_eq!(
        probe(
            &mut std::io::Cursor::new(&bytes),
            bytes.len() as u64,
            &ResolverCancellation::default()
        ),
        Ok(LocalAudioFormat::Adts)
    );
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("synthetic-aac.bin");
    fs::write(&path, &bytes).unwrap();
    let cancel = ResolverCancellation::default();
    let (mut decoder, pin) = open_native_pinned(input(&path), identity(), cancel.clone()).unwrap();
    assert!(decoder.uses_packet_clock());
    assert_eq!(decoder.format().sample_rate(), rate);
    assert_eq!(decoder.format().channels(), channels);
    let mut gate = DecodedAudioGate::new(identity(), decoder.format()).unwrap();
    let mut mono = Vec::new();
    let mut last = 0;
    let mut blocks = 0;
    while let Some(block) = decoder.next(&cancel).unwrap() {
        gate.validate(&block).unwrap_or_else(|error| panic!("synthetic AAC rate {rate}, channels {channels}, block {blocks}, start {}, end {}, frames {}, previous end {last}: {error}", block.timestamp_us(), block.end_us(), block.frames()));
        assert_eq!(block.identity(), identity());
        assert!(block.samples().len() * 2 <= MAX_DECODED_BLOCK_BYTES);
        last = block.end_us();
        blocks += 1;
        mono.extend(
            block
                .samples()
                .chunks_exact(usize::from(channels))
                .map(|frame| frame[0]),
        );
    }
    assert!(blocks > 2 && mono.len() >= rate as usize);
    assert!((1_900_000..2_200_000).contains(&last), "native end {last}");
    let interior = &mono[rate as usize / 2..rate as usize];
    let rms = (interior
        .iter()
        .map(|sample| f64::from(*sample).powi(2))
        .sum::<f64>()
        / interior.len() as f64)
        .sqrt();
    assert!((3000.0..5500.0).contains(&rms), "synthetic AAC RMS {rms}");
    let crossings = interior
        .windows(2)
        .filter(|pair| pair[0] < 0 && pair[1] >= 0)
        .count();
    let frequency = crossings as f64 * f64::from(rate) / interior.len() as f64;
    assert!(
        (430.0..450.0).contains(&frequency),
        "synthetic AAC frequency {frequency}"
    );
    assert!(decoder.next(&cancel).unwrap().is_none());
    drop(decoder);
    assert!(OpenOptions::new().write(true).open(&path).is_err());
    assert!(fs::remove_file(&path).is_err());
    drop(pin);
    assert!(OpenOptions::new().write(true).open(&path).is_ok());
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}
#[test]
fn adts_borrowed_sample_clock_stays_rejected_and_native_cancellation_is_terminal() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    for channels in [1, 2] {
        assert_adts_cancellation(channels);
    }
}
fn assert_adts_cancellation(channels: u16) {
    let bytes = crate::win32::media_mp3_fixture::synthetic_adts(48_000, channels, 48_000).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("synthetic.bin");
    fs::write(&path, &bytes).unwrap();
    assert!(matches!(
        open_pinned(input(&path), identity(), ResolverCancellation::default()),
        Err(MediaError::UnsupportedAudioClock)
    ));
    assert!(OpenOptions::new().write(true).open(&path).is_ok());
    let cancel = ResolverCancellation::default();
    let (mut decoder, pin) = open_native_pinned(input(&path), identity(), cancel.clone()).unwrap();
    assert!(decoder.next(&cancel).unwrap().is_some());
    cancel.cancel();
    assert!(matches!(decoder.next(&cancel), Err(MediaError::Cancelled)));
    drop(decoder);
    drop(pin);
    assert!(OpenOptions::new().write(true).open(&path).is_ok());
}

#[test]
fn recognized_adts_unverified_clock_combinations_fail_before_pcm_and_release_pin() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    for (rate, channels) in [(44_100, 1), (44_100, 2)] {
        let bytes =
            crate::win32::media_mp3_fixture::synthetic_adts(rate, channels, rate as usize).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("synthetic.bin");
        fs::write(&path, &bytes).unwrap();
        assert!(matches!(
            open_native_pinned(input(&path), identity(), ResolverCancellation::default()),
            Err(MediaError::UnsupportedAudioClock)
        ));
        assert!(OpenOptions::new().write(true).open(&path).is_ok());
    }
}

#[test]
fn adts_native_sample_timing_checks_direct_duration_separately_from_frame_clock() {
    const HNS_PER_SECOND: u64 = 10_000_000;
    const HNS_PER_MICROSECOND: i64 = 10;
    // Thirteen seconds contains fewer than 1024 AAC-LC frames at either rate.
    // Bound even a reader that repeatedly returns empty ticks without EOF.
    const MAX_SAMPLE_READS: usize = 1024;
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    for (rate, channels) in [(44_100, 1), (44_100, 2), (48_000, 1), (48_000, 2)] {
        let bytes =
            crate::win32::media_mp3_fixture::synthetic_adts(rate, channels, 13 * rate as usize)
                .unwrap();
        let _apartment = super::super::resources::Apartment::new().unwrap();
        let _foundation = Foundation::new().unwrap();
        // SAFETY: test-owned bounded synthetic bytes are copied into a RAM
        // stream; all reader/type/sample references stay on this COM thread.
        unsafe {
            let stream = windows::Win32::UI::Shell::SHCreateMemStream(Some(&bytes)).unwrap();
            let byte_stream = MFCreateMFByteStreamOnStream(&stream).unwrap();
            byte_stream
                .cast::<IMFAttributes>()
                .unwrap()
                .SetString(
                    &MF_BYTESTREAM_CONTENT_TYPE,
                    &windows::core::HSTRING::from("audio/aac"),
                )
                .unwrap();
            let reader = MFCreateSourceReaderFromByteStream(&byte_stream, None).unwrap();
            let pcm = MFCreateMediaType().unwrap();
            pcm.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio).unwrap();
            pcm.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM).unwrap();
            pcm.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16).unwrap();
            reader
                .SetCurrentMediaType(MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32, None, &pcm)
                .unwrap();
            let format = current_format(&reader, &ResolverCancellation::default()).unwrap();
            assert_eq!((format.sample_rate(), format.channels()), (rate, channels));
            let mut count = 0;
            let mut previous_end = 0i64;
            let mut max_overlap = 0;
            let mut max_duration_delta = 0;
            let mut frame_total = 0u64;
            let mut final_end = 0;
            let mut ended = false;
            for _ in 0..MAX_SAMPLE_READS {
                let mut sample = None;
                let mut flags = 0;
                let mut timestamp = 0;
                reader
                    .ReadSample(
                        MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32,
                        0,
                        None,
                        Some(&mut flags),
                        Some(&mut timestamp),
                        Some(&mut sample),
                    )
                    .unwrap();
                if let Some(sample) = sample {
                    let time = sample.GetSampleTime().unwrap();
                    let duration = sample.GetSampleDuration().unwrap();
                    let length = sample.GetTotalLength().unwrap() as usize;
                    assert!(length > 0 && length <= MAX_NATIVE_SAMPLE_BYTES);
                    assert!(length.is_multiple_of(usize::from(channels) * 2));
                    let frames = length as u64 / (u64::from(channels) * 2);
                    assert_eq!(time, timestamp);
                    assert!(time >= 0 && frames > 0 && duration > 0);
                    let frame_duration = (frames * HNS_PER_SECOND / u64::from(rate)) as i64;
                    max_duration_delta = max_duration_delta.max((duration - frame_duration).abs());
                    max_overlap = max_overlap.max(previous_end - time);
                    previous_end = time + duration;
                    final_end = previous_end;
                    frame_total += frames;
                    count += 1;
                }
                if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                    ended = true;
                    break;
                }
            }
            assert!(ended && count > 1 && frame_total >= 13 * u64::from(rate));
            // Direct native duration is a separate observation, not a repair
            // for overlapping 44.1 kHz intervals. Product clocks stay gated.
            // 48 kHz observations must not exceed the unchanged 1 us gate.
            if rate == 48_000 {
                assert!(
                    max_overlap
                        <= crate::media::decoded::DECODED_TIMESTAMP_ROUNDING_US as i64
                            * HNS_PER_MICROSECOND,
                    "native overlap {max_overlap} hns"
                );
            }
            eprintln!(
                "synthetic ADTS {rate} Hz/{channels} channels: {count} native samples, {frame_total} frames, end {final_end} hns, max native duration overlap {max_overlap} hns, max duration versus frames delta {max_duration_delta} hns"
            );
        }
    }
}
