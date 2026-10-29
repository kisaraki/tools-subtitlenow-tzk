//! Bounded plain tx3g sample decoding, independent of container I/O and rendering.
//! A caller must validate the track, sample tables, data references and edit list
//! before supplying samples. This core does not discover tracks or grant Ready,
//! source arbitration or playback alignment. Run it on the Prepared CPU lane.
use super::{MediaError, decode_worker::MAX_DECODED_MEDIA_DURATION_US};
use crate::{
    ai::speech::StreamIdentity,
    resolver::contracts::{
        ExistingText, MAX_CUE_BYTES, MAX_TEXT_BYTES, MAX_TEXT_CUES, ResolverCancellation,
        SubtitleSource, TextCue, TextLanguage,
    },
};

/// Empty clearing samples count too; they cannot bypass the work/transport budget.
pub const MAX_TX3G_SAMPLES: usize = 2 * MAX_TEXT_CUES;
pub const MAX_TX3G_SAMPLE_BYTES: usize = MAX_CUE_BYTES + 2;
pub const MAX_TX3G_TRACK_BYTES: usize = 8 * 1024 * 1024;
const MICROS_PER_SECOND: u128 = 1_000_000;

/// Track media time, not wall-clock time. Composition/edit offsets must already
/// be resolved by the container reader; this core never estimates them.
#[derive(Clone, Copy)]
pub struct Tx3gSampleStamp {
    pub identity: StreamIdentity,
    pub sequence: u64,
    pub start_ticks: u64,
    pub duration_ticks: u64,
}

/// Session-only text owner. No Clone, Debug, serialization, export or disk I/O.
/// Any failed sample invalidates the whole builder and releases retained text.
pub struct Tx3gTrack {
    identity: StreamIdentity,
    track_id: u32,
    language: TextLanguage,
    timescale: u32,
    samples: usize,
    wire_bytes: usize,
    text_bytes: usize,
    previous_end: Option<u64>,
    cues: Vec<TextCue>,
    failure: Option<MediaError>,
}
impl Tx3gTrack {
    /// The declared language is explicit; ISO `zho` alone cannot establish zh-TW.
    pub fn new(
        identity: StreamIdentity,
        track_id: u32,
        timescale: u32,
        language: TextLanguage,
    ) -> Result<Self, MediaError> {
        if identity.session.0 == 0 || identity.generation == 0 || track_id == 0 || timescale == 0 {
            return Err(MediaError::InvalidInput);
        }
        Ok(Self {
            identity,
            track_id,
            language,
            timescale,
            samples: 0,
            wire_bytes: 0,
            text_bytes: 0,
            previous_end: None,
            cues: Vec::new(),
            failure: None,
        })
    }
    /// Empty samples clear text during their own duration; they never extend the
    /// preceding cue. Only the zero byte-length string is a clearing sample.
    pub fn push(
        &mut self,
        stamp: Tx3gSampleStamp,
        sample: &[u8],
        cancel: &ResolverCancellation,
    ) -> Result<(), MediaError> {
        let result = self.push_inner(stamp, sample, cancel);
        if let Err(error) = result {
            self.cues = Vec::new();
            self.failure = Some(error);
        }
        result
    }
    fn push_inner(
        &mut self,
        stamp: Tx3gSampleStamp,
        sample: &[u8],
        cancel: &ResolverCancellation,
    ) -> Result<(), MediaError> {
        check(cancel)?;
        if let Some(error) = self.failure {
            return Err(error);
        }
        if stamp.identity != self.identity {
            return Err(MediaError::Stale);
        }
        if stamp.sequence != self.samples as u64 + 1
            || stamp.duration_ticks == 0
            || self.previous_end.is_some_and(|end| stamp.start_ticks < end)
        {
            return Err(MediaError::InvalidSubtitle);
        }
        if self.samples == MAX_TX3G_SAMPLES || sample.len() > MAX_TX3G_SAMPLE_BYTES {
            return Err(MediaError::TooLarge);
        }
        let wire_bytes = self
            .wire_bytes
            .checked_add(sample.len())
            .filter(|bytes| *bytes <= MAX_TX3G_TRACK_BYTES)
            .ok_or(MediaError::TooLarge)?;
        let end_ticks = stamp
            .start_ticks
            .checked_add(stamp.duration_ticks)
            .ok_or(MediaError::InvalidSubtitle)?;
        let start_us = ticks_us(stamp.start_ticks, self.timescale)?;
        let end_us = ticks_us(end_ticks, self.timescale)?;
        let duration_us = end_us
            .checked_sub(start_us)
            .filter(|duration| *duration > 0)
            .ok_or(MediaError::InvalidSubtitle)?;
        let text = decode_sample(sample)?;
        check(cancel)?;
        if let Some(text) = text {
            if self.cues.len() == MAX_TEXT_CUES {
                return Err(MediaError::TooLarge);
            }
            let total = self
                .text_bytes
                .checked_add(text.len())
                .filter(|bytes| *bytes <= MAX_TEXT_BYTES)
                .ok_or(MediaError::TooLarge)?;
            self.cues.push(TextCue {
                start_us: Some(start_us),
                duration_us: Some(duration_us),
                text,
            });
            self.text_bytes = total;
        }
        self.samples += 1;
        self.wire_bytes = wire_bytes;
        self.previous_end = Some(end_ticks);
        Ok(())
    }
    /// Return the complete original document only. Chinese localization and
    /// source-language translation use the existing Prepared stages afterwards.
    pub fn finish(self, cancel: &ResolverCancellation) -> Result<ExistingText, MediaError> {
        check(cancel)?;
        if let Some(error) = self.failure {
            return Err(error);
        }
        let document = ExistingText {
            identity: self.identity,
            source: SubtitleSource::ExistingSubtitle,
            content_id: format!(
                "prepared-tx3g/{}/{}/{}",
                self.identity.session.0, self.identity.generation, self.track_id
            )
            .into(),
            language: self.language,
            cues: self.cues,
        };
        document
            .validate_document(
                self.identity,
                SubtitleSource::ExistingSubtitle,
                &document.content_id,
            )
            .map_err(|_| MediaError::InvalidSubtitle)?;
        check(cancel)?;
        Ok(document)
    }
}
fn check(cancel: &ResolverCancellation) -> Result<(), MediaError> {
    if cancel.is_cancelled() {
        Err(MediaError::Cancelled)
    } else {
        Ok(())
    }
}
fn ticks_us(ticks: u64, timescale: u32) -> Result<u64, MediaError> {
    // Convert absolute endpoints independently. Rounding each duration and
    // accumulating it would drift for timescales such as 90 kHz.
    let micros = u128::from(ticks) * MICROS_PER_SECOND / u128::from(timescale);
    u64::try_from(micros)
        .ok()
        .filter(|micros| *micros <= MAX_DECODED_MEDIA_DURATION_US)
        .ok_or(MediaError::TooLarge)
}
fn decode_sample(sample: &[u8]) -> Result<Option<Box<str>>, MediaError> {
    let header: [u8; 2] = sample
        .get(..2)
        .ok_or(MediaError::InvalidSubtitle)?
        .try_into()
        .map_err(|_| MediaError::InvalidSubtitle)?;
    let length = usize::from(u16::from_be_bytes(header));
    let text = sample
        .get(2..2 + length)
        .ok_or(MediaError::InvalidSubtitle)?;
    if sample.len() != 2 + length {
        // Modifier boxes (style, karaoke, links, positioning, etc.) need their
        // own validation/semantics. Do not silently discard unknown content.
        return Err(MediaError::UnsupportedSubtitle);
    }
    if length == 0 {
        return Ok(None);
    }
    let text = match text {
        [0xfe, 0xff, rest @ ..] => decode_utf16(rest, true)?,
        [0xff, 0xfe, rest @ ..] => decode_utf16(rest, false)?,
        _ => {
            let text = std::str::from_utf8(text).map_err(|_| MediaError::InvalidSubtitle)?;
            text.strip_prefix('\u{feff}').unwrap_or(text).to_owned()
        }
    };
    if text.trim().is_empty()
        || text
            .chars()
            .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
    {
        return Err(MediaError::InvalidSubtitle);
    }
    if text.len() > MAX_CUE_BYTES {
        return Err(MediaError::TooLarge);
    }
    // tx3g hard line/paragraph separators have newline semantics. Nothing is
    // interpreted as HTML/markup, a link, an instruction, or a language guess.
    Ok(Some(text.replace(['\u{2028}', '\u{2029}'], "\n").into()))
}
fn decode_utf16(bytes: &[u8], big_endian: bool) -> Result<String, MediaError> {
    if !bytes.len().is_multiple_of(2) {
        return Err(MediaError::InvalidSubtitle);
    }
    let units = bytes.as_chunks::<2>().0.iter().map(|pair| {
        let word = [pair[0], pair[1]];
        if big_endian {
            u16::from_be_bytes(word)
        } else {
            u16::from_le_bytes(word)
        }
    });
    char::decode_utf16(units)
        .collect::<Result<String, _>>()
        .map_err(|_| MediaError::InvalidSubtitle)
}

#[cfg(test)]
#[path = "embedded_tx3g_tests.rs"]
mod tests;
