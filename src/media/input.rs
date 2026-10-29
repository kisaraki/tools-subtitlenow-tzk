//! Read-only Prepared inputs. Paths and signed URLs deliberately have no Debug
//! or serialization implementation. Disk I/O belongs only on a bounded worker.
use super::MediaError;
use crate::resolver::{
    contracts::{ResolverCancellation, TextLanguage},
    timed_subtitle::SubtitleFormat,
    timed_subtitle_parse::MAX_PREPARED_DOCUMENT_BYTES,
};
use reqwest::Url;
use std::{
    fs::{File, Metadata, OpenOptions},
    io::{ErrorKind, Read},
    path::{Component, Path, PathBuf},
};

/// The path/URL itself stays in Session RAM; neither is logged or persisted.
pub struct MediaInput {
    location: MediaLocation,
}
enum MediaLocation {
    Local(PathBuf),
    Remote(Url),
}
/// Absolute paths are bounded independently of file contents.
pub const MAX_PREPARED_PATH_BYTES: usize = 4096;
pub const MAX_PREPARED_URL_BYTES: usize = 4096;
/// Check cancellation between bounded reads rather than one whole-file read.
const SUBTITLE_READ_CHUNK_BYTES: usize = 8 * 1024;

impl MediaInput {
    #[cfg(any(windows, test))]
    pub(crate) fn is_remote(&self) -> bool {
        matches!(&self.location, MediaLocation::Remote(_))
    }
    pub fn local(path: PathBuf) -> Result<Self, MediaError> {
        validate_local_path(&path)?;
        Ok(Self {
            location: MediaLocation::Local(path),
        })
    }
    /// URL construction validates syntax only and never starts network I/O.
    pub fn remote(raw: &str) -> Result<Self, MediaError> {
        if raw.is_empty()
            || raw.len() > MAX_PREPARED_URL_BYTES
            || raw.contains('\\')
            || raw.chars().any(|ch| ch.is_control() || ch.is_whitespace())
            || raw
                .split_once("://")
                .and_then(|(_, tail)| tail.split(['/', '?', '#']).next())
                .is_none_or(|authority| authority.is_empty() || authority.contains('@'))
        {
            return Err(MediaError::InvalidInput);
        }
        let url = Url::parse(raw).map_err(|_| MediaError::InvalidInput)?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(MediaError::InvalidInput);
        }
        Ok(Self {
            location: MediaLocation::Remote(url),
        })
    }
    pub(crate) fn into_remote_url(self) -> Result<Url, MediaError> {
        match self.location {
            MediaLocation::Remote(url) => Ok(url),
            MediaLocation::Local(_) => Err(MediaError::InvalidInput),
        }
    }
}

/// One explicitly selected local subtitle; no filename-based language inference.
pub struct LocalSubtitleInput {
    path: PathBuf,
    pub format: SubtitleFormat,
    pub language: TextLanguage,
}
impl LocalSubtitleInput {
    pub fn new(
        path: PathBuf,
        format: SubtitleFormat,
        language: TextLanguage,
    ) -> Result<Self, MediaError> {
        validate_local_path(&path)?;
        Ok(Self {
            path,
            format,
            language,
        })
    }
}

/// Retains a real read-only handle. Existence/size proves neither decode support
/// nor that this opaque file contains playable media.
pub(crate) struct OpenedMedia {
    _file: File,
    byte_len: u64,
}
impl OpenedMedia {
    pub fn byte_len(&self) -> u64 {
        self.byte_len
    }
    /// Duplicate this already validated readonly file object, never its path.
    /// The pin itself performs no read/seek while the decoder uses the shared
    /// file cursor; it exists only to retain source identity and sharing rules.
    pub(crate) fn try_clone_file(&self, cancel: &ResolverCancellation) -> Result<File, MediaError> {
        check_cancel(cancel)?;
        let duplicate = self
            ._file
            .try_clone()
            .map_err(|_| MediaError::Unavailable)?;
        check_cancel(cancel)?;
        Ok(duplicate)
    }
}

pub(crate) fn open_media(
    input: &MediaInput,
    cancel: &ResolverCancellation,
) -> Result<OpenedMedia, MediaError> {
    check_cancel(cancel)?;
    let path = match &input.location {
        MediaLocation::Local(path) => path,
        MediaLocation::Remote(_url) => return Err(MediaError::UnsupportedUrl),
    };
    let (file, metadata) = open_regular_file(path, cancel)?;
    if metadata.len() == 0 {
        return Err(MediaError::InvalidInput);
    }
    check_cancel(cancel)?;
    Ok(OpenedMedia {
        _file: file,
        byte_len: metadata.len(),
    })
}

pub(crate) fn read_subtitle(
    input: &LocalSubtitleInput,
    cancel: &ResolverCancellation,
) -> Result<Vec<u8>, MediaError> {
    let (file, metadata) = open_regular_file(&input.path, cancel)?;
    if metadata.len() > MAX_PREPARED_DOCUMENT_BYTES as u64 {
        return Err(MediaError::TooLarge);
    }
    read_bounded(file, metadata.len() as usize, cancel)
}

fn read_bounded(
    mut reader: impl Read,
    initial_size: usize,
    cancel: &ResolverCancellation,
) -> Result<Vec<u8>, MediaError> {
    check_cancel(cancel)?;
    let mut body = Vec::new();
    body.try_reserve_exact(initial_size)
        .map_err(|_| MediaError::Overloaded)?;
    let mut chunk = [0_u8; SUBTITLE_READ_CHUNK_BYTES];
    loop {
        check_cancel(cancel)?;
        // Probe at most one byte beyond the budget so a growing file cannot
        // evade the initial metadata check or allocate an unbounded buffer.
        let limit = chunk
            .len()
            .min(MAX_PREPARED_DOCUMENT_BYTES - body.len() + 1);
        let read = match reader.read(&mut chunk[..limit]) {
            Ok(read) => read,
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(_) => return Err(MediaError::Unavailable),
        };
        check_cancel(cancel)?;
        if read == 0 {
            return Ok(body);
        }
        if read > MAX_PREPARED_DOCUMENT_BYTES - body.len() {
            return Err(MediaError::TooLarge);
        }
        body.try_reserve(read).map_err(|_| MediaError::Overloaded)?;
        body.extend_from_slice(&chunk[..read]);
    }
}

fn check_cancel(cancel: &ResolverCancellation) -> Result<(), MediaError> {
    if cancel.is_cancelled() {
        Err(MediaError::Cancelled)
    } else {
        Ok(())
    }
}

fn validate_local_path(path: &Path) -> Result<(), MediaError> {
    let raw = path.to_str().ok_or(MediaError::InvalidInput)?;
    if !path.is_absolute()
        || raw.is_empty()
        || raw.len() > MAX_PREPARED_PATH_BYTES
        || raw.chars().any(char::is_control)
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(MediaError::InvalidInput);
    }
    #[cfg(windows)]
    validate_windows_path(path)?;
    Ok(())
}

#[cfg(windows)]
fn validate_windows_path(path: &Path) -> Result<(), MediaError> {
    use std::path::Prefix;
    if !matches!(
        path.components().next(),
        Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_))
    ) {
        return Err(MediaError::InvalidInput);
    }
    for component in path.components() {
        if let Component::Normal(name) = component {
            let name = name.to_str().ok_or(MediaError::InvalidInput)?;
            // Colon is an alternate data stream. Trailing dots/spaces and DOS
            // names can select a different file or a device despite a disk path.
            if name.contains(':')
                || name.ends_with(['.', ' '])
                || name
                    .chars()
                    .any(|ch| matches!(ch, '<' | '>' | '"' | '|' | '?' | '*'))
                || is_dos_device(name)
            {
                return Err(MediaError::InvalidInput);
            }
        }
    }
    Ok(())
}

#[cfg(windows)]
fn is_dos_device(name: &str) -> bool {
    let base = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(' ');
    if ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"]
        .iter()
        .any(|reserved| base.eq_ignore_ascii_case(reserved))
    {
        return true;
    }
    let mut chars = base.chars();
    let prefix: String = chars.by_ref().take(3).collect();
    (prefix.eq_ignore_ascii_case("COM") || prefix.eq_ignore_ascii_case("LPT"))
        && matches!(chars.next(), Some('1'..='9' | '¹' | '²' | '³'))
        && chars.next().is_none()
}

fn rejected_metadata(metadata: &Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
            return true;
        }
    }
    false
}

fn open_regular_file(
    path: &Path,
    cancel: &ResolverCancellation,
) -> Result<(File, Metadata), MediaError> {
    check_cancel(cancel)?;
    validate_local_path(path)?;
    #[cfg(windows)]
    crate::win32::media_input::require_local_drive(path)?;
    // Inspect each existing component without following the component itself.
    // Parent reparse points may lead to a network path even with a disk prefix.
    let mut current = PathBuf::new();
    for component in path.components() {
        check_cancel(cancel)?;
        current.push(component.as_os_str());
        // A drive prefix alone is drive-relative; inspect after the root arrives.
        if matches!(component, Component::Prefix(_)) {
            continue;
        }
        let metadata = std::fs::symlink_metadata(&current).map_err(|_| MediaError::Unavailable)?;
        if rejected_metadata(&metadata) {
            return Err(MediaError::NotFile);
        }
    }
    let metadata = std::fs::symlink_metadata(path).map_err(|_| MediaError::Unavailable)?;
    if !metadata.is_file() || rejected_metadata(&metadata) {
        return Err(MediaError::NotFile);
    }
    check_cancel(cancel)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows::Win32::Storage::FileSystem::{FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ};
        // SHARE_READ rejects simultaneous writers/deletion. OPEN_REPARSE_POINT
        // prevents a last-component link replacement from following its target.
        options
            .share_mode(FILE_SHARE_READ.0)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0);
    }
    let file = options.open(path).map_err(|_| MediaError::Unavailable)?;
    let metadata = file.metadata().map_err(|_| MediaError::Unavailable)?;
    if !metadata.is_file() || rejected_metadata(&metadata) {
        return Err(MediaError::NotFile);
    }
    check_cancel(cancel)?;
    Ok((file, metadata))
}

#[cfg(test)]
#[path = "input_tests.rs"]
mod tests;
