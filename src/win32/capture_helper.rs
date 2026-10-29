//! Session-owned native audio process. Binary PCM IPC is RAM-only, never a log/file.
use super::capture::{CaptureClock, ProcessTarget, capture_native, native};
use crate::audio::{
    capture::{
        AudioStamp, BLOCK_FRAMES, CaptureError, CaptureSession, CaptureSink, CaptureState,
        SessionId,
    },
    discovery::AudioSessionScanner,
    preprocess::NormalizedAudio,
};
use std::{
    io::{Read, Write},
    mem::size_of,
    os::windows::{
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
        process::CommandExt,
    },
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::{ERROR_BROKEN_PIPE, HANDLE},
        Storage::FileSystem::{FILE_TYPE_PIPE, GetFileType},
        System::{JobObjects::*, Pipes::PeekNamedPipe, Threading::CREATE_NO_WINDOW},
    },
    core::PCWSTR,
};

const MAGIC: &[u8; 8] = b"SNPCM001";
const HELLO_BYTES: usize = 20;
const HEADER_BYTES: usize = 13;
const MAX_PAYLOAD: usize = 4096;
const MAX_START_PREFIX: usize = 4096; // test harness banners; production sends none
const READY: u8 = 1;
const AUDIO: u8 = 2;
const ERROR: u8 = 3;
const DONE: u8 = 4;
const POLL: Duration = Duration::from_millis(2);
const START_TIMEOUT: Duration = Duration::from_secs(10);
const STOP_GRACE: Duration = Duration::from_millis(500);
const NONE: u64 = u64::MAX;
static LIVE_HELPER: AtomicBool = AtomicBool::new(false);

struct Permit;
impl Drop for Permit {
    fn drop(&mut self) {
        LIVE_HELPER.store(false, Ordering::Release);
    }
}

fn available(handle: HANDLE) -> Result<usize, CaptureError> {
    let mut bytes = 0;
    // SAFETY: caller owns a live anonymous pipe handle; output is writable, no read/copy.
    unsafe { PeekNamedPipe(handle, None, 0, None, Some(&mut bytes), None) }.map_err(|error| {
        if error.code() == windows::core::HRESULT::from_win32(ERROR_BROKEN_PIPE.0) {
            CaptureError::WorkerExited
        } else {
            native("PeekNamedPipe", error)
        }
    })?;
    Ok(bytes as usize)
}

struct Helper {
    child: Child,
    input: Option<ChildStdin>,
    output: Option<ChildStdout>,
    job: Option<OwnedHandle>,
}
impl Helper {
    fn spawn() -> Result<Self, CaptureError> {
        // SAFETY: unnamed, non-inheritable job; a successful handle transfers once below.
        let job = unsafe { CreateJobObjectW(None, PCWSTR::null()) }
            .map_err(|error| native("CreateJobObjectW", error))?;
        // SAFETY: a real owning handle, not a pseudo handle.
        let job = unsafe { OwnedHandle::from_raw_handle(job.0) };
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: correct information class, initialized struct and exact size. Closing our
        // last job handle kills only its assigned helper, never the selected source process.
        unsafe {
            SetInformationJobObject(
                HANDLE(job.as_raw_handle()),
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        }
        .map_err(|error| native("SetInformationJobObject", error))?;
        let mut command =
            Command::new(std::env::current_exe().map_err(|_| CaptureError::WorkerProcessStart)?);
        #[cfg(not(test))]
        command.arg("--capture-helper");
        #[cfg(test)]
        command
            .args([
                "--exact",
                "win32::tests::native_session_appears_updates_and_disappears",
                "--ignored",
                "--nocapture",
            ])
            .env("SUBTITLENOW_CAPTURE_HELPER", "1")
            .env_remove("SUBTITLENOW_NATIVE_FIXTURE");
        let mut child = command
            .creation_flags(CREATE_NO_WINDOW.0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| CaptureError::WorkerProcessStart)?;
        // Child blocks on our hello before any discovery/capture. If assignment fails it
        // is killed here; if this parent crashes, inherited stdin closes or the job kills it.
        // SAFETY: both handles are owned and the process handle is our newly spawned helper.
        if let Err(error) = unsafe {
            AssignProcessToJobObject(HANDLE(job.as_raw_handle()), HANDLE(child.as_raw_handle()))
        } {
            let _ = child.kill();
            let _ = child.wait();
            return Err(native("AssignProcessToJobObject", error));
        }
        let input = child.stdin.take();
        let output = child.stdout.take();
        Ok(Self {
            child,
            input,
            output,
            job: Some(job),
        })
    }
}
impl Drop for Helper {
    fn drop(&mut self) {
        // Closing stdout first unblocks a helper transport write. EOF on stdin requests stop.
        self.output.take();
        self.input.take();
        let deadline = Instant::now() + STOP_GRACE;
        while matches!(self.child.try_wait(), Ok(None)) && Instant::now() < deadline {
            std::thread::park_timeout(POLL);
        }
        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            // SAFETY: this job contains only our helper. A blocked native call may prevent
            // graceful stop; process termination releases its bounded native resources.
            if let Some(job) = &self.job {
                let _ = unsafe { TerminateJobObject(HANDLE(job.as_raw_handle()), 1) };
            }
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

fn send(output: &mut impl Write, kind: u8, payload: &[u8]) -> Result<(), CaptureError> {
    if payload.len() > MAX_PAYLOAD {
        return Err(CaptureError::WorkerProtocol);
    }
    output
        .write_all(MAGIC)
        .and_then(|_| output.write_all(&[kind]))
        .and_then(|_| output.write_all(&(payload.len() as u32).to_le_bytes()))
        .and_then(|_| output.write_all(payload))
        .and_then(|_| output.flush())
        .map_err(|_| CaptureError::WorkerExited)
}

struct Reader {
    bytes: Vec<u8>,
    skipped: usize,
    synchronized: bool,
}
impl Reader {
    fn new() -> Self {
        Self {
            bytes: Vec::with_capacity(MAX_PAYLOAD + HEADER_BYTES),
            skipped: 0,
            synchronized: false,
        }
    }
    fn next(&mut self, output: &mut ChildStdout) -> Result<Option<(u8, Vec<u8>)>, CaptureError> {
        if let Some(message) = self.buffered()? {
            return Ok(Some(message));
        }
        let count = available(HANDLE(output.as_raw_handle()))?
            .min(MAX_PAYLOAD + HEADER_BYTES - self.bytes.len());
        if count != 0 {
            let mut chunk = [0; MAX_PAYLOAD + HEADER_BYTES];
            output
                .read_exact(&mut chunk[..count])
                .map_err(|_| CaptureError::WorkerExited)?;
            self.bytes.extend_from_slice(&chunk[..count]);
        }
        self.buffered()
    }

    fn buffered(&mut self) -> Result<Option<(u8, Vec<u8>)>, CaptureError> {
        while !self.synchronized
            && self.bytes.len() >= MAGIC.len()
            && &self.bytes[..MAGIC.len()] != MAGIC
        {
            self.bytes.remove(0);
            self.skipped += 1;
            if self.skipped > MAX_START_PREFIX {
                return Err(CaptureError::WorkerProtocol);
            }
        }
        if self.bytes.len() < HEADER_BYTES {
            return Ok(None);
        }
        if &self.bytes[..8] != MAGIC {
            return Err(CaptureError::WorkerProtocol);
        }
        self.synchronized = true;
        let kind = self.bytes[8];
        let length = u32::from_le_bytes(
            self.bytes[9..13]
                .try_into()
                .map_err(|_| CaptureError::WorkerProtocol)?,
        ) as usize;
        if length > MAX_PAYLOAD {
            return Err(CaptureError::WorkerProtocol);
        }
        if self.bytes.len() < HEADER_BYTES + length {
            return Ok(None);
        }
        let payload = self.bytes[HEADER_BYTES..HEADER_BYTES + length].to_vec();
        self.bytes.drain(..HEADER_BYTES + length);
        Ok(Some((kind, payload)))
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
enum Failure {
    InvalidSource,
    SourceExited,
    Disconnected,
    Native { operation: String, code: i32 },
    ActivationTimeout,
    ActivationPending,
    WorkerStart,
    WorkerPanicked,
    InvalidData(String),
    Protocol,
}
fn encode_error(error: CaptureError) -> Failure {
    match error {
        CaptureError::InvalidSource => Failure::InvalidSource,
        CaptureError::SourceExited => Failure::SourceExited,
        CaptureError::Disconnected => Failure::Disconnected,
        CaptureError::Native { operation, code } => Failure::Native {
            operation: operation.into(),
            code,
        },
        CaptureError::ActivationTimeout => Failure::ActivationTimeout,
        CaptureError::ActivationPending => Failure::ActivationPending,
        CaptureError::WorkerStart => Failure::WorkerStart,
        CaptureError::WorkerPanicked => Failure::WorkerPanicked,
        CaptureError::InvalidData(reason) => Failure::InvalidData(reason.into()),
        _ => Failure::Protocol,
    }
}
fn decode_error(payload: &[u8]) -> Result<CaptureError, CaptureError> {
    if payload.len() > 512 {
        return Err(CaptureError::WorkerProtocol);
    }
    let failure: Failure =
        serde_json::from_slice(payload).map_err(|_| CaptureError::WorkerProtocol)?;
    Ok(match failure {
        Failure::InvalidSource => CaptureError::InvalidSource,
        Failure::SourceExited => CaptureError::SourceExited,
        Failure::Disconnected => CaptureError::Disconnected,
        Failure::ActivationTimeout => CaptureError::ActivationTimeout,
        Failure::ActivationPending => CaptureError::ActivationPending,
        Failure::WorkerStart => CaptureError::WorkerStart,
        Failure::WorkerPanicked => CaptureError::WorkerPanicked,
        Failure::Native { operation, code } => {
            const OPERATIONS: &[&str] = &[
                "OpenProcess",
                "CoInitializeEx",
                "ActivateAudioInterfaceAsync",
                "activation_callback",
                "GetActivateResult",
                "activation_result",
                "IAudioClient",
                "IAudioClient.Initialize",
                "CreateEventW",
                "GetCaptureService",
                "SetEventHandle",
                "IAudioClient.Start",
                "IAudioClient.Stop",
                "QueryPerformanceCounter",
                "QueryPerformanceFrequency",
                "WaitForMultipleObjects",
                "GetNextPacketSize",
                "GetBuffer",
                "ReleaseBuffer",
            ];
            let operation = OPERATIONS
                .iter()
                .copied()
                .find(|known| *known == operation)
                .ok_or(CaptureError::WorkerProtocol)?;
            CaptureError::Native { operation, code }
        }
        Failure::InvalidData(reason) => {
            const REASONS: &[&str] = &[
                "activation_interface",
                "activation_callback_lost",
                "com_initialization",
                "qpc_frequency",
                "qpc_age",
                "qpc_counter",
                "packet_size",
                "packet_pointer",
                "pcm_length",
                "sequence",
                "position",
                "session_id",
            ];
            let reason = REASONS
                .iter()
                .copied()
                .find(|known| *known == reason)
                .ok_or(CaptureError::WorkerProtocol)?;
            CaptureError::InvalidData(reason)
        }
        Failure::Protocol => CaptureError::WorkerProtocol,
    })
}

fn take_u64(payload: &[u8], offset: usize) -> Result<u64, CaptureError> {
    Ok(u64::from_le_bytes(
        payload
            .get(offset..offset + 8)
            .ok_or(CaptureError::WorkerProtocol)?
            .try_into()
            .map_err(|_| CaptureError::WorkerProtocol)?,
    ))
}
fn decode_audio(
    payload: &[u8],
    session: SessionId,
    clock: &CaptureClock,
) -> Result<NormalizedAudio, CaptureError> {
    const METADATA: usize = 49;
    if payload.len() < METADATA || take_u64(payload, 0)? != session.0 {
        return Err(CaptureError::WorkerProtocol);
    }
    let frames = take_u64(payload, 40)? as usize;
    if frames == 0
        || frames > BLOCK_FRAMES
        || payload.len() != METADATA + frames * 4
        || payload[48] > 1
    {
        return Err(CaptureError::WorkerProtocol);
    }
    let position = take_u64(payload, 16)?;
    let qpc = take_u64(payload, 24)?;
    let received_qpc = take_u64(payload, 32)?;
    if received_qpc > clock.now_100ns()? {
        return Err(CaptureError::WorkerProtocol);
    }
    let mut block = NormalizedAudio {
        stamp: AudioStamp {
            session,
            sequence: take_u64(payload, 8)?,
            frame_position: (position != NONE).then_some(position),
            qpc_100ns: (qpc != NONE).then_some(qpc),
            received: clock.received(received_qpc)?,
            discontinuity: payload[48] != 0,
        },
        frames,
        samples: [0.0; BLOCK_FRAMES],
    };
    for (sample, bytes) in block.samples[..frames]
        .iter_mut()
        .zip(payload[METADATA..].as_chunks::<4>().0)
    {
        *sample = f32::from_le_bytes(*bytes);
        if !sample.is_finite() || sample.abs() > 1.0 {
            return Err(CaptureError::WorkerProtocol);
        }
    }
    Ok(block)
}

pub(super) fn receive(target: ProcessTarget, mut sink: CaptureSink) -> Result<(), CaptureError> {
    // Only PID liveness/ownership belongs here. The session-owned helper performs fresh
    // single-process audio-session confirmation before it can initialize native capture.
    if sink.cancelled() {
        return Ok(());
    }
    LIVE_HELPER
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| CaptureError::ActivationPending)?;
    let _permit = Permit;
    target.ensure_alive()?;
    let mut helper = Helper::spawn()?;
    if sink.measurements_enabled() {
        use std::os::windows::io::AsHandle;
        match helper.child.as_handle().try_clone_to_owned() {
            Ok(handle) => {
                sink.attach_resource_probe(super::performance::NativeProcessProbe::owned(handle))
            }
            Err(error) => {
                tracing::warn!(code = error.raw_os_error(), "音訊背景程序資源量測無法啟用")
            }
        }
    }
    let mut hello = [0; HELLO_BYTES];
    hello[..8].copy_from_slice(MAGIC);
    hello[8..12].copy_from_slice(&target.process_id().to_le_bytes());
    hello[12..].copy_from_slice(&sink.session_id().0.to_le_bytes());
    helper
        .input
        .as_mut()
        .ok_or(CaptureError::WorkerProtocol)?
        .write_all(&hello)
        .map_err(|_| CaptureError::WorkerExited)?;
    let clock = CaptureClock::new()?;
    let mut reader = Reader::new();
    let deadline = Instant::now() + START_TIMEOUT;
    let mut ready = false;
    loop {
        if sink.cancelled() {
            return Ok(());
        }
        target.ensure_alive()?;
        match reader.next(helper.output.as_mut().ok_or(CaptureError::WorkerProtocol)?)? {
            Some((READY, payload)) if payload.as_slice() == sink.session_id().0.to_le_bytes() => {
                ready = true;
                sink.started();
            }
            Some((AUDIO, payload)) if ready => {
                sink.push_normalized(decode_audio(&payload, sink.session_id(), &clock)?)?
            }
            Some((ERROR, payload)) => return Err(decode_error(&payload)?),
            Some((DONE, payload)) if payload.is_empty() => return Ok(()),
            Some(_) => return Err(CaptureError::WorkerProtocol),
            None => {
                if helper
                    .child
                    .try_wait()
                    .map_err(|_| CaptureError::WorkerExited)?
                    .is_some()
                {
                    return Err(CaptureError::WorkerExited);
                }
                if !ready && Instant::now() >= deadline {
                    return Err(CaptureError::ActivationTimeout);
                }
                std::thread::park_timeout(POLL);
            }
        }
    }
}

/// Internal entry point. Requires inherited anonymous pipes and a parent hello; no config,
/// cloud provider or source change is allowed. It never outputs PCM to a console or file.
pub fn run() -> Result<(), CaptureError> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    // SAFETY: borrowed standard handles, inspected only. A file/console cannot be a helper.
    if unsafe {
        GetFileType(HANDLE(stdin.as_raw_handle())) != FILE_TYPE_PIPE
            || GetFileType(HANDLE(stdout.as_raw_handle())) != FILE_TYPE_PIPE
    } {
        return Err(CaptureError::WorkerProtocol);
    }
    let mut input = stdin.lock();
    let mut output = stdout.lock();
    let mut hello = [0; HELLO_BYTES];
    input
        .read_exact(&mut hello)
        .map_err(|_| CaptureError::WorkerProtocol)?;
    if &hello[..8] != MAGIC || take_u64(&hello, 12)? == 0 {
        return Err(CaptureError::WorkerProtocol);
    }
    let pid = u32::from_le_bytes(
        hello[8..12]
            .try_into()
            .map_err(|_| CaptureError::WorkerProtocol)?,
    );
    let session_id = SessionId(u64::from_le_bytes(
        hello[12..]
            .try_into()
            .map_err(|_| CaptureError::WorkerProtocol)?,
    ));
    let result = (|| {
        let mut target = ProcessTarget::open(pid)?;
        let mut scanner =
            super::audio::WindowsSessionScanner::new().map_err(|_| CaptureError::InvalidSource)?;
        target.confirm(
            &scanner
                .scan(&AtomicBool::new(false))
                .map_err(|_| CaptureError::InvalidSource)?,
        )?;
        drop(scanner);
        let mut session = CaptureSession::start(move |sink| capture_native(target, sink))?;
        let clock = CaptureClock::new()?;
        let mut ready = false;
        loop {
            match available(HANDLE(stdin.as_raw_handle())) {
                Ok(_) => {}
                Err(CaptureError::WorkerExited) => break,
                Err(error) => return Err(error),
            }
            if !ready && session.state() == CaptureState::Running {
                send(&mut output, READY, &session_id.0.to_le_bytes())?;
                ready = true;
            }
            while let Some(block) = session.try_next() {
                if !ready {
                    send(&mut output, READY, &session_id.0.to_le_bytes())?;
                    ready = true;
                }
                let received_qpc = clock.now_100ns()?.saturating_sub(
                    block
                        .stamp
                        .received
                        .elapsed()
                        .as_nanos()
                        .min(u128::from(u64::MAX)) as u64
                        / 100,
                );
                let mut payload = Vec::with_capacity(49 + BLOCK_FRAMES * 4);
                for value in [
                    session_id.0,
                    block.stamp.sequence,
                    block.stamp.frame_position.unwrap_or(NONE),
                    block.stamp.qpc_100ns.unwrap_or(NONE),
                    received_qpc,
                    block.frames as u64,
                ] {
                    payload.extend_from_slice(&value.to_le_bytes());
                }
                payload.push(u8::from(block.stamp.discontinuity));
                for sample in &block.samples[..block.frames] {
                    payload.extend_from_slice(&sample.to_le_bytes());
                }
                send(&mut output, AUDIO, &payload)?;
            }
            if let CaptureState::Stopped(result) = session.state() {
                result?;
                break;
            }
            std::thread::park_timeout(POLL);
        }
        session.stop()?;
        send(&mut output, DONE, &[])
    })();
    if let Err(error) = result {
        let payload = serde_json::to_vec(&encode_error(error.clone()))
            .map_err(|_| CaptureError::WorkerProtocol)?;
        let _ = send(&mut output, ERROR, &payload);
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragmented_messages_keep_buffered_error_after_ready() {
        let mut wire = Vec::new();
        send(&mut wire, READY, &7_u64.to_le_bytes()).unwrap();
        let split = wire.len() - 1;
        let mut reader = Reader::new();
        reader.bytes.extend_from_slice(b"test harness\n");
        reader.bytes.extend_from_slice(&wire[..split]);
        assert!(reader.buffered().unwrap().is_none());
        reader.bytes.extend_from_slice(&wire[split..]);
        send(&mut reader.bytes, ERROR, b"\"SourceExited\"").unwrap();
        assert_eq!(
            reader.buffered().unwrap(),
            Some((READY, 7_u64.to_le_bytes().to_vec()))
        );
        let (kind, payload) = reader.buffered().unwrap().unwrap();
        assert_eq!(kind, ERROR);
        assert_eq!(decode_error(&payload).unwrap(), CaptureError::SourceExited);
    }

    #[test]
    fn excessive_payload_and_prefix_fail_before_allocation() {
        let mut reader = Reader::new();
        reader.bytes.extend_from_slice(MAGIC);
        reader.bytes.push(AUDIO);
        reader
            .bytes
            .extend_from_slice(&(MAX_PAYLOAD as u32 + 1).to_le_bytes());
        assert_eq!(reader.buffered(), Err(CaptureError::WorkerProtocol));
        let mut reader = Reader::new();
        reader.skipped = MAX_START_PREFIX;
        reader.bytes.extend_from_slice(b"notmagic");
        assert_eq!(reader.buffered(), Err(CaptureError::WorkerProtocol));
    }

    #[test]
    fn audio_payload_rejects_wrong_session_size_time_and_nonfinite_values() {
        let clock = CaptureClock::new().unwrap();
        let mut payload = Vec::new();
        for value in [
            7,
            1,
            48_000,
            clock.now_100ns().unwrap(),
            clock.now_100ns().unwrap(),
            1,
        ] {
            payload.extend_from_slice(&value.to_le_bytes());
        }
        payload.push(0);
        payload.extend_from_slice(&0.25_f32.to_le_bytes());
        let audio = decode_audio(&payload, SessionId(7), &clock).unwrap();
        assert_eq!(audio.samples[0], 0.25);
        assert!(matches!(
            decode_audio(&payload, SessionId(8), &clock),
            Err(CaptureError::WorkerProtocol)
        ));
        payload[40..48].copy_from_slice(&0_u64.to_le_bytes());
        assert!(matches!(
            decode_audio(&payload, SessionId(7), &clock),
            Err(CaptureError::WorkerProtocol)
        ));
        payload[40..48].copy_from_slice(&1_u64.to_le_bytes());
        payload[49..].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(matches!(
            decode_audio(&payload, SessionId(7), &clock),
            Err(CaptureError::WorkerProtocol)
        ));
        payload[49..].copy_from_slice(&0.25_f32.to_le_bytes());
        payload[32..40].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(matches!(
            decode_audio(&payload, SessionId(7), &clock),
            Err(CaptureError::WorkerProtocol)
        ));
    }

    #[test]
    fn worker_error_preserves_native_stage_and_rejects_unknown_operations() {
        let error = CaptureError::Native {
            operation: "GetBuffer",
            code: -1,
        };
        let payload = serde_json::to_vec(&encode_error(error.clone())).unwrap();
        assert_eq!(decode_error(&payload).unwrap(), error);
        let payload = serde_json::to_vec(&Failure::Native {
            operation: "unknown".into(),
            code: -1,
        })
        .unwrap();
        assert_eq!(decode_error(&payload), Err(CaptureError::WorkerProtocol));
    }

    #[test]
    fn helper_rejects_a_live_process_without_an_audio_session() {
        let _serial = super::super::capture::ACTIVATION_TEST_LOCK.lock().unwrap();
        // Our first helper blocks on hello and cannot create an audio session. It is a
        // known live, owned process: the capture facade must not treat OpenProcess as
        // proof of an audio source, or publish Running before the second helper validates it.
        let source = Helper::spawn().unwrap();
        let target = ProcessTarget::open(source.child.id()).unwrap();
        let mut session =
            CaptureSession::start_normalized(move |sink| receive(target, sink)).unwrap();
        let deadline = Instant::now() + START_TIMEOUT;
        loop {
            match session.state() {
                CaptureState::Starting => {}
                CaptureState::Running => panic!("non-audio process must never start capture"),
                CaptureState::Stopped(result) => {
                    assert_eq!(result, Err(CaptureError::InvalidSource));
                    break;
                }
            }
            assert!(Instant::now() < deadline);
            std::thread::park_timeout(POLL);
        }
        assert_eq!(session.stop(), Err(CaptureError::InvalidSource));
        assert_eq!(session.metrics().captured_frames, 0);
        assert!(session.try_next().is_none());
        // SAFETY: this is the source process we own; helper failure must not kill it.
        assert_eq!(
            unsafe {
                windows::Win32::System::Threading::WaitForSingleObject(
                    HANDLE(source.child.as_raw_handle()),
                    0,
                )
            },
            windows::Win32::Foundation::WAIT_TIMEOUT
        );
    }

    #[test]
    fn cancelling_unstarted_helper_reaps_only_its_process() {
        use windows::Win32::{
            Foundation::WAIT_OBJECT_0,
            System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
        };
        let helper = Helper::spawn().unwrap();
        // SAFETY: opens only our new helper for synchronization, not the selected source.
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, helper.child.id()) }.unwrap();
        // SAFETY: successful owning process handle transfers exactly once.
        let handle = unsafe { OwnedHandle::from_raw_handle(handle.0) };
        let before = Instant::now();
        drop(helper);
        assert!(before.elapsed() < Duration::from_secs(2));
        // SAFETY: owned process handle and zero-timeout liveness check.
        assert_eq!(
            unsafe { WaitForSingleObject(HANDLE(handle.as_raw_handle()), 0) },
            WAIT_OBJECT_0
        );
    }

    #[test]
    fn closing_job_kills_helper_even_with_control_pipe_open() {
        use windows::Win32::{Foundation::WAIT_OBJECT_0, System::Threading::WaitForSingleObject};
        let mut helper = Helper::spawn().unwrap();
        assert!(helper.input.is_some());
        // No hello was sent and stdin remains open, so EOF cannot explain this exit.
        drop(helper.job.take());
        // SAFETY: process handle belongs to our spawned helper; bounded termination wait.
        assert_eq!(
            unsafe { WaitForSingleObject(HANDLE(helper.child.as_raw_handle()), 1_000) },
            WAIT_OBJECT_0
        );
        helper.child.wait().unwrap();
    }
}
