//! Conservative RIFF subset: one canonical PCM16 fmt, one data, bounded JUNK.
use super::*;
fn le32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}
fn le16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}
pub(super) async fn parse(
    media: &mut RemoteHttpMedia,
    cancel: &ResolverCancellation,
) -> Result<Layout, RemoteWaveError> {
    {
        let range = read(media, 0, 12).await?;
        let header = range.bytes()?;
        if &header[..4] != b"RIFF" || &header[8..] != b"WAVE" {
            return Err(MediaError::UnsupportedMedia.into());
        }
        if u64::from(le32(header, 4)) + 8 != media.byte_len() {
            return Err(MediaError::InvalidAudio.into());
        }
    }
    let mut position = 12u64;
    let mut count = 0;
    let mut junk = 0u64;
    let mut format = None;
    let mut data = None;
    while position < media.byte_len() {
        check(cancel)?;
        count += 1;
        if count > MAX_WAVE_CHUNKS {
            return Err(MediaError::TooLarge.into());
        }
        if media.byte_len() - position < 8 {
            return Err(MediaError::InvalidAudio.into());
        }
        let (kind, length) = {
            let range = read(media, position, 8).await?;
            let chunk = range.bytes()?;
            (
                [chunk[0], chunk[1], chunk[2], chunk[3]],
                u64::from(le32(chunk, 4)),
            )
        };
        let start = position + 8;
        let end = start
            .checked_add(length)
            .and_then(|v| v.checked_add(length % 2))
            .filter(|v| *v <= media.byte_len())
            .ok_or(MediaError::InvalidAudio)?;
        match &kind {
            b"fmt " => {
                if format.is_some() || data.is_some() {
                    return Err(MediaError::InvalidAudio.into());
                }
                if length != 16 {
                    return Err(MediaError::UnsupportedMedia.into());
                }
                let fmt = read(media, start, 16).await?;
                let fmt = fmt.bytes()?;
                if le16(fmt, 0) != 1 || le16(fmt, 14) != 16 {
                    return Err(MediaError::UnsupportedMedia.into());
                }
                // Canonical PCM has no channel mask. More than two channels
                // requires WAVEFORMATEXTENSIBLE, which this subset rejects.
                if !matches!(le16(fmt, 2), 1 | 2) {
                    return Err(MediaError::UnsupportedMedia.into());
                }
                let layout = DecodedAudioFormat::new(le32(fmt, 4), le16(fmt, 2))?;
                let align = layout
                    .channels()
                    .checked_mul(2)
                    .ok_or(MediaError::InvalidAudio)?;
                if le16(fmt, 12) != align
                    || Some(le32(fmt, 8)) != layout.sample_rate().checked_mul(u32::from(align))
                {
                    return Err(MediaError::InvalidAudio.into());
                }
                format = Some(layout);
            }
            b"data" => {
                if data.is_some() || format.is_none() || length == 0 {
                    return Err(MediaError::InvalidAudio.into());
                }
                data = Some((start, length));
            }
            b"JUNK" => {
                junk = junk
                    .checked_add(length)
                    .filter(|v| *v <= MAX_WAVE_JUNK_BYTES)
                    .ok_or(MediaError::TooLarge)?;
                if length % 2 != 0 && read(media, start + length, 1).await?.bytes()? != [0] {
                    return Err(MediaError::UnsupportedMedia.into());
                }
            }
            _ => return Err(MediaError::UnsupportedMedia.into()),
        }
        position = end;
    }
    let format = format.ok_or(MediaError::InvalidAudio)?;
    let (start, bytes) = data.ok_or(MediaError::InvalidAudio)?;
    let align = usize::from(format.channels()) * 2;
    let frames = bytes / align as u64;
    if !bytes.is_multiple_of(align as u64) || frames == 0 {
        return Err(MediaError::InvalidAudio.into());
    }
    if frames
        .checked_mul(US_PER_SECOND)
        .ok_or(MediaError::TooLarge)?
        / u64::from(format.sample_rate())
        > MAX_DECODED_MEDIA_DURATION_US
    {
        return Err(MediaError::TooLarge.into());
    }
    check(cancel)?;
    Ok(Layout {
        format,
        start,
        bytes,
        align,
    })
}
