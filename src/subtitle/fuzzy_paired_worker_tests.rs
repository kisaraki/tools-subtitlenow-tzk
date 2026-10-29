//! Real worker/owner behavior with explicit synthetic paired publications.
use super::*;
use crate::{
    session::fuzzy_proposals::{FuzzyProposalSession, ProposalState},
    subtitle::{
        fuzzy_alignment::{FuzzyTextOrigin, paired::tests::Fixture},
        fuzzy_track::FuzzyTrackStatus,
    },
};
use std::time::Duration;

#[tokio::test]
async fn fuzzy_paired_worker_borrows_both_allocations_and_stop_revokes_without_retaining_bundle() {
    let data = Fixture::new();
    let primary = data.bundle.text.cues[2].text.as_ptr() as usize;
    let reference = data.source().cues[1].text.as_ptr() as usize;
    let mut owner = FuzzyCueLocator::new_paired(data.scope, data.bundle).unwrap();
    owner.isolate_workers(Arc::new(Semaphore::new(1)));
    let weak = match owner.document.as_ref().unwrap() {
        AlignmentDocument::Paired(bundle) => Arc::downgrade(bundle),
        _ => panic!("expected paired publication"),
    };
    let proposal = owner
        .locate_using(data.event, data.stamp, data.cancel, move |input| {
            let AlignmentInput::Paired(bundle) = &input.document else {
                panic!("expected paired worker")
            };
            assert_eq!(bundle.text.cues[2].text.as_ptr() as usize, primary);
            assert_eq!(
                bundle.source_reference.as_ref().unwrap().text.cues[1]
                    .text
                    .as_ptr() as usize,
                reference
            );
            run_match(input)
        })
        .await
        .unwrap();
    assert_eq!(proposal.paired_primary_cue_range(), Some(2..3));
    owner.stop();
    assert!(weak.upgrade().is_none());
    assert_eq!(
        proposal.validate(data.scope, proposal.binding()),
        Err(ResolverError::Cancelled.into())
    );
}

#[tokio::test]
async fn fuzzy_paired_latest_manager_accumulates_distinct_primary_and_reference_observations() {
    let mut data = Fixture::new();
    let lines: Vec<_> = data
        .source()
        .cues
        .iter()
        .map(|cue| cue.text.clone())
        .collect();
    let mut locator = FuzzyCueLocator::new_paired(data.scope, data.bundle).unwrap();
    let cpu = Arc::new(Semaphore::new(1));
    locator.isolate_workers(cpu.clone());
    let (mut owner, task) = FuzzyProposalSession::new(locator).unwrap();
    let driver = tokio::spawn(task.run());
    for (index, line) in lines.iter().enumerate() {
        data.event.text = line.clone();
        data.event.utterance = index as u64 + 1;
        data.event.sequence = index as u64 + 1;
        data.event.start_us = Some(42_000_000 + index as u64 * 1_000_000);
        data.stamp.ended_at = Instant::now();
        owner.offer(data.event.clone(), data.stamp).unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), owner.changed())
                .await
                .unwrap(),
            ProposalState::Proposed
        );
        assert_eq!(owner.proposal().unwrap().cue_range(), index..index + 1);
        assert_eq!(
            owner.proposal().unwrap().paired_primary_cue_range(),
            Some(index + 1..index + 2)
        );
    }
    assert_eq!(owner.track_status(), FuzzyTrackStatus::Consistent(3));
    let track = owner.track_proposal().unwrap();
    assert_eq!(track.origin(), FuzzyTextOrigin::PairedSourceReference);
    assert_eq!(cpu.available_permits(), 1);
    owner.stop();
    assert_eq!(
        track.validate(data.scope),
        Err(FuzzyAlignmentError::Obsolete)
    );
    assert_eq!(driver.await.unwrap(), Ok(()));
}

#[tokio::test]
async fn fuzzy_paired_worker_enforces_shared_capacity_and_typed_mapping_failure() {
    let data = Fixture::new();
    let mut owner = FuzzyCueLocator::new_paired(data.scope, data.bundle).unwrap();
    let cpu = Arc::new(Semaphore::new(1));
    owner.isolate_workers(cpu.clone());
    let permit = cpu.clone().acquire_owned().await.unwrap();
    assert_eq!(
        owner
            .locate(data.event.clone(), data.stamp, data.cancel.clone())
            .await
            .err(),
        Some(FuzzyAlignmentError::Overloaded)
    );
    drop(permit);
    assert!(
        owner
            .locate(data.event, data.stamp, data.cancel)
            .await
            .is_ok()
    );
    let mut data = Fixture::new();
    data.bundle.text.cues[2].start_us = Some(601_000_001);
    let mut owner = FuzzyCueLocator::new_paired(data.scope, data.bundle).unwrap();
    owner.isolate_workers(cpu.clone());
    assert_eq!(
        owner
            .locate(data.event, data.stamp, data.cancel)
            .await
            .err(),
        Some(FuzzyAlignmentError::TimingMismatch)
    );
    assert_eq!(cpu.available_permits(), 1);
}

#[tokio::test]
async fn fuzzy_paired_owner_drop_cancels_unpolled_work_and_does_not_cancel_parent() {
    let mut data = Fixture::new();
    data.pair();
    let parent = data.cancel.clone();
    let owner = FuzzyCueLocator::new_paired(data.scope, data.bundle).unwrap();
    let future = owner.locate(data.event, data.stamp, data.cancel);
    drop(owner);
    assert_eq!(future.await.err(), Some(ResolverError::Cancelled.into()));
    assert!(!parent.is_cancelled());
}
