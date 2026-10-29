//! Bounded content preflight over the same pinned local file used by MF, or
//! the private, quota-controlled remote range reader. No location-based hints.
//! This is a format gate, not a complete decoder or media matching evidence.
use super::MediaError;
use crate::resolver::contracts::ResolverCancellation;
use std::io::{ErrorKind, Read, Seek, SeekFrom};

/// Includes the ID3 header/footer. Large artwork/metadata is intentionally not
/// supported by this first MP3 path; never allocate or scan an arbitrary tag.
const MAX_ID3_TAG_BYTES: u64 = 1024 * 1024;
const ID3_HEADER_BYTES: u64 = 10;
const MAX_INTERRUPTED_READS: usize = 32;
const MPEG1_SAMPLE_RATES: [u32; 3] = [44_100, 48_000, 32_000];
/// First AAC path: MPEG-4 AAC-LC, one raw block, explicit mono/stereo at the
/// two rates exercised by the Windows encoder. No HE-AAC/PCE/implicit format.
const ADTS_HEADER_BYTES: usize = 7;
const MPEG1_LAYER3_KBPS: [u32; 16] = [
    0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LocalAudioFormat {
    Wave,
    Mp3,
    Adts,
    Mp4,
}
impl LocalAudioFormat {
    pub(crate) const fn mime(self) -> &'static str {
        match self {
            Self::Wave => "audio/wav",
            Self::Mp3 => "audio/mpeg",
            Self::Adts => "audio/aac",
            Self::Mp4 => "audio/mp4",
        }
    }
    pub(crate) const fn uses_packet_clock(self) -> bool {
        !matches!(self, Self::Wave)
    }
}

/// No filename, extension, URL or whole-file buffer participates. At most 31
/// bytes for WAVE/MP3, or 20 for ADTS. MP4 has its own bounded metadata gate;
/// media payload is never buffered here. Success resets the cursor.
pub(crate) fn probe<R: Read + Seek>(
    reader: &mut R,
    byte_len: u64,
    cancel: &ResolverCancellation,
) -> Result<LocalAudioFormat, MediaError> {
    check(cancel)?;
    let mut prefix = [0u8; 12];
    read_at(reader, 0, &mut prefix, byte_len, cancel)?;
    let format = if &prefix[..4] == b"RIFF" && &prefix[8..] == b"WAVE" {
        LocalAudioFormat::Wave
    } else if &prefix[4..8] == b"ftyp" {
        mp4::probe(reader, byte_len, cancel)?;
        LocalAudioFormat::Mp4
    } else if prefix[0] == 0xff && prefix[1] & 0xf0 == 0xf0 {
        // MPEG-1 Layer III uses 0xfb/0xfa; require the ADTS layer bits first.
        if prefix[1] & 0x06 == 0 {
            probe_adts(reader, &prefix, byte_len, cancel)?
        } else {
            probe_mp3(reader, &prefix, byte_len, cancel)?
        }
    } else {
        probe_mp3(reader, &prefix, byte_len, cancel)?
    };
    seek(reader, 0, cancel)?;
    Ok(format)
}
fn probe_mp3<R: Read + Seek>(
    reader: &mut R,
    prefix: &[u8; 12],
    byte_len: u64,
    cancel: &ResolverCancellation,
) -> Result<LocalAudioFormat, MediaError> {
    let offset = if &prefix[..3] == b"ID3" {
        id3_audio_offset(reader, prefix, byte_len, cancel)?
    } else {
        0
    };
    let mut header = [0u8; 4];
    read_at(reader, offset, &mut header, byte_len, cancel)?;
    let first = mp3_frame(header)?;
    let second_offset = offset
        .checked_add(u64::from(first.bytes))
        .ok_or(MediaError::UnsupportedMedia)?;
    read_at(reader, second_offset, &mut header, byte_len, cancel)?;
    let second = mp3_frame(header)?;
    if first.rate != second.rate || first.channels != second.channels {
        return Err(MediaError::UnsupportedMedia);
    }
    // Two complete adjacent MPEG-1 Layer III frames are required. Bit rate
    // and padding may vary; do not mistake VBR for a new PCM format.
    let end = second_offset
        .checked_add(u64::from(second.bytes))
        .ok_or(MediaError::UnsupportedMedia)?;
    read_at(reader, end - 1, &mut [0u8; 1], byte_len, cancel)?;
    Ok(LocalAudioFormat::Mp3)
}
#[derive(Clone, Copy)]
struct AdtsFrame {
    bytes: u64,
    rate_index: u8,
    channels: u8,
}
fn adts_frame(header: &[u8]) -> Result<AdtsFrame, MediaError> {
    let rate_index = (header[2] >> 2) & 0x0f;
    let channels = ((header[2] & 1) << 2) | (header[3] >> 6);
    let length =
        (u64::from(header[3] & 3) << 11) | (u64::from(header[4]) << 3) | u64::from(header[5] >> 5);
    // MPEG-4 only (ID=0), layer=0, AAC-LC profile=1, one raw_data_block.
    // Both protected and unprotected headers count toward the frame length.
    let header_length = if header[1] & 1 == 0 {
        9
    } else {
        ADTS_HEADER_BYTES as u64
    };
    if header[0] != 0xff
        || header[1] & 0xfe != 0xf0
        || header[2] >> 6 != 1
        || !matches!(rate_index, 3 | 4)
        || !matches!(channels, 1 | 2)
        || header[6] & 3 != 0
        || length <= header_length
    {
        return Err(MediaError::UnsupportedMedia);
    }
    Ok(AdtsFrame {
        bytes: length,
        rate_index,
        channels,
    })
}
fn probe_adts<R: Read + Seek>(
    reader: &mut R,
    prefix: &[u8; 12],
    len: u64,
    cancel: &ResolverCancellation,
) -> Result<LocalAudioFormat, MediaError> {
    let first = adts_frame(&prefix[..ADTS_HEADER_BYTES])?;
    let mut header = [0u8; ADTS_HEADER_BYTES];
    read_at(reader, first.bytes, &mut header, len, cancel)?;
    let second = adts_frame(&header)?;
    if first.rate_index != second.rate_index || first.channels != second.channels {
        return Err(MediaError::UnsupportedMedia);
    }
    let end = first
        .bytes
        .checked_add(second.bytes)
        .ok_or(MediaError::UnsupportedMedia)?;
    read_at(reader, end - 1, &mut [0u8; 1], len, cancel)?;
    Ok(LocalAudioFormat::Adts)
}

fn id3_audio_offset<R: Read + Seek>(
    reader: &mut R,
    prefix: &[u8; 12],
    len: u64,
    cancel: &ResolverCancellation,
) -> Result<u64, MediaError> {
    let flags = prefix[5];
    let allowed = match prefix[3] {
        2 if flags & 0x40 == 0 => 0xc0,
        3 => 0xe0,
        4 => 0xf0,
        _ => return Err(MediaError::UnsupportedMedia),
    };
    if prefix[4] == 0xff || flags & !allowed != 0 || prefix[6..10].iter().any(|b| b & 0x80 != 0) {
        return Err(MediaError::UnsupportedMedia);
    }
    let payload = prefix[6..10]
        .iter()
        .fold(0u64, |value, byte| (value << 7) | u64::from(*byte));
    let footer = prefix[3] == 4 && flags & 0x10 != 0;
    let footer_offset = ID3_HEADER_BYTES + payload;
    let end = footer_offset + if footer { ID3_HEADER_BYTES } else { 0 };
    if end > MAX_ID3_TAG_BYTES || end > len {
        return Err(MediaError::UnsupportedMedia);
    }
    if footer {
        let mut bytes = [0u8; 10];
        read_at(reader, footer_offset, &mut bytes, len, cancel)?;
        if &bytes[..3] != b"3DI" || bytes[3..] != prefix[3..10] {
            return Err(MediaError::UnsupportedMedia);
        }
    }
    Ok(end)
}

struct Frame {
    bytes: u32,
    rate: u32,
    channels: u8,
}
fn mp3_frame(header: [u8; 4]) -> Result<Frame, MediaError> {
    // MPEG-1 Layer III only in this batch. Reserved/free-format headers and
    // MPEG-2/2.5 are explicit unsupported cases, never guessed frame lengths.
    let bits = u32::from_be_bytes(header);
    let rate_index = ((bits >> 10) & 3) as usize;
    let bitrate_index = ((bits >> 12) & 15) as usize;
    if bits >> 21 != 0x7ff
        || (bits >> 19) & 3 != 3
        || (bits >> 17) & 3 != 1
        || rate_index == 3
        || matches!(bitrate_index, 0 | 15)
        || bits & 3 == 2
    {
        return Err(MediaError::UnsupportedMedia);
    }
    let rate = MPEG1_SAMPLE_RATES[rate_index];
    let bytes = 144_000 * MPEG1_LAYER3_KBPS[bitrate_index] / rate + ((bits >> 9) & 1);
    Ok(Frame {
        bytes,
        rate,
        channels: if (bits >> 6) & 3 == 3 { 1 } else { 2 },
    })
}

fn seek<R: Seek>(
    reader: &mut R,
    offset: u64,
    cancel: &ResolverCancellation,
) -> Result<(), MediaError> {
    check(cancel)?;
    let result = reader.seek(SeekFrom::Start(offset));
    check(cancel)?;
    if result.map_err(|_| MediaError::Unavailable)? != offset {
        return Err(MediaError::Unavailable);
    }
    Ok(())
}
fn read_at<R: Read + Seek>(
    reader: &mut R,
    offset: u64,
    output: &mut [u8],
    len: u64,
    cancel: &ResolverCancellation,
) -> Result<(), MediaError> {
    check(cancel)?;
    if offset
        .checked_add(output.len() as u64)
        .is_none_or(|end| end > len)
    {
        return Err(MediaError::UnsupportedMedia);
    }
    seek(reader, offset, cancel)?;
    let mut filled = 0;
    let mut interruptions = 0;
    while filled < output.len() {
        check(cancel)?;
        let result = reader.read(&mut output[filled..]);
        check(cancel)?;
        match result {
            Ok(0) => return Err(MediaError::UnsupportedMedia),
            Ok(count) if count <= output.len() - filled => filled += count,
            Ok(_) => return Err(MediaError::Unavailable),
            Err(error) if error.kind() == ErrorKind::Interrupted => {
                interruptions += 1;
                if interruptions == MAX_INTERRUPTED_READS {
                    return Err(MediaError::Unavailable);
                }
            }
            Err(error) if error.kind() == ErrorKind::UnexpectedEof => {
                return Err(MediaError::UnsupportedMedia);
            }
            Err(_) => return Err(MediaError::Unavailable),
        }
    }
    Ok(())
}
fn check(cancel: &ResolverCancellation) -> Result<(), MediaError> {
    if cancel.is_cancelled() {
        Err(MediaError::Cancelled)
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "local_audio_format_tests.rs"]
mod tests;

#[path = "local_mp4_format.rs"]
mod mp4;

#[cfg(test)]
pub(crate) use mp4::audio_probe_fixture as mp4_audio_probe_fixture;
pub(crate) use mp4::subtitle::discover_subtitle as discover_mp4_subtitle;
pub(crate) use mp4::subtitle::discover_subtitles as discover_mp4_subtitles;
pub(crate) use mp4::subtitle::read_subtitle as read_mp4_subtitle;

#[cfg(test)]
pub(crate) use mp4::subtitle::tests::fixture as mp4_subtitle_fixture;

#[cfg(test)]
pub(crate) use mp4::subtitle::tests::cropped_fixture as mp4_cropped_subtitle_fixture;
#[cfg(test)]
pub(crate) use mp4::subtitle::tests::cropped_v1_fixture as mp4_v1_subtitle_fixture;
#[cfg(test)]
pub(crate) use mp4::subtitle::tests::delayed_fixture as mp4_delayed_subtitle_fixture;
#[cfg(test)]
pub(crate) use mp4::subtitle::tests::multi_fixture as mp4_multi_subtitle_fixture;
#[cfg(test)]
pub(crate) use mp4::subtitle::tests::prepared_fixture as mp4_prepared_fixture;
