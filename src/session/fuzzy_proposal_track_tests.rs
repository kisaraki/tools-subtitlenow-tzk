//! Owner/worker integration for metadata-only multi-observation hints.
use super::*;
use crate::subtitle::{
    fuzzy_alignment::FuzzyTextOrigin,
    fuzzy_track::{FuzzyTrackStatus, tests::Fixture as TrackFixture},
};

fn fixture_track() -> (
    TrackFixture,
    FuzzyProposalSession,
    FuzzyProposalTask,
    Arc<Semaphore>,
) {
    let data = TrackFixture::default();
    let cpu = Arc::new(Semaphore::new(1));
    let mut locator = data.locator();
    locator.isolate_workers(cpu.clone());
    let (owner, task) = FuzzyProposalSession::new(locator).unwrap();
    (data, owner, task, cpu)
}
async fn seed(data: &TrackFixture, owner: &mut FuzzyProposalSession) {
    for index in 0..3 {
        owner.offer(data.event(index), data.stamp(index)).unwrap();
        assert_eq!(update(owner).await, ProposalState::Proposed);
    }
    assert_eq!(owner.track_status(), FuzzyTrackStatus::Consistent(3));
}

#[tokio::test]
async fn fuzzy_adjacent_pairs_flow_through_real_worker_and_latest_manager() {
    let data = TrackFixture {
        step: 1_000_000,
        ..TrackFixture::default()
    };
    let mut locator = data.locator();
    let cpu = Arc::new(Semaphore::new(1));
    locator.isolate_workers(cpu.clone());
    let (mut owner, task) = FuzzyProposalSession::new(locator).unwrap();
    let driver = tokio::spawn(task.run());
    for index in [0, 2, 4] {
        let mut input = data.event(index);
        input.duration_us = Some(2_000_000);
        input.text = format!("{} {}", input.text, data.event(index + 1).text).into();
        owner.offer(input, data.stamp(index)).unwrap();
        assert_eq!(update(&mut owner).await, ProposalState::Proposed);
        assert_eq!(owner.proposal().unwrap().cue_range(), index..index + 2);
    }
    let track = owner.track_proposal().unwrap();
    assert_eq!(track.observations(), 3);
    track.validate(owner.scope()).unwrap();
    assert_eq!(cpu.available_permits(), 1);
    owner.stop();
    assert_eq!(
        track.validate(owner.scope()),
        Err(FuzzyAlignmentError::Obsolete)
    );
    assert_eq!(driver.await.unwrap(), Ok(()));
}

#[tokio::test]
async fn fuzzy_track_keeps_independent_final_metadata_while_old_jobs_are_revoked() {
    let (data, mut owner, task, cpu) = fixture_track();
    let driver = tokio::spawn(task.run());
    owner.offer(data.event(0), data.stamp(0)).unwrap();
    assert_eq!(update(&mut owner).await, ProposalState::Proposed);
    let old_lexical = owner.accepted.as_ref().unwrap().clone();
    for index in 1..3 {
        owner.offer(data.event(index), data.stamp(index)).unwrap();
        assert_eq!(update(&mut owner).await, ProposalState::Proposed);
    }
    assert!(
        old_lexical
            .result
            .as_ref()
            .unwrap()
            .validate(owner.scope(), old_lexical.binding)
            .is_err()
    );
    let track = owner.track_proposal().unwrap();
    assert_eq!(track.observations(), 3);
    assert_eq!(track.origin(), FuzzyTextOrigin::ExistingText);
    track.validate(owner.scope()).unwrap();
    assert_eq!(cpu.available_permits(), 1);
    owner.stop();
    assert_eq!(
        track.validate(owner.scope()),
        Err(FuzzyAlignmentError::Obsolete)
    );
    assert_eq!(owner.track_status(), FuzzyTrackStatus::Stopped);
    assert_eq!(driver.await.unwrap(), Ok(()));
}

#[tokio::test]
async fn fuzzy_track_duplicate_current_input_preserves_snapshot_but_old_replay_withdraws_support() {
    let (data, mut owner, task, _) = fixture_track();
    let driver = tokio::spawn(task.run());
    seed(&data, &mut owner).await;
    let track = owner.track_proposal().unwrap();
    assert_eq!(owner.offer(data.event(2), data.stamp(2)), Ok(false));
    track.validate(owner.scope()).unwrap();
    let binding = owner.binding();
    assert_eq!(
        owner.offer(data.event(0), data.stamp(0)),
        Err(FuzzyAlignmentError::Obsolete)
    );
    assert_eq!(owner.binding(), binding);
    assert_eq!(owner.state(), ProposalState::Proposed);
    assert_eq!(owner.track_status(), FuzzyTrackStatus::Collecting(2));
    assert_eq!(
        track.validate(owner.scope()),
        Err(FuzzyAlignmentError::Obsolete)
    );
    owner.stop();
    assert_eq!(driver.await.unwrap(), Ok(()));
}

#[tokio::test]
async fn fuzzy_track_new_revision_withdraws_before_work_and_does_not_count_twice() {
    let (data, mut owner, task, _) = fixture_track();
    let driver = tokio::spawn(task.run());
    seed(&data, &mut owner).await;
    let old = owner.track_proposal().unwrap();
    let mut event = data.event(2);
    event.revision += 1;
    owner.offer(event, data.stamp(2)).unwrap();
    assert_eq!(owner.track_status(), FuzzyTrackStatus::Collecting(2));
    assert_eq!(
        old.validate(owner.scope()),
        Err(FuzzyAlignmentError::Obsolete)
    );
    assert_eq!(update(&mut owner).await, ProposalState::Proposed);
    assert_eq!(owner.track_proposal().unwrap().observations(), 3);
    let old = owner.track_proposal().unwrap();
    let mut mutation = data.event(2);
    mutation.revision = 2;
    mutation.text = "different source under an unchanged binding".into();
    assert_eq!(
        owner.offer(mutation, data.stamp(2)),
        Err(ResolverError::InvalidData.into())
    );
    assert!(owner.track_proposal().is_none());
    assert_eq!(
        old.validate(owner.scope()),
        Err(FuzzyAlignmentError::Obsolete)
    );
    owner.stop();
    assert_eq!(driver.await.unwrap(), Ok(()));
}

#[tokio::test]
async fn fuzzy_track_current_unmatched_or_overload_retracts_delivered_track() {
    for overload in [false, true] {
        let (data, mut owner, task, cpu) = fixture_track();
        let driver = tokio::spawn(task.run());
        seed(&data, &mut owner).await;
        let old = owner.track_proposal().unwrap();
        let permit = if overload {
            Some(cpu.clone().acquire_owned().await.unwrap())
        } else {
            None
        };
        let mut event = data.event(3);
        if !overload {
            event.text = "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx".into();
        }
        owner.offer(event, data.stamp(3)).unwrap();
        let expected = if overload {
            ProposalState::Failed(FuzzyAlignmentError::Overloaded)
        } else {
            ProposalState::Unmatched
        };
        assert_eq!(update(&mut owner).await, expected);
        assert!(owner.track_proposal().is_none());
        assert_eq!(
            old.validate(owner.scope()),
            Err(FuzzyAlignmentError::Obsolete)
        );
        drop(permit);
        owner.stop();
        assert_eq!(driver.await.unwrap(), Ok(()));
    }
}

#[tokio::test]
async fn fuzzy_track_pending_deadline_and_driver_abort_retract_snapshots() {
    let (data, mut owner, task, _) = fixture_track();
    let driver = tokio::spawn(task.run());
    seed(&data, &mut owner).await;
    let old = owner.track_proposal().unwrap();
    owner.offer(data.event(3), data.stamp(3)).unwrap();
    replace_deadline(&mut owner, Instant::now() - Duration::from_millis(1));
    assert_eq!(
        owner.poll(),
        ProposalState::Failed(ResolverError::Stale.into())
    );
    assert_eq!(
        old.validate(owner.scope()),
        Err(FuzzyAlignmentError::Obsolete)
    );
    driver.abort();
    assert!(driver.await.unwrap_err().is_cancelled());
    assert_eq!(owner.track_status(), FuzzyTrackStatus::Cancelled);
}

#[tokio::test]
async fn fuzzy_track_temporal_conflict_restarts_without_reclassifying_lexical_match() {
    let (data, mut owner, task, _) = fixture_track();
    let driver = tokio::spawn(task.run());
    seed(&data, &mut owner).await;
    let old = owner.track_proposal().unwrap();
    let mut event = data.event(3);
    event.start_us = event.start_us.map(|start| start + 500_000);
    owner.offer(event, data.stamp(3)).unwrap();
    assert_eq!(update(&mut owner).await, ProposalState::Proposed);
    assert_eq!(owner.track_status(), FuzzyTrackStatus::Restarted(1));
    assert!(owner.track_proposal().is_none());
    assert_eq!(
        old.validate(owner.scope()),
        Err(FuzzyAlignmentError::Obsolete)
    );
    owner.stop();
    assert_eq!(driver.await.unwrap(), Ok(()));
}

#[tokio::test]
async fn fuzzy_track_changed_wakes_when_older_support_expires_before_latest_proposal() {
    let (data, mut owner, task, _) = fixture_track();
    let driver = tokio::spawn(task.run());
    // The production getter uses real capture-window deadlines; only this
    // private fixture shortens one observation to exercise the timer quickly.
    seed(&data, &mut owner).await;
    owner
        .track
        .shorten_oldest_for_test(Instant::now() + Duration::from_millis(20));
    let old = owner.track_proposal().unwrap();
    assert_eq!(update(&mut owner).await, ProposalState::Proposed);
    assert_eq!(owner.track_status(), FuzzyTrackStatus::Collecting(2));
    assert!(old.validate(owner.scope()).is_err());
    assert!(owner.proposal().is_some());
    owner.stop();
    assert_eq!(driver.await.unwrap(), Ok(()));
}
