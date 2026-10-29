//! Direct COM calls over owned synthetic bytes; these are not recorded media.
use super::*;
use std::{fs, os::windows::fs::OpenOptionsExt, path::PathBuf, thread};
use tempfile::TempDir;
use windows::{
    Win32::{
        Storage::FileSystem::FILE_SHARE_READ,
        System::Com::{STATFLAG_DEFAULT, STATFLAG_NONAME, STGC_DEFAULT},
    },
    core::AgileReference,
};

fn fixture(body: &[u8]) -> (TempDir, PathBuf, IStream, ResolverCancellation) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("synthetic-pinned-stream.bin");
    fs::write(&path, body).unwrap();
    let file = fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ.0)
        .open(&path)
        .unwrap();
    let cancel = ResolverCancellation::default();
    let input = stream(file, cancel.clone()).unwrap();
    (directory, path, input, cancel)
}

fn native_read(input: &IStream, bytes: &mut [u8]) -> (HRESULT, u32) {
    let mut count = u32::MAX;
    // SAFETY: slice is writable for precisely its length, output is an aligned
    // live u32, and neither reference survives this synchronous COM call.
    let status = unsafe {
        input.Read(
            bytes.as_mut_ptr().cast(),
            bytes.len().try_into().unwrap(),
            Some(&mut count),
        )
    };
    (status, count)
}

fn native_seek(input: &IStream, offset: i64, origin: STREAM_SEEK) -> windows::core::Result<u64> {
    let mut position = u64::MAX;
    // SAFETY: owned COM object and a live aligned output for this call.
    unsafe { input.Seek(offset, origin, Some(&mut position)) }?;
    Ok(position)
}

fn error_code<T>(result: windows::core::Result<T>) -> HRESULT {
    match result {
        Err(error) => error.code(),
        Ok(_) => panic!("expected typed native failure"),
    }
}

#[test]
fn pinned_read_fills_chunks_and_reports_only_real_eof_without_modifying_file() {
    let _apartment = super::super::resources::Apartment::new().unwrap();
    let body: Vec<_> = (0..(STREAM_READ_CHUNK_BYTES * 2 + 13))
        .map(|index| (index % 251) as u8)
        .collect();
    let (_directory, path, input, _) = fixture(&body);
    let mut output = vec![0; STREAM_READ_CHUNK_BYTES * 2];
    assert_eq!(
        native_read(&input, &mut output),
        (S_OK, output.len() as u32)
    );
    assert_eq!(output, body[..output.len()]);
    let mut tail = [0xee; 19];
    assert_eq!(native_read(&input, &mut tail), (S_FALSE, 13));
    assert_eq!(&tail[..13], &body[output.len()..]);
    assert_eq!(&tail[13..], &[0xee; 6]);
    assert_eq!(native_read(&input, &mut tail), (S_FALSE, 0));
    assert_eq!(fs::read(&path).unwrap(), body);
}

#[test]
fn invalid_read_requests_clear_count_without_touching_caller_memory() {
    let _apartment = super::super::resources::Apartment::new().unwrap();
    let (_directory, _path, input, _) = fixture(b"read-only synthetic bytes");
    let mut count = 999;
    // SAFETY: the documented zero-byte case permits null; a nonzero null request
    // is deliberately rejected before the implementation constructs any slice.
    unsafe {
        assert_eq!(input.Read(std::ptr::null_mut(), 0, Some(&mut count)), S_OK);
        assert_eq!(count, 0);
        count = 999;
        assert_eq!(
            input.Read(std::ptr::null_mut(), 1, Some(&mut count)),
            STG_E_INVALIDPOINTER
        );
        assert_eq!(count, 0);
        assert_eq!(input.Read(std::ptr::null_mut(), 0, None), S_OK);
    }
    let mut oversized = vec![0xa5; MAX_STREAM_READ_BYTES + 1];
    assert_eq!(
        native_read(&input, &mut oversized),
        (STG_E_INVALIDFUNCTION, 0)
    );
    assert!(oversized.iter().all(|byte| *byte == 0xa5));
    let mut first = [0; 4];
    assert_eq!(native_read(&input, &mut first), (S_OK, 4));
    assert_eq!(&first, b"read");
}

#[test]
fn seek_supports_relative_end_and_past_eof_but_rejects_invalid_offsets() {
    let _apartment = super::super::resources::Apartment::new().unwrap();
    let (_directory, _path, input, _) = fixture(b"0123456789");
    assert_eq!(native_seek(&input, 4, STREAM_SEEK_SET).unwrap(), 4);
    assert_eq!(native_seek(&input, -2, STREAM_SEEK_CUR).unwrap(), 2);
    let mut output = [0; 3];
    assert_eq!(native_read(&input, &mut output), (S_OK, 3));
    assert_eq!(&output, b"234");
    assert_eq!(native_seek(&input, -1, STREAM_SEEK_END).unwrap(), 9);
    assert_eq!(native_seek(&input, 17, STREAM_SEEK_END).unwrap(), 27);
    assert_eq!(native_read(&input, &mut output), (S_FALSE, 0));
    for (offset, origin) in [
        (-1, STREAM_SEEK_SET),
        (-28, STREAM_SEEK_CUR),
        (-11, STREAM_SEEK_END),
        (i64::MAX, STREAM_SEEK_CUR),
        (0, STREAM_SEEK(7)),
    ] {
        assert_eq!(
            error_code(native_seek(&input, offset, origin)),
            STG_E_INVALIDFUNCTION
        );
        assert_eq!(native_seek(&input, 0, STREAM_SEEK_CUR).unwrap(), 27);
    }
    // SAFETY: optional seek output is null and the owned stream stays live.
    unsafe { input.Seek(0, STREAM_SEEK_SET, None) }.unwrap();
    assert_eq!(native_seek(&input, 0, STREAM_SEEK_CUR).unwrap(), 0);
}

#[test]
fn stat_returns_bounded_size_and_no_path_or_allocated_name() {
    let _apartment = super::super::resources::Apartment::new().unwrap();
    let (_directory, _path, input, _) = fixture(b"opaque bytes");
    let mut stat = STATSTG::default();
    // SAFETY: every non-null output points at a live writable STATSTG. The null
    // case verifies the adapter rejects it without dereferencing it.
    unsafe {
        for flags in [STATFLAG_DEFAULT, STATFLAG_NONAME, STATFLAG(2)] {
            input.Stat(&mut stat, flags).unwrap();
            assert_eq!(stat.cbSize, 12);
            assert_eq!(stat.r#type, STGTY_STREAM.0 as u32);
            assert_eq!(stat.grfMode, STGM_READ);
            assert!(stat.pwcsName.is_null());
        }
        assert_eq!(
            error_code(input.Stat(&mut stat, STATFLAG(99))),
            E_INVALIDARG
        );
        assert!(stat.pwcsName.is_null());
        assert_eq!(stat.cbSize, 0);
        assert_eq!(
            error_code(input.Stat(std::ptr::null_mut(), STATFLAG_DEFAULT)),
            STG_E_INVALIDPOINTER
        );
    }
}

#[test]
fn mutation_and_output_methods_fail_and_never_change_input() {
    let _apartment = super::super::resources::Apartment::new().unwrap();
    let (_directory, path, input, _) = fixture(b"never modified");
    let mut written = 999;
    let mut copied_read = 999;
    let mut copied_written = 999;
    // SAFETY: live input bytes, owned COM object, and aligned optional counts;
    // denied operations do not read or retain any caller buffers.
    unsafe {
        assert_eq!(
            input.Write(b"x".as_ptr().cast(), 1, Some(&mut written)),
            STG_E_ACCESSDENIED
        );
        assert_eq!(written, 0);
        assert_eq!(input.Write(std::ptr::null(), 0, None), STG_E_ACCESSDENIED);
        assert_eq!(error_code(input.SetSize(0)), STG_E_ACCESSDENIED);
        assert_eq!(
            error_code(input.CopyTo(&input, 5, Some(&mut copied_read), Some(&mut copied_written))),
            STG_E_ACCESSDENIED
        );
        assert_eq!((copied_read, copied_written), (0, 0));
        assert_eq!(
            error_code(input.Commit(STGC_DEFAULT)),
            STG_E_INVALIDFUNCTION
        );
        assert_eq!(error_code(input.Revert()), STG_E_INVALIDFUNCTION);
        assert_eq!(
            error_code(input.LockRegion(0, 1, LOCKTYPE(1))),
            STG_E_INVALIDFUNCTION
        );
        assert_eq!(
            error_code(input.UnlockRegion(0, 1, 1)),
            STG_E_INVALIDFUNCTION
        );
    }
    assert_eq!(fs::read(&path).unwrap(), b"never modified");
    assert_eq!(native_seek(&input, 0, STREAM_SEEK_CUR).unwrap(), 0);
}

#[test]
fn clone_has_its_own_cursor_and_holds_file_pin_until_last_reference_drops() {
    let _apartment = super::super::resources::Apartment::new().unwrap();
    let (_directory, path, input, _) = fixture(b"abcdefghij");
    assert_eq!(native_seek(&input, 2, STREAM_SEEK_SET).unwrap(), 2);
    // SAFETY: initialized apartment and live owned COM input.
    let clone = unsafe { input.Clone() }.unwrap();
    let mut output = [0; 2];
    assert_eq!(native_read(&clone, &mut output), (S_OK, 2));
    assert_eq!(&output, b"cd");
    assert_eq!(native_seek(&input, 0, STREAM_SEEK_CUR).unwrap(), 2);
    assert_eq!(native_seek(&clone, 0, STREAM_SEEK_CUR).unwrap(), 4);
    assert_eq!(native_seek(&input, 7, STREAM_SEEK_SET).unwrap(), 7);
    assert_eq!(native_read(&clone, &mut output), (S_OK, 2));
    assert_eq!(&output, b"ef");
    assert!(fs::OpenOptions::new().write(true).open(&path).is_err());
    drop(input);
    assert!(fs::OpenOptions::new().write(true).open(&path).is_err());
    drop(clone);
    assert!(fs::OpenOptions::new().write(true).open(&path).is_ok());
    assert_eq!(fs::read(path).unwrap(), b"abcdefghij");
}

#[test]
fn concurrent_agile_clones_seek_and_read_independent_positions() {
    let _apartment = super::super::resources::Apartment::new().unwrap();
    let body: Arc<[u8]> = (0..64_000)
        .map(|index| (index % 251) as u8)
        .collect::<Vec<_>>()
        .into();
    let (_directory, _path, input, _) = fixture(&body);
    let mut workers = Vec::new();
    for worker_index in 0..4 {
        // SAFETY: object is live in MTA; the agile reference, rather than a raw
        // interface pointer, performs the documented transfer to another MTA.
        let clone = unsafe { input.Clone() }.unwrap();
        let reference = AgileReference::new(&clone).unwrap();
        let expected = body.clone();
        workers.push(thread::spawn(move || {
            let _apartment = super::super::resources::Apartment::new().unwrap();
            let input = reference.resolve().unwrap();
            for turn in 0..128 {
                let offset = worker_index * 37 + turn * 257;
                native_seek(&input, offset as i64, STREAM_SEEK_SET).unwrap();
                let mut output = [0; 150];
                assert_eq!(native_read(&input, &mut output), (S_OK, 150));
                assert_eq!(&output, &expected[offset..offset + output.len()]);
            }
        }));
    }
    for worker in workers {
        worker.join().unwrap();
    }
    assert_eq!(native_seek(&input, 0, STREAM_SEEK_CUR).unwrap(), 0);
}

#[test]
fn cancellation_stops_constructor_and_all_read_only_com_operations() {
    let _apartment = super::super::resources::Apartment::new().unwrap();
    let (_directory, path, input, cancel) = fixture(b"cancelled source");
    let child = ResolverCancellation::linked(cancel.clone()).unwrap();
    let child_input = stream(File::open(&path).unwrap(), child).unwrap();
    cancel.cancel();
    let mut output = [0xa5; 4];
    assert_eq!(native_read(&input, &mut output), (E_ABORT, 0));
    assert_eq!(native_read(&child_input, &mut output), (E_ABORT, 0));
    assert_eq!(output, [0xa5; 4]);
    assert_eq!(error_code(native_seek(&input, 1, STREAM_SEEK_SET)), E_ABORT);
    let mut stat = STATSTG::default();
    // SAFETY: owned COM object, live writable stat, and no retained outputs.
    unsafe {
        assert_eq!(error_code(input.Clone()), E_ABORT);
        assert_eq!(error_code(input.Stat(&mut stat, STATFLAG_NONAME)), E_ABORT);
    }
    assert_eq!(
        stream(File::open(&path).unwrap(), cancel).err(),
        Some(MediaError::Cancelled)
    );
}

#[test]
fn poisoned_mutexes_are_native_errors_instead_of_abi_panics() {
    let _apartment = super::super::resources::Apartment::new().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("synthetic-poison.bin");
    fs::write(&path, b"synthetic").unwrap();
    let file = Arc::new(Mutex::new(File::open(path).unwrap()));
    let poison = file.clone();
    assert!(
        thread::spawn(move || {
            let _guard = poison.lock().unwrap();
            panic!("synthetic poison");
        })
        .join()
        .is_err()
    );
    let input: IStream = FileStream {
        file,
        position: Mutex::new(0),
        byte_len: 9,
        cancel: ResolverCancellation::default(),
    }
    .into();
    let mut output = [0xa5; 4];
    assert_eq!(native_read(&input, &mut output), (E_FAIL, 0));
    assert_eq!(output, [0xa5; 4]);
}

#[test]
fn poisoned_cursor_rejects_read_seek_and_clone_without_crossing_abi() {
    let _apartment = super::super::resources::Apartment::new().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("synthetic-cursor-poison.bin");
    fs::write(&path, b"synthetic").unwrap();
    let position = Mutex::new(0);
    assert!(
        catch_unwind(AssertUnwindSafe(|| {
            let _guard = position.lock().unwrap();
            panic!("synthetic cursor poison");
        }))
        .is_err()
    );
    let input: IStream = FileStream {
        file: Arc::new(Mutex::new(File::open(path).unwrap())),
        position,
        byte_len: 9,
        cancel: ResolverCancellation::default(),
    }
    .into();
    let mut output = [0xa5; 4];
    assert_eq!(native_read(&input, &mut output), (E_FAIL, 0));
    assert_eq!(error_code(native_seek(&input, 0, STREAM_SEEK_SET)), E_FAIL);
    // SAFETY: owned COM input stays live in this initialized apartment.
    assert_eq!(error_code(unsafe { input.Clone() }), E_FAIL);
    assert_eq!(output, [0xa5; 4]);
}

#[test]
fn abi_guards_turn_rust_panics_into_fixed_native_errors() {
    assert_eq!(abi_hresult(|| panic!("synthetic ABI panic")), E_FAIL);
    assert_eq!(
        error_code(abi_result::<()>(|| panic!("synthetic ABI panic"))),
        E_FAIL
    );
}
