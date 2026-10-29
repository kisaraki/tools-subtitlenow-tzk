//! Synthetic text and explicit mocks only; HTTP integration lives in lyrics_tests.
use super::*;
use crate::{ai::speech::SpeechLanguage, audio::capture::SessionId};
use std::sync::atomic::{AtomicUsize, Ordering};

fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(920),
        generation: 2,
    }
}
fn stamp(sequence: u64) -> WindowStamp {
    WindowStamp {
        identity: identity(),
        sequence,
        ended_at: Instant::now(),
        start_us: None,
        duration_us: 12_000_000,
    }
}
fn query(sequence: u64, id: &str) -> TextQuery {
    TextQuery {
        stamp: stamp(sequence),
        source: SubtitleSource::ExistingLyrics,
        content_id: id.into(),
    }
}
fn resolution(id: &str) -> Resolution {
    Resolution {
        kind: ContentType::Music,
        song_state: SongState::Confirmed,
        song: Some(Arc::new(SongCandidate {
            id: id.into(),
            title: "synthetic title".into(),
            artist: Some("synthetic artist".into()),
            confidence: Confidence::new(900).unwrap(),
            match_offset_us: None,
        })),
        movies: Vec::new(),
    }
}
fn text(query: &TextQuery) -> Result<Option<ExistingText>, ResolverError> {
    Ok(Some(ExistingText {
        identity: query.stamp.identity,
        source: query.source,
        content_id: query.content_id.clone(),
        language: TextLanguage::Source(SpeechLanguage::English),
        cues: vec![TextCue {
            start_us: Some(1_000_000),
            duration_us: Some(1_000_000),
            text: "original synthetic fixture words".into(),
        }],
    }))
}
struct Call {
    sequence: u64,
    cancel: ResolverCancellation,
}
struct MockProvider {
    delay: Duration,
    behavior: fn(&TextQuery) -> Result<Option<ExistingText>, ResolverError>,
    calls: AtomicUsize,
    active: Arc<AtomicUsize>,
    entered: tokio::sync::mpsc::Sender<Call>,
    supported: bool,
}
struct Active(Arc<AtomicUsize>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
impl ExistingTextProvider for MockProvider {
    fn supports(&self, source: SubtitleSource) -> bool {
        self.supported
            && matches!(
                source,
                SubtitleSource::ExistingLyrics | SubtitleSource::ExistingSubtitle
            )
    }
    fn lookup<'a>(&'a self, query: &'a TextQuery, cancel: ResolverCancellation) -> TextFuture<'a> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.active.fetch_add(1, Ordering::SeqCst);
            let _active = Active(Arc::clone(&self.active));
            self.entered
                .try_send(Call {
                    sequence: query.stamp.sequence,
                    cancel,
                })
                .unwrap();
            tokio::time::sleep(self.delay).await;
            (self.behavior)(query)
        })
    }
}
fn mock(
    delay: Duration,
    behavior: fn(&TextQuery) -> Result<Option<ExistingText>, ResolverError>,
) -> (Arc<MockProvider>, tokio::sync::mpsc::Receiver<Call>) {
    let (entered, receiver) = tokio::sync::mpsc::channel(16);
    (
        Arc::new(MockProvider {
            delay,
            behavior,
            calls: AtomicUsize::new(0),
            active: Arc::new(AtomicUsize::new(0)),
            entered,
            supported: true,
        }),
        receiver,
    )
}
async fn entered(receiver: &mut tokio::sync::mpsc::Receiver<Call>) -> Call {
    tokio::time::timeout(Duration::from_secs(5), receiver.recv())
        .await
        .unwrap()
        .unwrap()
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

#[tokio::test]
async fn confirmed_song_lookup_and_repeated_windows_keep_one_request_and_original_stamp() {
    let (provider, mut receiver) = mock(Duration::from_millis(20), text);
    let (mut session, task) = ExistingTextSession::new(identity());
    let first = stamp(1);
    assert!(
        session
            .select_confirmed_lyrics(first, &resolution("A"), Some(provider.clone()))
            .unwrap()
    );
    let job = tokio::spawn(task.run());
    let call = entered(&mut receiver).await;
    assert!(
        !session
            .select_confirmed_lyrics(stamp(2), &resolution("A"), Some(provider.clone()))
            .unwrap()
    );
    assert!(!call.cancel.is_cancelled());
    let update = wait(&mut session, TextLookupStatus::Available).await;
    assert_eq!(update.query.as_ref().unwrap().stamp.sequence, 1);
    assert_eq!(update.text.as_ref().unwrap().content_id.as_ref(), "A");
    assert!(update.source_reference.is_none());
    assert!(
        ExistingTextLease::from_update(&update)
            .unwrap()
            .source_reference()
            .is_none()
    );
    assert_eq!(update.identity, identity());
    assert!(call.cancel.is_cancelled());
    assert!(
        !session
            .select_confirmed_lyrics(stamp(3), &resolution("A"), Some(provider.clone()))
            .unwrap()
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    session.stop().unwrap();
    job.await.unwrap().unwrap();
}

#[tokio::test]
async fn switching_songs_cancels_request_immediately_and_keeps_only_latest_pending_selection() {
    let (provider, mut receiver) = mock(Duration::from_millis(150), text);
    let (mut session, task) = ExistingTextSession::new(identity());
    session.select(query(1, "old"), provider.clone()).unwrap();
    let job = tokio::spawn(task.run());
    let old = entered(&mut receiver).await;
    assert_eq!(old.sequence, 1);
    for sequence in 2..=50 {
        session
            .select(
                query(sequence, &format!("song-{sequence}")),
                provider.clone(),
            )
            .unwrap();
    }
    assert!(old.cancel.is_cancelled());
    let frame = session.input.borrow().clone();
    assert_eq!(
        frame.selection.as_ref().unwrap().query.content_id.as_ref(),
        "song-50"
    );
    let result = wait(&mut session, TextLookupStatus::Available).await;
    assert_eq!(result.text.as_ref().unwrap().content_id.as_ref(), "song-50");
    assert_eq!(result.stamp.unwrap().sequence, 50);
    assert!(provider.calls.load(Ordering::SeqCst) <= 2);
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
    session.stop().unwrap();
    job.await.unwrap().unwrap();
}

#[tokio::test]
async fn transition_unknown_speech_and_weak_candidate_clear_lyrics_without_lookup() {
    let (provider, mut receiver) = mock(Duration::from_secs(30), text);
    let (mut session, task) = ExistingTextSession::new(identity());
    session
        .select_confirmed_lyrics(stamp(1), &resolution("A"), Some(provider.clone()))
        .unwrap();
    let job = tokio::spawn(task.run());
    let call = entered(&mut receiver).await;
    for (sequence, kind, song_state) in [
        (2, ContentType::Mixed, SongState::Transition),
        (3, ContentType::Unknown, SongState::Unknown),
        (4, ContentType::Speech, SongState::Unknown),
        (5, ContentType::Music, SongState::Candidate),
    ] {
        let mut resolution = resolution("A");
        resolution.kind = kind;
        resolution.song_state = song_state;
        assert!(
            !session
                .select_confirmed_lyrics(stamp(sequence), &resolution, Some(provider.clone()))
                .unwrap()
        );
        let update = session.poll().unwrap();
        assert_eq!(update.status, TextLookupStatus::Cleared);
        assert!(update.text.is_none() && update.query.is_none());
    }
    assert!(call.cancel.is_cancelled());
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    session.stop().unwrap();
    job.await.unwrap().unwrap();
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn unread_old_text_is_released_on_clear_and_provider_or_source_replacement() {
    let (provider, _receiver) = mock(Duration::ZERO, text);
    let (mut session, task) = ExistingTextSession::new(identity());
    let job = tokio::spawn(task.run());
    session.select(query(1, "A"), provider.clone()).unwrap();
    let result = wait(&mut session, TextLookupStatus::Available).await;
    let weak = Arc::downgrade(result.text.as_ref().unwrap());
    drop(result);
    session.clear().unwrap();
    assert!(weak.upgrade().is_none());
    assert_eq!(session.poll().unwrap().status, TextLookupStatus::Cleared);
    session.select(query(2, "A"), provider.clone()).unwrap();
    wait(&mut session, TextLookupStatus::Available).await;
    let (replacement, _receiver) = mock(Duration::ZERO, text);
    assert!(session.select(query(3, "A"), replacement.clone()).unwrap());
    let result = wait(&mut session, TextLookupStatus::Available).await;
    assert_eq!(result.query.as_ref().unwrap().stamp.sequence, 3);
    let mut subtitle_query = query(4, "A");
    subtitle_query.source = SubtitleSource::ExistingSubtitle;
    assert!(session.select(subtitle_query, replacement.clone()).unwrap());
    let result = wait(&mut session, TextLookupStatus::Available).await;
    assert_eq!(
        result.text.as_ref().unwrap().source,
        SubtitleSource::ExistingSubtitle
    );
    assert_eq!(replacement.calls.load(Ordering::SeqCst), 2);
    session.stop().unwrap();
    job.await.unwrap().unwrap();
}

#[tokio::test]
async fn unavailable_and_provider_errors_never_retry_without_explicit_refresh() {
    for behavior in [
        (|_: &TextQuery| Ok(None)) as fn(&TextQuery) -> _,
        (|_: &TextQuery| Err(ResolverError::RateLimited)) as fn(&TextQuery) -> _,
        (|_: &TextQuery| Err(ResolverError::Credential)) as fn(&TextQuery) -> _,
    ] {
        let (provider, mut receiver) = mock(Duration::ZERO, behavior);
        let (mut session, task) = ExistingTextSession::new(identity());
        let job = tokio::spawn(task.run());
        session.select(query(1, "A"), provider.clone()).unwrap();
        let expected = match behavior(&query(1, "A")) {
            Ok(_) => TextLookupStatus::Unavailable,
            Err(error) => TextLookupStatus::Failed(error),
        };
        wait(&mut session, expected).await;
        assert!(entered(&mut receiver).await.cancel.is_cancelled());
        assert!(!session.select(query(2, "A"), provider.clone()).unwrap());
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
        session.refresh(query(3, "A"), provider.clone()).unwrap();
        wait(&mut session, expected).await;
        assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
        session.stop().unwrap();
        job.await.unwrap().unwrap();
    }
}

#[tokio::test]
async fn slow_or_unpolled_input_stale_completion_and_timeout_fail_safely() {
    let (provider, mut receiver) = mock(Duration::from_secs(30), text);
    let (mut session, task) = ExistingTextSession::new(identity());
    session.select(query(1, "A"), provider.clone()).unwrap();
    let job = tokio::spawn(task.run());
    let call = entered(&mut receiver).await;
    wait(
        &mut session,
        TextLookupStatus::Failed(ResolverError::Timeout),
    )
    .await;
    assert!(call.cancel.is_cancelled());
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
    session.stop().unwrap();
    job.await.unwrap().unwrap();
    let (provider, _receiver) = mock(Duration::from_millis(100), text);
    let (mut session, task) = ExistingTextSession::new(identity());
    let mut q = query(1, "A");
    q.stamp.ended_at -= MAX_WINDOW_AGE - Duration::from_millis(50);
    session.select(q, provider).unwrap();
    let job = tokio::spawn(task.run());
    wait(&mut session, TextLookupStatus::Failed(ResolverError::Stale)).await;
    session.stop().unwrap();
    job.await.unwrap().unwrap();
    let (provider, _receiver) = mock(Duration::ZERO, text);
    let (mut session, task) = ExistingTextSession::new(identity());
    session.select(query(1, "A"), provider.clone()).unwrap();
    // Deterministic unpolled backlog fixture: original query has genuinely expired.
    let old = session.input.borrow().selection.clone().unwrap();
    let mut old_query = query(1, "A");
    old_query.stamp.ended_at -= MAX_WINDOW_AGE + Duration::from_secs(1);
    session.input.send_modify(|frame| {
        frame.selection = Some(Arc::new(Selection {
            query: Arc::new(old_query),
            provider: old.provider.clone(),
            cancel: old.cancel.clone(),
            provider_guard: old.provider_guard.clone(),
        }))
    });
    let job = tokio::spawn(task.run());
    wait(&mut session, TextLookupStatus::Failed(ResolverError::Stale)).await;
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    session.stop().unwrap();
    job.await.unwrap().unwrap();
}

#[tokio::test]
async fn session_drop_cancels_live_request_and_releases_provider_future() {
    let (provider, mut receiver) = mock(Duration::from_secs(30), text);
    let (mut session, task) = ExistingTextSession::new(identity());
    session.select(query(1, "A"), provider.clone()).unwrap();
    let job = tokio::spawn(task.run());
    let call = entered(&mut receiver).await;
    drop(session);
    assert!(call.cancel.is_cancelled());
    job.await.unwrap().unwrap();
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn task_future_drop_stops_owner_cancels_request_and_clears_text() {
    let (provider, mut receiver) = mock(Duration::from_secs(30), text);
    let (mut session, task) = ExistingTextSession::new(identity());
    session.select(query(1, "A"), provider.clone()).unwrap();
    let mut run = Box::pin(task.run());
    let call = tokio::select! { _ = &mut run => panic!("unexpected completion"), call = entered(&mut receiver) => call };
    drop(run);
    assert!(call.cancel.is_cancelled());
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
    let update = session.poll().unwrap();
    assert_eq!(update.status, TextLookupStatus::Stopped);
    assert!(update.text.is_none());
    assert_eq!(
        session.select(query(2, "B"), provider),
        Err(ResolverError::Cancelled)
    );
    assert!(session.next().await.is_none());
}

#[tokio::test]
async fn stop_before_first_poll_and_repeated_stop_never_start_lookup() {
    let (provider, _receiver) = mock(Duration::ZERO, text);
    let (mut session, task) = ExistingTextSession::new(identity());
    session.select(query(1, "A"), provider.clone()).unwrap();
    session.stop().unwrap();
    session.stop().unwrap();
    task.run().await.unwrap();
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    assert_eq!(session.poll().unwrap().status, TextLookupStatus::Stopped);
    assert_eq!(session.clear(), Err(ResolverError::Cancelled));
}

#[test]
fn foreign_generation_sequence_clock_and_unsupported_sources_do_not_mutate_selection() {
    let (provider, _receiver) = mock(Duration::ZERO, text);
    let (mut session, _task) = ExistingTextSession::new(identity());
    let first = query(10, "A");
    let first_time = first.stamp.ended_at;
    session.select(first, provider.clone()).unwrap();
    let epoch = session.epoch;
    for field in 0..8 {
        let mut q = query(11, "B");
        match field {
            0 => q.stamp.identity.generation += 1,
            1 => q.stamp.identity.session = SessionId(999),
            2 => q.stamp.sequence = 10,
            3 => q.stamp.ended_at = first_time - Duration::from_millis(1),
            4 => q.stamp.ended_at += Duration::from_secs(1),
            5 => q.stamp.ended_at -= MAX_WINDOW_AGE + Duration::from_secs(1),
            6 => q.source = SubtitleSource::LiveAi,
            _ => q.content_id = "bad\0id".into(),
        }
        assert!(session.select(q, provider.clone()).is_err());
        assert_eq!(session.epoch, epoch);
    }
    let (mut unsupported, _receiver) = mock(Duration::ZERO, text);
    Arc::get_mut(&mut unsupported).unwrap().supported = false;
    assert_eq!(
        session.select(query(11, "B"), unsupported),
        Err(ResolverError::UnsupportedCapability)
    );
    assert_eq!(session.epoch, epoch);
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn malformed_confirmation_or_missing_service_clears_old_selection_without_fake_provider() {
    let (provider, _receiver) = mock(Duration::ZERO, text);
    let (mut session, _task) = ExistingTextSession::new(identity());
    session.select(query(1, "A"), provider.clone()).unwrap();
    assert_eq!(
        session.select_confirmed_lyrics(stamp(2), &resolution("B"), None),
        Err(ResolverError::NotConfigured)
    );
    assert!(session.poll().unwrap().text.is_none());
    assert!(session.input.borrow().selection.is_none());
    let mut missing = resolution("B");
    missing.song = None;
    assert_eq!(
        session.select_confirmed_lyrics(stamp(3), &missing, Some(provider.clone())),
        Err(ResolverError::InvalidData)
    );
    let mut weak = resolution("B");
    Arc::get_mut(weak.song.as_mut().unwrap())
        .unwrap()
        .confidence = Confidence::new(100).unwrap();
    assert_eq!(
        session.select_confirmed_lyrics(stamp(4), &weak, Some(provider.clone())),
        Err(ResolverError::InvalidData)
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn malformed_old_session_wrong_content_and_oversized_results_are_rejected() {
    for behavior in [
        (|q: &TextQuery| {
            let mut v = text(q)?.unwrap();
            v.identity.generation += 1;
            Ok(Some(v))
        }) as fn(&TextQuery) -> _,
        (|q: &TextQuery| {
            let mut v = text(q)?.unwrap();
            v.content_id = "other".into();
            Ok(Some(v))
        }) as fn(&TextQuery) -> _,
        (|q: &TextQuery| {
            let mut v = text(q)?.unwrap();
            v.cues[0].duration_us = Some(0);
            Ok(Some(v))
        }) as fn(&TextQuery) -> _,
        (|q: &TextQuery| {
            let mut v = text(q)?.unwrap();
            v.cues = (0..=MAX_SESSION_TEXT_CUES)
                .map(|_| TextCue {
                    start_us: None,
                    duration_us: None,
                    text: "fixture".into(),
                })
                .collect();
            Ok(Some(v))
        }) as fn(&TextQuery) -> _,
        (|q: &TextQuery| {
            let mut v = text(q)?.unwrap();
            v.cues = (0..=MAX_SESSION_TEXT_BYTES / MAX_CUE_BYTES)
                .map(|_| TextCue {
                    start_us: None,
                    duration_us: None,
                    text: "x".repeat(MAX_CUE_BYTES).into(),
                })
                .collect();
            Ok(Some(v))
        }) as fn(&TextQuery) -> _,
    ] {
        let (provider, _receiver) = mock(Duration::ZERO, behavior);
        let (mut session, task) = ExistingTextSession::new(identity());
        session.select(query(1, "A"), provider).unwrap();
        let job = tokio::spawn(task.run());
        let result = wait(
            &mut session,
            TextLookupStatus::Failed(ResolverError::InvalidData),
        )
        .await;
        assert!(result.text.is_none());
        session.stop().unwrap();
        job.await.unwrap().unwrap();
    }
}

#[tokio::test]
async fn selection_lease_expires_on_poll_and_next_without_lookup_or_forever_history() {
    let (provider, _receiver) = mock(Duration::ZERO, text);
    let (mut session, task) = ExistingTextSession::new(identity());
    session.select(query(1, "A"), provider.clone()).unwrap();
    let job = tokio::spawn(task.run());
    let result = wait(&mut session, TextLookupStatus::Available).await;
    let weak = Arc::downgrade(result.text.as_ref().unwrap());
    drop(result);
    session.previous.as_mut().unwrap().ended_at =
        Instant::now() - TEXT_SELECTION_LEASE - Duration::from_millis(1);
    assert_eq!(
        session.poll().unwrap().status,
        TextLookupStatus::Failed(ResolverError::Stale)
    );
    assert!(weak.upgrade().is_none());
    session.select(query(2, "A"), provider.clone()).unwrap();
    wait(&mut session, TextLookupStatus::Available).await;
    session.previous.as_mut().unwrap().ended_at =
        Instant::now() - TEXT_SELECTION_LEASE + Duration::from_millis(20);
    let result = wait(&mut session, TextLookupStatus::Failed(ResolverError::Stale)).await;
    assert!(result.text.is_none() && result.query.is_none());
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    session.stop().unwrap();
    job.await.unwrap().unwrap();
}

#[test]
fn overflow_stops_selection_without_epoch_wrap_or_retaining_pending_private_data() {
    let (provider, _receiver) = mock(Duration::ZERO, text);
    let (mut session, _task) = ExistingTextSession::new(identity());
    session.select(query(1, "A"), provider).unwrap();
    session.epoch = u64::MAX;
    assert_eq!(session.clear(), Err(ResolverError::InvalidData));
    assert!(session.cancel.is_cancelled());
    assert!(session.input.borrow().selection.is_none());
    assert!(session.output.borrow().is_none());
    assert!(session.poll().is_none());
}

#[test]
fn concurrent_clear_and_publication_leave_no_old_text_in_the_shared_output_slot() {
    let (provider, _receiver) = mock(Duration::ZERO, text);
    let (mut session, task) = ExistingTextSession::new(identity());
    let task = Arc::new(task);
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let (sender, receiver) = std::sync::mpsc::sync_channel::<InputFrame>(1);
    std::thread::scope(|scope| {
        let task = Arc::clone(&task);
        let worker_barrier = Arc::clone(&barrier);
        scope.spawn(move || {
            for _ in 0..200 {
                let frame = receiver.recv().unwrap();
                let value = text(&frame.selection.as_ref().unwrap().query)
                    .unwrap()
                    .unwrap();
                worker_barrier.wait();
                task.publish(
                    &frame,
                    TextLookupStatus::Available,
                    Some(ExistingTextBundle {
                        text: value,
                        source_reference: None,
                    }),
                );
                worker_barrier.wait();
            }
        });
        for sequence in 1..=200 {
            session
                .select(query(sequence, "A"), provider.clone())
                .unwrap();
            sender.send(session.input.borrow().clone()).unwrap();
            barrier.wait();
            session.clear().unwrap();
            barrier.wait();
            // Inspect the actual slot, not only the consumer's generation filter.
            let result = session.output.borrow().clone().unwrap();
            assert_eq!(result.status, TextLookupStatus::Cleared);
            assert!(result.text.is_none() && result.query.is_none());
        }
    });
}

fn paired(query: &TextQuery) -> Result<Option<ExistingTextBundle>, ResolverError> {
    let mut primary = text(query)?.ok_or(ResolverError::InvalidData)?;
    primary.language = TextLanguage::TaiwanTraditional;
    primary.cues[0].text = "合成臺灣中文字幕，不含使用者內容。".into();
    Ok(Some(ExistingTextBundle {
        text: primary,
        source_reference: Some(SourceTextReference {
            text: text(query)?.ok_or(ResolverError::InvalidData)?,
            primary_edition_id: "synthetic-edition-a".into(),
            reference_edition_id: "synthetic-edition-a".into(),
        }),
    }))
}
struct BundleProvider {
    behavior: fn(&TextQuery) -> Result<Option<ExistingTextBundle>, ResolverError>,
    delay: Duration,
    calls: AtomicUsize,
    active: Arc<AtomicUsize>,
    entered: tokio::sync::mpsc::Sender<Call>,
}
impl ExistingTextProvider for BundleProvider {
    fn supports(&self, source: SubtitleSource) -> bool {
        matches!(
            source,
            SubtitleSource::ExistingLyrics | SubtitleSource::ExistingSubtitle
        )
    }
    fn lookup<'a>(&'a self, _: &'a TextQuery, _: ResolverCancellation) -> TextFuture<'a> {
        // This fixture explicitly overrides bundle lookup. Falling back to the
        // legacy method would hide the source-reference path under test.
        Box::pin(async { Err(ResolverError::UnsupportedCapability) })
    }
    fn lookup_bundle<'a>(
        &'a self,
        query: &'a TextQuery,
        cancel: ResolverCancellation,
    ) -> TextBundleFuture<'a> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.active.fetch_add(1, Ordering::SeqCst);
            let _active = Active(self.active.clone());
            self.entered
                .try_send(Call {
                    sequence: query.stamp.sequence,
                    cancel,
                })
                .unwrap();
            tokio::time::sleep(self.delay).await;
            (self.behavior)(query)
        })
    }
}
fn bundle_mock(
    delay: Duration,
    behavior: fn(&TextQuery) -> Result<Option<ExistingTextBundle>, ResolverError>,
) -> (Arc<BundleProvider>, tokio::sync::mpsc::Receiver<Call>) {
    let (entered, receiver) = tokio::sync::mpsc::channel(16);
    (
        Arc::new(BundleProvider {
            behavior,
            delay,
            calls: AtomicUsize::new(0),
            active: Arc::new(AtomicUsize::new(0)),
            entered,
        }),
        receiver,
    )
}

#[tokio::test]
async fn default_bundle_lookup_preserves_legacy_success_absence_and_error_with_one_request() {
    for behavior in [
        text as fn(&TextQuery) -> _,
        (|_: &TextQuery| Ok(None)) as fn(&TextQuery) -> _,
        (|_: &TextQuery| Err(ResolverError::Network)) as fn(&TextQuery) -> _,
    ] {
        let (provider, _receiver) = mock(Duration::ZERO, behavior);
        let query = query(1, "A");
        let object: &dyn ExistingTextProvider = provider.as_ref();
        let result = object
            .lookup_bundle(&query, ResolverCancellation::default())
            .await;
        match behavior(&query) {
            Ok(Some(_)) => {
                let bundle = result.unwrap().unwrap();
                assert!(bundle.source_reference.is_none());
                assert_eq!(bundle.text.content_id.as_ref(), "A");
            }
            Ok(None) => assert!(result.unwrap().is_none()),
            Err(error) => assert!(matches!(result, Err(actual) if actual == error)),
        }
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
        assert_eq!(provider.active.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn paired_publication_lease_shares_retirement_and_preserves_immutable_reference_on_renewal() {
    let (provider, mut receiver) = bundle_mock(Duration::ZERO, paired);
    let (mut session, task) = ExistingTextSession::new(identity());
    session.select(query(1, "A"), provider.clone()).unwrap();
    let job = tokio::spawn(task.run());
    let update = wait(&mut session, TextLookupStatus::Available).await;
    let reference = update.source_reference.as_ref().unwrap();
    let weak_reference = Arc::downgrade(reference);
    let weak_primary = Arc::downgrade(update.text.as_ref().unwrap());
    let mut lease = ExistingTextLease::from_update(&update).unwrap();
    lease
        .renew(stamp(1), Confidence::new(900).unwrap())
        .unwrap();
    let original = lease.clone();
    lease
        .renew(stamp(2), Confidence::new(900).unwrap())
        .unwrap();
    assert_eq!(original.query().stamp.sequence, 1);
    assert_eq!(lease.query().stamp.sequence, 2);
    assert!(std::ptr::eq(
        original.source_reference().unwrap(),
        lease.source_reference().unwrap()
    ));
    assert!(std::ptr::eq(
        reference.as_ref(),
        lease.source_reference().unwrap()
    ));
    assert_eq!(
        lease.source_reference().unwrap().text.content_id,
        lease.text().content_id
    );
    assert!(entered(&mut receiver).await.cancel.is_cancelled());
    assert!(!session.select(query(2, "A"), provider.clone()).unwrap());
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    session.clear().unwrap();
    let cleared = session.poll().unwrap();
    assert!(cleared.text.is_none() && cleared.source_reference.is_none());
    // Production LiveLyrics retires this one shared parent when it clears text.
    lease.retire();
    assert_eq!(lease.validate(identity()), Err(ResolverError::Cancelled));
    assert_eq!(original.validate(identity()), Err(ResolverError::Cancelled));
    drop(original);
    drop(lease);
    drop(update);
    assert!(weak_reference.upgrade().is_none());
    assert!(weak_primary.upgrade().is_none());
    session.stop().unwrap();
    job.await.unwrap().unwrap();
}

#[test]
fn paired_contract_requires_chinese_primary_known_source_language_identity_and_equal_edit() {
    let query = query(1, "A");
    for language in [
        TextLanguage::TaiwanTraditional,
        TextLanguage::OtherTraditional,
        TextLanguage::SimplifiedChinese,
    ] {
        for source in [
            SpeechLanguage::English,
            SpeechLanguage::Japanese,
            SpeechLanguage::French,
            SpeechLanguage::German,
            SpeechLanguage::Korean,
        ] {
            let mut bundle = paired(&query).unwrap().unwrap();
            bundle.text.language = language;
            bundle.source_reference.as_mut().unwrap().text.language = TextLanguage::Source(source);
            bundle.validate(&query).unwrap();
        }
    }
    for mutate in [
        (|bundle: &mut ExistingTextBundle| {
            bundle.text.language = TextLanguage::Source(SpeechLanguage::English)
        }) as fn(&mut ExistingTextBundle),
        (|bundle: &mut ExistingTextBundle| {
            bundle.source_reference.as_mut().unwrap().text.language =
                TextLanguage::TaiwanTraditional
        }) as fn(&mut ExistingTextBundle),
        (|bundle: &mut ExistingTextBundle| {
            bundle
                .source_reference
                .as_mut()
                .unwrap()
                .text
                .identity
                .generation += 1
        }) as fn(&mut ExistingTextBundle),
        (|bundle: &mut ExistingTextBundle| {
            bundle
                .source_reference
                .as_mut()
                .unwrap()
                .text
                .identity
                .session = SessionId(999)
        }) as fn(&mut ExistingTextBundle),
        (|bundle: &mut ExistingTextBundle| {
            bundle.source_reference.as_mut().unwrap().text.source = SubtitleSource::ExistingSubtitle
        }) as fn(&mut ExistingTextBundle),
        (|bundle: &mut ExistingTextBundle| {
            bundle.source_reference.as_mut().unwrap().text.content_id = "different-content".into()
        }) as fn(&mut ExistingTextBundle),
        (|bundle: &mut ExistingTextBundle| {
            bundle
                .source_reference
                .as_mut()
                .unwrap()
                .reference_edition_id = "synthetic-edition-b".into()
        }) as fn(&mut ExistingTextBundle),
        (|bundle: &mut ExistingTextBundle| {
            bundle.source_reference.as_mut().unwrap().primary_edition_id = "".into()
        }) as fn(&mut ExistingTextBundle),
        (|bundle: &mut ExistingTextBundle| {
            let reference = bundle.source_reference.as_mut().unwrap();
            reference.primary_edition_id = "bad\0edit".into();
            reference.reference_edition_id = "bad\0edit".into();
        }) as fn(&mut ExistingTextBundle),
        (|bundle: &mut ExistingTextBundle| {
            let reference = bundle.source_reference.as_mut().unwrap();
            reference.primary_edition_id = "x".repeat(MAX_METADATA_BYTES + 1).into();
            reference.reference_edition_id = "x".repeat(MAX_METADATA_BYTES + 1).into();
        }) as fn(&mut ExistingTextBundle),
        (|bundle: &mut ExistingTextBundle| {
            bundle.source_reference.as_mut().unwrap().text.cues[0].duration_us = Some(0)
        }) as fn(&mut ExistingTextBundle),
        (|bundle: &mut ExistingTextBundle| {
            bundle.source_reference.as_mut().unwrap().text.cues[0].text = "bad\0text".into()
        }) as fn(&mut ExistingTextBundle),
        (|bundle: &mut ExistingTextBundle| bundle.text.cues[0].duration_us = None)
            as fn(&mut ExistingTextBundle),
    ] {
        let mut bundle = paired(&query).unwrap().unwrap();
        mutate(&mut bundle);
        assert_eq!(bundle.validate(&query), Err(ResolverError::InvalidData));
    }
}

#[test]
fn paired_budget_counts_both_tracks_together_and_retains_unpaired_limits() {
    let query = query(1, "A");
    let cues = |count: usize, bytes: usize| {
        (0..count)
            .map(|index| TextCue {
                start_us: Some(index as u64 * 1_000_000),
                duration_us: Some(1_000_000),
                text: "x".repeat(bytes).into(),
            })
            .collect::<Vec<_>>()
    };
    let mut bundle = paired(&query).unwrap().unwrap();
    bundle.text.cues = cues(MAX_SESSION_TEXT_CUES / 2, 1);
    bundle.source_reference.as_mut().unwrap().text.cues = cues(MAX_SESSION_TEXT_CUES / 2, 1);
    validate_bundle_budget(&bundle.text, bundle.source_reference.as_ref()).unwrap();
    bundle
        .source_reference
        .as_mut()
        .unwrap()
        .text
        .cues
        .push(TextCue {
            start_us: Some(1_000_000_000),
            duration_us: Some(1_000_000),
            text: "x".into(),
        });
    // Each track alone is within budget; their combined cue count is rejected.
    validate_text_budget(&bundle.text).unwrap();
    validate_text_budget(&bundle.source_reference.as_ref().unwrap().text).unwrap();
    assert_eq!(
        validate_bundle_budget(&bundle.text, bundle.source_reference.as_ref()),
        Err(ResolverError::InvalidData)
    );
    let half = MAX_SESSION_TEXT_BYTES / MAX_CUE_BYTES / 2;
    bundle.text.cues = cues(half, MAX_CUE_BYTES);
    bundle.source_reference.as_mut().unwrap().text.cues = cues(half, MAX_CUE_BYTES);
    bundle.validate(&query).unwrap();
    validate_bundle_budget(&bundle.text, bundle.source_reference.as_ref()).unwrap();
    bundle
        .source_reference
        .as_mut()
        .unwrap()
        .text
        .cues
        .push(TextCue {
            start_us: Some(1_000_000_000),
            duration_us: Some(1_000_000),
            text: "x".into(),
        });
    assert_eq!(
        validate_bundle_budget(&bundle.text, bundle.source_reference.as_ref()),
        Err(ResolverError::InvalidData)
    );
    bundle.source_reference = None;
    bundle.text.cues = cues(MAX_SESSION_TEXT_CUES, 1);
    validate_bundle_budget(&bundle.text, None).unwrap();
    bundle.text.cues = cues(MAX_SESSION_TEXT_BYTES / MAX_CUE_BYTES, MAX_CUE_BYTES);
    validate_bundle_budget(&bundle.text, None).unwrap();
}

#[test]
fn cancellation_scope_compares_parent_identity_instead_of_equal_current_state() {
    let parent = ResolverCancellation::default();
    let same = parent.clone();
    let different = ResolverCancellation::default();
    assert!(parent.same_scope(&same));
    assert!(!parent.same_scope(&different));
    parent.cancel();
    different.cancel();
    assert!(parent.same_scope(&same));
    assert!(!parent.same_scope(&different));
}

#[tokio::test]
async fn invalid_or_over_budget_bundle_is_rejected_atomically_before_publication() {
    for behavior in [
        (|query: &TextQuery| {
            let mut bundle = paired(query)?.unwrap();
            bundle
                .source_reference
                .as_mut()
                .unwrap()
                .reference_edition_id = "other-edit".into();
            Ok(Some(bundle))
        }) as fn(&TextQuery) -> _,
        (|query: &TextQuery| {
            let mut bundle = paired(query)?.unwrap();
            bundle.source_reference.as_mut().unwrap().text.cues = (0..MAX_SESSION_TEXT_CUES)
                .map(|index| TextCue {
                    start_us: Some(index as u64),
                    duration_us: Some(1),
                    text: "fixture".into(),
                })
                .collect();
            Ok(Some(bundle))
        }) as fn(&TextQuery) -> _,
    ] {
        let (provider, _receiver) = bundle_mock(Duration::ZERO, behavior);
        let (mut session, task) = ExistingTextSession::new(identity());
        session.select(query(1, "A"), provider).unwrap();
        let job = tokio::spawn(task.run());
        let update = wait(
            &mut session,
            TextLookupStatus::Failed(ResolverError::InvalidData),
        )
        .await;
        assert!(update.text.is_none() && update.source_reference.is_none());
        session.stop().unwrap();
        job.await.unwrap().unwrap();
    }
}

#[tokio::test]
async fn cancelled_old_pair_never_replaces_new_publication_or_releases_only_one_track() {
    let (provider, mut receiver) = bundle_mock(Duration::from_millis(40), paired);
    let (mut session, task) = ExistingTextSession::new(identity());
    session.select(query(1, "old"), provider.clone()).unwrap();
    let job = tokio::spawn(task.run());
    let old = entered(&mut receiver).await;
    session.select(query(2, "new"), provider.clone()).unwrap();
    assert!(old.cancel.is_cancelled());
    let update = wait(&mut session, TextLookupStatus::Available).await;
    assert_eq!(update.text.as_ref().unwrap().content_id.as_ref(), "new");
    assert_eq!(
        update
            .source_reference
            .as_ref()
            .unwrap()
            .text
            .content_id
            .as_ref(),
        "new"
    );
    assert_eq!(update.query.as_ref().unwrap().stamp.sequence, 2);
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
    let weak_primary = Arc::downgrade(update.text.as_ref().unwrap());
    let weak_reference = Arc::downgrade(update.source_reference.as_ref().unwrap());
    drop(update);
    session.clear().unwrap();
    assert!(weak_primary.upgrade().is_none() && weak_reference.upgrade().is_none());
    let cleared = session.poll().unwrap();
    assert!(cleared.text.is_none() && cleared.source_reference.is_none());
    session.stop().unwrap();
    job.await.unwrap().unwrap();
}

#[tokio::test]
async fn dropping_pair_lookup_task_cancels_request_and_clears_both_tracks() {
    let (provider, mut receiver) = bundle_mock(Duration::from_secs(30), paired);
    let (mut session, task) = ExistingTextSession::new(identity());
    session.select(query(1, "A"), provider.clone()).unwrap();
    let mut work = Box::pin(task.run());
    let call = tokio::select! { _ = work.as_mut() => panic!("unexpected task completion"), call = entered(&mut receiver) => call };
    drop(work);
    assert!(call.cancel.is_cancelled());
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
    let stopped = session.poll().unwrap();
    assert_eq!(stopped.status, TextLookupStatus::Stopped);
    assert!(stopped.text.is_none() && stopped.source_reference.is_none());
}
