//! Native local audio decoding. COM/MF objects never leave this bounded worker.
//! Pinned WAVE/MP3/AAC (ADTS or single-track MP4); no URLs or output files.
use crate::{
    ai::speech::StreamIdentity,
    media::{
        MediaError,
        decoded::{DecodedAudioBlock, DecodedAudioFormat, MAX_DECODED_BLOCK_BYTES, MediaDecoder},
        input::{MediaInput, OpenedMedia, open_media},
        local_audio_format::{LocalAudioFormat, probe},
    },
    resolver::contracts::ResolverCancellation,
};
use std::{marker::PhantomData, rc::Rc};
use windows::{
    Win32::{Media::MediaFoundation::*, System::Com::IStream},
    core::Interface,
};

/// The handler may supply more than one output block. Retain at most one native
/// sample of this size, and copy only a frame-aligned 64 KiB slice per pull.
const MAX_NATIVE_SAMPLE_BYTES: usize = 1024 * 1024;
/// Empty ticks are legal, but a broken source must not spin without progress.
const MAX_EMPTY_SAMPLES: usize = 32;
const MICROSECONDS_PER_SECOND: u64 = 1_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReadDisposition {
    Sample { ended: bool },
    Empty,
    End,
}
/// END can accompany a final sample; deliver its bounded pieces before EOF.
/// Errors or changed formats reject that sample rather than interpreting it
/// using the previously negotiated PCM layout.
fn read_disposition(flags: u32, has_sample: bool) -> Result<ReadDisposition, MediaError> {
    if flags & MF_SOURCE_READERF_ERROR.0 as u32 != 0 {
        return Err(MediaError::InvalidAudio);
    }
    let changed = MF_SOURCE_READERF_NATIVEMEDIATYPECHANGED.0 as u32
        | MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0 as u32
        | MF_SOURCE_READERF_NEWSTREAM.0 as u32;
    if flags & changed != 0 {
        return Err(MediaError::UnsupportedMedia);
    }
    let ended = flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0;
    Ok(if has_sample {
        ReadDisposition::Sample { ended }
    } else if ended {
        ReadDisposition::End
    } else {
        ReadDisposition::Empty
    })
}

struct Foundation(PhantomData<Rc<()>>);
impl Foundation {
    fn new() -> Result<Self, MediaError> {
        // SAFETY: COM MTA was initialized on this worker. One successful startup
        // (including no-socket LITE) is paired with Shutdown on this same thread.
        unsafe { MFStartup(MF_VERSION, MFSTARTUP_LITE) }
            .map_err(|_| MediaError::DecoderUnavailable)?;
        Ok(Self(PhantomData))
    }
}
impl Drop for Foundation {
    fn drop(&mut self) {
        // SAFETY: reader, buffer and byte-stream fields/local bindings were
        // released first. The guard is !Send and its COM apartment is still alive.
        unsafe {
            let _ = MFShutdown();
        }
    }
}
struct Pending {
    buffer: IMFMediaBuffer,
    timestamp_us: u64,
    length: usize,
    offset: usize,
}
/// Field order releases sample -> reader -> streams -> MF -> COM. The guards
/// make this owner !Send/!Sync, including when used behind MediaDecoder.
pub(crate) struct NativeMediaDecoder {
    pending: Option<Pending>,
    reader: IMFSourceReader,
    _byte_stream: IMFByteStream,
    _stream: IStream,
    identity: StreamIdentity,
    format: DecodedAudioFormat,
    packet_clock: bool,
    sequence: u64,
    ended: bool,
    failed: bool,
    stream_index: Option<u32>,
    cancellation: ResolverCancellation,
    _foundation: Foundation,
    _apartment: super::resources::Apartment,
}

pub(crate) fn open(
    input: MediaInput,
    identity: StreamIdentity,
    cancel: ResolverCancellation,
) -> Result<NativeMediaDecoder, MediaError> {
    open_native_pinned(input, identity, cancel).map(|(decoder, _media)| decoder)
}

/// The public borrowed DSP path requires the exact PCM frame clock. MP3 packet
/// clocks need the private owned actor's chunk maps; reject before PCM here.
pub(crate) fn open_prepared_audio(
    input: MediaInput,
    identity: StreamIdentity,
    cancel: ResolverCancellation,
) -> Result<NativeMediaDecoder, MediaError> {
    open_pinned(input, identity, cancel).map(|(decoder, _media)| decoder)
}

/// Open the selected path once. The decoder consumes a duplicate of the same
/// readonly file object; the original pin can outlive COM/MF destruction.
pub(crate) fn open_pinned(
    input: MediaInput,
    identity: StreamIdentity,
    cancel: ResolverCancellation,
) -> Result<(NativeMediaDecoder, OpenedMedia), MediaError> {
    open_checked(input, identity, cancel, ClockRequirement::SampleAccurate)
}
pub(crate) fn open_native_pinned(
    input: MediaInput,
    identity: StreamIdentity,
    cancel: ResolverCancellation,
) -> Result<(NativeMediaDecoder, OpenedMedia), MediaError> {
    open_checked(input, identity, cancel, ClockRequirement::Native)
}
enum ClockRequirement {
    Native,
    SampleAccurate,
}
fn open_checked(
    input: MediaInput,
    identity: StreamIdentity,
    cancel: ResolverCancellation,
    clock: ClockRequirement,
) -> Result<(NativeMediaDecoder, OpenedMedia), MediaError> {
    check(&cancel)?;
    if identity.session.0 == 0 || identity.generation == 0 {
        return Err(MediaError::InvalidAudio);
    }
    let media = open_media(&input, &cancel)?;
    let mut file = media.try_clone_file(&cancel)?;
    let container = probe(&mut file, media.byte_len(), &cancel)?;
    if matches!(clock, ClockRequirement::SampleAccurate) && container.uses_packet_clock() {
        return Err(MediaError::UnsupportedAudioClock);
    }
    check(&cancel)?;
    let apartment =
        super::resources::Apartment::new().map_err(|_| MediaError::DecoderUnavailable)?;
    let foundation = Foundation::new()?;
    let stream = super::media_stream::stream(file, cancel.clone())?;
    // SAFETY: pinned readonly IStream is alive, agile callbacks synchronize its
    // logical cursor and File. All COM owner interfaces remain on this worker.
    let byte_stream = native(unsafe { MFCreateMFByteStreamOnStream(&stream) }, &cancel)?;
    let attributes: IMFAttributes = native(byte_stream.cast(), &cancel)?;
    // SAFETY: trusted MIME comes only from the bounded content preflight, never a
    // private pathname or untrusted URL. The API copies this terminated literal.
    native(
        unsafe {
            attributes.SetString(
                &MF_BYTESTREAM_CONTENT_TYPE,
                &windows::core::HSTRING::from(container.mime()),
            )
        },
        &cancel,
    )?;
    // SAFETY: no async callback, no URL/origin attribute, and no renderer. Reader
    // owns the referenced byte stream; native blocking stays off UI/capture.
    let reader = native(
        unsafe { MFCreateSourceReaderFromByteStream(&byte_stream, None) },
        &cancel,
    )?;
    // SAFETY: only first audio is selected; video/other streams cannot produce
    // output. Indices are documented Source Reader selectors, not HWND/PIDs.
    native(
        unsafe { reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false) },
        &cancel,
    )?;
    native(
        unsafe { reader.SetStreamSelection(MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32, true) },
        &cancel,
    )?;
    let expected_aac = if matches!(container, LocalAudioFormat::Adts | LocalAudioFormat::Mp4) {
        Some(aac_native_format(
            &reader,
            container == LocalAudioFormat::Mp4,
            &cancel,
        )?)
    } else {
        None
    };
    // Partial media type requests PCM16 only. Do not assume resampling/mono:
    // inspect the actual negotiated rate/channels/block alignment afterward.
    let pcm = native(unsafe { MFCreateMediaType() }, &cancel)?;
    // SAFETY: setters mutate our live media-type interface and copy fixed GUIDs.
    unsafe {
        native(pcm.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio), &cancel)?;
        native(pcm.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM), &cancel)?;
        native(pcm.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16), &cancel)?;
        native(
            reader.SetCurrentMediaType(MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32, None, &pcm),
            &cancel,
        )?;
    }
    let format = current_format(&reader, &cancel)?;
    if expected_aac.is_some_and(|expected| expected != format) {
        return Err(MediaError::UnsupportedMedia);
    }
    check(&cancel)?;
    Ok((
        NativeMediaDecoder {
            pending: None,
            reader,
            _byte_stream: byte_stream,
            _stream: stream,
            identity,
            format,
            packet_clock: container.uses_packet_clock(),
            sequence: 1,
            ended: false,
            failed: false,
            stream_index: None,
            cancellation: cancel,
            _foundation: foundation,
            _apartment: apartment,
        },
        media,
    ))
}
fn aac_native_format(
    reader: &IMFSourceReader,
    mp4: bool,
    cancel: &ResolverCancellation,
) -> Result<DecodedAudioFormat, MediaError> {
    // SAFETY: same-thread live Source Reader; native media type owns its
    // reference. Getters copy scalars and the bounded owned blob below.
    unsafe {
        let media = native(
            reader.GetNativeMediaType(MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32, 0),
            cancel,
        )?;
        if native(media.GetGUID(&MF_MT_MAJOR_TYPE), cancel)? != MFMediaType_Audio
            || native(media.GetGUID(&MF_MT_SUBTYPE), cancel)? != MFAudioFormat_AAC
        {
            return Err(MediaError::UnsupportedMedia);
        }
        let rate = native(media.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND), cancel)?;
        let channels = native(media.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS), cancel)?;
        if !matches!(rate, 44_100 | 48_000) || !matches!(channels, 1 | 2) {
            return Err(MediaError::UnsupportedMedia);
        }
        if mp4 {
            // SAFETY: native getter writes into our fixed stack buffer and
            // copies at most its slice length. Size and returned length must
            // match HEAACWAVEINFO's 12-byte tail plus the supported 2-byte ASC.
            if native(media.GetBlobSize(&MF_MT_USER_DATA), cancel)? != 14 {
                return Err(MediaError::UnsupportedMedia);
            }
            let mut data = [0u8; 14];
            let mut length = 0;
            native(
                media.GetBlob(&MF_MT_USER_DATA, &mut data, Some(&mut length)),
                cancel,
            )?;
            let rate_index = if rate == 48_000 { 3 } else { 4 };
            let expected = (2u16 << 11) | (rate_index << 7) | ((channels as u16) << 3);
            if length != 14 || data[..2] != [0, 0] || data[12..] != expected.to_be_bytes() {
                return Err(MediaError::UnsupportedMedia);
            }
        }
        // ADTS coalesced 44.1 kHz samples exceeded the 1 us frame gate (43 hns
        // ticks of native duration overlap). MP4's own timestamps are separately
        // exercised at both rates; keep the restriction specific to ADTS.
        if !mp4 && rate != 48_000 {
            return Err(MediaError::UnsupportedAudioClock);
        }
        DecodedAudioFormat::new(rate, channels as u16)
    }
}
fn current_format(
    reader: &IMFSourceReader,
    cancel: &ResolverCancellation,
) -> Result<DecodedAudioFormat, MediaError> {
    // SAFETY: same-thread live reader/type; native getters write owned scalar
    // return values, not pointers into files or across threads.
    unsafe {
        let media = native(
            reader.GetCurrentMediaType(MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32),
            cancel,
        )?;
        if native(media.GetGUID(&MF_MT_MAJOR_TYPE), cancel)? != MFMediaType_Audio
            || native(media.GetGUID(&MF_MT_SUBTYPE), cancel)? != MFAudioFormat_PCM
            || native(media.GetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE), cancel)? != 16
        {
            return Err(MediaError::UnsupportedMedia);
        }
        let channels = native(media.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS), cancel)?;
        let rate = native(media.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND), cancel)?;
        if native(media.GetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT), cancel)?
            != channels.checked_mul(2).ok_or(MediaError::InvalidAudio)?
        {
            return Err(MediaError::InvalidAudio);
        }
        DecodedAudioFormat::new(
            rate,
            u16::try_from(channels).map_err(|_| MediaError::InvalidAudio)?,
        )
    }
}
impl MediaDecoder for NativeMediaDecoder {
    fn format(&self) -> DecodedAudioFormat {
        self.format
    }
    fn next(
        &mut self,
        cancel: &ResolverCancellation,
    ) -> Result<Option<DecodedAudioBlock>, MediaError> {
        check(cancel)?;
        check(&self.cancellation)?;
        if self.failed {
            return Err(MediaError::InvalidAudio);
        }
        let result = self.next_inner(cancel);
        if result.is_err() {
            self.failed = true;
            self.pending = None;
        }
        result
    }
}
impl NativeMediaDecoder {
    /// Only content-probed compressed input requires a separate sample clock.
    pub(crate) const fn uses_packet_clock(&self) -> bool {
        self.packet_clock
    }
    fn next_inner(
        &mut self,
        cancel: &ResolverCancellation,
    ) -> Result<Option<DecodedAudioBlock>, MediaError> {
        if self.pending.is_some() {
            return self.take_block(cancel).map(Some);
        }
        if self.ended {
            return Ok(None);
        }
        for _ in 0..MAX_EMPTY_SAMPLES {
            check(cancel)?;
            check(&self.cancellation)?;
            let mut sample = None;
            let mut flags = 0u32;
            let mut timestamp = 0i64;
            let mut index = 0u32;
            // SAFETY: writable initialized scalar/Option outputs live through
            // this synchronous call. Reader is used only on its owning worker.
            let result = unsafe {
                self.reader.ReadSample(
                    MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32,
                    0,
                    Some(&mut index),
                    Some(&mut flags),
                    Some(&mut timestamp),
                    Some(&mut sample),
                )
            };
            check(cancel)?;
            native(result, &self.cancellation)?;
            match read_disposition(flags, sample.is_some())? {
                ReadDisposition::End => {
                    self.ended = true;
                    return Ok(None);
                }
                ReadDisposition::Empty => {
                    // Empty samples/ticks do not manufacture PCM or EOF. Future
                    // blocks retain their actual timestamps so the gate sees gaps.
                    continue;
                }
                ReadDisposition::Sample { ended } => self.ended = ended,
            }
            let sample = sample.ok_or(MediaError::InvalidAudio)?;
            if self.stream_index.is_some_and(|previous| previous != index) {
                return Err(MediaError::InvalidAudio);
            }
            self.stream_index = Some(index);
            let timestamp_us = u64::try_from(timestamp).map_err(|_| MediaError::InvalidAudio)? / 10;
            // SAFETY: query length before contiguous-buffer allocation; reject
            // pathological native samples without allocating app PCM for them.
            let length = native(unsafe { sample.GetTotalLength() }, &self.cancellation)? as usize;
            let frame_bytes = usize::from(self.format.channels()) * 2;
            if length == 0
                || length > MAX_NATIVE_SAMPLE_BYTES
                || !length.is_multiple_of(frame_bytes)
            {
                return Err(MediaError::InvalidAudio);
            }
            // SAFETY: sample owns valid native buffers; result is independently
            // reference-counted and retained only for bounded sliced pulls.
            let buffer = native(
                unsafe { sample.ConvertToContiguousBuffer() },
                &self.cancellation,
            )?;
            self.pending = Some(Pending {
                buffer,
                timestamp_us,
                length,
                offset: 0,
            });
            return self.take_block(cancel).map(Some);
        }
        Err(MediaError::InvalidAudio)
    }
    fn take_block(
        &mut self,
        cancel: &ResolverCancellation,
    ) -> Result<DecodedAudioBlock, MediaError> {
        check(cancel)?;
        check(&self.cancellation)?;
        let pending = self.pending.as_mut().ok_or(MediaError::InvalidAudio)?;
        let channels = usize::from(self.format.channels());
        let frame_bytes = channels * 2;
        let remaining = pending
            .length
            .checked_sub(pending.offset)
            .ok_or(MediaError::InvalidAudio)?;
        let bytes = remaining.min(MAX_DECODED_BLOCK_BYTES / frame_bytes * frame_bytes);
        if bytes == 0 {
            return Err(MediaError::InvalidAudio);
        }
        let mut data = std::ptr::null_mut();
        let mut maximum = 0u32;
        let mut current = 0u32;
        // SAFETY: buffer is live on this worker. Native Lock initializes the
        // pointer/lengths. The guard immediately owns Unlock on all exits.
        let lock_result = unsafe {
            pending
                .buffer
                .Lock(&mut data, Some(&mut maximum), Some(&mut current))
        };
        if let Err(error) = lock_result {
            return native(Err(error), &self.cancellation);
        }
        struct Unlock<'a>(&'a IMFMediaBuffer, bool);
        impl Unlock<'_> {
            fn release(mut self) -> windows::core::Result<()> {
                // SAFETY: one successful Lock is paired with this explicit
                // Unlock; disarm Drop even on native error to avoid a double call.
                let result = unsafe { self.0.Unlock() };
                self.1 = false;
                result
            }
        }
        impl Drop for Unlock<'_> {
            fn drop(&mut self) {
                // SAFETY: matches this successful Lock before any buffer drop.
                if self.1 {
                    unsafe {
                        let _ = self.0.Unlock();
                    }
                }
            }
        }
        let locked = Unlock(&pending.buffer, true);
        // Install Unlock before observing cancellation: Lock may have succeeded
        // while the caller stopped, and that buffer still requires release.
        check(cancel)?;
        check(&self.cancellation)?;
        if data.is_null()
            || current > maximum
            || current as usize != pending.length
            || pending
                .offset
                .checked_add(bytes)
                .is_none_or(|end| end > current as usize)
        {
            return Err(MediaError::InvalidAudio);
        }
        // SAFETY: nonnull locked pointer covers current bytes, checked offset
        // and length fit it. Decode pairs rather than assuming i16 alignment.
        let slice = unsafe { std::slice::from_raw_parts(data.add(pending.offset), bytes) };
        let mut samples = Vec::new();
        samples
            .try_reserve_exact(bytes / 2)
            .map_err(|_| MediaError::Overloaded)?;
        samples.extend(
            slice
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| i16::from_le_bytes(*pair)),
        );
        native(locked.release(), &self.cancellation)?;
        check(cancel)?;
        check(&self.cancellation)?;
        let frame_offset = pending.offset / frame_bytes;
        let offset_us = (frame_offset as u64)
            .checked_mul(MICROSECONDS_PER_SECOND)
            .ok_or(MediaError::InvalidAudio)?
            / u64::from(self.format.sample_rate());
        let timestamp = pending
            .timestamp_us
            .checked_add(offset_us)
            .ok_or(MediaError::InvalidAudio)?;
        let block = DecodedAudioBlock::new(
            self.identity,
            self.sequence,
            timestamp,
            self.format,
            samples,
        )?;
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or(MediaError::InvalidAudio)?;
        pending.offset += bytes;
        if pending.offset == pending.length {
            self.pending = None;
        }
        Ok(block)
    }
}
fn check(cancel: &ResolverCancellation) -> Result<(), MediaError> {
    if cancel.is_cancelled() {
        Err(MediaError::Cancelled)
    } else {
        Ok(())
    }
}
fn native<T>(
    result: windows::core::Result<T>,
    cancel: &ResolverCancellation,
) -> Result<T, MediaError> {
    check(cancel)?;
    result.map_err(|error| match error.code() {
        MF_E_LICENSE_REQUIRED
        | MF_E_LICENSE_INCORRECT_RIGHTS
        | MF_E_UNSUPPORTED_CONTENT_PROTECTION_SYSTEM
        | MF_E_CONTENT_PROTECTION_SYSTEM_NOT_ENABLED => MediaError::ProtectedMedia,
        MF_E_UNSUPPORTED_BYTESTREAM_TYPE | MF_E_INVALIDMEDIATYPE | MF_E_TOPO_CODEC_NOT_FOUND => {
            MediaError::UnsupportedMedia
        }
        _ => MediaError::InvalidAudio,
    })
}

#[cfg(test)]
#[path = "media_adts_tests.rs"]
mod adts_tests;
#[cfg(test)]
#[path = "media_mp3_chunk_clock_tests.rs"]
mod mp3_chunk_clock_tests;
#[cfg(test)]
#[path = "media_mp3_clock_tests.rs"]
mod mp3_clock_tests;
#[cfg(test)]
#[path = "media_mp3_tests.rs"]
mod mp3_tests;
#[cfg(test)]
#[path = "media_mp4_tests.rs"]
mod mp4_tests;
#[cfg(test)]
#[path = "media_decode_tests.rs"]
mod tests;
