//! Read-only drive classification for Prepared worker I/O. A drive letter may
//! refer to a mapped network share, so syntax alone cannot prove local storage.
use crate::media::MediaError;
use std::path::{Component, Path, Prefix};
use windows::{
    Win32::{
        Storage::FileSystem::GetDriveTypeW,
        System::WindowsProgramming::{
            DRIVE_CDROM, DRIVE_FIXED, DRIVE_NO_ROOT_DIR, DRIVE_RAMDISK, DRIVE_REMOTE,
            DRIVE_REMOVABLE, DRIVE_UNKNOWN,
        },
    },
    core::PCWSTR,
};

/// Called on the Prepared worker, before any metadata lookup or file open.
/// This function neither reads the file nor changes drive mappings.
pub(crate) fn require_local_drive(path: &Path) -> Result<(), MediaError> {
    let root = disk_root(path)?;
    // SAFETY: `root` is exactly an ASCII drive letter followed by `:\\` and
    // a terminating NUL. Its four UTF-16 units remain live through the synchronous
    // query; Windows only borrows this immutable pointer and retains no ownership.
    let kind = unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) };
    classify_drive(kind)
}

fn disk_root(path: &Path) -> Result<[u16; 4], MediaError> {
    if !path.is_absolute() {
        return Err(MediaError::InvalidInput);
    }
    let Some(Component::Prefix(prefix)) = path.components().next() else {
        return Err(MediaError::InvalidInput);
    };
    let Prefix::Disk(drive) = prefix.kind() else {
        return Err(MediaError::InvalidInput);
    };
    if !drive.is_ascii_alphabetic() {
        return Err(MediaError::InvalidInput);
    }
    Ok([u16::from(drive), u16::from(b':'), u16::from(b'\\'), 0])
}

fn classify_drive(kind: u32) -> Result<(), MediaError> {
    match kind {
        DRIVE_FIXED | DRIVE_REMOVABLE | DRIVE_CDROM | DRIVE_RAMDISK => Ok(()),
        DRIVE_REMOTE => Err(MediaError::NotFile),
        DRIVE_UNKNOWN | DRIVE_NO_ROOT_DIR => Err(MediaError::Unavailable),
        _ => Err(MediaError::Unavailable),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permits_only_drive_types_reported_as_local_by_windows() {
        for kind in [DRIVE_FIXED, DRIVE_REMOVABLE, DRIVE_CDROM, DRIVE_RAMDISK] {
            assert_eq!(classify_drive(kind), Ok(()));
        }
        assert_eq!(classify_drive(DRIVE_REMOTE), Err(MediaError::NotFile));
        for kind in [DRIVE_UNKNOWN, DRIVE_NO_ROOT_DIR, u32::MAX] {
            assert_eq!(classify_drive(kind), Err(MediaError::Unavailable));
        }
    }

    #[test]
    fn native_query_receives_only_a_valid_nul_terminated_disk_root() {
        assert_eq!(
            disk_root(Path::new(r"C:\synthetic\file.bin")).unwrap(),
            [67, 58, 92, 0]
        );
        assert_eq!(
            disk_root(Path::new(r"z:\synthetic\file.bin")).unwrap(),
            [90, 58, 92, 0]
        );
        for raw in [
            r"C:relative.bin",
            r"relative.bin",
            r"\\server\share\file.bin",
            r"\\?\C:\file.bin",
            r"\\.\pipe\synthetic",
        ] {
            assert_eq!(
                disk_root(Path::new(raw)).err(),
                Some(MediaError::InvalidInput)
            );
        }
    }
}
