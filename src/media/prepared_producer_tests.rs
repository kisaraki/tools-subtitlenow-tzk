//! Isolated, explicit synthetic native owners. No file, device, provider or key.
use super::*;
use crate::{
    audio::capture::SessionId,
    media::decoded::{DecodedAudioBlock, DecodedAudioFormat},
};
use std::{
    collections::VecDeque,
    rc::Rc,
    sync::{
        Condvar, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread::{self, ThreadId},
    time::Duration,
};

const TEST_TIMEOUT: Duration = Duration::from_secs(3);
const GATE_TIMEOUT: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(1);
type Events = Arc<Mutex<Vec<(&'static str, ThreadId)>>>;

fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(717),
        generation: 2,
    }
}
fn lane() -> Arc<Semaphore> {
    Arc::new(Semaphore::new(1))
}
fn format(rate: u32) -> DecodedAudioFormat {
    DecodedAudioFormat::new(rate, 1).unwrap()
}
fn block(rate: u32, sequence: u64, timestamp: u64, frames: usize, value: i16) -> DecodedAudioBlock {
    DecodedAudioBlock::new(
        identity(),
        sequence,
        timestamp,
        format(rate),
        vec![value; frames],
    )
    .unwrap()
}
fn run(rate: u32, anchor: u64, frames: usize) -> Vec<DecodedAudioBlock> {
    let mut blocks = Vec::new();
    let mut offset = 0;
    while offset < frames {
        let count = (rate as usize).min(frames - offset);
        blocks.push(block(
            rate,
            blocks.len() as u64 + 1,
            anchor + offset as u64 * 1_000_000 / u64::from(rate),
            count,
            901,
        ));
        offset += count;
    }
    blocks
}
fn event(events: &Events, name: &'static str) {
    events.lock().unwrap().push((name, thread::current().id()));
}
async fn wait_for(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(TEST_TIMEOUT, async {
        while !condition() {
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    })
    .await
    .expect("synthetic worker did not reach a bounded checkpoint");
}
async fn released(cpu: &Semaphore, owners: &Semaphore) {
    wait_for(|| cpu.available_permits() == 1 && owners.available_permits() == 1).await;
}
async fn bounded_pull(
    source: &mut PreparedLocalSource,
) -> Result<PreparedLocalBatch, PreparedAsrError> {
    tokio::time::timeout(TEST_TIMEOUT, source.pull())
        .await
        .expect("synthetic pull exceeded its bounded test timeout")
}

#[derive(Default)]
struct Gate {
    entered: AtomicBool,
    released: Mutex<bool>,
    wake: Condvar,
}
impl Gate {
    fn wait(&self) -> Result<(), MediaError> {
        self.entered.store(true, Ordering::SeqCst);
        let released = self.released.lock().unwrap();
        let (released, _) = self
            .wake
            .wait_timeout_while(released, GATE_TIMEOUT, |released| !*released)
            .unwrap();
        if *released {
            Ok(())
        } else {
            Err(MediaError::WorkerFailed)
        }
    }
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.wake.notify_all();
    }
}
/// Any assertion/timeout still releases the finite blocking fixture.
struct ReleaseOnDrop(Arc<Gate>);
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.release();
    }
}

struct SyntheticDecoder {
    rate: u32,
    blocks: VecDeque<DecodedAudioBlock>,
    eof_error: Option<MediaError>,
    panic_at_eof: bool,
    next_gate: Option<Arc<Gate>>,
    pulls: Arc<AtomicUsize>,
    events: Events,
    owner: ThreadId,
    _affine: Rc<()>,
}
impl MediaDecoder for SyntheticDecoder {
    fn format(&self) -> DecodedAudioFormat {
        format(self.rate)
    }
    fn next(
        &mut self,
        cancel: &ResolverCancellation,
    ) -> Result<Option<DecodedAudioBlock>, MediaError> {
        assert_eq!(thread::current().id(), self.owner);
        event(&self.events, "next");
        self.pulls.fetch_add(1, Ordering::SeqCst);
        if let Some(gate) = self.next_gate.take() {
            gate.wait()?;
        }
        if cancel.is_cancelled() {
            return Err(MediaError::Cancelled);
        }
        if let Some(block) = self.blocks.pop_front() {
            return Ok(Some(block));
        }
        assert!(!self.panic_at_eof, "explicit synthetic decoder panic");
        self.eof_error.map_or(Ok(None), Err)
    }
}
impl Drop for SyntheticDecoder {
    fn drop(&mut self) {
        event(&self.events, "drop");
    }
}
struct PermitProbeDecoder {
    inner: SyntheticDecoder,
    cpu: Arc<Semaphore>,
    owners: Arc<Semaphore>,
    drop_counts: Arc<Mutex<Option<(usize, usize)>>>,
}
impl MediaDecoder for PermitProbeDecoder {
    fn format(&self) -> DecodedAudioFormat {
        self.inner.format()
    }
    fn next(
        &mut self,
        cancel: &ResolverCancellation,
    ) -> Result<Option<DecodedAudioBlock>, MediaError> {
        self.inner.next(cancel)
    }
}
impl Drop for PermitProbeDecoder {
    fn drop(&mut self) {
        // Record rather than assert in Drop: a broken panic-cleanup order must
        // fail the test safely instead of panicking again during unwinding.
        *self.drop_counts.lock().unwrap() = Some((
            self.cpu.available_permits(),
            self.owners.available_permits(),
        ));
    }
}
fn factory(
    rate: u32,
    blocks: Vec<DecodedAudioBlock>,
    pulls: Arc<AtomicUsize>,
    events: Events,
) -> Factory {
    Box::new(move |_| {
        event(&events, "create");
        Ok(Box::new(SyntheticDecoder {
            rate,
            blocks: blocks.into(),
            eof_error: None,
            panic_at_eof: false,
            next_gate: None,
            pulls,
            events,
            owner: thread::current().id(),
            _affine: Rc::new(()),
        }))
    })
}
fn source(
    factory: Factory,
    rate: PreparedSampleRate,
    cancel: ResolverCancellation,
    cpu: Arc<Semaphore>,
    owners: Arc<Semaphore>,
) -> PreparedLocalSource {
    PreparedLocalSource::with_factory(identity(), rate, cancel, factory, cpu, owners).unwrap()
}
fn assert_affinity(events: &Events, caller: ThreadId) {
    let events = events.lock().unwrap();
    let owner = events.iter().find(|(name, _)| *name == "create").unwrap().1;
    assert_ne!(owner, caller);
    assert!(events.iter().all(|(_, thread)| *thread == owner));
    assert_eq!(events.iter().filter(|(name, _)| *name == "drop").count(), 1);
}

#[tokio::test]
async fn unpolled_pull_cancels_without_factory_or_native_work() {
    let cpu = lane();
    let owners = lane();
    let cancel = ResolverCancellation::default();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let mut source = source(
        Box::new(move |_| {
            observed.fetch_add(1, Ordering::SeqCst);
            Err(MediaError::Unavailable)
        }),
        PreparedSampleRate::Hz16000,
        cancel.clone(),
        cpu.clone(),
        owners.clone(),
    );
    drop(source.pull());
    assert!(cancel.is_cancelled());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(source.worker.is_none() && source.factory.is_none());
    assert_eq!(cpu.available_permits(), 1);
    assert_eq!(owners.available_permits(), 1);
}

#[tokio::test]
async fn actual_create_next_and_drop_share_the_native_thread_and_eof_is_reaped() {
    for rate in [PreparedSampleRate::Hz16000, PreparedSampleRate::Hz24000] {
        let caller = thread::current().id();
        let cpu = lane();
        let owners = lane();
        let cancel = ResolverCancellation::default();
        let events = Arc::new(Mutex::new(Vec::new()));
        let pulls = Arc::new(AtomicUsize::new(0));
        let mut source = source(
            factory(
                rate.hz(),
                vec![block(rate.hz(), 1, 33, 47, 321)],
                pulls.clone(),
                events.clone(),
            ),
            rate,
            cancel.clone(),
            cpu.clone(),
            owners.clone(),
        );
        let batch = tokio::time::timeout(TEST_TIMEOUT, source.pull())
            .await
            .unwrap()
            .unwrap();
        assert!(batch.eof);
        assert_eq!(batch.chunks.len(), 1);
        let chunk = &batch.chunks[0];
        assert_eq!(chunk.stamp().identity, identity());
        assert_eq!(chunk.stamp().sequence, 1);
        assert_eq!(chunk.stamp().audio_start_us, 33);
        assert_eq!(chunk.audio_end_us(), chunk.stamp().body_end_us);
        assert_eq!(chunk.samples(), &[321; 47]);
        assert_eq!(pulls.load(Ordering::SeqCst), 2);
        assert!(source.worker.is_none() && source.commands.is_none());
        assert!(source.terminal);
        assert_eq!(cpu.available_permits(), 1);
        assert_eq!(owners.available_permits(), 1);
        assert!(!cancel.is_cancelled());
        assert_affinity(&events, caller);
        drop(source);
        assert!(!cancel.is_cancelled());
    }
}

#[tokio::test]
async fn ten_point_one_second_eof_has_two_bounded_closed_bodies() {
    let cpu = lane();
    let owners = lane();
    let cancel = ResolverCancellation::default();
    let events = Arc::new(Mutex::new(Vec::new()));
    let pulls = Arc::new(AtomicUsize::new(0));
    let mut source = source(
        factory(16_000, run(16_000, 17, 161_600), pulls, events),
        PreparedSampleRate::Hz16000,
        cancel,
        cpu.clone(),
        owners.clone(),
    );
    let batch = tokio::time::timeout(TEST_TIMEOUT, source.pull())
        .await
        .unwrap()
        .unwrap();
    assert!(batch.eof);
    assert_eq!(batch.chunks.len(), MAX_LOCAL_BATCH_CHUNKS);
    assert_eq!(batch.chunks[0].stamp().body_end_us, 10_000_017);
    assert_eq!(batch.chunks[1].stamp().body_start_us, 10_000_017);
    assert_eq!(batch.chunks[1].stamp().body_end_us, 10_100_017);
    assert_eq!(
        batch.chunks[1].audio_end_us(),
        batch.chunks[1].stamp().body_end_us
    );
    assert!(
        batch
            .chunks
            .iter()
            .all(|chunk| chunk.samples().len() <= 176_000)
    );
    assert_eq!(cpu.available_permits(), 1);
    assert_eq!(owners.available_permits(), 1);
}

#[tokio::test]
async fn gap_returns_two_old_bodies_and_new_run_never_crosses_old_context() {
    let cpu = lane();
    let owners = lane();
    let cancel = ResolverCancellation::default();
    let events = Arc::new(Mutex::new(Vec::new()));
    let pulls = Arc::new(AtomicUsize::new(0));
    let mut blocks = run(16_000, 3, 161_600);
    blocks.push(block(
        16_000,
        blocks.len() as u64 + 1,
        20_000_003,
        1600,
        -901,
    ));
    let mut source = source(
        factory(16_000, blocks, pulls, events),
        PreparedSampleRate::Hz16000,
        cancel,
        cpu.clone(),
        owners.clone(),
    );
    let before = tokio::time::timeout(TEST_TIMEOUT, source.pull())
        .await
        .unwrap()
        .unwrap();
    assert!(!before.eof);
    assert_eq!(before.chunks.len(), 2);
    assert!(
        before
            .chunks
            .iter()
            .all(|chunk| chunk.stamp().run_id == 1 && chunk.audio_end_us() <= 10_100_003)
    );
    assert_eq!(
        before.chunks[1].audio_end_us(),
        before.chunks[1].stamp().body_end_us
    );
    let after = tokio::time::timeout(TEST_TIMEOUT, source.pull())
        .await
        .unwrap()
        .unwrap();
    assert!(after.eof);
    assert_eq!(after.chunks.len(), 1);
    let chunk = &after.chunks[0];
    assert_eq!(chunk.stamp().run_id, 2);
    assert_eq!(chunk.stamp().sequence, 3);
    assert!(chunk.stamp().discontinuity);
    assert_eq!(chunk.stamp().audio_start_us, 20_000_003);
    assert_eq!(chunk.stamp().body_start_us, 20_000_003);
    assert!(chunk.samples().iter().all(|sample| *sample == -901));
    assert_eq!(chunk.audio_end_us(), chunk.stamp().body_end_us);
    assert_eq!(owners.available_permits(), 1);
    assert_eq!(cpu.available_permits(), 1);
}

#[tokio::test]
async fn cpu_saturation_is_immediate_overload_and_never_calls_factory() {
    let cpu = lane();
    let owners = lane();
    let held = cpu.clone().try_acquire_owned().unwrap();
    let cancel = ResolverCancellation::default();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let mut source = source(
        Box::new(move |_| {
            observed.fetch_add(1, Ordering::SeqCst);
            Err(MediaError::Unavailable)
        }),
        PreparedSampleRate::Hz16000,
        cancel.clone(),
        cpu.clone(),
        owners.clone(),
    );
    assert_eq!(
        bounded_pull(&mut source).await.err(),
        Some(PreparedAsrError::Media(MediaError::Overloaded))
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(cancel.is_cancelled());
    assert_eq!(owners.available_permits(), 1);
    drop(held);
    assert_eq!(cpu.available_permits(), 1);
}

#[tokio::test]
async fn native_owner_saturation_releases_step_cpu_and_never_calls_factory() {
    let cpu = lane();
    let owners = lane();
    let held = owners.clone().try_acquire_owned().unwrap();
    let cancel = ResolverCancellation::default();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let mut source = source(
        Box::new(move |_| {
            observed.fetch_add(1, Ordering::SeqCst);
            Err(MediaError::Unavailable)
        }),
        PreparedSampleRate::Hz16000,
        cancel.clone(),
        cpu.clone(),
        owners.clone(),
    );
    assert_eq!(
        bounded_pull(&mut source).await.err(),
        Some(PreparedAsrError::Media(MediaError::Overloaded))
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(cpu.available_permits(), 1);
    assert!(cancel.is_cancelled());
    drop(held);
    assert_eq!(owners.available_permits(), 1);
}

#[tokio::test]
async fn provider_idle_retains_native_owner_but_releases_cpu_and_stops_reading() {
    let cpu = lane();
    let owners = lane();
    let cancel = ResolverCancellation::default();
    let pulls = Arc::new(AtomicUsize::new(0));
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut source = source(
        factory(16_000, run(16_000, 0, 176_000), pulls.clone(), events),
        PreparedSampleRate::Hz16000,
        cancel.clone(),
        cpu.clone(),
        owners.clone(),
    );
    let first = tokio::time::timeout(TEST_TIMEOUT, source.pull())
        .await
        .unwrap()
        .unwrap();
    assert!(!first.eof);
    assert_eq!(first.chunks.len(), 1);
    assert_eq!(pulls.load(Ordering::SeqCst), 11);
    assert_eq!(owners.available_permits(), 0);
    let cpu_for_other_work = cpu.clone().try_acquire_owned().unwrap();
    for _ in 0..3 {
        tokio::task::yield_now().await;
    }
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(pulls.load(Ordering::SeqCst), 11);
    drop(cpu_for_other_work);
    let last = tokio::time::timeout(TEST_TIMEOUT, source.pull())
        .await
        .unwrap()
        .unwrap();
    assert!(last.eof);
    assert_eq!(last.chunks.len(), 1);
    assert_eq!(
        last.chunks[0].audio_end_us(),
        last.chunks[0].stamp().body_end_us
    );
    assert_eq!(owners.available_permits(), 1);
    assert_eq!(cpu.available_permits(), 1);
    assert!(!cancel.is_cancelled());
}

#[tokio::test]
async fn parent_cancellation_wakes_idle_native_thread_without_another_pull() {
    let caller = thread::current().id();
    let cpu = lane();
    let owners = lane();
    let parent = ResolverCancellation::default();
    let cancel = ResolverCancellation::linked(parent.clone()).unwrap();
    let pulls = Arc::new(AtomicUsize::new(0));
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut source = source(
        factory(
            16_000,
            run(16_000, 0, 176_000),
            pulls.clone(),
            events.clone(),
        ),
        PreparedSampleRate::Hz16000,
        cancel,
        cpu.clone(),
        owners.clone(),
    );
    assert!(!bounded_pull(&mut source).await.unwrap().eof);
    parent.cancel();
    released(&cpu, &owners).await;
    assert_eq!(pulls.load(Ordering::SeqCst), 11);
    assert_affinity(&events, caller);
    assert_eq!(
        bounded_pull(&mut source).await.err(),
        Some(PreparedAsrError::Media(MediaError::Cancelled))
    );
}

#[tokio::test]
async fn dropped_started_factory_retains_both_permits_until_real_exit() {
    let caller = thread::current().id();
    let cpu = lane();
    let owners = lane();
    let cancel = ResolverCancellation::default();
    let gate = Arc::new(Gate::default());
    let _release = ReleaseOnDrop(gate.clone());
    let events = Arc::new(Mutex::new(Vec::new()));
    let observed_events = events.clone();
    let observed_gate = gate.clone();
    let mut source = source(
        Box::new(move |_| {
            observed_gate.wait()?;
            event(&observed_events, "create");
            Ok(Box::new(SyntheticDecoder {
                rate: 16_000,
                blocks: VecDeque::new(),
                eof_error: None,
                panic_at_eof: false,
                next_gate: None,
                pulls: Arc::new(AtomicUsize::new(0)),
                events: observed_events,
                owner: thread::current().id(),
                _affine: Rc::new(()),
            }))
        }),
        PreparedSampleRate::Hz16000,
        cancel.clone(),
        cpu.clone(),
        owners.clone(),
    );
    let mut pull = Box::pin(source.pull());
    tokio::select! {
        () = wait_for(|| gate.entered.load(Ordering::SeqCst)) => {},
        _ = &mut pull => panic!("blocked synthetic factory returned early"),
    }
    drop(pull);
    assert!(cancel.is_cancelled());
    assert_eq!(cpu.available_permits(), 0);
    assert_eq!(owners.available_permits(), 0);
    gate.release();
    released(&cpu, &owners).await;
    assert_affinity(&events, caller);
}

#[tokio::test]
async fn dropped_started_next_retains_both_permits_until_native_object_drops() {
    let caller = thread::current().id();
    let cpu = lane();
    let owners = lane();
    let cancel = ResolverCancellation::default();
    let gate = Arc::new(Gate::default());
    let _release = ReleaseOnDrop(gate.clone());
    let observed_gate = gate.clone();
    let events = Arc::new(Mutex::new(Vec::new()));
    let observed_events = events.clone();
    let pulls = Arc::new(AtomicUsize::new(0));
    let observed_pulls = pulls.clone();
    let mut source = source(
        Box::new(move |_| {
            event(&observed_events, "create");
            Ok(Box::new(SyntheticDecoder {
                rate: 16_000,
                blocks: vec![block(16_000, 1, 0, 160, 1)].into(),
                eof_error: None,
                panic_at_eof: false,
                next_gate: Some(observed_gate),
                pulls: observed_pulls,
                events: observed_events,
                owner: thread::current().id(),
                _affine: Rc::new(()),
            }))
        }),
        PreparedSampleRate::Hz16000,
        cancel.clone(),
        cpu.clone(),
        owners.clone(),
    );
    let mut pull = Box::pin(source.pull());
    tokio::select! {
        () = wait_for(|| gate.entered.load(Ordering::SeqCst)) => {},
        _ = &mut pull => panic!("blocked synthetic next returned early"),
    }
    drop(pull);
    assert!(cancel.is_cancelled());
    assert_eq!(cpu.available_permits(), 0);
    assert_eq!(owners.available_permits(), 0);
    assert_eq!(pulls.load(Ordering::SeqCst), 1);
    gate.release();
    released(&cpu, &owners).await;
    assert_affinity(&events, caller);
}

#[tokio::test]
async fn native_typed_error_after_an_earlier_batch_never_becomes_eof_and_lanes_recover() {
    let cpu = lane();
    let owners = lane();
    let cancel = ResolverCancellation::default();
    let events = Arc::new(Mutex::new(Vec::new()));
    let observed = events.clone();
    let pulls = Arc::new(AtomicUsize::new(0));
    let observed_pulls = pulls.clone();
    let mut source = source(
        Box::new(move |_| {
            event(&observed, "create");
            Ok(Box::new(SyntheticDecoder {
                rate: 16_000,
                blocks: run(16_000, 0, 176_000).into(),
                eof_error: Some(MediaError::UnsupportedMedia),
                panic_at_eof: false,
                next_gate: None,
                pulls: observed_pulls,
                events: observed,
                owner: thread::current().id(),
                _affine: Rc::new(()),
            }))
        }),
        PreparedSampleRate::Hz16000,
        cancel.clone(),
        cpu.clone(),
        owners.clone(),
    );
    assert!(!bounded_pull(&mut source).await.unwrap().eof);
    assert_eq!(
        bounded_pull(&mut source).await.err(),
        Some(PreparedAsrError::Media(MediaError::UnsupportedMedia))
    );
    assert!(cancel.is_cancelled());
    released(&cpu, &owners).await;
    assert_eq!(pulls.load(Ordering::SeqCst), 12);
    let mut fresh = self::source(
        factory(
            16_000,
            vec![block(16_000, 1, 0, 16, 5)],
            Arc::new(AtomicUsize::new(0)),
            Arc::new(Mutex::new(Vec::new())),
        ),
        PreparedSampleRate::Hz16000,
        ResolverCancellation::default(),
        cpu.clone(),
        owners.clone(),
    );
    assert!(bounded_pull(&mut fresh).await.unwrap().eof);
    assert_eq!(cpu.available_permits(), 1);
    assert_eq!(owners.available_permits(), 1);
}

#[tokio::test]
async fn factory_and_native_panics_return_worker_error_and_release_capacity() {
    for factory_panic in [true, false] {
        let cpu = lane();
        let owners = lane();
        let cancel = ResolverCancellation::default();
        let events = Arc::new(Mutex::new(Vec::new()));
        let observed = events.clone();
        let drop_counts = Arc::new(Mutex::new(None));
        let observed_drop = drop_counts.clone();
        let observed_cpu = cpu.clone();
        let observed_owners = owners.clone();
        let mut source = source(
            Box::new(move |_| {
                assert!(!factory_panic, "explicit synthetic factory panic");
                event(&observed, "create");
                Ok(Box::new(PermitProbeDecoder {
                    inner: SyntheticDecoder {
                        rate: 16_000,
                        blocks: VecDeque::new(),
                        eof_error: None,
                        panic_at_eof: true,
                        next_gate: None,
                        pulls: Arc::new(AtomicUsize::new(0)),
                        events: observed,
                        owner: thread::current().id(),
                        _affine: Rc::new(()),
                    },
                    cpu: observed_cpu,
                    owners: observed_owners,
                    drop_counts: observed_drop,
                }))
            }),
            PreparedSampleRate::Hz16000,
            cancel.clone(),
            cpu.clone(),
            owners.clone(),
        );
        assert_eq!(
            tokio::time::timeout(TEST_TIMEOUT, source.pull())
                .await
                .unwrap()
                .err(),
            Some(PreparedAsrError::Media(MediaError::WorkerFailed))
        );
        assert!(cancel.is_cancelled());
        released(&cpu, &owners).await;
        if !factory_panic {
            assert_affinity(&events, thread::current().id());
            assert_eq!(
                *drop_counts.lock().unwrap(),
                Some((0, 0)),
                "panic cleanup must destroy the native object under both retained permits"
            );
        }
        let mut fresh = self::source(
            factory(
                16_000,
                vec![block(16_000, 1, 0, 16, 9)],
                Arc::new(AtomicUsize::new(0)),
                Arc::new(Mutex::new(Vec::new())),
            ),
            PreparedSampleRate::Hz16000,
            ResolverCancellation::default(),
            cpu.clone(),
            owners.clone(),
        );
        assert!(bounded_pull(&mut fresh).await.unwrap().eof);
        assert_eq!(owners.available_permits(), 1);
    }
}

#[tokio::test]
async fn first_actual_output_after_a_subframe_native_run_starts_without_false_gap() {
    for rate in [PreparedSampleRate::Hz16000, PreparedSampleRate::Hz24000] {
        let cpu = lane();
        let owners = lane();
        let events = Arc::new(Mutex::new(Vec::new()));
        let pulls = Arc::new(AtomicUsize::new(0));
        let mut source = source(
            factory(
                192_000,
                vec![
                    block(192_000, 1, 0, 1, 17),
                    block(192_000, 2, 1_000_000, 1920, -201),
                ],
                pulls,
                events,
            ),
            rate,
            ResolverCancellation::default(),
            cpu.clone(),
            owners.clone(),
        );
        let batch = bounded_pull(&mut source).await.unwrap();
        assert!(batch.eof);
        assert_eq!(batch.chunks.len(), 1);
        let chunk = &batch.chunks[0];
        assert_eq!(chunk.stamp().run_id, 1);
        assert!(!chunk.stamp().discontinuity);
        assert_eq!(chunk.stamp().audio_start_us, 1_000_000);
        assert_eq!(chunk.stamp().body_end_us, 1_010_000);
        assert_eq!(chunk.audio_end_us(), chunk.stamp().body_end_us);
        assert_eq!(chunk.samples().len(), rate.hz() as usize / 100);
        assert!(chunk.samples().iter().all(|sample| *sample == -201));
        assert_eq!(cpu.available_permits(), 1);
        assert_eq!(owners.available_permits(), 1);
    }
}

#[cfg(windows)]
struct PanicDuringDropDecoder(SyntheticDecoder);
#[cfg(windows)]
impl MediaDecoder for PanicDuringDropDecoder {
    fn format(&self) -> DecodedAudioFormat {
        self.0.format()
    }
    fn next(
        &mut self,
        cancel: &ResolverCancellation,
    ) -> Result<Option<DecodedAudioBlock>, MediaError> {
        self.0.next(cancel)
    }
}
#[cfg(windows)]
impl Drop for PanicDuringDropDecoder {
    fn drop(&mut self) {
        panic!("explicit synthetic native destruction panic");
    }
}

#[cfg(windows)]
#[tokio::test]
async fn eof_pin_is_dropped_when_native_destruction_prevents_reply_and_join_fails() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("synthetic-pin.bin");
    std::fs::write(&path, b"opaque test owner, no captured media").unwrap();
    let input = MediaInput::local(path.clone()).unwrap();
    let cpu = lane();
    let owners = lane();
    let cancel = ResolverCancellation::default();
    let events = Arc::new(Mutex::new(Vec::new()));
    let observed = events.clone();
    let mut source = PreparedLocalSource::with_pinned_factory(
        identity(),
        PreparedSampleRate::Hz16000,
        cancel.clone(),
        Box::new(move |cancel| {
            let media = super::super::input::open_media(&input, cancel)?;
            event(&observed, "create");
            Ok(OpenedDecoder {
                decoder: Box::new(PanicDuringDropDecoder(SyntheticDecoder {
                    rate: 16_000,
                    blocks: vec![block(16_000, 1, 0, 160, 9)].into(),
                    eof_error: None,
                    panic_at_eof: false,
                    next_gate: None,
                    pulls: Arc::new(AtomicUsize::new(0)),
                    events: observed,
                    owner: thread::current().id(),
                    _affine: Rc::new(()),
                })),
                media: Some(media),
                clock: false,
            })
        }),
        cpu.clone(),
        owners.clone(),
    )
    .unwrap();
    assert_eq!(
        bounded_pull(&mut source).await.err(),
        Some(PreparedAsrError::Media(MediaError::WorkerFailed))
    );
    assert!(cancel.is_cancelled());
    released(&cpu, &owners).await;
    assert_affinity(&events, thread::current().id());
    assert!(std::fs::OpenOptions::new().write(true).open(&path).is_ok());
    assert!(std::fs::remove_file(&path).is_ok());
}
