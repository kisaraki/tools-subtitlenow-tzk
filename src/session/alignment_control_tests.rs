//! Actual RAM lookup publications; gates below simulate lifecycle interruptions
//! only, never successful subtitle matching or a production worker.
use super::*;
use crate::{audio::capture::SessionId, subtitle::fuzzy_worker::lookup_tests::Publication};
use futures_util::FutureExt;
use std::{sync::Arc, time::Duration};
use tokio::sync::Semaphore;

fn controller(publication: &Publication) -> AlignmentController {
    let mut owner =
        AlignmentController::from_lookup(publication.lease(), publication.scope.capture_identity())
            .unwrap();
    owner
        .source
        .as_mut()
        .unwrap()
        .isolate(Arc::new(Semaphore::new(1)));
    owner
}
async fn changed(owner: &mut AlignmentController) -> ProposalState {
    tokio::time::timeout(Duration::from_secs(2), owner.changed())
        .await
        .unwrap()
        .unwrap()
}
async fn seed(publication: &Publication, owner: &mut AlignmentController) -> FuzzyTrackProposal {
    for index in 0..3 {
        let mut event = publication.event.clone();
        event.identity = owner.scope().unwrap().capture_identity();
        event.utterance = index + 1;
        event.sequence = index + 1;
        event.start_us = Some(42_000_000 + index * 1_000_000);
        event.text = char::from(b'a' + index as u8).to_string().repeat(32).into();
        let mut stamp = publication.stamp;
        stamp.identity = event.identity;
        owner.offer(event, stamp).unwrap();
        assert_eq!(changed(owner).await, ProposalState::Proposed);
    }
    assert_eq!(owner.track_status(), Some(FuzzyTrackStatus::Consistent(3)));
    owner.track_proposal().unwrap()
}

#[tokio::test]
async fn alignment_control_is_passive_and_resume_is_idempotent_until_pause() {
    let publication = Publication::new(false, Duration::ZERO).await;
    let mut owner = controller(&publication);
    assert_eq!(owner.state(), AlignmentControlState::Paused);
    assert!(owner.active.is_none() && owner.scope().is_none());
    assert_eq!(
        owner.offer(publication.event.clone(), publication.stamp),
        Err(AlignmentControlError::Paused)
    );
    owner.pause().await.unwrap();
    let first = owner.resume().await.unwrap();
    assert_eq!(owner.resume().await.unwrap(), first);
    assert_eq!(owner.state(), AlignmentControlState::Running);
    owner.pause().await.unwrap();
    assert!(owner.active.is_none());
    let second = owner.resume().await.unwrap();
    assert_ne!(first.epoch(), second.epoch());
    assert_eq!(first.document_identity(), second.document_identity());
    owner.stop().await.unwrap();
    assert_eq!(publication.calls(), 1);
    assert!(!publication.parent_cancelled());
    publication.close().await;
}

#[tokio::test]
async fn alignment_control_pause_withdraws_tracks_and_resume_recollects_for_both_lookup_kinds() {
    for paired in [false, true] {
        let publication = Publication::new(paired, Duration::ZERO).await;
        let mut owner = controller(&publication);
        let first = owner.resume().await.unwrap();
        let old = seed(&publication, &mut owner).await;
        old.validate(first).unwrap();
        owner.pause().await.unwrap();
        assert!(old.validate(first).is_err());
        assert!(owner.proposal().is_none() && owner.track_proposal().is_none());
        let second = owner.resume().await.unwrap();
        assert_ne!(first, second);
        assert_eq!(owner.track_status(), Some(FuzzyTrackStatus::Collecting(0)));
        let fresh = seed(&publication, &mut owner).await;
        fresh.validate(second).unwrap();
        assert!(old.validate(second).is_err());
        owner.stop().await.unwrap();
        assert!(fresh.validate(second).is_err());
        assert_eq!(publication.calls(), 1);
        publication.close().await;
    }
}

#[tokio::test]
async fn alignment_control_reset_rebinds_capture_and_never_reuses_an_epoch() {
    let publication = Publication::new(true, Duration::ZERO).await;
    let mut owner = controller(&publication);
    let first = owner.resume().await.unwrap();
    let old = seed(&publication, &mut owner).await;
    let next_capture = StreamIdentity {
        session: SessionId(9977),
        generation: 8,
    };
    let second = owner.reset(next_capture).await.unwrap();
    assert_ne!(first.epoch(), second.epoch());
    assert_eq!(second.document_identity(), first.document_identity());
    assert_eq!(second.capture_identity(), next_capture);
    assert!(old.validate(first).is_err());
    assert!(
        owner
            .offer(publication.event.clone(), publication.stamp)
            .is_err()
    );
    let fresh = seed(&publication, &mut owner).await;
    fresh.validate(second).unwrap();
    let third = owner.reset(next_capture).await.unwrap();
    assert_ne!(third.epoch(), second.epoch());
    assert!(fresh.validate(second).is_err());
    owner.stop().await.unwrap();
    publication.close().await;
}

#[tokio::test]
async fn alignment_control_invalid_reset_preserves_the_active_track() {
    let publication = Publication::new(false, Duration::ZERO).await;
    let mut owner = controller(&publication);
    let scope = owner.resume().await.unwrap();
    let track = seed(&publication, &mut owner).await;
    for capture in [
        StreamIdentity {
            session: SessionId(0),
            generation: 1,
        },
        StreamIdentity {
            session: SessionId(9),
            generation: 0,
        },
    ] {
        assert_eq!(
            owner.reset(capture).await,
            Err(FuzzyAlignmentError::Invalid(ResolverError::InvalidData).into())
        );
        assert_eq!(owner.scope(), Some(scope));
        track.validate(scope).unwrap();
    }
    owner.stop().await.unwrap();
    publication.close().await;
}

#[tokio::test]
async fn alignment_control_controllers_have_distinct_epochs_and_independent_cancellation() {
    let publication = Publication::new(false, Duration::ZERO).await;
    let mut first = controller(&publication);
    let mut second = controller(&publication);
    let first_scope = first.resume().await.unwrap();
    let second_scope = second.resume().await.unwrap();
    assert_ne!(first_scope.epoch(), second_scope.epoch());
    let retained = seed(&publication, &mut second).await;
    first.stop().await.unwrap();
    retained.validate(second_scope).unwrap();
    assert_eq!(second.state(), AlignmentControlState::Running);
    assert!(!publication.parent_cancelled());
    second.stop().await.unwrap();
    publication.close().await;
}

#[tokio::test]
async fn alignment_control_stop_is_terminal_and_idempotent() {
    let publication = Publication::new(false, Duration::ZERO).await;
    let mut owner = controller(&publication);
    owner.resume().await.unwrap();
    owner.stop().await.unwrap();
    owner.stop().await.unwrap();
    assert_eq!(owner.state(), AlignmentControlState::Stopped);
    assert!(owner.source.is_none() && owner.active.is_none());
    assert_eq!(owner.resume().await, Err(AlignmentControlError::Stopped));
    assert_eq!(owner.pause().await, Err(AlignmentControlError::Stopped));
    assert_eq!(
        owner.reset(publication.scope.capture_identity()).await,
        Err(AlignmentControlError::Stopped)
    );
    assert_eq!(owner.poll(), Err(AlignmentControlError::Stopped));
    publication.close().await;
}

#[tokio::test]
async fn alignment_control_paused_binding_cannot_keep_lookup_text_alive_or_resume_it() {
    let mut publication = Publication::new(true, Duration::ZERO).await;
    let mut owner = controller(&publication);
    publication.release();
    assert!(publication.text_released());
    assert_eq!(
        owner.state(),
        AlignmentControlState::Unavailable(ResolverError::Cancelled.into())
    );
    assert_eq!(
        owner.resume().await,
        Err(FuzzyAlignmentError::Invalid(ResolverError::Cancelled).into())
    );
    assert!(owner.active.is_none());
    owner.stop().await.unwrap();
    publication.close().await;
}

#[tokio::test]
async fn alignment_control_lease_retirement_withdraws_active_output_without_renewal() {
    let publication = Publication::new(true, Duration::ZERO).await;
    let mut owner = controller(&publication);
    let scope = owner.resume().await.unwrap();
    let track = seed(&publication, &mut owner).await;
    publication.lease().retire();
    assert!(track.validate(scope).is_err());
    assert_eq!(changed(&mut owner).await, ProposalState::Cancelled);
    assert!(owner.scope().is_none() && owner.proposal().is_none());
    assert_eq!(
        owner.resume().await,
        Err(FuzzyAlignmentError::Invalid(ResolverError::Cancelled).into())
    );
    assert_eq!(publication.calls(), 1);
    owner.stop().await.unwrap();
    publication.close().await;
}

#[tokio::test]
async fn alignment_control_pause_does_not_refresh_the_original_lookup_deadline() {
    let publication = Publication::new(false, Duration::from_millis(4500)).await;
    let mut owner = controller(&publication);
    owner.resume().await.unwrap();
    owner.pause().await.unwrap();
    tokio::time::sleep(Duration::from_millis(550)).await;
    assert_eq!(
        owner.state(),
        AlignmentControlState::Unavailable(ResolverError::Stale.into())
    );
    assert_eq!(
        owner.resume().await,
        Err(FuzzyAlignmentError::Invalid(ResolverError::Stale).into())
    );
    assert_eq!(publication.calls(), 1);
    owner.stop().await.unwrap();
    publication.close().await;
}

#[test]
fn alignment_control_epoch_exhaustion_is_checked_without_wraparound() {
    let counter = AtomicU64::new(u64::MAX - 1);
    assert_eq!(allocate_epoch(&counter), Ok(u64::MAX - 1));
    assert_eq!(
        allocate_epoch(&counter),
        Err(AlignmentControlError::EpochExhausted)
    );
    assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
}

#[test]
fn alignment_control_requires_an_explicit_runtime_only_on_resume() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (mut owner, lease) = runtime.block_on(async {
        let publication = Publication::new(false, Duration::ZERO).await;
        let owner = controller(&publication);
        let lease = publication.lease().clone();
        publication.close().await;
        (owner, lease)
    });
    drop(runtime);
    lease.validate(lease.query().stamp.identity).unwrap();
    assert_eq!(owner.state(), AlignmentControlState::Paused);
    assert_eq!(
        owner.resume().now_or_never(),
        Some(Err(FuzzyAlignmentError::RuntimeUnavailable.into()))
    );
    assert!(owner.active.is_none());
}

#[tokio::test]
async fn alignment_control_cancelled_pause_caches_join_and_waits_for_input_cleanup() {
    let publication = Publication::new(false, Duration::ZERO).await;
    let mut owner = controller(&publication);
    let first = owner.resume().await.unwrap();
    // Simulate a still-cleaning admitted input through the actual local barrier.
    // The normal idle driver joins; dropping pause at this wait must retain it.
    let local = owner.source.as_ref().unwrap().local();
    let permit = local.clone().acquire_owned().await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            assert!(owner.pause().now_or_never().is_none());
            if owner.active.as_ref().unwrap().joined.is_some() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(owner.scope().is_none());
    assert!(owner.resume().now_or_never().is_none());
    drop(permit);
    let second = owner.resume().await.unwrap();
    assert_ne!(first.epoch(), second.epoch());
    assert_eq!(local.available_permits(), 1);
    owner.stop().await.unwrap();
    publication.close().await;
}

#[tokio::test]
async fn alignment_control_cancelled_stop_remains_terminal_until_cleanup_is_rejoined() {
    let publication = Publication::new(false, Duration::ZERO).await;
    let mut owner = controller(&publication);
    owner.resume().await.unwrap();
    let permit = owner
        .source
        .as_ref()
        .unwrap()
        .local()
        .acquire_owned()
        .await
        .unwrap();
    assert!(owner.stop().now_or_never().is_none());
    assert_eq!(owner.state(), AlignmentControlState::Stopped);
    assert!(owner.source.is_some() && owner.active.is_some());
    assert!(owner.active.as_ref().unwrap().joined.is_none());
    assert_eq!(owner.resume().await, Err(AlignmentControlError::Stopped));
    drop(permit);
    owner.stop().await.unwrap();
    assert!(owner.source.is_none() && owner.active.is_none());
    publication.close().await;
}

#[tokio::test]
async fn alignment_control_cancelled_reset_keeps_old_capture_paused_until_explicit_resume() {
    let publication = Publication::new(false, Duration::ZERO).await;
    let mut owner = controller(&publication);
    let first = owner.resume().await.unwrap();
    let next = StreamIdentity {
        session: SessionId(9978),
        generation: 9,
    };
    // The current-thread driver has not been polled. This drops reset during
    // its join wait, before either capture rebinding or new task creation.
    assert!(owner.reset(next).now_or_never().is_none());
    assert!(owner.active.as_ref().unwrap().joined.is_none());
    assert_eq!(owner.capture, first.capture_identity());
    assert_eq!(owner.state(), AlignmentControlState::Paused);
    assert_eq!(
        owner.offer(publication.event.clone(), publication.stamp),
        Err(AlignmentControlError::Paused)
    );
    let resumed = owner.resume().await.unwrap();
    assert_ne!(resumed.epoch(), first.epoch());
    assert_eq!(resumed.capture_identity(), first.capture_identity());
    let reset = owner.reset(next).await.unwrap();
    assert_ne!(reset.epoch(), resumed.epoch());
    assert_eq!(reset.capture_identity(), next);
    owner.stop().await.unwrap();
    publication.close().await;
}

#[tokio::test]
async fn alignment_control_aborted_driver_reports_error_then_allows_explicit_recovery() {
    let publication = Publication::new(false, Duration::ZERO).await;
    let mut owner = controller(&publication);
    let first = owner.resume().await.unwrap();
    let local = owner.source.as_ref().unwrap().local();
    let permit = local.clone().acquire_owned().await.unwrap();
    owner.active.as_ref().unwrap().worker.abort();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            assert!(owner.pause().now_or_never().is_none());
            if let Some(result) = owner.active.as_ref().unwrap().joined {
                assert_eq!(result, Err(AlignmentControlError::WorkerFailed));
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    // Even a join error cannot bypass actual input cleanup or start a new run.
    assert!(owner.resume().now_or_never().is_none());
    drop(permit);
    assert_eq!(
        owner.pause().await,
        Err(AlignmentControlError::WorkerFailed)
    );
    assert!(owner.active.is_none());
    assert_eq!(
        owner.source.as_ref().unwrap().local().available_permits(),
        1
    );
    let second = owner.resume().await.unwrap();
    assert_ne!(first.epoch(), second.epoch());
    seed(&publication, &mut owner)
        .await
        .validate(second)
        .unwrap();
    owner.stop().await.unwrap();
    publication.close().await;
}

#[tokio::test]
async fn alignment_control_drop_immediately_revokes_delivered_tracks_without_cancelling_source() {
    let publication = Publication::new(false, Duration::ZERO).await;
    let mut owner = controller(&publication);
    let scope = owner.resume().await.unwrap();
    let track = seed(&publication, &mut owner).await;
    drop(owner);
    assert!(track.validate(scope).is_err());
    assert!(!publication.parent_cancelled());
    publication
        .lease()
        .validate(scope.document_identity())
        .unwrap();
    publication.close().await;
}
