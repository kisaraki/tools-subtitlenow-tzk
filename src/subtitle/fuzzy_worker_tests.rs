//! Synthetic owned documents; test gates emulate slow CPU cleanup, never I/O in production.
use super::*;
use crate::{
    ai::speech::{SpeechLanguage, StreamIdentity, TranscriptKind},
    audio::capture::SessionId,
    resolver::contracts::{SubtitleSource, TextCue, TextLanguage},
};
use futures_util::FutureExt;
use std::{
    sync::{
        Weak,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    task::Poll,
    time::Duration,
};
use tokio::sync::oneshot;

const LINE: &str = "the synthetic subtitle belongs to this document";
fn identity(session: u64) -> StreamIdentity {
    StreamIdentity {
        session: SessionId(session),
        generation: 1,
    }
}
fn scope(epoch: u64) -> AlignmentScope {
    AlignmentScope::new(identity(901), identity(902), epoch).unwrap()
}
fn document() -> ExistingText {
    ExistingText {
        identity: identity(901),
        source: SubtitleSource::ExistingSubtitle,
        content_id: "synthetic-document".into(),
        language: TextLanguage::Source(SpeechLanguage::English),
        cues: vec![TextCue {
            start_us: Some(600_000_000),
            duration_us: Some(2_000_000),
            text: LINE.into(),
        }],
    }
}
fn event() -> TranscriptEvent {
    TranscriptEvent {
        identity: identity(902),
        utterance: 3,
        sequence: 4,
        revision: 1,
        start_us: Some(42_000_000),
        duration_us: Some(2_000_000),
        language: Some(SpeechLanguage::English),
        kind: TranscriptKind::Final,
        text: LINE.into(),
    }
}
fn stamp() -> WindowStamp {
    WindowStamp {
        identity: identity(902),
        sequence: 8,
        ended_at: Instant::now(),
        start_us: Some(40_000_000),
        duration_us: 12_000_000,
    }
}
fn owner(cpu: Arc<Semaphore>) -> FuzzyCueLocator {
    let mut owner = FuzzyCueLocator::new(scope(1), document()).unwrap();
    owner.workers = cpu;
    owner
}
async fn pending<F: Future>(future: std::pin::Pin<&mut F>) {
    let mut future = future;
    std::future::poll_fn(|cx| {
        assert!(future.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
}
fn gate() -> (
    oneshot::Sender<()>,
    oneshot::Receiver<()>,
    mpsc::SyncSender<()>,
    mpsc::Receiver<()>,
) {
    let (entered, entered_rx) = oneshot::channel();
    let (release, release_rx) = mpsc::sync_channel(1);
    (entered, entered_rx, release, release_rx)
}
fn gated(
    entered: oneshot::Sender<()>,
    release: mpsc::Receiver<()>,
) -> impl FnOnce(&WorkInput) -> Result<FuzzyCueProposal, FuzzyAlignmentError> + Send {
    move |input| {
        entered.send(()).unwrap();
        release.recv_timeout(Duration::from_secs(5)).unwrap();
        run_match(input)
    }
}
async fn reclaimed(cpu: &Arc<Semaphore>) {
    let permit = tokio::time::timeout(Duration::from_secs(2), cpu.clone().acquire_owned())
        .await
        .unwrap()
        .unwrap();
    drop(permit);
}
fn weak(owner: &FuzzyCueLocator) -> Weak<ExistingText> {
    match owner.document.as_ref().unwrap() {
        AlignmentDocument::Owned(document) => Arc::downgrade(document),
        AlignmentDocument::Prepared(_)
        | AlignmentDocument::Paired(_)
        | AlignmentDocument::Lookup(_) => {
            panic!("expected standalone fixture")
        }
    }
}

#[tokio::test]
async fn current_thread_entry_returns_metadata_and_stop_releases_document() {
    let cpu = Arc::new(Semaphore::new(1));
    let mut owner = owner(cpu.clone());
    let document = weak(&owner);
    let proposal = owner
        .locate(event(), stamp(), ResolverCancellation::default())
        .await
        .unwrap();
    assert_eq!(proposal.scope(), owner.scope());
    assert_eq!(proposal.capture_range_us(), 42_000_000..44_000_000);
    assert_eq!(proposal.media_range_us(), 600_000_000..602_000_000);
    assert_eq!(proposal.edit_distance(), 0);
    assert_eq!(cpu.available_permits(), 1);
    assert_eq!(document.strong_count(), 1);
    proposal
        .validate(owner.scope(), TimelineBinding::from_event(&event()))
        .unwrap();
    owner.stop();
    assert!(document.upgrade().is_none());
    assert_eq!(
        proposal.validate(owner.scope(), proposal.binding()),
        Err(ResolverError::Cancelled.into())
    );
    assert_eq!(
        owner
            .locate(event(), stamp(), ResolverCancellation::default())
            .await
            .err(),
        Some(ResolverError::Cancelled.into())
    );
}

#[test]
fn public_owners_share_existing_alignment_capacity_without_new_pool() {
    let first = FuzzyCueLocator::new(scope(1), document()).unwrap();
    let second = FuzzyCueLocator::new(scope(2), document()).unwrap();
    assert!(Arc::ptr_eq(&first.workers, &second.workers));
    assert!(Arc::ptr_eq(
        &first.workers,
        &super::super::alignment_work::workers()
    ));
    let cores = std::thread::available_parallelism().map_or(1, usize::from);
    assert!(first.workers.available_permits() <= cores.min(2));
    assert_eq!(first.local.available_permits(), 1);
}

#[test]
fn admission_rejects_wrong_identity_and_unbounded_owned_document_shape() {
    for kind in 0..6 {
        let mut document = document();
        match kind {
            0 => document.identity.generation += 1,
            1 => document.cues.clear(),
            2 => document.content_id = "x".repeat(MAX_METADATA_BYTES + 1).into(),
            3 => document.cues[0].text = "x".repeat(MAX_CUE_BYTES + 1).into(),
            4 => {
                document.cues = (0..=MAX_TEXT_CUES)
                    .map(|_| TextCue {
                        start_us: Some(1),
                        duration_us: Some(1),
                        text: LINE.into(),
                    })
                    .collect()
            }
            _ => {
                document.cues = (0..=MAX_TEXT_BYTES / MAX_CUE_BYTES)
                    .map(|_| TextCue {
                        start_us: Some(1),
                        duration_us: Some(1),
                        text: "x".repeat(MAX_CUE_BYTES).into(),
                    })
                    .collect()
            }
        }
        assert_eq!(
            FuzzyCueLocator::new(scope(1), document).err().unwrap(),
            ResolverError::InvalidData.into()
        );
    }
}

#[test]
fn polling_without_runtime_is_typed_and_does_not_consume_capacity() {
    let cpu = Arc::new(Semaphore::new(1));
    let owner = owner(cpu.clone());
    let result = owner
        .locate(event(), stamp(), ResolverCancellation::default())
        .now_or_never()
        .unwrap();
    assert_eq!(result.err(), Some(FuzzyAlignmentError::RuntimeUnavailable));
    assert_eq!(cpu.available_permits(), 1);
}

#[test]
fn unpolled_drop_starts_nothing_and_does_not_cancel_parent_or_owner() {
    let cpu = Arc::new(Semaphore::new(1));
    let owner = owner(cpu.clone());
    let document = weak(&owner);
    let parent = ResolverCancellation::default();
    let called = Arc::new(AtomicUsize::new(0));
    let counter = called.clone();
    let request = owner.locate_using(event(), stamp(), parent.clone(), move |input| {
        counter.fetch_add(1, Ordering::SeqCst);
        run_match(input)
    });
    drop(request);
    assert_eq!(called.load(Ordering::SeqCst), 0);
    assert_eq!(cpu.available_permits(), 1);
    assert_eq!(document.strong_count(), 1);
    assert!(!parent.is_cancelled());
    assert!(!owner.stopped.is_cancelled());
    drop(owner);
    assert!(document.upgrade().is_none());
}

#[tokio::test]
async fn precancel_and_wrong_window_never_dispatch_work() {
    let cpu = Arc::new(Semaphore::new(1));
    let owner = owner(cpu.clone());
    let cancel = ResolverCancellation::default();
    cancel.cancel();
    assert_eq!(
        owner.locate(event(), stamp(), cancel).await.err(),
        Some(ResolverError::Cancelled.into())
    );
    let mut wrong = stamp();
    wrong.identity = identity(999);
    assert_eq!(
        owner
            .locate(event(), wrong, ResolverCancellation::default())
            .await
            .err(),
        Some(ResolverError::InvalidData.into())
    );
    assert_eq!(cpu.available_permits(), 1);
    assert_eq!(owner.local.available_permits(), 1);
}

#[tokio::test]
async fn saturated_shared_lane_fails_without_queue_and_recovers() {
    let cpu = Arc::new(Semaphore::new(1));
    let owner = owner(cpu.clone());
    let held = cpu.clone().try_acquire_owned().unwrap();
    let result = owner
        .locate_using(event(), stamp(), ResolverCancellation::default(), |_| {
            panic!("overloaded job ran")
        })
        .await;
    assert_eq!(result.err(), Some(FuzzyAlignmentError::Overloaded));
    assert_eq!(owner.local.available_permits(), 1);
    drop(held);
    assert!(
        owner
            .locate(event(), stamp(), ResolverCancellation::default())
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn one_owner_cannot_run_two_requests_even_with_spare_global_capacity() {
    let cpu = Arc::new(Semaphore::new(2));
    let owner = owner(cpu.clone());
    let (entered, entered_rx, release, release_rx) = gate();
    let first = owner.locate_using(
        event(),
        stamp(),
        ResolverCancellation::default(),
        gated(entered, release_rx),
    );
    tokio::pin!(first);
    pending(first.as_mut()).await;
    entered_rx.await.unwrap();
    let second = owner
        .locate(event(), stamp(), ResolverCancellation::default())
        .await;
    assert_eq!(second.err(), Some(FuzzyAlignmentError::Overloaded));
    assert_eq!(cpu.available_permits(), 1);
    release.send(()).unwrap();
    assert!(first.await.is_ok());
    assert_eq!(cpu.available_permits(), 2);
}

#[tokio::test]
async fn explicit_cancel_joins_started_cleanup_before_returning() {
    let cpu = Arc::new(Semaphore::new(1));
    let owner = owner(cpu.clone());
    let cancel = ResolverCancellation::default();
    let (entered, entered_rx, release, release_rx) = gate();
    let request = owner.locate_using(event(), stamp(), cancel.clone(), gated(entered, release_rx));
    tokio::pin!(request);
    pending(request.as_mut()).await;
    entered_rx.await.unwrap();
    cancel.cancel();
    pending(request.as_mut()).await;
    assert_eq!(cpu.available_permits(), 0);
    release.send(()).unwrap();
    assert_eq!(request.await.err(), Some(ResolverError::Cancelled.into()));
    assert_eq!(cpu.available_permits(), 1);
    assert_eq!(owner.local.available_permits(), 1);
}

#[tokio::test]
async fn started_future_drop_keeps_document_and_permits_until_real_exit() {
    let cpu = Arc::new(Semaphore::new(1));
    let mut owner = owner(cpu.clone());
    let document = weak(&owner);
    let parent = ResolverCancellation::default();
    let (entered, entered_rx, release, release_rx) = gate();
    let mut request =
        Box::pin(owner.locate_using(event(), stamp(), parent.clone(), gated(entered, release_rx)));
    pending(request.as_mut()).await;
    entered_rx.await.unwrap();
    drop(request);
    owner.stop();
    assert_eq!(cpu.available_permits(), 0);
    assert_eq!(owner.local.available_permits(), 0);
    assert!(document.upgrade().is_some());
    assert!(!parent.is_cancelled());
    release.send(()).unwrap();
    reclaimed(&cpu).await;
    assert!(document.upgrade().is_none());
    assert_eq!(owner.local.available_permits(), 1);
}

#[tokio::test]
async fn owner_drop_cancels_inflight_and_cannot_replace_new_epoch_result() {
    let cpu = Arc::new(Semaphore::new(1));
    let first = owner(cpu.clone());
    let old_document = weak(&first);
    let (entered, entered_rx, release, release_rx) = gate();
    let request = first.locate_using(
        event(),
        stamp(),
        ResolverCancellation::default(),
        gated(entered, release_rx),
    );
    tokio::pin!(request);
    pending(request.as_mut()).await;
    entered_rx.await.unwrap();
    drop(first);
    pending(request.as_mut()).await;
    assert!(old_document.upgrade().is_some());
    release.send(()).unwrap();
    assert_eq!(request.await.err(), Some(ResolverError::Cancelled.into()));
    assert!(old_document.upgrade().is_none());
    let mut replacement = owner(cpu);
    replacement.scope = scope(2);
    let proposal = replacement
        .locate(event(), stamp(), ResolverCancellation::default())
        .await
        .unwrap();
    assert_eq!(
        proposal.validate(scope(1), proposal.binding()),
        Err(FuzzyAlignmentError::Obsolete)
    );
    proposal.validate(scope(2), proposal.binding()).unwrap();
}

#[tokio::test]
async fn delivered_proposals_keep_both_owner_and_caller_cancellation_links() {
    let owner = owner(Arc::new(Semaphore::new(1)));
    let external = ResolverCancellation::default();
    let proposal = owner
        .locate(event(), stamp(), external.clone())
        .await
        .unwrap();
    assert!(!external.is_cancelled());
    external.cancel();
    assert_eq!(
        proposal.validate(scope(1), proposal.binding()),
        Err(ResolverError::Cancelled.into())
    );
    let proposal = owner
        .locate(event(), stamp(), ResolverCancellation::default())
        .await
        .unwrap();
    drop(owner);
    assert_eq!(
        proposal.validate(scope(1), proposal.binding()),
        Err(ResolverError::Cancelled.into())
    );
}

#[tokio::test]
async fn panic_and_match_failure_release_capacity_without_poisoning_owner() {
    let cpu = Arc::new(Semaphore::new(1));
    let owner = owner(cpu.clone());
    let failed = owner
        .locate_using(event(), stamp(), ResolverCancellation::default(), |_| {
            panic!("synthetic worker panic")
        })
        .await;
    assert_eq!(failed.err(), Some(FuzzyAlignmentError::WorkerFailed));
    assert_eq!(cpu.available_permits(), 1);
    let mut too_long = event();
    too_long.text = "a".repeat(257).into();
    assert_eq!(
        owner
            .locate(too_long, stamp(), ResolverCancellation::default())
            .await
            .err(),
        Some(FuzzyAlignmentError::BudgetExceeded)
    );
    assert_eq!(cpu.available_permits(), 1);
    assert!(
        owner
            .locate(event(), stamp(), ResolverCancellation::default())
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn excessive_cancellation_ancestry_is_explicit_and_does_not_dispatch() {
    let cpu = Arc::new(Semaphore::new(1));
    let owner = owner(cpu.clone());
    let mut parent = ResolverCancellation::default();
    for _ in 0..3 {
        parent = ResolverCancellation::linked(parent).unwrap();
    }
    assert_eq!(
        owner.locate(event(), stamp(), parent.clone()).await.err(),
        Some(ResolverError::InvalidData.into())
    );
    assert_eq!(cpu.available_permits(), 1);
    assert!(!parent.is_cancelled());
}

#[test]
fn queued_job_keeps_original_deadline_and_releases_inputs_before_admission() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(async {
        let cpu = Arc::new(Semaphore::new(1));
        let mut owner = owner(cpu.clone());
        let document = weak(&owner);
        let (entered, entered_rx, release, release_rx) = gate();
        let blocker = tokio::task::spawn_blocking(move || {
            entered.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        });
        entered_rx.await.unwrap();
        let mut old = stamp();
        old.ended_at -= MAX_WINDOW_AGE - Duration::from_millis(400);
        let request = owner.locate_using(event(), old, ResolverCancellation::default(), |_| {
            panic!("expired queued work ran")
        });
        tokio::pin!(request);
        pending(request.as_mut()).await;
        assert_eq!(cpu.available_permits(), 0);
        // Leave the Future pending while actual wall time crosses the original
        // window deadline. A fresh timeout at worker start must not revive it.
        tokio::time::sleep(Duration::from_millis(450)).await;
        release.send(()).unwrap();
        blocker.await.unwrap();
        assert_eq!(request.await.err(), Some(ResolverError::Stale.into()));
        owner.stop();
        reclaimed(&cpu).await;
        assert!(document.upgrade().is_none());
    });
}

#[test]
fn queued_future_drop_cancels_without_running_and_restores_capacity() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(async {
        let cpu = Arc::new(Semaphore::new(1));
        let owner = owner(cpu.clone());
        let document = weak(&owner);
        let (entered, entered_rx, release, release_rx) = gate();
        let blocker = tokio::task::spawn_blocking(move || {
            entered.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        });
        entered_rx.await.unwrap();
        let mut request = Box::pin(owner.locate_using(
            event(),
            stamp(),
            ResolverCancellation::default(),
            |_| panic!("aborted queued work ran"),
        ));
        pending(request.as_mut()).await;
        assert_eq!(cpu.available_permits(), 0);
        drop(request);
        drop(owner);
        release.send(()).unwrap();
        blocker.await.unwrap();
        reclaimed(&cpu).await;
        assert!(document.upgrade().is_none());
    });
}

#[test]
fn request_deadline_includes_time_waiting_in_tokio_blocking_pool() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(async {
        let owner = owner(Arc::new(Semaphore::new(1)));
        let (entered, entered_rx, release, release_rx) = gate();
        let blocker = tokio::task::spawn_blocking(move || {
            entered.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        });
        entered_rx.await.unwrap();
        let (deadline_tx, deadline_rx) = oneshot::channel();
        let before_poll = Instant::now();
        let request = owner.locate_using(
            event(),
            stamp(),
            ResolverCancellation::default(),
            move |input| {
                deadline_tx.send(input.deadline).unwrap();
                run_match(input)
            },
        );
        tokio::pin!(request);
        pending(request.as_mut()).await;
        let after_poll = Instant::now();
        tokio::time::sleep(Duration::from_millis(30)).await;
        release.send(()).unwrap();
        blocker.await.unwrap();
        assert!(request.await.is_ok());
        let deadline = deadline_rx.await.unwrap();
        assert!(before_poll + FUZZY_MATCH_TIMEOUT <= deadline);
        assert!(deadline <= after_poll + FUZZY_MATCH_TIMEOUT);
    });
}

#[tokio::test]
async fn started_timeout_waits_for_actual_cleanup_and_reports_stale() {
    let cpu = Arc::new(Semaphore::new(1));
    let owner = owner(cpu.clone());
    let (entered, entered_rx, release, release_rx) = gate();
    let mut old = stamp();
    old.ended_at -= MAX_WINDOW_AGE - Duration::from_millis(400);
    let request = owner.locate_using(
        event(),
        old,
        ResolverCancellation::default(),
        gated(entered, release_rx),
    );
    tokio::pin!(request);
    pending(request.as_mut()).await;
    entered_rx.await.unwrap();
    tokio::time::sleep(Duration::from_millis(450)).await;
    pending(request.as_mut()).await;
    assert_eq!(cpu.available_permits(), 0);
    assert_eq!(owner.local.available_permits(), 0);
    release.send(()).unwrap();
    assert_eq!(request.await.err(), Some(ResolverError::Stale.into()));
    assert_eq!(cpu.available_permits(), 1);
    assert_eq!(owner.local.available_permits(), 1);
}
