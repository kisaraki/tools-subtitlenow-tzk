//! Synthetic PCM and official-shaped native timestamp fixtures only. No HTTP,
//! provider account, credentials, user media or persistent audio is involved.
use super::*;
use crate::{
    audio::capture::SessionId, domain::SourceLanguage, media::prepared_audio::PreparedSampleRate,
};
use serde_json::{Value, json};
use std::{
    sync::{
        Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::oneshot;

const TEST_TIMEOUT: Duration = Duration::from_secs(5);
const GATE_TIMEOUT: Duration = Duration::from_secs(10);
fn selected(language: SpeechLanguage) -> SourceLanguage {
    match language {
        SpeechLanguage::English => SourceLanguage::English,
        SpeechLanguage::Japanese => SourceLanguage::Japanese,
        SpeechLanguage::French => SourceLanguage::French,
        SpeechLanguage::German => SourceLanguage::German,
        SpeechLanguage::Korean => SourceLanguage::Korean,
    }
}
fn request(
    language: SourceLanguage,
    rate: PreparedSampleRate,
    left_ms: u64,
    body_ms: u64,
    right_ms: u64,
    context: Vec<Box<str>>,
) -> PreparedSpeechRequest {
    let frames = rate.hz() as usize * (left_ms + body_ms + right_ms) as usize / 1000;
    PreparedSpeechRequest::new(
        super::super::prepared_speech::PreparedSpeechChunk::new(
            PreparedChunkStamp {
                identity: super::super::speech::StreamIdentity {
                    session: SessionId(999),
                    generation: 4,
                },
                sequence: 3,
                run_id: 2,
                rate,
                audio_start_us: 1_000_000,
                body_start_us: 1_000_000 + left_ms * 1000,
                body_end_us: 1_000_000 + (left_ms + body_ms) * 1000,
                discontinuity: false,
            },
            vec![-901; frames],
        )
        .unwrap(),
        language,
        context,
    )
    .unwrap()
}
fn english() -> PreparedSpeechRequest {
    request(
        SourceLanguage::English,
        PreparedSampleRate::Hz16000,
        500,
        1000,
        500,
        Vec::new(),
    )
}
fn input(request: &PreparedSpeechRequest) -> EncodeInput {
    EncodeInput {
        metadata: Metadata::from_request(request).unwrap(),
        samples: request.chunk().samples().to_vec().into_boxed_slice(),
        context: request.previous_source().to_vec().into_boxed_slice(),
    }
}
fn parse_fixture(
    provider: SpeechProvider,
    value: Value,
    request: &PreparedSpeechRequest,
) -> Result<PreparedSpeechResponse, PreparedSpeechError> {
    let response = parse_inner(
        provider,
        &serde_json::to_vec(&value).unwrap(),
        Metadata::from_request(request)?,
        &ResolverCancellation::default(),
    )?;
    response.validate_for(request)?;
    Ok(response)
}
fn openai(segments: Vec<Value>, language: &str) -> Value {
    let text = segments
        .iter()
        .map(|segment| segment["text"].as_str().unwrap_or(""))
        .collect::<Vec<_>>()
        .join(" ");
    json!({"language":language,"duration":2.0,"text":text,"segments":segments})
}
fn segment(id: u64, start: f64, end: f64, text: &str) -> Value {
    json!({"id":id,"start":start,"end":end,"text":text})
}
fn word(text: &str, start: &str, end: &str) -> Value {
    json!({"type":"word_info","text":text,"start_offset":start,"end_offset":end})
}
fn gemini(text: &str, annotations: Vec<Value>) -> Value {
    json!({"status":"completed","steps":[{"type":"model_output","content":[{"type":"text","text":text,"annotations":annotations}]}]})
}

#[test]
fn two_rates_and_five_languages_encode_real_mono_wave_and_explicit_hints() {
    for rate in [PreparedSampleRate::Hz16000, PreparedSampleRate::Hz24000] {
        for language in SpeechLanguage::ALL {
            let request = request(selected(language), rate, 500, 1000, 500, Vec::new());
            let payload = encode_inner(
                SpeechProvider::Gemini,
                input(&request),
                &ResolverCancellation::default(),
            )
            .unwrap();
            assert_eq!(payload.content_type.as_ref(), "application/json");
            assert!(payload.body.len() <= MAX_WIRE_BODY_BYTES);
            let value: Value = serde_json::from_slice(&payload.body).unwrap();
            assert_eq!(value["model"], "gemini-3.5-transcribe");
            assert_eq!(value["store"], false);
            assert_eq!(value["background"], false);
            assert_eq!(value["stream"], false);
            assert!(value.get("previous_interaction_id").is_none());
            assert_eq!(
                value["generation_config"]["transcription_config"]["language_codes"],
                json!([language.locale()])
            );
            assert_eq!(
                value["generation_config"]["transcription_config"]["mode"],
                json!({"type":"verbatim","timestamp_granularities":["word"]})
            );
            assert_eq!(value["input"][0]["mime_type"], "audio/wav");
            let wav = STANDARD
                .decode(value["input"][0]["data"].as_str().unwrap())
                .unwrap();
            assert_eq!(&wav[..4], b"RIFF");
            assert_eq!(&wav[8..16], b"WAVEfmt ");
            assert_eq!(u16::from_le_bytes(wav[22..24].try_into().unwrap()), 1);
            assert_eq!(
                u32::from_le_bytes(wav[24..28].try_into().unwrap()),
                rate.hz()
            );
            assert_eq!(u16::from_le_bytes(wav[34..36].try_into().unwrap()), 16);
            assert_eq!(wav.len(), 44 + request.chunk().samples().len() * 2);
            assert!(
                wav[44..]
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .all(|pair| i16::from_le_bytes(*pair) == -901)
            );
        }
    }
}

#[test]
fn openai_multipart_has_known_native_time_fields_and_bounded_unicode_context_tail() {
    let context = "舊內容".repeat(500) + " 最新內容";
    let request = request(
        SourceLanguage::French,
        PreparedSampleRate::Hz24000,
        0,
        10_000,
        0,
        vec![context.into()],
    );
    let payload = encode_inner(
        SpeechProvider::OpenAi,
        input(&request),
        &ResolverCancellation::default(),
    )
    .unwrap();
    assert!(
        payload
            .content_type
            .starts_with("multipart/form-data; boundary=")
    );
    let boundary = payload.content_type.split_once("boundary=").unwrap().1;
    assert!(boundary.len() <= 70);
    let text = String::from_utf8_lossy(&payload.body);
    assert!(text.contains("name=\"model\"\r\n\r\nwhisper-1\r\n"));
    assert!(text.contains("name=\"language\"\r\n\r\nfr\r\n"));
    assert!(text.contains("name=\"response_format\"\r\n\r\nverbose_json\r\n"));
    assert!(text.contains("name=\"timestamp_granularities[]\"\r\n\r\nsegment\r\n"));
    assert!(text.contains("filename=\"chunk.wav\"\r\nContent-Type: audio/wav"));
    let prompt = text
        .split("name=\"prompt\"\r\n\r\n")
        .nth(1)
        .unwrap()
        .split("\r\n")
        .next()
        .unwrap();
    assert!(prompt.len() <= MAX_OPENAI_PROMPT_BYTES);
    assert!(prompt.ends_with("最新內容"));
    assert!(!prompt.contains('\u{fffd}'));
    assert!(
        payload
            .body
            .ends_with(format!("--{boundary}--\r\n").as_bytes())
    );
}

#[test]
fn multipart_boundary_collision_is_rejected_in_prompt_and_pcm() {
    let boundary = format!("subtitlenow-{:016x}-{:016x}", 999, 3);
    let request = request(
        SourceLanguage::English,
        PreparedSampleRate::Hz16000,
        0,
        1000,
        0,
        vec![boundary.clone().into()],
    );
    assert_eq!(
        encode_inner(
            SpeechProvider::OpenAi,
            input(&request),
            &ResolverCancellation::default()
        )
        .err(),
        Some(PreparedSpeechError::InvalidData)
    );
    let request = english();
    let mut source = input(&request);
    let mut bytes = boundary.into_bytes();
    if !bytes.len().is_multiple_of(2) {
        bytes.push(0);
    }
    for (sample, pair) in source
        .samples
        .iter_mut()
        .zip(bytes.as_chunks::<2>().0.iter())
    {
        *sample = i16::from_le_bytes(*pair);
    }
    assert_eq!(
        encode_inner(
            SpeechProvider::OpenAi,
            source,
            &ResolverCancellation::default()
        )
        .err(),
        Some(PreparedSpeechError::InvalidData)
    );
}

#[test]
fn maximum_eleven_second_chunks_keep_wave_and_transport_bounded() {
    for provider in [SpeechProvider::OpenAi, SpeechProvider::Gemini] {
        for rate in [PreparedSampleRate::Hz16000, PreparedSampleRate::Hz24000] {
            let request = request(SourceLanguage::Korean, rate, 500, 10_000, 500, Vec::new());
            let payload =
                encode_inner(provider, input(&request), &ResolverCancellation::default()).unwrap();
            assert!(payload.body.len() <= MAX_WIRE_BODY_BYTES);
            assert_eq!(payload.body.capacity(), payload.body.len());
        }
    }
}

#[test]
fn openai_preserves_five_language_native_timestamps_and_excludes_whole_context() {
    for (language, name) in [
        (SpeechLanguage::English, "english"),
        (SpeechLanguage::Japanese, "japanese"),
        (SpeechLanguage::French, "french"),
        (SpeechLanguage::German, "german"),
        (SpeechLanguage::Korean, "korean"),
    ] {
        let request = request(
            selected(language),
            PreparedSampleRate::Hz16000,
            500,
            1000,
            500,
            Vec::new(),
        );
        let response = parse_fixture(
            SpeechProvider::OpenAi,
            openai(
                vec![
                    segment(0, 0.1, 0.2, "left"),
                    segment(1, 0.75, 1.25, "owned"),
                    segment(2, 1.6, 1.7, "right"),
                ],
                name,
            ),
            &request,
        )
        .unwrap();
        assert_eq!(response.identity, request.chunk().stamp().identity);
        assert_eq!(response.sequence, 3);
        assert_eq!(response.run_id, 2);
        assert_eq!(response.language, Some(language));
        assert_eq!(response.cues.len(), 1);
        assert_eq!(response.cues[0].start_us, 750_000);
        assert_eq!(response.cues[0].duration_us, 500_000);
        assert_eq!(response.cues[0].text.as_ref(), "owned");
    }
}

#[test]
fn openai_crossing_segments_fail_without_clipping_and_context_must_still_be_valid() {
    for (start, end, error) in [
        (0.4, 0.6, PreparedSpeechError::BoundaryMismatch),
        (1.4, 1.6, PreparedSpeechError::BoundaryMismatch),
        (0.2, 0.1, PreparedSpeechError::InvalidData),
        (1.9, 2.1, PreparedSpeechError::InvalidData),
    ] {
        assert_eq!(
            parse_fixture(
                SpeechProvider::OpenAi,
                openai(vec![segment(0, start, end, "text")], "english"),
                &english()
            )
            .err(),
            Some(error)
        );
    }
}

#[test]
fn openai_rejects_wrong_language_bad_duration_ids_order_and_text_consistency() {
    let request = english();
    for language in ["french", "spanish"] {
        assert_eq!(
            parse_fixture(
                SpeechProvider::OpenAi,
                openai(vec![segment(0, 0.6, 0.7, "text")], language),
                &request
            )
            .err(),
            Some(PreparedSpeechError::UnsupportedLanguage)
        );
    }
    let mut malformed = Vec::new();
    let mut duration = openai(vec![segment(0, 0.6, 0.7, "text")], "english");
    duration["duration"] = json!(20.0);
    malformed.push(duration);
    malformed.push(openai(
        vec![segment(0, 0.6, 0.7, "a"), segment(0, 0.8, 0.9, "b")],
        "english",
    ));
    malformed.push(openai(
        vec![segment(0, 0.8, 0.9, "a"), segment(1, 0.6, 0.7, "b")],
        "english",
    ));
    malformed.push(openai(vec![segment(0, -0.1, 0.2, "a")], "english"));
    malformed.push(openai(vec![segment(0, 0.6, 1e200, "a")], "english"));
    let mut mismatch = openai(vec![segment(0, 0.6, 0.7, "a")], "english");
    mismatch["text"] = json!("different");
    malformed.push(mismatch);
    for value in malformed {
        assert_eq!(
            parse_fixture(SpeechProvider::OpenAi, value, &request).err(),
            Some(PreparedSpeechError::InvalidData)
        );
    }
}

#[test]
fn openai_silence_is_explicit_and_missing_or_duplicate_native_fields_are_not_success() {
    let empty = parse_fixture(
        SpeechProvider::OpenAi,
        openai(Vec::new(), "english"),
        &english(),
    )
    .unwrap();
    assert!(empty.cues.is_empty());
    for body in [br#"{"duration":2,"text":"text","segments":[]}"#.as_slice(), br#"{"language":"english","duration":2,"text":"text"}"#, br#"{"language":"english","duration":2,"text":""}"#, br#"{"language":"english","duration":2,"text":"a","segments":[{"id":0,"start":0.6,"start":0.7,"end":0.8,"text":"a"}]}"#, br#"{"language":"english","duration":NaN,"text":"","segments":[]}"#] {
        assert_eq!(parse_inner(SpeechProvider::OpenAi,body,Metadata::from_request(&english()).unwrap(),&ResolverCancellation::default()).err(),Some(PreparedSpeechError::InvalidData));
    }
}

#[test]
fn gemini_native_words_group_exact_text_and_real_first_last_timing_without_indices() {
    for language in SpeechLanguage::ALL {
        let request = request(
            selected(language),
            PreparedSampleRate::Hz24000,
            500,
            1000,
            500,
            Vec::new(),
        );
        let response = parse_fixture(
            SpeechProvider::Gemini,
            gemini(
                "Left Alpha Beta Right",
                vec![
                    word("Left", "0.100s", "0.200s"),
                    word("Alpha", "0.600s", "0.700s"),
                    word("Beta", "0.800s", "1.100s"),
                    word("Right", "1.600s", "1.700s"),
                ],
            ),
            &request,
        )
        .unwrap();
        assert_eq!(response.cues.len(), 1);
        assert_eq!(response.cues[0].text.as_ref(), "Alpha Beta");
        assert_eq!(response.cues[0].start_us, 600_000);
        assert_eq!(response.cues[0].duration_us, 500_000);
        // Gemini exposes native word timing, but no authoritative detected-language
        // result here: this is the exact explicit caller hint, never Auto detection.
        assert_eq!(response.language, Some(language));
    }
}

#[test]
fn gemini_indices_are_utf8_byte_positions_and_must_correlate_exactly() {
    let first = json!({"type":"word_info","text":"語","start_index":0,"end_index":3,"start_offset":"0.600s","end_offset":"0.700s"});
    let second = json!({"type":"word_info","text":"詞","start_index":4,"end_index":7,"start_offset":"0.800s","end_offset":"1.000s"});
    let response = parse_fixture(
        SpeechProvider::Gemini,
        gemini("語 詞", vec![first.clone(), second.clone()]),
        &english(),
    )
    .unwrap();
    assert_eq!(response.cues[0].text.as_ref(), "語 詞");
    for end in [1, 2, 4] {
        let mut bad = first.clone();
        bad["end_index"] = json!(end);
        assert_eq!(
            parse_fixture(
                SpeechProvider::Gemini,
                gemini("語 詞", vec![bad, second.clone()]),
                &english()
            )
            .err(),
            Some(PreparedSpeechError::InvalidData)
        );
    }
    let mut missing = first;
    missing.as_object_mut().unwrap().remove("end_index");
    assert_eq!(
        parse_fixture(
            SpeechProvider::Gemini,
            gemini("語 詞", vec![missing, second]),
            &english()
        )
        .err(),
        Some(PreparedSpeechError::InvalidData)
    );
}

#[test]
fn gemini_unannotated_text_wrong_words_or_duplicate_words_are_rejected() {
    for value in [
        gemini("Alpha Beta", vec![word("Alpha", "0.6s", "0.7s")]),
        gemini("Alpha", vec![word("Other", "0.6s", "0.7s")]),
        gemini(
            "Alpha",
            vec![word("Alpha", "0.6s", "0.7s"), word("Alpha", "0.8s", "0.9s")],
        ),
        gemini(
            "Alpha, Beta",
            vec![word("Alpha", "0.6s", "0.7s"), word("Beta", "0.8s", "0.9s")],
        ),
    ] {
        assert_eq!(
            parse_fixture(SpeechProvider::Gemini, value, &english()).err(),
            Some(PreparedSpeechError::InvalidData)
        );
    }
}

#[test]
fn gemini_crossing_words_fail_instead_of_fabricating_trimmed_timestamps() {
    for (start, end, error) in [
        ("0.4s", "0.6s", PreparedSpeechError::BoundaryMismatch),
        ("1.4s", "1.6s", PreparedSpeechError::BoundaryMismatch),
        ("0.2s", "0.1s", PreparedSpeechError::InvalidData),
        ("1.9s", "2.1s", PreparedSpeechError::InvalidData),
    ] {
        assert_eq!(
            parse_fixture(
                SpeechProvider::Gemini,
                gemini("Alpha", vec![word("Alpha", start, end)]),
                &english()
            )
            .err(),
            Some(error)
        );
    }
}

#[test]
fn gemini_missing_output_or_native_annotations_never_becomes_silence() {
    let empty = parse_fixture(
        SpeechProvider::Gemini,
        gemini(" \n", Vec::new()),
        &english(),
    )
    .unwrap();
    assert!(empty.cues.is_empty());
    for value in [
        json!({"status":"completed","steps":[]}),
        json!({"status":"completed","steps":[{"type":"model_output","content":[]}]}),
        json!({"status":"completed","steps":[{"type":"model_output","content":[{"type":"text","text":""}]}]}),
        json!({"status":"in_progress","steps":[]}),
        json!({"status":"completed","output_text":"Alpha","steps":[]}),
    ] {
        assert_eq!(
            parse_fixture(SpeechProvider::Gemini, value, &english()).err(),
            Some(PreparedSpeechError::InvalidData)
        );
    }
}

#[test]
fn native_google_duration_grammar_and_rounding_are_checked() {
    for (value, expected) in [
        ("0s", 0),
        ("0.125s", 125_000),
        ("1.234567499s", 1_234_567),
        ("1.234567500s", 1_234_568),
        ("0.999999999s", 1_000_000),
    ] {
        assert_eq!(offset_us(value), Ok(expected));
    }
    for value in [
        "NaNs",
        "-0.1s",
        "+0.1s",
        "1e-3s",
        ".1s",
        "1.s",
        "0.1234567890s",
        "18446744073709551615s",
        "12s",
        "0.1",
    ] {
        assert_eq!(offset_us(value), Err(PreparedSpeechError::InvalidData));
    }
}

#[test]
fn response_cue_and_text_capacity_checks_apply_before_context_discard() {
    let segments = (0..=MAX_ASR_CUES_PER_CHUNK)
        .map(|id| segment(id as u64, 0.1, 0.2, "a"))
        .collect();
    assert_eq!(
        parse_fixture(
            SpeechProvider::OpenAi,
            openai(segments, "english"),
            &english()
        )
        .err(),
        Some(PreparedSpeechError::InvalidData)
    );
    let too_long = "a".repeat(MAX_CUE_BYTES + 1);
    assert_eq!(
        parse_fixture(
            SpeechProvider::Gemini,
            gemini(&too_long, vec![word(&too_long, "0.1s", "0.2s")]),
            &english()
        )
        .err(),
        Some(PreparedSpeechError::InvalidData)
    );
    let many_segments = (0..20)
        .map(|id| {
            segment(
                id,
                0.01 + id as f64 * 0.02,
                0.02 + id as f64 * 0.02,
                &"a".repeat(7000),
            )
        })
        .collect();
    assert_eq!(
        parse_fixture(
            SpeechProvider::OpenAi,
            openai(many_segments, "english"),
            &english()
        )
        .err(),
        Some(PreparedSpeechError::InvalidData)
    );
    let contents=(0..20).map(|index|{
        let text="a".repeat(7000);
        json!({"type":"text","text":text,"annotations":[word(&text,&format!("0.{:06}s",10_000+index*20_000),&format!("0.{:06}s",20_000+index*20_000))]})
    }).collect::<Vec<_>>();
    let aggregate =
        json!({"status":"completed","steps":[{"type":"model_output","content":contents}]});
    assert!(serde_json::to_vec(&aggregate).unwrap().len() < MAX_ASR_HTTP_RESPONSE_BYTES);
    assert_eq!(
        parse_fixture(SpeechProvider::Gemini, aggregate, &english()).err(),
        Some(PreparedSpeechError::InvalidData)
    );
    let annotations = (0..=MAX_NATIVE_WORDS)
        .map(|index| {
            word(
                "a",
                &format!("0.{:06}s", 10 + index * 200),
                &format!("0.{:06}s", 20 + index * 200),
            )
        })
        .collect();
    assert_eq!(
        parse_fixture(
            SpeechProvider::Gemini,
            gemini(&vec!["a"; MAX_NATIVE_WORDS + 1].join(" "), annotations),
            &english()
        )
        .err(),
        Some(PreparedSpeechError::InvalidData)
    );
    for provider in [SpeechProvider::OpenAi, SpeechProvider::Gemini] {
        let context = match provider {
            SpeechProvider::OpenAi => openai(vec![segment(0, 0.1, 0.2, "Only context")], "english"),
            SpeechProvider::Gemini => gemini(
                "Only context",
                vec![
                    word("Only", "0.1s", "0.15s"),
                    word("context", "0.16s", "0.2s"),
                ],
            ),
        };
        assert!(
            parse_fixture(provider, context, &english())
                .unwrap()
                .cues
                .is_empty()
        );
    }
}

#[test]
fn async_encode_parse_reject_auto_precancel_and_raw_size_without_work() {
    let _serial = work::TEST_OWNERS.lock().unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let auto = request(
                SourceLanguage::Auto,
                PreparedSampleRate::Hz16000,
                0,
                1000,
                0,
                Vec::new(),
            );
            for provider in [SpeechProvider::OpenAi, SpeechProvider::Gemini] {
                assert_eq!(
                    encode(provider, &auto, &ResolverCancellation::default())
                        .await
                        .err(),
                    Some(PreparedSpeechError::UnsupportedLanguage)
                );
                assert_eq!(
                    parse(
                        provider,
                        vec![b'{'],
                        &auto,
                        &ResolverCancellation::default()
                    )
                    .await
                    .err(),
                    Some(PreparedSpeechError::UnsupportedLanguage)
                );
            }
            let cancel = ResolverCancellation::default();
            cancel.cancel();
            assert_eq!(
                encode(SpeechProvider::OpenAi, &english(), &cancel)
                    .await
                    .err(),
                Some(PreparedSpeechError::Cancelled)
            );
            assert_eq!(
                parse(
                    SpeechProvider::OpenAi,
                    vec![b' '; MAX_ASR_HTTP_RESPONSE_BYTES + 1],
                    &english(),
                    &ResolverCancellation::default()
                )
                .await
                .err(),
                Some(PreparedSpeechError::InvalidData)
            );
            assert_eq!(
                parse(
                    SpeechProvider::Gemini,
                    Vec::new(),
                    &english(),
                    &ResolverCancellation::default()
                )
                .await
                .err(),
                Some(PreparedSpeechError::InvalidData)
            );
        });
}

#[test]
fn actual_shared_worker_encode_and_parse_return_only_validated_wire_owners() {
    let _serial = work::TEST_OWNERS.lock().unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let request = english();
            for provider in [SpeechProvider::OpenAi, SpeechProvider::Gemini] {
                let cancel = ResolverCancellation::default();
                let payload = encode(provider, &request, &cancel).await.unwrap();
                assert!(!payload.body.is_empty());
                let value = match provider {
                    SpeechProvider::OpenAi => {
                        openai(vec![segment(0, 0.6, 0.7, "Alpha")], "english")
                    }
                    SpeechProvider::Gemini => gemini("Alpha", vec![word("Alpha", "0.6s", "0.7s")]),
                };
                let response = parse(
                    provider,
                    serde_json::to_vec(&value).unwrap(),
                    &request,
                    &cancel,
                )
                .await
                .unwrap();
                response.validate_for(&request).unwrap();
                assert_eq!(response.cues.len(), 1);
                assert!(!cancel.is_cancelled());
            }
        });
}

#[derive(Default)]
struct Gate {
    released: Mutex<bool>,
    wake: Condvar,
}
impl Gate {
    fn wait(&self) -> Result<(), PreparedSpeechError> {
        let released = self.released.lock().unwrap();
        let (released, _) = self
            .wake
            .wait_timeout_while(released, GATE_TIMEOUT, |value| !*value)
            .unwrap();
        if *released {
            Ok(())
        } else {
            Err(PreparedSpeechError::Timeout)
        }
    }
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.wake.notify_all();
    }
}
struct ReleaseGate(Arc<Gate>);
impl Drop for ReleaseGate {
    fn drop(&mut self) {
        self.0.release();
    }
}

#[tokio::test]
async fn started_wire_worker_keeps_cpu_and_http_lease_until_real_exit_after_future_drop() {
    let cpu = Arc::new(Semaphore::new(1));
    let requests = Arc::new(Semaphore::new(1));
    let lease = Arc::new(requests.clone().try_acquire_owned().unwrap());
    let cancel = ResolverCancellation::default();
    let gate = Arc::new(Gate::default());
    let _release = ReleaseGate(gate.clone());
    let worker_gate = gate.clone();
    let done = Arc::new(AtomicBool::new(false));
    let worker_done = done.clone();
    let (entered, ready) = oneshot::channel();
    let mut work = Box::pin(run_worker(
        &cancel,
        cpu.clone(),
        Some(lease.clone()),
        move |_| {
            let _ = entered.send(());
            worker_gate.wait()?;
            worker_done.store(true, Ordering::SeqCst);
            Ok(7u8)
        },
    ));
    tokio::select! {result=&mut work=>panic!("gated wire worker completed early: {result:?}"),ready=tokio::time::timeout(TEST_TIMEOUT,ready)=>{ready.unwrap().unwrap();}}
    drop(work);
    drop(lease);
    assert!(cancel.is_cancelled());
    assert_eq!(cpu.available_permits(), 0);
    assert_eq!(requests.available_permits(), 0);
    assert!(!done.load(Ordering::SeqCst));
    gate.release();
    let returned = tokio::time::timeout(TEST_TIMEOUT, cpu.clone().acquire_owned())
        .await
        .unwrap()
        .unwrap();
    let network = tokio::time::timeout(TEST_TIMEOUT, requests.clone().acquire_owned())
        .await
        .unwrap()
        .unwrap();
    assert!(done.load(Ordering::SeqCst));
    drop(returned);
    drop(network);
    assert_eq!(cpu.available_permits(), 1);
    assert_eq!(requests.available_permits(), 1);
}

#[tokio::test]
async fn worker_overload_and_unpolled_drop_do_not_run_operation() {
    let cpu = Arc::new(Semaphore::new(0));
    let cancel = ResolverCancellation::default();
    let result = run_worker(&cancel, cpu, None, |_| -> Result<u8, PreparedSpeechError> {
        panic!("overloaded worker must not run")
    })
    .await;
    assert_eq!(result, Err(PreparedSpeechError::Overloaded));
    assert!(cancel.is_cancelled());
    let cpu = Arc::new(Semaphore::new(1));
    let cancel = ResolverCancellation::default();
    drop(run_worker(
        &cancel,
        cpu.clone(),
        None,
        |_| -> Result<u8, PreparedSpeechError> { panic!("unpolled worker must not run") },
    ));
    assert!(cancel.is_cancelled());
    assert_eq!(cpu.available_permits(), 1);
}

#[tokio::test]
async fn worker_cancelled_success_and_panic_release_capacity_without_success() {
    for panic in [false, true] {
        let cpu = Arc::new(Semaphore::new(1));
        let cancel = ResolverCancellation::default();
        let result = run_worker(&cancel, cpu.clone(), None, move |scope| {
            assert!(!panic, "explicit synthetic wire panic");
            scope.cancel();
            Ok(1u8)
        })
        .await;
        assert_eq!(
            result,
            Err(if panic {
                PreparedSpeechError::InvalidData
            } else {
                PreparedSpeechError::Cancelled
            })
        );
        let returned = tokio::time::timeout(TEST_TIMEOUT, cpu.clone().acquire_owned())
            .await
            .unwrap()
            .unwrap();
        drop(returned);
        assert_eq!(cpu.available_permits(), 1);
        assert!(cancel.is_cancelled());
    }
}
