//! Bounded line timing parser for the read-only lyrics adapter. No playback
//! alignment, localization, file output or word-level timing is inferred here.
use super::contracts::{MAX_CUE_BYTES, ResolverError, TextCue};

/// Strict adapter subset: bounded parsing work and expanded text, including chorus
/// duplication. General ExistingText contracts permit larger other-provider data.
pub(super) const MAX_LYRIC_CUES: usize = 1024;
const MAX_LYRIC_BYTES: usize = 128 * 1024;

/// Legacy LRC supported here: [minutes:ss[.fraction]], 1–3 fractional digits.
/// Known metadata tags are skipped. Nonzero offsets/enhanced LRC fail explicitly.
fn timestamp(input: &str) -> Result<u64, ResolverError> {
    let (minutes, seconds) = input.split_once(':').ok_or(ResolverError::InvalidData)?;
    if minutes.is_empty() || minutes.len() > 3 || !minutes.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ResolverError::InvalidData);
    }
    let (seconds, fraction) = seconds
        .split_once('.')
        .map_or((seconds, None), |(a, b)| (a, Some(b)));
    if seconds.len() != 2 || !seconds.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ResolverError::InvalidData);
    }
    let minutes: u64 = minutes.parse().map_err(|_| ResolverError::InvalidData)?;
    let seconds: u64 = seconds.parse().map_err(|_| ResolverError::InvalidData)?;
    if seconds >= 60 {
        return Err(ResolverError::InvalidData);
    }
    let micros = match fraction {
        None => 0,
        Some(value)
            if (1..=3).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_digit()) =>
        {
            value
                .parse::<u64>()
                .map_err(|_| ResolverError::InvalidData)?
                * 10u64.pow(6 - value.len() as u32)
        }
        _ => return Err(ResolverError::InvalidData),
    };
    Ok((minutes * 60 + seconds) * 1_000_000 + micros)
}
fn metadata_line(line: &str) -> Result<bool, ResolverError> {
    let Some(tag) = line
        .strip_prefix('[')
        .and_then(|line| line.strip_suffix(']'))
    else {
        return Ok(false);
    };
    let Some((key, value)) = tag.split_once(':') else {
        return Ok(false);
    };
    match key {
        "offset" => {
            if !matches!(value.trim(), "0" | "+0" | "-0") {
                return Err(ResolverError::UnsupportedCapability);
            }
            Ok(true)
        }
        "ar" | "ti" | "al" | "by" | "au" | "re" | "ve" | "length" | "tool" => Ok(true),
        _ => Ok(false),
    }
}
pub(super) fn synced(text: &str, track_duration_us: u64) -> Result<Vec<TextCue>, ResolverError> {
    if text.len() > MAX_LYRIC_BYTES {
        return Err(ResolverError::InvalidData);
    }
    let mut entries: Vec<(u64, Box<str>)> = Vec::new();
    let mut text_bytes = 0usize;
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if metadata_line(line)? {
            continue;
        }
        let mut rest = line;
        let mut times = Vec::new();
        while let Some(body) = rest.strip_prefix('[') {
            let (time, tail) = body.split_once(']').ok_or(ResolverError::InvalidData)?;
            let start = timestamp(time)?;
            if start > track_duration_us || times.len() + entries.len() >= MAX_LYRIC_CUES {
                return Err(ResolverError::InvalidData);
            }
            times.push(start);
            rest = tail;
        }
        if times.is_empty() {
            return Err(ResolverError::InvalidData);
        }
        let text = rest.trim();
        if text.len() > MAX_CUE_BYTES || text.contains('<') || text.contains('>') {
            // Enhanced LRC word markers are deliberately unsupported, not stripped.
            return Err(ResolverError::InvalidData);
        }
        for start in times {
            text_bytes = text_bytes
                .checked_add(text.len())
                .ok_or(ResolverError::InvalidData)?;
            if text_bytes > MAX_LYRIC_BYTES {
                return Err(ResolverError::InvalidData);
            }
            entries.push((start, text.into()));
        }
    }
    entries.sort_by_key(|(start, _)| *start); // Restore multi-tag chorus order, preserve equal-time order.
    let mut cues = Vec::new();
    let mut index = 0;
    while index < entries.len() {
        let start = entries[index].0;
        let end_index = entries[index..]
            .iter()
            .position(|entry| entry.0 != start)
            .map_or(entries.len(), |offset| index + offset);
        let end = entries
            .get(end_index)
            .map_or(track_duration_us, |entry| entry.0);
        for (_, text) in &entries[index..end_index] {
            if !text.is_empty() {
                if end <= start {
                    return Err(ResolverError::InvalidData);
                }
                cues.push(TextCue {
                    start_us: Some(start),
                    duration_us: Some(end - start),
                    text: text.clone(),
                });
            }
        }
        index = end_index;
    }
    if cues.is_empty() {
        return Err(ResolverError::InvalidData);
    }
    Ok(cues)
}
pub(super) fn plain(text: &str) -> Result<Vec<TextCue>, ResolverError> {
    if text.len() > MAX_LYRIC_BYTES {
        return Err(ResolverError::InvalidData);
    }
    let mut cues = Vec::new();
    for text in text.lines().map(str::trim).filter(|text| !text.is_empty()) {
        if cues.len() >= MAX_LYRIC_CUES || text.len() > MAX_CUE_BYTES {
            return Err(ResolverError::InvalidData);
        }
        cues.push(TextCue {
            start_us: None,
            duration_us: None,
            text: text.into(),
        });
    }
    if cues.is_empty() {
        return Err(ResolverError::InvalidData);
    }
    Ok(cues)
}
