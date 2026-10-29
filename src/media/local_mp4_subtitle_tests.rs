//! Synthetic MP4 structure with real tx3g bytes; audio bytes are gate fixtures,
//! not playable codecs. No muxer/native/cloud compatibility is asserted here.
use super::super::tests::{
    atom, body, fixture as audio_fixture, replace_atom, replace_word, video_fixture, words,
};
use super::*;
use crate::{ai::speech::SpeechLanguage, audio::capture::SessionId};
use std::io::{self, Cursor, SeekFrom};

#[path = "local_mp4_subtitle_edit_tests.rs"]
mod edits;
#[path = "local_mp4_multitrack_tests.rs"]
mod multiple;
#[path = "local_mp4_subtitle_v1_edit_tests.rs"]
mod v1_edits;
pub(crate) use edits::cropped_fixture;
pub(crate) use edits::cropped_v1_fixture;
pub(crate) use edits::delayed_fixture;
#[path = "local_mp4_subtitle_crop_tests.rs"]
mod crops;

/// Structural AAC/H.264 fixture with distinct, real text per track. No codec
/// compatibility claim. Text IDs are 3.., in the caller's declared language order.
pub(crate) fn multi_fixture(languages: &[TextLanguage]) -> Vec<u8> {
    let base = video_fixture();
    let cancel = ResolverCancellation::default();
    let roots = children(&base, &[b"ftyp", b"mdat", b"moov"], &cancel).unwrap();
    let ftyp = atom(b"ftyp", get(&roots, b"ftyp").unwrap());
    let mut payload = get(&roots, b"mdat").unwrap().to_vec();
    let mut movie = get(&roots, b"moov").unwrap().to_vec();
    replace_word(&mut movie, b"mvhd", 12, 1000);
    replace_word(&mut movie, b"mvhd", 16, 3000);
    for (index, declared) in languages.iter().enumerate() {
        let id = index as u32 + 3;
        let (first, last, code) = match declared {
            TextLanguage::Source(source) => (
                format!("Aurora Engine track {id} one"),
                format!("Aurora Engine track {id} two"),
                match source {
                    SpeechLanguage::English => b"eng",
                    SpeechLanguage::Japanese => b"jpn",
                    SpeechLanguage::French => b"fra",
                    SpeechLanguage::German => b"deu",
                    SpeechLanguage::Korean => b"kor",
                },
            ),
            _ => (
                format!("字幕軌 {id} 软件与内存"),
                format!("字幕軌 {id} 鼠标与服务器"),
                b"zho",
            ),
        };
        let samples = [sample(&first), vec![0, 0], sample(&last)];
        let offset = (ftyp.len() + 8 + payload.len()) as u32;
        let mut track = text_track(&[offset], &samples.each_ref().map(Vec::len), false);
        replace_word(&mut track, b"tkhd", 12, id);
        let at = body(&track, b"mdhd") + 20;
        track[at..at + 2].copy_from_slice(&language(code));
        movie.extend(track);
        payload.extend(samples.concat());
    }
    [ftyp, atom(b"mdat", &payload), atom(b"moov", &movie)].concat()
}

pub(crate) fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(826),
        generation: 3,
    }
}
fn selection() -> Mp4SubtitleSelection {
    Mp4SubtitleSelection::new(3, TextLanguage::TaiwanTraditional).unwrap()
}
fn sample(text: &str) -> Vec<u8> {
    [
        u16::try_from(text.len()).unwrap().to_be_bytes().to_vec(),
        text.as_bytes().to_vec(),
    ]
    .concat()
}
fn language(code: &[u8; 3]) -> [u8; 2] {
    ((u16::from(code[0] - 0x60) << 10)
        | (u16::from(code[1] - 0x60) << 5)
        | u16::from(code[2] - 0x60))
    .to_be_bytes()
}
fn description() -> Vec<u8> {
    let mut entry = vec![0; 38];
    entry[7] = 1;
    entry[31] = 1;
    entry[33] = 18;
    entry[34..38].fill(255);
    entry.extend(atom(
        b"ftab",
        &[0, 1, 0, 1, 5, b'A', b'r', b'i', b'a', b'l'],
    ));
    [words(&[0, 1]), atom(b"tx3g", &entry)].concat()
}
fn text_track(offsets: &[u32], sizes: &[usize], constant: bool) -> Vec<u8> {
    let mut mdhd = vec![0; 24];
    mdhd[12..16].copy_from_slice(&1000u32.to_be_bytes());
    mdhd[16..20].copy_from_slice(&2100u32.to_be_bytes());
    mdhd[20..22].copy_from_slice(&language(b"zho"));
    let mut handler = vec![0; 24];
    handler[8..12].copy_from_slice(b"text");
    let mut tkhd = vec![0; 84];
    tkhd[3] = 3;
    tkhd[15] = 3;
    tkhd[20..24].copy_from_slice(&2100u32.to_be_bytes());
    let mut sizes_body = words(&[
        0,
        if constant { sizes[0] as u32 } else { 0 },
        sizes.len() as u32,
    ]);
    if !constant {
        sizes_body.extend(sizes.iter().flat_map(|size| (*size as u32).to_be_bytes()));
    }
    let mut offset_body = words(&[0, offsets.len() as u32]);
    offset_body.extend(offsets.iter().flat_map(|offset| offset.to_be_bytes()));
    let stsc = if offsets.len() == 1 {
        words(&[0, 1, 1, sizes.len() as u32, 1])
    } else {
        words(&[0, 2, 1, 1, 1, 2, 2, 1])
    };
    let stbl = atom(
        b"stbl",
        &[
            atom(b"stsd", &description()),
            atom(b"stts", &words(&[0, 3, 1, 700, 1, 400, 1, 1000])),
            atom(b"stsc", &stsc),
            atom(b"stsz", &sizes_body),
            atom(b"stco", &offset_body),
        ]
        .concat(),
    );
    let dref = atom(
        b"dref",
        &[words(&[0, 1]), atom(b"url ", &[0, 0, 0, 1])].concat(),
    );
    let minf = atom(
        b"minf",
        &[atom(b"nmhd", &[0; 4]), atom(b"dinf", &dref), stbl].concat(),
    );
    let mdia = atom(
        b"mdia",
        &[atom(b"mdhd", &mdhd), atom(b"hdlr", &handler), minf].concat(),
    );
    atom(b"trak", &[atom(b"tkhd", &tkhd), mdia].concat())
}
fn make(video: bool, chunks: bool, constant: bool) -> Vec<u8> {
    let samples = if constant {
        vec![sample("甲"), sample("乙"), sample("丙")]
    } else {
        vec![sample("第一句"), vec![0, 0], sample("第二句 😀")]
    };
    make_samples(video, chunks, constant, samples)
}
fn make_samples(video: bool, chunks: bool, constant: bool, samples: Vec<Vec<u8>>) -> Vec<u8> {
    let base = if video {
        video_fixture()
    } else {
        audio_fixture()
    };
    let cancel = ResolverCancellation::default();
    let roots = children(&base, &[b"ftyp", b"mdat", b"moov"], &cancel).unwrap();
    let mut movie = get(&roots, b"moov").unwrap().to_vec();
    // The existing gate fixture uses one tick per second and no movie duration;
    // this text fixture explicitly declares a 1000 Hz, three-second movie.
    replace_word(&mut movie, b"mvhd", 12, 1000);
    replace_word(&mut movie, b"mvhd", 16, 3000);
    let ftyp = atom(b"ftyp", get(&roots, b"ftyp").unwrap());
    let mut payload = get(&roots, b"mdat").unwrap().to_vec();
    let offset = (ftyp.len() + 8 + payload.len()) as u32;
    let mut offsets = vec![offset];
    payload.extend_from_slice(&samples[0]);
    if chunks {
        payload.extend_from_slice(&[0; 7]); // Unreferenced gap, never read as text.
        offsets.push((ftyp.len() + 8 + payload.len()) as u32);
    }
    for sample in &samples[1..] {
        payload.extend_from_slice(sample);
    }
    movie.extend(text_track(
        &offsets,
        &samples.iter().map(Vec::len).collect::<Vec<_>>(),
        constant,
    ));
    [ftyp, atom(b"mdat", &payload), atom(b"moov", &movie)].concat()
}
pub(crate) fn fixture() -> Vec<u8> {
    make(true, true, false)
}
/// Explicit synthetic document for the Prepared owner/UI fixtures. The audio
/// and video remain structure-only bytes, not a playable media claim.
pub(crate) fn prepared_fixture(declared: TextLanguage) -> Vec<u8> {
    let (first, last, code) = match declared {
        TextLanguage::Source(source) => (
            "Aurora Engine one",
            "Aurora Engine two",
            match source {
                SpeechLanguage::English => b"eng",
                SpeechLanguage::Japanese => b"jpn",
                SpeechLanguage::French => b"fra",
                SpeechLanguage::German => b"deu",
                SpeechLanguage::Korean => b"kor",
            },
        ),
        _ => ("软件与内存", "鼠标与服务器", b"zho"),
    };
    let mut bytes = make_samples(
        true,
        true,
        false,
        vec![sample(first), vec![0, 0], sample(last)],
    );
    let offset = last_body(&bytes, b"mdhd") + 20;
    bytes[offset..offset + 2].copy_from_slice(&language(code));
    bytes
}
fn run(bytes: &[u8]) -> Result<ExistingText, MediaError> {
    read_subtitle(
        &mut Cursor::new(bytes),
        bytes.len() as u64,
        identity(),
        selection(),
        &ResolverCancellation::default(),
    )
}
fn last_body(bytes: &[u8], name: &[u8; 4]) -> usize {
    bytes.windows(4).rposition(|bytes| bytes == name).unwrap() + 4
}
fn change_word(bytes: &mut [u8], name: &[u8; 4], index: usize, word: u32) {
    let start = last_body(bytes, name) + index;
    bytes[start..start + 4].copy_from_slice(&word.to_be_bytes());
}
fn replace_text_atom(bytes: &[u8], path: &[&[u8; 4]], replacement: &[u8]) -> Vec<u8> {
    let start = last_body(bytes, b"trak") - 8;
    let size = u32_at(bytes, start).unwrap() as usize;
    let track = replace_atom(&bytes[start..start + size], path, replacement);
    let mut changed = bytes.to_vec();
    changed.splice(start..start + size, track);
    let moov = body(&changed, b"moov") - 8;
    let length = (changed.len() - moov) as u32;
    changed[moov..moov + 4].copy_from_slice(&length.to_be_bytes());
    changed
}
fn reject(bytes: &[u8], expected: MediaError) {
    assert!(matches!(run(bytes), Err(error) if error == expected));
}

#[test]
fn mp4_subtitle_real_text_samples_keep_timing_clear_gaps_and_identity() {
    for video in [false, true] {
        for chunks in [false, true] {
            let bytes = make(video, chunks, false);
            let original = run(&bytes).unwrap();
            assert_eq!(original.identity, identity());
            assert_eq!(original.cues.len(), 2);
            assert_eq!(&*original.cues[0].text, "第一句");
            assert_eq!(&*original.cues[1].text, "第二句 😀");
            assert_eq!(original.cues[0].start_us, Some(0));
            assert_eq!(original.cues[0].duration_us, Some(700_000));
            assert_eq!(original.cues[1].start_us, Some(1_100_000));
            assert_eq!(original.cues[1].duration_us, Some(1_000_000));
            assert!(original.language == TextLanguage::TaiwanTraditional);
            // Native audio opening has not silently adopted subtitle-track files.
            assert_eq!(
                super::super::probe(
                    &mut Cursor::new(&bytes),
                    bytes.len() as u64,
                    &ResolverCancellation::default()
                ),
                Err(MediaError::UnsupportedMedia)
            );
        }
    }
}
#[test]
fn mp4_subtitle_constant_sizes_and_64bit_offsets_share_the_same_reader() {
    let bytes = make(true, true, true);
    assert_eq!(run(&bytes).unwrap().cues.len(), 3);
    let start = last_body(&bytes, b"stco");
    let count = u32_at(&bytes, start + 4).unwrap();
    let mut table = words(&[0, count]);
    for i in 0..count as usize {
        table.extend(u64::from(u32_at(&bytes, start + 8 + i * 4).unwrap()).to_be_bytes());
    }
    let changed = replace_text_atom(
        &bytes,
        &[b"trak", b"mdia", b"minf", b"stbl", b"stco"],
        &atom(b"co64", &table),
    );
    assert_eq!(run(&changed).unwrap().cues[2].text.as_ref(), "丙");
}
#[test]
fn mp4_subtitle_language_is_explicit_with_metadata_conflict_rejection() {
    let cancel = ResolverCancellation::default();
    for (code, language) in [
        (b"eng", TextLanguage::Source(SpeechLanguage::English)),
        (b"jpn", TextLanguage::Source(SpeechLanguage::Japanese)),
        (b"fra", TextLanguage::Source(SpeechLanguage::French)),
        (b"deu", TextLanguage::Source(SpeechLanguage::German)),
        (b"kor", TextLanguage::Source(SpeechLanguage::Korean)),
        (b"zho", TextLanguage::SimplifiedChinese),
        (b"zho", TextLanguage::OtherTraditional),
        (b"zho", TextLanguage::TaiwanTraditional),
    ] {
        let mut bytes = fixture();
        let offset = last_body(&bytes, b"mdhd") + 20;
        bytes[offset..offset + 2].copy_from_slice(&self::language(code));
        let selected = Mp4SubtitleSelection::new(3, language).unwrap();
        let doc = read_subtitle(
            &mut Cursor::new(&bytes),
            bytes.len() as u64,
            identity(),
            selected,
            &cancel,
        )
        .unwrap();
        assert!(doc.language == language);
        bytes[offset..offset + 2].copy_from_slice(&self::language(b"und"));
        assert!(
            read_subtitle(
                &mut Cursor::new(&bytes),
                bytes.len() as u64,
                identity(),
                selected,
                &cancel
            )
            .is_ok()
        );
    }
    let mut bytes = fixture();
    let offset = last_body(&bytes, b"mdhd") + 20;
    for code in [
        self::language(b"eng"),
        self::language(b"mul"),
        [0, 0],
        [0xff, 0xff],
    ] {
        bytes[offset..offset + 2].copy_from_slice(&code);
        reject(&bytes, MediaError::UnsupportedMedia);
    }
}
#[test]
fn mp4_subtitle_track_ids_types_and_required_audio_are_checked() {
    let cancel = ResolverCancellation::default();
    for id in [1, 2, 4] {
        let bytes = fixture();
        assert!(
            read_subtitle(
                &mut Cursor::new(&bytes),
                bytes.len() as u64,
                identity(),
                Mp4SubtitleSelection::new(id, TextLanguage::TaiwanTraditional).unwrap(),
                &cancel
            )
            .is_err()
        );
    }
    for id in [0, 1, 2] {
        let mut bytes = fixture();
        change_word(&mut bytes, b"tkhd", 12, id);
        reject(&bytes, MediaError::UnsupportedMedia);
    }
    for kind in [b"sbtl", b"subt", b"hint"] {
        let mut bytes = fixture();
        let offset = last_body(&bytes, b"hdlr") + 8;
        bytes[offset..offset + 4].copy_from_slice(kind);
        reject(&bytes, MediaError::UnsupportedMedia);
    }
    let bytes = fixture();
    let start = body(&bytes, b"trak") - 8;
    let size = u32_at(&bytes, start).unwrap() as usize;
    let mut changed = bytes.clone();
    changed.drain(start..start + size);
    let moov = body(&changed, b"moov") - 8;
    let length = (changed.len() - moov) as u32;
    changed[moov..moov + 4].copy_from_slice(&length.to_be_bytes());
    reject(&changed, MediaError::UnsupportedMedia);
}
#[test]
fn mp4_subtitle_cross_track_aliases_and_out_of_mdat_offsets_reject() {
    for target in [body(&fixture(), b"stco"), last_body(&fixture(), b"stco")] {
        let mut bytes = fixture();
        let aliased = u32_at(&bytes, body(&bytes, b"stco") + 8).unwrap();
        if target == last_body(&bytes, b"stco") {
            bytes[target + 8..target + 12].copy_from_slice(&aliased.to_be_bytes());
        } else {
            let text = u32_at(&bytes, last_body(&bytes, b"stco") + 8).unwrap();
            bytes[target + 8..target + 12].copy_from_slice(&text.to_be_bytes());
        }
        reject(&bytes, MediaError::UnsupportedMedia);
    }
    for offset in [0, 8, u32::MAX] {
        let mut bytes = fixture();
        change_word(&mut bytes, b"stco", 8, offset);
        reject(&bytes, MediaError::UnsupportedMedia);
    }
    // Third track must not alias the middle video track either.
    let mut bytes = fixture();
    let offsets: Vec<_> = bytes
        .windows(4)
        .enumerate()
        .filter_map(|(i, b)| (b == b"stco").then_some(i + 4))
        .collect();
    let video = u32_at(&bytes, offsets[1] + 8).unwrap();
    change_word(&mut bytes, b"stco", 8, video);
    reject(&bytes, MediaError::UnsupportedMedia);
}
#[test]
fn mp4_subtitle_tables_and_durations_must_match_before_text_reading() {
    for (kind, offset, value) in [
        (b"stts", 4, 0),
        (b"stts", 8, 0),
        (b"stts", 12, 0),
        (b"stts", 12, 701),
        (b"stsc", 8, 2),
        (b"stsc", 12, 0),
        (b"stsc", 16, 2),
        (b"stsz", 8, 4),
        (b"mdhd", 12, 0),
        (b"mdhd", 16, 2099),
        (b"tkhd", 20, 2099),
    ] {
        let mut bytes = fixture();
        change_word(&mut bytes, kind, offset, value);
        reject(&bytes, MediaError::UnsupportedMedia);
    }
    let mut bytes = fixture();
    replace_word(&mut bytes, b"mvhd", 16, 2099);
    reject(&bytes, MediaError::UnsupportedMedia);
}
#[test]
fn mp4_subtitle_rejects_external_reference_edit_composition_and_encryption() {
    let bytes = fixture();
    let mut changed = bytes.clone();
    change_word(&mut changed, b"url ", 0, 0);
    reject(&changed, MediaError::UnsupportedMedia);
    for (parent, extra) in [
        (&[b"trak"][..], atom(b"edts", &[0; 8])),
        (
            &[b"trak", b"mdia", b"minf", b"stbl"][..],
            atom(b"ctts", &words(&[0, 1, 3, 0])),
        ),
        (
            &[b"trak", b"mdia", b"minf", b"stbl"][..],
            atom(b"senc", &[0; 8]),
        ),
    ] {
        let name = parent.last().unwrap();
        let offset = last_body(&bytes, name);
        let length = u32_at(&bytes, offset - 8).unwrap() as usize;
        let replaced = atom(
            name,
            &[bytes[offset..offset + length - 8].to_vec(), extra].concat(),
        );
        reject(
            &replace_text_atom(&bytes, parent, &replaced),
            MediaError::UnsupportedMedia,
        );
    }
}
#[test]
fn mp4_subtitle_description_rejects_dynamic_flags_ranges_and_font_errors() {
    for (offset, value) in [
        (0, 1),
        (7, 2),
        (11, 0x20),
        (12, 2),
        (13, 2),
        (27, 1),
        (29, 1),
        (31, 2),
        (32, 8),
    ] {
        let mut bytes = fixture();
        let start = last_body(&bytes, b"tx3g");
        bytes[start + offset] = value;
        reject(&bytes, MediaError::UnsupportedMedia);
    }
    for (offset, value) in [(1, 33), (3, 0), (4, 0), (5, 0xff), (5, 0)] {
        let mut bytes = fixture();
        let start = last_body(&bytes, b"ftab");
        bytes[start + offset] = value;
        reject(&bytes, MediaError::UnsupportedMedia);
    }
    let bytes = fixture();
    let mut description = description();
    let entry = body(&description, b"tx3g");
    description[entry + 18..entry + 20].copy_from_slice(&1i16.to_be_bytes());
    reject(
        &replace_text_atom(
            &bytes,
            &[b"trak", b"mdia", b"minf", b"stbl", b"stsd"],
            &atom(b"stsd", &description),
        ),
        MediaError::UnsupportedMedia,
    );
}
#[test]
fn mp4_subtitle_malformed_second_text_sample_never_returns_first_cue() {
    let mut bytes = fixture();
    let offset = u32_at(&bytes, last_body(&bytes, b"stco") + 12).unwrap() as usize;
    bytes[offset..offset + 2].copy_from_slice(&1u16.to_be_bytes()); // Clearing sample has only two bytes.
    reject(&bytes, MediaError::InvalidSubtitle);
    let mut bytes = fixture();
    let first = u32_at(&bytes, last_body(&bytes, b"stco") + 8).unwrap() as usize;
    bytes[first + 2] = 0xff;
    reject(&bytes, MediaError::InvalidSubtitle);
}
#[test]
fn mp4_subtitle_extended_top_box_and_cancel_share_bounded_loader() {
    let bytes = fixture();
    let start = body(&bytes, b"moov") - 8;
    let mut changed = bytes[..start].to_vec();
    changed.extend(words(&[1]));
    changed.extend(b"moov");
    changed.extend(((bytes.len() - start + 8) as u64).to_be_bytes());
    changed.extend_from_slice(&bytes[start + 8..]);
    assert_eq!(run(&changed).unwrap().cues.len(), 2);
    let cancel = ResolverCancellation::default();
    cancel.cancel();
    assert!(matches!(
        read_subtitle(
            &mut Cursor::new(&bytes),
            bytes.len() as u64,
            identity(),
            selection(),
            &cancel
        ),
        Err(MediaError::Cancelled)
    ));
}
#[test]
fn mp4_subtitle_every_file_truncation_rejects_and_never_panics() {
    let bytes = fixture();
    for end in 0..bytes.len() {
        assert!(run(&bytes[..end]).is_err(), "accepted truncation at {end}");
    }
}
struct Counted {
    inner: Cursor<Vec<u8>>,
    text_reads: usize,
    payload: (u64, u64),
    cancel: ResolverCancellation,
    cancel_at_payload: bool,
}
impl Read for Counted {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let at = self.inner.position();
        // The shared container loader performs one last-byte mdat liveness
        // probe; only complete text-sample reads participate in this counter.
        if at >= self.payload.0 && at < self.payload.1 && bytes.len() > 1 {
            self.text_reads += 1;
            if self.cancel_at_payload {
                self.cancel.cancel();
            }
        }
        self.inner.read(bytes)
    }
}
impl Seek for Counted {
    fn seek(&mut self, at: SeekFrom) -> io::Result<u64> {
        self.inner.seek(at)
    }
}
fn counted(bytes: Vec<u8>, cancel_at_payload: bool) -> Counted {
    let start = body(&bytes, b"mdat") as u64;
    let end = start + u64::from(u32_at(&bytes, start as usize - 8).unwrap()) - 8;
    Counted {
        inner: Cursor::new(bytes),
        text_reads: 0,
        payload: (start, end),
        cancel: ResolverCancellation::default(),
        cancel_at_payload,
    }
}
#[test]
fn mp4_discovery_language_hints_never_infer_chinese_region_or_script() {
    for (code, expected) in [
        (b"zho", Mp4SubtitleLanguage::Chinese),
        (b"und", Mp4SubtitleLanguage::Undetermined),
        (b"eng", Mp4SubtitleLanguage::Source(SpeechLanguage::English)),
        (
            b"jpn",
            Mp4SubtitleLanguage::Source(SpeechLanguage::Japanese),
        ),
        (b"fra", Mp4SubtitleLanguage::Source(SpeechLanguage::French)),
        (b"deu", Mp4SubtitleLanguage::Source(SpeechLanguage::German)),
        (b"kor", Mp4SubtitleLanguage::Source(SpeechLanguage::Korean)),
    ] {
        let mut bytes = fixture();
        let at = last_body(&bytes, b"mdhd") + 20;
        bytes[at..at + 2].copy_from_slice(&language(code));
        let mut reader = counted(bytes, false);
        let len = reader.inner.get_ref().len() as u64;
        let result = discover_subtitle(&mut reader, len, &ResolverCancellation::default()).unwrap();
        assert_eq!(
            result,
            Mp4SubtitleTrack {
                track_id: 3,
                language: expected
            }
        );
        assert_eq!(reader.text_reads, 0); // Except shared loader's one-byte EOF probe.
    }
}
#[test]
fn mp4_discovery_validates_tables_ranges_languages_and_description_before_reporting_track() {
    let base = fixture();
    let mut cases = Vec::new();
    for (kind, offset, value) in [
        (b"stco", 8, 0),
        (b"stsz", 8, 50_000),
        (b"stts", 12, 0),
        (b"tkhd", 12, 0),
        (b"mdhd", 16, 1),
    ] {
        let mut bytes = base.clone();
        change_word(&mut bytes, kind, offset, value);
        cases.push(bytes);
    }
    for code in [b"mul", b"spa"] {
        let mut bytes = base.clone();
        let at = last_body(&bytes, b"mdhd") + 20;
        bytes[at..at + 2].copy_from_slice(&language(code));
        cases.push(bytes);
    }
    let mut bytes = base.clone();
    let at = last_body(&bytes, b"tx3g");
    bytes[at + 7] = 2; // External data reference.
    cases.push(bytes);
    for bytes in cases {
        let mut reader = counted(bytes, false);
        let len = reader.inner.get_ref().len() as u64;
        assert!(discover_subtitle(&mut reader, len, &ResolverCancellation::default()).is_err());
        assert_eq!(reader.text_reads, 0);
    }
}
#[test]
fn mp4_discovery_is_not_text_validation_or_ready_and_extraction_rechecks_payload() {
    let mut bytes = fixture();
    let at = body(&bytes, b"mdat");
    // Corrupt the first actual text sample length, not any metadata table.
    let text_offset = u32_at(&bytes, last_body(&bytes, b"stco") + 8).unwrap() as usize;
    assert!(text_offset >= at);
    bytes[text_offset..text_offset + 2].copy_from_slice(&u16::MAX.to_be_bytes());
    assert!(
        discover_subtitle(
            &mut Cursor::new(&bytes),
            bytes.len() as u64,
            &ResolverCancellation::default()
        )
        .is_ok()
    );
    assert!(run(&bytes).is_err());
}
#[test]
fn mp4_discovery_truncation_and_pre_cancel_never_report_a_candidate() {
    let bytes = fixture();
    for end in 0..bytes.len() {
        assert!(
            discover_subtitle(
                &mut Cursor::new(&bytes[..end]),
                end as u64,
                &ResolverCancellation::default()
            )
            .is_err()
        );
    }
    let cancel = ResolverCancellation::default();
    cancel.cancel();
    assert_eq!(
        discover_subtitle(&mut Cursor::new(&bytes), bytes.len() as u64, &cancel),
        Err(MediaError::Cancelled)
    );
}
#[test]
fn mp4_subtitle_invalid_range_is_rejected_before_reading_any_text_sample() {
    let mut bytes = fixture();
    change_word(&mut bytes, b"stco", 8, 0);
    let mut reader = counted(bytes, false);
    let len = reader.inner.get_ref().len() as u64;
    let cancel = reader.cancel.clone();
    assert!(matches!(
        read_subtitle(&mut reader, len, identity(), selection(), &cancel),
        Err(MediaError::UnsupportedMedia)
    ));
    assert_eq!(reader.text_reads, 0);
}

#[test]
fn mp4_subtitle_utf16_sample_sizes_drive_actual_offsets_and_decoding() {
    for big_endian in [true, false] {
        let mut raw = if big_endian {
            vec![0xfe, 0xff]
        } else {
            vec![0xff, 0xfe]
        };
        for word in "台灣 😀".encode_utf16() {
            raw.extend(if big_endian {
                word.to_be_bytes()
            } else {
                word.to_le_bytes()
            });
        }
        let first = [
            u16::try_from(raw.len()).unwrap().to_be_bytes().to_vec(),
            raw,
        ]
        .concat();
        let bytes = make_samples(true, true, false, vec![first, vec![0, 0], sample("後句")]);
        let document = run(&bytes).unwrap();
        assert_eq!(document.cues[0].text.as_ref(), "台灣 😀");
        assert_eq!(document.cues[1].text.as_ref(), "後句");
        assert_eq!(document.cues[1].start_us, Some(1_100_000));
    }
}

#[test]
fn mp4_subtitle_movie_duration_rounding_does_not_rewrite_media_ticks() {
    for declared in [23, 24] {
        let mut bytes = fixture();
        change_word(&mut bytes, b"mdhd", 12, 90_000);
        change_word(&mut bytes, b"tkhd", 20, declared);
        let document = run(&bytes).unwrap();
        assert_eq!(document.cues[0].duration_us, Some(7777));
        assert_eq!(document.cues[1].start_us, Some(12222));
        assert_eq!(document.cues[1].duration_us, Some(11111));
    }
    let mut bytes = fixture();
    change_word(&mut bytes, b"mdhd", 12, 90_000);
    change_word(&mut bytes, b"tkhd", 20, 25);
    reject(&bytes, MediaError::UnsupportedMedia);
}

#[test]
fn mp4_subtitle_sample_capacity_and_empty_text_are_typed_without_partial_output() {
    let small = make_samples(true, true, false, vec![vec![0], vec![0, 0], sample("後句")]);
    reject(&small, MediaError::InvalidSubtitle);
    let empty = make_samples(true, true, true, vec![vec![0, 0]; 3]);
    reject(&empty, MediaError::InvalidSubtitle);
    let oversized = make_samples(
        true,
        true,
        false,
        vec![
            vec![0; MAX_TX3G_SAMPLE_BYTES + 1],
            vec![0, 0],
            sample("後句"),
        ],
    );
    let mut reader = counted(oversized, false);
    let len = reader.inner.get_ref().len() as u64;
    let cancel = reader.cancel.clone();
    assert!(matches!(
        read_subtitle(&mut reader, len, identity(), selection(), &cancel),
        Err(MediaError::TooLarge)
    ));
    assert_eq!(reader.text_reads, 0);
    let mut too_many = fixture();
    change_word(&mut too_many, b"stsz", 8, MAX_TX3G_SAMPLES as u32 + 1);
    reject(&too_many, MediaError::TooLarge);
}

#[test]
fn mp4_subtitle_description_font_table_is_bounded_unique_and_exact() {
    let cancel = ResolverCancellation::default();
    let valid = description();
    for end in 0..valid.len() {
        assert!(validate_description(&valid[..end], &cancel).is_err());
    }
    let mut repeated = valid.clone();
    let start = body(&repeated, b"ftab");
    repeated[start..start + 2].copy_from_slice(&2u16.to_be_bytes());
    let record = repeated[start + 2..].to_vec();
    repeated.extend_from_slice(&record);
    let font_size = (repeated.len() - start + 8) as u32;
    repeated[start - 8..start - 4].copy_from_slice(&font_size.to_be_bytes());
    let entry = body(&repeated, b"tx3g");
    let entry_size = (repeated.len() - entry + 8) as u32;
    repeated[entry - 8..entry - 4].copy_from_slice(&entry_size.to_be_bytes());
    assert_eq!(
        validate_description(&repeated, &cancel),
        Err(MediaError::UnsupportedMedia)
    );
    // Valid empty font table with unspecified default font; no font is loaded.
    let mut plain = vec![0; TX3G_ENTRY_HEADER_BYTES];
    plain[7] = 1;
    plain.extend(atom(b"ftab", &[0, 0]));
    assert_eq!(
        validate_description(&[words(&[0, 1]), atom(b"tx3g", &plain)].concat(), &cancel),
        Ok(())
    );
    let mut extra = valid.clone();
    extra.push(0);
    assert_eq!(
        validate_description(&extra, &cancel),
        Err(MediaError::UnsupportedMedia)
    );
}
#[test]
fn mp4_subtitle_cancellation_inside_payload_read_discards_the_document() {
    let mut reader = counted(fixture(), true);
    let len = reader.inner.get_ref().len() as u64;
    let cancel = reader.cancel.clone();
    assert!(matches!(
        read_subtitle(&mut reader, len, identity(), selection(), &cancel),
        Err(MediaError::Cancelled)
    ));
    assert_eq!(reader.text_reads, 1);
}
