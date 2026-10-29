use super::*;

fn convert(input: &[f64], input_rate: u32, output_rate: u32, chunk: usize) -> Vec<i16> {
    let cancel = ResolverCancellation::default();
    let mut processor = PreparedResampler::new(input_rate, output_rate, &cancel).unwrap();
    let mut output = Vec::new();
    let mut emit = |sample| {
        output.push(sample);
        Ok(())
    };
    for part in input.chunks(chunk) {
        for &sample in part {
            processor.push(sample, &cancel, &mut emit).unwrap();
        }
    }
    processor.finish(&cancel, &mut emit).unwrap();
    assert_eq!(processor.source_frames(), input.len() as u64);
    assert_eq!(processor.output_frames(), output.len() as u64);
    assert_eq!(
        output.len() as u128,
        input.len() as u128 * u128::from(output_rate) / u128::from(input_rate)
    );
    output
}

fn tone(input_rate: u32, hz: f64) -> Vec<f64> {
    (0..input_rate / 5)
        .map(|index| (2.0 * PI * hz * f64::from(index) / f64::from(input_rate)).sin() * 8_000.0)
        .collect()
}

fn rms(input: &[i16]) -> f64 {
    (input
        .iter()
        .map(|&sample| f64::from(sample).powi(2))
        .sum::<f64>()
        / input.len() as f64)
        .sqrt()
}

fn tone_amplitude(input: &[i16], rate: u32, hz: f64) -> f64 {
    let (sin, cos) = input
        .iter()
        .enumerate()
        .fold((0.0, 0.0), |(sin, cos), (index, &sample)| {
            let angle = 2.0 * PI * hz * index as f64 / f64::from(rate);
            (
                sin + f64::from(sample) * angle.sin(),
                cos + f64::from(sample) * angle.cos(),
            )
        });
    2.0 * sin.hypot(cos) / input.len() as f64
}

#[test]
fn representative_and_odd_rates_have_exact_floor_frame_counts() {
    for input_rate in [
        8_000, 16_000, 24_000, 44_100, 48_000, 96_000, 191_999, 192_000,
    ] {
        for output_rate in [16_000, 24_000] {
            let output = convert(&vec![0.0; 1_001], input_rate, output_rate, 137);
            assert!(output.iter().all(|sample| *sample == 0));
        }
    }
}

#[test]
fn arbitrary_external_input_partitioning_preserves_filter_state() {
    let input = tone(44_100, 1_731.0);
    for output_rate in [16_000, 24_000] {
        let regular = convert(&input, 44_100, output_rate, 960);
        for chunk in [1, 137, 959, 32_768] {
            assert_eq!(regular, convert(&input, 44_100, output_rate, chunk));
        }
    }
}

#[test]
fn dc_and_pcm_extrema_survive_both_edges_and_all_phases() {
    for (input_rate, output_rate) in [(8_000, 24_000), (44_100, 16_000), (192_000, 16_000)] {
        for value in [-32768.0, -1.0, 0.0, 1.0, 32767.0] {
            let output = convert(&vec![value; 777], input_rate, output_rate, 137);
            assert!(output.iter().all(|sample| *sample == value as i16));
        }
    }
}

#[test]
fn same_rate_bypass_is_exact_without_filter_startup_or_tail() {
    let input: Vec<_> = (-32768_i32..=32767).step_by(97).map(f64::from).collect();
    for rate in [16_000, 24_000] {
        let output = convert(&input, rate, rate, 1);
        assert_eq!(
            output,
            input
                .iter()
                .map(|sample| *sample as i16)
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn same_rate_push_delivers_immediately_and_does_not_allocate_a_table() {
    let cancel = ResolverCancellation::default();
    let mut processor = PreparedResampler::new(16_000, 16_000, &cancel).unwrap();
    let mut output = Vec::new();
    processor
        .push(123.0, &cancel, &mut |sample| {
            output.push(sample);
            Ok(())
        })
        .unwrap();
    assert_eq!(output, [123]);
    assert!(processor.coefficients.is_empty());
    assert_eq!(processor.source_frames(), 1);
    assert_eq!(processor.output_frames(), 1);
}

#[test]
fn centered_filter_waits_for_bounded_lookahead_and_finishes_exact_tail() {
    let cancel = ResolverCancellation::default();
    let mut processor = PreparedResampler::new(48_000, 16_000, &cancel).unwrap();
    let mut output = Vec::new();
    for _ in 0..FILTER_RADIUS {
        processor
            .push(1_000.0, &cancel, &mut |sample| {
                output.push(sample);
                Ok(())
            })
            .unwrap();
    }
    assert!(output.is_empty());
    processor
        .push(1_000.0, &cancel, &mut |sample| {
            output.push(sample);
            Ok(())
        })
        .unwrap();
    assert_eq!(output, [1_000]);
    processor
        .finish(&cancel, &mut |sample| {
            output.push(sample);
            Ok(())
        })
        .unwrap();
    assert_eq!(output.len(), (FILTER_RADIUS + 1) / 3);
    assert!(output.iter().all(|sample| *sample == 1_000));
}

#[test]
fn empty_and_short_eof_use_floor_duration_without_extra_output() {
    assert!(convert(&[], 48_000, 16_000, 1).is_empty());
    assert!(convert(&[123.0], 192_000, 16_000, 1).is_empty());
    assert!(convert(&[123.0; 11], 192_000, 16_000, 1).is_empty());
    assert_eq!(convert(&[123.0; 12], 192_000, 16_000, 1), [123]);
    assert_eq!(convert(&[123.0], 8_000, 24_000, 1), [123; 3]);
}

#[test]
fn output_position_remains_exact_rational_for_large_frame_indices() {
    let cancel = ResolverCancellation::default();
    let mut processor = PreparedResampler::new(44_100, 24_000, &cancel).unwrap();
    for count in [0, 1, 2, 10_000_001, u64::MAX / 4] {
        processor.output_frames = count;
        let numerator = u128::from(count) * 44_100;
        let (center, phase) = processor.position().unwrap();
        assert_eq!(u128::from(center), numerator / 24_000);
        assert_eq!(
            phase as u128,
            numerator % 24_000 * FILTER_PHASES as u128 / 24_000
        );
    }
}

#[test]
fn passband_and_downsample_alias_attenuation_include_worst_rate_ratio() {
    // These tone checks document tested frequency points, not a promise about
    // every frequency in the finite filter's transition band.
    for (input_rate, output_rate, stop_hz) in [
        (48_000, 16_000, 12_000.0),
        (44_100, 16_000, 12_000.0),
        (96_000, 24_000, 30_000.0),
        (192_000, 16_000, 12_000.0),
        (192_000, 24_000, 18_000.0),
    ] {
        let pass = convert(&tone(input_rate, 1_000.0), input_rate, output_rate, 137);
        let stop = convert(&tone(input_rate, stop_hz), input_rate, output_rate, 137);
        let pass_rms = rms(&pass[256..pass.len() - 256]);
        let stop_rms = rms(&stop[256..stop.len() - 256]);
        assert!(
            (5_600.0..5_720.0).contains(&pass_rms),
            "{input_rate}->{output_rate}: pass RMS {pass_rms}"
        );
        assert!(
            stop_rms < 8.0,
            "{input_rate}->{output_rate}: stop RMS {stop_rms}"
        );
    }
}

#[test]
fn upsample_preserves_passband_and_attenuates_image_frequencies() {
    for output_rate in [16_000, 24_000] {
        let output = convert(&tone(8_000, 1_000.0), 8_000, output_rate, 137);
        let body = &output[256..output.len() - 256];
        let pass = tone_amplitude(body, output_rate, 1_000.0);
        let image = tone_amplitude(body, output_rate, 7_000.0);
        assert!((7_900.0..8_100.0).contains(&pass), "pass amplitude {pass}");
        assert!(image < 16.0, "image amplitude {image}");
    }
}

#[test]
fn phase_table_and_history_have_fixed_capacity_even_for_odd_rates() {
    let cancel = ResolverCancellation::default();
    let processor = PreparedResampler::new(191_999, 24_000, &cancel).unwrap();
    assert_eq!(processor.coefficients.len(), FILTER_PHASES);
    assert_eq!(processor.history.len(), FILTER_TAPS);
    assert_eq!(
        processor.coefficients.len() * size_of::<[f64; FILTER_TAPS]>(),
        526_336
    );
    for row in processor.coefficients.iter() {
        assert!(row.iter().all(|value| value.is_finite()));
        assert!((row.iter().sum::<f64>() - 1.0).abs() < 1e-12);
    }
}

#[test]
fn invalid_rates_and_precancelled_initialization_are_typed() {
    let cancel = ResolverCancellation::default();
    for (input_rate, output_rate) in [
        (7_999, 16_000),
        (192_001, 24_000),
        (48_000, 8_000),
        (48_000, 48_000),
    ] {
        assert_eq!(
            PreparedResampler::new(input_rate, output_rate, &cancel).err(),
            Some(MediaError::InvalidAudio)
        );
    }
    cancel.cancel();
    assert_eq!(
        PreparedResampler::new(48_000, 16_000, &cancel).err(),
        Some(MediaError::Cancelled)
    );
}

#[test]
fn invalid_samples_seal_the_processor_without_emitting() {
    for sample in [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        -32769.0,
        32768.0,
    ] {
        let cancel = ResolverCancellation::default();
        let mut processor = PreparedResampler::new(16_000, 16_000, &cancel).unwrap();
        let mut emit = |_| panic!("invalid input must not emit");
        assert_eq!(
            processor.push(sample, &cancel, &mut emit),
            Err(MediaError::InvalidAudio)
        );
        assert_eq!(processor.source_frames(), 0);
        assert_eq!(
            processor.push(0.0, &cancel, &mut emit),
            Err(MediaError::InvalidAudio)
        );
        assert_eq!(
            processor.finish(&cancel, &mut emit),
            Err(MediaError::InvalidAudio)
        );
    }
}

#[test]
fn finished_processor_refuses_duplicate_finish_and_further_input() {
    let cancel = ResolverCancellation::default();
    let mut processor = PreparedResampler::new(48_000, 16_000, &cancel).unwrap();
    processor.finish(&cancel, &mut |_| Ok(())).unwrap();
    assert_eq!(
        processor.finish(&cancel, &mut |_| Ok(())),
        Err(MediaError::InvalidAudio)
    );
    assert_eq!(
        processor.push(0.0, &cancel, &mut |_| Ok(())),
        Err(MediaError::InvalidAudio)
    );
}

#[test]
fn cancellation_before_push_or_finish_never_emits_and_seals_state() {
    for at_finish in [false, true] {
        let cancel = ResolverCancellation::default();
        let mut processor = PreparedResampler::new(48_000, 16_000, &cancel).unwrap();
        for _ in 0..12 {
            processor.push(100.0, &cancel, &mut |_| Ok(())).unwrap();
        }
        cancel.cancel();
        let mut emit = |_| panic!("cancelled state must not emit");
        let result = if at_finish {
            processor.finish(&cancel, &mut emit)
        } else {
            processor.push(0.0, &cancel, &mut emit)
        };
        assert_eq!(result, Err(MediaError::Cancelled));
        assert!(processor.failed);
    }
}

#[test]
fn cancellation_during_callback_stops_before_the_next_tail_sample() {
    let cancel = ResolverCancellation::default();
    let mut processor = PreparedResampler::new(8_000, 24_000, &cancel).unwrap();
    processor.push(100.0, &cancel, &mut |_| Ok(())).unwrap();
    let mut calls = 0;
    let result = processor.finish(&cancel, &mut |_| {
        calls += 1;
        cancel.cancel();
        Ok(())
    });
    assert_eq!(result, Err(MediaError::Cancelled));
    assert_eq!(calls, 1);
    assert_eq!(processor.output_frames(), 1);
}

#[test]
fn callback_error_is_preserved_and_prevents_future_output() {
    for bypass in [false, true] {
        let cancel = ResolverCancellation::default();
        let input_rate = if bypass { 16_000 } else { 8_000 };
        let mut processor = PreparedResampler::new(input_rate, 16_000, &cancel).unwrap();
        let mut calls = 0;
        let mut failed_emit = |_| {
            calls += 1;
            Err(MediaError::Stale)
        };
        let result = if bypass {
            processor.push(100.0, &cancel, &mut failed_emit)
        } else {
            processor.push(100.0, &cancel, &mut |_| Ok(())).unwrap();
            processor.finish(&cancel, &mut failed_emit)
        };
        assert_eq!(result, Err(MediaError::Stale));
        assert_eq!(calls, 1);
        assert_eq!(processor.output_frames(), 0);
        assert_eq!(
            processor.push(0.0, &cancel, &mut |_| panic!("failed state must not emit")),
            Err(MediaError::InvalidAudio)
        );
    }
}

#[test]
fn frame_counter_and_output_limit_overflow_are_typed_and_terminal() {
    let cancel = ResolverCancellation::default();
    let mut processor = PreparedResampler::new(16_000, 16_000, &cancel).unwrap();
    processor.source_frames = u64::MAX;
    assert_eq!(
        processor.push(0.0, &cancel, &mut |_| panic!("overflow must not emit")),
        Err(MediaError::InvalidAudio)
    );
    let mut processor = PreparedResampler::new(8_000, 24_000, &cancel).unwrap();
    processor.source_frames = u64::MAX / 2;
    assert_eq!(
        processor.finish(&cancel, &mut |_| panic!("overflow must not emit")),
        Err(MediaError::InvalidAudio)
    );
}
