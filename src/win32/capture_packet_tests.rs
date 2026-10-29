//! Explicit mock COM packet tests. They exercise the real adapter, not Windows capture.
use super::*;
use crate::audio::{
    capture::{CaptureSession, CaptureState, MAX_AUDIO_AGE},
    preprocess::NormalizedAudio,
};
use std::{
    sync::{Arc, atomic::AtomicUsize},
    thread::{self, ThreadId},
};
use windows::Win32::Foundation::{E_POINTER, E_UNEXPECTED};

#[derive(Clone, Copy)]
enum PointerKind {
    Samples,
    Null,
    Unaligned,
}

#[derive(Default)]
struct Calls {
    acquired: AtomicUsize,
    released: AtomicUsize,
    released_frames: AtomicUsize,
    outstanding: AtomicBool,
    destroyed: AtomicBool,
}

struct PacketFixture {
    samples: Box<[i16]>,
    frames: u32,
    flags: u32,
    pointer: PointerKind,
    age_100ns: u64,
    acquire_error: Option<HRESULT>,
    release_error: Option<HRESULT>,
}

impl Default for PacketFixture {
    fn default() -> Self {
        Self {
            samples: vec![i16::MIN, i16::MIN, i16::MAX, i16::MAX, i16::MIN, i16::MAX]
                .into_boxed_slice(),
            frames: 3,
            flags: 0,
            pointer: PointerKind::Samples,
            age_100ns: 0,
            acquire_error: None,
            release_error: None,
        }
    }
}

#[implement(IAudioCaptureClient)]
struct MockCapture {
    fixture: PacketFixture,
    calls: Arc<Calls>,
    owner: ThreadId,
}

impl Drop for MockCapture {
    fn drop(&mut self) {
        self.calls.destroyed.store(true, Ordering::Release);
    }
}

fn qpc_now() -> windows::core::Result<u64> {
    let (mut ticks, mut frequency) = (0, 0);
    // SAFETY: two valid writable outputs; these functions do not open an audio device.
    unsafe {
        QueryPerformanceFrequency(&mut frequency)?;
        QueryPerformanceCounter(&mut ticks)?;
    }
    if frequency <= 0 || ticks < 0 {
        return Err(E_UNEXPECTED.into());
    }
    u64::try_from(ticks as u128 * 10_000_000 / frequency as u128).map_err(|_| E_UNEXPECTED.into())
}

impl IAudioCaptureClient_Impl for MockCapture_Impl {
    fn GetBuffer(
        &self,
        data: *mut *mut u8,
        frames: *mut u32,
        flags: *mut u32,
        position: *mut u64,
        qpc: *mut u64,
    ) -> windows::core::Result<()> {
        self.calls.acquired.fetch_add(1, Ordering::Relaxed);
        if thread::current().id() != self.owner {
            return Err(E_UNEXPECTED.into());
        }
        if let Some(error) = self.fixture.acquire_error {
            return Err(error.into());
        }
        if data.is_null() || frames.is_null() || flags.is_null() {
            return Err(E_POINTER.into());
        }
        if self.calls.outstanding.load(Ordering::Acquire) {
            return Err(AUDCLNT_E_OUT_OF_ORDER.into());
        }
        let timestamp = qpc_now()?.saturating_sub(self.fixture.age_100ns);
        let pointer = match self.fixture.pointer {
            PointerKind::Samples => self.fixture.samples.as_ptr().cast_mut().cast::<u8>(),
            PointerKind::Null => std::ptr::null_mut(),
            // Deliberately invalid alignment, rejected before any slice is constructed.
            PointerKind::Unaligned => self
                .fixture
                .samples
                .as_ptr()
                .cast_mut()
                .cast::<u8>()
                .wrapping_add(1),
        };
        // SAFETY: only read_packet calls this test double, using writable stack outputs.
        // The sample allocation remains stable until ReleaseBuffer/the COM owner drops.
        // Oversized/invalid-pointer fixtures are rejected by the adapter before dereferencing.
        unsafe {
            *data = pointer;
            *frames = self.fixture.frames;
            *flags = self.fixture.flags;
            if !position.is_null() {
                *position = 48_000;
            }
            if !qpc.is_null() {
                *qpc = timestamp;
            }
        }
        self.calls
            .outstanding
            .store(self.fixture.frames != 0, Ordering::Release);
        Ok(())
    }

    fn ReleaseBuffer(&self, frames: u32) -> windows::core::Result<()> {
        self.calls.released.fetch_add(1, Ordering::Relaxed);
        self.calls
            .released_frames
            .store(frames as usize, Ordering::Relaxed);
        if thread::current().id() != self.owner
            || !self.calls.outstanding.swap(false, Ordering::AcqRel)
        {
            return Err(AUDCLNT_E_OUT_OF_ORDER.into());
        }
        if frames != self.fixture.frames {
            return Err(E_UNEXPECTED.into());
        }
        self.fixture
            .release_error
            .map_or(Ok(()), |error| Err(error.into()))
    }

    fn GetNextPacketSize(&self) -> windows::core::Result<u32> {
        Ok(self.fixture.frames)
    }
}

fn run(
    fixture: PacketFixture,
    buffer_frames: u32,
) -> (CaptureSession, Arc<Calls>, Result<(), CaptureError>) {
    let calls = Arc::new(Calls::default());
    let worker_calls = Arc::clone(&calls);
    let (sender, receiver) = mpsc::sync_channel(1);
    let session = CaptureSession::start(move |mut sink| {
        let _apartment = Apartment::new().unwrap();
        let client: IAudioCaptureClient = MockCapture {
            fixture,
            calls: worker_calls,
            owner: thread::current().id(),
        }
        .into();
        let clock = CaptureClock::new()?;
        sink.started();
        let result = read_packet(
            &client,
            &mut sink,
            buffer_frames,
            &clock,
            &mut super::super::loopback_clock::LoopbackClock::default(),
        );
        drop(client);
        let _ = sender.try_send(result.clone());
        result?;
        while !sink.cancelled() {
            thread::park_timeout(Duration::from_millis(2));
        }
        Ok(())
    })
    .unwrap();
    let result = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
    (session, calls, result)
}

fn next(session: &mut CaptureSession) -> NormalizedAudio {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(block) = session.try_next() {
            return block;
        }
        assert!(
            Instant::now() < deadline,
            "packet not delivered: {:?}",
            session.state()
        );
        thread::sleep(Duration::from_millis(1));
    }
}

fn assert_release(calls: &Calls, count: usize, frames: usize) {
    assert_eq!(calls.acquired.load(Ordering::Acquire), 1);
    assert_eq!(calls.released.load(Ordering::Acquire), count);
    assert_eq!(calls.released_frames.load(Ordering::Acquire), frames);
    assert!(!calls.outstanding.load(Ordering::Acquire));
    assert!(calls.destroyed.load(Ordering::Acquire));
}

#[test]
fn native_adapter_copies_non_silent_pcm_before_releasing_com_buffer() {
    let (mut session, calls, result) = run(PacketFixture::default(), 960);
    assert_eq!(result, Ok(()));
    assert_release(&calls, 1, 3);
    // The native COM allocation is already destroyed, but the pipeline owns its PCM copy.
    let block = next(&mut session);
    assert_eq!(block.frames, 3);
    assert_eq!(
        block.samples[..3],
        [-1.0, 32767.0 / 32768.0, -1.0 / 65536.0]
    );
    assert_eq!(block.stamp.frame_position, Some(48_000));
    assert!(block.stamp.qpc_100ns.is_some());
    assert_eq!(block.start_us(), Some(1_000_000));
    assert!(block.stamp.received.elapsed() < MAX_AUDIO_AGE);
    session.stop().unwrap();
}

#[test]
fn silent_null_buffer_with_timestamp_error_is_normalized_without_dereferencing() {
    let fixture = PacketFixture {
        pointer: PointerKind::Null,
        flags: (AUDCLNT_BUFFERFLAGS_SILENT.0 | AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0) as u32,
        ..Default::default()
    };
    let (mut session, calls, result) = run(fixture, 960);
    assert_eq!(result, Ok(()));
    let block = next(&mut session);
    assert!(
        block.samples[..block.frames]
            .iter()
            .all(|sample| *sample == 0.0)
    );
    assert_eq!(block.stamp.frame_position, None);
    assert_eq!(block.stamp.qpc_100ns, None);
    assert!(block.stamp.discontinuity);
    assert_release(&calls, 1, 3);
    session.stop().unwrap();
}

#[test]
fn malformed_packet_releases_exactly_once_and_stops_without_delivering_audio() {
    for (fixture, expected) in [
        (
            PacketFixture {
                frames: 961,
                pointer: PointerKind::Null,
                ..Default::default()
            },
            "packet_size",
        ),
        (
            PacketFixture {
                pointer: PointerKind::Null,
                ..Default::default()
            },
            "packet_pointer",
        ),
        (
            PacketFixture {
                pointer: PointerKind::Unaligned,
                ..Default::default()
            },
            "packet_pointer",
        ),
    ] {
        let frames = fixture.frames as usize;
        let (mut session, calls, result) = run(fixture, 960);
        assert_eq!(result, Err(CaptureError::InvalidData(expected)));
        assert_eq!(session.stop(), result);
        assert_release(&calls, 1, frames);
        assert!(session.try_next().is_none());
        assert_eq!(session.metrics().captured_frames, 0);
    }
}

#[test]
fn empty_packet_needs_no_release_and_creates_no_audio() {
    let (mut session, calls, result) = run(
        PacketFixture {
            frames: 0,
            pointer: PointerKind::Null,
            ..Default::default()
        },
        960,
    );
    assert_eq!(result, Ok(()));
    assert_release(&calls, 0, 0);
    assert!(session.try_next().is_none());
    assert_eq!(session.metrics().captured_frames, 0);
    session.stop().unwrap();
}

#[test]
fn acquire_and_release_disconnects_stop_without_retaining_a_packet() {
    for on_release in [false, true] {
        let fixture = PacketFixture {
            acquire_error: (!on_release).then_some(AUDCLNT_E_DEVICE_INVALIDATED),
            release_error: on_release.then_some(AUDCLNT_E_RESOURCES_INVALIDATED),
            ..Default::default()
        };
        let (mut session, calls, result) = run(fixture, 960);
        assert_eq!(result, Err(CaptureError::Disconnected));
        assert_eq!(session.stop(), result);
        assert_eq!(
            session.state(),
            CaptureState::Stopped(Err(CaptureError::Disconnected))
        );
        assert_release(
            &calls,
            usize::from(on_release),
            if on_release { 3 } else { 0 },
        );
        assert!(session.try_next().is_none());
        assert_eq!(session.metrics().normalized_queue_depth, 0);
    }
}

#[test]
fn old_qpc_packet_is_released_and_counted_without_replaying_stale_audio() {
    // One second is safely beyond the 250 ms policy, independent of scheduler jitter.
    let (mut session, calls, result) = run(
        PacketFixture {
            age_100ns: 10_000_000,
            ..Default::default()
        },
        960,
    );
    assert_eq!(result, Ok(()));
    assert_release(&calls, 1, 3);
    assert_eq!(session.metrics().stale_frames, 3);
    assert_eq!(session.metrics().raw_queue_depth, 0);
    assert!(session.try_next().is_none());
    session.stop().unwrap();
}
