//! Explicit local hardware tests. Silence and low-level synthetic tones stay in RAM.

use std::{
    io::{BufRead, BufReader, Read, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{atomic::AtomicBool, mpsc},
    time::{Duration, Instant},
};

use windows::{
    Win32::{
        Media::Audio::{
            AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_NOPERSIST,
            DEVICE_STATE_ACTIVE, IAudioClient, IAudioRenderClient, IAudioSessionControl,
            IAudioSessionControl2, IMMDeviceEnumerator, MMDeviceEnumerator, WAVEFORMATEXTENSIBLE,
            eConsole, eRender,
        },
        System::Com::{CLSCTX_ALL, CoCreateGuid, CoCreateInstance, CoTaskMemFree},
    },
    core::Interface,
};

use super::{audio::WindowsSessionScanner, resources::take_string};
use crate::audio::{
    discovery::AudioSessionScanner,
    sessions::{AudioSourceId, ScanReport, SessionState},
};

// PCM16 format conversion can dither silence by one LSB per channel. Allow two LSBs
// for rounding while retaining a very low (-84 dBFS) silence threshold.
const SILENCE_PCM16_TOLERANCE: f32 = 2.0 / 32768.0;
const FIXTURE_TIMEOUT: Duration = Duration::from_secs(5);
const FIXTURE_POLL: Duration = Duration::from_millis(10);
const MAX_FIXTURE_LINE_BYTES: usize = 4096;
const TONE_AMPLITUDE: f64 = 1.0 / 32.0; // about -30 dBFS; no system volume changes
const TARGET_FREQUENCY: f64 = 440.0;
const UNRELATED_FREQUENCY: f64 = 880.0;
const CHILD_FREQUENCY: f64 = 1320.0;

struct SilentStream {
    client: IAudioClient,
    render: IAudioRenderClient,
    buffer_frames: u32,
    channels: usize,
    sample_rate: u32,
    bits: u16,
    is_float: bool,
    phase: u64,
}

impl SilentStream {
    fn new() -> (Self, AudioSourceId) {
        Self::on_endpoint(None)
    }

    fn on_endpoint(endpoint_id: Option<&str>) -> (Self, AudioSourceId) {
        // SAFETY: the test creates a scanner first, initializing MTA on this thread. Every
        // COM object and format allocation stays here and drops before the scanner apartment.
        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).unwrap();
            let endpoint = match endpoint_id {
                Some(id) => enumerator
                    .GetDevice(&windows::core::HSTRING::from(id))
                    .unwrap(),
                None => enumerator
                    .GetDefaultAudioEndpoint(eRender, eConsole)
                    .expect("hardware test requires a default playback device"),
            };
            let client: IAudioClient = endpoint.Activate(CLSCTX_ALL, None).unwrap();
            let format = client.GetMixFormat().unwrap();
            let channels = usize::from((*format).nChannels);
            let sample_rate = (*format).nSamplesPerSec;
            let bits = (*format).wBitsPerSample;
            let is_float = (*format).wFormatTag == 3
                || ((*format).wFormatTag == 0xfffe
                    && usize::from((*format).cbSize)
                        >= std::mem::size_of::<WAVEFORMATEXTENSIBLE>()
                            - std::mem::size_of::<windows::Win32::Media::Audio::WAVEFORMATEX>()
                    && {
                        let sub_format = (*format.cast::<WAVEFORMATEXTENSIBLE>()).SubFormat;
                        sub_format
                            == windows::core::GUID::from_u128(
                                0x00000003_0000_0010_8000_00aa00389b71,
                            )
                    });
            let guid = CoCreateGuid().unwrap();
            // 100 ms in 100 ns units; NOPERSIST prevents persisting this test session's settings.
            let initialized = client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_NOPERSIST,
                1_000_000,
                0,
                format,
                Some(&guid),
            );
            CoTaskMemFree(Some(format.cast()));
            initialized.unwrap();
            let render: IAudioRenderClient = client.GetService().unwrap();
            let frames = client.GetBufferSize().unwrap();
            render.GetBuffer(frames).unwrap();
            // SILENT means the buffer is never read by us and no audible test tone is played.
            render
                .ReleaseBuffer(frames, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32)
                .unwrap();
            let session: IAudioSessionControl = client.GetService().unwrap();
            let session: IAudioSessionControl2 = session.cast().unwrap();
            let id = AudioSourceId {
                endpoint: take_string(endpoint.GetId().unwrap()).unwrap(),
                instance: take_string(session.GetSessionInstanceIdentifier().unwrap()).unwrap(),
            };
            client.Start().unwrap();
            (
                Self {
                    client,
                    render,
                    buffer_frames: frames,
                    channels,
                    sample_rate,
                    bits,
                    is_float,
                    phase: 0,
                },
                id,
            )
        }
    }

    fn stop(&self) {
        // SAFETY: owned render client, same MTA as Initialize/Start.
        unsafe { self.client.Stop() }.unwrap();
    }

    fn pump(&mut self, frequency: Option<f64>) {
        // SAFETY: the initialized render client is used only on this MTA. Available frames
        // are obtained from its padding, and SILENT prevents reading uninitialized memory.
        unsafe {
            let available = self.buffer_frames - self.client.GetCurrentPadding().unwrap();
            if available != 0 {
                let data = self.render.GetBuffer(available).unwrap();
                if let Some(frequency) = frequency {
                    assert!(self.channels > 0 && self.sample_rate > 0);
                    assert!(
                        (self.is_float && self.bits == 32) || (!self.is_float && self.bits == 16),
                        "tone fixture requires float32 or PCM16 mix format"
                    );
                    assert!(!data.is_null());
                    for frame in 0..available as usize {
                        let value = TONE_AMPLITUDE
                            * (std::f64::consts::TAU * frequency * self.phase as f64
                                / f64::from(self.sample_rate))
                            .sin();
                        for channel in 0..self.channels {
                            let offset = frame * self.channels + channel;
                            // SAFETY: GetBuffer supplies available * channels * sample_bytes
                            // writable bytes for the negotiated mix format until ReleaseBuffer.
                            // Packed formats are written unaligned; every channel is initialized.
                            if self.is_float {
                                data.cast::<f32>().add(offset).write_unaligned(value as f32);
                            } else {
                                data.cast::<i16>()
                                    .add(offset)
                                    .write_unaligned((value * 32767.0).round() as i16);
                            }
                        }
                        self.phase = (self.phase + 1) % u64::from(self.sample_rate);
                    }
                    self.render.ReleaseBuffer(available, 0).unwrap();
                } else {
                    self.render
                        .ReleaseBuffer(available, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32)
                        .unwrap();
                }
            }
        }
    }
}

impl Drop for SilentStream {
    fn drop(&mut self) {
        // SAFETY: initialized render client on its owning thread; double Stop is harmless.
        let _ = unsafe { self.client.Stop() };
    }
}

/// A real independent audio process: retained discovery controls must not keep it running.
pub(super) struct FixtureProcess {
    child: Child,
    input: ChildStdin,
    output: mpsc::Receiver<String>,
    reader: Option<std::thread::JoinHandle<()>>,
    id: AudioSourceId,
}

impl FixtureProcess {
    pub(super) fn pid(&self) -> u32 {
        self.child.id()
    }

    pub(super) fn start() -> Self {
        Self::start_with_frequency(TARGET_FREQUENCY)
    }

    fn start_with_frequency(frequency: f64) -> Self {
        Self::start_mode(frequency, false)
    }

    fn start_mode(frequency: f64, orphan_grace: bool) -> Self {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "win32::tests::native_session_appears_updates_and_disappears",
                "--ignored",
                "--nocapture",
            ])
            .env("SUBTITLENOW_NATIVE_FIXTURE", "1")
            .env("SUBTITLENOW_TEST_FREQUENCY", frequency.to_string())
            .env(
                "SUBTITLENOW_TEST_ORPHAN_GRACE",
                if orphan_grace { "1" } else { "0" },
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, output) = mpsc::sync_channel(4);
        let reader = std::thread::spawn(move || {
            let mut stdout = BufReader::new(stdout);
            loop {
                let mut line = Vec::new();
                let count = (&mut stdout)
                    .take((MAX_FIXTURE_LINE_BYTES + 1) as u64)
                    .read_until(b'\n', &mut line)
                    .unwrap();
                if count == 0 {
                    break;
                }
                assert!(
                    count <= MAX_FIXTURE_LINE_BYTES,
                    "fixture output line exceeds bound"
                );
                if sender.send(String::from_utf8(line).unwrap()).is_err() {
                    break;
                }
            }
        });
        let mut fixture = Self {
            child,
            input,
            output,
            reader: Some(reader),
            id: AudioSourceId {
                endpoint: String::new(),
                instance: String::new(),
            },
        };
        let line = fixture.read_marker("SUBTITLE_TEST_READY ");
        let (endpoint, instance): (String, String) = serde_json::from_str(&line).unwrap();
        fixture.id = AudioSourceId { endpoint, instance };
        fixture
    }

    fn read_marker(&mut self, prefix: &str) -> String {
        let deadline = Instant::now() + FIXTURE_TIMEOUT;
        loop {
            let line = self
                .output
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("fixture marker timed out or process exited");
            if let Some(value) = line.trim().strip_prefix(prefix) {
                return value.to_owned();
            }
        }
    }

    fn stop_stream(&mut self) {
        self.input.write_all(b"s").unwrap();
        self.input.flush().unwrap();
        self.read_marker("SUBTITLE_TEST_STOPPED");
    }

    pub(super) fn play(&mut self, playing: bool) {
        self.input
            .write_all(if playing { b"a" } else { b"m" })
            .unwrap();
        self.input.flush().unwrap();
        self.read_marker("SUBTITLE_TEST_TONE");
    }

    fn spawn_audio_child(&mut self) -> u32 {
        self.input.write_all(b"c").unwrap();
        self.input.flush().unwrap();
        self.read_marker("SUBTITLE_TEST_CHILD ").parse().unwrap()
    }

    fn add_second_endpoint(&mut self) {
        self.input.write_all(b"d").unwrap();
        self.input.flush().unwrap();
        self.read_marker("SUBTITLE_TEST_SECOND_ENDPOINT");
    }

    fn stop_primary_endpoint(&mut self) {
        self.input.write_all(b"x").unwrap();
        self.input.flush().unwrap();
        self.read_marker("SUBTITLE_TEST_PRIMARY_STOPPED");
    }
}

impl Drop for FixtureProcess {
    fn drop(&mut self) {
        if self
            .input
            .write_all(b"q")
            .and_then(|_| self.input.flush())
            .is_err()
        {
            let _ = self.child.kill();
        }
        let deadline = Instant::now() + FIXTURE_TIMEOUT;
        while matches!(self.child.try_wait(), Ok(None)) && Instant::now() < deadline {
            while self.output.try_recv().is_ok() {}
            std::thread::sleep(FIXTURE_POLL);
        }
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
        // Drain before joining so the bounded stdout mailbox cannot trap its reader.
        while self.output.recv_timeout(FIXTURE_POLL).is_ok() {}
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

fn fixture_main() {
    let _apartment = super::resources::Apartment::new().unwrap();
    let (mut stream, id) = SilentStream::new();
    let preset_frequency: f64 = std::env::var("SUBTITLENOW_TEST_FREQUENCY")
        .unwrap()
        .parse()
        .unwrap();
    assert!([TARGET_FREQUENCY, UNRELATED_FREQUENCY, CHILD_FREQUENCY].contains(&preset_frequency));
    println!(
        "SUBTITLE_TEST_READY {}",
        serde_json::to_string(&(&id.endpoint, &id.instance)).unwrap()
    );
    std::io::stdout().flush().unwrap();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let reader = std::thread::spawn(move || {
        let mut input = std::io::stdin().lock();
        let mut command = [0];
        while input.read_exact(&mut command).is_ok() {
            if sender.send(command[0]).is_err() || command[0] == b'q' {
                break;
            }
        }
    });
    let mut running = true;
    let mut primary_running = true;
    let mut frequency = None;
    let mut child = None;
    let mut second_stream: Option<SilentStream> = None;
    let orphan_grace = std::env::var("SUBTITLENOW_TEST_ORPHAN_GRACE").unwrap() == "1";
    let mut orphan_deadline = None;
    loop {
        if running {
            if primary_running {
                stream.pump(frequency);
            }
            if let Some(second) = &mut second_stream {
                second.pump(frequency.map(|_| UNRELATED_FREQUENCY));
            }
        }
        let command = match receiver.recv_timeout(FIXTURE_POLL) {
            Ok(command) => command,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                if !orphan_grace {
                    break;
                }
                // A test descendant may outlive its root briefly to prove capture stops
                // on root exit. It self-exits within two seconds even if the test crashes.
                let deadline =
                    orphan_deadline.get_or_insert_with(|| Instant::now() + Duration::from_secs(2));
                if Instant::now() >= *deadline {
                    break;
                }
                std::thread::sleep(FIXTURE_POLL);
                continue;
            }
        };
        match command {
            b's' => {
                stream.stop();
                running = false;
                println!("SUBTITLE_TEST_STOPPED");
                std::io::stdout().flush().unwrap();
            }
            b'q' => break,
            b'a' | b'm' => {
                frequency = (command == b'a').then_some(preset_frequency);
                println!("SUBTITLE_TEST_TONE");
                std::io::stdout().flush().unwrap();
            }
            b'c' => {
                assert!(child.is_none(), "fixture already has an audio child");
                let mut fixture = FixtureProcess::start_mode(CHILD_FREQUENCY, true);
                fixture.play(true);
                println!("SUBTITLE_TEST_CHILD {}", fixture.child.id());
                std::io::stdout().flush().unwrap();
                child = Some(fixture);
            }
            b'd' => {
                assert!(second_stream.is_none());
                // SAFETY: fixture owns this MTA. Enumeration only selects an existing
                // second render endpoint for our own stream; system defaults do not change.
                let endpoint_id = unsafe {
                    let enumerator: IMMDeviceEnumerator =
                        CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).unwrap();
                    let endpoints = enumerator
                        .EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)
                        .unwrap();
                    (0..endpoints.GetCount().unwrap())
                        .find_map(|index| {
                            let endpoint = endpoints.Item(index).unwrap();
                            let candidate = take_string(endpoint.GetId().unwrap()).unwrap();
                            (candidate != id.endpoint).then_some(candidate)
                        })
                        .expect("test requires two active playback endpoints")
                };
                second_stream = Some(SilentStream::on_endpoint(Some(&endpoint_id)).0);
                println!("SUBTITLE_TEST_SECOND_ENDPOINT");
                std::io::stdout().flush().unwrap();
            }
            b'x' => {
                assert!(primary_running && second_stream.is_some());
                stream.stop();
                primary_running = false;
                println!("SUBTITLE_TEST_PRIMARY_STOPPED");
                std::io::stdout().flush().unwrap();
            }
            _ => panic!("invalid fixture command"),
        }
    }
    drop(child);
    drop(second_stream);
    reader.join().unwrap();
}

#[track_caller]
pub(super) fn wait_for(
    scanner: &mut WindowsSessionScanner,
    predicate: impl Fn(&ScanReport) -> bool,
) -> ScanReport {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let report = scanner.scan(&AtomicBool::new(false)).unwrap();
        if predicate(&report) {
            return report;
        }
        assert!(
            Instant::now() < deadline,
            "local audio lifecycle did not reach the expected state; own sessions: {:?}; skipped={}; unavailable={}; notification_loss={}",
            report
                .sources
                .iter()
                .filter(|source| source.process_id == std::process::id())
                .map(|source| (source.state, source.peak))
                .collect::<Vec<_>>(),
            report.skipped_sessions,
            report.unavailable_devices,
            report.notifications_lost
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
#[ignore = "requires a local Windows playback device; generates only silent in-memory streams"]
fn native_session_appears_updates_and_disappears() {
    if std::env::var_os("SUBTITLENOW_CAPTURE_HELPER").as_deref() == Some(std::ffi::OsStr::new("1"))
    {
        let _ = super::capture_helper::run();
        return;
    }
    if std::env::var_os("SUBTITLENOW_NATIVE_FIXTURE").as_deref() == Some(std::ffi::OsStr::new("1"))
    {
        fixture_main();
        return;
    }
    let mut scanner = WindowsSessionScanner::new().unwrap();
    let first = FixtureProcess::start();
    let first_id = first.id.clone();
    let initial = wait_for(&mut scanner, |report| {
        report.sources.iter().any(|source| source.id == first_id)
    });
    assert!(!initial.notifications_lost);
    let first_source = initial
        .sources
        .iter()
        .find(|source| source.id == first_id)
        .unwrap();
    assert_eq!(first_source.process_id, first.child.id());
    assert_eq!(first_source.state, SessionState::Active);
    assert_eq!(first_source.peak, Some(0.0));
    assert!(first_source.application_name.is_some());
    // Created after initial enumeration, exercising live registration and the bounded mailbox.
    let mut second = FixtureProcess::start();
    let second_id = second.id.clone();
    assert_ne!(first_id, second_id);
    let report = wait_for(&mut scanner, |report| {
        report.sources.iter().any(|source| source.id == second_id)
    });
    assert!(report.sources.iter().any(|source| source.id == first_id));
    assert!(!report.notifications_lost);
    second.stop_stream();
    wait_for(&mut scanner, |report| {
        report
            .sources
            .iter()
            .any(|source| source.id == second_id && source.state == SessionState::Inactive)
    });
    drop(second);
    wait_for(&mut scanner, |report| {
        report.sources.iter().all(|source| source.id != second_id)
    });
    drop(first);
    wait_for(&mut scanner, |report| {
        report.sources.iter().all(|source| source.id != first_id)
    });
}

#[test]
fn native_icon_extracts_owned_bounded_pixels() {
    let path =
        std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("explorer.exe");
    let icon = super::icon::extract(path.to_str().unwrap())
        .expect("Windows Explorer icon must be readable");
    assert_eq!((icon.width, icon.height), (32, 32));
    assert_eq!(icon.bgra.len(), 32 * 32 * 4);
    assert!(
        icon.bgra
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[3] > 0)
    );
}

#[test]
#[ignore = "requires a local Windows playback device; RAM-only silent PCM"]
fn native_process_capture_stops_and_detects_source_exit() {
    use crate::audio::capture::{CaptureError, CaptureSession, CaptureState};
    let _serial = super::capture::ACTIVATION_TEST_LOCK.lock().unwrap();
    let mut scanner = WindowsSessionScanner::new().unwrap();
    let fixture = FixtureProcess::start();
    let mut previous_id = None;
    for _ in 0..3 {
        let mut target = super::capture::ProcessTarget::open(fixture.child.id()).unwrap();
        let report = wait_for(&mut scanner, |report| {
            report.sources.iter().any(|source| source.id == fixture.id)
        });
        target.confirm(&report).unwrap();
        let mut session =
            CaptureSession::start_normalized(move |sink| super::capture::capture(target, sink))
                .unwrap();
        assert_ne!(Some(session.id()), previous_id);
        previous_id = Some(session.id());
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut blocks = 0;
        let mut last_sequence = 0;
        while blocks < 5 {
            assert!(
                Instant::now() < deadline,
                "no native audio; state={:?}",
                session.state()
            );
            assert!(
                !matches!(session.state(), CaptureState::Stopped(_)),
                "{:?}",
                session.state()
            );
            while let Some(block) = session.try_next() {
                assert_eq!(block.stamp.session, session.id());
                assert!(block.stamp.sequence > last_sequence);
                last_sequence = block.stamp.sequence;
                let peak = block.samples[..block.frames]
                    .iter()
                    .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
                assert!(
                    peak <= SILENCE_PCM16_TOLERANCE,
                    "silent fixture PCM peak={peak}"
                );
                blocks += 1;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(session.metrics().captured_frames > 0);
        let before = Instant::now();
        session.stop().unwrap();
        assert!(before.elapsed() < Duration::from_secs(1));
        assert_eq!(session.metrics().raw_queue_depth, 0);
        assert_eq!(session.metrics().normalized_queue_depth, 0);
        assert!(session.try_next().is_none());
    }
    let mut target = super::capture::ProcessTarget::open(fixture.child.id()).unwrap();
    target
        .confirm(&scanner.scan(&AtomicBool::new(false)).unwrap())
        .unwrap();
    let mut session =
        CaptureSession::start_normalized(move |sink| super::capture::capture(target, sink))
            .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while session.state() == CaptureState::Starting {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(session.state(), CaptureState::Running);
    drop(fixture);
    while session.state() == CaptureState::Running {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        session.state(),
        CaptureState::Stopped(Err(CaptureError::SourceExited))
    );
    assert_eq!(session.stop(), Err(CaptureError::SourceExited));
    assert!(session.try_next().is_none());
}

// 100 ms gives exact frequency bins for these integer test tones at 48 kHz. Windows
// render buffering and source transitions get 250 ms to settle before a window is measured.
const SIGNAL_WINDOW_SAMPLES: usize = 4_800;
const SIGNAL_SETTLE: Duration = Duration::from_millis(250);
const MIN_TARGET_AMPLITUDE: f64 = 0.005; // requires a real, comfortably detectable tone
const MAX_UNRELATED_AMPLITUDE: f64 = 0.00025; // under 1% of the generated tone

pub(super) fn start_fixture_capture(
    fixture: &FixtureProcess,
) -> crate::audio::capture::CaptureSession {
    let target = super::capture::ProcessTarget::open(fixture.child.id()).unwrap();
    crate::audio::capture::CaptureSession::start_normalized(move |sink| {
        super::capture::capture(target, sink)
    })
    .unwrap()
}

pub(super) fn signal_window(session: &mut crate::audio::capture::CaptureSession) -> Vec<f32> {
    use crate::audio::capture::CaptureState;
    let settle = Instant::now() + SIGNAL_SETTLE;
    while Instant::now() < settle {
        while session.try_next().is_some() {}
        assert!(
            !matches!(session.state(), CaptureState::Stopped(_)),
            "{:?}",
            session.state()
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    let deadline = Instant::now() + FIXTURE_TIMEOUT;
    let mut samples = Vec::with_capacity(SIGNAL_WINDOW_SAMPLES);
    while samples.len() < SIGNAL_WINDOW_SAMPLES && Instant::now() < deadline {
        while let Some(block) = session.try_next() {
            assert_eq!(block.stamp.session, session.id());
            let count = block.frames.min(SIGNAL_WINDOW_SAMPLES - samples.len());
            samples.extend_from_slice(&block.samples[..count]);
            if samples.len() == SIGNAL_WINDOW_SAMPLES {
                break;
            }
        }
        assert!(
            !matches!(session.state(), CaptureState::Stopped(_)),
            "{:?}",
            session.state()
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    samples
}

pub(super) fn tone_amplitude(samples: &[f32], frequency: f64) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let coefficient = 2.0 * (std::f64::consts::TAU * frequency / 48_000.0).cos();
    let (mut previous, mut before_previous) = (0.0, 0.0);
    for sample in samples {
        let current = f64::from(*sample) + coefficient * previous - before_previous;
        before_previous = previous;
        previous = current;
    }
    let power = previous * previous + before_previous * before_previous
        - coefficient * previous * before_previous;
    2.0 * power.max(0.0).sqrt() / samples.len() as f64
}

#[test]
#[ignore = "requires a local Windows playback device; plays low-level RAM-only synthetic tones"]
fn native_process_capture_isolates_tones_and_includes_audio_children() {
    let _serial = super::capture::ACTIVATION_TEST_LOCK.lock().unwrap();
    println!(
        "Playing low-level synthetic test tones; system volume is unchanged, no audio files are created."
    );
    let mut target = FixtureProcess::start_with_frequency(TARGET_FREQUENCY);
    let mut unrelated = FixtureProcess::start_with_frequency(UNRELATED_FREQUENCY);
    assert_ne!(target.child.id(), unrelated.child.id());
    let mut session = start_fixture_capture(&target);
    target.play(true);
    let samples = signal_window(&mut session);
    let target_amplitude = tone_amplitude(&samples, TARGET_FREQUENCY);
    let unrelated_amplitude = tone_amplitude(&samples, UNRELATED_FREQUENCY);
    println!("A alone: target={target_amplitude:.6}, unrelated={unrelated_amplitude:.6}");
    assert_eq!(samples.len(), SIGNAL_WINDOW_SAMPLES);
    assert!(target_amplitude > MIN_TARGET_AMPLITUDE);
    assert!(unrelated_amplitude < MAX_UNRELATED_AMPLITUDE);

    target.play(false);
    unrelated.play(true);
    let samples = signal_window(&mut session);
    let peak = samples
        .iter()
        .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
    println!("B alone while capturing A: quiet peak={peak:.6}");
    assert!(peak <= SILENCE_PCM16_TOLERANCE);
    assert!(tone_amplitude(&samples, UNRELATED_FREQUENCY) < MAX_UNRELATED_AMPLITUDE);

    target.play(true);
    let samples = signal_window(&mut session);
    let target_amplitude = tone_amplitude(&samples, TARGET_FREQUENCY);
    let unrelated_amplitude = tone_amplitude(&samples, UNRELATED_FREQUENCY);
    println!("A+B capturing A: target={target_amplitude:.6}, unrelated={unrelated_amplitude:.6}");
    assert!(target_amplitude > MIN_TARGET_AMPLITUDE);
    assert!(unrelated_amplitude < MAX_UNRELATED_AMPLITUDE);
    let old_session = session.id();
    session.stop().unwrap();
    assert!(session.try_next().is_none());

    let mut session = start_fixture_capture(&unrelated);
    assert_ne!(session.id(), old_session);
    let samples = signal_window(&mut session);
    let target_amplitude = tone_amplitude(&samples, TARGET_FREQUENCY);
    let unrelated_amplitude = tone_amplitude(&samples, UNRELATED_FREQUENCY);
    println!("A+B capturing B: target={unrelated_amplitude:.6}, unrelated={target_amplitude:.6}");
    assert!(unrelated_amplitude > MIN_TARGET_AMPLITUDE); // proves B genuinely produces audio
    assert!(target_amplitude < MAX_UNRELATED_AMPLITUDE);
    session.stop().unwrap();

    target.play(false);
    let mut session = start_fixture_capture(&target);
    let child_pid = target.spawn_audio_child();
    let child_guard = TestChildGuard::open(child_pid);
    assert_ne!(child_pid, target.child.id());
    let samples = signal_window(&mut session);
    let child_amplitude = tone_amplitude(&samples, CHILD_FREQUENCY);
    println!("New child C while capturing A: child={child_amplitude:.6}");
    assert!(child_amplitude > MIN_TARGET_AMPLITUDE);
    assert!(tone_amplitude(&samples, UNRELATED_FREQUENCY) < MAX_UNRELATED_AMPLITUDE);
    session.stop().unwrap();

    // The same child now predates capture: both existing and newly created descendants count.
    let mut session = start_fixture_capture(&target);
    let samples = signal_window(&mut session);
    assert!(tone_amplitude(&samples, CHILD_FREQUENCY) > MIN_TARGET_AMPLITUDE);
    assert!(tone_amplitude(&samples, UNRELATED_FREQUENCY) < MAX_UNRELATED_AMPLITUDE);
    target.child.kill().unwrap();
    target.child.wait().unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    while session.state() == crate::audio::capture::CaptureState::Running {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(
        child_guard.alive(),
        "descendant must still be alive when root exit stops capture"
    );
    assert_eq!(
        session.stop(),
        Err(crate::audio::capture::CaptureError::SourceExited)
    );
    assert!(session.try_next().is_none());
    drop(child_guard);
}

/// Owns only the descendant PID returned by our fixture, including panic-path cleanup.
struct TestChildGuard(std::os::windows::io::OwnedHandle);
impl TestChildGuard {
    fn open(pid: u32) -> Self {
        use std::os::windows::io::FromRawHandle;
        use windows::Win32::System::Threading::{
            OpenProcess, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
        };
        // SAFETY: pid comes from our root fixture's newly spawned child, not a user source.
        let handle =
            unsafe { OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_TERMINATE, false, pid) }.unwrap();
        // SAFETY: successful owning handle transferred once to OwnedHandle.
        Self(unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(handle.0) })
    }
    fn alive(&self) -> bool {
        use std::os::windows::io::AsRawHandle;
        // SAFETY: live owned process handle, zero-timeout liveness check only.
        unsafe {
            windows::Win32::System::Threading::WaitForSingleObject(
                windows::Win32::Foundation::HANDLE(self.0.as_raw_handle()),
                0,
            ) == windows::Win32::Foundation::WAIT_TIMEOUT
        }
    }
}
impl Drop for TestChildGuard {
    fn drop(&mut self) {
        use std::os::windows::io::AsRawHandle;
        let handle = windows::Win32::Foundation::HANDLE(self.0.as_raw_handle());
        // SAFETY: terminates only our test-owned descendant, then waits for its cleanup.
        unsafe {
            if self.alive() {
                let _ = windows::Win32::System::Threading::TerminateProcess(handle, 0);
            }
            windows::Win32::System::Threading::WaitForSingleObject(
                handle,
                FIXTURE_TIMEOUT.as_millis() as u32,
            );
        }
    }
}

#[test]
#[ignore = "requires two active playback endpoints; low-level RAM-only synthetic tones"]
fn native_process_capture_spans_two_render_endpoints() {
    let _serial = super::capture::ACTIVATION_TEST_LOCK.lock().unwrap();
    let mut target = FixtureProcess::start();
    target.play(true);
    let mut unrelated = FixtureProcess::start_with_frequency(CHILD_FREQUENCY);
    unrelated.play(true);
    let mut session = start_fixture_capture(&target);
    let first_samples = signal_window(&mut session);
    assert!(tone_amplitude(&first_samples, TARGET_FREQUENCY) > MIN_TARGET_AMPLITUDE);
    target.add_second_endpoint(); // dynamically appears after capture has already started
    let samples = signal_window(&mut session);
    let first = tone_amplitude(&samples, TARGET_FREQUENCY);
    let second = tone_amplitude(&samples, UNRELATED_FREQUENCY);
    let excluded = tone_amplitude(&samples, CHILD_FREQUENCY);
    println!(
        "TWO endpoints, one selected PID: first={first:.6}, second={second:.6}, unrelated={excluded:.6}"
    );
    assert!(first > MIN_TARGET_AMPLITUDE && second > MIN_TARGET_AMPLITUDE);
    assert!(excluded < MAX_UNRELATED_AMPLITUDE);
    target.stop_primary_endpoint();
    let moved = signal_window(&mut session);
    let remaining = tone_amplitude(&moved, UNRELATED_FREQUENCY);
    println!("Source moves to second endpoint: remaining={remaining:.6}");
    assert!(remaining > MIN_TARGET_AMPLITUDE);
    assert!(tone_amplitude(&moved, TARGET_FREQUENCY) < MAX_UNRELATED_AMPLITUDE);
    assert!(tone_amplitude(&moved, CHILD_FREQUENCY) < MAX_UNRELATED_AMPLITUDE);
    target.play(false);
    let samples = signal_window(&mut session);
    assert!(
        samples
            .iter()
            .all(|sample| sample.abs() <= SILENCE_PCM16_TOLERANCE)
    );
    session.stop().unwrap();
    assert!(session.try_next().is_none());
}

#[test]
#[ignore = "requires a local Windows playback device; 100 RAM-only silent capture restarts"]
fn native_process_capture_restarts_keep_handles_bounded() {
    const RESTARTS: usize = 100;
    const WARMUP_RESTARTS: usize = 3; // allow the Windows activation pool to initialize
    use crate::audio::capture::CaptureState;
    let _serial = super::capture::ACTIVATION_TEST_LOCK.lock().unwrap();
    let fixture = FixtureProcess::start();
    let handle_count = || {
        let mut count = 0;
        // SAFETY: current-process pseudo handle and valid writable count, no audio content.
        unsafe {
            windows::Win32::System::Threading::GetProcessHandleCount(
                windows::Win32::System::Threading::GetCurrentProcess(),
                &mut count,
            )
        }
        .unwrap();
        count
    };
    let mut baseline = 0;
    let mut stop_times = Vec::with_capacity(RESTARTS);
    let mut first_pcm_times = Vec::with_capacity(RESTARTS);
    for iteration in 0..RESTARTS + WARMUP_RESTARTS {
        let started = Instant::now();
        let mut session = start_fixture_capture(&fixture);
        let deadline = Instant::now() + FIXTURE_TIMEOUT;
        while session.try_next().is_none() {
            assert!(
                Instant::now() < deadline && !matches!(session.state(), CaptureState::Stopped(_)),
                "{:?}",
                session.state()
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        let first_pcm = started.elapsed();
        let stopping = Instant::now();
        session.stop().unwrap();
        let stop = stopping.elapsed();
        assert_eq!(session.metrics().raw_queue_depth, 0);
        assert_eq!(session.metrics().normalized_queue_depth, 0);
        drop(session);
        if iteration + 1 == WARMUP_RESTARTS {
            baseline = handle_count();
        }
        if iteration >= WARMUP_RESTARTS {
            stop_times.push(stop);
            first_pcm_times.push(first_pcm);
        }
    }
    let after_restarts = handle_count();
    let settle_deadline = Instant::now() + FIXTURE_TIMEOUT;
    let mut final_handles = handle_count();
    while final_handles > baseline + 2 && Instant::now() < settle_deadline {
        std::thread::sleep(FIXTURE_POLL);
        final_handles = handle_count();
    }
    stop_times.sort();
    first_pcm_times.sort();
    println!(
        "100 restarts: handles baseline={baseline}, after restarts={after_restarts}, final={final_handles}; stop P50={:?}, P95={:?}, max={:?}; first PCM P50={:?}, P95={:?}, max={:?}",
        stop_times[49],
        stop_times[94],
        stop_times[99],
        first_pcm_times[49],
        first_pcm_times[94],
        first_pcm_times[99]
    );
    // Same scanner/fixture remain alive at both samples; allow two transient system handles.
    assert!(
        final_handles <= baseline + 2,
        "process handles grew across capture restarts"
    );
}

#[test]
#[ignore = "isolated process handle accounting for mock workers; no audio capture"]
fn stopped_mock_capture_workers_keep_handles_bounded() {
    let count = || {
        let mut count = 0;
        // SAFETY: current-process pseudo handle and initialized output, no audio interfaces.
        unsafe {
            windows::Win32::System::Threading::GetProcessHandleCount(
                windows::Win32::System::Threading::GetCurrentProcess(),
                &mut count,
            )
        }
        .unwrap();
        count
    };
    let baseline = count();
    for _ in 0..100 {
        let mut session = crate::audio::capture::CaptureSession::start(|sink| {
            while !sink.cancelled() {
                std::thread::park_timeout(Duration::from_millis(1));
            }
            Ok(())
        })
        .unwrap();
        session.stop().unwrap();
    }
    let final_handles = count();
    println!("100 mock worker restarts: handles baseline={baseline}, final={final_handles}");
    assert!(final_handles <= baseline + 2);
}

#[test]
#[ignore = "isolated native discovery handle accounting; local playback device and RAM-only silence"]
fn native_discovery_restarts_keep_handles_bounded() {
    let fixture = FixtureProcess::start();
    let count = || {
        let mut count = 0;
        // SAFETY: current-process pseudo handle and initialized output only.
        unsafe {
            windows::Win32::System::Threading::GetProcessHandleCount(
                windows::Win32::System::Threading::GetCurrentProcess(),
                &mut count,
            )
        }
        .unwrap();
        count
    };
    let mut baseline = 0;
    for iteration in 0..103 {
        let mut scanner = WindowsSessionScanner::new().unwrap();
        wait_for(&mut scanner, |report| {
            report.sources.iter().any(|source| source.id == fixture.id)
        });
        drop(scanner);
        if iteration == 2 {
            baseline = count();
        }
    }
    let final_handles = count();
    println!("100 discovery restarts: handles baseline={baseline}, final={final_handles}");
    assert!(final_handles <= baseline + 2);
}
