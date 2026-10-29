//! Narrow, self-contained AAC MP4 gate with optional ordinary H.264 video.
//! Parse bounded metadata; skip encoded audio by seeking the same pinned file.
use super::{MediaError, ResolverCancellation, check, read_at};
use std::io::{Read, Seek};

const MAX_TOP_BOXES: usize = 128;
const MAX_CHILD_BOXES: usize = 128;
const MAX_MOOV_BYTES: usize = 1024 * 1024;
const MAX_FTYP_BYTES: usize = 128;
// Native samples and sample tables have independent budgets. The latter bounds
// validation work even when all compressed samples use a constant byte length.
const MAX_SAMPLES: usize = 1_000_000;

fn require(condition: bool) -> Result<(), MediaError> {
    if condition {
        Ok(())
    } else {
        Err(MediaError::UnsupportedMedia)
    }
}
fn u32_at(data: &[u8], offset: usize) -> Result<u32, MediaError> {
    let bytes = data
        .get(offset..offset + 4)
        .ok_or(MediaError::UnsupportedMedia)?;
    Ok(u32::from_be_bytes(
        bytes.try_into().map_err(|_| MediaError::UnsupportedMedia)?,
    ))
}
fn u64_at(data: &[u8], offset: usize) -> Result<u64, MediaError> {
    let bytes = data
        .get(offset..offset + 8)
        .ok_or(MediaError::UnsupportedMedia)?;
    Ok(u64::from_be_bytes(
        bytes.try_into().map_err(|_| MediaError::UnsupportedMedia)?,
    ))
}

pub(super) fn probe<R: Read + Seek>(
    reader: &mut R,
    len: u64,
    cancel: &ResolverCancellation,
) -> Result<(), MediaError> {
    let container = load_container(reader, len, cancel)?;
    validate_moov(&container.moov, container.mdat, cancel)?;
    check(cancel)
}

struct Container {
    moov: Vec<u8>,
    mdat: (u64, u64),
}
fn load_container<R: Read + Seek>(
    reader: &mut R,
    len: u64,
    cancel: &ResolverCancellation,
) -> Result<Container, MediaError> {
    let mut position = 0u64;
    let mut count = 0;
    let mut moov = None;
    let mut mdat = None;
    while position < len {
        check(cancel)?;
        count += 1;
        require(count <= MAX_TOP_BOXES)?;
        let mut header = [0u8; 16];
        read_at(reader, position, &mut header[..8], len, cancel)?;
        let small_size = u32_at(&header, 0)?;
        let (size, header_len) = if small_size == 1 {
            read_at(reader, position + 8, &mut header[8..], len, cancel)?;
            (u64_at(&header, 8)?, 16u64)
        } else {
            (u64::from(small_size), 8u64)
        };
        let end = position
            .checked_add(size)
            .ok_or(MediaError::UnsupportedMedia)?;
        require(size >= header_len && end <= len)?; // size=0/open-ended is unsupported.
        let body_start = position + header_len;
        let body_len = size - header_len;
        if position == 0 {
            require(&header[4..8] == b"ftyp")?;
        }
        match &header[4..8] {
            b"ftyp" => {
                require(position == 0 && body_len <= MAX_FTYP_BYTES as u64)?;
                let mut bytes = [0u8; MAX_FTYP_BYTES];
                let body = &mut bytes[..body_len as usize];
                read_at(reader, body_start, body, len, cancel)?;
                require(body.len() >= 12 && body.len().is_multiple_of(4))?;
                let brand = |b: &[u8]| matches!(b, b"isom" | b"mp41" | b"mp42" | b"M4A ");
                require(
                    brand(&body[..4]) && body[8..].as_chunks::<4>().0.iter().any(|b| brand(b)),
                )?;
            }
            b"moov" => {
                require(moov.is_none() && body_len > 0 && body_len <= MAX_MOOV_BYTES as u64)?;
                let mut body = Vec::new();
                body.try_reserve_exact(body_len as usize)
                    .map_err(|_| MediaError::Overloaded)?;
                body.resize(body_len as usize, 0);
                // Read in small pieces to keep cancellation responsive on local I/O.
                for (index, bytes) in body.chunks_mut(8192).enumerate() {
                    read_at(
                        reader,
                        body_start + (index * 8192) as u64,
                        bytes,
                        len,
                        cancel,
                    )?;
                }
                moov = Some(body);
            }
            b"mdat" => {
                require(mdat.is_none() && body_len > 0)?;
                read_at(reader, end - 1, &mut [0], len, cancel)?;
                mdat = Some((body_start, end));
            }
            b"free" | b"skip" => {}
            // UUID metadata is intentionally not accepted until its exact
            // native producer form is checked below.
            b"uuid" => validate_uuid(reader, body_start, body_len, len, cancel)?,
            _ => return Err(MediaError::UnsupportedMedia),
        }
        position = end;
    }
    check(cancel)?;
    Ok(Container {
        moov: moov.ok_or(MediaError::UnsupportedMedia)?,
        mdat: mdat.ok_or(MediaError::UnsupportedMedia)?,
    })
}

fn validate_uuid<R: Read + Seek>(
    reader: &mut R,
    start: u64,
    size: u64,
    len: u64,
    cancel: &ResolverCancellation,
) -> Result<(), MediaError> {
    // Windows sink version marker observed in genuine native fixtures. Accept
    // only this identifier plus a bounded dotted numeric version, not arbitrary
    // UUID extensions or opaque payload that a native plugin might interpret.
    const WRITER_VERSION_UUID: [u8; 16] = [
        92, 167, 8, 251, 50, 142, 66, 5, 168, 97, 101, 14, 202, 10, 149, 150,
    ];
    require((21..=64).contains(&size))?;
    let mut bytes = [0u8; 64];
    let body = &mut bytes[..size as usize];
    read_at(reader, start, body, len, cancel)?;
    require(
        body[..16] == WRITER_VERSION_UUID
            && u32_at(body, 16)? as usize == body.len() - 20
            && body[20..].iter().all(|b| b.is_ascii_digit() || *b == b'.'),
    )
}

struct Atom<'a> {
    kind: &'a [u8],
    body: &'a [u8],
}
fn children<'a>(
    data: &'a [u8],
    allowed: &[&[u8]],
    cancel: &ResolverCancellation,
) -> Result<Vec<Atom<'a>>, MediaError> {
    children_with_tracks(data, allowed, false, cancel)
}
fn children_with_tracks<'a>(
    data: &'a [u8],
    allowed: &[&[u8]],
    repeat_tracks: bool,
    cancel: &ResolverCancellation,
) -> Result<Vec<Atom<'a>>, MediaError> {
    let mut result: Vec<Atom<'a>> = Vec::new();
    let mut position = 0;
    while position < data.len() {
        check(cancel)?;
        require(result.len() < MAX_CHILD_BOXES)?;
        let size = u32_at(data, position)? as usize;
        require(size >= 8)?; // Nested extended/open-ended boxes intentionally unsupported.
        let end = position
            .checked_add(size)
            .ok_or(MediaError::UnsupportedMedia)?;
        let atom = data
            .get(position..end)
            .ok_or(MediaError::UnsupportedMedia)?;
        let kind = &atom[4..8];
        require(
            allowed.contains(&kind)
                && ((repeat_tracks && kind == b"trak")
                    || !result.iter().any(|old| old.kind == kind)),
        )?;
        result.push(Atom {
            kind,
            body: &atom[8..],
        });
        position = end;
    }
    Ok(result)
}
fn get<'a>(atoms: &[Atom<'a>], kind: &[u8]) -> Result<&'a [u8], MediaError> {
    atoms
        .iter()
        .find(|atom| atom.kind == kind)
        .map(|atom| atom.body)
        .ok_or(MediaError::UnsupportedMedia)
}
fn full_zero(data: &[u8]) -> Result<(), MediaError> {
    require(u32_at(data, 0)? == 0)
}

fn validate_moov(
    data: &[u8],
    mdat: (u64, u64),
    cancel: &ResolverCancellation,
) -> Result<(), MediaError> {
    let moov = children_with_tracks(data, &[b"mvhd", b"trak", b"udta"], true, cancel)?;
    let mvhd = get(&moov, b"mvhd")?;
    full_zero(mvhd)?;
    require(mvhd.len() == 100 && u32_at(mvhd, 12)? > 0)?;
    // Optional metadata is validated separately and never used as timing,
    // subtitle, recognition or translation input.
    if let Some(udta) = moov.iter().find(|atom| atom.kind == b"udta") {
        validate_udta(udta.body, cancel)?;
    }
    let tracks = moov
        .iter()
        .filter(|atom| atom.kind == b"trak")
        .collect::<Vec<_>>();
    require((1..=2).contains(&tracks.len()))?;
    let mut audio = None;
    let mut video = None;
    let mut previous_id = None;
    for track in tracks {
        let (id, is_audio, chunks) = validate_track(track.body, mdat, cancel)?;
        require(previous_id != Some(id))?;
        previous_id = Some(id);
        let slot = if is_audio { &mut audio } else { &mut video };
        require(slot.is_none())?;
        *slot = Some(chunks);
    }
    let mut audio = audio.ok_or(MediaError::UnsupportedMedia)?;
    if let Some(mut video) = video {
        // Both sorted track tables were checked against mdat. Merge borrowed
        // cursors to detect aliases without allocating one range per sample.
        let mut a = audio.next(cancel)?;
        let mut v = video.next(cancel)?;
        while let (Some(ar), Some(vr)) = (a, v) {
            require(ar.1 <= vr.0 || vr.1 <= ar.0)?;
            if ar.0 < vr.0 {
                a = audio.next(cancel)?;
            } else {
                v = video.next(cancel)?;
            }
        }
    }
    Ok(())
}

fn validate_track<'a>(
    data: &'a [u8],
    mdat: (u64, u64),
    cancel: &ResolverCancellation,
) -> Result<(u32, bool, ChunkCursor<'a>), MediaError> {
    let trak = children(data, &[b"tkhd", b"mdia"], cancel)?;
    let tkhd = get(&trak, b"tkhd")?;
    require(tkhd.len() == 84 && tkhd[0] == 0 && u32_at(tkhd, 12)? != 0)?;
    let mdia = children(get(&trak, b"mdia")?, &[b"mdhd", b"hdlr", b"minf"], cancel)?;
    let mdhd = get(&mdia, b"mdhd")?;
    full_zero(mdhd)?;
    require(mdhd.len() == 24 && u32_at(mdhd, 12)? > 0)?;
    let hdlr = get(&mdia, b"hdlr")?;
    full_zero(hdlr)?;
    require(hdlr.len() >= 24 && matches!(&hdlr[8..12], b"soun" | b"vide"))?;
    let audio = &hdlr[8..12] == b"soun";
    let minf = children(
        get(&mdia, b"minf")?,
        if audio {
            &[b"smhd", b"dinf", b"stbl"]
        } else {
            &[b"vmhd", b"dinf", b"stbl"]
        },
        cancel,
    )?;
    if audio {
        let smhd = get(&minf, b"smhd")?;
        full_zero(smhd)?;
        require(smhd.len() == 8)?;
    } else {
        let vmhd = get(&minf, b"vmhd")?;
        require(vmhd.len() == 12 && u32_at(vmhd, 0)? == 1 && vmhd[4..] == [0; 8])?;
    }
    let dinf = children(get(&minf, b"dinf")?, &[b"dref"], cancel)?;
    let dref = get(&dinf, b"dref")?;
    full_zero(dref)?;
    require(u32_at(dref, 4)? == 1)?;
    let refs = children(
        dref.get(8..).ok_or(MediaError::UnsupportedMedia)?,
        &[b"url "],
        cancel,
    )?;
    require(get(&refs, b"url ")? == [0, 0, 0, 1])?;
    let stbl = children(
        get(&minf, b"stbl")?,
        if audio {
            &[
                b"stsd", b"stts", b"stsc", b"stsz", b"stco", b"co64", b"ctts",
            ]
        } else {
            &[
                b"stsd", b"stts", b"stsc", b"stsz", b"stco", b"co64", b"stss", b"ctts",
            ]
        },
        cancel,
    )?;
    if audio {
        validate_stsd(get(&stbl, b"stsd")?, cancel)?;
    } else {
        video::validate_stsd(get(&stbl, b"stsd")?, cancel)?;
    }
    let cursor = validate_tables(&stbl, mdat, !audio, cancel)?;
    if let Some(sync) = stbl.iter().find(|atom| atom.kind == b"stss") {
        let mut previous = 0;
        for index in 0..table(sync.body, 4)? {
            check(cancel)?;
            let sample = u32_at(sync.body, 8 + index * 4)? as usize;
            require(sample > previous && sample <= cursor.samples)?;
            previous = sample;
        }
    }
    Ok((u32_at(tkhd, 12)?, audio, cursor))
}
fn validate_udta(data: &[u8], cancel: &ResolverCancellation) -> Result<(), MediaError> {
    // A narrow mdir/ilst subset, without keys/iloc/references or freeform data.
    let atoms = children(data, &[b"meta", b"Xtra"], cancel)?;
    if let Some(xtra) = atoms.iter().find(|a| a.kind == b"Xtra") {
        require(xtra.body.is_empty())?;
    }
    if let Some(meta) = atoms.iter().find(|a| a.kind == b"meta") {
        full_zero(meta.body)?;
        let atoms = children(&meta.body[4..], &[b"hdlr", b"ilst"], cancel)?;
        let handler = get(&atoms, b"hdlr")?;
        full_zero(handler)?;
        require(handler.len() >= 24 && &handler[8..12] == b"mdir")?;
        metadata::validate(get(&atoms, b"ilst")?, cancel)?;
    }
    Ok(())
}

fn validate_stsd(data: &[u8], cancel: &ResolverCancellation) -> Result<(), MediaError> {
    full_zero(data)?;
    require(u32_at(data, 4)? == 1)?;
    let entries = children(
        data.get(8..).ok_or(MediaError::UnsupportedMedia)?,
        &[b"mp4a"],
        cancel,
    )?;
    let audio = get(&entries, b"mp4a")?;
    require(audio.len() >= 28)?;
    require(audio[..6] == [0; 6] && audio[6..8] == [0, 1] && audio[8..16] == [0; 8])?;
    let channels = u16::from_be_bytes([audio[16], audio[17]]);
    require(matches!(channels, 1 | 2) && audio[18..24] == [0, 16, 0, 0, 0, 0])?;
    let rate_fixed = u32_at(audio, 24)?;
    require(matches!(rate_fixed, x if x == 44_100 << 16 || x == 48_000 << 16))?;
    let extensions = children(&audio[28..], &[b"esds", b"btrt"], cancel)?;
    if let Some(btrt) = extensions.iter().find(|a| a.kind == b"btrt") {
        require(btrt.body.len() == 12)?;
    }
    let esds = get(&extensions, b"esds")?;
    full_zero(esds)?;
    let (es, tail) = descriptor(&esds[4..], 3)?;
    require(tail.is_empty() && es.len() >= 3 && es[2] == 0)?; // no ES URL/dependency/OCR.
    let (decoder, tail) = descriptor(&es[3..], 4)?;
    require(decoder.len() >= 13 && decoder[0] == 0x40 && decoder[1] == 0x15)?;
    let (asc, extra) = descriptor(&decoder[13..], 5)?;
    require(extra.is_empty() && asc.len() == 2)?;
    let rate_index = if rate_fixed >> 16 == 48_000 { 3 } else { 4 };
    let expected = (2u16 << 11) | (rate_index << 7) | (channels << 3);
    require(asc == expected.to_be_bytes())?; // LC, 1024 frames, no SBR/PS/PCE/extension.
    let (sl, tail) = descriptor(tail, 6)?;
    require(sl == [2] && tail.is_empty())
}
fn descriptor(data: &[u8], tag: u8) -> Result<(&[u8], &[u8]), MediaError> {
    require(data.first() == Some(&tag))?;
    let mut length = 0usize;
    for index in 1..=4 {
        let byte = *data.get(index).ok_or(MediaError::UnsupportedMedia)?;
        length = (length << 7) | usize::from(byte & 0x7f);
        if byte & 0x80 == 0 {
            let end = (index + 1)
                .checked_add(length)
                .ok_or(MediaError::UnsupportedMedia)?;
            return Ok((
                data.get(index + 1..end)
                    .ok_or(MediaError::UnsupportedMedia)?,
                data.get(end..).ok_or(MediaError::UnsupportedMedia)?,
            ));
        }
    }
    Err(MediaError::UnsupportedMedia)
}

fn table(data: &[u8], width: usize) -> Result<usize, MediaError> {
    full_zero(data)?;
    table_rows(data, width)
}
fn table_rows(data: &[u8], width: usize) -> Result<usize, MediaError> {
    let count = u32_at(data, 4)? as usize;
    require(count > 0 && count <= MAX_SAMPLES && data.len() == 8 + count * width)?;
    Ok(count)
}
fn validate_tables<'a>(
    atoms: &[Atom<'a>],
    mdat: (u64, u64),
    video: bool,
    cancel: &ResolverCancellation,
) -> Result<ChunkCursor<'a>, MediaError> {
    let stsz = get(atoms, b"stsz")?;
    full_zero(stsz)?;
    let constant = u32_at(stsz, 4)?;
    let samples = u32_at(stsz, 8)? as usize;
    require(
        samples > 0
            && samples <= MAX_SAMPLES
            && stsz.len() == 12 + if constant == 0 { samples * 4 } else { 0 },
    )?;
    let stts = get(atoms, b"stts")?;
    let mut timed_samples = 0usize;
    for index in 0..table(stts, 8)? {
        check(cancel)?;
        let count = u32_at(stts, 8 + index * 8)? as usize;
        require(count > 0 && u32_at(stts, 12 + index * 8)? > 0)?;
        timed_samples = timed_samples
            .checked_add(count)
            .ok_or(MediaError::UnsupportedMedia)?;
    }
    require(timed_samples == samples)?;
    if let Some(composition) = atoms.iter().find(|a| a.kind == b"ctts") {
        // Video permits signed v1 offsets as well as unsigned v0. Audio stays
        // at v0. Neither table substitutes for native audio timestamps.
        let version_flags = u32_at(composition.body, 0)?;
        require(version_flags == 0 || (video && version_flags == 0x0100_0000))?;
        let rows = table_rows(composition.body, 8)?;
        let mut composed_samples = 0usize;
        for index in 0..rows {
            check(cancel)?;
            let count = u32_at(composition.body, 8 + index * 8)? as usize;
            require(count > 0)?;
            composed_samples = composed_samples
                .checked_add(count)
                .ok_or(MediaError::UnsupportedMedia)?;
        }
        require(composed_samples == samples)?;
    }
    let offsets = atoms
        .iter()
        .filter(|a| matches!(a.kind, b"stco" | b"co64"))
        .collect::<Vec<_>>();
    require(offsets.len() == 1)?;
    let width = if offsets[0].kind == b"stco" { 4 } else { 8 };
    let chunks = table(offsets[0].body, width)?;
    let stsc = get(atoms, b"stsc")?;
    let runs = table(stsc, 12)?;
    let mut previous = 0;
    for index in 0..runs {
        check(cancel)?;
        let first = u32_at(stsc, 8 + index * 12)? as usize;
        require(
            first > previous
                && first <= chunks
                && (index != 0 || first == 1)
                && u32_at(stsc, 12 + index * 12)? > 0
                && u32_at(stsc, 16 + index * 12)? == 1,
        )?;
        previous = first;
    }
    let cursor = ChunkCursor {
        stsz,
        constant,
        samples,
        offsets: offsets[0].body,
        width,
        chunks,
        stsc,
        runs,
        sample_index: 0,
        run: 0,
        chunk: 0,
    };
    let mut validated = cursor.clone();
    let mut previous_end = mdat.0;
    while let Some((start, end)) = validated.next(cancel)? {
        require(start >= previous_end && end <= mdat.1)?;
        previous_end = end;
    }
    require(validated.sample_index == samples)?;
    Ok(cursor)
}

#[derive(Clone)]
struct ChunkCursor<'a> {
    stsz: &'a [u8],
    constant: u32,
    samples: usize,
    offsets: &'a [u8],
    width: usize,
    chunks: usize,
    stsc: &'a [u8],
    runs: usize,
    sample_index: usize,
    run: usize,
    chunk: usize,
}
impl ChunkCursor<'_> {
    fn next(&mut self, cancel: &ResolverCancellation) -> Result<Option<(u64, u64)>, MediaError> {
        check(cancel)?;
        if self.chunk == self.chunks {
            return Ok(None);
        }
        if self.run + 1 < self.runs
            && self.chunk + 1 == u32_at(self.stsc, 8 + (self.run + 1) * 12)? as usize
        {
            self.run += 1;
        }
        let count = u32_at(self.stsc, 12 + self.run * 12)? as usize;
        require(count <= self.samples - self.sample_index)?;
        let start = if self.width == 4 {
            u64::from(u32_at(self.offsets, 8 + self.chunk * self.width)?)
        } else {
            u64_at(self.offsets, 8 + self.chunk * self.width)?
        };
        let mut end = start;
        for _ in 0..count {
            check(cancel)?;
            let size = if self.constant != 0 {
                self.constant
            } else {
                u32_at(self.stsz, 12 + self.sample_index * 4)?
            };
            require(size > 0)?;
            end = end
                .checked_add(u64::from(size))
                .ok_or(MediaError::UnsupportedMedia)?;
            self.sample_index += 1;
        }
        self.chunk += 1;
        Ok(Some((start, end)))
    }
}

#[path = "local_mp4_video.rs"]
mod video;

#[cfg(test)]
#[path = "local_mp4_format_tests.rs"]
mod tests;

#[cfg(test)]
pub(crate) fn audio_probe_fixture() -> Vec<u8> {
    tests::fixture()
}

#[path = "local_mp4_metadata.rs"]
mod metadata;

#[path = "local_mp4_subtitle.rs"]
pub(super) mod subtitle;
