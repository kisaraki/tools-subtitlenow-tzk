//! Process-tree WASAPI loopback. No endpoint/system-mix fallback exists.

use super::resources::Apartment;
use crate::audio::{
    capture::{CaptureError, CaptureSink, SAMPLE_RATE},
    sessions::{ProcessScope, ScanReport},
};
use std::{
    mem::{ManuallyDrop, size_of},
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, RecvTimeoutError, SyncSender},
    },
    time::{Duration, Instant},
};
use windows::{
    Wdk::System::SystemServices::RtlGetVersion,
    Win32::{
        Foundation::{HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT},
        Media::Audio::*,
        System::{
            Com::{
                BLOB,
                StructuredStorage::{
                    PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
                },
            },
            Performance::{QueryPerformanceCounter, QueryPerformanceFrequency},
            SystemInformation::OSVERSIONINFOW,
            Threading::{
                CreateEventW, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
                WaitForMultipleObjects, WaitForSingleObject,
            },
            Variant::VT_BLOB,
        },
    },
    core::{HRESULT, Interface, Ref, implement},
};

const WAIT_SLICE_MS: u32 = 20;
const ACTIVATION_TIMEOUT: Duration = Duration::from_secs(10);
/// Cap work per wake even if the engine continuously adds packets.
const MAX_PACKETS_PER_WAKE: usize = 32;
/// Reject packets larger than one second before constructing a PCM slice. Process-loopback
/// GetBufferSize is unreliable on the baseline; use GetBuffer's count and this independent cap.
const MAX_CAPTURE_PACKET_FRAMES: u32 = SAMPLE_RATE;
static ACTIVATION_PENDING: AtomicBool = AtomicBool::new(false);
// Only test fixtures share this lock; it never participates in production capture.
#[cfg(test)]
pub(super) static ACTIVATION_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(super) fn native(operation: &'static str, error: windows::core::Error) -> CaptureError {
    match error.code() {
        AUDCLNT_E_DEVICE_INVALIDATED
        | AUDCLNT_E_RESOURCES_INVALIDATED
        | AUDCLNT_E_SERVICE_NOT_RUNNING => CaptureError::Disconnected,
        code => CaptureError::Native {
            operation,
            code: code.0,
        },
    }
}

fn handle(owned: &OwnedHandle) -> HANDLE {
    HANDLE(owned.as_raw_handle())
}

/// Pin the process before refreshing discovery: a held process handle prevents PID reuse.
/// The session helper confirms a fresh single-process audio session before native capture.
pub struct ProcessTarget {
    process: OwnedHandle,
    pid: u32,
    confirmed: bool,
}

impl ProcessTarget {
    pub(crate) fn creation_time(&self) -> Option<u64> {
        super::process::creation_time_for(handle(&self.process)).filter(|created| *created != 0)
    }
    pub fn verify_creation_time(&self, expected: u64) -> Result<(), CaptureError> {
        self.ensure_alive()?;
        if expected == 0
            || super::process::creation_time_for(handle(&self.process)) != Some(expected)
        {
            return Err(CaptureError::InvalidSource);
        }
        Ok(())
    }
    pub(super) fn process_id(&self) -> u32 {
        self.pid
    }
    pub fn open(pid: u32) -> Result<Self, CaptureError> {
        if pid == 0 || pid == std::process::id() {
            return Err(CaptureError::InvalidSource);
        }
        // SAFETY: no borrowed pointers; access is query/synchronize only. Successful handle
        // ownership transfers exactly once to OwnedHandle, which closes it on every exit path.
        let process = unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                false,
                pid,
            )
        }
        .map_err(|error| native("OpenProcess", error))?;
        // SAFETY: OpenProcess returned a valid owning handle, not a pseudo handle.
        let process = unsafe { OwnedHandle::from_raw_handle(process.0) };
        let target = Self {
            process,
            pid,
            confirmed: false,
        };
        target.ensure_alive()?;
        Ok(target)
    }

    pub fn confirm(&mut self, report: &ScanReport) -> Result<(), CaptureError> {
        self.ensure_alive()?;
        if !report.sources.iter().any(|source| {
            source.process_id == self.pid && source.process_scope == ProcessScope::SingleProcess
        }) {
            return Err(CaptureError::InvalidSource);
        }
        self.confirmed = true;
        Ok(())
    }

    pub(crate) fn ensure_alive(&self) -> Result<(), CaptureError> {
        // SAFETY: owned process has SYNCHRONIZE access and is held across the zero-time wait.
        match unsafe { WaitForSingleObject(handle(&self.process), 0) } {
            WAIT_TIMEOUT => Ok(()),
            WAIT_OBJECT_0 => Err(CaptureError::SourceExited),
            _ => Err(native(
                "WaitForSingleObject",
                windows::core::Error::from_thread(),
            )),
        }
    }
}

struct ActivationPermit;
impl ActivationPermit {
    fn acquire() -> Result<Self, CaptureError> {
        ACTIVATION_PENDING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| CaptureError::ActivationPending)?;
        Ok(Self)
    }
}
impl Drop for ActivationPermit {
    fn drop(&mut self) {
        ACTIVATION_PENDING.store(false, Ordering::Release);
    }
}

/// Private transfer between the activation callback and the capture worker in the same MTA.
struct MtaActivationOperation(IActivateAudioInterfaceAsyncOperation);

// SAFETY: Windows invokes ActivateCompleted in the process MTA, and the only receiver is
// activate(), called after this worker's Apartment::new(). Threads in one MTA can directly
// transfer interface references without marshaling. Only the receiving worker retrieves,
// initializes, starts, reads and stops the client; a cancelled operation is released in MTA.
// This private wrapper never reaches an STA/UI thread or a generic application channel.
unsafe impl Send for MtaActivationOperation {}

// windows::implement supplies IAgileObject and a free-threaded marshaler for the callback.
#[implement(IActivateAudioInterfaceCompletionHandler)]
struct Completion {
    sender: SyncSender<Result<MtaActivationOperation, CaptureError>>,
    _params: Box<AUDIOCLIENT_ACTIVATION_PARAMS>,
    _permit: ActivationPermit,
}

/// Build a non-owning VT_BLOB. The caller keeps params alive until Windows releases
/// the callback, and must not convert this wrapper into an owning PROPVARIANT.
fn borrowed_activation_variant(
    params: &AUDIOCLIENT_ACTIVATION_PARAMS,
) -> ManuallyDrop<PROPVARIANT> {
    // windows 0.62 adds PROPVARIANT::Drop in its extensions module. Suppressing only
    // the inner union's Drop would still PropVariantClear this borrowed Box allocation.
    ManuallyDrop::new(PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_BLOB,
                Anonymous: PROPVARIANT_0_0_0 {
                    blob: BLOB {
                        cbSize: size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
                        pBlobData: (params as *const AUDIOCLIENT_ACTIVATION_PARAMS)
                            .cast_mut()
                            .cast(),
                    },
                },
                ..Default::default()
            }),
        },
    })
}

impl IActivateAudioInterfaceCompletionHandler_Impl for Completion_Impl {
    fn ActivateCompleted(
        &self,
        operation: Ref<IActivateAudioInterfaceAsyncOperation>,
    ) -> windows::core::Result<()> {
        let result = operation
            .ok()
            .map(|operation| MtaActivationOperation(operation.clone()))
            .map_err(|error| native("activation_callback", error));
        // On cancellation the receiver is gone: release the result here, never start it.
        let _ = self.sender.try_send(result);
        Ok(())
    }
}

fn completed_audio_client(
    operation: &IActivateAudioInterfaceAsyncOperation,
) -> Result<IAudioClient, CaptureError> {
    let mut result = HRESULT(0);
    let mut activated = None;
    // SAFETY: owning completed operation and initialized output pointers, called by the
    // capture worker in its MTA. The audio client is first retrieved and used on this worker.
    unsafe { operation.GetActivateResult(&mut result, &mut activated) }
        .map_err(|error| native("GetActivateResult", error))?;
    result
        .ok()
        .map_err(|error| native("activation_result", error))?;
    activated
        .ok_or(CaptureError::InvalidData("activation_interface"))?
        .cast()
        .map_err(|error| native("IAudioClient", error))
}

fn activate(
    target: &ProcessTarget,
    sink: &CaptureSink,
) -> Result<Option<IAudioClient>, CaptureError> {
    if sink.cancelled() {
        return Ok(None);
    }
    target.ensure_alive()?;
    let permit = ActivationPermit::acquire()?;
    let params = Box::new(AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: target.pid,
                ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
            },
        },
    });
    let prop = borrowed_activation_variant(&params);
    let (sender, receiver) = mpsc::sync_channel(1);
    let completion: IActivateAudioInterfaceCompletionHandler = Completion {
        sender,
        _params: params,
        _permit: permit,
    }
    .into();
    if sink.cancelled() {
        return Ok(None);
    }
    target.ensure_alive()?;
    // SAFETY: VT_BLOB describes the exact activation struct. Box storage stays stable and is
    // owned by the callback until Windows releases it, including late completion after stop.
    // PROPVARIANT borrows this storage; never PropVariantClear it (not a CoTaskMem allocation).
    let _operation = unsafe {
        ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some(&*prop),
            &completion,
        )
    }
    .map_err(|error| native("ActivateAudioInterfaceAsync", error))?;
    let deadline = Instant::now() + ACTIVATION_TIMEOUT;
    loop {
        if sink.cancelled() {
            return Ok(None);
        }
        target.ensure_alive()?;
        match receiver.recv_timeout(Duration::from_millis(u64::from(WAIT_SLICE_MS))) {
            Ok(result) => {
                // GetActivateResult/QI give us an owning client reference. Release the
                // activation operation here; it must not retain another client reference
                // beyond the capture client's event/COM-resource lifetime below.
                let completed = result?;
                if sink.cancelled() {
                    return Ok(None);
                }
                return completed_audio_client(&completed.0).map(Some);
            }
            Err(RecvTimeoutError::Timeout) if Instant::now() < deadline => {}
            Err(RecvTimeoutError::Timeout) => return Err(CaptureError::ActivationTimeout),
            Err(RecvTimeoutError::Disconnected) => {
                return Err(CaptureError::InvalidData("activation_callback_lost"));
            }
        }
    }
}

struct RunningClient(IAudioClient, bool);
impl Drop for RunningClient {
    fn drop(&mut self) {
        // SAFETY: same MTA that initialized/started this client, before its COM apartment drops.
        if self.1 {
            let _ = unsafe { self.0.Stop() };
        }
    }
}

pub(super) struct CaptureClock(u64);

impl CaptureClock {
    pub(super) fn new() -> Result<Self, CaptureError> {
        let mut frequency = 0;
        // SAFETY: valid writable output; Windows QPC frequency is fixed for this boot.
        unsafe { QueryPerformanceFrequency(&mut frequency) }
            .map_err(|error| native("QueryPerformanceFrequency", error))?;
        if frequency <= 0 {
            return Err(CaptureError::InvalidData("qpc_frequency"));
        }
        Ok(Self(frequency as u64))
    }

    pub(super) fn now_100ns(&self) -> Result<u64, CaptureError> {
        let mut ticks = 0;
        // SAFETY: valid initialized writable counter, no audio resources opened.
        unsafe { QueryPerformanceCounter(&mut ticks) }
            .map_err(|error| native("QueryPerformanceCounter", error))?;
        if ticks < 0 {
            return Err(CaptureError::InvalidData("qpc_counter"));
        }
        u64::try_from(ticks as u128 * 10_000_000 / u128::from(self.0))
            .map_err(|_| CaptureError::InvalidData("qpc_counter"))
    }

    pub(super) fn received(&self, packet_qpc: u64) -> Result<Instant, CaptureError> {
        let mut ticks = 0;
        // SAFETY: valid writable output, safe on the capture MTA and requires no allocation.
        unsafe { QueryPerformanceCounter(&mut ticks) }
            .map_err(|error| native("QueryPerformanceCounter", error))?;
        if ticks < 0 {
            return Err(CaptureError::InvalidData("qpc_counter"));
        }
        let now = Instant::now();
        let current = u64::try_from(ticks as u128 * 10_000_000 / u128::from(self.0))
            .map_err(|_| CaptureError::InvalidData("qpc_overflow"))?;
        let age = current.saturating_sub(packet_qpc);
        let elapsed = Duration::new(age / 10_000_000, (age % 10_000_000) as u32 * 100);
        now.checked_sub(elapsed)
            .ok_or(CaptureError::InvalidData("qpc_age"))
    }
}

/// Diagnostic information only; a build number does not prove process-loopback capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowsVersion {
    pub major: u32,
    pub minor: u32,
    pub build: u32,
}

/// No audio interfaces or process handles are opened here.
pub fn windows_version() -> Result<WindowsVersion, CaptureError> {
    let mut version = OSVERSIONINFOW {
        dwOSVersionInfoSize: size_of::<OSVERSIONINFOW>() as u32,
        ..Default::default()
    };
    // SAFETY: correctly sized, writable OSVERSIONINFOW; avoids manifest-dependent version lies.
    unsafe { RtlGetVersion(&mut version) }
        .ok()
        .map_err(|error| native("RtlGetVersion", error))?;
    Ok(WindowsVersion {
        major: version.dwMajorVersion,
        minor: version.dwMinorVersion,
        build: version.dwBuildNumber,
    })
}

pub fn capture(target: ProcessTarget, sink: CaptureSink) -> Result<(), CaptureError> {
    super::capture_helper::receive(target, sink)
}

pub(super) fn capture_native(
    target: ProcessTarget,
    mut sink: CaptureSink,
) -> Result<(), CaptureError> {
    if !target.confirmed {
        return Err(CaptureError::InvalidSource);
    }
    if sink.cancelled() {
        return Ok(());
    }
    match windows_version() {
        Ok(version) => tracing::debug!(
            windows_major = version.major,
            windows_minor = version.minor,
            windows_build = version.build,
            "Starting selected process-loopback capture"
        ),
        Err(error) => tracing::debug!(%error, "Windows version diagnostic unavailable"),
    }
    let _apartment = Apartment::new().map_err(|error| match error {
        crate::audio::sessions::DiscoveryError::Native { operation, code } => {
            CaptureError::Native { operation, code }
        }
        _ => CaptureError::InvalidData("com_initialization"),
    })?;
    let Some(client) = activate(&target, &sink)? else {
        return Ok(());
    };
    if sink.cancelled() {
        return Ok(());
    }
    target.ensure_alive()?;
    // PCM conversion belongs to the audio engine; normalize stereo i16 to mono f32 on our
    // separate worker. Never interpret an unknown mix format as PCM.
    let format = WAVEFORMATEX {
        wFormatTag: 1,
        nChannels: 2,
        nSamplesPerSec: SAMPLE_RATE,
        nAvgBytesPerSec: SAMPLE_RATE * 4,
        nBlockAlign: 4,
        wBitsPerSample: 16,
        cbSize: 0,
    };
    // SAFETY: client was transferred within this MTA. Format lives through Initialize, event is an
    // owned auto-reset event and stays alive until the client has stopped and been released.
    let event = unsafe { CreateEventW(None, false, false, None) }
        .map_err(|error| native("CreateEventW", error))?;
    // SAFETY: new successful CreateEventW handle transfers ownership once.
    let event = unsafe { OwnedHandle::from_raw_handle(event.0) };
    // Declare client guard after event so COM releases its event reference before CloseHandle.
    let mut client = RunningClient(client, false);
    let clock = CaptureClock::new()?;
    let mut loopback_clock = super::loopback_clock::LoopbackClock::default();
    // SAFETY: all COM calls stay on this worker; resources outlive every call and packet.
    unsafe {
        client
            .0
            .Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_LOOPBACK
                    | AUDCLNT_STREAMFLAGS_EVENTCALLBACK
                    | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                    | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
                0,
                0,
                &format,
                None,
            )
            .map_err(|error| native("IAudioClient.Initialize", error))?;
        let capture: IAudioCaptureClient = client
            .0
            .GetService()
            .map_err(|error| native("GetCaptureService", error))?;
        client
            .0
            .SetEventHandle(handle(&event))
            .map_err(|error| native("SetEventHandle", error))?;
        if sink.cancelled() {
            return Ok(());
        }
        client
            .0
            .Start()
            .map_err(|error| native("IAudioClient.Start", error))?;
        client.1 = true;
        sink.started();
        while !sink.cancelled() {
            let waited = WaitForMultipleObjects(
                &[handle(&target.process), handle(&event)],
                false,
                WAIT_SLICE_MS,
            );
            if waited == WAIT_OBJECT_0 {
                return Err(CaptureError::SourceExited);
            }
            if waited != WAIT_TIMEOUT && waited.0 != WAIT_OBJECT_0.0 + 1 {
                return Err(native(
                    "WaitForMultipleObjects",
                    windows::core::Error::from_thread(),
                ));
            }
            // Also poll on timeout: detect audio-service invalidation when event delivery stops.
            for _ in 0..MAX_PACKETS_PER_WAKE {
                if sink.cancelled() {
                    break;
                }
                let packet_size = capture
                    .GetNextPacketSize()
                    .map_err(|error| native("GetNextPacketSize", error))?;
                if packet_size == 0 {
                    break;
                }
                read_packet(
                    &capture,
                    &mut sink,
                    MAX_CAPTURE_PACKET_FRAMES,
                    &clock,
                    &mut loopback_clock,
                )?;
            }
        }
        client.1 = false;
        client
            .0
            .Stop()
            .map_err(|error| native("IAudioClient.Stop", error))?;
    }
    Ok(())
}

fn read_packet(
    capture: &IAudioCaptureClient,
    sink: &mut CaptureSink,
    buffer_frames: u32,
    clock: &CaptureClock,
    loopback_clock: &mut super::loopback_clock::LoopbackClock,
) -> Result<(), CaptureError> {
    let (mut data, mut frames, mut flags, mut position, mut qpc) =
        (std::ptr::null_mut(), 0, 0, 0, 0);
    // SAFETY: live capture service in owning MTA, all output pointers valid. GetBuffer and
    // ReleaseBuffer always run on this thread. No other method holds an outstanding packet.
    unsafe {
        capture.GetBuffer(
            &mut data,
            &mut frames,
            &mut flags,
            Some(&mut position),
            Some(&mut qpc),
        )
    }
    .map_err(|error| native("GetBuffer", error))?;
    if frames == 0 {
        return Ok(());
    }
    struct Packet<'a>(&'a IAudioCaptureClient, u32);
    impl Drop for Packet<'_> {
        fn drop(&mut self) {
            // SAFETY: owns the one outstanding nonempty packet; also releases on panic/error.
            let _ = unsafe { self.0.ReleaseBuffer(self.1) };
        }
    }
    let packet = Packet(capture, frames);
    if frames > buffer_frames {
        return Err(CaptureError::InvalidData("packet_size"));
    }
    let silent = flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0;
    let pcm = if silent {
        None
    } else {
        if data.is_null() || !(data as usize).is_multiple_of(std::mem::align_of::<i16>()) {
            return Err(CaptureError::InvalidData("packet_pointer"));
        }
        // SAFETY: Initialize negotiated stereo PCM16; GetBuffer guarantees frames * 4 valid
        // bytes until ReleaseBuffer. Count is checked against the bounded packet limit.
        Some(unsafe { std::slice::from_raw_parts(data.cast::<i16>(), frames as usize * 2) })
    };
    let reliable = flags & AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0 as u32 == 0;
    let timestamp = loopback_clock.observe(
        position,
        reliable.then_some(qpc),
        frames,
        flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32 != 0,
    )?;
    let received = if let Some(qpc) = timestamp.qpc {
        clock.received(qpc)?
    } else {
        Instant::now()
    };
    let result = sink.push_pcm_received(
        pcm,
        frames as usize,
        timestamp.position,
        timestamp.qpc,
        timestamp.discontinuity,
        received,
    );
    // SAFETY: same owning thread, packet has not been released. Suppress guard's second release.
    let released =
        unsafe { capture.ReleaseBuffer(frames) }.map_err(|error| native("ReleaseBuffer", error));
    std::mem::forget(packet);
    result.and(released)
}

#[cfg(test)]
#[path = "capture_packet_tests.rs"]
mod packet_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use windows::core::AgileReference;

    #[test]
    fn borrowed_activation_blob_survives_variant_scope_and_owned_copy_cleanup() {
        let _apartment = Apartment::new().unwrap();
        let params = Box::new(AUDIOCLIENT_ACTIVATION_PARAMS {
            ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
            Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
                ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                    TargetProcessId: 42,
                    ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
                },
            },
        });
        {
            let borrowed = borrowed_activation_variant(&params);
            let copy = (*borrowed).clone();
            // SAFETY: both variants explicitly contain VT_BLOB; the owned copy was created
            // by PropVariantCopy and must use a different allocation from our borrowed Box.
            unsafe {
                assert_eq!(copy.Anonymous.Anonymous.vt, VT_BLOB);
                let blob = copy.Anonymous.Anonymous.Anonymous.blob;
                assert_eq!(
                    blob.cbSize as usize,
                    size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>()
                );
                assert_ne!(
                    blob.pBlobData,
                    (&*params as *const AUDIOCLIENT_ACTIVATION_PARAMS)
                        .cast_mut()
                        .cast()
                );
                assert_eq!(
                    (*blob.pBlobData.cast::<AUDIOCLIENT_ACTIVATION_PARAMS>()).ActivationType,
                    params.ActivationType
                );
            }
            drop(copy); // only the copied CoTaskMem allocation may be cleared
        }
        assert_eq!(
            params.ActivationType,
            AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK
        );
        // SAFETY: the initialized activation type selects this union member. Its Box is
        // still owned here after both variant lifetimes end and will be freed exactly once.
        assert_eq!(
            unsafe { params.Anonymous.ProcessLoopbackParams.TargetProcessId },
            42
        );
    }

    #[test]
    fn cancellation_before_native_start_does_not_acquire_activation_or_capture() {
        use crate::audio::capture::{CaptureSession, CaptureState};
        let _serial = ACTIVATION_TEST_LOCK.lock().unwrap();
        let _permit = ActivationPermit::acquire().unwrap();
        // SAFETY: test-owned handle to this process with read/synchronize access only.
        let raw = unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                false,
                std::process::id(),
            )
        }
        .unwrap();
        // SAFETY: successful owning OpenProcess handle, transferred once to OwnedHandle.
        let process = unsafe { OwnedHandle::from_raw_handle(raw.0) };
        let target = ProcessTarget {
            process,
            pid: std::process::id(),
            confirmed: true,
        };
        let created = super::super::process::creation_time(std::process::id()).unwrap();
        target.verify_creation_time(created).unwrap();
        assert_eq!(
            target.verify_creation_time(created + 1),
            Err(CaptureError::InvalidSource)
        );
        let mut session = CaptureSession::start(move |sink| {
            while !sink.cancelled() {
                std::thread::park_timeout(Duration::from_millis(1));
            }
            capture(target, sink)
        })
        .unwrap();
        session.stop().unwrap();
        assert_eq!(session.state(), CaptureState::Stopped(Ok(())));
        assert_eq!(session.metrics().captured_frames, 0);
        assert!(matches!(
            ActivationPermit::acquire(),
            Err(CaptureError::ActivationPending)
        ));
    }

    #[implement(IActivateAudioInterfaceAsyncOperation)]
    struct FailedActivation {
        call_error: Option<HRESULT>,
        result_error: HRESULT,
    }

    impl IActivateAudioInterfaceAsyncOperation_Impl for FailedActivation_Impl {
        fn GetActivateResult(
            &self,
            result: *mut HRESULT,
            interface: windows::core::OutRef<windows::core::IUnknown>,
        ) -> windows::core::Result<()> {
            if let Some(error) = self.call_error {
                return Err(error.into());
            }
            if result.is_null() {
                return Err(windows::Win32::Foundation::E_POINTER.into());
            }
            // SAFETY: our callback supplies a valid initialized HRESULT output.
            unsafe {
                *result = self.result_error;
            }
            interface.write(None)
        }
    }

    #[test]
    fn activation_callback_preserves_call_and_completion_error_stages() {
        use windows::Win32::Foundation::{E_ACCESSDENIED, E_FAIL};
        let _serial = ACTIVATION_TEST_LOCK.lock().unwrap();
        let _apartment = Apartment::new().unwrap();
        for (call_error, stage, code) in [
            (Some(E_FAIL), "GetActivateResult", E_FAIL),
            (None, "activation_result", E_ACCESSDENIED),
        ] {
            let (sender, receiver) = mpsc::sync_channel(1);
            let handler: IActivateAudioInterfaceCompletionHandler = Completion {
                sender,
                _params: Box::default(),
                _permit: ActivationPermit::acquire().unwrap(),
            }
            .into();
            let operation: IActivateAudioInterfaceAsyncOperation = FailedActivation {
                call_error,
                result_error: E_ACCESSDENIED,
            }
            .into();
            // SAFETY: explicit mock operation and callback in this MTA; no audio API runs.
            unsafe { handler.ActivateCompleted(&operation) }.unwrap();
            assert!(
                matches!(receiver.recv_timeout(Duration::from_secs(1)).unwrap().and_then(|completed| completed_audio_client(&completed.0)), Err(CaptureError::Native { operation, code: actual }) if operation == stage && actual == code.0)
            );
            drop(handler);
        }
    }

    #[test]
    fn device_and_service_invalidations_require_explicit_restart() {
        for code in [
            AUDCLNT_E_DEVICE_INVALIDATED,
            AUDCLNT_E_RESOURCES_INVALIDATED,
            AUDCLNT_E_SERVICE_NOT_RUNNING,
        ] {
            assert_eq!(
                native("capture", windows::core::Error::from(code)),
                CaptureError::Disconnected
            );
        }
        assert!(matches!(
            native(
                "capture",
                windows::core::Error::from(windows::Win32::Foundation::E_ACCESSDENIED)
            ),
            CaptureError::Native { .. }
        ));
        assert!(matches!(
            ProcessTarget::open(0),
            Err(CaptureError::InvalidSource)
        ));
        assert!(matches!(
            ProcessTarget::open(std::process::id()),
            Err(CaptureError::InvalidSource)
        ));
    }

    #[test]
    fn activation_permit_and_late_callback_are_bounded() {
        let _serial = ACTIVATION_TEST_LOCK.lock().unwrap();
        let _apartment = Apartment::new().unwrap();
        let permit = ActivationPermit::acquire().unwrap();
        assert!(matches!(
            ActivationPermit::acquire(),
            Err(CaptureError::ActivationPending)
        ));
        let (sender, receiver) = mpsc::sync_channel(1);
        let handler: IActivateAudioInterfaceCompletionHandler = Completion {
            sender,
            _params: Box::default(),
            _permit: permit,
        }
        .into();
        // The cancelled worker has no receiver. The late callback must discard its result
        // without starting capture, blocking, or retaining the activation permit forever.
        drop(receiver);
        let marshaled = AgileReference::new(&handler).unwrap();
        std::thread::spawn(move || {
            let _apartment = Apartment::new().unwrap();
            let callback = marshaled.resolve().unwrap();
            // SAFETY: explicit test invocation on an MTA, with an absent operation to exercise
            // the error path; our callback handles the optional COM reference before any use.
            unsafe { callback.ActivateCompleted(None::<&IActivateAudioInterfaceAsyncOperation>) }
                .unwrap();
        })
        .join()
        .unwrap();
        assert!(matches!(
            ActivationPermit::acquire(),
            Err(CaptureError::ActivationPending)
        ));
        drop(handler);
        let next = ActivationPermit::acquire().unwrap();
        drop(next);
    }
}
