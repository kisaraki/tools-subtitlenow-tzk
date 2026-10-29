//! Test-only native encoding of deterministic PCM in COM RAM streams.
//! Not a product recorder/exporter; no key, capture, user media or output path.
use windows::{
    Win32::{
        Foundation::HGLOBAL, Media::MediaFoundation::*,
        System::Com::StructuredStorage::CreateStreamOnHGlobal, UI::Shell::SHCreateMemStream,
    },
    core::{Interface, Result},
};

// Sixteen seconds at the largest supported rate, so a native clock test can
// exceed the twelve-second rolling window without a whole-media PCM buffer.
const MAX_FIXTURE_FRAMES: usize = 16 * 48_000;
const MAX_ENCODED_BYTES: u64 = 320 * 1024;
const BITRATE_BYTES_PER_SECOND: u32 = 16_000;
const TONE_HZ: f64 = 440.0;
const TONE_AMPLITUDE: f64 = 6000.0;

struct Foundation;
impl Drop for Foundation {
    fn drop(&mut self) {
        // SAFETY: all same-thread encoder/sink/streams were released first.
        unsafe {
            let _ = MFShutdown();
        }
    }
}
struct Sink(IMFMediaSink);
impl Drop for Sink {
    fn drop(&mut self) {
        // SAFETY: writer/samples are gone before this local sink guard drops.
        unsafe {
            let _ = self.0.Shutdown();
        }
    }
}
struct Locked<'a>(&'a IMFMediaBuffer);
impl Drop for Locked<'_> {
    fn drop(&mut self) {
        // SAFETY: constructed immediately after one successful buffer Lock.
        unsafe {
            let _ = self.0.Unlock();
        }
    }
}

pub(crate) fn synthetic_mp3(rate: u32, channels: u16, frames: usize) -> Result<Vec<u8>> {
    assert!(matches!(rate, 32_000 | 44_100 | 48_000));
    encode(rate, channels, frames, Encoding::Mp3)
}
/// Test-only real AAC encoding in an MP4 RAM sink, then lossless packet wrapping
/// as ADTS. No product encoding API, temp file or guessed compressed payload.
pub(crate) fn synthetic_adts(rate: u32, channels: u16, frames: usize) -> Result<Vec<u8>> {
    assert!(matches!(rate, 44_100 | 48_000));
    encode(rate, channels, frames, Encoding::Adts)
}
/// The actual finalized audio-only MP4 from the same bounded RAM sink.
pub(crate) fn synthetic_mp4(rate: u32, channels: u16, frames: usize) -> Result<Vec<u8>> {
    assert!(matches!(rate, 44_100 | 48_000));
    encode(rate, channels, frames, Encoding::Mp4)
}
/// Real Baseline H.264 plus AAC in RAM; video is synthetic NV12, not user media.
pub(crate) fn synthetic_mp4_video(rate: u32, channels: u16, frames: usize) -> Result<Vec<u8>> {
    synthetic_mp4_video_profile(rate, channels, frames, H264Profile::Baseline)
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum H264Profile {
    Baseline = 66,
    Main = 77,
    High = 100,
}
pub(crate) fn synthetic_mp4_video_profile(
    rate: u32,
    channels: u16,
    frames: usize,
    profile: H264Profile,
) -> Result<Vec<u8>> {
    assert!(matches!(rate, 44_100 | 48_000));
    encode(rate, channels, frames, Encoding::Mp4Video(profile))
}
pub(crate) fn synthetic_mp4_b_frames(
    rate: u32,
    channels: u16,
    frames: usize,
    profile: H264Profile,
) -> Result<Vec<u8>> {
    assert!(matches!(rate, 44_100 | 48_000));
    assert!(matches!(profile, H264Profile::Main | H264Profile::High));
    assert!(frames >= 4 * rate as usize);
    encode(rate, channels, frames, Encoding::Mp4BFrames(profile))
}
/// Replace only the trailing user-data atom of a genuine synthetic MP4. The
/// encoder's mdat, tables, AAC bytes and offsets stay byte-for-byte identical.
pub(crate) fn tagged_mp4(bytes: &[u8]) -> Vec<u8> {
    fn atom(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        [
            ((body.len() + 8) as u32).to_be_bytes().to_vec(),
            kind.to_vec(),
            body.to_vec(),
        ]
        .concat()
    }
    fn item(kind: &[u8; 4], value_type: u32, value: &[u8]) -> Vec<u8> {
        atom(
            kind,
            &atom(
                b"data",
                &[
                    value_type.to_be_bytes().to_vec(),
                    vec![0; 4],
                    value.to_vec(),
                ]
                .concat(),
            ),
        )
    }
    fn png_chunk(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut crc = !0u32;
        for byte in kind.iter().chain(body) {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb88320u32 & (0u32.wrapping_sub(crc & 1)));
            }
        }
        [
            (body.len() as u32).to_be_bytes().to_vec(),
            kind.to_vec(),
            body.to_vec(),
            (!crc).to_be_bytes().to_vec(),
        ]
        .concat()
    }
    // A real 1x1 black grayscale PNG: CRC-covered chunks and zlib's stored
    // DEFLATE block with Adler32 over [filter=0,pixel=0]. No image decoder.
    let png = [
        b"\x89PNG\r\n\x1a\n".to_vec(),
        png_chunk(b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 0, 0, 0, 0]),
        png_chunk(b"IDAT", &[0x78, 1, 1, 2, 0, 0xfd, 0xff, 0, 0, 0, 2, 0, 1]),
        png_chunk(b"IEND", &[]),
    ]
    .concat();
    let items = [
        item(b"\xa9nam", 1, "臺灣合成標題".as_bytes()),
        item(b"\xa9ART", 1, b"synthetic artist"),
        item(b"\xa9alb", 1, b"synthetic album"),
        item(b"\xa9cmt", 1, b"https://127.0.0.1/synthetic-never-fetch"),
        item(b"trkn", 0, &[0, 0, 0, 1, 0, 10, 0, 0]),
        item(b"disk", 0, &[0, 0, 0, 1, 0, 2]),
        item(b"tmpo", 21, &[0, 120]),
        item(b"cpil", 21, &[1]),
        item(b"covr", 14, &png),
    ]
    .concat();
    let mut handler = vec![0; 24];
    handler[8..12].copy_from_slice(b"mdir");
    let udta = atom(
        b"udta",
        &atom(
            b"meta",
            &[vec![0; 4], atom(b"hdlr", &handler), atom(b"ilst", &items)].concat(),
        ),
    );
    let mut result = Vec::new();
    let mut offset = 0;
    let mut replaced = false;
    while offset < bytes.len() {
        let size = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
        assert!(size >= 8 && offset + size <= bytes.len());
        if &bytes[offset + 4..offset + 8] == b"moov" {
            assert_eq!(
                offset + size,
                bytes.len(),
                "native fixture moov must be last"
            );
            let mut movie = Vec::new();
            let mut child = offset + 8;
            while child < offset + size {
                let size = u32::from_be_bytes(bytes[child..child + 4].try_into().unwrap()) as usize;
                assert!(size >= 8 && child + size <= bytes.len());
                if &bytes[child + 4..child + 8] != b"udta" {
                    movie.extend_from_slice(&bytes[child..child + size]);
                }
                child += size;
            }
            movie.extend_from_slice(&udta);
            result.extend(atom(b"moov", &movie));
            replaced = true;
        } else {
            result.extend_from_slice(&bytes[offset..offset + size]);
        }
        offset += size;
    }
    assert!(replaced && result.len() <= MAX_ENCODED_BYTES as usize);
    result
}
enum Encoding {
    Mp3,
    Adts,
    Mp4,
    Mp4Video(H264Profile),
    Mp4BFrames(H264Profile),
}
fn encode(rate: u32, channels: u16, frames: usize, encoding: Encoding) -> Result<Vec<u8>> {
    let aac = !matches!(encoding, Encoding::Mp3);
    let b_frames = matches!(encoding, Encoding::Mp4BFrames(_));
    let video_profile =
        if let Encoding::Mp4Video(profile) | Encoding::Mp4BFrames(profile) = encoding {
            Some(profile)
        } else {
            None
        };
    let video = video_profile.is_some();
    assert!(matches!(channels, 1 | 2));
    assert!(frames > 0 && frames <= MAX_FIXTURE_FRAMES);
    let _apartment = super::resources::Apartment::new().expect("synthetic MTA initialization");
    // SAFETY: same-thread MTA; a successful startup is paired by the guard.
    unsafe {
        MFStartup(MF_VERSION, MFSTARTUP_LITE)?;
    }
    let _foundation = Foundation;
    // SAFETY: no supplied memory handle. COM allocates/owns the stream and frees
    // its RAM on last Release; no disk, filename, URL or user content is used.
    let stream = unsafe {
        if aac {
            // MP4 requires an explicitly writable/seekable stream. HGLOBAL's
            // Stat reports read-only on this host despite allowing Write.
            SHCreateMemStream(None).expect("synthetic read/write RAM stream")
        } else {
            CreateStreamOnHGlobal(HGLOBAL::default(), true)?
        }
    };
    // SAFETY: all interfaces are live on this fixture's thread; writer copies
    // fixed media types and owns referenced buffers until Finalize completes.
    unsafe {
        let bytes = MFCreateMFByteStreamOnStream(&stream)?;
        let output = MFCreateMediaType()?;
        output.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        output.SetGUID(
            &MF_MT_SUBTYPE,
            if aac {
                &MFAudioFormat_AAC
            } else {
                &MFAudioFormat_MP3
            },
        )?;
        output.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, rate)?;
        output.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, u32::from(channels))?;
        output.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, BITRATE_BYTES_PER_SECOND)?;
        if aac {
            output.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
            output.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, 1)?;
            output.SetUINT32(&MF_MT_AVG_BITRATE, BITRATE_BYTES_PER_SECOND * 8)?;
            output.SetUINT32(&MF_MT_ALL_SAMPLES_INDEPENDENT, 0)?;
            output.SetUINT32(&MF_MT_AAC_PAYLOAD_TYPE, 0)?;
            output.SetUINT32(&MF_MT_AAC_AUDIO_PROFILE_LEVEL_INDICATION, 0x29)?;
            let asc =
                (2u16 << 11) | ((if rate == 48_000 { 3u16 } else { 4u16 }) << 7) | (channels << 3);
            let mut user_data = vec![0, 0, 0x29, 0, 0, 0, 0, 0, 0, 0, 0, 0];
            user_data.extend_from_slice(&asc.to_be_bytes());
            output.SetBlob(&MF_MT_USER_DATA, &user_data)?;
        }
        let video_type = if let Some(profile) = video_profile {
            let media = video_type(MFVideoFormat_H264)?;
            media.SetUINT32(&MF_MT_AVG_BITRATE, 64_000)?;
            media.SetUINT32(&MF_MT_MPEG2_PROFILE, profile as u32)?;
            Some(media)
        } else {
            None
        };
        let sink = Sink(if aac {
            MFCreateMPEG4MediaSink(&bytes, video_type.as_ref(), &output).map_err(|error| {
                windows::core::Error::new(
                    error.code(),
                    format!(
                        "synthetic AAC sink creation, stream capabilities {}",
                        bytes.GetCapabilities().unwrap_or(0)
                    ),
                )
            })?
        } else {
            MFCreateMP3MediaSink(&bytes)?
        });
        // The MP3 sink has one fixed stream. Configure that stream before
        // wrapping it; AddStream would fail with MF_E_STREAMSINKS_FIXED.
        let fixed = sink.0.GetStreamSinkByIndex(0)?;
        if !aac {
            fixed.GetMediaTypeHandler()?.SetCurrentMediaType(&output)?;
        }
        let writer = MFCreateSinkWriterFromMediaSink(&sink.0, None)?;
        // Sink Writer uses zero-based indices, not the MP4 sink's stream ID.
        let mut index = 0;
        let mut video_index = None;
        for candidate in 0..sink.0.GetStreamSinkCount()? {
            let major = sink
                .0
                .GetStreamSinkByIndex(candidate)?
                .GetMediaTypeHandler()?
                .GetMajorType()?;
            if major == MFMediaType_Audio {
                index = candidate;
            } else if major == MFMediaType_Video {
                video_index = Some(candidate);
            }
        }
        let input = MFCreateMediaType()?;
        input.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        input.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)?;
        input.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, rate)?;
        input.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, u32::from(channels))?;
        input.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
        input.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, u32::from(channels) * 2)?;
        input.SetUINT32(
            &MF_MT_AUDIO_AVG_BYTES_PER_SECOND,
            rate * u32::from(channels) * 2,
        )?;
        input.SetUINT32(&MF_MT_ALL_SAMPLES_INDEPENDENT, 1)?;
        writer
            .SetInputMediaType(index, &input, None)
            .map_err(|error| {
                windows::core::Error::new(error.code(), "synthetic encoder input type")
            })?;
        if let Some(video_index) = video_index {
            writer.SetInputMediaType(video_index, &self::video_type(MFVideoFormat_NV12)?, None)?;
            if b_frames {
                configure_b_frames(&writer, video_index)?;
            }
        }
        writer
            .BeginWriting()
            .map_err(|error| windows::core::Error::new(error.code(), "synthetic encoder begin"))?;
        let chunk = (rate / 50) as usize;
        for start in (0..frames).step_by(chunk) {
            if let Some(video_index) = video_index.filter(|_| start.is_multiple_of(rate as usize)) {
                const VIDEO_BYTES: u32 = 128 * 96 * 3 / 2;
                let buffer = MFCreateMemoryBuffer(VIDEO_BYTES)?;
                let mut pointer = std::ptr::null_mut();
                let mut maximum = 0;
                buffer.Lock(&mut pointer, Some(&mut maximum), None)?;
                let locked = Locked(&buffer);
                assert!(!pointer.is_null() && maximum >= VIDEO_BYTES);
                // SAFETY: locked buffer covers VIDEO_BYTES; NV12 planes are
                // initialized before Unlock and before the writer sees them.
                let data = std::slice::from_raw_parts_mut(pointer, VIDEO_BYTES as usize);
                data[..128 * 96].fill(80);
                data[128 * 96..].fill(128);
                drop(locked);
                buffer.SetCurrentLength(VIDEO_BYTES)?;
                let sample = MFCreateSample()?;
                sample.AddBuffer(&buffer)?;
                sample.SetSampleTime(start as i64 * 10_000_000 / i64::from(rate))?;
                sample.SetSampleDuration(
                    (frames - start).min(rate as usize) as i64 * 10_000_000 / i64::from(rate),
                )?;
                writer.WriteSample(video_index, &sample)?;
            }
            let end = (start + chunk).min(frames);
            let length = ((end - start) * usize::from(channels) * 2) as u32;
            let buffer = MFCreateMemoryBuffer(length)?;
            let mut pointer = std::ptr::null_mut();
            let mut maximum = 0;
            buffer.Lock(&mut pointer, Some(&mut maximum), None)?;
            let locked = Locked(&buffer);
            assert!(!pointer.is_null() && maximum >= length);
            // SAFETY: successful Lock covers length initialized bytes. Use byte
            // pairs, never assume i16 alignment; guard Unlock precedes handoff.
            let data = std::slice::from_raw_parts_mut(pointer, length as usize);
            for (offset, sample) in data.as_chunks_mut::<2>().0.iter_mut().enumerate() {
                let frame = start + offset / usize::from(channels);
                let value = (TONE_AMPLITUDE
                    * (std::f64::consts::TAU * TONE_HZ * frame as f64 / f64::from(rate)).sin())
                .round() as i16;
                *sample = value.to_le_bytes();
            }
            drop(locked);
            buffer.SetCurrentLength(length)?;
            let sample = MFCreateSample()?;
            sample.AddBuffer(&buffer)?;
            let time = start as i64 * 10_000_000 / i64::from(rate);
            let end_time = end as i64 * 10_000_000 / i64::from(rate);
            sample.SetSampleTime(time)?;
            sample.SetSampleDuration(end_time - time)?;
            writer.WriteSample(index, &sample)?;
        }
        writer.Finalize()?;
        bytes.Flush()?;
        let length = bytes.GetLength()?;
        assert!(length > 0 && length <= MAX_ENCODED_BYTES);
        bytes.SetCurrentPosition(0)?;
        if video {
            verify_video(&bytes, frames.div_ceil(rate as usize), b_frames)?;
            bytes.SetCurrentPosition(0)?;
        }
        if matches!(encoding, Encoding::Adts) {
            return as_adts(&bytes, rate, channels);
        }
        let mut result = vec![0; length as usize];
        let mut filled = 0;
        while filled < result.len() {
            let mut read = 0;
            bytes.Read(&mut result[filled..], &mut read)?;
            assert!(read > 0 && read as usize <= result.len() - filled);
            filled += read as usize;
        }
        Ok(result)
    }
}

fn video_type(subtype: windows::core::GUID) -> Result<IMFMediaType> {
    // SAFETY: fixture caller initialized same-thread COM/MF; fixed attributes
    // contain no pointers, and returned interface stays on that thread.
    unsafe {
        let media = MFCreateMediaType()?;
        media.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        media.SetGUID(&MF_MT_SUBTYPE, &subtype)?;
        media.SetUINT64(&MF_MT_FRAME_SIZE, (128u64 << 32) | 96)?;
        media.SetUINT64(&MF_MT_FRAME_RATE, (1u64 << 32) | 1)?;
        media.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1u64 << 32) | 1)?;
        media.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        Ok(media)
    }
}

fn configure_b_frames(writer: &IMFSinkWriter, index: u32) -> Result<()> {
    // SAFETY: fixture-owned writer has not begun streaming. Its synchronous
    // encoder remains on this MTA; saved types and numeric VARIANTs stay live
    // across copied COM calls, and the same input/output types are restored.
    unsafe {
        let extended: IMFSinkWriterEx = writer.cast()?;
        let mut transform = None;
        extended.GetTransformForStream(index, 0, None, &mut transform)?;
        let transform = transform.expect("synthetic video encoder");
        let output = transform.GetOutputCurrentType(0)?;
        assert_eq!(output.GetGUID(&MF_MT_SUBTYPE)?, MFVideoFormat_H264);
        let input = transform.GetInputCurrentType(0)?;
        let codec: ICodecAPI = transform.cast()?;
        transform.SetInputType(0, None, 0)?;
        transform.SetOutputType(0, None, 0)?;
        codec.SetValue(
            &CODECAPI_AVEncMPVDefaultBPictureCount,
            &windows::Win32::System::Variant::VARIANT::from(2u32),
        )?;
        transform.SetOutputType(0, &output, 0)?;
        transform.SetInputType(0, &input, 0)?;
        Ok(())
    }
}

fn verify_video(bytes: &IMFByteStream, expected: usize, b_frames: bool) -> Result<()> {
    // SAFETY: finalized synthetic stream, live same-thread COM/MF. Raw H.264
    // samples remain in bounded native buffers and are never saved/rendered.
    unsafe {
        let attributes: IMFAttributes = bytes.cast()?;
        attributes.SetString(
            &MF_BYTESTREAM_CONTENT_TYPE,
            &windows::core::HSTRING::from("video/mp4"),
        )?;
        let reader = MFCreateSourceReaderFromByteStream(bytes, None)?;
        let video = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
        reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)?;
        reader.SetStreamSelection(video, true)?;
        let native = reader.GetNativeMediaType(video, 0)?;
        assert_eq!(native.GetGUID(&MF_MT_SUBTYPE)?, MFVideoFormat_H264);
        assert_eq!(native.GetUINT64(&MF_MT_FRAME_SIZE)?, (128u64 << 32) | 96);
        reader.SetCurrentMediaType(video, None, &native)?;
        let mut count = 0;
        let mut b_count = 0;
        let mut previous_time = None;
        let mut reordered = 0;
        for _ in 0..64 {
            let mut flags = 0;
            let mut sample = None;
            reader.ReadSample(video, 0, None, Some(&mut flags), None, Some(&mut sample))?;
            assert_eq!(flags & MF_SOURCE_READERF_ERROR.0 as u32, 0);
            if let Some(sample) = sample {
                let time = sample.GetSampleTime()?;
                reordered += usize::from(previous_time.is_some_and(|previous| time < previous));
                previous_time = Some(time);
                let length = sample.GetTotalLength()?;
                assert!(length > 0 && u64::from(length) <= MAX_ENCODED_BYTES);
                count += 1;
                if b_frames {
                    let buffer = sample.ConvertToContiguousBuffer()?;
                    let mut pointer = std::ptr::null_mut();
                    let mut maximum = 0;
                    let mut length = 0;
                    buffer.Lock(&mut pointer, Some(&mut maximum), Some(&mut length))?;
                    let locked = Locked(&buffer);
                    assert!(!pointer.is_null() && length > 0 && length <= maximum);
                    // SAFETY: successful Lock exposes length initialized bytes.
                    // Only bounded compressed fixture data is borrowed here.
                    let data = std::slice::from_raw_parts(pointer, length as usize);
                    b_count += usize::from(has_b_slice(data));
                    drop(locked);
                }
            }
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                assert_eq!(count, expected);
                if b_frames {
                    assert!(b_count > 0, "native encoder emitted no proven B slices");
                    assert!(
                        reordered > 0,
                        "native B-frame timestamps were not reordered"
                    );
                    eprintln!(
                        "synthetic B-frame video: frames={count}, B frames={b_count}, timestamp reversals={reordered}"
                    );
                }
                return Ok(());
            }
        }
        panic!("synthetic video EOF not reached within its bounded frame budget");
    }
}

fn has_b_slice(data: &[u8]) -> bool {
    // Test-only Annex B scan of bounded native H.264 samples. Both start-code
    // lengths have the same final 00 00 01 suffix; no packet data is rewritten.
    let mut from = 0;
    let mut found = false;
    while let Some(prefix) = data[from..].windows(3).position(|b| b == [0, 0, 1]) {
        let start = from + prefix + 3;
        let end = data[start..]
            .windows(3)
            .position(|b| b == [0, 0, 1])
            .map_or(data.len(), |offset| start + offset);
        if let Some(header) = data.get(start) {
            assert_eq!(header & 0x80, 0, "synthetic forbidden NAL bit");
            if matches!(header & 31, 1 | 5) {
                let mut bit = 0;
                let bytes = &data[start + 1..end];
                let first_mb =
                    fixture_exp_golomb(bytes, &mut bit).expect("synthetic first macroblock");
                // Fixed 128x96 fixture has only 48 macroblocks. Its initial two
                // UE fields cannot contain an emulation-prevention sequence.
                assert!(first_mb < 48);
                let kind = fixture_exp_golomb(bytes, &mut bit).expect("synthetic slice type");
                assert!(kind <= 9);
                found |= header & 31 == 1 && matches!(kind, 1 | 6); // B; second range is all slices.
            }
        }
        from = end;
    }
    found
}
fn fixture_exp_golomb(data: &[u8], bit: &mut usize) -> Option<u32> {
    let mut read = || {
        let byte = *data.get(*bit / 8)?;
        let value = (byte >> (7 - (*bit % 8))) & 1;
        *bit += 1;
        Some(value)
    };
    let mut zeros = 0;
    while read()? == 0 {
        zeros += 1;
        if zeros > 5 {
            return None;
        } // At most 48 macroblocks in this fixture.
    }
    let mut value = 1u32;
    for _ in 0..zeros {
        value = (value << 1) | u32::from(read()?);
    }
    Some(value - 1)
}

fn as_adts(bytes: &IMFByteStream, rate: u32, channels: u16) -> Result<Vec<u8>> {
    // SAFETY: this fixture's finalized, bounded MP4 is in its live RAM stream.
    // Same-thread interfaces remain alive; select raw AAC without a decoder.
    unsafe {
        let attributes: IMFAttributes = bytes.cast()?;
        attributes.SetString(
            &MF_BYTESTREAM_CONTENT_TYPE,
            &windows::core::HSTRING::from("audio/mp4"),
        )?;
        let reader = MFCreateSourceReaderFromByteStream(bytes, None)?;
        reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)?;
        reader.SetStreamSelection(MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32, true)?;
        let native = reader.GetNativeMediaType(MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32, 0)?;
        assert_eq!(native.GetGUID(&MF_MT_SUBTYPE)?, MFAudioFormat_AAC);
        assert_eq!(native.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND)?, rate);
        assert_eq!(
            native.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS)?,
            u32::from(channels)
        );
        reader.SetCurrentMediaType(MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32, None, &native)?;
        let mut result = Vec::new();
        let rate_index = if rate == 48_000 { 3 } else { 4 };
        // <=16 seconds /1024 frames, plus codec padding and empty ticks.
        for _ in 0..1024 {
            let mut flags = 0;
            let mut sample = None;
            reader.ReadSample(
                MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32,
                0,
                None,
                Some(&mut flags),
                None,
                Some(&mut sample),
            )?;
            assert_eq!(
                flags
                    & (MF_SOURCE_READERF_ERROR.0 as u32
                        | MF_SOURCE_READERF_NATIVEMEDIATYPECHANGED.0 as u32
                        | MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0 as u32),
                0
            );
            if let Some(sample) = sample {
                let buffer = sample.ConvertToContiguousBuffer()?;
                let mut pointer = std::ptr::null_mut();
                let mut maximum = 0;
                let mut length = 0;
                buffer.Lock(&mut pointer, Some(&mut maximum), Some(&mut length))?;
                let locked = Locked(&buffer);
                assert!(!pointer.is_null() && length > 0 && length <= maximum);
                let frame_length = length as usize + 7;
                assert!(
                    frame_length <= 8191
                        && result.len() + frame_length <= MAX_ENCODED_BYTES as usize
                );
                result.extend_from_slice(&[
                    0xff,
                    0xf1,
                    (1 << 6) | (rate_index << 2),
                    ((channels as u8) << 6) | ((frame_length >> 11) as u8),
                    (frame_length >> 3) as u8,
                    ((frame_length as u8 & 7) << 5) | 0x1f,
                    0xfc,
                ]);
                // SAFETY: successful Lock covers length initialized AAC bytes;
                // copying happens before the guard unlocks the native buffer.
                result.extend_from_slice(std::slice::from_raw_parts(pointer, length as usize));
                drop(locked);
            }
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                assert!(!result.is_empty());
                return Ok(result);
            }
        }
        panic!("bounded synthetic AAC packet extraction exceeded frame limit");
    }
}

#[cfg(test)]
mod b_slice_tests {
    use super::*;
    #[test]
    fn native_fixture_b_slice_probe_checks_nal_and_both_slice_type_ranges() {
        for prefix in [&[0, 0, 1][..], &[0, 0, 0, 1][..]] {
            for (header, body, expected) in [
                (1, 0xa0, true),
                (1, 0x9c, true),
                (0x61, 0xc0, false),
                (0x61, 0x98, false),
                (0x65, 0xb0, false),
                (0x65, 0x89, false),
                (0x67, 0xa0, false),
                (0x65, 0xa0, false),
            ] {
                let data = [prefix, &[header, body]].concat();
                assert_eq!(has_b_slice(&data), expected);
            }
        }
        assert!(has_b_slice(&[0, 0, 0, 1, 0x67, 0xa0, 0, 0, 1, 1, 0xa0]));
        assert!(!has_b_slice(&[1, 0xa0])); // No Annex B packet boundary.
        assert!(!has_b_slice(&[0, 0, 1]));
    }
    #[test]
    fn native_fixture_slice_prefix_is_bounded_and_refuses_truncation() {
        for data in [&[][..], &[0][..], &[0; 32][..]] {
            assert_eq!(fixture_exp_golomb(data, &mut 0), None);
        }
        assert_eq!(fixture_exp_golomb(&[0xa0], &mut 0), Some(0));
        let mut bit = 1;
        assert_eq!(fixture_exp_golomb(&[0xa0], &mut bit), Some(1));
        for data in [[0, 0, 1, 1, 0], [0, 0, 1, 0x81, 0xa0]] {
            assert!(std::panic::catch_unwind(|| has_b_slice(&data)).is_err());
        }
    }
}
