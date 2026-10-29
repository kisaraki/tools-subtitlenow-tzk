//! Lifecycle regressions use original synthetic fixtures only. No capture,
//! external content, network, credentials, or detached asynchronous tasks.
use super::*;
use crate::{audio::capture::SessionId, resolver::candidates::SongState};
use std::{
    sync::{
        Barrier, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc::sync_channel,
    },
    thread,
};

fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(8301),
        generation: 4,
    }
}
fn capabilities() -> ResolverCapabilities {
    ResolverCapabilities {
        classification: true,
        songs: true,
        movies: false,
        cancellation: true,
    }
}
fn window(sequence: u64, ended_at: Instant) -> Arc<AudioWindow> {
    Arc::new(AudioWindow {
        stamp: WindowStamp {
            identity: identity(),
            sequence,
            ended_at,
            start_us: None,
            duration_us: WINDOW_SECONDS as u64 * 1_000_000,
        },
        pcm: vec![0; WINDOW_FRAMES].into(),
    })
}
fn pair() -> (ResolverSession, ResolverTask) {
    ResolverSession::new(identity(), capabilities()).unwrap()
}
fn publication(task: &ResolverTask, stamp: WindowStamp) -> Weak<SongCandidate> {
    let song = Arc::new(SongCandidate {
        id: "synthetic-lifecycle-song".into(),
        title: "Original lifecycle fixture".into(),
        artist: Some("Synthetic fixture author".into()),
        confidence: Confidence::new(900).unwrap(),
        match_offset_us: None,
    });
    let weak = Arc::downgrade(&song);
    task.publish(
        stamp,
        ResolverStatus::Ready,
        Resolution {
            kind: ContentType::Music,
            song_state: SongState::Confirmed,
            song: Some(song),
            movies: Vec::new(),
        },
    );
    weak
}
fn assert_empty(update: &ResolverUpdate, status: ResolverStatus) {
    assert_eq!(update.status, status);
    assert_eq!(update.resolution.kind, ContentType::Unknown);
    assert_eq!(update.resolution.song_state, SongState::Unknown);
    assert!(update.resolution.song.is_none());
    assert!(update.resolution.movies.is_empty());
}

#[derive(Default)]
struct PendingIdentifier {
    token: Mutex<Option<ResolverCancellation>>,
    active: Arc<AtomicUsize>,
    calls: AtomicUsize,
}
impl PendingIdentifier {
    fn token(&self) -> ResolverCancellation {
        self.token.lock().unwrap().as_ref().unwrap().clone()
    }
}
struct Active(Arc<AtomicUsize>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
impl ContentIdentifier for PendingIdentifier {
    fn capabilities(&self) -> ResolverCapabilities {
        capabilities()
    }
    fn identify<'a>(
        &'a self,
        _: &'a AudioWindow,
        cancel: ResolverCancellation,
    ) -> IdentifyFuture<'a> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.active.fetch_add(1, Ordering::SeqCst);
            let _active = Active(self.active.clone());
            *self.token.lock().unwrap() = Some(cancel.clone());
            cancel.cancelled().await;
            Err(ResolverError::Cancelled)
        })
    }
}

#[tokio::test]
async fn invalidate_cancels_active_request_before_task_is_polled_again() {
    let provider = PendingIdentifier::default();
    let (mut session, task) = pair();
    let input = window(1, Instant::now());
    let stamp = input.stamp;
    session.try_submit(input).unwrap();
    let mut job = Box::pin(task.run(&provider));
    assert!(futures_util::poll!(job.as_mut()).is_pending());
    let token = provider.token();
    assert!(!token.is_cancelled());

    session.invalidate().unwrap();

    assert!(token.is_cancelled());
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    let reset = session.poll().unwrap();
    assert_empty(
        &reset,
        ResolverStatus::Failed(ResolverError::AudioDiscontinuity),
    );
    assert_eq!(reset.stamp.sequence, stamp.sequence);
    assert!(session.poll().is_none());
    assert!(!session.cancellation().is_cancelled());
    drop(job);
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
}

#[test]
fn invalidate_releases_unread_metadata_without_polling_the_task() {
    let (mut session, task) = pair();
    let input = window(1, Instant::now());
    let stamp = input.stamp;
    session.try_submit(input).unwrap();
    let weak = publication(&task, stamp);
    assert!(weak.upgrade().is_some());

    session.invalidate().unwrap();

    assert!(weak.upgrade().is_none());
    let slot = session.output.borrow();
    assert_empty(
        slot.as_ref().unwrap(),
        ResolverStatus::Failed(ResolverError::AudioDiscontinuity),
    );
}

#[tokio::test]
async fn replacement_cancels_active_request_without_task_progress() {
    let provider = PendingIdentifier::default();
    let (mut session, task) = pair();
    session.try_submit(window(1, Instant::now())).unwrap();
    let mut job = Box::pin(task.run(&provider));
    assert!(futures_util::poll!(job.as_mut()).is_pending());
    let token = provider.token();
    assert!(session.poll().is_some());

    session.try_submit(window(2, Instant::now())).unwrap();

    assert!(token.is_cancelled());
    assert!(session.output.borrow().is_none());
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    assert!(!session.cancellation().is_cancelled());
    drop(job);
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
}

#[test]
fn replacement_releases_unread_result_and_rejects_late_same_epoch_publication() {
    let (mut session, task) = pair();
    let first = window(1, Instant::now());
    let first_stamp = first.stamp;
    session.try_submit(first).unwrap();
    let old = publication(&task, first_stamp);
    let second = window(2, Instant::now());
    let second_stamp = second.stamp;

    session.try_submit(second).unwrap();

    assert!(old.upgrade().is_none());
    assert!(session.output.borrow().is_none());
    let late = publication(&task, first_stamp);
    assert!(late.upgrade().is_none());
    assert!(session.output.borrow().is_none());
    publication(&task, second_stamp);
    let latest = session.poll().unwrap();
    assert_eq!(latest.status, ResolverStatus::Ready);
    assert_eq!(latest.stamp.sequence, second_stamp.sequence);
}

#[test]
fn external_cancellation_filters_unread_ready_before_task_progress() {
    let (mut session, task) = pair();
    let input = window(1, Instant::now());
    let stamp = input.stamp;
    session.try_submit(input).unwrap();
    publication(&task, stamp);

    session.cancellation().cancel();

    assert!(
        session
            .poll()
            .is_none_or(|update| update.status == ResolverStatus::Stopped)
    );
    assert!(session.poll().is_none());
}

#[tokio::test]
async fn next_wakes_on_external_cancellation_without_polling_the_task() {
    let (mut session, _task) = pair();
    let cancel = session.cancellation();
    let mut next = Box::pin(session.next());
    assert!(futures_util::poll!(next.as_mut()).is_pending());

    cancel.cancel();

    let result = tokio::time::timeout(Duration::from_secs(1), next)
        .await
        .unwrap();
    assert!(result.is_none_or(|update| update.status == ResolverStatus::Stopped));
}

#[tokio::test]
async fn stop_clears_result_and_has_one_terminal_notification_without_task_progress() {
    let (mut session, task) = pair();
    let input = window(1, Instant::now());
    let stamp = input.stamp;
    session.try_submit(input).unwrap();
    let weak = publication(&task, stamp);

    session.stop();

    assert!(weak.upgrade().is_none());
    assert!(session.cancellation().is_cancelled());
    assert!(session.input.borrow().window.is_none());
    let stopped = session.next().await.unwrap();
    assert_empty(&stopped, ResolverStatus::Stopped);
    assert_eq!(stopped.stamp.sequence, stamp.sequence);
    session.stop();
    assert!(session.poll().is_none());
    assert!(session.next().await.is_none());
    let late = publication(&task, stamp);
    assert!(late.upgrade().is_none());
    assert_empty(
        session.output.borrow().as_ref().unwrap(),
        ResolverStatus::Stopped,
    );
}

#[tokio::test]
async fn stop_cancels_active_token_immediately_and_future_drop_releases_pcm() {
    let provider = PendingIdentifier::default();
    let (mut session, task) = pair();
    let input = window(1, Instant::now());
    let weak = Arc::downgrade(&input);
    session.try_submit(input).unwrap();
    let mut job = Box::pin(task.run(&provider));
    assert!(futures_util::poll!(job.as_mut()).is_pending());
    let token = provider.token();

    session.stop();

    assert!(token.is_cancelled());
    assert_empty(&session.poll().unwrap(), ResolverStatus::Stopped);
    drop(job);
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
    assert!(weak.upgrade().is_none());
}

#[tokio::test]
async fn owner_drop_cancels_active_token_and_clears_shared_publication_slot() {
    let provider = PendingIdentifier::default();
    let (mut session, task) = pair();
    let output = task.output.subscribe();
    session.try_submit(window(1, Instant::now())).unwrap();
    let mut job = Box::pin(task.run(&provider));
    assert!(futures_util::poll!(job.as_mut()).is_pending());
    let token = provider.token();

    drop(session);

    assert!(token.is_cancelled());
    assert_empty(output.borrow().as_ref().unwrap(), ResolverStatus::Stopped);
    drop(job);
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn dropping_task_future_publishes_empty_terminal_state_and_cancels_request() {
    let provider = PendingIdentifier::default();
    let (mut session, task) = pair();
    session.try_submit(window(1, Instant::now())).unwrap();
    let mut job = Box::pin(task.run(&provider));
    assert!(futures_util::poll!(job.as_mut()).is_pending());
    let token = provider.token();

    drop(job);

    assert!(token.is_cancelled());
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
    assert_empty(&session.poll().unwrap(), ResolverStatus::Stopped);
    assert!(session.next().await.is_none());
}

#[tokio::test]
async fn stopping_without_input_does_not_fabricate_a_window_stamp() {
    let (mut session, _task) = pair();
    session.stop();
    session.stop();
    assert!(session.cancellation().is_cancelled());
    assert!(session.output.borrow().is_none());
    assert!(session.poll().is_none());
    assert!(session.next().await.is_none());
}

#[tokio::test]
async fn stale_ready_and_resolving_clear_read_and_unread_results_without_task_progress() {
    let ended_at = Instant::now() - Duration::from_secs(4);
    let mut fixtures = Vec::new();
    for status in [ResolverStatus::Ready, ResolverStatus::Resolving] {
        for consumed in [false, true] {
            let (mut session, task) = pair();
            let input = window(1, ended_at);
            let stamp = input.stamp;
            session.try_submit(input).unwrap();
            let weak = if status == ResolverStatus::Ready {
                Some(publication(&task, stamp))
            } else {
                task.publish(stamp, status, Resolution::default());
                None
            };
            if consumed {
                let update = session.poll().unwrap();
                assert_eq!(update.status, status);
                drop(update);
            }
            fixtures.push((session, task, weak));
        }
    }

    tokio::time::sleep(Duration::from_millis(1100)).await;

    for (mut session, _task, weak) in fixtures {
        let failure = session.poll().unwrap();
        assert_empty(&failure, ResolverStatus::Failed(ResolverError::Stale));
        assert_eq!(failure.stamp.sequence, 1);
        assert!(session.input.borrow().window.is_none());
        assert!(session.poll().is_none());
        assert!(weak.is_none_or(|weak| weak.upgrade().is_none()));
        assert!(!session.cancellation().is_cancelled());
    }
}

#[tokio::test]
async fn next_wakes_at_freshness_deadline_without_new_input_or_task_progress() {
    let (mut session, task) = pair();
    let input = window(1, Instant::now() - Duration::from_secs(4));
    let stamp = input.stamp;
    session.try_submit(input).unwrap();
    let weak = publication(&task, stamp);
    assert_eq!(session.poll().unwrap().status, ResolverStatus::Ready);

    let failure = tokio::time::timeout(Duration::from_secs(2), session.next())
        .await
        .unwrap()
        .unwrap();

    assert_empty(&failure, ResolverStatus::Failed(ResolverError::Stale));
    assert!(weak.upgrade().is_none());
    assert!(session.input.borrow().window.is_none());
}

#[test]
fn concurrent_invalidation_and_publication_leave_no_hidden_old_result_in_200_races() {
    let (mut session, mut task) = pair();
    let (start, receive) = sync_channel::<(u64, WindowStamp, Arc<Barrier>)>(1);
    let (done, completed) = sync_channel::<Weak<SongCandidate>>(1);
    let worker = thread::spawn(move || {
        while let Ok((epoch, stamp, barrier)) = receive.recv() {
            task.epoch = epoch;
            barrier.wait();
            let weak = publication(&task, stamp);
            done.send(weak).unwrap();
        }
    });

    for sequence in 1..=200 {
        let input = window(sequence, Instant::now());
        let stamp = input.stamp;
        session.try_submit(input).unwrap();
        let barrier = Arc::new(Barrier::new(2));
        start
            .send((session.epoch, stamp, Arc::clone(&barrier)))
            .unwrap();
        barrier.wait();
        session.invalidate().unwrap();
        let weak = completed.recv_timeout(Duration::from_secs(5)).unwrap();

        // Inspect the actual slot, not only poll's epoch filtering. An old write
        // after the clear would retain metadata here even if poll rejects it.
        assert!(weak.upgrade().is_none());
        let slot = session.output.borrow();
        let update = slot.as_ref().unwrap();
        assert_empty(
            update,
            ResolverStatus::Failed(ResolverError::AudioDiscontinuity),
        );
        assert_eq!(update.epoch, session.epoch);
        assert_eq!(update.stamp.sequence, sequence);
    }
    drop(start);
    worker.join().unwrap();
}

#[test]
fn reset_epoch_overflow_cancels_and_releases_content_instead_of_reusing_an_epoch() {
    let (mut session, mut task) = pair();
    let input = window(1, Instant::now());
    let stamp = input.stamp;
    session.try_submit(input).unwrap();
    let weak = publication(&task, stamp);
    session.epoch = u64::MAX;
    session.input.send_modify(|frame| frame.epoch = u64::MAX);
    task.epoch = u64::MAX;

    assert_eq!(session.invalidate(), Err(ResolverError::InvalidData));

    assert!(session.cancellation().is_cancelled());
    assert!(weak.upgrade().is_none());
    assert!(session.input.borrow().window.is_none());
    assert!(
        session
            .poll()
            .is_none_or(|update| update.status == ResolverStatus::Stopped)
    );
    assert_eq!(
        session.try_submit(window(2, Instant::now())),
        Err(ResolverError::Cancelled)
    );
    let late = publication(&task, stamp);
    assert!(late.upgrade().is_none());
    assert!(
        session
            .output
            .borrow()
            .as_ref()
            .is_none_or(|update| update.status == ResolverStatus::Stopped)
    );
}

#[tokio::test]
async fn task_reset_keeps_the_owners_latest_stamp_and_does_not_repeat_its_notification() {
    struct ImmediateIdentifier;
    impl ContentIdentifier for ImmediateIdentifier {
        fn capabilities(&self) -> ResolverCapabilities {
            capabilities()
        }
        fn identify<'a>(
            &'a self,
            _: &'a AudioWindow,
            _: ResolverCancellation,
        ) -> IdentifyFuture<'a> {
            Box::pin(async {
                Ok(ContentEvidence {
                    kind: ContentType::Speech,
                    confidence: Confidence::new(900).unwrap(),
                    songs: Vec::new(),
                    movies: Vec::new(),
                })
            })
        }
    }
    let (mut session, task) = pair();
    session.try_submit(window(1, Instant::now())).unwrap();
    let mut job = Box::pin(task.run(&ImmediateIdentifier));
    assert!(futures_util::poll!(job.as_mut()).is_pending());
    let first = session.poll().unwrap();
    assert_eq!(first.status, ResolverStatus::Ready);
    assert_eq!(first.stamp.sequence, 1);
    drop(first);

    // The task still remembers window 1. Window 2 never reaches the provider.
    session.try_submit(window(2, Instant::now())).unwrap();
    session.invalidate().unwrap();
    let reset = session.poll().unwrap();
    assert_empty(
        &reset,
        ResolverStatus::Failed(ResolverError::AudioDiscontinuity),
    );
    assert_eq!(reset.stamp.sequence, 2);

    assert!(futures_util::poll!(job.as_mut()).is_pending());

    {
        let slot = session.output.borrow();
        let current = slot.as_ref().unwrap();
        assert!(Arc::ptr_eq(current, &reset));
        assert_eq!(current.stamp.sequence, 2);
    }
    assert!(session.poll().is_none());
    drop(job);
}

#[tokio::test]
async fn task_drop_preserves_a_consumed_stop_snapshot_and_its_latest_window_stamp() {
    let provider = PendingIdentifier::default();
    let (mut session, task) = pair();
    session.try_submit(window(1, Instant::now())).unwrap();
    let mut job = Box::pin(task.run(&provider));
    assert!(futures_util::poll!(job.as_mut()).is_pending());
    session.try_submit(window(2, Instant::now())).unwrap();
    session.stop();
    let stopped = session.poll().unwrap();
    assert_empty(&stopped, ResolverStatus::Stopped);
    assert_eq!(stopped.stamp.sequence, 2);

    drop(job);

    {
        let slot = session.output.borrow();
        let current = slot.as_ref().unwrap();
        assert!(Arc::ptr_eq(current, &stopped));
        assert_eq!(current.stamp.sequence, 2);
    }
    assert!(session.poll().is_none());
    assert!(session.next().await.is_none());
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn dropping_an_unrun_task_uses_the_queued_real_stamp_and_next_terminates() {
    let (mut session, task) = pair();
    session.try_submit(window(2, Instant::now())).unwrap();

    drop(task);

    assert!(session.cancellation().is_cancelled());
    assert!(session.input.borrow().window.is_none());
    let stopped = session.next().await.unwrap();
    assert_empty(&stopped, ResolverStatus::Stopped);
    assert_eq!(stopped.stamp.identity, identity());
    assert_eq!(stopped.stamp.sequence, 2);
    assert!(session.poll().is_none());
    assert!(session.next().await.is_none());
}

#[test]
fn replacement_cannot_erase_a_valid_new_window_publication_in_200_races() {
    let (mut session, mut task) = pair();
    session.try_submit(window(1, Instant::now())).unwrap();
    let (start, receive) = sync_channel::<(WindowStamp, Arc<Barrier>)>(1);
    let (done, completed) = sync_channel::<()>(1);
    let worker = thread::spawn(move || {
        while let Ok((stamp, barrier)) = receive.recv() {
            barrier.wait();
            let deadline = Instant::now() + Duration::from_secs(5);
            while task
                .input
                .borrow()
                .window
                .as_ref()
                .is_none_or(|window| window.stamp.sequence != stamp.sequence)
            {
                assert!(Instant::now() < deadline, "new input never became visible");
                thread::yield_now();
            }
            task.epoch = task.input.borrow().epoch;
            publication(&task, stamp);
            done.send(()).unwrap();
        }
    });
    for sequence in 2..=201 {
        let input = window(sequence, Instant::now());
        let stamp = input.stamp;
        let barrier = Arc::new(Barrier::new(2));
        start.send((stamp, barrier.clone())).unwrap();
        barrier.wait();
        session.try_submit(input).unwrap();
        completed.recv_timeout(Duration::from_secs(5)).unwrap();
        let update = session.output.borrow();
        let update = update.as_ref().unwrap();
        assert_eq!(update.status, ResolverStatus::Ready);
        assert_eq!(update.stamp.sequence, sequence);
        assert!(update.resolution.song.is_some());
    }
    drop(start);
    worker.join().unwrap();
}

#[test]
fn task_drop_and_new_submission_leave_no_pending_audio_in_200_races() {
    let (start, receive) = sync_channel::<(ResolverTask, Arc<Barrier>)>(1);
    let (done, completed) = sync_channel::<()>(1);
    let worker = thread::spawn(move || {
        while let Ok((task, barrier)) = receive.recv() {
            barrier.wait();
            drop(task);
            done.send(()).unwrap();
        }
    });
    for _ in 0..200 {
        let (mut session, task) = pair();
        let first = window(1, Instant::now());
        let stamp = first.stamp;
        session.try_submit(first).unwrap();
        let weak = publication(&task, stamp);
        let barrier = Arc::new(Barrier::new(2));
        start.send((task, barrier.clone())).unwrap();
        barrier.wait();
        let result = session.try_submit(window(2, Instant::now()));
        assert!(matches!(result, Ok(()) | Err(ResolverError::Cancelled)));
        completed.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(session.cancellation().is_cancelled());
        assert!(session.input.borrow().window.is_none());
        assert!(weak.upgrade().is_none());
        assert_empty(&session.poll().unwrap(), ResolverStatus::Stopped);
        assert!(session.poll().is_none());
    }
    drop(start);
    worker.join().unwrap();
}
