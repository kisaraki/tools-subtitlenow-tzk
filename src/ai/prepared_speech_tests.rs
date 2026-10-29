//! Synthetic offline requests and responses stay entirely in test RAM.
use super::*;
use crate::audio::capture::SessionId;

fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(201),
        generation: 2,
    }
}
fn stamp() -> PreparedChunkStamp {
    PreparedChunkStamp {
        identity: identity(),
        sequence: 1,
        run_id: 1,
        rate: PreparedSampleRate::Hz16000,
        audio_start_us: 1_000_000,
        body_start_us: 1_500_000,
        body_end_us: 11_500_000,
        discontinuity: false,
    }
}
fn request(language: SourceLanguage) -> PreparedSpeechRequest {
    PreparedSpeechRequest::new(
        PreparedSpeechChunk::new(stamp(), vec![0; 176_000]).unwrap(),
        language,
        Vec::new(),
    )
    .unwrap()
}
fn response(cues: Vec<PreparedSpeechCue>) -> PreparedSpeechResponse {
    PreparedSpeechResponse {
        identity: identity(),
        sequence: 1,
        run_id: 1,
        language: Some(SpeechLanguage::English),
        cues,
    }
}
fn cue(start_us: u64, duration_us: u64, text: &str) -> PreparedSpeechCue {
    PreparedSpeechCue {
        start_us,
        duration_us,
        text: text.into(),
    }
}
fn capabilities() -> PreparedSpeechCapabilities {
    PreparedSpeechCapabilities {
        cancellation: true,
        timestamped_finals: true,
        supported_languages: [true; 5],
        constrained_auto_detection: true,
        sample_rates: [true; 2],
        max_concurrent: 4,
    }
}

#[test]
fn maximum_chunk_preserves_borrowed_pcm_and_exact_media_end_at_both_rates() {
    for rate in [PreparedSampleRate::Hz16000, PreparedSampleRate::Hz24000] {
        let mut stamp = stamp();
        stamp.rate = rate;
        let frames = rate.hz() as usize * 11;
        let chunk = PreparedSpeechChunk::new(stamp, vec![17; frames]).unwrap();
        assert_eq!(chunk.stamp(), stamp);
        assert_eq!(chunk.samples().len(), frames);
        assert_eq!(chunk.samples()[frames - 1], 17);
        assert_eq!(chunk.audio_end_us(), 12_000_000);
        assert!(matches!(
            PreparedSpeechChunk::new(stamp, vec![0; frames + 1]),
            Err(PreparedSpeechError::InvalidData)
        ));
    }
}

#[test]
fn chunk_rejects_zero_identity_sequence_run_and_empty_audio() {
    let original = stamp();
    let mut invalid = Vec::new();
    let mut value = original;
    value.identity.session.0 = 0;
    invalid.push(value);
    value = original;
    value.identity.generation = 0;
    invalid.push(value);
    value = original;
    value.sequence = 0;
    invalid.push(value);
    value = original;
    value.run_id = 0;
    invalid.push(value);
    for stamp in invalid {
        assert!(matches!(
            PreparedSpeechChunk::new(stamp, vec![0; 176_000]),
            Err(PreparedSpeechError::InvalidData)
        ));
    }
    assert!(matches!(
        PreparedSpeechChunk::new(original, Vec::new()),
        Err(PreparedSpeechError::InvalidData)
    ));
}

#[test]
fn body_must_be_nonempty_within_audio_and_no_longer_than_ten_seconds() {
    let original = stamp();
    for (start, end) in [
        (1_500_000, 1_500_000),
        (1_500_000, 1_499_999),
        (999_999, 11_000_000),
        (1_500_000, 12_000_001),
        (1_000_000, 11_000_001),
    ] {
        let mut stamp = original;
        stamp.body_start_us = start;
        stamp.body_end_us = end;
        assert!(matches!(
            PreparedSpeechChunk::new(stamp, vec![0; 176_000]),
            Err(PreparedSpeechError::InvalidData)
        ));
    }
}

#[test]
fn overlap_rounding_is_limited_to_one_microsecond_per_edge() {
    let mut value = stamp();
    value.body_start_us += 1;
    value.body_end_us += 1;
    assert!(PreparedSpeechChunk::new(value, vec![0; 176_000]).is_ok());
    value.body_start_us += 1;
    value.body_end_us += 1;
    assert!(matches!(
        PreparedSpeechChunk::new(value, vec![0; 176_000]),
        Err(PreparedSpeechError::InvalidData)
    ));
    value = stamp();
    value.body_start_us -= 1;
    value.body_end_us -= 1;
    assert!(PreparedSpeechChunk::new(value, vec![0; 176_000]).is_ok());
    value.body_start_us -= 1;
    value.body_end_us -= 1;
    assert!(matches!(
        PreparedSpeechChunk::new(value, vec![0; 176_000]),
        Err(PreparedSpeechError::InvalidData)
    ));
}

#[test]
fn partial_frame_duration_and_twenty_four_hour_boundary_are_checked() {
    let mut value = stamp();
    value.audio_start_us = MAX_DECODED_MEDIA_DURATION_US - 62;
    value.body_start_us = value.audio_start_us;
    value.body_end_us = MAX_DECODED_MEDIA_DURATION_US;
    let chunk = PreparedSpeechChunk::new(value, vec![3]).unwrap();
    assert_eq!(chunk.audio_end_us(), MAX_DECODED_MEDIA_DURATION_US);
    value.audio_start_us += 1;
    value.body_start_us += 1;
    value.body_end_us += 1;
    assert!(matches!(
        PreparedSpeechChunk::new(value, vec![3]),
        Err(PreparedSpeechError::InvalidData)
    ));
    value.audio_start_us = u64::MAX;
    value.body_start_us = u64::MAX;
    value.body_end_us = u64::MAX;
    assert!(matches!(
        PreparedSpeechChunk::new(value, vec![3]),
        Err(PreparedSpeechError::InvalidData)
    ));
}

#[test]
fn request_context_accepts_exact_bound_and_rejects_count_text_and_control_errors() {
    let context = (0..8).map(|_| "x".repeat(8192).into_boxed_str()).collect();
    let request = PreparedSpeechRequest::new(
        PreparedSpeechChunk::new(stamp(), vec![0; 176_000]).unwrap(),
        SourceLanguage::Japanese,
        context,
    )
    .unwrap();
    assert_eq!(request.language(), SourceLanguage::Japanese);
    assert_eq!(request.previous_source().len(), 8);
    assert_eq!(
        request
            .previous_source()
            .iter()
            .map(|text| text.len())
            .sum::<usize>(),
        MAX_ASR_CONTEXT_BYTES
    );
    for context in [
        vec!["x".into(); 9],
        vec![" ".into()],
        vec!["x\r".into()],
        vec!["x\0".into()],
        vec!["x".repeat(8193).into_boxed_str()],
    ] {
        assert!(matches!(
            PreparedSpeechRequest::new(
                PreparedSpeechChunk::new(stamp(), vec![0; 176_000]).unwrap(),
                SourceLanguage::Auto,
                context
            ),
            Err(PreparedSpeechError::InvalidData)
        ));
    }
}

#[test]
fn required_capabilities_and_concurrency_are_real_requirements() {
    for index in 0..5 {
        let mut caps = capabilities();
        match index {
            0 => caps.cancellation = false,
            1 => caps.timestamped_finals = false,
            2 => caps.sample_rates[0] = false,
            3 => caps.max_concurrent = 0,
            _ => caps.max_concurrent = MAX_ASR_BATCH_CHUNKS + 1,
        }
        assert_eq!(
            caps.validate(SourceLanguage::English, PreparedSampleRate::Hz16000),
            Err(PreparedSpeechError::UnsupportedCapability)
        );
    }
    for concurrent in 1..=4 {
        let mut caps = capabilities();
        caps.max_concurrent = concurrent;
        assert_eq!(
            caps.validate(SourceLanguage::Auto, PreparedSampleRate::Hz24000),
            Ok(())
        );
    }
}

#[test]
fn five_language_support_and_auto_detection_are_constrained() {
    for source in [
        SourceLanguage::English,
        SourceLanguage::Japanese,
        SourceLanguage::French,
        SourceLanguage::German,
        SourceLanguage::Korean,
    ] {
        let mut caps = capabilities();
        let language = SpeechLanguage::selected(source).unwrap();
        caps.supported_languages[language as usize] = false;
        assert_eq!(
            caps.validate(source, PreparedSampleRate::Hz16000),
            Err(PreparedSpeechError::UnsupportedLanguage)
        );
        assert_eq!(
            caps.validate(SourceLanguage::Auto, PreparedSampleRate::Hz24000),
            Err(PreparedSpeechError::UnsupportedLanguage)
        );
    }
    let mut caps = capabilities();
    caps.constrained_auto_detection = false;
    assert_eq!(
        caps.validate(SourceLanguage::Auto, PreparedSampleRate::Hz16000),
        Err(PreparedSpeechError::UnsupportedLanguage)
    );
    assert_eq!(
        caps.validate(SourceLanguage::English, PreparedSampleRate::Hz16000),
        Ok(())
    );
}

#[test]
fn response_accepts_body_edges_and_native_relative_timing() {
    let request = request(SourceLanguage::English);
    let response = response(vec![
        cue(500_000, 5_000_000, "one\n\ttwo"),
        cue(5_500_000, 5_000_000, "three"),
    ]);
    assert_eq!(response.validate_for(&request), Ok(()));
}

#[test]
fn response_rejects_old_session_generation_sequence_and_run() {
    let request = request(SourceLanguage::English);
    for index in 0..4 {
        let mut response = response(vec![cue(500_000, 1, "one")]);
        match index {
            0 => response.identity.session.0 += 1,
            1 => response.identity.generation += 1,
            2 => response.sequence += 1,
            _ => response.run_id += 1,
        }
        assert_eq!(
            response.validate_for(&request),
            Err(PreparedSpeechError::Stale)
        );
    }
}

#[test]
fn nonempty_response_requires_selected_supported_language_but_silence_does_not() {
    let selected = request(SourceLanguage::English);
    let mut reply = response(vec![cue(500_000, 1, "one")]);
    reply.language = None;
    assert_eq!(
        reply.validate_for(&selected),
        Err(PreparedSpeechError::InvalidData)
    );
    reply.language = Some(SpeechLanguage::Japanese);
    assert_eq!(
        reply.validate_for(&selected),
        Err(PreparedSpeechError::UnsupportedLanguage)
    );
    assert_eq!(reply.validate_for(&request(SourceLanguage::Auto)), Ok(()));
    reply.cues.clear();
    reply.language = None;
    assert_eq!(reply.validate_for(&selected), Ok(()));
}

#[test]
fn crossing_body_edges_returns_boundary_error_without_cropping_or_clipping_text() {
    let request = request(SourceLanguage::English);
    for cue in [
        cue(499_999, 2, "cross prefix"),
        cue(10_499_999, 2, "cross suffix"),
        cue(0, 1, "context only"),
    ] {
        assert_eq!(
            response(vec![cue]).validate_for(&request),
            Err(PreparedSpeechError::BoundaryMismatch)
        );
    }
}

#[test]
fn response_rejects_audio_overflow_zero_duration_and_unordered_or_overlapping_cues() {
    let request = request(SourceLanguage::English);
    for cues in [
        vec![cue(500_000, 0, "zero")],
        vec![cue(500_000, u64::MAX, "overflow")],
        vec![cue(u64::MAX, 1, "start overflow")],
        vec![cue(11_000_000, 1, "outside")],
        vec![cue(500_001, 2, "one"), cue(500_002, 2, "overlap")],
        vec![cue(500_002, 1, "one"), cue(500_000, 1, "backwards")],
    ] {
        assert_eq!(
            response(cues).validate_for(&request),
            Err(PreparedSpeechError::InvalidData)
        );
    }
}

#[test]
fn response_text_is_bounded_and_only_newline_and_tab_controls_are_accepted() {
    let request = request(SourceLanguage::English);
    for text in [
        " ".to_owned(),
        "x\r".to_owned(),
        "x\0".to_owned(),
        "x\u{7f}".to_owned(),
        "x".repeat(8193),
    ] {
        assert_eq!(
            response(vec![cue(500_000, 1, &text)]).validate_for(&request),
            Err(PreparedSpeechError::InvalidData)
        );
    }
    assert_eq!(
        response(vec![cue(500_000, 1, &"x".repeat(8192))]).validate_for(&request),
        Ok(())
    );
}

#[test]
fn response_count_and_total_bytes_accept_exact_limits_then_reject_overflow() {
    let request = request(SourceLanguage::English);
    let make = |count: usize, length: usize| {
        (0..count)
            .map(|index| cue(500_000 + index as u64, 1, &"x".repeat(length)))
            .collect()
    };
    assert_eq!(response(make(512, 256)).validate_for(&request), Ok(()));
    assert_eq!(
        response(make(513, 1)).validate_for(&request),
        Err(PreparedSpeechError::InvalidData)
    );
    assert_eq!(
        response(make(512, 257)).validate_for(&request),
        Err(PreparedSpeechError::InvalidData)
    );
}

#[test]
fn retry_classification_excludes_credentials_boundary_and_invalid_data() {
    assert!(PreparedSpeechError::Timeout.retryable());
    assert!(PreparedSpeechError::Network.retryable());
    assert!(PreparedSpeechError::RateLimited.retryable());
    for error in [
        PreparedSpeechError::Credential,
        PreparedSpeechError::InvalidData,
        PreparedSpeechError::BoundaryMismatch,
        PreparedSpeechError::Cancelled,
        PreparedSpeechError::Stale,
    ] {
        assert!(!error.retryable());
    }
}

#[test]
fn chunk_discards_large_spare_pcm_capacity_before_retaining_audio() {
    let mut samples = Vec::with_capacity(352_000);
    samples.resize(176_000, 17);
    assert!(samples.capacity() > samples.len());
    let chunk = PreparedSpeechChunk::new(stamp(), samples).unwrap();
    assert_eq!(chunk.samples().len(), 176_000);
    assert_eq!(chunk.samples.into_vec().capacity(), 176_000);

    let mut invalid_samples = Vec::with_capacity(352_000);
    invalid_samples.resize(176_000, 17);
    let mut invalid_stamp = stamp();
    invalid_stamp.identity.session.0 = 0;
    // The invalid constructor consumes its allocation and retains no PCM owner.
    assert!(matches!(
        PreparedSpeechChunk::new(invalid_stamp, invalid_samples),
        Err(PreparedSpeechError::InvalidData)
    ));
}

#[test]
fn request_discards_large_spare_context_capacity_after_validation() {
    let mut context = Vec::with_capacity(1024);
    context.push("previous source".into());
    assert!(context.capacity() > context.len());
    let request = PreparedSpeechRequest::new(
        PreparedSpeechChunk::new(stamp(), vec![0; 176_000]).unwrap(),
        SourceLanguage::English,
        context,
    )
    .unwrap();
    assert_eq!(request.previous_source()[0].as_ref(), "previous source");
    assert_eq!(request.previous_source.into_vec().capacity(), 1);

    let mut invalid_context = Vec::with_capacity(1024);
    invalid_context.push("\0".into());
    // Both the owned chunk and context are dropped on this error path.
    assert!(matches!(
        PreparedSpeechRequest::new(
            PreparedSpeechChunk::new(stamp(), vec![0; 176_000]).unwrap(),
            SourceLanguage::English,
            invalid_context,
        ),
        Err(PreparedSpeechError::InvalidData)
    ));
}
