//! Actual synthesized MP3 -> explicit sample clock -> unchanged bounded DSP.
//! This proof does not enable the product ASR route or establish playback sync.
use super::*;
use crate::{
    audio::capture::SessionId,
    media::{
        decoded_clock::{DecodedClockNormalizer, DecodedClockWindow},
        prepared_audio::{PreparedAudioProcessor, PreparedPcmBlock, PreparedSampleRate},
    },
};
use std::{fs, fs::OpenOptions};

fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(351),
        generation: 4,
    }
}
fn checksum(samples: &[i16]) -> u64 {
    samples.iter().fold(0u64, |hash, sample| {
        hash.rotate_left(7) ^ u64::from(*sample as u16)
    })
}

#[test]
fn mp3_clock_map_preserves_native_evidence_and_exact_dsp_frames_at_both_rates() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    for rate in [32_000, 44_100, 48_000] {
        for channels in [1, 2] {
            let bytes =
                crate::win32::media_mp3_fixture::synthetic_mp3(rate, channels, 2 * rate as usize)
                    .unwrap();
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("synthetic-clock.bin");
            fs::write(&path, &bytes).unwrap();
            for output_rate in [PreparedSampleRate::Hz16000, PreparedSampleRate::Hz24000] {
                let cancel = ResolverCancellation::default();
                let (mut decoder, pin) = open_native_pinned(
                    MediaInput::local(path.clone()).unwrap(),
                    identity(),
                    cancel.clone(),
                )
                .unwrap();
                let mut clock =
                    DecodedClockNormalizer::new(identity(), decoder.format(), &cancel).unwrap();
                let mut window = DecodedClockWindow::new(identity(), &cancel).unwrap();
                let mut dsp =
                    PreparedAudioProcessor::new(identity(), decoder.format(), output_rate, &cancel)
                        .unwrap();
                let mut input_runs = Vec::<u64>::new();
                let mut native_gaps = 0;
                let mut drift_observed = false;
                let mut output_runs = Vec::<u64>::new();
                let mut output_last_native = 0;
                let mut squares = 0f64;
                let mut last_output_sample = 0;
                while let Some(block) = decoder.next(&cancel).unwrap() {
                    let source_pointer = block.samples().as_ptr();
                    let source_hash = checksum(block.samples());
                    let native_start = block.timestamp_us();
                    let native_end = block.end_us();
                    let frames = block.frames() as u64;
                    let mapped = clock.normalize(block, &cancel).unwrap();
                    let span = mapped.span();
                    let normalized = mapped.sample_clock_block();
                    assert_eq!(normalized.samples().as_ptr(), source_pointer);
                    assert_eq!(checksum(normalized.samples()), source_hash);
                    assert_eq!(normalized.frames() as u64, frames);
                    assert_eq!(span.native_start_us(), native_start);
                    assert_eq!(span.native_end_us(), native_end);
                    assert_eq!(normalized.timestamp_us(), span.sample_start_us());
                    assert!(normalized.end_us().abs_diff(span.sample_end_us()) <= 1);
                    drift_observed |= span.sample_start_us().abs_diff(native_start) > 1;
                    if span.discontinuity() {
                        native_gaps += 1;
                    }
                    if input_runs.len() < span.run_id() as usize {
                        input_runs.push(0);
                    }
                    *input_runs.last_mut().unwrap() += frames;
                    window.append(span, &cancel).unwrap();
                    let interval = window
                        .map_interval(
                            identity(),
                            span.run_id(),
                            span.sample_start_us(),
                            span.sample_end_us(),
                            &cancel,
                        )
                        .unwrap();
                    assert_eq!(interval.identity(), identity());
                    assert_eq!(interval.run_id(), span.run_id());
                    assert_eq!(interval.first_source_sequence(), span.sequence());
                    assert_eq!(interval.last_source_sequence(), span.sequence());
                    assert_eq!(
                        (interval.start_us(), interval.end_us()),
                        (native_start, native_end)
                    );
                    let mut emit = |output: &PreparedPcmBlock, scope: &ResolverCancellation| {
                        record(
                            output,
                            scope,
                            &mut window,
                            &mut output_runs,
                            &mut output_last_native,
                            &mut last_output_sample,
                            &mut squares,
                        )
                    };
                    dsp.push(normalized, &cancel, &mut emit).unwrap();
                }
                let mut emit = |output: &PreparedPcmBlock, scope: &ResolverCancellation| {
                    record(
                        output,
                        scope,
                        &mut window,
                        &mut output_runs,
                        &mut output_last_native,
                        &mut last_output_sample,
                        &mut squares,
                    )
                };
                let summary = dsp.finish(&cancel, &mut emit).unwrap();
                assert_eq!(summary.gaps(), native_gaps);
                assert!(native_gaps > 0);
                assert_eq!(input_runs.len(), output_runs.len());
                for (input_frames, output_frames) in input_runs.iter().zip(&output_runs) {
                    assert_eq!(
                        *output_frames,
                        input_frames * u64::from(output_rate.hz()) / u64::from(rate)
                    );
                }
                assert_eq!(summary.frames(), output_runs.iter().sum::<u64>());
                assert_eq!(summary.last_end_us(), last_output_sample);
                let rms = (squares / summary.frames() as f64).sqrt();
                assert!(
                    (3000.0..5500.0).contains(&rms),
                    "decoded/resampled tone RMS {rms}"
                );
                // Two seconds of real 44.1 kHz packet cadence previously failed
                // unchanged DSP. The explicit map now preserves that drift as
                // native metadata while its sample clock passes the same gate.
                if rate == 44_100 {
                    assert!(drift_observed);
                }
                drop(decoder);
                assert!(OpenOptions::new().write(true).open(&path).is_err());
                drop(pin);
                assert!(OpenOptions::new().write(true).open(&path).is_ok());
                assert_eq!(fs::read(&path).unwrap(), bytes);
                assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
            }
        }
    }
}

fn record(
    output: &PreparedPcmBlock,
    cancel: &ResolverCancellation,
    window: &mut DecodedClockWindow,
    runs: &mut Vec<u64>,
    last_native: &mut u64,
    last_sample: &mut u64,
    squares: &mut f64,
) -> Result<(), MediaError> {
    if runs.is_empty() || output.discontinuity() {
        runs.push(0);
    }
    assert_eq!(output.identity(), identity());
    assert!(output.frames() <= output.sample_rate() as usize / 50);
    let interval = window.map_interval(
        identity(),
        runs.len() as u64,
        output.timestamp_us(),
        output.end_us(),
        cancel,
    )?;
    // Tolerate only the existing native endpoint rounding, never accumulated
    // drift, fake silence or provider guessed timing. Every output has evidence.
    assert_eq!(interval.identity(), identity());
    assert_eq!(interval.run_id(), runs.len() as u64);
    assert!(interval.first_source_sequence() <= interval.last_source_sequence());
    assert!(interval.start_us() + 1 >= *last_native);
    *last_native = interval.end_us();
    *last_sample = output.end_us();
    *runs.last_mut().unwrap() += output.frames() as u64;
    *squares += output
        .samples()
        .iter()
        .map(|sample| f64::from(*sample).powi(2))
        .sum::<f64>();
    Ok(())
}

#[test]
fn mp3_clock_mapping_rolls_past_twelve_seconds_without_retaining_old_evidence() {
    use crate::media::decoded_clock::MAX_CLOCK_WINDOW_DURATION_US;
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    let bytes = crate::win32::media_mp3_fixture::synthetic_mp3(44_100, 2, 13 * 44_100).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("synthetic-long-clock.bin");
    fs::write(&path, &bytes).unwrap();
    for rate in [PreparedSampleRate::Hz16000, PreparedSampleRate::Hz24000] {
        let cancel = ResolverCancellation::default();
        let (mut decoder, pin) = open_native_pinned(
            MediaInput::local(path.clone()).unwrap(),
            identity(),
            cancel.clone(),
        )
        .unwrap();
        let mut clock = DecodedClockNormalizer::new(identity(), decoder.format(), &cancel).unwrap();
        let mut window = DecodedClockWindow::new(identity(), &cancel).unwrap();
        let mut dsp =
            PreparedAudioProcessor::new(identity(), decoder.format(), rate, &cancel).unwrap();
        let mut input_runs = Vec::<u64>::new();
        let mut output_runs = Vec::<u64>::new();
        let mut last_native = 0;
        let mut last_sample = 0;
        let mut squares = 0f64;
        let mut final_span = None;
        let mut first_span = None;
        while let Some(block) = decoder.next(&cancel).unwrap() {
            let frames = block.frames() as u64;
            let mapped = clock.normalize(block, &cancel).unwrap();
            let span = mapped.span();
            if input_runs.len() < span.run_id() as usize {
                input_runs.push(0);
            }
            *input_runs.last_mut().unwrap() += frames;
            // No outstanding ASR request exists in this synchronous DSP proof.
            // The last emitted sample endpoint is safe to prune; retain the
            // straddling span for the real FIR lookahead and EOF tail.
            if last_sample > 0 {
                window.prune_before(last_sample, &cancel).unwrap();
            }
            window.append(span, &cancel).unwrap();
            first_span.get_or_insert(span);
            final_span = Some(span);
            dsp.push(
                mapped.sample_clock_block(),
                &cancel,
                &mut |output, scope| {
                    record(
                        output,
                        scope,
                        &mut window,
                        &mut output_runs,
                        &mut last_native,
                        &mut last_sample,
                        &mut squares,
                    )
                },
            )
            .unwrap();
        }
        let summary = dsp
            .finish(&cancel, &mut |output, scope| {
                record(
                    output,
                    scope,
                    &mut window,
                    &mut output_runs,
                    &mut last_native,
                    &mut last_sample,
                    &mut squares,
                )
            })
            .unwrap();
        let last = final_span.unwrap();
        assert!(last.sample_end_us() > MAX_CLOCK_WINDOW_DURATION_US);
        assert!(last.native_end_us() > MAX_CLOCK_WINDOW_DURATION_US);
        assert!(last.sample_end_us().abs_diff(last.native_end_us()) > 10);
        assert_eq!(input_runs.len(), output_runs.len());
        for (input, output) in input_runs.iter().zip(&output_runs) {
            assert_eq!(*output, input * u64::from(rate.hz()) / 44_100);
        }
        assert_eq!(summary.frames(), output_runs.iter().sum::<u64>());
        assert_eq!(summary.last_end_us(), last_sample);
        // The actual resampling tail is shorter than one target frame; it is
        // mapped where it ends, never stretched/extrapolated to native EOF.
        assert!(last.native_end_us() >= last_native);
        assert!(
            last.native_end_us() - last_native <= 1_000_000u64.div_ceil(u64::from(rate.hz())) + 1
        );
        let first = first_span.unwrap();
        assert_eq!(
            window.map_interval(
                identity(),
                first.run_id(),
                first.sample_start_us(),
                first.sample_end_us(),
                &cancel
            ),
            Err(MediaError::InvalidAudio)
        );
        drop(decoder);
        assert!(OpenOptions::new().write(true).open(&path).is_err());
        drop(pin);
        assert!(OpenOptions::new().write(true).open(&path).is_ok());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}

#[test]
fn mp3_clock_mapping_cancel_keeps_native_lifecycle_and_new_session_isolation() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    let bytes = crate::win32::media_mp3_fixture::synthetic_mp3(44_100, 2, 88_200).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("synthetic-cancel.bin");
    fs::write(&path, &bytes).unwrap();
    let cancel = ResolverCancellation::default();
    let mut decoder = open(
        MediaInput::local(path.clone()).unwrap(),
        identity(),
        cancel.clone(),
    )
    .unwrap();
    let mut clock = DecodedClockNormalizer::new(identity(), decoder.format(), &cancel).unwrap();
    let mut window = DecodedClockWindow::new(identity(), &cancel).unwrap();
    let mapped = clock
        .normalize(decoder.next(&cancel).unwrap().unwrap(), &cancel)
        .unwrap();
    let span = mapped.span();
    window.append(span, &cancel).unwrap();
    cancel.cancel();
    let fresh = ResolverCancellation::default();
    assert_eq!(
        window.map_interval(
            identity(),
            span.run_id(),
            span.sample_start_us(),
            span.sample_end_us(),
            &fresh
        ),
        Err(MediaError::Cancelled)
    );
    assert_eq!(decoder.next(&fresh).err(), Some(MediaError::Cancelled));
    drop(mapped);
    drop(decoder);
    assert!(OpenOptions::new().write(true).open(&path).is_ok());
    let new_identity = StreamIdentity {
        session: SessionId(352),
        generation: 5,
    };
    let mut decoder = open(
        MediaInput::local(path.clone()).unwrap(),
        new_identity,
        fresh.clone(),
    )
    .unwrap();
    let mut clock = DecodedClockNormalizer::new(new_identity, decoder.format(), &fresh).unwrap();
    let mapped = clock
        .normalize(decoder.next(&fresh).unwrap().unwrap(), &fresh)
        .unwrap();
    assert_eq!(mapped.span().identity(), new_identity);
    assert_eq!(mapped.span().sequence(), 1);
    assert_eq!(mapped.span().run_id(), 1);
    let mut new_window = DecodedClockWindow::new(new_identity, &fresh).unwrap();
    assert_eq!(new_window.append(span, &fresh), Err(MediaError::Stale));
    drop(mapped);
    drop(decoder);
    assert!(OpenOptions::new().write(true).open(&path).is_ok());
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}
