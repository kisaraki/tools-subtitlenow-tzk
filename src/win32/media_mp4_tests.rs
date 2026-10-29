//! Genuine RAM-encoded AAC / H.264 MP4; no user media or cloud.
use super::*;
use crate::{audio::capture::SessionId, media::decoded::DecodedAudioGate};
use std::{fs, fs::OpenOptions};
fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(719),
        generation: 1,
    }
}
fn input(path: &std::path::Path) -> MediaInput {
    MediaInput::local(path.to_path_buf()).unwrap()
}

#[test]
fn native_mp4_b_frames_are_proven_in_compressed_video() {
    use crate::win32::media_mp3_fixture::{H264Profile, synthetic_mp4_b_frames};
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    for profile in [H264Profile::Main, H264Profile::High] {
        for rate in [44_100u32, 48_000] {
            for channels in [1, 2] {
                let bytes =
                    synthetic_mp4_b_frames(rate, channels, 6 * rate as usize + 137, profile)
                        .unwrap();
                assert_eq!(
                    probe(
                        &mut std::io::Cursor::new(&bytes),
                        bytes.len() as u64,
                        &ResolverCancellation::default()
                    ),
                    Ok(LocalAudioFormat::Mp4)
                );
            }
        }
    }
}

#[test]
fn native_mp4_main_high_fixture_profiles_are_real() {
    use crate::win32::media_mp3_fixture::{H264Profile, synthetic_mp4_video_profile};
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    for profile in [H264Profile::Main, H264Profile::High] {
        let bytes = synthetic_mp4_video_profile(48_000, 2, 96_137, profile).unwrap();
        let start = bytes.windows(4).position(|b| b == b"avcC").unwrap();
        let size = u32::from_be_bytes(bytes[start - 4..start].try_into().unwrap()) as usize;
        assert_eq!(bytes[start + 5], profile as u8);
        assert!(size > 8 && size < 64 * 1024);
        assert_eq!(
            probe(
                &mut std::io::Cursor::new(&bytes),
                bytes.len() as u64,
                &ResolverCancellation::default()
            ),
            Ok(LocalAudioFormat::Mp4)
        );
        assert!(bytes.len() < 320 * 1024);
    }
}

#[test]
fn native_mp4_video_tone_clock_and_pin_for_both_rates_and_channels() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    for rate in [44_100, 48_000] {
        assert_mp4_tone_clock_with_video(rate, true);
    }
}

#[test]
fn native_mp4_tone_clock_and_pin_survive_wrong_extension_for_both_channels() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    assert_mp4_tone_clock(48_000);
}
#[test]
fn native_mp4_44100_tone_clock_and_pin_survive_wrong_extension_for_both_channels() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    assert_mp4_tone_clock(44_100);
}
fn assert_mp4_tone_clock(rate: u32) {
    assert_mp4_tone_clock_with_video(rate, false);
}
fn assert_mp4_tone_clock_with_video(rate: u32, video: bool) {
    assert_mp4_tone_clock_with_profile(
        rate,
        video.then_some(crate::win32::media_mp3_fixture::H264Profile::Baseline),
    );
}
#[test]
fn native_mp4_main_high_tone_clock_pin_for_both_rates_channels() {
    use crate::win32::media_mp3_fixture::H264Profile;
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    for profile in [H264Profile::Main, H264Profile::High] {
        for rate in [44_100, 48_000] {
            assert_mp4_tone_clock_with_profile(rate, Some(profile));
        }
    }
}
fn assert_mp4_tone_clock_with_profile(
    rate: u32,
    profile: Option<crate::win32::media_mp3_fixture::H264Profile>,
) {
    assert_mp4_tone_clock_profile_clock(rate, profile, false);
}
#[test]
fn native_mp4_b_frames_tone_clock_pin_for_both_profiles_rates_channels() {
    use crate::win32::media_mp3_fixture::H264Profile;
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    for profile in [H264Profile::Main, H264Profile::High] {
        for rate in [44_100, 48_000] {
            assert_mp4_tone_clock_profile_clock(rate, Some(profile), true);
        }
    }
}
fn assert_mp4_tone_clock_profile_clock(
    rate: u32,
    profile: Option<crate::win32::media_mp3_fixture::H264Profile>,
    b_frames: bool,
) {
    let seconds = if b_frames { 6 } else { 2 };
    for channels in [1u16, 2] {
        let bytes = if b_frames {
            crate::win32::media_mp3_fixture::synthetic_mp4_b_frames(
                rate,
                channels,
                seconds * rate as usize + 137,
                profile.unwrap(),
            )
        } else if let Some(profile) = profile {
            crate::win32::media_mp3_fixture::synthetic_mp4_video_profile(
                rate,
                channels,
                2 * rate as usize + 137,
                profile,
            )
        } else {
            crate::win32::media_mp3_fixture::synthetic_mp4(rate, channels, 2 * rate as usize + 137)
        }
        .unwrap();
        assert_eq!(
            probe(
                &mut std::io::Cursor::new(&bytes),
                bytes.len() as u64,
                &ResolverCancellation::default()
            ),
            Ok(LocalAudioFormat::Mp4)
        );
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("synthetic.wav");
        fs::write(&path, &bytes).unwrap();
        let cancel = ResolverCancellation::default();
        let (mut decoder, pin) =
            open_native_pinned(input(&path), identity(), cancel.clone()).unwrap();
        assert!(decoder.uses_packet_clock());
        assert_eq!(
            decoder.format(),
            DecodedAudioFormat::new(rate, channels).unwrap()
        );
        let mut gate = DecodedAudioGate::new(identity(), decoder.format()).unwrap();
        let mut frames = 0;
        let mut squares = 0f64;
        let mut crossings = 0;
        let mut previous = 0;
        let mut end = 0;
        while let Some(block) = decoder.next(&cancel).unwrap() {
            gate.validate(&block).unwrap();
            assert!(block.samples().len() * 2 <= MAX_DECODED_BLOCK_BYTES);
            assert_eq!(block.identity(), identity());
            end = block.end_us();
            for frame in block.samples().chunks_exact(channels as usize) {
                if previous < 0 && frame[0] >= 0 {
                    crossings += 1;
                }
                previous = frame[0];
                squares += f64::from(frame[0]).powi(2);
                frames += 1;
            }
        }
        assert!(
            (seconds as u64 * 1_000_000 - 100_000..seconds as u64 * 1_000_000 + 200_000)
                .contains(&end),
            "native MP4 end={end}"
        );
        let frequency = crossings as f64 * f64::from(rate) / frames as f64;
        assert!((425.0..450.0).contains(&frequency), "frequency={frequency}");
        assert!((3000.0..5500.0).contains(&(squares / frames as f64).sqrt()));
        drop(decoder);
        assert!(OpenOptions::new().write(true).open(&path).is_err());
        assert!(fs::remove_file(&path).is_err());
        drop(pin);
        assert!(OpenOptions::new().write(true).open(&path).is_ok());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
}
#[test]
fn native_mp4_cancellation_and_borrowed_clock_rejection_release_the_same_pin() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    assert_mp4_cancellation(false);
}
#[test]
fn native_mp4_video_cancellation_and_borrowed_clock_rejection_release_the_same_pin() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    assert_mp4_cancellation(true);
}
fn assert_mp4_cancellation(video: bool) {
    assert_mp4_cancellation_profile(
        video.then_some(crate::win32::media_mp3_fixture::H264Profile::Baseline),
    );
}
#[test]
fn native_mp4_main_high_cancel_and_borrowed_clock_release_pin() {
    use crate::win32::media_mp3_fixture::H264Profile;
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    for profile in [H264Profile::Main, H264Profile::High] {
        assert_mp4_cancellation_profile(Some(profile));
    }
}
fn assert_mp4_cancellation_profile(profile: Option<crate::win32::media_mp3_fixture::H264Profile>) {
    assert_mp4_cancellation_profile_clock(profile, false);
}
#[test]
fn native_mp4_b_frames_cancel_and_borrowed_clock_release_pin() {
    use crate::win32::media_mp3_fixture::H264Profile;
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    for profile in [H264Profile::Main, H264Profile::High] {
        assert_mp4_cancellation_profile_clock(Some(profile), true);
    }
}
fn assert_mp4_cancellation_profile_clock(
    profile: Option<crate::win32::media_mp3_fixture::H264Profile>,
    b_frames: bool,
) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("synthetic.bin");
    for channels in [1, 2] {
        for rate in [44_100u32, 48_000] {
            let bytes = if b_frames {
                crate::win32::media_mp3_fixture::synthetic_mp4_b_frames(
                    rate,
                    channels,
                    6 * rate as usize,
                    profile.unwrap(),
                )
            } else if let Some(profile) = profile {
                crate::win32::media_mp3_fixture::synthetic_mp4_video_profile(
                    rate,
                    channels,
                    rate as usize,
                    profile,
                )
            } else {
                crate::win32::media_mp3_fixture::synthetic_mp4(rate, channels, rate as usize)
            }
            .unwrap();
            fs::write(&path, bytes).unwrap();
            let cancel = ResolverCancellation::default();
            assert!(matches!(
                open_pinned(input(&path), identity(), cancel.clone()),
                Err(MediaError::UnsupportedAudioClock)
            ));
            let (mut decoder, pin) =
                open_native_pinned(input(&path), identity(), cancel.clone()).unwrap();
            assert!(decoder.next(&cancel).unwrap().is_some());
            cancel.cancel();
            assert!(matches!(decoder.next(&cancel), Err(MediaError::Cancelled)));
            drop(decoder);
            drop(pin);
            assert!(OpenOptions::new().write(true).open(&path).is_ok());
        }
    }
}

#[test]
fn native_mp4_tags_and_png_cover_preserve_every_pcm_sample_and_native_timestamp() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    for (rate, channels) in [(44_100u32, 1), (44_100, 2), (48_000, 1), (48_000, 2)] {
        let original =
            crate::win32::media_mp3_fixture::synthetic_mp4(rate, channels, 2 * rate as usize + 137)
                .unwrap();
        let tagged = crate::win32::media_mp3_fixture::tagged_mp4(&original);
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("original.bin");
        let second = directory.path().join("tagged.bin");
        fs::write(&first, &original).unwrap();
        fs::write(&second, &tagged).unwrap();
        let cancel = ResolverCancellation::default();
        let (mut left, left_pin) =
            open_native_pinned(input(&first), identity(), cancel.clone()).unwrap();
        let (mut right, right_pin) =
            open_native_pinned(input(&second), identity(), cancel.clone()).unwrap();
        assert_eq!(left.format(), right.format());
        let mut gate = DecodedAudioGate::new(identity(), right.format()).unwrap();
        let mut count = 0;
        loop {
            match (left.next(&cancel).unwrap(), right.next(&cancel).unwrap()) {
                (Some(a), Some(b)) => {
                    gate.validate(&b).unwrap();
                    count += 1;
                    assert_eq!(
                        (a.timestamp_us(), a.end_us(), a.sequence()),
                        (b.timestamp_us(), b.end_us(), b.sequence())
                    );
                    assert_eq!(a.samples(), b.samples());
                }
                (None, None) => break,
                _ => panic!("metadata changed genuine audio EOF"),
            }
        }
        assert!(count > 2);
        drop(left);
        drop(right);
        drop(left_pin);
        assert!(OpenOptions::new().write(true).open(&second).is_err());
        assert!(fs::remove_file(&second).is_err());
        drop(right_pin);
        assert!(OpenOptions::new().write(true).open(&second).is_ok());
        assert_eq!(fs::read(&second).unwrap(), tagged);
        assert_eq!(fs::read(&first).unwrap(), original);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
    }
}

#[test]
fn mp4_native_aac_timing_observation_is_independent_of_product_clock_maps() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    assert_mp4_native_timing(false);
}
#[test]
fn mp4_video_native_aac_timing_observation_is_independent_of_product_clock_maps() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    assert_mp4_native_timing(true);
}
fn assert_mp4_native_timing(video: bool) {
    assert_mp4_native_timing_profile(
        video.then_some(crate::win32::media_mp3_fixture::H264Profile::Baseline),
    );
}
#[test]
fn mp4_main_high_native_aac_timing_independent_of_product_clock_maps() {
    use crate::win32::media_mp3_fixture::H264Profile;
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    for profile in [H264Profile::Main, H264Profile::High] {
        assert_mp4_native_timing_profile(Some(profile));
    }
}
fn assert_mp4_native_timing_profile(profile: Option<crate::win32::media_mp3_fixture::H264Profile>) {
    assert_mp4_native_timing_profile_clock(profile, false);
}
#[test]
fn mp4_b_frames_native_aac_timing_independent_of_product_clock_maps() {
    use crate::win32::media_mp3_fixture::H264Profile;
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    for profile in [H264Profile::Main, H264Profile::High] {
        assert_mp4_native_timing_profile_clock(Some(profile), true);
    }
}
fn assert_mp4_native_timing_profile_clock(
    profile: Option<crate::win32::media_mp3_fixture::H264Profile>,
    b_frames: bool,
) {
    const MAX_READS: usize = 1024;
    const HNS_PER_MICROSECOND: i64 = 10;
    const MICROSECONDS_PER_SECOND: u64 = 1_000_000;
    for rate in [44_100u32, 48_000] {
        for channels in [1u16, 2] {
            for input_frames in [
                if b_frames { 4 } else { 1 } * rate as usize,
                if b_frames { 6 } else { 2 } * rate as usize + 137,
                13 * rate as usize + 311,
            ] {
                let bytes = if b_frames {
                    crate::win32::media_mp3_fixture::synthetic_mp4_b_frames(
                        rate,
                        channels,
                        input_frames,
                        profile.unwrap(),
                    )
                } else if let Some(profile) = profile {
                    crate::win32::media_mp3_fixture::synthetic_mp4_video_profile(
                        rate,
                        channels,
                        input_frames,
                        profile,
                    )
                } else {
                    crate::win32::media_mp3_fixture::synthetic_mp4(rate, channels, input_frames)
                }
                .unwrap();
                let _apartment = super::super::resources::Apartment::new().unwrap();
                let _foundation = Foundation::new().unwrap();
                // SAFETY: test-owned, bounded encoder output is copied to a RAM
                // IStream. Reader, media types and samples stay on this MTA;
                // each interface is released before MF/COM guards. Getters only
                // copy metadata; no decoded PCM allocation is retained here.
                unsafe {
                    let stream =
                        windows::Win32::UI::Shell::SHCreateMemStream(Some(&bytes)).unwrap();
                    let byte_stream = MFCreateMFByteStreamOnStream(&stream).unwrap();
                    byte_stream
                        .cast::<IMFAttributes>()
                        .unwrap()
                        .SetString(
                            &MF_BYTESTREAM_CONTENT_TYPE,
                            &windows::core::HSTRING::from("audio/mp4"),
                        )
                        .unwrap();
                    let reader = MFCreateSourceReaderFromByteStream(&byte_stream, None).unwrap();
                    reader
                        .SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)
                        .unwrap();
                    reader
                        .SetStreamSelection(MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32, true)
                        .unwrap();
                    let pcm = MFCreateMediaType().unwrap();
                    pcm.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio).unwrap();
                    pcm.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM).unwrap();
                    pcm.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16).unwrap();
                    reader
                        .SetCurrentMediaType(
                            MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32,
                            None,
                            &pcm,
                        )
                        .unwrap();
                    assert_eq!(
                        current_format(&reader, &ResolverCancellation::default()).unwrap(),
                        DecodedAudioFormat::new(rate, channels).unwrap()
                    );
                    let mut samples = 0;
                    let mut frames = 0u64;
                    let mut prior_native_end = 0i64;
                    let mut prior_frame_end = 0i64;
                    let mut max_native_overlap = 0;
                    let mut max_frame_overlap = 0;
                    let mut final_end = 0;
                    let mut ended = false;
                    for _ in 0..MAX_READS {
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
                        assert_eq!(
                            flags
                                & (MF_SOURCE_READERF_ERROR.0 as u32
                                    | MF_SOURCE_READERF_NATIVEMEDIATYPECHANGED.0 as u32
                                    | MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0 as u32
                                    | MF_SOURCE_READERF_NEWSTREAM.0 as u32),
                            0
                        );
                        if let Some(sample) = sample {
                            let time = sample.GetSampleTime().unwrap();
                            let duration = sample.GetSampleDuration().unwrap();
                            let length = sample.GetTotalLength().unwrap() as usize;
                            assert_eq!(time, timestamp);
                            assert!(
                                time >= 0
                                    && duration > 0
                                    && length > 0
                                    && length <= MAX_NATIVE_SAMPLE_BYTES
                            );
                            assert!(length.is_multiple_of(usize::from(channels) * 2));
                            let frame_count = length as u64 / (u64::from(channels) * 2);
                            max_native_overlap = max_native_overlap.max(prior_native_end - time);
                            max_frame_overlap =
                                max_frame_overlap.max(prior_frame_end - time / HNS_PER_MICROSECOND);
                            prior_native_end = time + duration;
                            prior_frame_end = time / HNS_PER_MICROSECOND
                                + (frame_count * MICROSECONDS_PER_SECOND / u64::from(rate)) as i64;
                            frames += frame_count;
                            final_end = prior_frame_end;
                            samples += 1;
                        }
                        if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                            ended = true;
                            break;
                        }
                    }
                    assert!(ended && samples > 1 && frames >= input_frames as u64);
                    let tolerance = crate::media::decoded::DECODED_TIMESTAMP_ROUNDING_US as i64;
                    assert!(max_native_overlap <= tolerance * HNS_PER_MICROSECOND);
                    assert!(max_frame_overlap <= tolerance);
                    eprintln!(
                        "synthetic MP4 profile={profile:?} B={b_frames} {rate}/{channels}, input {input_frames} frames: samples={samples}, PCM frames={frames}, end={final_end} us, native overlap={max_native_overlap} hns, frame overlap={max_frame_overlap} us"
                    );
                }
            }
        }
    }
}
