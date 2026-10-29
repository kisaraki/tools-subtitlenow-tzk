//! Strict, bounded SRT/WebVTT plain-text subset. Parsing retains provider timing;
//! it never estimates an end time, recognizes language, or grants adoption.
use super::{
    contracts::{
        ExistingText, MAX_CUE_BYTES, MAX_METADATA_BYTES, MAX_TEXT_BYTES, MAX_TEXT_CUES,
        ResolverCancellation, ResolverError, SubtitleSource, TextCue, TextLanguage, TextQuery,
        metadata,
    },
    text_session::{MAX_SESSION_TEXT_BYTES, MAX_SESSION_TEXT_CUES},
    timed_subtitle::SubtitleFormat,
};
use crate::ai::speech::StreamIdentity;
use std::{str::Lines, time::Instant};

/// Transport overhead is bounded separately from the smaller decoded text budget.
pub(super) const MAX_SUBTITLE_DOCUMENT_BYTES: usize = 256 * 1024;
/// Whole-media scripts need a larger transport budget than a Live lookup, but
/// remain bounded independently of the 4 MiB decoded-text and 8192-cue limits.
pub(crate) const MAX_PREPARED_DOCUMENT_BYTES: usize = 8 * 1024 * 1024;
const MICROS_PER_SECOND: u64 = 1_000_000;
const MICROS_PER_MILLISECOND: u64 = 1_000;

#[derive(Clone, Copy)]
enum Scope<'a> {
    Live(&'a TextQuery),
    Prepared {
        identity: StreamIdentity,
        content_id: &'a str,
    },
}
impl Scope<'_> {
    fn check(self, cancel: &ResolverCancellation) -> Result<(), ResolverError> {
        if cancel.is_cancelled() {
            return Err(ResolverError::Cancelled);
        }
        match self {
            Self::Live(query) => query.validate(Instant::now()),
            Self::Prepared { .. } => Ok(()),
        }
    }
    fn limits(self) -> (usize, usize, usize) {
        match self {
            Self::Live(_) => (
                MAX_SUBTITLE_DOCUMENT_BYTES,
                MAX_SESSION_TEXT_CUES,
                MAX_SESSION_TEXT_BYTES,
            ),
            Self::Prepared { .. } => (MAX_PREPARED_DOCUMENT_BYTES, MAX_TEXT_CUES, MAX_TEXT_BYTES),
        }
    }
}

struct Document<'a, 'b> {
    lines: Lines<'a>,
    scope: Scope<'b>,
    cancel: &'b ResolverCancellation,
}
impl<'a> Document<'a, '_> {
    fn next(&mut self) -> Result<Option<&'a str>, ResolverError> {
        self.scope.check(self.cancel)?;
        let line = self.lines.next();
        // str::lines removes CRLF. A remaining CR is an unsupported bare CR.
        if line.is_some_and(|line| line.contains('\r')) {
            return Err(ResolverError::InvalidData);
        }
        Ok(line)
    }
    fn nonblank(&mut self) -> Result<Option<&'a str>, ResolverError> {
        while let Some(line) = self.next()? {
            if !line.trim().is_empty() {
                return Ok(Some(line));
            }
        }
        Ok(None)
    }
}

fn decimal(input: &str, min: usize, max: usize) -> Result<u64, ResolverError> {
    if !(min..=max).contains(&input.len()) || !input.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ResolverError::InvalidData);
    }
    input.parse().map_err(|_| ResolverError::InvalidData)
}

fn timestamp(input: &str, format: SubtitleFormat) -> Result<u64, ResolverError> {
    let mut parts = input.split(':');
    let first = parts.next().ok_or(ResolverError::InvalidData)?;
    let second = parts.next().ok_or(ResolverError::InvalidData)?;
    let third = parts.next();
    if parts.next().is_some() {
        return Err(ResolverError::InvalidData);
    }
    let (hours, minutes, seconds) = match (format, third) {
        (SubtitleFormat::Srt, Some(seconds)) => {
            (decimal(first, 2, 2)?, decimal(second, 2, 2)?, seconds)
        }
        (SubtitleFormat::WebVtt, Some(seconds)) => {
            (decimal(first, 2, 20)?, decimal(second, 2, 2)?, seconds)
        }
        (SubtitleFormat::WebVtt, None) => (0, decimal(first, 2, 2)?, second),
        (SubtitleFormat::Srt, None) => return Err(ResolverError::InvalidData),
    };
    let delimiter = match format {
        SubtitleFormat::Srt => ',',
        SubtitleFormat::WebVtt => '.',
    };
    let (seconds, millis) = seconds
        .split_once(delimiter)
        .ok_or(ResolverError::InvalidData)?;
    let seconds = decimal(seconds, 2, 2)?;
    let millis = decimal(millis, 3, 3)?;
    if minutes >= 60 || seconds >= 60 {
        return Err(ResolverError::InvalidData);
    }
    hours
        .checked_mul(3600)
        .and_then(|value| value.checked_add(minutes * 60))
        .and_then(|value| value.checked_add(seconds))
        .and_then(|value| value.checked_mul(MICROS_PER_SECOND))
        .and_then(|value| value.checked_add(millis * MICROS_PER_MILLISECOND))
        .ok_or(ResolverError::InvalidData)
}

fn timing(line: &str, format: SubtitleFormat) -> Result<(u64, u64), ResolverError> {
    let (start, end) = line.split_once("-->").ok_or(ResolverError::InvalidData)?;
    let mut start_parts = start.split_whitespace();
    let start = start_parts.next().ok_or(ResolverError::InvalidData)?;
    let mut end_parts = end.split_whitespace();
    let end = end_parts.next().ok_or(ResolverError::InvalidData)?;
    if start_parts.next().is_some() || end_parts.next().is_some() {
        // Cue settings and positioning annotations require explicit renderer support.
        return Err(ResolverError::UnsupportedCapability);
    }
    let start = timestamp(start, format)?;
    let end = timestamp(end, format)?;
    let duration = end
        .checked_sub(start)
        .filter(|duration| *duration > 0)
        .ok_or(ResolverError::InvalidData)?;
    Ok((start, duration))
}

fn unsupported_text(line: &str) -> bool {
    // Fail closed instead of stripping tags, entities, ASS positioning or hard
    // line-break escapes. Ordinary ampersands without entity syntax remain text.
    line.contains(['<', '>'])
        || line.contains("{\\")
        || line.contains("\\N")
        || line.contains("\\n")
        || line.match_indices('&').any(|(index, _)| {
            // Scan only the bounded entity prefix, avoiding repeated scans of a
            // long cue tail containing many ordinary ampersands.
            for (offset, byte) in line.as_bytes()[index + 1..].iter().take(33).enumerate() {
                if *byte == b';' {
                    return offset != 0;
                }
                if !byte.is_ascii_alphanumeric() && *byte != b'#' {
                    return false;
                }
            }
            false
        })
}

fn text(document: &mut Document<'_, '_>) -> Result<Box<str>, ResolverError> {
    let mut result = String::new();
    while let Some(line) = document.next()? {
        if line.trim().is_empty() {
            break;
        }
        let separator = usize::from(!result.is_empty());
        let length = result
            .len()
            .checked_add(separator)
            .and_then(|length| length.checked_add(line.len()))
            .ok_or(ResolverError::InvalidData)?;
        if length > MAX_CUE_BYTES || line.chars().any(|c| c.is_control() && c != '\t') {
            return Err(ResolverError::InvalidData);
        }
        if unsupported_text(line) {
            return Err(ResolverError::UnsupportedCapability);
        }
        if separator != 0 {
            result.push('\n');
        }
        result.push_str(line);
    }
    if result.trim().is_empty() {
        return Err(ResolverError::InvalidData);
    }
    Ok(result.into_boxed_str())
}

fn vtt_annotation(line: &str) -> bool {
    ["NOTE", "STYLE", "REGION", "X-TIMESTAMP-MAP"]
        .iter()
        .any(|prefix| {
            line == *prefix
                || line
                    .strip_prefix(*prefix)
                    .is_some_and(|tail| tail.starts_with([' ', '\t', '=', ':']))
        })
}

/// UTF-8 (optional BOM), LF/CRLF, positive numeric SRT index, or WEBVTT header
/// followed immediately by its blank terminator. VTT cue identifiers are optional.
/// Only explicit positive intervals and plain multiline text are supported. Cue
/// duplicates/overlaps are retained; later alignment must handle their ambiguity.
pub(super) fn parse(
    format: SubtitleFormat,
    body: &[u8],
    query: &TextQuery,
    language: TextLanguage,
    cancel: &ResolverCancellation,
) -> Result<ExistingText, ResolverError> {
    Scope::Live(query).check(cancel)?;
    if query.source != SubtitleSource::ExistingSubtitle {
        return Err(ResolverError::InvalidData);
    }
    parse_document(format, body, Scope::Live(query), language, cancel)
}

/// Prepare a bounded whole-media document without a Live capture timestamp or
/// its five-second freshness lease. The caller owns a unique Session and runs
/// this CPU work on its bounded blocking lane. This is original subtitle text,
/// not translated/adopted text, and parsing never starts playback.
pub(crate) fn parse_prepared(
    format: SubtitleFormat,
    body: &[u8],
    identity: StreamIdentity,
    content_id: &str,
    language: TextLanguage,
    cancel: &ResolverCancellation,
) -> Result<ExistingText, ResolverError> {
    parse_document(
        format,
        body,
        Scope::Prepared {
            identity,
            content_id,
        },
        language,
        cancel,
    )
}

fn parse_document(
    format: SubtitleFormat,
    body: &[u8],
    scope: Scope<'_>,
    language: TextLanguage,
    cancel: &ResolverCancellation,
) -> Result<ExistingText, ResolverError> {
    scope.check(cancel)?;
    // Validate fixed metadata once, rather than scanning it for every blank
    // line in an otherwise bounded whole-media document.
    if matches!(scope, Scope::Prepared { content_id, .. } if !metadata(content_id)) {
        return Err(ResolverError::InvalidData);
    }
    let (max_document_bytes, max_cues, max_text_bytes) = scope.limits();
    if body.len() > max_document_bytes {
        return Err(ResolverError::InvalidData);
    }
    let body = std::str::from_utf8(body).map_err(|_| ResolverError::InvalidData)?;
    let body = body.strip_prefix('\u{feff}').unwrap_or(body);
    let mut document = Document {
        lines: body.lines(),
        scope,
        cancel,
    };
    if format == SubtitleFormat::WebVtt {
        let header = document.next()?.ok_or(ResolverError::InvalidData)?;
        if header != "WEBVTT" {
            return Err(if header.starts_with("WEBVTT") {
                ResolverError::UnsupportedCapability
            } else {
                ResolverError::InvalidData
            });
        }
        let end = document.next()?.ok_or(ResolverError::InvalidData)?;
        if !end.trim().is_empty() {
            return Err(ResolverError::UnsupportedCapability);
        }
    }
    let mut cues: Vec<TextCue> = Vec::new();
    let mut total = 0usize;
    while let Some(first) = document.nonblank()? {
        if cues.len() >= max_cues {
            return Err(ResolverError::InvalidData);
        }
        let timing_line = match format {
            SubtitleFormat::Srt => {
                if decimal(first, 1, 20)? == 0 {
                    return Err(ResolverError::InvalidData);
                }
                document.next()?.ok_or(ResolverError::InvalidData)?
            }
            SubtitleFormat::WebVtt => {
                if vtt_annotation(first) {
                    return Err(ResolverError::UnsupportedCapability);
                }
                if first.contains("-->") {
                    first
                } else {
                    if first.len() > MAX_METADATA_BYTES
                        || first.chars().any(char::is_control)
                        || unsupported_text(first)
                    {
                        return Err(ResolverError::InvalidData);
                    }
                    document.next()?.ok_or(ResolverError::InvalidData)?
                }
            }
        };
        let (start, duration) = timing(timing_line, format)?;
        if cues
            .last()
            .and_then(|cue| cue.start_us)
            .is_some_and(|previous| start < previous)
        {
            return Err(ResolverError::InvalidData);
        }
        let text = text(&mut document)?;
        total = total
            .checked_add(text.len())
            .filter(|total| *total <= max_text_bytes)
            .ok_or(ResolverError::InvalidData)?;
        cues.push(TextCue {
            start_us: Some(start),
            duration_us: Some(duration),
            text,
        });
    }
    scope.check(cancel)?;
    let (identity, content_id) = match scope {
        Scope::Live(query) => (query.stamp.identity, &*query.content_id),
        Scope::Prepared {
            identity,
            content_id,
        } => (identity, content_id),
    };
    let result = ExistingText {
        identity,
        source: SubtitleSource::ExistingSubtitle,
        content_id: content_id.into(),
        language,
        cues,
    };
    match scope {
        Scope::Live(query) => result.validate(query)?,
        Scope::Prepared { .. } => {
            result.validate_document(identity, SubtitleSource::ExistingSubtitle, content_id)?
        }
    }
    scope.check(cancel)?;
    Ok(result)
}

#[cfg(test)]
#[path = "timed_subtitle_parse_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "prepared_subtitle_parse_tests.rs"]
mod prepared_tests;
