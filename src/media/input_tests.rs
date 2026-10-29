//! Owned synthetic files only; fixtures contain no recorded audio or user data.
use super::*;
use std::fs;
use tempfile::TempDir;

fn fixture() -> (TempDir, PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("synthetic-media.bin");
    fs::write(&path, b"opaque synthetic bytes; no media encoding").unwrap();
    (directory, path)
}

fn subtitle(path: PathBuf) -> LocalSubtitleInput {
    LocalSubtitleInput::new(path, SubtitleFormat::Srt, TextLanguage::TaiwanTraditional).unwrap()
}

#[test]
fn opens_existing_opaque_file_without_reading_or_modifying_it() {
    let (_directory, path) = fixture();
    let original = fs::read(&path).unwrap();
    let input = MediaInput::local(path.clone()).unwrap();
    let opened = open_media(&input, &ResolverCancellation::default()).unwrap();
    assert_eq!(opened.byte_len(), original.len() as u64);
    assert_eq!(fs::read(&path).unwrap(), original);
    drop(opened);
    assert!(path.is_file());
}

#[test]
fn clone_uses_the_selected_file_object_and_pre_cancel_rejects_new_duplicates() {
    let (_directory, path) = fixture();
    let original = fs::read(&path).unwrap();
    let opened = open_media(
        &MediaInput::local(path).unwrap(),
        &ResolverCancellation::default(),
    )
    .unwrap();
    let mut duplicate = opened
        .try_clone_file(&ResolverCancellation::default())
        .unwrap();
    let mut bytes = Vec::new();
    duplicate.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, original);
    assert_eq!(opened.byte_len(), original.len() as u64);
    let cancelled = ResolverCancellation::default();
    cancelled.cancel();
    assert!(matches!(
        opened.try_clone_file(&cancelled),
        Err(MediaError::Cancelled)
    ));
}

#[cfg(windows)]
#[test]
fn original_pin_and_duplicate_independently_retain_write_delete_denial() {
    let (_directory, path) = fixture();
    let opened = open_media(
        &MediaInput::local(path.clone()).unwrap(),
        &ResolverCancellation::default(),
    )
    .unwrap();
    let duplicate = opened
        .try_clone_file(&ResolverCancellation::default())
        .unwrap();
    assert!(fs::OpenOptions::new().write(true).open(&path).is_err());
    assert!(fs::remove_file(&path).is_err());
    drop(opened);
    assert!(fs::OpenOptions::new().write(true).open(&path).is_err());
    assert!(fs::remove_file(&path).is_err());
    drop(duplicate);
    assert!(fs::OpenOptions::new().write(true).open(&path).is_ok());
    assert!(fs::remove_file(&path).is_ok());
}

#[test]
fn constructors_are_validation_only_and_do_not_create_missing_files() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("not-created.bin");
    let input = MediaInput::local(path.clone()).unwrap();
    assert!(!path.exists());
    assert!(matches!(
        open_media(&input, &ResolverCancellation::default()),
        Err(MediaError::Unavailable)
    ));
    assert_eq!(
        read_subtitle(&subtitle(path.clone()), &ResolverCancellation::default()).err(),
        Some(MediaError::Unavailable)
    );
    assert!(!path.exists());
}

#[test]
fn empty_media_and_directories_cannot_be_mistaken_for_opened_media() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("empty.bin");
    fs::write(&path, []).unwrap();
    assert!(matches!(
        open_media(
            &MediaInput::local(path).unwrap(),
            &ResolverCancellation::default()
        ),
        Err(MediaError::InvalidInput)
    ));
    assert!(matches!(
        open_media(
            &MediaInput::local(directory.path().into()).unwrap(),
            &ResolverCancellation::default()
        ),
        Err(MediaError::NotFile)
    ));
    assert_eq!(
        read_subtitle(
            &subtitle(directory.path().into()),
            &ResolverCancellation::default()
        )
        .err(),
        Some(MediaError::NotFile)
    );
}

#[test]
fn pre_cancelled_inputs_do_not_open_or_read() {
    let (_directory, path) = fixture();
    let cancel = ResolverCancellation::default();
    cancel.cancel();
    assert!(matches!(
        open_media(&MediaInput::local(path.clone()).unwrap(), &cancel),
        Err(MediaError::Cancelled)
    ));
    assert_eq!(
        read_subtitle(&subtitle(path), &cancel).err(),
        Some(MediaError::Cancelled)
    );
    assert!(matches!(
        open_media(
            &MediaInput::remote("https://example.invalid/media").unwrap(),
            &cancel
        ),
        Err(MediaError::Cancelled)
    ));
}

#[test]
fn remote_locations_are_explicitly_unsupported_without_fetching() {
    for raw in [
        "http://example.invalid/video.mp4",
        "https://example.invalid/video?signature=synthetic-private-token",
        "HTTPS://example.invalid/video",
    ] {
        let input = MediaInput::remote(raw).unwrap();
        assert!(matches!(
            open_media(&input, &ResolverCancellation::default()),
            Err(MediaError::UnsupportedUrl)
        ));
    }
}

#[test]
fn rejects_invalid_or_sensitive_url_forms() {
    for raw in [
        "",
        "https:///file",
        "file:///C:/test.mp4",
        "ftp://example.invalid/video.mp4",
        "https://@example.invalid/video",
        "https://user@example.invalid/video",
        "https://user:secret@example.invalid/video",
        "https://example.invalid/video#fragment",
        "https://example.invalid/video#",
        "https://example.invalid/video\n",
        "https://example.invalid/a b",
        "https://example.invalid/a\\b",
        "example.invalid/video",
        "https://",
    ] {
        assert!(matches!(
            MediaInput::remote(raw),
            Err(MediaError::InvalidInput)
        ));
    }
    let oversized = format!(
        "https://example.invalid/{}",
        "x".repeat(MAX_PREPARED_URL_BYTES)
    );
    assert!(matches!(
        MediaInput::remote(&oversized),
        Err(MediaError::InvalidInput)
    ));
    let at_limit = format!(
        "https://example.invalid/{}",
        "x".repeat(MAX_PREPARED_URL_BYTES - "https://example.invalid/".len())
    );
    assert!(MediaInput::remote(&at_limit).is_ok());
}

#[test]
fn local_paths_require_absolute_bounded_utf8_without_control_characters() {
    for path in [PathBuf::new(), PathBuf::from("relative-media.bin")] {
        assert!(matches!(
            MediaInput::local(path),
            Err(MediaError::InvalidInput)
        ));
    }
    let directory = tempfile::tempdir().unwrap();
    for name in ["has\nnewline.bin", "has\tTab.bin", "has\0nul.bin"] {
        let path = directory.path().join(name);
        assert!(matches!(
            MediaInput::local(path.clone()),
            Err(MediaError::InvalidInput)
        ));
        assert!(matches!(
            LocalSubtitleInput::new(path, SubtitleFormat::Srt, TextLanguage::TaiwanTraditional),
            Err(MediaError::InvalidInput)
        ));
    }
    let oversized = directory.path().join("x".repeat(MAX_PREPARED_PATH_BYTES));
    assert!(matches!(
        MediaInput::local(oversized),
        Err(MediaError::InvalidInput)
    ));
    assert!(matches!(
        MediaInput::local(directory.path().join("..").join("outside.bin")),
        Err(MediaError::InvalidInput)
    ));
}

#[test]
fn non_utf8_local_path_is_not_lossily_normalized() {
    #[cfg(windows)]
    let name = {
        use std::os::windows::ffi::OsStringExt;
        std::ffi::OsString::from_wide(&[0xd800])
    };
    #[cfg(unix)]
    let name = {
        use std::os::unix::ffi::OsStringExt;
        std::ffi::OsString::from_vec(vec![0xff])
    };
    let directory = tempfile::tempdir().unwrap();
    assert!(matches!(
        MediaInput::local(directory.path().join(name)),
        Err(MediaError::InvalidInput)
    ));
}

#[test]
fn subtitle_reader_retains_exact_bytes_and_declared_language_and_format() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("synthetic.srt");
    let bytes = "1\r\n00:00:01,000 --> 00:00:02,000\r\n合成的臺灣字幕\r\n".as_bytes();
    fs::write(&path, bytes).unwrap();
    let input = subtitle(path.clone());
    assert_eq!(input.format, SubtitleFormat::Srt);
    assert!(input.language == TextLanguage::TaiwanTraditional);
    assert_eq!(
        read_subtitle(&input, &ResolverCancellation::default()).unwrap(),
        bytes
    );
    assert_eq!(fs::read(&path).unwrap(), bytes);
    // Decoding/UTF-8 validation belongs to the parser, not the filesystem reader.
    let invalid = directory.path().join("synthetic-invalid-utf8.srt");
    fs::write(&invalid, [0xff, 0xfe, 0x01]).unwrap();
    assert_eq!(
        read_subtitle(&subtitle(invalid), &ResolverCancellation::default()).unwrap(),
        [0xff, 0xfe, 0x01]
    );
}

#[test]
fn exact_subtitle_byte_budget_is_accepted_and_one_byte_more_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("synthetic-budget.srt");
    let bytes = vec![b' '; MAX_PREPARED_DOCUMENT_BYTES];
    fs::write(&path, &bytes).unwrap();
    let input = subtitle(path.clone());
    assert_eq!(
        read_subtitle(&input, &ResolverCancellation::default()).unwrap(),
        bytes
    );
    fs::write(&path, vec![b' '; MAX_PREPARED_DOCUMENT_BYTES + 1]).unwrap();
    assert_eq!(
        read_subtitle(&input, &ResolverCancellation::default()).err(),
        Some(MediaError::TooLarge)
    );
    assert_eq!(
        fs::metadata(path).unwrap().len(),
        (MAX_PREPARED_DOCUMENT_BYTES + 1) as u64
    );
}

#[test]
fn cancellation_during_a_read_drops_the_partial_document() {
    struct CancelDuringRead {
        cancel: ResolverCancellation,
    }
    impl Read for CancelDuringRead {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            buffer[0] = b'x';
            self.cancel.cancel();
            Ok(1)
        }
    }
    let cancel = ResolverCancellation::default();
    assert_eq!(
        read_bounded(
            CancelDuringRead {
                cancel: cancel.clone()
            },
            0,
            &cancel
        )
        .err(),
        Some(MediaError::Cancelled)
    );
}

#[test]
fn document_growth_is_probed_only_one_byte_past_the_budget() {
    struct GrowingDocument {
        read_bytes: usize,
    }
    impl Read for GrowingDocument {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            buffer.fill(b'x');
            self.read_bytes += buffer.len();
            Ok(buffer.len())
        }
    }
    let mut growing = GrowingDocument { read_bytes: 0 };
    assert_eq!(
        read_bounded(&mut growing, 1, &ResolverCancellation::default()).err(),
        Some(MediaError::TooLarge)
    );
    assert_eq!(growing.read_bytes, MAX_PREPARED_DOCUMENT_BYTES + 1);
}

#[cfg(windows)]
#[test]
fn windows_device_unc_names_and_alternate_streams_are_refused() {
    for path in [
        r"\\server\share\media.mp4",
        r"\\?\C:\media.mp4",
        r"\\.\pipe\synthetic-pipe",
        r"C:\media.mp4:stream",
        r"C:\NUL",
        r"C:\NUL .txt",
        r"C:\con.txt",
        r"C:\aux.mp4",
        r"C:\PRN.txt",
        r"C:\COM1.txt",
        r"C:\LPT9.srt",
        "C:\\COM¹.txt",
        r"C:\CONIN$",
        r"C:\CONOUT$.txt",
        r"C:\trailing-dot.",
        "C:\\trailing-space ",
        r"C:\wild*.srt",
    ] {
        assert!(matches!(
            MediaInput::local(path.into()),
            Err(MediaError::InvalidInput)
        ));
    }
    assert!(MediaInput::local(r"C:\normal\COM10.txt".into()).is_ok());
}

#[cfg(windows)]
#[test]
fn windows_owned_fixture_drive_is_verified_by_the_native_query() {
    let (_directory, path) = fixture();
    assert_eq!(
        crate::win32::media_input::require_local_drive(&path),
        Ok(())
    );
    // This exercises the current local drive only. DRIVE_REMOTE rejection has
    // pure classification coverage; the test never mounts or modifies SMB drives.
    assert!(
        open_media(
            &MediaInput::local(path).unwrap(),
            &ResolverCancellation::default()
        )
        .is_ok()
    );
}

#[cfg(windows)]
#[test]
fn windows_open_media_pins_against_writes_or_deletion_until_release() {
    let (_directory, path) = fixture();
    let input = MediaInput::local(path.clone()).unwrap();
    let opened = open_media(&input, &ResolverCancellation::default()).unwrap();
    assert!(OpenOptions::new().write(true).open(&path).is_err());
    assert!(fs::remove_file(&path).is_err());
    assert!(path.is_file());
    drop(opened);
    let writer = OpenOptions::new().write(true).open(&path).unwrap();
    assert!(matches!(
        open_media(&input, &ResolverCancellation::default()),
        Err(MediaError::Unavailable)
    ));
    drop(writer);
    assert!(open_media(&input, &ResolverCancellation::default()).is_ok());
}

#[cfg(unix)]
#[test]
fn refuses_symlink_files_and_parent_directories_without_following_them() {
    use std::os::unix::fs::symlink;
    let (directory, target) = fixture();
    let link = directory.path().join("synthetic-link.bin");
    symlink(&target, &link).unwrap();
    let linked_input = MediaInput::local(link.clone()).unwrap();
    assert!(matches!(
        open_media(&linked_input, &ResolverCancellation::default()),
        Err(MediaError::NotFile)
    ));
    assert_eq!(
        read_subtitle(&subtitle(link), &ResolverCancellation::default()).err(),
        Some(MediaError::NotFile)
    );
    let target_directory = directory.path().join("target");
    fs::create_dir(&target_directory).unwrap();
    fs::write(target_directory.join("marker.bin"), b"synthetic marker").unwrap();
    let link_directory = directory.path().join("linked-directory");
    symlink(&target_directory, &link_directory).unwrap();
    assert!(matches!(
        open_media(
            &MediaInput::local(link_directory.join("marker.bin")).unwrap(),
            &ResolverCancellation::default()
        ),
        Err(MediaError::NotFile)
    ));
    fs::remove_file(link_directory).unwrap();
    assert_eq!(
        fs::read(target_directory.join("marker.bin")).unwrap(),
        b"synthetic marker"
    );
}

#[cfg(windows)]
#[test]
fn windows_refuses_junction_parents_and_fixture_cleanup_preserves_target() {
    let directory = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(directory.path()).unwrap();
    let target = directory.path().join("synthetic-target");
    let link = directory.path().join("synthetic-junction");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("marker.bin"), b"synthetic marker").unwrap();
    let resolved_target = fs::canonicalize(&target).unwrap();
    let resolved_link_parent = fs::canonicalize(link.parent().unwrap()).unwrap();
    assert!(resolved_target.starts_with(&root));
    assert_eq!(resolved_link_parent, root);
    // cmd's mklink is a builtin. Arguments contain only verified fixture paths;
    // no shell-based delete/move is used, and metacharacters fail the fixture.
    for path in [&target, &link] {
        let raw = path.to_str().unwrap();
        assert!(path.is_absolute());
        assert!(
            !raw.chars()
                .any(|ch| ch.is_control() || "\"&|<>^%!?".contains(ch))
        );
    }
    let status = std::process::Command::new("cmd.exe")
        .args(["/D", "/C", "mklink", "/J"])
        .arg(&link)
        .arg(&target)
        .output()
        .unwrap()
        .status;
    assert!(status.success());
    assert!(rejected_metadata(&fs::symlink_metadata(&link).unwrap()));
    let input = MediaInput::local(link.join("marker.bin")).unwrap();
    assert!(matches!(
        open_media(&input, &ResolverCancellation::default()),
        Err(MediaError::NotFile)
    ));
    assert_eq!(
        read_subtitle(
            &subtitle(link.join("marker.bin")),
            &ResolverCancellation::default()
        )
        .err(),
        Some(MediaError::NotFile)
    );
    // Rust's directory-link removal removes the junction itself, not its target.
    fs::remove_dir(&link).unwrap();
    assert!(!link.exists());
    assert_eq!(
        fs::read(target.join("marker.bin")).unwrap(),
        b"synthetic marker"
    );
}
