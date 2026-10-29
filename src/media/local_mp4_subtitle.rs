//! Selected, self-contained plain tx3g track. Audio decoding keeps its own gate.
use super::*;
use crate::{
    ai::speech::StreamIdentity,
    media::{
        decode_worker::MAX_DECODED_MEDIA_DURATION_US,
        embedded_mp4::{
            MAX_MP4_SUBTITLE_TRACKS, Mp4SubtitleLanguage, Mp4SubtitleSelection, Mp4SubtitleTrack,
            Mp4SubtitleTracks,
        },
        embedded_tx3g::{
            MAX_TX3G_SAMPLE_BYTES, MAX_TX3G_SAMPLES, MAX_TX3G_TRACK_BYTES, Tx3gSampleStamp,
            Tx3gTrack,
        },
    },
    resolver::contracts::{ExistingText, MAX_TEXT_CUES, TextLanguage},
};

// At most eight text choices, one audio and one optional video track.
const MAX_EMBEDDED_TRACKS: usize = MAX_MP4_SUBTITLE_TRACKS + 2;
const MAX_TX3G_FONTS: usize = 32;
const TX3G_ENTRY_HEADER_BYTES: usize = 38;
const MICROS_PER_SECOND: u128 = 1_000_000;

#[path = "local_mp4_text_edit.rs"]
mod edit;
use edit::TextEdit;

struct TextTrack<'a> {
    chunks: ChunkCursor<'a>,
    stts: &'a [u8],
    duration: u64,
    edit: TextEdit,
}

pub(crate) fn read_subtitle<R: Read + Seek>(
    reader: &mut R,
    len: u64,
    identity: StreamIdentity,
    selection: Mp4SubtitleSelection,
    cancel: &ResolverCancellation,
) -> Result<ExistingText, MediaError> {
    // Validate all track ranges before reading complete text samples. The
    // shared top-level loader first probes the last mdat byte for readable EOF.
    let container = load_container(reader, len, cancel)?;
    let (_, text) = selected_track(&container, Some(selection), cancel)?;
    let text = text.ok_or(MediaError::UnsupportedMedia)?;
    let mut builder = Tx3gTrack::new(
        identity,
        selection.track_id(),
        text.edit.media_timescale(),
        selection.language(),
    )?;
    let mut chunks = text.chunks;
    let mut timing = Timing {
        table: text.stts,
        run: 0,
        left: 0,
        delta: 0,
        ticks: 0,
    };
    let mut sample = [0u8; MAX_TX3G_SAMPLE_BYTES];
    // Only bounded numeric mappings, one per original nonempty cue. The sole
    // text owner remains the existing builder (including cropped-out text).
    let mut presentation = Vec::new();
    if text.edit.is_edited() {
        presentation
            .try_reserve_exact(MAX_TEXT_CUES)
            .map_err(|_| MediaError::Overloaded)?;
    }
    while chunks.chunk < chunks.chunks {
        let first = chunks.sample_index;
        let (mut position, end) = chunks.next(cancel)?.ok_or(MediaError::UnsupportedMedia)?;
        for index in first..chunks.sample_index {
            check(cancel)?;
            let size = sample_size(&chunks, index)?;
            read_at(reader, position, &mut sample[..size], len, cancel)?;
            let (start, duration) = timing.next(cancel)?;
            builder.push(
                Tx3gSampleStamp {
                    identity,
                    sequence: index as u64 + 1,
                    start_ticks: start,
                    duration_ticks: duration,
                },
                &sample[..size],
                cancel,
            )?;
            if text.edit.is_edited() && sample[..2] != [0, 0] {
                if presentation.len() == MAX_TEXT_CUES {
                    return Err(MediaError::TooLarge);
                }
                presentation.push(text.edit.sample_time(start, duration)?);
            }
            position += size as u64; // Already validated within this chunk/mdat.
        }
        require(position == end)?;
    }
    require(
        timing.ticks == text.duration && timing.left == 0 && timing.run == table(text.stts, 8)?,
    )?;
    let mut document = builder.finish(cancel)?;
    if text.edit.is_edited() {
        if presentation.len() != document.cues.len() {
            return Err(MediaError::InvalidSubtitle);
        }
        let mut index = 0;
        document.cues.retain_mut(|cue| {
            let mapped = presentation[index];
            index += 1;
            if cancel.is_cancelled() {
                return false;
            }
            if let Some((start, duration)) = mapped {
                cue.start_us = Some(start);
                cue.duration_us = Some(duration);
                true
            } else {
                false
            }
        });
        check(cancel)?;
        document
            .validate_document(identity, document.source, &document.content_id)
            .map_err(|_| MediaError::InvalidSubtitle)?;
    }
    check(cancel)?;
    Ok(document)
}

pub(crate) fn discover_subtitle<R: Read + Seek>(
    reader: &mut R,
    len: u64,
    cancel: &ResolverCancellation,
) -> Result<Mp4SubtitleTrack, MediaError> {
    discover_subtitles(reader, len, cancel)?.single()
}
pub(crate) fn discover_subtitles<R: Read + Seek>(
    reader: &mut R,
    len: u64,
    cancel: &ResolverCancellation,
) -> Result<Mp4SubtitleTracks, MediaError> {
    let container = load_container(reader, len, cancel)?;
    let (info, _) = selected_track(&container, None, cancel)?;
    check(cancel)?;
    Ok(info)
}

fn selected_track<'a>(
    container: &'a Container,
    selection: Option<Mp4SubtitleSelection>,
    cancel: &ResolverCancellation,
) -> Result<(Mp4SubtitleTracks, Option<TextTrack<'a>>), MediaError> {
    let movie = children_with_tracks(&container.moov, &[b"mvhd", b"trak", b"udta"], true, cancel)?;
    let mvhd = get(&movie, b"mvhd")?;
    full_zero(mvhd)?;
    require(mvhd.len() == 100 && u32_at(mvhd, 12)? > 0)?;
    if let Some(udta) = movie.iter().find(|atom| atom.kind == b"udta") {
        validate_udta(udta.body, cancel)?;
    }
    let tracks: Vec<_> = movie.iter().filter(|atom| atom.kind == b"trak").collect();
    if tracks.len() > MAX_EMBEDDED_TRACKS {
        return Err(MediaError::TooLarge);
    }
    require(tracks.len() >= 2)?;
    let mut ranges = Vec::with_capacity(MAX_EMBEDDED_TRACKS);
    let mut ids = Vec::with_capacity(MAX_EMBEDDED_TRACKS);
    let mut audio = false;
    let mut video = false;
    let mut text = None;
    let mut info = Mp4SubtitleTracks::default();
    for track in tracks {
        // Only the text branch accepts a bounded edit. Audio/video go through
        // their unchanged gate, which still rejects edts and references.
        let atoms = children(track.body, &[b"tkhd", b"mdia", b"edts"], cancel)?;
        let tkhd = get(&atoms, b"tkhd")?;
        require(tkhd.len() == 84 && tkhd[0] == 0)?;
        let id = u32_at(tkhd, 12)?;
        require(id != 0 && !ids.contains(&id))?;
        ids.push(id);
        let mdia = children(get(&atoms, b"mdia")?, &[b"mdhd", b"hdlr", b"minf"], cancel)?;
        let handler = get(&mdia, b"hdlr")?;
        full_zero(handler)?;
        require(handler.len() >= 24)?;
        match &handler[8..12] {
            b"text" => {
                let mdhd = get(&mdia, b"mdhd")?;
                let language =
                    language_hint(mdhd.get(20..22).ok_or(MediaError::UnsupportedMedia)?)?;
                info.push(Mp4SubtitleTrack {
                    track_id: id,
                    language,
                })?;
                let chosen = selection.filter(|s| id == s.track_id());
                let selected = validate_text_track(
                    &atoms,
                    &mdia,
                    mvhd,
                    container.mdat,
                    chosen.map(Mp4SubtitleSelection::language),
                    cancel,
                )?;
                ranges.push(selected.chunks.clone());
                if chosen.is_some() {
                    text = Some(selected);
                }
            }
            b"soun" | b"vide" => {
                let (id, is_audio, chunks) = validate_track(track.body, container.mdat, cancel)?;
                require(selection.is_none_or(|s| id != s.track_id()))?;
                let slot = if is_audio { &mut audio } else { &mut video };
                require(!*slot)?;
                *slot = true;
                ranges.push(chunks);
            }
            _ => return Err(MediaError::UnsupportedMedia),
        }
    }
    require(audio)?;
    if info.is_empty() {
        return Err(MediaError::UnsupportedSubtitle);
    }
    require(selection.is_none() || text.is_some())?;
    for i in 0..ranges.len() {
        for j in i + 1..ranges.len() {
            non_overlapping(ranges[i].clone(), ranges[j].clone(), cancel)?;
        }
    }
    Ok((info, text))
}
fn non_overlapping(
    mut a: ChunkCursor<'_>,
    mut b: ChunkCursor<'_>,
    cancel: &ResolverCancellation,
) -> Result<(), MediaError> {
    let mut ar = a.next(cancel)?;
    let mut br = b.next(cancel)?;
    while let (Some(first), Some(second)) = (ar, br) {
        require(first.1 <= second.0 || second.1 <= first.0)?;
        if first.0 < second.0 {
            ar = a.next(cancel)?;
        } else {
            br = b.next(cancel)?;
        }
    }
    Ok(())
}
fn validate_text_track<'a>(
    atoms: &[Atom<'a>],
    mdia: &[Atom<'a>],
    mvhd: &[u8],
    mdat: (u64, u64),
    language: Option<TextLanguage>,
    cancel: &ResolverCancellation,
) -> Result<TextTrack<'a>, MediaError> {
    let tkhd = get(atoms, b"tkhd")?;
    require(u32_at(tkhd, 0)? & !7 == 0)?;
    let mdhd = get(mdia, b"mdhd")?;
    full_zero(mdhd)?;
    require(mdhd.len() == 24 && u32_at(mdhd, 12)? > 0 && mdhd[22..] == [0; 2])?;
    if let Some(language) = language {
        validate_language(&mdhd[20..22], language)?;
    }
    let timescale = u32_at(mdhd, 12)?;
    let duration = u64::from(u32_at(mdhd, 16)?);
    require(duration > 0)?;
    let micros = u128::from(duration) * MICROS_PER_SECOND / u128::from(timescale);
    require(micros <= u128::from(MAX_DECODED_MEDIA_DURATION_US))?;
    let edit = TextEdit::parse(
        atoms
            .iter()
            .find(|atom| atom.kind == b"edts")
            .map(|atom| atom.body),
        u32_at(mvhd, 12)?,
        timescale,
        duration,
        u64::from(u32_at(tkhd, 20)?),
        u64::from(u32_at(mvhd, 16)?),
        cancel,
    )?;
    let minf = children(get(mdia, b"minf")?, &[b"nmhd", b"dinf", b"stbl"], cancel)?;
    require(get(&minf, b"nmhd")? == [0; 4])?;
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
        &[b"stsd", b"stts", b"stsc", b"stsz", b"stco", b"co64"],
        cancel,
    )?;
    validate_description(get(&stbl, b"stsd")?, cancel)?;
    let sizes = get(&stbl, b"stsz")?;
    if u32_at(sizes, 8)? as usize > MAX_TX3G_SAMPLES {
        return Err(MediaError::TooLarge);
    }
    let chunks = validate_tables(&stbl, mdat, false, cancel)?;
    let mut total_bytes = 0usize;
    for index in 0..chunks.samples {
        check(cancel)?;
        let size = sample_size(&chunks, index)?;
        if size < 2 {
            return Err(MediaError::InvalidSubtitle);
        }
        if size > MAX_TX3G_SAMPLE_BYTES {
            return Err(MediaError::TooLarge);
        }
        total_bytes = total_bytes
            .checked_add(size)
            .filter(|bytes| *bytes <= MAX_TX3G_TRACK_BYTES)
            .ok_or(MediaError::TooLarge)?;
    }
    let stts = get(&stbl, b"stts")?;
    let mut ticks = 0u64;
    for index in 0..table(stts, 8)? {
        check(cancel)?;
        ticks = ticks
            .checked_add(
                u64::from(u32_at(stts, 8 + index * 8)?) * u64::from(u32_at(stts, 12 + index * 8)?),
            )
            .ok_or(MediaError::UnsupportedMedia)?;
    }
    require(ticks == duration)?;
    Ok(TextTrack {
        chunks,
        stts,
        duration,
        edit,
    })
}
fn sample_size(cursor: &ChunkCursor<'_>, index: usize) -> Result<usize, MediaError> {
    Ok(if cursor.constant != 0 {
        cursor.constant as usize
    } else {
        u32_at(cursor.stsz, 12 + index * 4)? as usize
    })
}
fn validate_language(bytes: &[u8], selected: TextLanguage) -> Result<(), MediaError> {
    let actual = language_hint(bytes)?;
    let expected = match selected {
        TextLanguage::Source(language) => Mp4SubtitleLanguage::Source(language),
        _ => Mp4SubtitleLanguage::Chinese,
    };
    require(actual == expected || actual == Mp4SubtitleLanguage::Undetermined)
}
fn language_hint(bytes: &[u8]) -> Result<Mp4SubtitleLanguage, MediaError> {
    let packed = u16::from_be_bytes([bytes[0], bytes[1]]);
    require(packed & 0x8000 == 0)?;
    let letters = [(packed >> 10) & 31, (packed >> 5) & 31, packed & 31];
    require(letters.iter().all(|letter| (1..=26).contains(letter)))?;
    let code = letters.map(|letter| letter as u8 + 0x60);
    use crate::ai::speech::SpeechLanguage::*;
    Ok(match &code {
        b"zho" => Mp4SubtitleLanguage::Chinese,
        b"und" => Mp4SubtitleLanguage::Undetermined,
        b"eng" => Mp4SubtitleLanguage::Source(English),
        b"jpn" => Mp4SubtitleLanguage::Source(Japanese),
        b"fra" => Mp4SubtitleLanguage::Source(French),
        b"deu" => Mp4SubtitleLanguage::Source(German),
        b"kor" => Mp4SubtitleLanguage::Source(Korean),
        _ => return Err(MediaError::UnsupportedMedia),
    })
}
fn validate_description(data: &[u8], cancel: &ResolverCancellation) -> Result<(), MediaError> {
    full_zero(data)?;
    require(u32_at(data, 4)? == 1)?;
    let entries = children(
        data.get(8..).ok_or(MediaError::UnsupportedMedia)?,
        &[b"tx3g"],
        cancel,
    )?;
    let entry = get(&entries, b"tx3g")?;
    require(entry.len() >= TX3G_ENTRY_HEADER_BYTES)?;
    require(entry[..6] == [0; 6] && entry[6..8] == [0, 1] && u32_at(entry, 8)? == 0)?;
    require(matches!(entry[12], 0 | 1 | 255) && matches!(entry[13], 0 | 1 | 255))?;
    let word = |index| i16::from_be_bytes([entry[index], entry[index + 1]]);
    require(word(18) <= word(22) && word(20) <= word(24))?;
    require(entry[26..30] == [0; 4] && entry[32] & !7 == 0)?;
    let default_font = u16::from_be_bytes([entry[30], entry[31]]);
    let boxes = children(&entry[TX3G_ENTRY_HEADER_BYTES..], &[b"ftab"], cancel)?;
    let fonts = get(&boxes, b"ftab")?;
    let count = fonts.get(..2).ok_or(MediaError::UnsupportedMedia)?;
    let count = u16::from_be_bytes([count[0], count[1]]) as usize;
    require(count <= MAX_TX3G_FONTS)?;
    let mut ids = Vec::with_capacity(MAX_TX3G_FONTS);
    let mut offset = 2usize;
    for _ in 0..count {
        check(cancel)?;
        let header = fonts
            .get(offset..offset + 3)
            .ok_or(MediaError::UnsupportedMedia)?;
        let id = u16::from_be_bytes([header[0], header[1]]);
        require(id != 0 && !ids.contains(&id) && header[2] > 0)?;
        ids.push(id);
        offset += 3;
        let end = offset + header[2] as usize;
        let name = std::str::from_utf8(fonts.get(offset..end).ok_or(MediaError::UnsupportedMedia)?)
            .map_err(|_| MediaError::UnsupportedMedia)?;
        require(!name.trim().is_empty() && !name.chars().any(char::is_control))?;
        offset = end;
    }
    require(
        offset == fonts.len() && (ids.contains(&default_font) || (count == 0 && default_font == 0)),
    )
}
struct Timing<'a> {
    table: &'a [u8],
    run: usize,
    left: u32,
    delta: u32,
    ticks: u64,
}
impl Timing<'_> {
    fn next(&mut self, cancel: &ResolverCancellation) -> Result<(u64, u64), MediaError> {
        check(cancel)?;
        if self.left == 0 {
            require(self.run < table(self.table, 8)?)?;
            self.left = u32_at(self.table, 8 + self.run * 8)?;
            self.delta = u32_at(self.table, 12 + self.run * 8)?;
            self.run += 1;
        }
        let start = self.ticks;
        self.ticks = self
            .ticks
            .checked_add(u64::from(self.delta))
            .ok_or(MediaError::UnsupportedMedia)?;
        self.left -= 1;
        Ok((start, u64::from(self.delta)))
    }
}

#[cfg(test)]
#[path = "local_mp4_subtitle_tests.rs"]
pub(crate) mod tests;
