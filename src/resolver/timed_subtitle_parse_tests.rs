//! Synthetic documents only. No files, network, keys or actual media content.
use super::*;
use crate::{
    ai::speech::{SpeechLanguage, StreamIdentity},
    audio::capture::SessionId,
    resolver::contracts::{MAX_WINDOW_AGE, WindowStamp},
};

fn query() -> TextQuery {
    TextQuery {
        stamp: WindowStamp {
            identity: StreamIdentity {
                session: SessionId(711),
                generation: 3,
            },
            sequence: 19,
            ended_at: Instant::now(),
            start_us: Some(3_000_000),
            duration_us: 10_000_000,
        },
        source: SubtitleSource::ExistingSubtitle,
        content_id: "synthetic-subtitle-release".into(),
    }
}
fn parse_text(format: SubtitleFormat, text: &str) -> Result<ExistingText, ResolverError> {
    parse(
        format,
        text.as_bytes(),
        &query(),
        TextLanguage::Source(SpeechLanguage::English),
        &ResolverCancellation::default(),
    )
}
fn rejects(format: SubtitleFormat, text: &str, error: ResolverError) {
    assert!(matches!(parse_text(format, text), Err(actual) if actual == error));
}
fn srt(text: &str) -> String {
    format!("1\n00:00:00,000 --> 00:00:01,000\n{text}\n")
}
fn vtt(text: &str) -> String {
    format!("WEBVTT\n\n00:00.000 --> 00:01.000\n{text}\n")
}

#[test]
fn srt_bom_crlf_multiline_text_and_query_identity_are_preserved() {
    let query = query();
    let parsed = parse(
        SubtitleFormat::Srt,
        "\u{feff}12\r\n00:00:01,001 --> 00:00:02,234\r\n  first line  \r\nsecond\tline\r\n\r\n4\r\n00:00:02,300 --> 00:00:03,000\r\nlast cue".as_bytes(),
        &query,
        TextLanguage::Source(SpeechLanguage::English),
        &ResolverCancellation::default(),
    )
    .unwrap();
    assert_eq!(parsed.identity, query.stamp.identity);
    assert_eq!(parsed.source, SubtitleSource::ExistingSubtitle);
    assert_eq!(parsed.content_id, query.content_id);
    assert_eq!(parsed.cues.len(), 2);
    assert_eq!(parsed.cues[0].start_us, Some(1_001_000));
    assert_eq!(parsed.cues[0].duration_us, Some(1_233_000));
    assert_eq!(&*parsed.cues[0].text, "  first line  \nsecond\tline");
    assert_eq!(parsed.cues[1].duration_us, Some(700_000));
}

#[test]
fn webvtt_accepts_explicit_hours_or_minutes_and_optional_cue_identifiers() {
    let parsed = parse_text(
        SubtitleFormat::WebVtt,
        "\u{feff}WEBVTT\r\n\r\ncue identifier\r\n00:01.001 --> 00:02.250\r\nfirst\r\nsecond\r\n\r\n01:00:03.000 --> 01:00:04.001\r\nlast",
    )
    .unwrap();
    assert_eq!(parsed.cues.len(), 2);
    assert_eq!(parsed.cues[0].start_us, Some(1_001_000));
    assert_eq!(parsed.cues[0].duration_us, Some(1_249_000));
    assert_eq!(&*parsed.cues[0].text, "first\nsecond");
    assert_eq!(parsed.cues[1].start_us, Some(3_603_000_000));
    assert_eq!(parsed.cues[1].duration_us, Some(1_001_000));
}

#[test]
fn provider_declared_five_source_and_three_chinese_languages_are_not_guessed() {
    let languages = [
        TextLanguage::Source(SpeechLanguage::English),
        TextLanguage::Source(SpeechLanguage::Japanese),
        TextLanguage::Source(SpeechLanguage::French),
        TextLanguage::Source(SpeechLanguage::German),
        TextLanguage::Source(SpeechLanguage::Korean),
        TextLanguage::TaiwanTraditional,
        TextLanguage::OtherTraditional,
        TextLanguage::SimplifiedChinese,
    ];
    for language in languages {
        for format in [SubtitleFormat::Srt, SubtitleFormat::WebVtt] {
            let body = match format {
                SubtitleFormat::Srt => srt("Synthetic text 臺灣 컴퓨터"),
                SubtitleFormat::WebVtt => vtt("Synthetic text 臺灣 컴퓨터"),
            };
            let parsed = parse(
                format,
                body.as_bytes(),
                &query(),
                language,
                &ResolverCancellation::default(),
            )
            .unwrap();
            assert!(parsed.language == language);
            assert_eq!(&*parsed.cues[0].text, "Synthetic text 臺灣 컴퓨터");
        }
    }
}

#[test]
fn malformed_utf8_empty_or_header_only_documents_are_invalid() {
    for format in [SubtitleFormat::Srt, SubtitleFormat::WebVtt] {
        for body in [&[][..], &[0xff][..], &[0xef, 0xbb][..]] {
            assert!(matches!(
                parse(
                    format,
                    body,
                    &query(),
                    TextLanguage::TaiwanTraditional,
                    &ResolverCancellation::default()
                ),
                Err(ResolverError::InvalidData)
            ));
        }
    }
    rejects(SubtitleFormat::Srt, "\n \n", ResolverError::InvalidData);
    rejects(
        SubtitleFormat::WebVtt,
        "WEBVTT\n\n",
        ResolverError::InvalidData,
    );
    rejects(SubtitleFormat::WebVtt, "WEBVTT", ResolverError::InvalidData);
}

#[test]
fn srt_requires_positive_decimal_index_without_implicit_numbering() {
    for index in ["0", "-1", "+1", "1a", " 1 ", "18446744073709551616"] {
        rejects(
            SubtitleFormat::Srt,
            &format!("{index}\n00:00:00,000 --> 00:00:01,000\ntext"),
            ResolverError::InvalidData,
        );
    }
    rejects(
        SubtitleFormat::Srt,
        "00:00:00,000 --> 00:00:01,000\ntext",
        ResolverError::InvalidData,
    );
}

#[test]
fn timestamp_fields_and_fraction_precision_are_strict() {
    for start in [
        "0:00:00,000",
        "00:0:00,000",
        "00:00:0,000",
        "00:00:00,00",
        "00:00:00,0000",
        "00:00:00.000",
        "00:60:00,000",
        "00:00:60,000",
        "00:00,000",
        "-1:00:00,000",
        "00:00:00,0a0",
        "00:00:00:00,000",
    ] {
        rejects(
            SubtitleFormat::Srt,
            &format!("1\n{start} --> 99:59:59,999\ntext"),
            ResolverError::InvalidData,
        );
    }
    for start in [
        "0:00.000",
        "00:0.000",
        "00:00.00",
        "00:00.0000",
        "00:00,000",
        "60:00.000",
        "00:60.000",
        "0:00:00.000",
        "00:60:00.000",
        "00:00:60.000",
        "00:00.0a0",
        "00:00:00:00.000",
    ] {
        rejects(
            SubtitleFormat::WebVtt,
            &format!("WEBVTT\n\n{start} --> 99:59:59.999\ntext"),
            ResolverError::InvalidData,
        );
    }
}

#[test]
fn timestamp_arithmetic_never_wraps_large_webvtt_hours() {
    for hours in [
        "18446744073709551615",
        "18446744073709551616",
        "999999999999999999999",
    ] {
        rejects(
            SubtitleFormat::WebVtt,
            &format!("WEBVTT\n\n{hours}:00:00.000 --> {hours}:00:01.000\ntext"),
            ResolverError::InvalidData,
        );
    }
    let parsed = parse_text(
        SubtitleFormat::Srt,
        "1\n99:59:59,998 --> 99:59:59,999\ntext",
    )
    .unwrap();
    assert_eq!(parsed.cues[0].start_us, Some(359_999_998_000));
    assert_eq!(parsed.cues[0].duration_us, Some(1_000));
}

#[test]
fn zero_or_reversed_intervals_are_invalid() {
    for (format, header, start, end) in [
        (SubtitleFormat::Srt, "1\n", "00:00:01,000", "00:00:01,000"),
        (SubtitleFormat::Srt, "1\n", "00:00:02,000", "00:00:01,000"),
        (
            SubtitleFormat::WebVtt,
            "WEBVTT\n\n",
            "00:01.000",
            "00:01.000",
        ),
        (
            SubtitleFormat::WebVtt,
            "WEBVTT\n\n",
            "00:02.000",
            "00:01.000",
        ),
    ] {
        rejects(
            format,
            &format!("{header}{start} --> {end}\ntext"),
            ResolverError::InvalidData,
        );
    }
}

#[test]
fn decreasing_starts_are_rejected_instead_of_silently_sorted() {
    rejects(
        SubtitleFormat::Srt,
        "1\n00:00:02,000 --> 00:00:03,000\nlater\n\n2\n00:00:01,000 --> 00:00:02,000\nearlier",
        ResolverError::InvalidData,
    );
    rejects(
        SubtitleFormat::WebVtt,
        "WEBVTT\n\n00:02.000 --> 00:03.000\nlater\n\n00:01.000 --> 00:02.000\nearlier",
        ResolverError::InvalidData,
    );
}

#[test]
fn equal_starts_duplicates_and_overlaps_remain_visible_to_alignment() {
    let parsed = parse_text(
        SubtitleFormat::Srt,
        "1\n00:00:00,000 --> 00:00:02,000\nrepeat\n\n1\n00:00:00,000 --> 00:00:02,000\nrepeat\n\n2\n00:00:01,000 --> 00:00:03,000\noverlap",
    )
    .unwrap();
    assert_eq!(parsed.cues.len(), 3);
    assert_eq!(parsed.cues[0].start_us, parsed.cues[1].start_us);
    assert_eq!(parsed.cues[0].duration_us, parsed.cues[1].duration_us);
    assert_eq!(parsed.cues[0].text, parsed.cues[1].text);
    assert_eq!(parsed.cues[2].start_us, Some(1_000_000));
}

#[test]
fn truncated_or_empty_cues_never_receive_invented_end_times_or_text() {
    for body in [
        "1",
        "1\n00:00:00,000",
        "1\n00:00:00,000 -->",
        "1\n00:00:00,000 --> 00:00:01,000",
        "1\n00:00:00,000 --> 00:00:01,000\n \n",
        "1\n\ntext",
    ] {
        rejects(SubtitleFormat::Srt, body, ResolverError::InvalidData);
    }
    for body in [
        "WEBVTT\n\nidentifier",
        "WEBVTT\n\n00:00.000 -->",
        "WEBVTT\n\n00:00.000 --> 00:01.000",
        "WEBVTT\n\n00:00.000 --> 00:01.000\n\t\n",
    ] {
        rejects(SubtitleFormat::WebVtt, body, ResolverError::InvalidData);
    }
}

#[test]
fn html_entities_tags_and_ass_annotations_are_explicitly_unsupported() {
    for text in [
        "<b>bold</b>",
        "<00:01.000>word",
        "&amp;",
        "&#32;",
        "&#x20;",
        "&nbsp;",
        "{\\an8}positioned",
        "first\\Nsecond",
        "first\\nsecond",
    ] {
        rejects(
            SubtitleFormat::Srt,
            &srt(text),
            ResolverError::UnsupportedCapability,
        );
        rejects(
            SubtitleFormat::WebVtt,
            &vtt(text),
            ResolverError::UnsupportedCapability,
        );
    }
    for text in ["R&B", "rock & roll", "a & b;", "a {plain} sentence"] {
        let parsed = parse_text(SubtitleFormat::Srt, &srt(text)).unwrap();
        assert_eq!(&*parsed.cues[0].text, text);
    }
}

#[test]
fn dense_ampersands_and_long_nonentity_text_are_preserved_with_bounded_scanning() {
    for text in [
        "&".repeat(MAX_CUE_BYTES),
        format!("&{};", "a".repeat(MAX_CUE_BYTES - 2)),
        format!("{}&amp;", "&".repeat(MAX_CUE_BYTES - 5)),
    ] {
        if text.ends_with("&amp;") {
            rejects(
                SubtitleFormat::Srt,
                &srt(&text),
                ResolverError::UnsupportedCapability,
            );
        } else {
            let parsed = parse_text(SubtitleFormat::Srt, &srt(&text)).unwrap();
            assert_eq!(&*parsed.cues[0].text, text);
        }
    }
}

#[test]
fn webvtt_notes_styles_regions_maps_and_nonempty_headers_are_not_rendered() {
    for prefix in [
        "NOTE",
        "NOTE hidden comment",
        "NOTE\thidden comment",
        "STYLE",
        "REGION",
        "X-TIMESTAMP-MAP=LOCAL:00:00.000,MPEGTS:0",
    ] {
        rejects(
            SubtitleFormat::WebVtt,
            &format!("WEBVTT\n\n{prefix}\nprivate metadata\n\n00:00.000 --> 00:01.000\ntext"),
            ResolverError::UnsupportedCapability,
        );
    }
    for header in [
        "WEBVTT annotation\n\n",
        "WEBVTT\nLanguage: en\n\n",
        "WEBVTT\nX-TIMESTAMP-MAP=LOCAL:00:00.000,MPEGTS:0\n\n",
    ] {
        rejects(
            SubtitleFormat::WebVtt,
            &format!("{header}00:00.000 --> 00:01.000\ntext"),
            ResolverError::UnsupportedCapability,
        );
    }
}

#[test]
fn cue_settings_and_positioning_do_not_silently_change_the_plain_text_subset() {
    rejects(
        SubtitleFormat::Srt,
        "1\n00:00:00,000 --> 00:00:01,000 X1:1 X2:2\ntext",
        ResolverError::UnsupportedCapability,
    );
    for setting in [
        "line:90%",
        "align:start",
        "vertical:rl",
        "position:50%",
        "region:example",
    ] {
        rejects(
            SubtitleFormat::WebVtt,
            &format!("WEBVTT\n\n00:00.000 --> 00:01.000 {setting}\ntext"),
            ResolverError::UnsupportedCapability,
        );
    }
}

#[test]
fn missing_block_separator_cannot_leak_a_timing_line_into_subtitle_text() {
    rejects(
        SubtitleFormat::Srt,
        "1\n00:00:00,000 --> 00:00:01,000\none\n2\n00:00:01,000 --> 00:00:02,000\ntwo",
        ResolverError::UnsupportedCapability,
    );
    rejects(
        SubtitleFormat::WebVtt,
        "WEBVTT\n\n00:00.000 --> 00:01.000\none\n00:01.000 --> 00:02.000\ntwo",
        ResolverError::UnsupportedCapability,
    );
}

#[test]
fn raw_document_budget_is_inclusive_and_checked_before_parsing() {
    let mut body = srt("text");
    body.push('\n');
    body.push_str(&" ".repeat(MAX_SUBTITLE_DOCUMENT_BYTES - body.len()));
    assert!(parse_text(SubtitleFormat::Srt, &body).is_ok());
    body.push(' ');
    rejects(SubtitleFormat::Srt, &body, ResolverError::InvalidData);
}

#[test]
fn cue_text_byte_limit_includes_multiline_separators_and_utf8_bytes() {
    assert!(parse_text(SubtitleFormat::Srt, &srt(&"a".repeat(MAX_CUE_BYTES))).is_ok());
    rejects(
        SubtitleFormat::Srt,
        &srt(&"a".repeat(MAX_CUE_BYTES + 1)),
        ResolverError::InvalidData,
    );
    let text = format!("{}\nb", "a".repeat(MAX_CUE_BYTES - 2));
    assert!(parse_text(SubtitleFormat::WebVtt, &vtt(&text)).is_ok());
    rejects(
        SubtitleFormat::WebVtt,
        &vtt(&(text + "b")),
        ResolverError::InvalidData,
    );
    rejects(
        SubtitleFormat::Srt,
        &srt(&"臺".repeat(MAX_CUE_BYTES / 3 + 1)),
        ResolverError::InvalidData,
    );
}

#[test]
fn total_decoded_text_budget_is_independent_of_document_overhead() {
    let cue = srt(&"a".repeat(MAX_CUE_BYTES)) + "\n";
    let body = cue.repeat(MAX_SESSION_TEXT_BYTES / MAX_CUE_BYTES);
    let parsed = parse_text(SubtitleFormat::Srt, &body).unwrap();
    assert_eq!(
        parsed.cues.iter().map(|cue| cue.text.len()).sum::<usize>(),
        MAX_SESSION_TEXT_BYTES
    );
    rejects(
        SubtitleFormat::Srt,
        &(body + &srt("b")),
        ResolverError::InvalidData,
    );
}

#[test]
fn cue_count_is_bounded_even_with_small_duplicate_text() {
    let cue = srt("x") + "\n";
    let body = cue.repeat(MAX_SESSION_TEXT_CUES);
    assert_eq!(
        parse_text(SubtitleFormat::Srt, &body).unwrap().cues.len(),
        MAX_SESSION_TEXT_CUES
    );
    rejects(
        SubtitleFormat::Srt,
        &(body + &cue),
        ResolverError::InvalidData,
    );
}

#[test]
fn cancellation_and_stale_queries_reject_even_otherwise_valid_documents() {
    let cancel = ResolverCancellation::default();
    cancel.cancel();
    assert!(matches!(
        parse(
            SubtitleFormat::Srt,
            srt("text").as_bytes(),
            &query(),
            TextLanguage::TaiwanTraditional,
            &cancel
        ),
        Err(ResolverError::Cancelled)
    ));
    let mut stale = query();
    stale.stamp.ended_at = Instant::now() - MAX_WINDOW_AGE - std::time::Duration::from_millis(1);
    assert!(matches!(
        parse(
            SubtitleFormat::WebVtt,
            vtt("text").as_bytes(),
            &stale,
            TextLanguage::TaiwanTraditional,
            &ResolverCancellation::default()
        ),
        Err(ResolverError::Stale)
    ));
}

#[test]
fn unsupported_query_source_or_invalid_identity_metadata_is_not_rebound() {
    for source in [
        SubtitleSource::ExistingLyrics,
        SubtitleSource::LiveAi,
        SubtitleSource::Hybrid,
    ] {
        let mut query = query();
        query.source = source;
        assert!(matches!(
            parse(
                SubtitleFormat::Srt,
                srt("text").as_bytes(),
                &query,
                TextLanguage::TaiwanTraditional,
                &ResolverCancellation::default()
            ),
            Err(ResolverError::InvalidData)
        ));
    }
    let mut query = query();
    query.content_id = "private\0invalid-id".into();
    assert!(matches!(
        parse(
            SubtitleFormat::Srt,
            srt("text").as_bytes(),
            &query,
            TextLanguage::TaiwanTraditional,
            &ResolverCancellation::default()
        ),
        Err(ResolverError::InvalidData)
    ));
    query.content_id = "synthetic".into();
    query.stamp.duration_us = 0;
    assert!(matches!(
        parse(
            SubtitleFormat::Srt,
            srt("text").as_bytes(),
            &query,
            TextLanguage::TaiwanTraditional,
            &ResolverCancellation::default()
        ),
        Err(ResolverError::InvalidData)
    ));
}

#[test]
fn control_characters_bare_cr_and_oversized_vtt_ids_are_invalid() {
    for text in ["one\0two", "one\u{7}two", "one\rtwo"] {
        rejects(SubtitleFormat::Srt, &srt(text), ResolverError::InvalidData);
        rejects(
            SubtitleFormat::WebVtt,
            &vtt(text),
            ResolverError::InvalidData,
        );
    }
    rejects(
        SubtitleFormat::WebVtt,
        &format!(
            "WEBVTT\n\n{}\n00:00.000 --> 00:01.000\ntext",
            "x".repeat(MAX_METADATA_BYTES + 1)
        ),
        ResolverError::InvalidData,
    );
}
