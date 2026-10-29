//! Real lookup publication, weak borrow, worker and latest-owner lifecycle.
use super::*;
use crate::{
    resolver::{
        contracts::{
            Confidence, ExistingTextProvider, SourceTextReference, SubtitleSource,
            TextBundleFuture, TextFuture, TextQuery,
        },
        text_session::{ExistingTextSession, TextLookupStatus},
    },
    session::fuzzy_proposals::{FuzzyProposalSession, ProposalState},
    subtitle::{
        fuzzy_alignment::{FuzzyTextOrigin, paired::tests::Fixture},
        fuzzy_track::FuzzyTrackStatus,
    },
};
use std::{
    sync::{
        Mutex, Weak,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    task::Poll,
    time::Duration,
};
use tokio::sync::oneshot;

struct Provider {
    bundle: Mutex<Option<ExistingTextBundle>>,
    parent: ResolverCancellation,
    calls: AtomicUsize,
}
impl ExistingTextProvider for Provider {
    fn supports(&self, _: SubtitleSource) -> bool {
        true
    }
    fn publication_guard(&self) -> Option<ResolverCancellation> {
        Some(self.parent.clone())
    }
    fn lookup<'a>(&'a self, _: &'a TextQuery, _: ResolverCancellation) -> TextFuture<'a> {
        Box::pin(async { Err(ResolverError::UnsupportedCapability) })
    }
    fn lookup_bundle<'a>(
        &'a self,
        _: &'a TextQuery,
        _: ResolverCancellation,
    ) -> TextBundleFuture<'a> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.bundle.lock().unwrap().take())
        })
    }
}
pub(crate) struct Publication {
    lookup: ExistingTextSession,
    driver: tokio::task::JoinHandle<Result<(), ResolverError>>,
    lease: Option<ExistingTextLease>,
    provider: Arc<Provider>,
    primary: Weak<ExistingText>,
    reference: Option<Weak<SourceTextReference>>,
    pub(crate) scope: AlignmentScope,
    pub(crate) event: TranscriptEvent,
    pub(crate) stamp: WindowStamp,
}
impl Publication {
    pub(crate) async fn new(paired: bool, age: Duration) -> Self {
        Self::build(paired, age, ResolverCancellation::default()).await
    }
    async fn build(paired: bool, age: Duration, parent: ResolverCancellation) -> Self {
        let mut data = Fixture::new();
        if !paired {
            data.bundle.text = data.bundle.source_reference.take().unwrap().text;
        }
        let provider = Arc::new(Provider {
            bundle: Mutex::new(Some(data.bundle)),
            parent,
            calls: AtomicUsize::new(0),
        });
        let mut query_stamp = data.stamp;
        query_stamp.identity = data.scope.document_identity();
        query_stamp.ended_at = Instant::now() - age;
        let (mut lookup, task) = ExistingTextSession::new(query_stamp.identity);
        lookup
            .select(
                TextQuery {
                    stamp: query_stamp,
                    source: SubtitleSource::ExistingSubtitle,
                    content_id: "synthetic-paired-document".into(),
                },
                provider.clone(),
            )
            .unwrap();
        let driver = tokio::spawn(task.run());
        let update = loop {
            let update = tokio::time::timeout(Duration::from_secs(2), lookup.next())
                .await
                .unwrap()
                .unwrap();
            if update.status == TextLookupStatus::Available {
                break update;
            }
            assert!(!matches!(update.status, TextLookupStatus::Failed(_)));
        };
        let primary = Arc::downgrade(update.text.as_ref().unwrap());
        let reference = update.source_reference.as_ref().map(Arc::downgrade);
        let lease = ExistingTextLease::from_update(&update).unwrap();
        Self {
            lookup,
            driver,
            lease: Some(lease),
            provider,
            primary,
            reference,
            scope: data.scope,
            event: data.event,
            stamp: data.stamp,
        }
    }
    pub(crate) fn lease(&self) -> &ExistingTextLease {
        self.lease.as_ref().unwrap()
    }
    fn locator(&self, cpu: Arc<Semaphore>) -> FuzzyCueLocator {
        let mut locator = FuzzyCueLocator::from_lookup(self.scope, self.lease()).unwrap();
        locator.isolate_workers(cpu);
        locator
    }
    fn release_text(&mut self) {
        self.lookup.clear().unwrap();
        drop(self.lease.take());
    }
    pub(crate) fn calls(&self) -> usize {
        self.provider.calls.load(Ordering::SeqCst)
    }
    pub(crate) fn parent_cancelled(&self) -> bool {
        self.provider.parent.is_cancelled()
    }
    pub(crate) fn release(&mut self) {
        self.release_text();
    }
    pub(crate) fn text_released(&self) -> bool {
        self.primary.strong_count() == 0
            && self
                .reference
                .as_ref()
                .is_none_or(|reference| reference.strong_count() == 0)
    }
    pub(crate) async fn close(mut self) {
        self.lookup.stop().unwrap();
        assert_eq!(self.driver.await.unwrap(), Ok(()));
    }
}
async fn changed(owner: &mut FuzzyProposalSession) -> ProposalState {
    tokio::time::timeout(Duration::from_secs(2), owner.changed())
        .await
        .unwrap()
}
async fn seed(publication: &Publication, owner: &mut FuzzyProposalSession) {
    for index in 0..3 {
        let mut event = publication.event.clone();
        event.utterance = index + 1;
        event.sequence = index + 1;
        event.start_us = Some(42_000_000 + index * 1_000_000);
        event.text = char::from(b'a' + index as u8).to_string().repeat(32).into();
        owner.offer(event, publication.stamp).unwrap();
        assert_eq!(changed(owner).await, ProposalState::Proposed);
    }
    assert_eq!(owner.track_status(), FuzzyTrackStatus::Consistent(3));
}
async fn pending<F: Future>(future: std::pin::Pin<&mut F>) {
    let mut future = future;
    std::future::poll_fn(|cx| {
        assert!(future.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
}

#[tokio::test]
async fn fuzzy_lookup_source_and_paired_publications_borrow_original_allocations_without_refetch() {
    for paired in [false, true] {
        let publication = Publication::new(paired, Duration::ZERO).await;
        let pointer = publication.lease().text().cues[0].text.as_ptr() as usize;
        let reference_pointer = publication
            .lease()
            .source_reference()
            .map(|reference| reference.text.cues[0].text.as_ptr() as usize);
        let locator = publication.locator(Arc::new(Semaphore::new(1)));
        let proposal = locator
            .locate_using(
                publication.event.clone(),
                publication.stamp,
                ResolverCancellation::default(),
                move |input| {
                    let AlignmentInput::Lookup(document, _) = &input.document else {
                        panic!("expected lookup borrow")
                    };
                    assert_eq!(document.text.cues[0].text.as_ptr() as usize, pointer);
                    assert_eq!(
                        document
                            .source_reference
                            .as_ref()
                            .map(|reference| reference.text.cues[0].text.as_ptr() as usize),
                        reference_pointer
                    );
                    run_match(input)
                },
            )
            .await
            .unwrap();
        assert_eq!(
            proposal.origin(),
            if paired {
                FuzzyTextOrigin::PairedSourceReference
            } else {
                FuzzyTextOrigin::ExistingText
            }
        );
        assert_eq!(proposal.paired_primary_cue_range(), paired.then_some(2..3));
        assert_eq!(publication.provider.calls.load(Ordering::SeqCst), 1);
        publication.close().await;
    }
}

#[tokio::test]
async fn fuzzy_lookup_unpolled_future_and_locator_cannot_keep_either_track_alive() {
    let mut publication = Publication::new(true, Duration::ZERO).await;
    let locator = publication.locator(Arc::new(Semaphore::new(1)));
    let future = locator.locate(
        publication.event.clone(),
        publication.stamp,
        ResolverCancellation::default(),
    );
    publication.release_text();
    assert!(publication.primary.upgrade().is_none());
    assert!(publication.reference.as_ref().unwrap().upgrade().is_none());
    assert_eq!(future.await.err(), Some(ResolverError::Cancelled.into()));
    publication.close().await;
}

#[tokio::test]
async fn fuzzy_lookup_retirement_revokes_delivered_proposal_and_never_cancels_provider() {
    let mut publication = Publication::new(true, Duration::ZERO).await;
    let mut locator = publication.locator(Arc::new(Semaphore::new(1)));
    let proposal = locator
        .locate(
            publication.event.clone(),
            publication.stamp,
            ResolverCancellation::default(),
        )
        .await
        .unwrap();
    publication.lease().retire();
    publication.release_text();
    assert_eq!(
        proposal.validate(publication.scope, proposal.binding()),
        Err(ResolverError::Cancelled.into())
    );
    assert!(publication.primary.upgrade().is_none());
    assert!(publication.reference.as_ref().unwrap().upgrade().is_none());
    locator.stop();
    assert!(!publication.provider.parent.is_cancelled());
    publication.close().await;
}

#[tokio::test]
async fn fuzzy_lookup_last_content_release_is_detected_by_snapshot_validation_and_poll() {
    let mut publication = Publication::new(true, Duration::ZERO).await;
    let (mut owner, task) =
        FuzzyProposalSession::new(publication.locator(Arc::new(Semaphore::new(1)))).unwrap();
    let driver = tokio::spawn(task.run());
    seed(&publication, &mut owner).await;
    let track = owner.track_proposal().unwrap();
    publication.release_text(); // No explicit retire event; validate/poll must still reject.
    assert!(publication.primary.upgrade().is_none());
    assert!(publication.reference.as_ref().unwrap().upgrade().is_none());
    assert_eq!(
        track.validate(publication.scope),
        Err(ResolverError::Cancelled.into())
    );
    assert_eq!(owner.poll(), ProposalState::Cancelled);
    owner.stop();
    assert_eq!(driver.await.unwrap(), Ok(()));
    publication.close().await;
}

#[tokio::test]
async fn fuzzy_lookup_parent_retirement_wakes_idle_pending_and_delivered_owners() {
    for mode in 0..3 {
        let publication = Publication::new(true, Duration::ZERO).await;
        let (mut owner, task) =
            FuzzyProposalSession::new(publication.locator(Arc::new(Semaphore::new(1)))).unwrap();
        let driver = tokio::spawn(task.run());
        if mode > 0 {
            owner
                .offer(publication.event.clone(), publication.stamp)
                .unwrap();
        }
        if mode == 2 {
            assert_eq!(changed(&mut owner).await, ProposalState::Proposed);
        }
        publication.provider.parent.cancel();
        assert_eq!(changed(&mut owner).await, ProposalState::Cancelled);
        assert_eq!(driver.await.unwrap(), Err(ResolverError::Cancelled.into()));
        publication.close().await;
    }
}

#[tokio::test]
async fn fuzzy_lookup_idle_original_deadline_wakes_owner_and_exits_driver_without_asr() {
    let publication = Publication::new(true, MAX_WINDOW_AGE - Duration::from_millis(800)).await;
    let (mut owner, task) =
        FuzzyProposalSession::new(publication.locator(Arc::new(Semaphore::new(1)))).unwrap();
    let driver = tokio::spawn(task.run());
    assert_eq!(
        changed(&mut owner).await,
        ProposalState::Failed(ResolverError::Stale.into())
    );
    assert_eq!(driver.await.unwrap(), Err(ResolverError::Stale.into()));
    assert_eq!(
        owner.offer(publication.event.clone(), publication.stamp),
        Err(ResolverError::Stale.into())
    );
    publication.close().await;
}

#[tokio::test]
async fn fuzzy_lookup_track_and_lexical_hint_expire_before_fresh_capture_window() {
    let publication = Publication::new(true, MAX_WINDOW_AGE - Duration::from_millis(800)).await;
    let (mut owner, task) =
        FuzzyProposalSession::new(publication.locator(Arc::new(Semaphore::new(1)))).unwrap();
    let driver = tokio::spawn(task.run());
    seed(&publication, &mut owner).await;
    let track = owner.track_proposal().unwrap();
    assert_eq!(
        changed(&mut owner).await,
        ProposalState::Failed(ResolverError::Stale.into())
    );
    assert!(owner.proposal().is_none());
    assert_eq!(
        track.validate(publication.scope),
        Err(ResolverError::Stale.into())
    );
    assert_eq!(driver.await.unwrap(), Err(ResolverError::Stale.into()));
    publication
        .stamp
        .validate(publication.scope.capture_identity(), Instant::now())
        .unwrap();
    publication.close().await;
}

#[tokio::test]
async fn fuzzy_lookup_renewal_does_not_extend_old_weak_snapshot_or_candidate() {
    let mut publication = Publication::new(true, MAX_WINDOW_AGE - Duration::from_millis(800)).await;
    let old_locator = publication.locator(Arc::new(Semaphore::new(1)));
    let old = old_locator
        .locate(
            publication.event.clone(),
            publication.stamp,
            ResolverCancellation::default(),
        )
        .await
        .unwrap();
    let deadline = publication.lease().query().stamp.ended_at + MAX_WINDOW_AGE;
    let mut renewed = publication.lease().query().stamp;
    renewed.sequence += 1;
    renewed.ended_at = Instant::now();
    publication
        .lease
        .as_mut()
        .unwrap()
        .renew(renewed, Confidence::new(900).unwrap())
        .unwrap();
    let new_scope = AlignmentScope::new(
        publication.scope.document_identity(),
        publication.scope.capture_identity(),
        2,
    )
    .unwrap();
    let mut new_locator = FuzzyCueLocator::from_lookup(new_scope, publication.lease()).unwrap();
    new_locator.isolate_workers(Arc::new(Semaphore::new(1)));
    let new = new_locator
        .locate(
            publication.event.clone(),
            publication.stamp,
            ResolverCancellation::default(),
        )
        .await
        .unwrap();
    tokio::time::sleep_until((deadline + Duration::from_millis(10)).into()).await;
    assert_eq!(
        old.validate(publication.scope, old.binding()),
        Err(ResolverError::Stale.into())
    );
    new.validate(new_scope, new.binding()).unwrap();
    assert_eq!(publication.provider.calls.load(Ordering::SeqCst), 1);
    publication.close().await;
}

#[tokio::test]
async fn fuzzy_lookup_reduced_confidence_retires_old_root_without_cancelling_parent() {
    let mut publication = Publication::new(true, Duration::ZERO).await;
    let mut stamp = publication.lease().query().stamp;
    publication
        .lease
        .as_mut()
        .unwrap()
        .renew(stamp, Confidence::new(900).unwrap())
        .unwrap();
    let locator = publication.locator(Arc::new(Semaphore::new(1)));
    let proposal = locator
        .locate(
            publication.event.clone(),
            publication.stamp,
            ResolverCancellation::default(),
        )
        .await
        .unwrap();
    stamp.sequence += 1;
    stamp.ended_at = Instant::now();
    publication
        .lease
        .as_mut()
        .unwrap()
        .renew(stamp, Confidence::new(800).unwrap())
        .unwrap();
    assert_eq!(
        proposal.validate(publication.scope, proposal.binding()),
        Err(ResolverError::Cancelled.into())
    );
    assert!(!publication.provider.parent.is_cancelled());
    assert!(FuzzyCueLocator::from_lookup(publication.scope, publication.lease()).is_ok());
    publication.close().await;
}

#[tokio::test]
async fn fuzzy_lookup_started_retirement_waits_for_real_cleanup_and_releases_both_tracks() {
    let mut publication = Publication::new(true, Duration::ZERO).await;
    let cpu = Arc::new(Semaphore::new(1));
    let locator = publication.locator(cpu.clone());
    let (entered, entered_rx) = oneshot::channel();
    let (release, release_rx) = mpsc::sync_channel(1);
    let request = locator.locate_using(
        publication.event.clone(),
        publication.stamp,
        ResolverCancellation::default(),
        move |input| {
            entered.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            run_match(input)
        },
    );
    tokio::pin!(request);
    pending(request.as_mut()).await;
    entered_rx.await.unwrap();
    publication.lease().retire();
    publication.release_text();
    pending(request.as_mut()).await;
    assert!(publication.primary.upgrade().is_some());
    assert!(publication.reference.as_ref().unwrap().upgrade().is_some());
    assert_eq!(cpu.available_permits(), 0);
    release.send(()).unwrap();
    assert_eq!(request.await.err(), Some(ResolverError::Cancelled.into()));
    assert!(publication.primary.upgrade().is_none());
    assert!(publication.reference.as_ref().unwrap().upgrade().is_none());
    assert_eq!(cpu.available_permits(), 1);
    publication.close().await;
}

#[tokio::test]
async fn fuzzy_lookup_work_deadline_is_original_source_deadline_not_dispatch_time() {
    let publication = Publication::new(true, MAX_WINDOW_AGE - Duration::from_millis(800)).await;
    let locator = publication.locator(Arc::new(Semaphore::new(1)));
    let expected = publication.lease().query().stamp.ended_at + MAX_WINDOW_AGE;
    locator
        .locate_using(
            publication.event.clone(),
            publication.stamp,
            ResolverCancellation::default(),
            move |input| {
                assert_eq!(input.deadline, expected);
                run_match(input)
            },
        )
        .await
        .unwrap();
    assert_eq!(locator.source_deadline(), Some(expected));
    publication.close().await;
}

#[tokio::test]
async fn fuzzy_lookup_source_expiry_joins_started_cleanup_before_releasing_capacity() {
    let publication = Publication::new(true, MAX_WINDOW_AGE - Duration::from_millis(800)).await;
    let cpu = Arc::new(Semaphore::new(1));
    let locator = publication.locator(cpu.clone());
    let deadline = locator.source_deadline().unwrap();
    let (entered, entered_rx) = oneshot::channel();
    let (release, release_rx) = mpsc::sync_channel(1);
    let request = locator.locate_using(
        publication.event.clone(),
        publication.stamp,
        ResolverCancellation::default(),
        move |input| {
            entered.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            run_match(input)
        },
    );
    tokio::pin!(request);
    pending(request.as_mut()).await;
    entered_rx.await.unwrap();
    tokio::time::sleep_until((deadline + Duration::from_millis(10)).into()).await;
    pending(request.as_mut()).await;
    assert_eq!(cpu.available_permits(), 0);
    assert_eq!(locator.local.available_permits(), 0);
    release.send(()).unwrap();
    assert_eq!(request.await.err(), Some(ResolverError::Stale.into()));
    assert_eq!(cpu.available_permits(), 1);
    publication.close().await;
}

#[tokio::test]
async fn fuzzy_lookup_four_parent_limit_is_not_expanded_and_children_do_not_cancel_parent() {
    let publication = Publication::new(true, Duration::ZERO).await;
    let locator = publication.locator(Arc::new(Semaphore::new(1)));
    assert!(
        locator
            .locate(
                publication.event.clone(),
                publication.stamp,
                ResolverCancellation::default()
            )
            .await
            .is_ok()
    );
    let parent = ResolverCancellation::default();
    let child = ResolverCancellation::linked(parent.clone()).unwrap();
    assert_eq!(
        locator
            .locate(publication.event.clone(), publication.stamp, child)
            .await
            .err(),
        Some(ResolverError::InvalidData.into())
    );
    assert!(!parent.is_cancelled());
    assert!(!publication.provider.parent.is_cancelled());
    publication.close().await;
}

#[tokio::test]
async fn fuzzy_lookup_wrong_document_scope_and_retired_source_are_not_admitted() {
    let publication = Publication::new(true, Duration::ZERO).await;
    let mut document = publication.scope.document_identity();
    document.generation += 1;
    let scope = AlignmentScope::new(document, publication.scope.capture_identity(), 1).unwrap();
    assert_eq!(
        FuzzyCueLocator::from_lookup(scope, publication.lease()).err(),
        Some(ResolverError::InvalidData.into())
    );
    publication.lease().retire();
    assert_eq!(
        FuzzyCueLocator::from_lookup(publication.scope, publication.lease()).err(),
        Some(ResolverError::Cancelled.into())
    );
    publication.close().await;
}
