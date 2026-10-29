//! Synthetic byte samples only. These are not MP4/muxer compatibility fixtures.
use super::*;
use crate::{ai::speech::SpeechLanguage, audio::capture::SessionId};

fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(813),
        generation: 4,
    }
}
fn track(scale: u32) -> Tx3gTrack {
    Tx3gTrack::new(identity(), 7, scale, TextLanguage::TaiwanTraditional).unwrap()
}
fn stamp(sequence: u64, start_ticks: u64, duration_ticks: u64) -> Tx3gSampleStamp {
    Tx3gSampleStamp {
        identity: identity(),
        sequence,
        start_ticks,
        duration_ticks,
    }
}
fn sample(bytes: &[u8]) -> Vec<u8> {
    let mut result = u16::try_from(bytes.len()).unwrap().to_be_bytes().to_vec();
    result.extend_from_slice(bytes);
    result
}
fn utf16(text: &str, big_endian: bool) -> Vec<u8> {
    let mut bytes = if big_endian {
        vec![0xfe, 0xff]
    } else {
        vec![0xff, 0xfe]
    };
    for word in text.encode_utf16() {
        bytes.extend_from_slice(&if big_endian {
            word.to_be_bytes()
        } else {
            word.to_le_bytes()
        });
    }
    sample(&bytes)
}
fn rejects(bytes: &[u8], expected: MediaError) {
    let cancel = ResolverCancellation::default();
    let mut track = track(1000);
    track
        .push(stamp(1, 0, 1), &sample(b"prior"), &cancel)
        .unwrap();
    assert_eq!(track.push(stamp(2, 1, 1), bytes, &cancel), Err(expected));
    assert!(track.cues.is_empty());
    assert_eq!(
        track.push(stamp(2, 1, 1), &sample(b"repair"), &cancel),
        Err(expected)
    );
    assert!(matches!(track.finish(&cancel), Err(error) if error == expected));
}

#[test]
fn tx3g_unicode_encodings_preserve_source_and_track_scope() {
    let cancel = ResolverCancellation::default();
    let source = "台灣 café 日本語 한국어 😀";
    for bytes in [
        sample(source.as_bytes()),
        utf16(source, true),
        utf16(source, false),
        sample(format!("\u{feff}{source}").as_bytes()),
    ] {
        let mut track = track(1000);
        track.push(stamp(1, 321, 2345), &bytes, &cancel).unwrap();
        let text = track.finish(&cancel).unwrap();
        assert_eq!(text.identity, identity());
        assert_eq!(&*text.content_id, "prepared-tx3g/813/4/7");
        assert!(text.language == TextLanguage::TaiwanTraditional);
        assert_eq!(text.source, SubtitleSource::ExistingSubtitle);
        assert_eq!(&*text.cues[0].text, source);
        assert_eq!(text.cues[0].start_us, Some(321_000));
        assert_eq!(text.cues[0].duration_us, Some(2_345_000));
    }
}
#[test]
fn tx3g_explicit_languages_are_not_inferred_or_localized_in_decoder() {
    let cancel = ResolverCancellation::default();
    for language in [
        TextLanguage::TaiwanTraditional,
        TextLanguage::OtherTraditional,
        TextLanguage::SimplifiedChinese,
        TextLanguage::Source(SpeechLanguage::English),
        TextLanguage::Source(SpeechLanguage::Japanese),
        TextLanguage::Source(SpeechLanguage::French),
        TextLanguage::Source(SpeechLanguage::German),
        TextLanguage::Source(SpeechLanguage::Korean),
    ] {
        let mut track = Tx3gTrack::new(identity(), 7, 1000, language).unwrap();
        track
            .push(stamp(1, 0, 1000), &sample("软件与内存".as_bytes()), &cancel)
            .unwrap();
        let text = track.finish(&cancel).unwrap();
        assert!(text.language == language);
        assert_eq!(&*text.cues[0].text, "软件与内存");
    }
}
#[test]
fn tx3g_empty_samples_clear_without_extending_previous_or_removing_gaps() {
    let cancel = ResolverCancellation::default();
    let mut track = track(1000);
    track.push(stamp(1, 0, 400), &[0, 0], &cancel).unwrap();
    track
        .push(stamp(2, 400, 600), &sample(b"first"), &cancel)
        .unwrap();
    track.push(stamp(3, 1000, 700), &[0, 0], &cancel).unwrap();
    track
        .push(stamp(4, 2000, 1000), &sample(b"next"), &cancel)
        .unwrap();
    let text = track.finish(&cancel).unwrap();
    assert_eq!(text.cues.len(), 2);
    assert_eq!(text.cues[0].start_us, Some(400_000));
    assert_eq!(text.cues[0].duration_us, Some(600_000));
    assert_eq!(text.cues[1].start_us, Some(2_000_000));
}
#[test]
fn tx3g_line_separators_have_newline_semantics_but_markup_is_plain_text() {
    let cancel = ResolverCancellation::default();
    let mut track = track(1000);
    track
        .push(
            stamp(1, 0, 1),
            &sample("<b>A</b>\u{2028}B\u{2029}C\r\n\tD".as_bytes()),
            &cancel,
        )
        .unwrap();
    assert_eq!(
        &*track.finish(&cancel).unwrap().cues[0].text,
        "<b>A</b>\nB\nC\r\n\tD"
    );
}
#[test]
fn tx3g_absolute_endpoint_conversion_does_not_accumulate_rounding() {
    let cancel = ResolverCancellation::default();
    let mut track = track(90_000);
    for index in 0..6000 {
        track
            .push(stamp(index + 1, index * 3001, 3001), &sample(b"x"), &cancel)
            .unwrap();
    }
    let text = track.finish(&cancel).unwrap();
    let last = text.cues.last().unwrap();
    assert_eq!(
        last.start_us.unwrap() + last.duration_us.unwrap(),
        200_066_666
    );
    assert!(text.cues.windows(2).all(|pair| pair[0].start_us.unwrap()
        + pair[0].duration_us.unwrap()
        == pair[1].start_us.unwrap()));
}
#[test]
fn tx3g_all_truncations_and_wrong_lengths_reject_without_partial_document() {
    let valid = utf16("A😀測試", true);
    for end in 0..valid.len() {
        rejects(&valid[..end], MediaError::InvalidSubtitle);
    }
    rejects(&[0, 1, b'a', b'b'], MediaError::UnsupportedSubtitle);
    rejects(&[0, 5, b'a'], MediaError::InvalidSubtitle);
}
#[test]
fn tx3g_invalid_unicode_never_uses_lossy_conversion() {
    for bytes in [
        &[0xff][..],
        &[0xc0, 0xaf],
        &[0xed, 0xa0, 0x80],
        &[0xfe, 0xff, 0x00],
        &[0xfe, 0xff, 0xd8, 0x00],
        &[0xfe, 0xff, 0xdc, 0x00],
        &[0xff, 0xfe, 0x00, 0xd8],
    ] {
        rejects(&sample(bytes), MediaError::InvalidSubtitle);
    }
}
#[test]
fn tx3g_blank_bom_only_and_control_text_are_not_clearing_samples() {
    for bytes in [
        sample(b" \n\t"),
        sample(b"a\0b"),
        sample(b"a\x1bb"),
        sample(b"a\x7fb"),
        sample(&[0xfe, 0xff]),
        utf16("\0", true),
        sample("\u{feff}".as_bytes()),
    ] {
        rejects(&bytes, MediaError::InvalidSubtitle);
    }
}
#[test]
fn tx3g_modifier_boxes_and_trailing_bytes_are_explicitly_unsupported() {
    for kind in [b"styl", b"krok", b"href", b"tbox", b"xxxx"] {
        let mut bytes = sample(b"text");
        bytes.extend_from_slice(&8u32.to_be_bytes());
        bytes.extend_from_slice(kind);
        rejects(&bytes, MediaError::UnsupportedSubtitle);
    }
    rejects(&[0, 0, 0], MediaError::UnsupportedSubtitle);
}
#[test]
fn tx3g_constructor_requires_nonzero_session_generation_track_and_scale() {
    for (identity, id, scale) in [
        (
            StreamIdentity {
                session: SessionId(0),
                generation: 1,
            },
            1,
            1,
        ),
        (
            StreamIdentity {
                session: SessionId(1),
                generation: 0,
            },
            1,
            1,
        ),
        (identity(), 0, 1),
        (identity(), 1, 0),
    ] {
        assert!(matches!(
            Tx3gTrack::new(identity, id, scale, TextLanguage::TaiwanTraditional),
            Err(MediaError::InvalidInput)
        ));
    }
}
#[test]
fn tx3g_empty_or_clearing_only_track_never_returns_a_document() {
    let cancel = ResolverCancellation::default();
    assert!(matches!(
        track(1).finish(&cancel),
        Err(MediaError::InvalidSubtitle)
    ));
    let mut track = track(1);
    track.push(stamp(1, 0, 1), &[0, 0], &cancel).unwrap();
    assert!(matches!(
        track.finish(&cancel),
        Err(MediaError::InvalidSubtitle)
    ));
}
#[test]
fn tx3g_old_session_or_generation_poison_the_builder() {
    let cancel = ResolverCancellation::default();
    for old in [
        StreamIdentity {
            session: SessionId(814),
            generation: 4,
        },
        StreamIdentity {
            session: SessionId(813),
            generation: 3,
        },
    ] {
        let mut track = track(1000);
        track
            .push(stamp(1, 0, 1), &sample(b"prior"), &cancel)
            .unwrap();
        let mut stale = stamp(2, 1, 1);
        stale.identity = old;
        assert_eq!(
            track.push(stale, &sample(b"old"), &cancel),
            Err(MediaError::Stale)
        );
        assert!(track.cues.is_empty());
        assert!(matches!(track.finish(&cancel), Err(MediaError::Stale)));
    }
}
#[test]
fn tx3g_sequence_and_overlap_include_empty_samples() {
    let cancel = ResolverCancellation::default();
    for bad in [
        stamp(0, 10, 1),
        stamp(1, 10, 1),
        stamp(3, 10, 1),
        stamp(u64::MAX, 10, 1),
        stamp(2, 9, 1),
        stamp(2, 10, 0),
        stamp(2, u64::MAX, 1),
    ] {
        let mut track = track(1000);
        track.push(stamp(1, 0, 10), &[0, 0], &cancel).unwrap();
        assert_eq!(
            track.push(bad, &sample(b"x"), &cancel),
            Err(MediaError::InvalidSubtitle)
        );
        assert!(matches!(
            track.finish(&cancel),
            Err(MediaError::InvalidSubtitle)
        ));
    }
}
#[test]
fn tx3g_rejects_sub_microsecond_collapse_and_overlong_media() {
    let cancel = ResolverCancellation::default();
    let mut track = track(u32::MAX);
    assert_eq!(
        track.push(stamp(1, 0, 1), &sample(b"x"), &cancel),
        Err(MediaError::InvalidSubtitle)
    );
    let mut track = super::tests::track(1);
    assert_eq!(
        track.push(stamp(1, 86400, 1), &sample(b"x"), &cancel),
        Err(MediaError::TooLarge)
    );
    assert_eq!(ticks_us(u64::MAX, 1), Err(MediaError::TooLarge));
}
#[test]
fn tx3g_exact_duration_limit_accepts_without_rewriting_time() {
    let cancel = ResolverCancellation::default();
    let mut track = track(1000);
    track
        .push(stamp(1, 86_399_999, 1), &sample(b"x"), &cancel)
        .unwrap();
    let text = track.finish(&cancel).unwrap();
    assert_eq!(
        text.cues[0].start_us.unwrap() + text.cues[0].duration_us.unwrap(),
        MAX_DECODED_MEDIA_DURATION_US
    );
}
#[test]
fn tx3g_cancellation_discards_retained_text_and_prevents_finish() {
    let cancel = ResolverCancellation::default();
    let mut track = track(1000);
    track.push(stamp(1, 0, 1), &sample(b"x"), &cancel).unwrap();
    cancel.cancel();
    assert_eq!(
        track.push(stamp(2, 1, 1), &sample(b"y"), &cancel),
        Err(MediaError::Cancelled)
    );
    assert!(track.cues.is_empty());
    assert!(matches!(track.finish(&cancel), Err(MediaError::Cancelled)));
    let cancelled = ResolverCancellation::default();
    cancelled.cancel();
    assert!(matches!(
        super::tests::track(1000).finish(&cancelled),
        Err(MediaError::Cancelled)
    ));
}
#[test]
fn tx3g_sample_and_decoded_cue_byte_limits_are_independent() {
    let cancel = ResolverCancellation::default();
    let mut track = track(1);
    track
        .push(stamp(1, 0, 1), &sample(&vec![b'x'; MAX_CUE_BYTES]), &cancel)
        .unwrap();
    assert_eq!(
        track.finish(&cancel).unwrap().cues[0].text.len(),
        MAX_CUE_BYTES
    );
    rejects(
        &sample(&vec![b'x'; MAX_CUE_BYTES + 1]),
        MediaError::TooLarge,
    );
    // Raw UTF-16 fits the sample budget; its UTF-8 representation does not.
    rejects(&utf16(&"測".repeat(3000), true), MediaError::TooLarge);
}
#[test]
fn tx3g_sample_count_bounds_clearing_work_and_preserves_one_real_cue() {
    let cancel = ResolverCancellation::default();
    let mut track = track(1);
    track.push(stamp(1, 0, 1), &sample(b"x"), &cancel).unwrap();
    for i in 1..MAX_TX3G_SAMPLES {
        track
            .push(stamp(i as u64 + 1, i as u64, 1), &[0, 0], &cancel)
            .unwrap();
    }
    assert_eq!(track.samples, MAX_TX3G_SAMPLES);
    assert_eq!(
        track.push(
            stamp(MAX_TX3G_SAMPLES as u64 + 1, MAX_TX3G_SAMPLES as u64, 1),
            &[0, 0],
            &cancel
        ),
        Err(MediaError::TooLarge)
    );
    assert!(track.cues.is_empty());
}
#[test]
fn tx3g_cue_count_and_total_text_budget_never_return_partial_documents() {
    let cancel = ResolverCancellation::default();
    for (bytes, count) in [
        (sample(b"x"), MAX_TEXT_CUES),
        (
            sample(&vec![b'x'; MAX_CUE_BYTES]),
            MAX_TEXT_BYTES / MAX_CUE_BYTES,
        ),
    ] {
        let mut track = track(1000);
        for i in 0..count {
            track
                .push(stamp(i as u64 + 1, i as u64, 1), &bytes, &cancel)
                .unwrap();
        }
        assert_eq!(track.cues.len(), count);
        assert_eq!(
            track.push(
                stamp(count as u64 + 1, count as u64, 1),
                &sample(b"x"),
                &cancel
            ),
            Err(MediaError::TooLarge)
        );
        assert!(track.cues.is_empty());
        assert!(matches!(track.finish(&cancel), Err(MediaError::TooLarge)));
    }
}
#[test]
fn tx3g_wire_budget_bounds_utf16_even_when_decoded_text_fits() {
    let cancel = ResolverCancellation::default();
    let bytes = utf16(&"x".repeat(4095), true);
    assert_eq!(bytes.len(), MAX_TX3G_SAMPLE_BYTES);
    let count = MAX_TX3G_TRACK_BYTES / bytes.len();
    let mut track = track(1000);
    for i in 0..count {
        track
            .push(stamp(i as u64 + 1, i as u64, 1), &bytes, &cancel)
            .unwrap();
    }
    assert!(track.text_bytes + 4095 <= MAX_TEXT_BYTES);
    assert_eq!(
        track.push(stamp(count as u64 + 1, count as u64, 1), &bytes, &cancel),
        Err(MediaError::TooLarge)
    );
    assert!(matches!(track.finish(&cancel), Err(MediaError::TooLarge)));
}

#[test]
fn tx3g_chinese_document_uses_existing_taiwan_localization_after_decode() {
    let cancel = ResolverCancellation::default();
    let mut track = Tx3gTrack::new(identity(), 7, 1000, TextLanguage::SimplifiedChinese).unwrap();
    track
        .push(
            stamp(1, 1234, 567),
            &utf16("软件、内存与鼠标", false),
            &cancel,
        )
        .unwrap();
    let original = track.finish(&cancel).unwrap();
    let localized =
        crate::subtitle::localization::taiwan_phrase_conversion(&original.cues[0].text).unwrap();
    assert_eq!(localized, "軟體、記憶體與滑鼠");
    assert_eq!(&*original.cues[0].text, "软件、内存与鼠标");
    assert_eq!(original.cues[0].start_us, Some(1_234_000));
    assert_eq!(original.cues[0].duration_us, Some(567_000));
}

struct TranslationProbe;
impl crate::ai::translation::Translator for TranslationProbe {
    fn capabilities(&self) -> crate::ai::translation::TranslationCapabilities {
        crate::ai::translation::TranslationCapabilities {
            streaming: true,
            cancellation: true,
            supported_languages: [true; 5],
            max_concurrent: 2,
        }
    }
    fn translate<'a>(
        &'a self,
        request: &'a crate::ai::translation::TranslationRequest,
        _output: &'a mut crate::ai::translation::TranslationEmitter,
        _cancel: crate::ai::translation::TranslationCancellation,
    ) -> crate::ai::translation::TranslationFuture<'a> {
        Box::pin(async move {
            use crate::ai::translation::TranslationPass;
            assert_eq!(request.source.identity, identity());
            assert_eq!(request.target_language(), "zh-TW");
            let context = request.prepared_context.as_ref().unwrap();
            match request.source.sequence {
                0 => {
                    assert_eq!(&*request.source.text, "first");
                    assert_eq!(request.source.start_us, Some(11_000));
                    assert_eq!(request.source.duration_us, Some(123_000));
                    assert!(context.previous_source.is_empty());
                    assert_eq!(context.following_source[0].as_ref(), "second");
                }
                1 => {
                    assert_eq!(&*request.source.text, "second");
                    assert_eq!(request.source.start_us, Some(789_000));
                    assert_eq!(request.source.duration_us, Some(234_000));
                    assert_eq!(context.previous_source[0].as_ref(), "first");
                    assert!(context.following_source.is_empty());
                }
                _ => panic!("unexpected synthetic cue"),
            }
            match request.pass {
                TranslationPass::Draft => {
                    assert!(request.draft.is_none());
                    Ok(format!("草稿 {}", request.source.sequence).into())
                }
                TranslationPass::Review => {
                    assert_eq!(
                        request.draft.as_deref(),
                        Some(format!("草稿 {}", request.source.sequence).as_str())
                    );
                    Ok(format!("完成 {}", request.source.sequence).into())
                }
            }
        })
    }
}

#[tokio::test]
async fn tx3g_source_document_shares_prepared_translation_context_and_timing() {
    use crate::{
        ai::{prepared_translation::translate_document, translation::SessionGlossary},
        domain::TranslationQuality,
    };
    let cancel = ResolverCancellation::default();
    for language in SpeechLanguage::ALL {
        let mut track =
            Tx3gTrack::new(identity(), 7, 1000, TextLanguage::Source(language)).unwrap();
        track
            .push(stamp(1, 11, 123), &sample(b"first"), &cancel)
            .unwrap();
        track.push(stamp(2, 134, 655), &[0, 0], &cancel).unwrap();
        track
            .push(stamp(3, 789, 234), &utf16("second", true), &cancel)
            .unwrap();
        let original = track.finish(&cancel).unwrap();
        let result = translate_document(
            &original,
            TranslationQuality::Standard,
            &SessionGlossary::new(identity()),
            &TranslationProbe,
            &cancel,
        )
        .await
        .unwrap();
        assert_eq!(
            result.iter().map(|text| text.as_ref()).collect::<Vec<_>>(),
            ["完成 0", "完成 1"]
        );
        assert_eq!(original.cues.len(), 2);
        assert_eq!(original.cues[1].start_us, Some(789_000));
    }
}
