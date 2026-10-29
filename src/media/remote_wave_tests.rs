use super::test_source as source;
use super::*;
use source::{Source, identity, riff, samples, wave};
use std::sync::atomic::Ordering;

#[tokio::test]
async fn remote_wave_exact_pcm_frames_fractional_timestamps_and_eof_for_supported_rates_channels() {
    for rate in [8000, 16000, 44100, 48000, 192000] {
        for channels in [1, 2] {
            let expected = samples(50003, channels);
            let source = Source::new(wave(rate, channels, &expected)).await;
            let (mut decoder, cancel) = source.decoder().await;
            assert_eq!(decoder.identity(), identity());
            assert_eq!(decoder.total_frames(), 50003);
            let mut gate = DecodedAudioGate::new(identity(), decoder.format()).unwrap();
            let mut offset = 0;
            let mut sequence = 1;
            while let Some(owner) = decoder.read_next().await.unwrap() {
                let block = owner.block().unwrap();
                assert_eq!(block.sequence(), sequence);
                assert_eq!(block.identity(), identity());
                assert_eq!(
                    block.timestamp_us(),
                    (offset as u64 / u64::from(channels)) * 1_000_000 / u64::from(rate)
                );
                assert_eq!(
                    block.samples(),
                    &expected[offset..offset + block.samples().len()]
                );
                assert!(block.samples().len() * 2 <= MAX_DECODED_BLOCK_BYTES);
                assert!(!gate.validate(block).unwrap());
                offset += block.samples().len();
                sequence += 1;
            }
            assert_eq!(offset, expected.len());
            assert!(sequence > 2);
            let requests = source.rows.lock().unwrap().len();
            assert!(decoder.read_next().await.unwrap().is_none());
            assert_eq!(source.rows.lock().unwrap().len(), requests);
            assert!(!cancel.is_cancelled());
            assert_eq!(source.cpu.available_permits(), 1);
            assert_eq!(source.buffers.available_permits(), 4);
            assert_eq!(source.pcm.available_permits(), 4);
            drop(decoder);
            assert!(cancel.is_cancelled());
            assert_eq!(source.owners.available_permits(), 1);
            source.finish().await;
        }
    }
}
fn fmt() -> Vec<u8> {
    wave(16000, 1, &[0])[20..36].to_vec()
}

#[tokio::test]
async fn remote_wave_validates_full_chunk_layout_and_word_padding_before_any_pcm() {
    for junk in [vec![], vec![0], vec![1, 2]] {
        let source = Source::new(riff(vec![
            (*b"JUNK", junk.clone()),
            (*b"fmt ", fmt()),
            (*b"data", vec![0, 128, 255, 127]),
            (*b"JUNK", junk),
        ]))
        .await;
        let (mut decoder, _) = source.decoder().await;
        let block = decoder.read_next().await.unwrap().unwrap();
        assert_eq!(block.block().unwrap().samples(), &[i16::MIN, i16::MAX]);
        assert!(decoder.read_next().await.unwrap().is_none());
        drop(block);
        drop(decoder);
        source.finish().await;
    }
}

#[tokio::test]
async fn remote_wave_rejects_non_pcm_16_extensible_multichannel_and_invalid_numeric_fields() {
    for (offset, value, wide, unsupported) in [
        (0, 3, false, true),
        (0, 0xfffe, false, true),
        (14, 8, false, true),
        (14, 24, false, true),
        (2, 0, false, true),
        (2, 3, false, true),
        (4, 0, true, false),
        (4, 192001, true, false),
        (8, 1, true, false),
        (12, 3, false, false),
    ] {
        let mut fmt = fmt();
        if wide {
            fmt[offset..offset + 4].copy_from_slice(&(value as u32).to_le_bytes());
        } else {
            fmt[offset..offset + 2].copy_from_slice(&(value as u16).to_le_bytes());
        }
        let source = Source::new(riff(vec![(*b"fmt ", fmt), (*b"data", vec![0, 0])])).await;
        let (input, cancel) = source.input(identity()).await;
        let result =
            RemoteWaveDecoder::open_with(input, source.cpu.clone(), source.pcm.clone()).await;
        let expected = if unsupported {
            MediaError::UnsupportedMedia
        } else {
            MediaError::InvalidAudio
        };
        assert!(matches!(result,Err(RemoteWaveError::Media(error)) if error==expected));
        assert!(cancel.is_cancelled());
        assert_eq!(source.owners.available_permits(), 1);
        source.finish().await;
    }
}

#[tokio::test]
async fn remote_wave_rejects_missing_duplicate_misordered_unknown_and_truncated_chunks() {
    let fmt = fmt();
    let good = riff(vec![(*b"fmt ", fmt.clone()), (*b"data", vec![0, 0])]);
    let mut wrong_riff = good.clone();
    wrong_riff[4] = 0;
    let mut oversized = good.clone();
    oversized[40..44].copy_from_slice(&100u32.to_le_bytes());
    let mut partial = good.clone();
    partial.push(0);
    let len = partial.len() as u32 - 8;
    partial[4..8].copy_from_slice(&len.to_le_bytes());
    let mut bad_pad = riff(vec![
        (*b"JUNK", vec![0]),
        (*b"fmt ", fmt.clone()),
        (*b"data", vec![0, 0]),
    ]);
    bad_pad[21] = 1;
    for bytes in [
        b"RIFF\x04\0\0\0WAVE".to_vec(),
        wrong_riff,
        oversized,
        partial,
        bad_pad,
        riff(vec![(*b"fmt ", fmt.clone())]),
        riff(vec![(*b"data", vec![0, 0]), (*b"fmt ", fmt.clone())]),
        riff(vec![
            (*b"fmt ", fmt.clone()),
            (*b"fmt ", fmt.clone()),
            (*b"data", vec![0, 0]),
        ]),
        riff(vec![
            (*b"fmt ", fmt.clone()),
            (*b"data", vec![0, 0]),
            (*b"data", vec![0, 0]),
        ]),
        riff(vec![(*b"fmt ", fmt.clone()), (*b"data", vec![])]),
        riff(vec![(*b"fmt ", fmt.clone()), (*b"data", vec![0])]),
        riff(vec![
            (*b"LIST", vec![0; 4]),
            (*b"fmt ", fmt.clone()),
            (*b"data", vec![0, 0]),
        ]),
        riff(vec![
            (*b"fmt ", [fmt.clone(), vec![0; 2]].concat()),
            (*b"data", vec![0, 0]),
        ]),
    ] {
        let source = Source::new(bytes).await;
        let (input, cancel) = source.input(identity()).await;
        assert!(
            RemoteWaveDecoder::open_with(input, source.cpu.clone(), source.pcm.clone())
                .await
                .is_err()
        );
        assert!(cancel.is_cancelled());
        assert_eq!(source.owners.available_permits(), 1);
        assert_eq!(source.buffers.available_permits(), 4);
        source.finish().await;
    }
}

#[tokio::test]
async fn remote_wave_chunk_count_and_total_junk_budgets_are_enforced() {
    for (count, accepted) in [(126, true), (127, false)] {
        let mut chunks = vec![(*b"JUNK", vec![]); count];
        chunks.extend([(*b"fmt ", fmt()), (*b"data", vec![0, 0])]);
        let source = Source::new(riff(chunks)).await;
        let (input, _) = source.input(identity()).await;
        let result =
            RemoteWaveDecoder::open_with(input, source.cpu.clone(), source.pcm.clone()).await;
        if accepted {
            drop(result.unwrap());
        } else {
            assert!(matches!(
                result,
                Err(RemoteWaveError::Media(MediaError::TooLarge))
            ));
        }
        source.finish().await;
    }
    for (length, accepted) in [
        (MAX_WAVE_JUNK_BYTES, true),
        (MAX_WAVE_JUNK_BYTES + 1, false),
    ] {
        let source = Source::new(riff(vec![
            (*b"JUNK", vec![0; length as usize]),
            (*b"fmt ", fmt()),
            (*b"data", vec![0, 0]),
        ]))
        .await;
        let (input, _) = source.input(identity()).await;
        let result =
            RemoteWaveDecoder::open_with(input, source.cpu.clone(), source.pcm.clone()).await;
        if accepted {
            drop(result.unwrap());
        } else {
            assert!(matches!(
                result,
                Err(RemoteWaveError::Media(MediaError::TooLarge))
            ));
        }
        // JUNK body is never downloaded; only its optional padding byte is read.
        assert!(
            source
                .rows
                .lock()
                .unwrap()
                .iter()
                .all(|(_, len)| *len <= 16)
        );
        source.finish().await;
    }
}

#[tokio::test]
async fn remote_wave_late_source_changes_revoke_old_pcm_and_preserve_the_actual_error() {
    for status in [0, 412, 416, 429] {
        let source = Source::new(wave(16000, 1, &samples(100000, 1))).await;
        let (mut decoder, cancel) = source.decoder().await;
        let old = decoder.read_next().await.unwrap().unwrap();
        if status == 0 {
            source.control.changed.store(true, Ordering::Release);
        } else {
            source.control.status.store(status, Ordering::Release);
        }
        let expected = if status == 429 {
            RemoteMediaError::RateLimited
        } else {
            RemoteMediaError::SourceChanged
        };
        assert!(
            matches!(decoder.read_next().await,Err(RemoteWaveError::Remote(error)) if error==expected)
        );
        assert!(cancel.is_cancelled());
        assert!(old.block().is_err());
        assert!(matches!(
            decoder.read_next().await,
            Err(RemoteWaveError::Remote(RemoteMediaError::Cancelled))
        ));
        drop(decoder);
        assert_eq!(source.owners.available_permits(), 1);
        assert_eq!(source.pcm.available_permits(), 3);
        drop(old);
        assert_eq!(source.pcm.available_permits(), 4);
        source.finish().await;
    }
}

#[tokio::test]
async fn remote_wave_truncated_pcm_never_returns_a_partial_block() {
    let source = Source::new(wave(16000, 1, &samples(100000, 1))).await;
    let (mut decoder, cancel) = source.decoder().await;
    source.control.short.store(true, Ordering::Release);
    assert!(matches!(
        decoder.read_next().await,
        Err(RemoteWaveError::Remote(RemoteMediaError::Network))
    ));
    assert!(cancel.is_cancelled());
    assert_eq!(source.pcm.available_permits(), 4);
    assert_eq!(source.buffers.available_permits(), 4);
    drop(decoder);
    source.finish().await;
}

#[path = "remote_wave_lifecycle_tests.rs"]
mod lifecycle;

#[tokio::test]
async fn remote_wave_duration_and_total_range_budget_are_checked_before_pcm_transfer() {
    for (rate, channels, frames, accepted) in [
        (8000u32, 1u16, 86400u64 * 8000, true),
        (8000, 1, 86400 * 8000 + 1, false),
        (
            192000,
            2,
            (super::super::remote_http::MAX_REMOTE_MEDIA_BYTES - 44) / 4,
            false,
        ),
    ] {
        let length = frames * u64::from(channels) * 2;
        let mut header = wave(rate, channels, &[]);
        header[4..8].copy_from_slice(&u32::try_from(length + 36).unwrap().to_le_bytes());
        header[40..44].copy_from_slice(&u32::try_from(length).unwrap().to_le_bytes());
        let source = Source::with_length(header, length + 44).await;
        let (input, cancel) = source.input(identity()).await;
        let result =
            RemoteWaveDecoder::open_with(input, source.cpu.clone(), source.pcm.clone()).await;
        if accepted {
            let decoder = result.unwrap();
            assert_eq!(decoder.total_frames(), frames);
            drop(decoder);
        } else {
            assert!(matches!(
                result,
                Err(RemoteWaveError::Media(MediaError::TooLarge))
            ));
        }
        assert!(cancel.is_cancelled());
        assert_eq!(source.owners.available_permits(), 1);
        assert!(
            source
                .rows
                .lock()
                .unwrap()
                .iter()
                .all(|(offset, len)| offset + *len as u64 <= 44)
        );
        source.finish().await;
    }
}

#[tokio::test]
async fn remote_wave_public_entry_rejects_another_probed_codec_without_new_reads_or_fallback() {
    let mut bytes = vec![0; 834];
    for offset in [0, 417] {
        bytes[offset..offset + 4].copy_from_slice(&[0xff, 0xfb, 0x90, 0xc0]);
    }
    let source = Source::new(bytes).await;
    let (input, cancel) = source.input(identity()).await;
    assert_eq!(input.format(), RemoteAudioFormat::Mp3);
    let reads = source.rows.lock().unwrap().len();
    assert!(matches!(
        RemoteWaveDecoder::open(input).await,
        Err(RemoteWaveError::Media(MediaError::UnsupportedMedia))
    ));
    assert!(cancel.is_cancelled());
    assert_eq!(source.rows.lock().unwrap().len(), reads);
    source.finish().await;
}

#[cfg(windows)]
#[tokio::test]
async fn remote_wave_public_entry_matches_windows_native_pcm_and_existing_preprocessing() {
    use crate::media::prepared_audio::{
        PreparedAudioProcessor, PreparedPcmBlock, PreparedSampleRate,
    };
    let expected = samples(50003, 2);
    let bytes = wave(44100, 2, &expected);
    let source = Source::new(bytes.clone()).await;
    let (input, cancel) = source.input(identity()).await;
    let scope = cancel.clone();
    let runtime = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || {
        let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
        {
            let _cpu = crate::media::work::workers().try_acquire_owned().unwrap();
            let fixture = tempfile::tempdir().unwrap();
            let path = fixture.path().join("self-owned-wave.wav");
            std::fs::write(&path, &bytes).unwrap();
            let local = crate::media::input::MediaInput::local(path.clone()).unwrap();
            let mut native = crate::win32::media_decode::open(
                local,
                identity(),
                ResolverCancellation::default(),
            )
            .unwrap();
            let mut offset = 0;
            while let Some(block) = super::super::decoded::MediaDecoder::next(
                &mut native,
                &ResolverCancellation::default(),
            )
            .unwrap()
            {
                assert_eq!(
                    block.samples(),
                    &expected[offset..offset + block.samples().len()]
                );
                offset += block.samples().len();
            }
            assert_eq!(offset, expected.len());
            drop(native);
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
            assert_eq!(std::fs::read_dir(fixture.path()).unwrap().count(), 1);
        }
        runtime.block_on(async move {
            let mut decoder = RemoteWaveDecoder::open(input).await.unwrap();
            let mut processor = PreparedAudioProcessor::new(
                identity(),
                decoder.format(),
                PreparedSampleRate::Hz16000,
                &scope,
            )
            .unwrap();
            let mut output_frames = 0u64;
            let mut sink = |block: &PreparedPcmBlock, _: &ResolverCancellation| {
                output_frames += block.frames() as u64;
                Ok(())
            };
            while let Some(owner) = decoder.read_next().await.unwrap() {
                let _cpu = crate::media::work::workers().try_acquire_owned().unwrap();
                processor
                    .push(owner.block().unwrap(), &scope, &mut sink)
                    .unwrap();
            }
            let summary = {
                let _cpu = crate::media::work::workers().try_acquire_owned().unwrap();
                processor.finish(&scope, &mut sink).unwrap()
            };
            assert_eq!(summary.frames(), 50003u64 * 16000 / 44100);
            assert_eq!(summary.frames(), output_frames);
            assert_eq!(summary.gaps(), 0);
            assert_eq!(summary.identity(), identity());
        });
    })
    .await
    .unwrap();
    assert!(cancel.is_cancelled());
    assert_eq!(source.owners.available_permits(), 1);
    source.finish().await;
}
