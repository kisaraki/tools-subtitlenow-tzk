//! Synthetic RAM-only fixtures for the private PCM ownership bridge. Public
//! borrowed callbacks keep their existing no-retention/no-network contract.
use super::*;
use crate::{audio::capture::SessionId, media::prepared_segments::PreparedSegmentBuilder};

#[test]
fn zero_output_native_runs_keep_separate_chunk_clock_identity_at_both_rates() {
    use crate::{
        ai::{
            prepared_speech::{PreparedSpeechCue, PreparedSpeechRequest, PreparedSpeechResponse},
            prepared_transcript::PreparedTranscriptMerger,
            speech::SpeechLanguage,
        },
        domain::SourceLanguage,
        media::decoded_clock::{DecodedClockNormalizer, DecodedClockWindow},
    };
    for rate in [PreparedSampleRate::Hz16000, PreparedSampleRate::Hz24000] {
        let cancel = ResolverCancellation::default();
        let native = format(192_000, 1);
        let mut clock = DecodedClockNormalizer::new(identity(), native, &cancel).unwrap();
        let mut window = DecodedClockWindow::new(identity(), &cancel).unwrap();
        let mut dsp = processor(native, rate, &cancel);
        let mut builder = PreparedSegmentBuilder::new(identity(), rate).unwrap();
        let mut chunks = Vec::new();
        for (index, (start, frames)) in
            [(0, 1), (1_000_000, 1920), (2_000_000, 1), (3_000_000, 1920)]
                .into_iter()
                .enumerate()
        {
            let mapped = clock
                .normalize(
                    input(index as u64 + 1, start, native, vec![123; frames]),
                    &cancel,
                )
                .unwrap();
            window.append(mapped.span(), &cancel).unwrap();
            dsp.push_owned(
                mapped.sample_clock_block(),
                &cancel,
                &mut |block, gap, _| {
                    let gap = gap && block.sequence() != 1;
                    builder.push(block, gap, &cancel).unwrap();
                    Ok(())
                },
            )
            .unwrap();
        }
        let summary = dsp
            .finish_owned(&cancel, &mut |block, gap, _| {
                let gap = gap && block.sequence() != 1;
                builder.push(block, gap, &cancel).unwrap();
                Ok(())
            })
            .unwrap();
        builder.finish(&cancel).unwrap();
        while let Some(chunk) = builder.take() {
            let stamp = chunk.stamp();
            let map = window
                .snapshot(
                    identity(),
                    stamp.audio_start_us,
                    chunk.audio_end_us(),
                    &cancel,
                )
                .unwrap();
            chunks.push(chunk.with_clock_map(map).unwrap());
        }
        assert_eq!(summary.frames(), 2 * u64::from(rate.hz()) / 100);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].stamp().run_id, 1);
        assert_eq!(chunks[0].source_clock().unwrap().source_run_id(), 2);
        assert_eq!(chunks[1].stamp().run_id, 2);
        assert_eq!(chunks[1].source_clock().unwrap().source_run_id(), 4);
        drop(window);
        drop(clock);
        let mut merger =
            PreparedTranscriptMerger::new(identity(), SourceLanguage::English).unwrap();
        for chunk in chunks {
            let request =
                PreparedSpeechRequest::new(chunk, SourceLanguage::English, Vec::new()).unwrap();
            let stamp = request.chunk().stamp();
            merger.register(&request).unwrap();
            merger
                .complete(
                    &request,
                    PreparedSpeechResponse {
                        identity: identity(),
                        sequence: stamp.sequence,
                        run_id: stamp.run_id,
                        language: Some(SpeechLanguage::English),
                        cues: vec![PreparedSpeechCue {
                            start_us: 0,
                            duration_us: 10_000,
                            text: "explicit bounded synthetic source".into(),
                        }],
                    },
                )
                .unwrap();
        }
        let transcript = merger.finish().unwrap();
        assert_eq!(transcript.cues()[0].start_us, 1_000_000);
        assert_eq!(transcript.cues()[1].start_us, 3_000_000);
    }
}

fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(907),
        generation: 8,
    }
}
fn format(rate: u32, channels: u16) -> DecodedAudioFormat {
    DecodedAudioFormat::new(rate, channels).unwrap()
}
fn input(
    sequence: u64,
    timestamp_us: u64,
    format: DecodedAudioFormat,
    samples: Vec<i16>,
) -> DecodedAudioBlock {
    DecodedAudioBlock::new(identity(), sequence, timestamp_us, format, samples).unwrap()
}
fn processor(
    native: DecodedAudioFormat,
    rate: PreparedSampleRate,
    cancel: &ResolverCancellation,
) -> PreparedAudioProcessor {
    PreparedAudioProcessor::new(identity(), native, rate, cancel).unwrap()
}
fn discard_owned(
    _: DecodedAudioBlock,
    _: bool,
    _: &ResolverCancellation,
) -> Result<(), MediaError> {
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    sequence: u64,
    timestamp_us: u64,
    duration_us: u64,
    end_us: u64,
    format: DecodedAudioFormat,
    discontinuity: bool,
    samples: Vec<i16>,
}
fn decoded_snapshot(block: &DecodedAudioBlock, discontinuity: bool) -> Snapshot {
    assert_eq!(block.identity(), identity());
    Snapshot {
        sequence: block.sequence(),
        timestamp_us: block.timestamp_us(),
        duration_us: block.duration_us(),
        end_us: block.end_us(),
        format: block.format(),
        discontinuity,
        samples: block.samples().to_vec(),
    }
}

#[test]
fn private_owned_and_public_borrowed_outputs_match_at_both_target_rates() {
    for rate in [PreparedSampleRate::Hz16000, PreparedSampleRate::Hz24000] {
        let cancel = ResolverCancellation::default();
        let native = format(44_100, 2);
        let mut borrowed = processor(native, rate, &cancel);
        let mut owned = processor(native, rate, &cancel);
        let mut expected = Vec::new();
        let mut actual = Vec::new();
        let mut borrowed_emit = |block: &PreparedPcmBlock, _: &ResolverCancellation| {
            expected.push(decoded_snapshot(&block.decoded, block.discontinuity));
            Ok(())
        };
        let mut owned_emit = |block: DecodedAudioBlock, gap, _: &ResolverCancellation| {
            actual.push((block, gap));
            Ok(())
        };
        let frames = 2203;
        let mut offset = 0;
        let mut sequence = 1;
        while offset < frames {
            let count = 137.min(frames - offset);
            let samples = (offset..offset + count)
                .flat_map(|frame| {
                    let sample = ((frame % 2001) - 1000) as i16;
                    [sample, sample / 2]
                })
                .collect();
            let block = input(
                sequence,
                frame_time(371, offset as u64, native.sample_rate()).unwrap(),
                native,
                samples,
            );
            borrowed.push(&block, &cancel, &mut borrowed_emit).unwrap();
            owned.push_owned(&block, &cancel, &mut owned_emit).unwrap();
            offset += count;
            sequence += 1;
        }
        let expected_summary = borrowed.finish(&cancel, &mut borrowed_emit).unwrap();
        let actual_summary = owned.finish_owned(&cancel, &mut owned_emit).unwrap();
        let actual: Vec<_> = actual
            .iter()
            .map(|(block, gap)| decoded_snapshot(block, *gap))
            .collect();
        assert_eq!(expected, actual);
        assert_eq!(expected_summary.identity(), actual_summary.identity());
        assert_eq!(expected_summary.sample_rate(), actual_summary.sample_rate());
        assert_eq!(expected_summary.blocks(), actual_summary.blocks());
        assert_eq!(expected_summary.frames(), actual_summary.frames());
        assert_eq!(
            expected_summary.first_timestamp_us(),
            actual_summary.first_timestamp_us()
        );
        assert_eq!(expected_summary.last_end_us(), actual_summary.last_end_us());
        assert_eq!(expected_summary.gaps(), actual_summary.gaps());
        assert!(owned.terminal && owned.run.is_none());
        assert!(borrowed.terminal && borrowed.run.is_none());
    }
}

#[test]
fn owned_emission_moves_the_existing_pending_allocation() {
    for rate in [PreparedSampleRate::Hz16000, PreparedSampleRate::Hz24000] {
        let cancel = ResolverCancellation::default();
        let native = format(rate.hz(), 1);
        let mut processor = processor(native, rate, &cancel);
        let full = rate.block_frames();
        processor
            .push_owned(
                &input(1, 90, native, vec![81; full - 1]),
                &cancel,
                &mut discard_owned,
            )
            .unwrap();
        let allocation = processor.run.as_ref().unwrap().pending.as_ptr();
        let mut output = None;
        processor
            .push_owned(
                &input(
                    2,
                    frame_time(90, full as u64 - 1, rate.hz()).unwrap(),
                    native,
                    vec![82],
                ),
                &cancel,
                &mut |block, gap, _| {
                    assert!(!gap);
                    output = Some(block);
                    Ok(())
                },
            )
            .unwrap();
        let block = output.take().unwrap();
        assert_eq!(block.samples().as_ptr(), allocation);
        assert_eq!(block.frames(), full);
        assert_eq!(block.samples()[full - 1], 82);
        assert!(
            block.samples()[..full - 1]
                .iter()
                .all(|sample| *sample == 81)
        );
        processor.finish_owned(&cancel, &mut discard_owned).unwrap();
        drop(processor);
        // The consumer owns the allocation after its source/DSP owner is gone.
        assert_eq!(block.sequence(), 1);
        assert_eq!(block.timestamp_us(), 90);
        assert_eq!(block.samples().as_ptr(), allocation);
    }
}

#[test]
fn owned_eof_delivers_real_partial_tail_without_reusing_moved_output() {
    for rate in [PreparedSampleRate::Hz16000, PreparedSampleRate::Hz24000] {
        let cancel = ResolverCancellation::default();
        let native = format(rate.hz(), 1);
        let full = rate.block_frames();
        let mut processor = processor(native, rate, &cancel);
        let mut output = Vec::new();
        let mut emit = |block, gap, _: &ResolverCancellation| {
            output.push((block, gap));
            Ok(())
        };
        processor
            .push_owned(
                &input(1, 0, native, vec![391; full + 17]),
                &cancel,
                &mut emit,
            )
            .unwrap();
        let summary = processor.finish_owned(&cancel, &mut emit).unwrap();
        assert_eq!(output.len(), 2);
        assert_eq!(output[0].0.frames(), full);
        assert_eq!(output[1].0.frames(), 17);
        assert_eq!(output[0].0.sequence(), 1);
        assert_eq!(output[1].0.sequence(), 2);
        assert_eq!(output[1].0.timestamp_us(), 20_000);
        assert_eq!(
            output[1].0.end_us(),
            frame_time(0, (full + 17) as u64, rate.hz()).unwrap()
        );
        assert!(
            output
                .iter()
                .all(|(block, gap)| !gap && block.samples().iter().all(|sample| *sample == 391))
        );
        assert_eq!(summary.frames(), (full + 17) as u64);
        assert_eq!(summary.last_end_us(), output[1].0.end_us());
        assert!(processor.run.is_none());
    }
}

#[test]
fn owned_gap_flushes_then_resets_filter_and_marks_only_new_run() {
    for rate in [PreparedSampleRate::Hz16000, PreparedSampleRate::Hz24000] {
        let cancel = ResolverCancellation::default();
        let native = format(44_100, 1);
        let mut processor = processor(native, rate, &cancel);
        let mut output = Vec::new();
        let mut emit = |block, gap, _: &ResolverCancellation| {
            output.push((block, gap));
            Ok(())
        };
        processor
            .push_owned(&input(1, 100, native, vec![111; 100]), &cancel, &mut emit)
            .unwrap();
        processor
            .push_owned(
                &input(2, 1_000_100, native, vec![-222; 100]),
                &cancel,
                &mut emit,
            )
            .unwrap();
        let summary = processor.finish_owned(&cancel, &mut emit).unwrap();
        assert_eq!(output.len(), 2);
        assert!(!output[0].1);
        assert!(output[1].1);
        assert_eq!(output[0].0.timestamp_us(), 100);
        assert_eq!(output[1].0.timestamp_us(), 1_000_100);
        assert_eq!(output[0].0.sequence(), 1);
        assert_eq!(output[1].0.sequence(), 2);
        assert!(output[0].0.samples().iter().all(|sample| *sample == 111));
        assert!(output[1].0.samples().iter().all(|sample| *sample == -222));
        assert_eq!(output[0].0.frames(), 100 * rate.hz() as usize / 44_100);
        assert_eq!(summary.gaps(), 1);
        assert_eq!(
            summary.frames(),
            (output[0].0.frames() + output[1].0.frames()) as u64
        );
    }
}

#[test]
fn owned_callback_error_is_preserved_and_clears_dsp() {
    let cancel = ResolverCancellation::default();
    let native = format(24_000, 1);
    let mut processor = processor(native, PreparedSampleRate::Hz24000, &cancel);
    let mut calls = 0;
    assert_eq!(
        processor.push_owned(
            &input(1, 0, native, vec![3; 2000]),
            &cancel,
            &mut |_, _, _| {
                calls += 1;
                Err(MediaError::Overloaded)
            }
        ),
        Err(MediaError::Overloaded)
    );
    assert_eq!(calls, 1);
    assert_eq!(processor.output.blocks, 0);
    assert!(processor.terminal && processor.run.is_none());
    assert_eq!(
        processor.finish_owned(&cancel, &mut discard_owned).err(),
        Some(MediaError::InvalidAudio)
    );
}

#[test]
fn owned_eof_error_has_no_successful_partial_summary() {
    let cancel = ResolverCancellation::default();
    let native = format(48_000, 1);
    let mut processor = processor(native, PreparedSampleRate::Hz16000, &cancel);
    processor
        .push_owned(
            &input(1, 0, native, vec![87; 100]),
            &cancel,
            &mut discard_owned,
        )
        .unwrap();
    let mut calls = 0;
    assert_eq!(
        processor
            .finish_owned(&cancel, &mut |_, _, _| {
                calls += 1;
                Err(MediaError::Unavailable)
            })
            .err(),
        Some(MediaError::Unavailable)
    );
    assert_eq!(calls, 1);
    assert!(processor.terminal && processor.run.is_none());
}

#[test]
fn owned_cancel_after_emission_is_rechecked_before_committing_metadata() {
    let cancel = ResolverCancellation::default();
    let native = format(16_000, 1);
    let mut processor = processor(native, PreparedSampleRate::Hz16000, &cancel);
    let mut calls = 0;
    assert_eq!(
        processor.push_owned(
            &input(1, 0, native, vec![12; 1000]),
            &cancel,
            &mut |block, _, scope| {
                calls += 1;
                assert_eq!(block.sequence(), 1);
                scope.cancel();
                Ok(())
            }
        ),
        Err(MediaError::Cancelled)
    );
    assert_eq!(calls, 1);
    assert_eq!(processor.output.blocks, 0);
    assert!(processor.terminal && processor.run.is_none());
}

#[test]
fn owned_cancel_during_eof_tail_clears_filter_and_rejects_summary() {
    let cancel = ResolverCancellation::default();
    let native = format(48_000, 1);
    let mut processor = processor(native, PreparedSampleRate::Hz24000, &cancel);
    processor
        .push_owned(
            &input(1, 0, native, vec![12; 100]),
            &cancel,
            &mut discard_owned,
        )
        .unwrap();
    assert_eq!(
        processor
            .finish_owned(&cancel, &mut |_, _, scope| {
                scope.cancel();
                Ok(())
            })
            .err(),
        Some(MediaError::Cancelled)
    );
    assert!(processor.terminal && processor.run.is_none());
    assert_eq!(processor.output.blocks, 0);
}

#[test]
fn owned_pre_cancel_never_invokes_sink_and_drops_existing_pending() {
    let cancel = ResolverCancellation::default();
    let native = format(16_000, 1);
    let mut processor = processor(native, PreparedSampleRate::Hz16000, &cancel);
    processor
        .push_owned(
            &input(1, 0, native, vec![1; 10]),
            &cancel,
            &mut discard_owned,
        )
        .unwrap();
    cancel.cancel();
    let mut calls = 0;
    assert_eq!(
        processor.push_owned(
            &input(2, 625, native, vec![1; 10]),
            &ResolverCancellation::default(),
            &mut |_, _, _| {
                calls += 1;
                Ok(())
            }
        ),
        Err(MediaError::Cancelled)
    );
    assert_eq!(calls, 0);
    assert!(processor.terminal && processor.run.is_none());
}

#[test]
fn owned_bridge_does_not_bypass_gate_order_or_session_validation() {
    let native = format(24_000, 1);
    let cancel = ResolverCancellation::default();
    for stale in [false, true] {
        let mut processor = processor(native, PreparedSampleRate::Hz24000, &cancel);
        let block = if stale {
            DecodedAudioBlock::new(
                StreamIdentity {
                    session: SessionId(908),
                    generation: 8,
                },
                1,
                0,
                native,
                vec![1; 480],
            )
            .unwrap()
        } else {
            input(2, 0, native, vec![1; 480])
        };
        let mut calls = 0;
        assert_eq!(
            processor.push_owned(&block, &cancel, &mut |_, _, _| {
                calls += 1;
                Ok(())
            }),
            Err(if stale {
                MediaError::Stale
            } else {
                MediaError::InvalidAudio
            })
        );
        assert_eq!(calls, 0);
        assert!(processor.run.is_none());
    }
}

#[test]
fn public_borrowed_and_private_owned_calls_share_one_ordered_dsp() {
    let cancel = ResolverCancellation::default();
    let native = format(16_000, 1);
    let mut processor = processor(native, PreparedSampleRate::Hz16000, &cancel);
    let mut sequence = Vec::new();
    processor
        .push(
            &input(1, 0, native, vec![1; 320]),
            &cancel,
            &mut |block, _| {
                sequence.push(block.sequence());
                Ok(())
            },
        )
        .unwrap();
    processor
        .push_owned(
            &input(2, 20_000, native, vec![2; 320]),
            &cancel,
            &mut |block, _, _| {
                sequence.push(block.sequence());
                Ok(())
            },
        )
        .unwrap();
    processor
        .push(
            &input(3, 40_000, native, vec![3; 13]),
            &cancel,
            &mut |_, _| Ok(()),
        )
        .unwrap();
    let summary = processor
        .finish_owned(&cancel, &mut |block, _, _| {
            sequence.push(block.sequence());
            Ok(())
        })
        .unwrap();
    assert_eq!(sequence, [1, 2, 3]);
    assert_eq!(summary.frames(), 653);
    assert_eq!(summary.blocks(), 3);
}

#[test]
fn owned_pcm_feeds_private_bounded_segments_without_borrowed_callback_copy() {
    for rate in [PreparedSampleRate::Hz16000, PreparedSampleRate::Hz24000] {
        let cancel = ResolverCancellation::default();
        let native = format(rate.hz(), 1);
        let mut processor = processor(native, rate, &cancel);
        let mut segments = PreparedSegmentBuilder::new(identity(), rate).unwrap();
        let frames = rate.hz() as usize * 101 / 10;
        let mut offset = 0;
        let mut sequence = 1;
        let mut sink = |block, gap, scope: &ResolverCancellation| {
            segments
                .push(block, gap, scope)
                .map_err(|_| MediaError::InvalidAudio)
        };
        while offset < frames {
            let count = 32_000.min(frames - offset);
            let block = input(
                sequence,
                frame_time(177, offset as u64, rate.hz()).unwrap(),
                native,
                vec![913; count],
            );
            processor.push_owned(&block, &cancel, &mut sink).unwrap();
            offset += count;
            sequence += 1;
        }
        let summary = processor.finish_owned(&cancel, &mut sink).unwrap();
        assert_eq!(summary.frames(), frames as u64);
        segments.finish(&cancel).unwrap();
        let first = segments.take().unwrap();
        let tail = segments.take().unwrap();
        assert_eq!(first.stamp().body_start_us, 177);
        assert_eq!(first.stamp().body_end_us, 10_000_177);
        assert_eq!(tail.stamp().body_start_us, 10_000_177);
        assert_eq!(tail.stamp().body_end_us, 10_100_177);
        assert_eq!(tail.audio_end_us(), tail.stamp().body_end_us);
        assert!(
            first
                .samples()
                .iter()
                .chain(tail.samples())
                .all(|sample| *sample == 913)
        );
        assert!(segments.take().is_none());
    }
}
