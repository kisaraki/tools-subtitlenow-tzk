//! Explicit synthetic paired publications and metadata lifetimes; no network.
use super::*;
use crate::{ai::speech::SpeechLanguage, audio::capture::SessionId};
use std::sync::atomic::{AtomicUsize, Ordering};

fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(973),
        generation: 1,
    }
}
fn query(sequence: u64) -> TextQuery {
    TextQuery {
        stamp: WindowStamp {
            identity: identity(),
            sequence,
            ended_at: Instant::now(),
            start_us: Some(0),
            duration_us: 12_000_000,
        },
        source: SubtitleSource::ExistingSubtitle,
        content_id: "synthetic film".into(),
    }
}
fn bundle(query: &TextQuery) -> ExistingTextBundle {
    let text = |language, words: &str| ExistingText {
        identity: query.stamp.identity,
        source: query.source,
        content_id: query.content_id.clone(),
        language,
        cues: vec![TextCue {
            start_us: Some(1_000_000),
            duration_us: Some(1_000_000),
            text: words.into(),
        }],
    };
    ExistingTextBundle {
        text: text(TextLanguage::TaiwanTraditional, "合成字幕測試"),
        source_reference: Some(SourceTextReference {
            text: text(
                TextLanguage::Source(SpeechLanguage::English),
                "synthetic complete cue",
            ),
            primary_edition_id: "synthetic edit".into(),
            reference_edition_id: "synthetic edit".into(),
        }),
    }
}
struct Active(Arc<AtomicUsize>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
struct PairProvider {
    pending: bool,
    guard: Option<ResolverCancellation>,
    lookups: AtomicUsize,
    bundles: AtomicUsize,
    active: Arc<AtomicUsize>,
    entered: tokio::sync::mpsc::Sender<ResolverCancellation>,
}
impl PairProvider {
    fn enter(&self, cancel: ResolverCancellation) -> Active {
        self.active.fetch_add(1, Ordering::SeqCst);
        self.entered.try_send(cancel).unwrap();
        Active(self.active.clone())
    }
}
impl ExistingTextProvider for PairProvider {
    fn supports(&self, source: SubtitleSource) -> bool {
        source == SubtitleSource::ExistingSubtitle
    }
    fn publication_guard(&self) -> Option<ResolverCancellation> {
        self.guard.clone()
    }
    fn lookup<'a>(&'a self, query: &'a TextQuery, cancel: ResolverCancellation) -> TextFuture<'a> {
        Box::pin(async move {
            self.lookups.fetch_add(1, Ordering::SeqCst);
            let _active = self.enter(cancel);
            if self.pending {
                std::future::pending::<()>().await;
            }
            Ok(Some(bundle(query).text))
        })
    }
    fn lookup_bundle<'a>(
        &'a self,
        query: &'a TextQuery,
        cancel: ResolverCancellation,
    ) -> TextBundleFuture<'a> {
        Box::pin(async move {
            self.bundles.fetch_add(1, Ordering::SeqCst);
            let _active = self.enter(cancel);
            if self.pending {
                std::future::pending::<()>().await;
            }
            Ok(Some(bundle(query)))
        })
    }
}
fn provider(
    pending: bool,
    guard: Option<ResolverCancellation>,
) -> (
    Arc<PairProvider>,
    tokio::sync::mpsc::Receiver<ResolverCancellation>,
) {
    let (entered, receiver) = tokio::sync::mpsc::channel(8);
    (
        Arc::new(PairProvider {
            pending,
            guard,
            lookups: AtomicUsize::new(0),
            bundles: AtomicUsize::new(0),
            active: Arc::new(AtomicUsize::new(0)),
            entered,
        }),
        receiver,
    )
}
async fn wait(
    session: &mut ExistingTextSession,
    wanted: TextLookupStatus,
) -> Arc<TextLookupUpdate> {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let update = session.next().await.unwrap();
            if update.status == wanted {
                return update;
            }
        }
    })
    .await
    .unwrap()
}
async fn entered(
    receiver: &mut tokio::sync::mpsc::Receiver<ResolverCancellation>,
) -> ResolverCancellation {
    tokio::time::timeout(Duration::from_secs(5), receiver.recv())
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn guarded_wrapper_preserves_the_same_atomic_reference_bundle() {
    let metadata = ResolverCancellation::default();
    let (inner, mut calls) = provider(false, None);
    let wrapped = guard_provider(inner.clone(), Some(metadata.clone()));
    assert!(wrapped.supports(SubtitleSource::ExistingSubtitle));
    assert!(!wrapped.supports(SubtitleSource::ExistingLyrics));
    assert!(wrapped.publication_guard().unwrap().same_scope(&metadata));
    let query = query(1);
    let result = wrapped
        .lookup_bundle(&query, ResolverCancellation::default())
        .await
        .unwrap()
        .unwrap();
    result.validate(&query).unwrap();
    assert_eq!(
        result
            .source_reference
            .unwrap()
            .reference_edition_id
            .as_ref(),
        "synthetic edit"
    );
    assert_eq!(inner.lookups.load(Ordering::SeqCst), 0);
    assert_eq!(inner.bundles.load(Ordering::SeqCst), 1);
    assert!(!entered(&mut calls).await.is_cancelled());
    assert_eq!(inner.active.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn retired_metadata_or_query_never_enters_provider_io() {
    for retire_metadata in [false, true] {
        let metadata = ResolverCancellation::default();
        let request = ResolverCancellation::default();
        let (inner, _) = provider(false, None);
        let wrapped = guard_provider(inner.clone(), Some(metadata.clone()));
        if retire_metadata {
            metadata.cancel();
        } else {
            request.cancel();
        }
        let query = query(1);
        assert!(matches!(
            wrapped.lookup(&query, request.clone()).await,
            Err(ResolverError::Cancelled)
        ));
        assert!(matches!(
            wrapped.lookup_bundle(&query, request).await,
            Err(ResolverError::Cancelled)
        ));
        assert_eq!(inner.lookups.load(Ordering::SeqCst), 0);
        assert_eq!(inner.bundles.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn metadata_and_query_retirement_each_drop_pending_inner_future() {
    for retire_metadata in [false, true] {
        let metadata = ResolverCancellation::default();
        let request = ResolverCancellation::default();
        let (inner, mut calls) = provider(true, None);
        let wrapped = guard_provider(inner.clone(), Some(metadata.clone()));
        let request_copy = request.clone();
        let job = tokio::spawn(async move { wrapped.lookup_bundle(&query(1), request_copy).await });
        entered(&mut calls).await;
        assert_eq!(inner.active.load(Ordering::SeqCst), 1);
        if retire_metadata {
            metadata.cancel();
        } else {
            request.cancel();
        }
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(5), job)
                .await
                .unwrap()
                .unwrap(),
            Err(ResolverError::Cancelled)
        ));
        assert_eq!(inner.active.load(Ordering::SeqCst), 0);
        assert_eq!(metadata.is_cancelled(), retire_metadata);
    }
}

#[tokio::test]
async fn dropping_guarded_request_releases_inner_future_without_retiring_parent() {
    let metadata = ResolverCancellation::default();
    let request = ResolverCancellation::default();
    let (inner, mut calls) = provider(true, None);
    let wrapped = guard_provider(inner.clone(), Some(metadata.clone()));
    let query = query(1);
    let mut future = wrapped.lookup(&query, request.clone());
    tokio::select! { biased;
        _ = &mut future => panic!("pending fixture completed"),
        _ = tokio::task::yield_now() => {},
    }
    entered(&mut calls).await;
    assert_eq!(inner.active.load(Ordering::SeqCst), 1);
    drop(future);
    assert_eq!(inner.active.load(Ordering::SeqCst), 0);
    assert!(!metadata.is_cancelled());
    assert!(!request.is_cancelled());
}

#[tokio::test]
async fn composed_provider_guard_observes_inner_and_outer_metadata() {
    let inner_metadata = ResolverCancellation::default();
    let outer_metadata = ResolverCancellation::default();
    let (inner, _) = provider(false, Some(inner_metadata.clone()));
    let wrapped = guard_provider(inner.clone(), Some(outer_metadata.clone()));
    let composite = wrapped.publication_guard().unwrap();
    composite.cancel();
    assert!(!inner_metadata.is_cancelled());
    assert!(!outer_metadata.is_cancelled());
    assert!(matches!(
        wrapped
            .lookup_bundle(&query(1), ResolverCancellation::default())
            .await,
        Err(ResolverError::Cancelled)
    ));
    assert_eq!(inner.bundles.load(Ordering::SeqCst), 0);
    let wrapped = guard_provider(inner, Some(outer_metadata.clone()));
    inner_metadata.cancel();
    assert!(wrapped.publication_guard().unwrap().is_cancelled());
    assert!(!outer_metadata.is_cancelled());
}

#[tokio::test]
async fn session_rejects_retired_selection_and_cancels_before_task_starts_io() {
    let metadata = ResolverCancellation::default();
    let (inner, _) = provider(true, None);
    let wrapped = guard_provider(inner.clone(), Some(metadata.clone()));
    let (mut session, task) = ExistingTextSession::new(identity());
    session.select(query(1), wrapped.clone()).unwrap();
    metadata.cancel();
    assert_eq!(
        session.select(query(2), wrapped),
        Err(ResolverError::Cancelled)
    );
    let job = tokio::spawn(task.run());
    let failed = wait(
        &mut session,
        TextLookupStatus::Failed(ResolverError::Cancelled),
    )
    .await;
    assert!(failed.text.is_none());
    assert!(failed.source_reference.is_none());
    assert!(session.poll().is_none());
    assert_eq!(inner.bundles.load(Ordering::SeqCst), 0);
    session.stop().unwrap();
    job.await.unwrap().unwrap();
}

#[tokio::test]
async fn next_wakes_once_on_metadata_retirement_while_lookup_is_pending() {
    let metadata = ResolverCancellation::default();
    let (inner, mut calls) = provider(true, None);
    let wrapped = guard_provider(inner.clone(), Some(metadata.clone()));
    let (mut session, task) = ExistingTextSession::new(identity());
    session.select(query(1), wrapped).unwrap();
    let job = tokio::spawn(task.run());
    let request = entered(&mut calls).await;
    wait(&mut session, TextLookupStatus::LookingUp).await;
    let retirement = async {
        tokio::task::yield_now().await;
        metadata.cancel();
    };
    let (failed, _) = tokio::join!(
        wait(
            &mut session,
            TextLookupStatus::Failed(ResolverError::Cancelled)
        ),
        retirement,
    );
    assert!(failed.text.is_none());
    assert!(request.is_cancelled());
    assert!(session.poll().is_none());
    let mut quiet = Box::pin(session.next());
    tokio::select! { biased;
        _ = &mut quiet => panic!("cancelled metadata caused repeated publication"),
        _ = tokio::task::yield_now() => {},
    }
    drop(quiet);
    session.stop().unwrap();
    job.await.unwrap().unwrap();
    assert_eq!(inner.active.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn completed_ready_publication_and_all_renewed_leases_retire_with_metadata() {
    let metadata = ResolverCancellation::default();
    let (inner, mut calls) = provider(false, None);
    let wrapped = guard_provider(inner, Some(metadata.clone()));
    let (mut session, task) = ExistingTextSession::new(identity());
    session.select(query(1), wrapped).unwrap();
    let job = tokio::spawn(task.run());
    let update = wait(&mut session, TextLookupStatus::Available).await;
    assert!(entered(&mut calls).await.is_cancelled()); // Request finished, metadata remains active.
    let mut lease = ExistingTextLease::from_update(&update).unwrap();
    lease
        .renew(query(1).stamp, Confidence::new(950).unwrap())
        .unwrap();
    let original = lease.clone();
    lease
        .renew(query(2).stamp, Confidence::new(900).unwrap())
        .unwrap();
    assert!(matches!(
        original.validate(identity()),
        Err(ResolverError::Cancelled)
    ));
    assert!(!metadata.is_cancelled());
    lease.validate(identity()).unwrap();
    assert!(lease.source_reference().is_some());
    let renewed = lease.clone();
    let retirement = async {
        tokio::task::yield_now().await;
        metadata.cancel();
    };
    let (failed, _) = tokio::join!(
        wait(
            &mut session,
            TextLookupStatus::Failed(ResolverError::Cancelled)
        ),
        retirement,
    );
    assert!(matches!(
        renewed.validate(identity()),
        Err(ResolverError::Cancelled)
    ));
    tokio::time::timeout(Duration::from_secs(5), renewed.availability().cancelled())
        .await
        .unwrap();
    assert_eq!(
        lease.renew(query(3).stamp, Confidence::new(850).unwrap()),
        Err(ResolverError::Cancelled)
    );
    assert!(matches!(
        ExistingTextLease::from_update(&update),
        Err(ResolverError::Cancelled)
    ));
    assert!(failed.text.is_none());
    assert!(failed.source_reference.is_none());
    assert!(session.poll().is_none());
    session.stop().unwrap();
    job.await.unwrap().unwrap();
}

#[tokio::test]
async fn retiring_one_lease_does_not_cancel_metadata_or_sibling_publication() {
    let metadata = ResolverCancellation::default();
    let (inner, _calls) = provider(false, None);
    let wrapped = guard_provider(inner, Some(metadata.clone()));
    let (mut session, task) = ExistingTextSession::new(identity());
    session.select(query(1), wrapped).unwrap();
    let job = tokio::spawn(task.run());
    let update = wait(&mut session, TextLookupStatus::Available).await;
    let first = ExistingTextLease::from_update(&update).unwrap();
    let sibling = ExistingTextLease::from_update(&update).unwrap();
    assert!(!first.availability().same_scope(&sibling.availability()));
    first.retire();
    assert!(!metadata.is_cancelled());
    sibling.validate(identity()).unwrap();
    metadata.cancel();
    assert!(matches!(
        sibling.validate(identity()),
        Err(ResolverError::Cancelled)
    ));
    session.stop().unwrap();
    job.await.unwrap().unwrap();
}
