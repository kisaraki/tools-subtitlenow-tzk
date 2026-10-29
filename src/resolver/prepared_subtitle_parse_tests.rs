//! Synthetic whole-media documents only; no media, files, network or credentials.
use super::*;
use crate::{
    ai::speech::SpeechLanguage,
    audio::capture::SessionId,
    resolver::contracts::{MAX_WINDOW_AGE, WindowStamp},
};

const CONTENT_ID: &str = "synthetic-prepared-edition";
fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(771),
        generation: 5,
    }
}
fn srt(text: &str) -> String {
    format!("1\n00:00:00,000 --> 00:00:01,000\n{text}\n\n")
}
fn vtt(text: &str) -> String {
    format!("WEBVTT\n\n00:00.000 --> 00:01.000\n{text}\n\n")
}
fn prepared(format: SubtitleFormat, body: &[u8]) -> Result<ExistingText, ResolverError> {
    parse_prepared(
        format,
        body,
        identity(),
        CONTENT_ID,
        TextLanguage::Source(SpeechLanguage::English),
        &ResolverCancellation::default(),
    )
}
fn rejects(format: SubtitleFormat, body: &str, expected: ResolverError) {
    assert!(matches!(prepared(format, body.as_bytes()), Err(error) if error == expected));
}

#[test]
fn prepared_document_needs_no_capture_clock_and_keeps_live_expiry_unchanged() {
    let body = srt("synthetic source text");
    let stale = TextQuery {
        stamp: WindowStamp {
            identity: identity(),
            sequence: 29,
            ended_at: Instant::now() - MAX_WINDOW_AGE - std::time::Duration::from_secs(60),
            start_us: Some(40_000_000),
            duration_us: 10_000_000,
        },
        source: SubtitleSource::ExistingSubtitle,
        content_id: CONTENT_ID.into(),
    };
    assert!(matches!(
        parse(
            SubtitleFormat::Srt,
            body.as_bytes(),
            &stale,
            TextLanguage::TaiwanTraditional,
            &ResolverCancellation::default()
        ),
        Err(ResolverError::Stale)
    ));
    // The exact same script is independent of the capture query's age. No
    // fabricated current WindowStamp is needed to extend a Prepared lifetime.
    let text = prepared(SubtitleFormat::Srt, body.as_bytes()).unwrap();
    assert_eq!(text.identity, identity());
    assert_eq!(&*text.content_id, CONTENT_ID);
    assert_eq!(text.source, SubtitleSource::ExistingSubtitle);
    assert!(text.language == TextLanguage::Source(SpeechLanguage::English));
    assert_eq!(text.cues[0].start_us, Some(0));
    assert_eq!(text.cues[0].duration_us, Some(1_000_000));
    assert_eq!(&*text.cues[0].text, "synthetic source text");
}

#[test]
fn prepared_accepts_whole_media_cues_beyond_live_budget_without_relaxing_live() {
    let body = srt("x").repeat(MAX_SESSION_TEXT_CUES + 1);
    assert_eq!(
        prepared(SubtitleFormat::Srt, body.as_bytes())
            .unwrap()
            .cues
            .len(),
        MAX_SESSION_TEXT_CUES + 1
    );
    let live = TextQuery {
        stamp: WindowStamp {
            identity: identity(),
            sequence: 1,
            ended_at: Instant::now(),
            start_us: Some(0),
            duration_us: 1_000_000,
        },
        source: SubtitleSource::ExistingSubtitle,
        content_id: CONTENT_ID.into(),
    };
    assert!(matches!(
        parse(
            SubtitleFormat::Srt,
            body.as_bytes(),
            &live,
            TextLanguage::TaiwanTraditional,
            &ResolverCancellation::default()
        ),
        Err(ResolverError::InvalidData)
    ));
}

#[test]
fn prepared_cue_count_limit_is_inclusive_and_no_cue_is_silently_truncated() {
    let cue = srt("x");
    let mut body = cue.repeat(MAX_TEXT_CUES);
    assert_eq!(
        prepared(SubtitleFormat::Srt, body.as_bytes())
            .unwrap()
            .cues
            .len(),
        MAX_TEXT_CUES
    );
    body.push_str(&cue);
    rejects(SubtitleFormat::Srt, &body, ResolverError::InvalidData);
}

#[test]
fn prepared_total_decoded_text_budget_is_independent_of_document_overhead() {
    let text_bytes = MAX_TEXT_BYTES / MAX_TEXT_CUES;
    let cue = srt(&"x".repeat(text_bytes));
    let mut body = cue.repeat(MAX_TEXT_CUES - 1);
    body.push_str(&cue);
    let text = prepared(SubtitleFormat::Srt, body.as_bytes()).unwrap();
    assert_eq!(
        text.cues.iter().map(|cue| cue.text.len()).sum::<usize>(),
        MAX_TEXT_BYTES
    );
    // Keep the same cue count, changing only one cue's decoded byte count.
    body.truncate(body.len() - cue.len());
    body.push_str(&srt(&"x".repeat(text_bytes + 1)));
    rejects(SubtitleFormat::Srt, &body, ResolverError::InvalidData);
}

#[test]
fn prepared_raw_body_budget_is_inclusive_and_checked_before_parsing() {
    let mut body = srt("text");
    body.push_str(&" ".repeat(MAX_PREPARED_DOCUMENT_BYTES - body.len()));
    assert!(prepared(SubtitleFormat::Srt, body.as_bytes()).is_ok());
    body.push(' ');
    rejects(SubtitleFormat::Srt, &body, ResolverError::InvalidData);
    let invalid_oversized = vec![0xff; MAX_PREPARED_DOCUMENT_BYTES + 1];
    assert!(matches!(
        prepared(SubtitleFormat::WebVtt, &invalid_oversized),
        Err(ResolverError::InvalidData)
    ));
}

#[test]
fn prepared_per_cue_budget_still_counts_utf8_and_multiline_separators() {
    assert!(
        prepared(
            SubtitleFormat::Srt,
            srt(&"x".repeat(MAX_CUE_BYTES)).as_bytes()
        )
        .is_ok()
    );
    rejects(
        SubtitleFormat::Srt,
        &srt(&"x".repeat(MAX_CUE_BYTES + 1)),
        ResolverError::InvalidData,
    );
    let multiline = format!("{}\nb", "x".repeat(MAX_CUE_BYTES - 2));
    assert!(prepared(SubtitleFormat::WebVtt, vtt(&multiline).as_bytes()).is_ok());
    rejects(
        SubtitleFormat::WebVtt,
        &vtt(&(multiline + "c")),
        ResolverError::InvalidData,
    );
    rejects(
        SubtitleFormat::Srt,
        &srt(&"臺".repeat(MAX_CUE_BYTES / 3 + 1)),
        ResolverError::InvalidData,
    );
}

#[test]
fn prepared_rejects_cancelled_local_and_parent_scopes_before_document_work() {
    let parent = ResolverCancellation::default();
    let child = ResolverCancellation::linked(parent.clone()).unwrap();
    let local = ResolverCancellation::default();
    parent.cancel();
    local.cancel();
    for token in [child, local] {
        assert!(matches!(
            parse_prepared(
                SubtitleFormat::Srt,
                srt("text").as_bytes(),
                identity(),
                CONTENT_ID,
                TextLanguage::TaiwanTraditional,
                &token
            ),
            Err(ResolverError::Cancelled)
        ));
    }
}

#[test]
fn prepared_document_metadata_and_structural_identity_are_validated() {
    for content_id in [
        "".to_owned(),
        "bad\0id".into(),
        "x".repeat(MAX_METADATA_BYTES + 1),
    ] {
        assert!(matches!(
            parse_prepared(
                SubtitleFormat::Srt,
                srt("text").as_bytes(),
                identity(),
                &content_id,
                TextLanguage::TaiwanTraditional,
                &ResolverCancellation::default()
            ),
            Err(ResolverError::InvalidData)
        ));
    }
    let text = prepared(SubtitleFormat::Srt, srt("text").as_bytes()).unwrap();
    let mut other = identity();
    other.generation += 1;
    assert_eq!(
        text.validate_document(other, SubtitleSource::ExistingSubtitle, CONTENT_ID),
        Err(ResolverError::InvalidData)
    );
    assert_eq!(
        text.validate_document(identity(), SubtitleSource::PreparedTranslation, CONTENT_ID),
        Err(ResolverError::InvalidData)
    );
    assert_eq!(
        text.validate_document(
            identity(),
            SubtitleSource::ExistingSubtitle,
            "different edition"
        ),
        Err(ResolverError::InvalidData)
    );
}

#[test]
fn prepared_provider_language_is_preserved_for_all_supported_languages() {
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
        let text = parse_prepared(
            SubtitleFormat::WebVtt,
            vtt("synthetic text 臺灣 컴퓨터").as_bytes(),
            identity(),
            CONTENT_ID,
            language,
            &ResolverCancellation::default(),
        )
        .unwrap();
        assert!(text.language == language);
        assert_eq!(text.source, SubtitleSource::ExistingSubtitle);
        assert_eq!(&*text.cues[0].text, "synthetic text 臺灣 컴퓨터");
    }
}

#[test]
fn prepared_bom_crlf_multiline_and_explicit_hours_preserve_exact_timing() {
    let text = prepared(
        SubtitleFormat::WebVtt,
        "\u{feff}WEBVTT\r\n\r\nsynthetic identifier\r\n01:00:03.001 --> 01:00:04.234\r\n first \r\nsecond\tline\r\n".as_bytes(),
    )
    .unwrap();
    assert_eq!(text.cues[0].start_us, Some(3_603_001_000));
    assert_eq!(text.cues[0].duration_us, Some(1_233_000));
    assert_eq!(&*text.cues[0].text, " first \nsecond\tline");
}

#[test]
fn prepared_charset_and_empty_documents_fail_without_guessed_decoding() {
    for format in [SubtitleFormat::Srt, SubtitleFormat::WebVtt] {
        for body in [
            &[][..],
            &[0xff][..],
            &[0xef, 0xbb][..],
            &[0xff, 0xfe, b'1', 0][..],
        ] {
            assert!(matches!(
                prepared(format, body),
                Err(ResolverError::InvalidData)
            ));
        }
    }
    for body in ["WEBVTT", "WEBVTT\n\n"] {
        rejects(SubtitleFormat::WebVtt, body, ResolverError::InvalidData);
    }
}

#[test]
fn prepared_syntax_never_strips_tags_entities_or_annotations() {
    for text in ["<b>bold</b>", "&amp;", "&#x20;", "{\\an8}text", "one\\Ntwo"] {
        for format in [SubtitleFormat::Srt, SubtitleFormat::WebVtt] {
            let body = if format == SubtitleFormat::Srt {
                srt(text)
            } else {
                vtt(text)
            };
            rejects(format, &body, ResolverError::UnsupportedCapability);
        }
    }
    for annotation in [
        "NOTE hidden",
        "STYLE",
        "REGION",
        "X-TIMESTAMP-MAP=LOCAL:00:00.000,MPEGTS:0",
    ] {
        rejects(
            SubtitleFormat::WebVtt,
            &format!("WEBVTT\n\n{annotation}\nmetadata\n\n00:00.000 --> 00:01.000\ntext"),
            ResolverError::UnsupportedCapability,
        );
    }
    rejects(
        SubtitleFormat::WebVtt,
        "WEBVTT\n\n00:00.000 --> 00:01.000 align:start\ntext",
        ResolverError::UnsupportedCapability,
    );
    rejects(
        SubtitleFormat::Srt,
        "1\n00:00:00,000 --> 00:00:01,000 X1:1\ntext",
        ResolverError::UnsupportedCapability,
    );
}

#[test]
fn prepared_timing_index_order_and_controls_remain_strict() {
    for body in [
        "0\n00:00:00,000 --> 00:00:01,000\ntext",
        "1\n00:60:00,000 --> 01:01:00,000\ntext",
        "1\n00:00:01,000 --> 00:00:01,000\ntext",
        "1\n00:00:02,000 --> 00:00:01,000\ntext",
        "1\n00:00:00,00 --> 00:00:01,000\ntext",
        "1\n00:00:02,000 --> 00:00:03,000\none\n\n2\n00:00:01,000 --> 00:00:02,000\ntwo",
    ] {
        rejects(SubtitleFormat::Srt, body, ResolverError::InvalidData);
    }
    rejects(
        SubtitleFormat::WebVtt,
        "WEBVTT\n\n18446744073709551615:00:00.000 --> 18446744073709551615:00:01.000\ntext",
        ResolverError::InvalidData,
    );
    for text in ["one\rtwo", "one\0two", "one\u{7}two"] {
        rejects(SubtitleFormat::Srt, &srt(text), ResolverError::InvalidData);
    }
}

#[test]
fn prepared_does_not_hide_duplicate_or_overlapping_alignment_ambiguity() {
    let body = "1\n00:00:00,000 --> 00:00:02,000\nrepeat\n\n2\n00:00:00,000 --> 00:00:02,000\nrepeat\n\n3\n00:00:01,000 --> 00:00:03,000\noverlap";
    let text = prepared(SubtitleFormat::Srt, body.as_bytes()).unwrap();
    assert_eq!(text.cues.len(), 3);
    assert_eq!(text.cues[0].start_us, text.cues[1].start_us);
    assert_eq!(text.cues[0].text, text.cues[1].text);
    assert_eq!(text.cues[2].start_us, Some(1_000_000));
}
