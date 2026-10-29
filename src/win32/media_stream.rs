//! Read-only COM stream over the already selected, pinned local file. No path,
//! network access, output file or whole-media allocation reaches this adapter.
use crate::{media::MediaError, resolver::contracts::ResolverCancellation};
use std::{
    ffi::c_void,
    fs::File,
    io::{ErrorKind, Read, Seek, SeekFrom},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Mutex},
};
use windows::{
    Win32::{
        Foundation::{
            E_ABORT, E_FAIL, E_INVALIDARG, S_FALSE, S_OK, STG_E_ACCESSDENIED,
            STG_E_INVALIDFUNCTION, STG_E_INVALIDPOINTER, STG_E_READFAULT,
        },
        System::Com::{
            ISequentialStream_Impl, IStream, IStream_Impl, LOCKTYPE, STATFLAG, STATSTG, STGC,
            STGM_READ, STGTY_STREAM, STREAM_SEEK, STREAM_SEEK_CUR, STREAM_SEEK_END,
            STREAM_SEEK_SET,
        },
    },
    core::{Error, HRESULT, Ref, implement},
};

/// An individual native read may not monopolize the worker with arbitrary work.
/// Supported handlers receive a complete request or a real EOF short read; this
/// limit is never implemented by presenting an artificial EOF to the decoder.
const MAX_STREAM_READ_BYTES: usize = 1024 * 1024;
/// Check cancellation between short local reads without allocating a copy.
const STREAM_READ_CHUNK_BYTES: usize = 64 * 1024;
/// Interrupted reads are retryable but cannot keep one COM call alive forever.
const MAX_INTERRUPTED_READS: usize = 8;

pub(super) fn stream(file: File, cancel: ResolverCancellation) -> Result<IStream, MediaError> {
    if cancel.is_cancelled() {
        return Err(MediaError::Cancelled);
    }
    let metadata = file.metadata().map_err(|_| MediaError::InvalidAudio)?;
    if !metadata.is_file() {
        return Err(MediaError::InvalidAudio);
    }
    if cancel.is_cancelled() {
        return Err(MediaError::Cancelled);
    }
    Ok(FileStream {
        file: Arc::new(Mutex::new(file)),
        position: Mutex::new(0),
        byte_len: metadata.len(),
        cancel,
    }
    .into())
}

// The implement macro supplies IAgileObject and a free-threaded marshaler. MF
// may call this object on its own MTA worker; all mutable fields are synchronized.
// Clones share the pinned file, but never share their logical seek pointer.
#[implement(IStream)]
struct FileStream {
    file: Arc<Mutex<File>>,
    position: Mutex<u64>,
    byte_len: u64,
    cancel: ResolverCancellation,
}

impl FileStream {
    fn check_cancel(&self) -> Result<(), HRESULT> {
        if self.cancel.is_cancelled() {
            Err(E_ABORT)
        } else {
            Ok(())
        }
    }

    /// The COM caller owns the buffer and optional count for this synchronous
    /// call. No reference to their memory survives the method return.
    fn read_native(&self, pv: *mut c_void, cb: u32, pcbread: *mut u32) -> HRESULT {
        // SAFETY: native callers provide either null or a writable, aligned u32.
        unsafe { write_optional(pcbread, 0) };
        let result = (|| {
            self.check_cancel()?;
            if cb as usize > MAX_STREAM_READ_BYTES {
                return Err(STG_E_INVALIDFUNCTION);
            }
            if cb == 0 {
                self.check_cancel()?;
                return Ok(true);
            }
            if pv.is_null() {
                return Err(STG_E_INVALIDPOINTER);
            }
            // SAFETY: the ISequentialStream contract gives this method exclusive
            // access to cb writable bytes for the call. Null/oversized requests
            // were rejected; cb <= 1 MiB fits isize. The slice stays local.
            let buffer = unsafe { std::slice::from_raw_parts_mut(pv.cast::<u8>(), cb as usize) };
            // All operations acquire position before the shared file. Seek and
            // read must be atomic relative to every independently positioned clone.
            let mut position = self.position.lock().map_err(|_| E_FAIL)?;
            let mut file = self.file.lock().map_err(|_| E_FAIL)?;
            self.check_cancel()?;
            file.seek(SeekFrom::Start(*position))
                .map_err(|_| STG_E_READFAULT)?;
            self.check_cancel()?;
            let mut total = 0;
            while total < buffer.len() {
                self.check_cancel()?;
                let end = (total + STREAM_READ_CHUNK_BYTES).min(buffer.len());
                let mut interruptions = 0;
                let read = loop {
                    self.check_cancel()?;
                    match file.read(&mut buffer[total..end]) {
                        Ok(read) => break read,
                        Err(error) if error.kind() == ErrorKind::Interrupted => {
                            interruptions += 1;
                            if interruptions >= MAX_INTERRUPTED_READS {
                                return Err(STG_E_READFAULT);
                            }
                        }
                        Err(_) => return Err(STG_E_READFAULT),
                    }
                };
                total += read;
                *position = position.checked_add(read as u64).ok_or(STG_E_READFAULT)?;
                // SAFETY: same optional output as at entry; total <= bounded cb.
                unsafe { write_optional(pcbread, total as u32) };
                self.check_cancel()?;
                if read == 0 {
                    return Ok(false);
                }
            }
            Ok(true)
        })();
        match result {
            Ok(true) => S_OK,
            Ok(false) => S_FALSE,
            Err(code) => code,
        }
    }

    fn seek_native(
        &self,
        delta: i64,
        origin: STREAM_SEEK,
        output: *mut u64,
    ) -> windows::core::Result<()> {
        // SAFETY: the COM output is nullable or valid for one aligned u64 write.
        unsafe { write_optional(output, 0) };
        self.check_cancel().map_err(Error::from_hresult)?;
        let mut position = self
            .position
            .lock()
            .map_err(|_| Error::from_hresult(E_FAIL))?;
        let base = match origin {
            STREAM_SEEK_SET => 0,
            STREAM_SEEK_CUR => *position,
            STREAM_SEEK_END => self.byte_len,
            _ => return Err(Error::from_hresult(STG_E_INVALIDFUNCTION)),
        };
        // Restrict this local-file adapter to Windows' signed file-offset range.
        // Seeking beyond EOF is valid; seeking before zero or overflowing is not.
        let next = i128::from(base) + i128::from(delta);
        if !(0..=i128::from(i64::MAX)).contains(&next) {
            return Err(Error::from_hresult(STG_E_INVALIDFUNCTION));
        }
        self.check_cancel().map_err(Error::from_hresult)?;
        *position = next as u64;
        // SAFETY: nullable output remains borrowed only for this synchronous call.
        unsafe { write_optional(output, *position) };
        self.check_cancel().map_err(Error::from_hresult)
    }
}

impl ISequentialStream_Impl for FileStream_Impl {
    fn Read(&self, pv: *mut c_void, cb: u32, pcbread: *mut u32) -> HRESULT {
        abi_hresult(|| self.read_native(pv, cb, pcbread))
    }

    fn Write(&self, _pv: *const c_void, _cb: u32, pcbwritten: *mut u32) -> HRESULT {
        abi_hresult(|| {
            // SAFETY: optional COM output is null or writable; no input is read.
            unsafe { write_optional(pcbwritten, 0) };
            STG_E_ACCESSDENIED
        })
    }
}

impl IStream_Impl for FileStream_Impl {
    fn Seek(&self, delta: i64, origin: STREAM_SEEK, output: *mut u64) -> windows::core::Result<()> {
        abi_result(|| self.seek_native(delta, origin, output))
    }

    fn SetSize(&self, _size: u64) -> windows::core::Result<()> {
        abi_result(|| Err(Error::from_hresult(STG_E_ACCESSDENIED)))
    }

    fn CopyTo(
        &self,
        _target: Ref<IStream>,
        _cb: u64,
        read: *mut u64,
        written: *mut u64,
    ) -> windows::core::Result<()> {
        abi_result(|| {
            // SAFETY: optional caller-owned output counts are aligned and writable.
            unsafe {
                write_optional(read, 0);
                write_optional(written, 0);
            }
            // This private decoder input does not copy content to an output stream.
            Err(Error::from_hresult(STG_E_ACCESSDENIED))
        })
    }

    fn Commit(&self, _flags: &STGC) -> windows::core::Result<()> {
        abi_result(|| Err(Error::from_hresult(STG_E_INVALIDFUNCTION)))
    }

    fn Revert(&self) -> windows::core::Result<()> {
        abi_result(|| Err(Error::from_hresult(STG_E_INVALIDFUNCTION)))
    }

    fn LockRegion(&self, _offset: u64, _cb: u64, _kind: &LOCKTYPE) -> windows::core::Result<()> {
        abi_result(|| Err(Error::from_hresult(STG_E_INVALIDFUNCTION)))
    }

    fn UnlockRegion(&self, _offset: u64, _cb: u64, _kind: u32) -> windows::core::Result<()> {
        abi_result(|| Err(Error::from_hresult(STG_E_INVALIDFUNCTION)))
    }

    fn Stat(&self, output: *mut STATSTG, flags: &STATFLAG) -> windows::core::Result<()> {
        abi_result(|| {
            if output.is_null() {
                return Err(Error::from_hresult(STG_E_INVALIDPOINTER));
            }
            // SAFETY: the non-null COM output is caller-owned and writable for one
            // STATSTG. Default has no allocated name; initialized even on failure.
            unsafe { output.write(STATSTG::default()) };
            self.check_cancel().map_err(Error::from_hresult)?;
            if !(0..=2).contains(&flags.0) {
                return Err(Error::from_hresult(E_INVALIDARG));
            }
            let stat = STATSTG {
                r#type: STGTY_STREAM.0 as u32,
                cbSize: self.byte_len,
                grfMode: STGM_READ,
                ..Default::default()
            };
            self.check_cancel().map_err(Error::from_hresult)?;
            // SAFETY: same validated output; no path/name allocation is returned.
            unsafe { output.write(stat) };
            self.check_cancel().map_err(Error::from_hresult)
        })
    }

    fn Clone(&self) -> windows::core::Result<IStream> {
        abi_result(|| {
            self.check_cancel().map_err(Error::from_hresult)?;
            let position = *self
                .position
                .lock()
                .map_err(|_| Error::from_hresult(E_FAIL))?;
            self.check_cancel().map_err(Error::from_hresult)?;
            let clone: IStream = FileStream {
                file: self.file.clone(),
                position: Mutex::new(position),
                byte_len: self.byte_len,
                cancel: self.cancel.clone(),
            }
            .into();
            self.check_cancel().map_err(Error::from_hresult)?;
            Ok(clone)
        })
    }
}

/// No Rust panic is allowed to cross a generated extern-system COM vtable.
fn abi_hresult(operation: impl FnOnce() -> HRESULT) -> HRESULT {
    catch_unwind(AssertUnwindSafe(operation)).unwrap_or(E_FAIL)
}

fn abi_result<T>(operation: impl FnOnce() -> windows::core::Result<T>) -> windows::core::Result<T> {
    catch_unwind(AssertUnwindSafe(operation)).unwrap_or_else(|_| Err(Error::from_hresult(E_FAIL)))
}

/// Caller guarantees non-null outputs are aligned and valid for one write.
unsafe fn write_optional<T>(output: *mut T, value: T) {
    if !output.is_null() {
        // SAFETY: upheld by this private helper's native-caller contract.
        unsafe { output.write(value) };
    }
}

#[cfg(test)]
#[path = "media_stream_tests.rs"]
mod tests;
