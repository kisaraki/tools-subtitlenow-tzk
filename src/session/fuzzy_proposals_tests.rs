//! Synthetic inputs only. Isolated CPU capacity prevents parallel suite interference.
use super::*;
use crate::{
    ai::speech::{SpeechLanguage, StreamIdentity},
    audio::capture::SessionId,
    resolver::contracts::{ExistingText, SubtitleSource, TextCue, TextLanguage},
};
use std::{future::Future, task::Poll, time::Duration};
use tokio::sync::Semaphore;
#[path = "fuzzy_proposal_track_tests.rs"]
mod track_tests;

const LINE: &str = "the synthetic subtitle belongs to this document";
const OTHER: &str = "another distinct sentence describes the winter landscape";
fn identity(session: u64) -> StreamIdentity {
    StreamIdentity {
        session: SessionId(session),
        generation: 1,
    }
}
fn scope() -> AlignmentScope {
    AlignmentScope::new(identity(901), identity(902), 1).unwrap()
}
fn document() -> ExistingText {
    ExistingText {
        identity: identity(901),
        source: SubtitleSource::ExistingSubtitle,
        content_id: "synthetic-document".into(),
        language: TextLanguage::Source(SpeechLanguage::English),
        cues: [LINE, OTHER]
            .into_iter()
            .enumerate()
            .map(|(index, line)| TextCue {
                start_us: Some(600_000_000 + index as u64 * 3_000_000),
                duration_us: Some(2_000_000),
                text: line.into(),
            })
            .collect(),
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
fn fixture() -> (FuzzyProposalSession, FuzzyProposalTask, Arc<Semaphore>) {
    let cpu = Arc::new(Semaphore::new(1));
    let mut locator = FuzzyCueLocator::new(scope(), document()).unwrap();
    locator.isolate_workers(cpu.clone());
    let (session, task) = FuzzyProposalSession::new(locator).unwrap();
    (session, task, cpu)
}
async fn update(session: &mut FuzzyProposalSession) -> ProposalState {
    tokio::time::timeout(Duration::from_secs(2), session.changed())
        .await
        .unwrap()
}
async fn pending<F: Future>(future: std::pin::Pin<&mut F>) {
    let mut future = future;
    std::future::poll_fn(|cx| {
        assert!(future.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
}
fn replace_deadline(session: &mut FuzzyProposalSession, deadline: Instant) {
    let old = session.active.as_ref().unwrap();
    let request = Arc::new(Request {
        ticket: old.ticket,
        event: old.event.clone(),
        stamp: old.stamp,
        deadline,
        cancel: old.cancel.clone(),
    });
    session.active = Some(request.clone());
    session.input.send_replace(Some(request));
}

#[tokio::test]
async fn latest_revision_before_driver_poll_is_the_only_delivered_proposal() {
    let (mut session, task, cpu) = fixture();
    assert_eq!(session.state(), ProposalState::Waiting);
    let mut input = event();
    for revision in 1..=20 {
        input.revision = revision;
        input.text = if revision == 20 { OTHER } else { LINE }.into();
        assert_eq!(session.offer(input.clone(), stamp()), Ok(true));
    }
    assert_eq!(session.ticket, 20);
    let driver = tokio::spawn(task.run());
    assert_eq!(update(&mut session).await, ProposalState::Proposed);
    let proposal = session.proposal().unwrap();
    assert_eq!(proposal.binding(), TimelineBinding::from_event(&input));
    assert_eq!(proposal.cue_index(), 1);
    assert_eq!(proposal.media_range_us(), 603_000_000..605_000_000);
    assert_eq!(cpu.available_permits(), 1);
    session.stop();
    assert_eq!(driver.await.unwrap(), Ok(()));
    assert!(session.active.is_none() && session.accepted.is_none());
    assert!(session.input.borrow().is_none() && session.output.borrow().is_none());
}

#[test]
fn duplicate_window_never_refreshes_budget_or_observation_ticket() {
    let (mut session, _task, _) = fixture();
    session.offer(event(), stamp()).unwrap();
    let deadline = session.active.as_ref().unwrap().deadline;
    let mut newer = stamp();
    newer.sequence += 1;
    assert_eq!(session.offer(event(), newer), Ok(false));
    assert_eq!(session.active.as_ref().unwrap().deadline, deadline);
    assert_eq!(session.active.as_ref().unwrap().stamp.sequence, 8);
    assert_eq!(session.ticket, 1);
}

#[test]
fn same_binding_mutations_withdraw_even_invalid_payloads_and_require_new_revision() {
    for variant in 0..6 {
        let (mut session, _task, _) = fixture();
        session.offer(event(), stamp()).unwrap();
        let old = session.active.as_ref().unwrap().clone();
        let mut changed = event();
        match variant {
            0 => changed.text = OTHER.into(),
            1 => changed.language = Some(SpeechLanguage::Japanese),
            2 => changed.start_us = Some(42_000_001),
            3 => changed.duration_us = None,
            4 => changed.kind = TranscriptKind::Partial,
            _ => changed.text = "x".repeat(MAX_TRANSCRIPT_BYTES + 1).into(),
        }
        assert_eq!(
            session.offer(changed, stamp()),
            Err(ResolverError::InvalidData.into())
        );
        assert!(old.cancel.is_cancelled());
        assert!(session.input.borrow().is_none());
        assert_eq!(
            session.state(),
            ProposalState::Failed(ResolverError::InvalidData.into())
        );
        assert_eq!(session.offer(event(), stamp()), Ok(false));
        assert_eq!(
            session.state(),
            ProposalState::Failed(ResolverError::InvalidData.into())
        );
        let mut next = event();
        next.revision += 1;
        assert_eq!(session.offer(next, stamp()), Ok(true));
        assert_eq!(session.state(), ProposalState::Locating);
    }
}

#[test]
fn older_revision_sequence_or_utterance_cannot_cancel_latest_input() {
    let (mut session, _task, _) = fixture();
    let mut latest = event();
    latest.revision = 4;
    session.offer(latest.clone(), stamp()).unwrap();
    for variant in 0..3 {
        let mut old = latest.clone();
        match variant {
            0 => old.revision -= 1,
            1 => old.sequence -= 1,
            _ => old.utterance -= 1,
        }
        assert_eq!(
            session.offer(old, stamp()),
            Err(FuzzyAlignmentError::Obsolete)
        );
        assert!(!session.active.as_ref().unwrap().cancel.is_cancelled());
        assert_eq!(
            session.binding(),
            Some(TimelineBinding::from_event(&latest))
        );
    }
}

#[test]
fn new_utterance_resets_revision_but_requires_both_ordered_identifiers() {
    let (mut session, _task, _) = fixture();
    let mut previous = event();
    previous.revision = 10;
    session.offer(previous, stamp()).unwrap();
    for variant in 0..2 {
        let mut invalid = event();
        if variant == 0 {
            invalid.sequence += 1;
        } else {
            invalid.utterance += 1;
        }
        assert_eq!(
            session.offer(invalid, stamp()),
            Err(ResolverError::InvalidData.into())
        );
    }
    let mut next = event();
    next.utterance += 1;
    next.sequence += 1;
    assert_eq!(session.offer(next.clone(), stamp()), Ok(true));
    assert_eq!(session.binding(), Some(TimelineBinding::from_event(&next)));
}

#[test]
fn regressing_capture_windows_are_rejected_without_replacing_request() {
    let (mut session, _task, _) = fixture();
    let original = stamp();
    session.offer(event(), original).unwrap();
    let mut next = event();
    next.revision += 1;
    let mut old = original;
    old.sequence -= 1;
    assert_eq!(
        session.offer(next.clone(), old),
        Err(FuzzyAlignmentError::Obsolete)
    );
    old = original;
    old.ended_at -= Duration::from_millis(1);
    assert_eq!(session.offer(next, old), Err(FuzzyAlignmentError::Obsolete));
    assert_eq!(session.ticket, 1);
}

#[test]
fn admission_rejects_wrong_identity_partial_missing_language_and_invalid_ranges() {
    for variant in 0..13 {
        let (mut session, _task, _) = fixture();
        let mut input = event();
        let mut window = stamp();
        match variant {
            0 => input.identity.generation += 1,
            1 => input.kind = TranscriptKind::Partial,
            2 => input.language = None,
            3 => input.sequence = 0,
            4 => input.revision = 0,
            5 => input.utterance = 0,
            6 => window.sequence = 0,
            7 => input.sequence = window.sequence + 1,
            8 => input.start_us = None,
            9 => input.duration_us = Some(0),
            10 => input.start_us = Some(u64::MAX),
            11 => input.start_us = Some(39_000_000),
            _ => input.duration_us = Some(20_000_000),
        }
        assert!(session.offer(input, window).is_err());
        assert_eq!(session.state(), ProposalState::Waiting);
        assert!(session.active.is_none() && session.input.borrow().is_none());
    }
}

#[tokio::test]
async fn delivered_proposal_is_withdrawn_immediately_by_new_revision() {
    let (mut session, task, _) = fixture();
    session.offer(event(), stamp()).unwrap();
    let driver = tokio::spawn(task.run());
    assert_eq!(update(&mut session).await, ProposalState::Proposed);
    let old = session.accepted.as_ref().unwrap().clone();
    let mut next = event();
    next.revision += 1;
    next.text = OTHER.into();
    session.offer(next, stamp()).unwrap();
    assert!(session.proposal().is_none() && session.accepted.is_none());
    assert_eq!(
        old.result.as_ref().unwrap().validate(scope(), old.binding),
        Err(ResolverError::Cancelled.into())
    );
    assert_eq!(update(&mut session).await, ProposalState::Proposed);
    assert_eq!(session.proposal().unwrap().cue_index(), 1);
    session.stop();
    assert_eq!(driver.await.unwrap(), Ok(()));
}

#[tokio::test]
async fn obsolete_output_ticket_scope_binding_and_error_cannot_erase_current_proposal() {
    let (mut session, task, _) = fixture();
    session.offer(event(), stamp()).unwrap();
    let driver = tokio::spawn(task.run());
    assert_eq!(update(&mut session).await, ProposalState::Proposed);
    let current_ticket = session.ticket;
    let binding = session.binding().unwrap();
    for variant in 0..3 {
        let mut wrong_binding = binding;
        wrong_binding.revision += 1;
        let output = Completion {
            ticket: if variant == 0 {
                current_ticket - 1
            } else {
                current_ticket
            },
            scope: if variant == 1 {
                AlignmentScope::new(identity(901), identity(902), 2).unwrap()
            } else {
                scope()
            },
            binding: if variant == 2 { wrong_binding } else { binding },
            result: Err(FuzzyAlignmentError::WorkerFailed),
        };
        session.reset_output.send_replace(Some(Arc::new(output)));
        assert_eq!(session.poll(), ProposalState::Proposed);
        assert_eq!(session.proposal().unwrap().binding(), binding);
    }
    session.stop();
    assert_eq!(driver.await.unwrap(), Ok(()));
}

#[test]
fn mismatched_output_is_rejected_before_any_completion_is_accepted() {
    let (mut session, _task, _) = fixture();
    session.offer(event(), stamp()).unwrap();
    let binding = session.binding().unwrap();
    for variant in 0..3 {
        let mut wrong_binding = binding;
        wrong_binding.revision += 1;
        session.reset_output.send_replace(Some(Arc::new(Completion {
            ticket: if variant == 0 { 0 } else { session.ticket },
            scope: if variant == 1 {
                AlignmentScope::new(identity(901), identity(902), 2).unwrap()
            } else {
                scope()
            },
            binding: if variant == 2 { wrong_binding } else { binding },
            result: Err(FuzzyAlignmentError::WorkerFailed),
        })));
        assert_eq!(session.poll(), ProposalState::Locating);
        assert!(session.accepted.is_none());
        assert_eq!(session.seen, 0);
    }
}

#[tokio::test]
async fn unmatched_and_worker_overload_are_explicit_and_duplicates_do_not_retry() {
    for overloaded in [false, true] {
        let (mut session, task, cpu) = fixture();
        let permit = if overloaded {
            Some(cpu.acquire().await.unwrap())
        } else {
            None
        };
        let mut input = event();
        if !overloaded {
            input.text = "a completely unrelated conversation concerning astronomy".into();
        }
        session.offer(input.clone(), stamp()).unwrap();
        let driver = tokio::spawn(task.run());
        let expected = if overloaded {
            ProposalState::Failed(FuzzyAlignmentError::Overloaded)
        } else {
            ProposalState::Unmatched
        };
        assert_eq!(update(&mut session).await, expected);
        drop(permit);
        assert_eq!(session.offer(input, stamp()), Ok(false));
        assert_eq!(session.state(), expected);
        assert!(session.proposal().is_none());
        session.stop();
        assert_eq!(driver.await.unwrap(), Ok(()));
    }
}

#[tokio::test]
async fn offer_deadline_is_not_reset_when_driver_starts_later() {
    let (mut session, task, cpu) = fixture();
    session.offer(event(), stamp()).unwrap();
    replace_deadline(&mut session, Instant::now() - Duration::from_millis(1));
    let job = task.run();
    tokio::pin!(job);
    pending(job.as_mut()).await;
    assert_eq!(
        session
            .output
            .borrow()
            .as_ref()
            .unwrap()
            .result
            .as_ref()
            .err(),
        Some(&FuzzyAlignmentError::Invalid(ResolverError::Stale))
    );
    assert_eq!(
        update(&mut session).await,
        ProposalState::Failed(ResolverError::Stale.into())
    );
    assert_eq!(cpu.available_permits(), 1);
    assert!(session.input.borrow().is_none() && session.accepted.is_none());
    session.stop();
    assert_eq!(job.await, Ok(()));
}

#[tokio::test]
async fn changed_wait_is_cancellation_safe_and_driver_drop_wakes_idle_owner() {
    let (mut session, task, _) = fixture();
    {
        let wait = session.changed();
        tokio::pin!(wait);
        pending(wait.as_mut()).await;
    }
    assert_eq!(session.state(), ProposalState::Waiting);
    drop(task);
    assert_eq!(update(&mut session).await, ProposalState::Cancelled);
    assert_eq!(
        session.offer(event(), stamp()),
        Err(ResolverError::Cancelled.into())
    );
}

#[tokio::test]
async fn changed_wait_expires_without_output_and_cancels_pending_input() {
    let (mut session, _task, _) = fixture();
    session.offer(event(), stamp()).unwrap();
    replace_deadline(&mut session, Instant::now() + Duration::from_millis(20));
    let old = session.active.as_ref().unwrap().clone();
    assert_eq!(
        update(&mut session).await,
        ProposalState::Failed(ResolverError::Stale.into())
    );
    assert!(old.cancel.is_cancelled());
    assert!(session.input.borrow().is_none() && session.proposal().is_none());
    assert_eq!(session.offer(event(), stamp()), Ok(false));
}

#[test]
fn old_capture_window_limits_offer_budget_and_invalid_windows_do_not_enter_queue() {
    let (mut session, _task, _) = fixture();
    let mut window = stamp();
    window.ended_at -= Duration::from_secs(4);
    session.offer(event(), window).unwrap();
    assert_eq!(
        session.active.as_ref().unwrap().deadline,
        window.ended_at + MAX_WINDOW_AGE
    );
    for variant in 0..4 {
        let (mut owner, _task, _) = fixture();
        let mut window = stamp();
        match variant {
            0 => window.ended_at -= MAX_WINDOW_AGE + Duration::from_secs(1),
            1 => window.ended_at += Duration::from_secs(1),
            2 => window.identity.generation += 1,
            _ => window.start_us = None,
        }
        assert!(owner.offer(event(), window).is_err());
        assert_eq!(owner.state(), ProposalState::Waiting);
    }
}

#[tokio::test]
async fn driver_abort_revokes_delivered_proposal_and_clears_owner_on_poll() {
    let (mut session, task, _) = fixture();
    session.offer(event(), stamp()).unwrap();
    let driver = tokio::spawn(task.run());
    assert_eq!(update(&mut session).await, ProposalState::Proposed);
    driver.abort();
    assert!(driver.await.unwrap_err().is_cancelled());
    assert_eq!(update(&mut session).await, ProposalState::Cancelled);
    assert!(session.proposal().is_none() && session.active.is_none());
    assert!(session.input.borrow().is_none() && session.output.borrow().is_none());
}

#[tokio::test]
async fn owner_drop_stops_idle_driver_without_a_hidden_task() {
    let (session, task, _) = fixture();
    let driver = tokio::spawn(task.run());
    drop(session);
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), driver)
            .await
            .unwrap()
            .unwrap(),
        Ok(())
    );
}

#[test]
fn exhausted_ticket_stops_and_releases_inputs() {
    let (mut session, _task, _) = fixture();
    session.offer(event(), stamp()).unwrap();
    session.ticket = u64::MAX;
    let mut next = event();
    next.revision += 1;
    assert_eq!(
        session.offer(next, stamp()),
        Err(ResolverError::InvalidData.into())
    );
    assert_eq!(session.state(), ProposalState::Stopped);
    assert!(session.active.is_none() && session.input.borrow().is_none());
    assert_eq!(
        session.offer(event(), stamp()),
        Err(ResolverError::Cancelled.into())
    );
}

#[test]
fn queued_job_replacements_coalesce_and_cleanup_before_latest_job() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let (entered, entered_rx) = tokio::sync::oneshot::channel();
        let (release, release_rx) = std::sync::mpsc::sync_channel(1);
        let blocker = tokio::task::spawn_blocking(move || {
            entered.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        });
        entered_rx.await.unwrap();
        let (mut session, task, cpu) = fixture();
        session.offer(event(), stamp()).unwrap();
        let job = task.run();
        tokio::pin!(job);
        pending(job.as_mut()).await;
        assert_eq!(cpu.available_permits(), 0);
        let previous = session.active.as_ref().unwrap().clone();
        let mut latest = event();
        for revision in 2..=30 {
            latest.revision = revision;
            latest.text = OTHER.into();
            session.offer(latest.clone(), stamp()).unwrap();
        }
        assert!(previous.cancel.is_cancelled());
        pending(job.as_mut()).await;
        // Old queued closure is cancelled and joined before replacement admission.
        assert_eq!(cpu.available_permits(), 0);
        release.send(()).unwrap();
        blocker.await.unwrap();
        tokio::select! {
            state = session.changed() => assert_eq!(state, ProposalState::Proposed),
            result = &mut job => panic!("driver exited early: {result:?}"),
        }
        assert_eq!(
            session.proposal().unwrap().binding(),
            TimelineBinding::from_event(&latest)
        );
        assert_eq!(session.proposal().unwrap().cue_index(), 1);
        assert_eq!(cpu.available_permits(), 1);
        session.stop();
        assert_eq!(job.await, Ok(()));
    });
}
