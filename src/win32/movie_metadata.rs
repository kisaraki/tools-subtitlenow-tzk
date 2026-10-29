//! Session-owned, bounded observation of the explicitly selected process caption.
//! Native queries stay on one worker; the movie provider reads only bounded RAM.
use super::process;
use crate::{
    ai::speech::StreamIdentity,
    app::{
        content::SelectedProcess,
        movie::{MovieHintMetadata, MovieMetadataSource},
        source_metadata::SourceMetadataBuffer,
    },
    audio::sessions::{DISCOVERY_INTERVAL, MAX_WINDOWS},
    resolver::contracts::{MAX_METADATA_BYTES, ResolverError, WindowStamp},
};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, RecvTimeoutError, SyncSender},
    },
    thread::{self, JoinHandle},
    time::Instant,
};
use windows::{
    Win32::{
        Foundation::{
            CloseHandle, GetLastError, HANDLE, HWND, LPARAM, SetLastError, WAIT_TIMEOUT,
            WIN32_ERROR,
        },
        System::Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
            WaitForSingleObject,
        },
        UI::WindowsAndMessaging::{
            EnumWindows, GWL_EXSTYLE, GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW,
            GetWindowThreadProcessId, IsIconic, IsWindowVisible, WS_EX_TOOLWINDOW,
        },
    },
    core::BOOL,
};

/// At most one metadata observer runs process-wide. The CPU-derived limit is
/// deliberately capped at one: this lane performs short periodic native queries.
const MAX_METADATA_WORKERS: usize = 1;
const CAPTION_UNITS: usize = MAX_METADATA_BYTES + 1;
static ACTIVE_WORKERS: AtomicUsize = AtomicUsize::new(0);

struct WorkerPermit;
impl WorkerPermit {
    fn acquire() -> Result<Self, ResolverError> {
        let limit = thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1)
            .min(MAX_METADATA_WORKERS);
        ACTIVE_WORKERS
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active < limit).then_some(active + 1)
            })
            .map_err(|_| ResolverError::TextOverloaded)?;
        Ok(Self)
    }
}
impl Drop for WorkerPermit {
    fn drop(&mut self) {
        ACTIVE_WORKERS.fetch_sub(1, Ordering::AcqRel);
    }
}

struct NativeProcess(HANDLE);
impl NativeProcess {
    fn open(pid: u32, created: u64) -> Result<Self, ResolverError> {
        // SAFETY: query/synchronize-only access to the explicit PID. This worker
        // owns the returned handle and closes it on every success/error path.
        let process = Self(
            unsafe {
                OpenProcess(
                    PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                    false,
                    pid,
                )
            }
            .map_err(|_| ResolverError::SourceMetadataUnavailable)?,
        );
        if process::creation_time_for(process.0) != Some(created) || !alive(process.0) {
            return Err(ResolverError::SourceMetadataUnavailable);
        }
        Ok(process)
    }
}
impl Drop for NativeProcess {
    fn drop(&mut self) {
        // SAFETY: this is the single owning, non-pseudo OpenProcess handle.
        let _ = unsafe { CloseHandle(self.0) };
    }
}
fn alive(process: HANDLE) -> bool {
    // SAFETY: borrowed live SYNCHRONIZE handle held by the worker. Zero timeout
    // never waits, and any result besides WAIT_TIMEOUT means identity is unusable.
    (unsafe { WaitForSingleObject(process, 0) }) == WAIT_TIMEOUT
}

struct Reader {
    buffer: Arc<SourceMetadataBuffer>,
    cancelled: Arc<AtomicBool>,
    wake: SyncSender<()>,
    worker: Mutex<Option<JoinHandle<()>>>,
}
impl MovieMetadataSource for Reader {
    fn metadata(&self, stamp: WindowStamp) -> Result<MovieHintMetadata, ResolverError> {
        self.buffer.metadata(stamp)
    }
}
impl Drop for Reader {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        self.buffer.stop();
        let _ = self.wake.try_send(());
        if let Some(worker) = self
            .worker
            .get_mut()
            .unwrap_or_else(|error| error.into_inner())
            .take()
        {
            let _ = worker.join();
        }
    }
}

/// Construction starts one owned worker, without executing native queries on
/// the caller. Missing birth identity cannot enable any observation.
pub(crate) fn start(
    identity: StreamIdentity,
    selected: SelectedProcess,
) -> Result<Arc<dyn MovieMetadataSource>, ResolverError> {
    let reader = start_reader(identity, selected)?;
    Ok(reader)
}
fn start_reader(
    identity: StreamIdentity,
    selected: SelectedProcess,
) -> Result<Arc<Reader>, ResolverError> {
    let Some(created) = selected.created.filter(|created| *created != 0) else {
        return Err(ResolverError::SourceMetadataUnavailable);
    };
    if selected.pid == 0 || selected.pid == std::process::id() {
        // Same-process caption APIs send synchronous window messages. Rejecting
        // our PID also prevents an observer from waiting on the UI that joins it.
        return Err(ResolverError::SourceMetadataUnavailable);
    }
    let permit = WorkerPermit::acquire()?;
    let buffer = Arc::new(SourceMetadataBuffer::new(identity));
    let cancelled = Arc::new(AtomicBool::new(false));
    let (wake, receiver) = mpsc::sync_channel(1);
    let worker_buffer = buffer.clone();
    let worker_cancelled = cancelled.clone();
    let worker = thread::Builder::new()
        .name("movie-metadata".into())
        .spawn(move || {
            let _permit = permit;
            let mut sequence = 0_u64;
            let result = catch_unwind(AssertUnwindSafe(|| {
                let process = NativeProcess::open(selected.pid, created)?;
                while !worker_cancelled.load(Ordering::Acquire) {
                    let observed_at = Instant::now();
                    sequence = sequence.checked_add(1).ok_or(ResolverError::InvalidData)?;
                    if !alive(process.0) {
                        return Err(ResolverError::SourceMetadataUnavailable);
                    }
                    let title = scan(selected.pid, process.0, &worker_cancelled);
                    if worker_cancelled.load(Ordering::Acquire) {
                        return Ok(());
                    }
                    // An ambiguous/missing/raced caption clears the publication,
                    // but the same live process may regain a unique title later.
                    if matches!(
                        worker_buffer.publish(sequence, observed_at, title),
                        Err(ResolverError::Cancelled)
                    ) {
                        return Ok(());
                    }
                    match receiver.recv_timeout(DISCOVERY_INTERVAL) {
                        Err(RecvTimeoutError::Timeout) => {}
                        Ok(()) | Err(RecvTimeoutError::Disconnected) => return Ok(()),
                    }
                }
                Ok(())
            }))
            .unwrap_or(Err(ResolverError::WorkerPanicked));
            if let Err(error) = result {
                worker_buffer.fail(error);
            }
        })
        .map_err(|_| ResolverError::WorkerStart)?;
    Ok(Arc::new(Reader {
        buffer,
        cancelled,
        wake,
        worker: Mutex::new(Some(worker)),
    }))
}

struct Search<'a> {
    pid: u32,
    process: HANDLE,
    cancelled: &'a AtomicBool,
    visited: usize,
    candidate: Option<HWND>,
    caption: [u16; CAPTION_UNITS],
    length: usize,
    failure: Option<ResolverError>,
}
impl Search<'_> {
    fn accept(&mut self, window: HWND, caption: [u16; CAPTION_UNITS], length: usize) -> bool {
        if self.candidate.is_some() {
            self.failure = Some(ResolverError::SourceMetadataUnavailable);
            return false;
        }
        self.candidate = Some(window);
        self.caption = caption;
        self.length = length;
        true
    }
    fn fail(&mut self, error: ResolverError) -> BOOL {
        self.failure = Some(error);
        false.into()
    }
}

fn scan(pid: u32, process: HANDLE, cancelled: &AtomicBool) -> Result<Box<str>, ResolverError> {
    let mut search = Search {
        pid,
        process,
        cancelled,
        visited: 0,
        candidate: None,
        caption: [0; CAPTION_UNITS],
        length: 0,
        failure: None,
    };
    // SAFETY: a unique initialized stack context survives synchronous enumeration.
    // The callback stores no pointers, allocates no memory, and cannot unwind.
    let complete = unsafe {
        EnumWindows(
            Some(visit),
            LPARAM((&mut search as *mut Search<'_>) as isize),
        )
    }
    .is_ok();
    if let Some(error) = search.failure {
        return Err(error);
    }
    if !complete || !alive(process) || cancelled.load(Ordering::Acquire) {
        return Err(ResolverError::SourceMetadataUnavailable);
    }
    let window = search
        .candidate
        .ok_or(ResolverError::SourceMetadataUnavailable)?;
    let mut final_caption = [0; CAPTION_UNITS];
    let final_length = read_caption(window, pid, process, &mut final_caption)?
        .ok_or(ResolverError::SourceMetadataUnavailable)?;
    if final_length != search.length || final_caption != search.caption {
        return Err(ResolverError::SourceMetadataUnavailable);
    }
    valid_caption(&search.caption[..search.length])
}

unsafe extern "system" fn visit(window: HWND, context: LPARAM) -> BOOL {
    // SAFETY: only scan provides this exclusive context; it remains live for the
    // synchronous EnumWindows call, and no pointer or native handle escapes.
    let search = unsafe { &mut *(context.0 as *mut Search<'_>) };
    if search.cancelled.load(Ordering::Acquire) {
        return search.fail(ResolverError::Cancelled);
    }
    if search.visited >= MAX_WINDOWS {
        return search.fail(ResolverError::SourceMetadataUnavailable);
    }
    search.visited += 1;
    match eligible(window, search.pid) {
        Ok(true) => {}
        Ok(false) => return true.into(),
        Err(error) => return search.fail(error),
    }
    let mut caption = [0; CAPTION_UNITS];
    match read_caption(window, search.pid, search.process, &mut caption) {
        Ok(Some(length)) => search.accept(window, caption, length).into(),
        Ok(None) => true.into(),
        Err(error) => search.fail(error),
    }
}

fn eligible(window: HWND, pid: u32) -> Result<bool, ResolverError> {
    let mut owner = 0;
    // SAFETY: HWND is supplied by EnumWindows or retained until its final recheck;
    // stale HWNDs fail ownership/visibility. These are query-only, short APIs.
    unsafe {
        if GetWindowThreadProcessId(window, Some(&mut owner)) == 0
            || owner != pid
            || !IsWindowVisible(window).as_bool()
            || IsIconic(window).as_bool()
        {
            return Ok(false);
        }
        SetLastError(WIN32_ERROR(0));
        let style = GetWindowLongPtrW(window, GWL_EXSTYLE);
        if style == 0 && GetLastError().0 != 0 {
            return Err(ResolverError::SourceMetadataUnavailable);
        }
        Ok(style as u32 & WS_EX_TOOLWINDOW.0 == 0)
    }
}
fn caption_length(window: HWND) -> Result<usize, ResolverError> {
    // SAFETY: only directly selected, external-process windows reach this query.
    // Clear last error to distinguish empty captions from failed/raced HWNDs.
    let (length, error) = unsafe {
        SetLastError(WIN32_ERROR(0));
        let length = GetWindowTextLengthW(window);
        (length, GetLastError())
    };
    if length < 0 || (length == 0 && error.0 != 0) || length as usize >= CAPTION_UNITS {
        return Err(ResolverError::SourceMetadataUnavailable);
    }
    Ok(length as usize)
}
fn read_caption(
    window: HWND,
    pid: u32,
    process: HANDLE,
    caption: &mut [u16; CAPTION_UNITS],
) -> Result<Option<usize>, ResolverError> {
    if !alive(process) || !eligible(window, pid)? {
        return Err(ResolverError::SourceMetadataUnavailable);
    }
    let before = caption_length(window)?;
    // SAFETY: fixed writable UTF-16 buffer includes a terminator slot. The window
    // belongs to another process, so GetWindowText reads its caption without
    // sending WM_GETTEXT to an application thread. No truncation is accepted.
    let copied = unsafe { GetWindowTextW(window, caption) };
    let after = caption_length(window)?;
    if copied < 0
        || copied as usize != before
        || after != before
        || !eligible(window, pid)?
        || !alive(process)
    {
        return Err(ResolverError::SourceMetadataUnavailable);
    }
    if before == 0 {
        Ok(None)
    } else {
        Ok(Some(before))
    }
}
fn valid_caption(units: &[u16]) -> Result<Box<str>, ResolverError> {
    let text = String::from_utf16(units).map_err(|_| ResolverError::SourceMetadataUnavailable)?;
    if text.trim().is_empty()
        || text.len() > MAX_METADATA_BYTES
        || text.chars().any(char::is_control)
    {
        return Err(ResolverError::SourceMetadataUnavailable);
    }
    Ok(text.into_boxed_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::capture::SessionId;
    use std::time::Duration;

    static NATIVE_TEST_LOCK: Mutex<()> = Mutex::new(());
    fn identity() -> StreamIdentity {
        StreamIdentity {
            session: SessionId(709001),
            generation: 1,
        }
    }
    fn stamp() -> WindowStamp {
        WindowStamp {
            identity: identity(),
            sequence: 1,
            ended_at: Instant::now(),
            start_us: Some(0),
            duration_us: 1_000_000,
        }
    }
    fn search(cancelled: &AtomicBool) -> Search<'_> {
        Search {
            pid: 123,
            process: HANDLE::default(),
            cancelled,
            visited: 0,
            candidate: None,
            caption: [0; CAPTION_UNITS],
            length: 0,
            failure: None,
        }
    }
    #[test]
    fn caption_validation_retains_original_unicode_and_rejects_invalid_or_unbounded_data() {
        let text = "Synthetic Lantern｜電影測試";
        assert_eq!(
            valid_caption(&text.encode_utf16().collect::<Vec<_>>())
                .unwrap()
                .as_ref(),
            text
        );
        for invalid in ["", " \u{3000}", "title\nprivate", "title\0tail"] {
            assert_eq!(
                valid_caption(&invalid.encode_utf16().collect::<Vec<_>>()),
                Err(ResolverError::SourceMetadataUnavailable)
            );
        }
        assert_eq!(
            valid_caption(&[0xd800]),
            Err(ResolverError::SourceMetadataUnavailable)
        );
        assert_eq!(
            valid_caption(&"片".repeat(342).encode_utf16().collect::<Vec<_>>()),
            Err(ResolverError::SourceMetadataUnavailable)
        );
        assert_eq!(
            valid_caption(
                &"a".repeat(MAX_METADATA_BYTES)
                    .encode_utf16()
                    .collect::<Vec<_>>()
            )
            .unwrap()
            .len(),
            MAX_METADATA_BYTES
        );
    }
    #[test]
    fn accumulator_rejects_multiple_windows_even_with_identical_captions() {
        let cancelled = AtomicBool::new(false);
        let mut search = search(&cancelled);
        let mut caption = [0; CAPTION_UNITS];
        caption[0] = u16::from(b'A');
        assert!(search.accept(HWND(std::ptr::dangling_mut()), caption, 1));
        assert!(!search.accept(HWND(2usize as *mut _), caption, 1));
        assert_eq!(
            search.failure,
            Some(ResolverError::SourceMetadataUnavailable)
        );
    }
    #[test]
    fn callback_stops_at_enumeration_limit_and_cancellation_without_native_queries() {
        let cancelled = AtomicBool::new(false);
        let mut search = search(&cancelled);
        search.visited = MAX_WINDOWS;
        // SAFETY: exclusive live test context; the pre-query limit branch exits.
        assert!(
            !unsafe {
                visit(
                    HWND::default(),
                    LPARAM((&mut search as *mut Search<'_>) as isize),
                )
            }
            .as_bool()
        );
        assert_eq!(
            search.failure,
            Some(ResolverError::SourceMetadataUnavailable)
        );
        cancelled.store(true, Ordering::Release);
        // SAFETY: same exclusive context; cancellation exits before native queries.
        assert!(
            !unsafe {
                visit(
                    HWND::default(),
                    LPARAM((&mut search as *mut Search<'_>) as isize),
                )
            }
            .as_bool()
        );
        assert_eq!(search.failure, Some(ResolverError::Cancelled));
    }
    #[test]
    fn missing_creation_or_own_process_never_starts_worker() {
        for selected in [
            SelectedProcess {
                pid: 123,
                created: None,
            },
            SelectedProcess {
                pid: 123,
                created: Some(0),
            },
            SelectedProcess {
                pid: 0,
                created: Some(1),
            },
            SelectedProcess {
                pid: std::process::id(),
                created: Some(1),
            },
        ] {
            assert!(matches!(
                start(identity(), selected),
                Err(ResolverError::SourceMetadataUnavailable)
            ));
        }
    }
    #[test]
    fn worker_permit_is_bounded_and_released_without_a_native_job() {
        let _guard = NATIVE_TEST_LOCK.lock().unwrap();
        let permit = WorkerPermit::acquire().unwrap();
        assert!(matches!(
            WorkerPermit::acquire(),
            Err(ResolverError::TextOverloaded)
        ));
        drop(permit);
        assert!(WorkerPermit::acquire().is_ok());
    }
    #[test]
    fn invalid_native_pid_retires_worker_and_reader_drop_joins_it() {
        let _guard = NATIVE_TEST_LOCK.lock().unwrap();
        let reader = start_reader(
            identity(),
            SelectedProcess {
                pid: u32::MAX,
                created: Some(1),
            },
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if reader
                .worker
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(JoinHandle::is_finished)
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "owned metadata worker did not retire"
            );
            thread::sleep(Duration::from_millis(2));
        }
        assert!(matches!(
            reader.metadata(stamp()),
            Err(ResolverError::SourceMetadataUnavailable)
        ));
        drop(reader);
        assert!(WorkerPermit::acquire().is_ok());
    }

    const CHILD_MARKER: &str = "SUBTITLENOW_TEST_METADATA_CHILD";
    const CHILD_FIXTURE: &str = "win32::movie_metadata::tests::native_caption_child_fixture";
    const NATIVE_TIMEOUT: Duration = Duration::from_secs(5);
    const TEST_POLL: Duration = Duration::from_millis(5);
    const MAX_CHILD_LINE_BYTES: u64 = 512;
    const FIRST_CAPTION: &str = "SubtitleNow synthetic movie fixture A";
    const SECOND_CAPTION: &str = "SubtitleNow synthetic movie fixture B";

    struct CaptionChild {
        child: std::process::Child,
        input: Option<std::process::ChildStdin>,
        lines: mpsc::Receiver<String>,
        output_worker: Option<JoinHandle<()>>,
    }
    impl CaptionChild {
        fn spawn() -> Self {
            use std::{
                io::BufRead,
                os::windows::process::CommandExt,
                process::{Command, Stdio},
            };
            use windows::Win32::System::Threading::CREATE_NO_WINDOW;
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--ignored",
                    "--exact",
                    CHILD_FIXTURE,
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env(CHILD_MARKER, "1")
                .creation_flags(CREATE_NO_WINDOW.0)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let input = child.stdin.take();
            let output = child.stdout.take().unwrap();
            let (sender, lines) = mpsc::sync_channel(8);
            let output_worker = thread::spawn(move || {
                let mut output = std::io::BufReader::new(output);
                loop {
                    let mut line = String::new();
                    let mut bounded = std::io::Read::take(&mut output, MAX_CHILD_LINE_BYTES);
                    match bounded.read_line(&mut line) {
                        Ok(0) | Err(_) => break,
                        Ok(_) if !line.ends_with('\n') || sender.try_send(line).is_err() => break,
                        Ok(_) => {}
                    }
                }
            });
            Self {
                child,
                input,
                lines,
                output_worker: Some(output_worker),
            }
        }
        fn wait_line(&self, marker: &str) {
            let deadline = Instant::now() + NATIVE_TIMEOUT;
            loop {
                let remaining = deadline
                    .checked_duration_since(Instant::now())
                    .expect("caption child exceeded handshake deadline");
                let line = self
                    .lines
                    .recv_timeout(remaining)
                    .expect("caption child did not acknowledge command");
                if line.contains(marker) {
                    return;
                }
            }
        }
        fn command(&mut self, command: &str) {
            use std::io::Write;
            writeln!(self.input.as_mut().unwrap(), "{command}").unwrap();
            self.input.as_mut().unwrap().flush().unwrap();
            self.wait_line(&format!("metadata-{command}"));
        }
    }
    impl Drop for CaptionChild {
        fn drop(&mut self) {
            self.input.take();
            let _ = self.child.kill();
            let _ = self.child.wait();
            if let Some(worker) = self.output_worker.take() {
                let _ = worker.join();
            }
        }
    }
    fn await_title(reader: &Reader, expected: &str) -> MovieHintMetadata {
        let deadline = Instant::now() + NATIVE_TIMEOUT;
        loop {
            if let Ok(metadata) = reader.metadata(stamp())
                && metadata.window_title.as_deref() == Some(expected)
            {
                return metadata;
            }
            assert!(
                Instant::now() < deadline,
                "native observer did not publish expected synthetic caption"
            );
            thread::sleep(TEST_POLL);
        }
    }
    fn await_unavailable(reader: &Reader) {
        let deadline = Instant::now() + NATIVE_TIMEOUT;
        loop {
            if matches!(
                reader.metadata(stamp()),
                Err(ResolverError::SourceMetadataUnavailable)
            ) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "native observer did not retire synthetic caption"
            );
            thread::sleep(TEST_POLL);
        }
    }
    fn await_finished(reader: &Reader) {
        let deadline = Instant::now() + NATIVE_TIMEOUT;
        loop {
            if reader
                .worker
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(JoinHandle::is_finished)
            {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "owned metadata worker did not finish"
            );
            thread::sleep(TEST_POLL);
        }
    }

    #[test]
    fn native_external_owned_caption_lifecycle_is_identity_bound_and_cancellable() {
        let _guard = NATIVE_TEST_LOCK.lock().unwrap();
        let mut child = CaptionChild::spawn();
        child.wait_line("metadata-ready");
        let created = process::creation_time(child.child.id()).unwrap();
        let wrong = start_reader(
            identity(),
            SelectedProcess {
                pid: child.child.id(),
                created: Some(created + 1),
            },
        )
        .unwrap();
        await_finished(&wrong);
        await_unavailable(&wrong);
        drop(wrong);
        let selected = SelectedProcess {
            pid: child.child.id(),
            created: Some(created),
        };
        let reader = start_reader(identity(), selected).unwrap();
        let first = await_title(&reader, FIRST_CAPTION);
        let first_lease = first.validity.unwrap();
        assert!(!first_lease.cancellation.is_cancelled());
        assert!(first.filename.is_none() && first.year.is_none() && first.duration_us.is_none());
        assert!(matches!(
            start(identity(), selected),
            Err(ResolverError::TextOverloaded)
        ));
        child.command("change");
        let second = await_title(&reader, SECOND_CAPTION).validity.unwrap();
        assert!(first_lease.cancellation.is_cancelled());
        child.command("duplicate");
        await_unavailable(&reader);
        assert!(second.cancellation.is_cancelled());
        child.command("single");
        let recovered = await_title(&reader, SECOND_CAPTION).validity.unwrap();
        child.command("remove");
        await_unavailable(&reader);
        assert!(recovered.cancellation.is_cancelled());
        child.command("restore");
        let dropped = await_title(&reader, FIRST_CAPTION).validity.unwrap();
        drop(reader);
        assert!(dropped.cancellation.is_cancelled());
        let reader = start_reader(identity(), selected).unwrap();
        let restarted = await_title(&reader, FIRST_CAPTION).validity.unwrap();
        child.command("exit");
        await_finished(&reader);
        await_unavailable(&reader);
        assert!(restarted.cancellation.is_cancelled());
        drop(reader);
        let deadline = Instant::now() + NATIVE_TIMEOUT;
        loop {
            if let Some(status) = child.child.try_wait().unwrap() {
                assert!(status.success(), "synthetic caption child failed");
                break;
            }
            assert!(
                Instant::now() < deadline,
                "synthetic caption child did not exit"
            );
            thread::sleep(TEST_POLL);
        }
        assert!(WorkerPermit::acquire().is_ok());
    }

    #[test]
    #[ignore = "child-only synthetic external caption fixture; exercised by normal parent test"]
    fn native_caption_child_fixture() {
        use std::io::{BufRead, Write};
        use windows::{
            Win32::UI::WindowsAndMessaging::{
                CreateWindowExW, DestroyWindow, SetWindowTextW, WS_EX_NOACTIVATE,
                WS_OVERLAPPEDWINDOW, WS_VISIBLE,
            },
            core::{HSTRING, w},
        };
        assert_eq!(std::env::var(CHILD_MARKER).as_deref(), Ok("1"));
        struct Window(HWND);
        impl Window {
            fn new(title: &str) -> Self {
                // SAFETY: built-in class and owned caption; this child creates only
                // its own offscreen, nonactivated top-level test windows.
                Self(
                    unsafe {
                        CreateWindowExW(
                            WS_EX_NOACTIVATE,
                            w!("STATIC"),
                            &HSTRING::from(title),
                            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                            -32_000,
                            -32_000,
                            100,
                            80,
                            None,
                            None,
                            None,
                            None,
                        )
                    }
                    .unwrap(),
                )
            }
        }
        impl Drop for Window {
            fn drop(&mut self) {
                // SAFETY: uniquely owned fixture HWND, destroyed on its UI thread.
                let _ = unsafe { DestroyWindow(self.0) };
            }
        }
        let mut primary = Some(Window::new(FIRST_CAPTION));
        let mut duplicate = None;
        println!("metadata-ready");
        std::io::stdout().flush().unwrap();
        let mut input = std::io::stdin().lock();
        loop {
            let mut command = String::new();
            let mut bounded = std::io::Read::take(&mut input, 64);
            let length = bounded.read_line(&mut command).unwrap();
            if length == 0 {
                break;
            }
            assert!(command.ends_with('\n'), "oversized caption fixture command");
            let command = command.trim();
            match command {
                "change" => {
                    // SAFETY: only this child's uniquely owned HWND is changed on
                    // its creating thread; no foreign window receives a message.
                    unsafe {
                        SetWindowTextW(primary.as_ref().unwrap().0, &HSTRING::from(SECOND_CAPTION))
                    }
                    .unwrap();
                }
                "duplicate" => duplicate = Some(Window::new(SECOND_CAPTION)),
                "single" => {
                    duplicate.take();
                }
                "remove" => {
                    duplicate.take();
                    primary.take();
                }
                "restore" => primary = Some(Window::new(FIRST_CAPTION)),
                "exit" => {
                    duplicate.take();
                    primary.take();
                }
                _ => panic!("invalid owned caption fixture command"),
            }
            println!("metadata-{command}");
            std::io::stdout().flush().unwrap();
            if command == "exit" {
                break;
            }
        }
    }
}
