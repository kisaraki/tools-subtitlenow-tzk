//! Synthetic ASR, clock and lookup; real latest-input worker and shared timeline.
use super::*;
use crate::{
    ai::{
        speech::SpeechLanguage,
        translation::{
            TranslationCancellation, TranslationCapabilities, TranslationEmitter, TranslationError,
            TranslationFuture, TranslationPass, TranslationRequest,
        },
    },
    audio::{
        capture::{AudioStamp, BLOCK_FRAMES, SessionId},
        preprocess::NormalizedAudio,
    },
    resolver::{
        contracts::{
            Confidence, ExistingText, ExistingTextProvider, SubtitleSource, TextCue, TextFuture,
            TextLanguage, TextQuery, WindowStamp,
        },
        text_session::{ExistingTextSession, TextLookupStatus},
    },
    session::playback_clock::LivePlaybackClock,
    ui::{
        content::{ContentDisclosure, ContentTransport},
        lyrics::{UiLyricsState, UiLyricsStatus},
        presentation::TimelineSnapshot,
    },
};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(940),
        generation: 1,
    }
}
fn stamp(sequence: u64) -> WindowStamp {
    WindowStamp {
        identity: identity(),
        sequence,
        ended_at: Instant::now(),
        start_us: Some(40_000_000),
        duration_us: 12_000_000,
    }
}
fn event(sequence: u64) -> TranscriptEvent {
    TranscriptEvent {
        identity: identity(),
        sequence,
        utterance: sequence,
        revision: 1,
        kind: TranscriptKind::Final,
        language: Some(SpeechLanguage::English),
        start_us: Some(42_000_000 + (sequence - 1) * 1_000_000),
        duration_us: Some(950_000),
        text: if sequence == 1 {
            "synthetic source one"
        } else {
            "synthetic source two"
        }
        .into(),
    }
}
struct MockLookup;
impl ExistingTextProvider for MockLookup {
    fn supports(&self, source: SubtitleSource) -> bool {
        source == SubtitleSource::ExistingLyrics
    }
    fn lookup<'a>(&'a self, query: &'a TextQuery, _: ResolverCancellation) -> TextFuture<'a> {
        Box::pin(async move {
            Ok(Some(ExistingText {
                identity: query.stamp.identity,
                source: query.source,
                content_id: query.content_id.clone(),
                language: TextLanguage::Source(SpeechLanguage::English),
                cues: ["synthetic source one", "synthetic source two"]
                    .into_iter()
                    .enumerate()
                    .map(|(index, text)| TextCue {
                        start_us: Some(5_000_000 + index as u64 * 1_000_000),
                        duration_us: Some(1_000_000),
                        text: text.into(),
                    })
                    .collect(),
            }))
        })
    }
}
async fn lease() -> ExistingTextLease {
    let initial = stamp(1);
    let (mut owner, task) = ExistingTextSession::new(identity());
    owner
        .select(
            TextQuery {
                stamp: initial,
                source: SubtitleSource::ExistingLyrics,
                content_id: "synthetic work".into(),
            },
            Arc::new(MockLookup),
        )
        .unwrap();
    let worker = task.run();
    tokio::pin!(worker);
    let mut input = loop {
        tokio::select! {
            result = &mut worker => panic!("mock lookup ended {result:?}"),
            update = owner.next() => { let update = update.unwrap(); if update.status == TextLookupStatus::Available { break ExistingTextLease::from_update(&update).unwrap(); } },
        }
    };
    input.renew(initial, Confidence::new(900).unwrap()).unwrap();
    owner.stop().unwrap();
    worker.await.unwrap();
    input
}
struct MockTranslator {
    pending: bool,
    error: bool,
    calls: AtomicUsize,
    active: Arc<AtomicUsize>,
    started: mpsc::Sender<TranslationCancellation>,
    passes: Mutex<Vec<(TranslationQuality, TranslationPass, usize)>>,
}
struct Active(Arc<AtomicUsize>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
fn provider(
    pending: bool,
    error: bool,
) -> (MockTranslator, mpsc::Receiver<TranslationCancellation>) {
    let (started, receiver) = mpsc::channel(4);
    (
        MockTranslator {
            pending,
            error,
            calls: AtomicUsize::new(0),
            active: Arc::new(AtomicUsize::new(0)),
            started,
            passes: Mutex::new(vec![]),
        },
        receiver,
    )
}
impl Translator for MockTranslator {
    fn capabilities(&self) -> TranslationCapabilities {
        TranslationCapabilities {
            streaming: true,
            cancellation: true,
            supported_languages: [true; 5],
            max_concurrent: 2,
        }
    }
    fn translate<'a>(
        &'a self,
        request: &'a TranslationRequest,
        _: &'a mut TranslationEmitter,
        cancel: TranslationCancellation,
    ) -> TranslationFuture<'a> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.active.fetch_add(1, Ordering::SeqCst);
            let _active = Active(self.active.clone());
            self.passes.lock().unwrap().push((
                request.quality,
                request.pass,
                request.previous_stable.len(),
            ));
            let _ = self.started.try_send(cancel);
            if self.error {
                return Err(TranslationError::Network);
            }
            if self.pending {
                return std::future::pending().await;
            }
            Ok("合成軟體譯文".into())
        })
    }
}
async fn completion<F: Future<Output = Result<(), TextPreparationError>>>(
    owner: &mut LiveTextPipeline,
    worker: &mut Pin<&mut F>,
) -> TextCompletion {
    tokio::time::timeout(Duration::from_secs(2), async { loop {
        tokio::select! { result = worker.as_mut() => panic!("text task ended {result:?}"), result = owner.changed() => if let Some(done) = result { return done; } }
    }}).await.unwrap()
}
fn observation(flow: &mut LiveTranslation<'_>, position: u64) -> PlaybackObservation {
    PlaybackObservation {
        version: flow.sources_mut().version(),
        sequence: 100,
        observed_at: Instant::now(),
        capture_position_us: position,
    }
}
#[tokio::test]
async fn scheduler_matches_prepares_adopts_and_publishes_shared_snapshot_for_all_qualities() {
    let _serial = SERIAL.lock().await;
    for quality in [
        TranslationQuality::Fast,
        TranslationQuality::Standard,
        TranslationQuality::High,
    ] {
        let input = lease().await;
        let (provider, _) = provider(false, false);
        let mut flow = LiveTranslation::new(identity(), quality, &provider).unwrap();
        flow.ingest(event(1)).unwrap();
        let (mut owner, task) = LiveTextPipeline::new(identity(), quality);
        assert!(
            owner
                .offer(
                    Some(input.clone()),
                    Some(&event(1)),
                    flow.sources_mut().version()
                )
                .unwrap()
        );
        let work = task.run(&provider);
        tokio::pin!(work);
        let done = completion(&mut owner, &mut work).await;
        let obs = observation(&mut flow, 42_500_000);
        assert!(owner.apply(done, &mut flow, Some(obs)).unwrap());
        assert_eq!(owner.status(), TextPipelineStatus::Ready);
        assert_eq!(flow.timeline().items().len(), 1);
        assert_eq!(
            flow.timeline().items()[0].display_origin(),
            SubtitleSource::ExistingLyrics
        );
        let snapshot = TimelineSnapshot::from_timeline(flow.timeline(), 1);
        let (mut publisher, mut inbox) = crate::ui::bridge::channel();
        publisher
            .begin(
                identity(),
                crate::ui::bridge::UiSource {
                    pid: 12345,
                    provider: crate::ai::provider::SpeechProvider::OpenAi,
                },
            )
            .unwrap();
        publisher.publish(flow.timeline(), true).unwrap();
        assert!(inbox.poll().unwrap().status == crate::ui::bridge::UiStatus::UsingExistingText);
        assert_eq!(
            snapshot.latest_origin(),
            Some(SubtitleSource::ExistingLyrics)
        );
        assert_eq!(
            provider.calls.load(Ordering::SeqCst),
            if quality == TranslationQuality::High {
                2
            } else {
                1
            }
        );
        assert!(
            !owner
                .offer(
                    Some(input.clone()),
                    Some(&event(1)),
                    flow.sources_mut().version()
                )
                .unwrap()
        );
        input.retire();
        assert_eq!(
            flow.timeline().items()[0].display_origin(),
            SubtitleSource::LiveAi
        );
        owner.stop();
        work.await.unwrap();
    }
}
#[tokio::test]
async fn past_or_unknown_playback_position_never_retroactively_adopts() {
    let _serial = SERIAL.lock().await;
    for position in [None, Some(43_000_000), Some(41_999_999)] {
        let input = lease().await;
        let (provider, _) = provider(false, false);
        let mut flow =
            LiveTranslation::new(identity(), TranslationQuality::Standard, &provider).unwrap();
        flow.ingest(event(1)).unwrap();
        let (mut owner, task) = LiveTextPipeline::new(identity(), TranslationQuality::Standard);
        owner
            .offer(Some(input), Some(&event(1)), flow.sources_mut().version())
            .unwrap();
        let work = task.run(&provider);
        tokio::pin!(work);
        let done = completion(&mut owner, &mut work).await;
        let obs = position.map(|position| observation(&mut flow, position));
        assert!(!owner.apply(done, &mut flow, obs).unwrap());
        assert_eq!(owner.status(), TextPipelineStatus::Unmatched);
        assert_eq!(flow.sources_mut().candidate_count(), 0);
        owner.stop();
        work.await.unwrap();
    }
}
#[tokio::test]
async fn no_unique_match_or_provider_error_keeps_live_available_and_has_safe_status() {
    let _serial = SERIAL.lock().await;
    for provider_error in [false, true] {
        let input = lease().await;
        let (provider, _) = provider(false, provider_error);
        let mut flow =
            LiveTranslation::new(identity(), TranslationQuality::Standard, &provider).unwrap();
        let mut source = event(1);
        if !provider_error {
            source.text = "unmatched synthetic words".into();
        }
        flow.ingest(source.clone()).unwrap();
        let (mut owner, task) = LiveTextPipeline::new(identity(), TranslationQuality::Standard);
        owner
            .offer(Some(input), Some(&source), flow.sources_mut().version())
            .unwrap();
        let work = task.run(&provider);
        tokio::pin!(work);
        let done = completion(&mut owner, &mut work).await;
        assert!(owner.apply(done, &mut flow, None).is_err());
        assert!(flow.cancellation().reason().is_none());
        assert_eq!(
            provider.calls.load(Ordering::SeqCst),
            usize::from(provider_error)
        );
        assert_eq!(
            owner.status(),
            if provider_error {
                TextPipelineStatus::Failed(
                    TextPreparationError::Preparation(
                        crate::subtitle::source_translation::SourceTranslationError::Translation(
                            TranslationError::Network,
                        ),
                    )
                    .into(),
                )
            } else {
                TextPipelineStatus::Unmatched
            }
        );
        owner.stop();
        work.await.unwrap();
    }
}
#[tokio::test]
async fn cancelled_wait_preserves_job_but_clear_and_drop_release_active_cloud_request() {
    let _serial = SERIAL.lock().await;
    for stop in [false, true] {
        let input = lease().await;
        let (provider, mut started) = provider(true, false);
        let mut flow =
            LiveTranslation::new(identity(), TranslationQuality::Standard, &provider).unwrap();
        let (mut owner, task) = LiveTextPipeline::new(identity(), TranslationQuality::Standard);
        owner
            .offer(
                Some(input.clone()),
                Some(&event(1)),
                flow.sources_mut().version(),
            )
            .unwrap();
        let work = task.run(&provider);
        tokio::pin!(work);
        let cancellation = tokio::select! { token = started.recv() => token.unwrap(), _ = &mut work => panic!("pending provider ended") };
        assert_eq!(provider.active.load(Ordering::SeqCst), 1);
        // Cancelling a UI wait must not cancel persistent work.
        assert!(
            tokio::time::timeout(Duration::from_millis(5), async {
                loop {
                    owner.changed().await;
                }
            })
            .await
            .is_err()
        );
        assert_eq!(provider.active.load(Ordering::SeqCst), 1);
        if stop {
            owner.stop();
            work.await.unwrap();
        } else {
            owner.clear();
            tokio::select! { _ = cancellation.cancelled() => {}, _ = &mut work => panic!("clear must not stop task") };
            owner.stop();
            work.await.unwrap();
        }
        assert!(cancellation.reason().is_some());
        assert_eq!(provider.active.load(Ordering::SeqCst), 0);
        assert!(input.validate(identity()).is_ok());
    }
}
#[tokio::test]
async fn newer_final_cancels_old_request_without_reusing_same_query_then_latest_runs() {
    let _serial = SERIAL.lock().await;
    let mut input = lease().await;
    let (provider, mut started) = provider(true, false);
    let mut flow =
        LiveTranslation::new(identity(), TranslationQuality::Standard, &provider).unwrap();
    let (mut owner, task) = LiveTextPipeline::new(identity(), TranslationQuality::Standard);
    owner
        .offer(
            Some(input.clone()),
            Some(&event(1)),
            flow.sources_mut().version(),
        )
        .unwrap();
    let work = task.run(&provider);
    tokio::pin!(work);
    let first = tokio::select! { token = started.recv() => token.unwrap(), _ = &mut work => panic!("pending provider ended") };
    assert!(
        !owner
            .offer(
                Some(input.clone()),
                Some(&event(2)),
                flow.sources_mut().version()
            )
            .unwrap()
    );
    tokio::select! { _ = first.cancelled() => {}, _ = &mut work => panic!("replacement stopped task") };
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    input
        .renew(stamp(2), Confidence::new(900).unwrap())
        .unwrap();
    assert!(
        owner
            .offer(Some(input), Some(&event(2)), flow.sources_mut().version())
            .unwrap()
    );
    let second = tokio::select! { token = started.recv() => token.unwrap(), _ = &mut work => panic!("replacement stopped task") };
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    assert_eq!(provider.active.load(Ordering::SeqCst), 1);
    owner.stop();
    work.await.unwrap();
    assert!(second.reason().is_some());
}
#[tokio::test]
async fn late_completed_result_rejected_after_new_input_and_latest_result_is_bounded() {
    let _serial = SERIAL.lock().await;
    let mut input = lease().await;
    let (provider, _) = provider(false, false);
    let mut flow =
        LiveTranslation::new(identity(), TranslationQuality::Standard, &provider).unwrap();
    flow.ingest(event(1)).unwrap();
    flow.ingest(event(2)).unwrap();
    let (mut owner, task) = LiveTextPipeline::new(identity(), TranslationQuality::Standard);
    owner
        .offer(
            Some(input.clone()),
            Some(&event(1)),
            flow.sources_mut().version(),
        )
        .unwrap();
    let work = task.run(&provider);
    tokio::pin!(work);
    let old = completion(&mut owner, &mut work).await;
    input
        .renew(stamp(2), Confidence::new(900).unwrap())
        .unwrap();
    owner
        .offer(Some(input), Some(&event(2)), flow.sources_mut().version())
        .unwrap();
    assert!(!owner.apply(old, &mut flow, None).unwrap());
    assert_eq!(owner.output.max_capacity(), 1);
    let latest = completion(&mut owner, &mut work).await;
    let obs = observation(&mut flow, 43_500_000);
    assert!(owner.apply(latest, &mut flow, Some(obs)).unwrap());
    assert_eq!(
        flow.timeline().items()[0].display_origin(),
        SubtitleSource::LiveAi
    );
    assert_eq!(
        flow.timeline().items()[1]
            .existing_display()
            .unwrap()
            .document_cue_index(),
        1
    );
    owner.stop();
    work.await.unwrap();
}
#[tokio::test]
async fn fresh_confirmations_do_not_restart_pending_same_cue_or_extend_original_request() {
    let _serial = SERIAL.lock().await;
    let mut input = lease().await;
    let original = input.query().stamp.ended_at;
    let (provider, mut started) = provider(true, false);
    let mut flow =
        LiveTranslation::new(identity(), TranslationQuality::Standard, &provider).unwrap();
    let (mut owner, task) = LiveTextPipeline::new(identity(), TranslationQuality::Standard);
    owner
        .offer(
            Some(input.clone()),
            Some(&event(1)),
            flow.sources_mut().version(),
        )
        .unwrap();
    let work = task.run(&provider);
    tokio::pin!(work);
    let token = tokio::select! { token = started.recv() => token.unwrap(), _ = &mut work => panic!("pending provider ended") };
    input
        .renew(stamp(2), Confidence::new(900).unwrap())
        .unwrap();
    assert!(
        !owner
            .offer(Some(input), Some(&event(1)), flow.sources_mut().version())
            .unwrap()
    );
    assert_eq!(
        owner.active.as_ref().unwrap().input.query().stamp.ended_at,
        original
    );
    assert!(token.reason().is_none());
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    owner.stop();
    work.await.unwrap();
}
#[tokio::test]
async fn invalid_epoch_withdrawn_lookup_foreign_session_partial_and_ticket_overflow_reject() {
    let _serial = SERIAL.lock().await;
    let input = lease().await;
    let (provider, _) = provider(false, false);
    let mut flow =
        LiveTranslation::new(identity(), TranslationQuality::Standard, &provider).unwrap();
    flow.ingest(event(1)).unwrap();
    let (mut owner, task) = LiveTextPipeline::new(identity(), TranslationQuality::Standard);
    let mut partial = event(1);
    partial.kind = TranscriptKind::Partial;
    assert!(
        !owner
            .offer(
                Some(input.clone()),
                Some(&partial),
                flow.sources_mut().version()
            )
            .unwrap()
    );
    partial = event(1);
    partial.identity.generation += 1;
    assert!(
        owner
            .offer(
                Some(input.clone()),
                Some(&partial),
                flow.sources_mut().version()
            )
            .is_err()
    );
    let mut oversized = event(1);
    oversized.text = "x"
        .repeat(crate::ai::speech::MAX_TRANSCRIPT_BYTES + 1)
        .into();
    assert!(
        owner
            .offer(
                Some(input.clone()),
                Some(&oversized),
                flow.sources_mut().version()
            )
            .is_err()
    );
    owner
        .offer(
            Some(input.clone()),
            Some(&event(1)),
            flow.sources_mut().version(),
        )
        .unwrap();
    let mut old = event(1);
    old.sequence = 0;
    assert!(
        owner
            .offer(
                Some(input.clone()),
                Some(&old),
                flow.sources_mut().version()
            )
            .is_err()
    );
    assert!(owner.busy);
    assert!(!owner.active.as_ref().unwrap().cancel.is_cancelled());
    let work = task.run(&provider);
    tokio::pin!(work);
    let done = completion(&mut owner, &mut work).await;
    flow.sources_mut().invalidate().unwrap();
    assert!(owner.apply(done, &mut flow, None).is_err());
    input.retire();
    assert!(
        owner
            .offer(Some(input), Some(&event(1)), flow.sources_mut().version())
            .is_err()
    );
    owner.clear();
    owner.ticket = u64::MAX;
    assert!(
        owner
            .offer(
                Some(lease().await),
                Some(&event(1)),
                flow.sources_mut().version()
            )
            .is_err()
    );
    owner.stop();
    work.await.unwrap();
}
#[tokio::test]
async fn successful_preparation_is_not_repeated_by_later_confirmations() {
    let _serial = SERIAL.lock().await;
    let mut input = lease().await;
    let (provider, _) = provider(false, false);
    let mut flow =
        LiveTranslation::new(identity(), TranslationQuality::Standard, &provider).unwrap();
    flow.ingest(event(1)).unwrap();
    let (mut owner, task) = LiveTextPipeline::new(identity(), TranslationQuality::Standard);
    owner
        .offer(
            Some(input.clone()),
            Some(&event(1)),
            flow.sources_mut().version(),
        )
        .unwrap();
    let work = task.run(&provider);
    tokio::pin!(work);
    let done = completion(&mut owner, &mut work).await;
    assert!(!owner.apply(done, &mut flow, None).unwrap());
    for sequence in 2..5 {
        owner.clear();
        input
            .renew(stamp(sequence), Confidence::new(900).unwrap())
            .unwrap();
        assert!(
            !owner
                .offer(
                    Some(input.clone()),
                    Some(&event(1)),
                    flow.sources_mut().version()
                )
                .unwrap()
        );
    }
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    owner.stop();
    work.await.unwrap();
}
#[tokio::test]
async fn slow_consumer_discards_unread_old_result_and_keeps_only_latest_replacement() {
    let _serial = SERIAL.lock().await;
    let mut input = lease().await;
    let (provider, _) = provider(false, false);
    let mut flow =
        LiveTranslation::new(identity(), TranslationQuality::Standard, &provider).unwrap();
    flow.ingest(event(1)).unwrap();
    let (mut owner, task) = LiveTextPipeline::new(identity(), TranslationQuality::Standard);
    owner
        .offer(
            Some(input.clone()),
            Some(&event(1)),
            flow.sources_mut().version(),
        )
        .unwrap();
    let work = task.run(&provider);
    tokio::pin!(work);
    tokio::time::timeout(Duration::from_secs(2), async {
        while owner.output.is_empty() {
            tokio::select! { _ = &mut work => panic!("task ended while UI slow"), _ = tokio::time::sleep(Duration::from_millis(5)) => {} }
        }
    }).await.unwrap();
    assert_eq!(owner.output.len(), 1);
    for sequence in 2..9 {
        input
            .renew(stamp(sequence), Confidence::new(900).unwrap())
            .unwrap();
        owner
            .offer(
                Some(input.clone()),
                Some(&event(sequence)),
                flow.sources_mut().version(),
            )
            .unwrap();
    }
    assert!(owner.output.is_empty());
    assert_eq!(owner.input.borrow().as_ref().unwrap().event.sequence, 8);
    flow.ingest(event(8)).unwrap();
    let done = completion(&mut owner, &mut work).await;
    let obs = observation(&mut flow, 49_500_000);
    assert!(owner.apply(done, &mut flow, Some(obs)).unwrap());
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    assert_eq!(provider.passes.lock().unwrap()[1].2, 1);
    owner.stop();
    work.await.unwrap();
}
#[tokio::test]
async fn explicit_withdrawal_clears_context_even_when_none_is_replaced_before_task_poll() {
    let _serial = SERIAL.lock().await;
    let mut input = lease().await;
    let (provider, _) = provider(false, false);
    let mut flow =
        LiveTranslation::new(identity(), TranslationQuality::Standard, &provider).unwrap();
    let (mut owner, task) = LiveTextPipeline::new(identity(), TranslationQuality::Standard);
    owner
        .offer(
            Some(input.clone()),
            Some(&event(1)),
            flow.sources_mut().version(),
        )
        .unwrap();
    let work = task.run(&provider);
    tokio::pin!(work);
    let first = completion(&mut owner, &mut work).await;
    drop(first);
    owner.clear();
    input
        .renew(stamp(2), Confidence::new(900).unwrap())
        .unwrap();
    owner
        .offer(Some(input), Some(&event(2)), flow.sources_mut().version())
        .unwrap();
    let second = completion(&mut owner, &mut work).await;
    drop(second);
    assert_eq!(
        provider
            .passes
            .lock()
            .unwrap()
            .iter()
            .map(|pass| pass.2)
            .collect::<Vec<_>>(),
        vec![0, 0]
    );
    owner.stop();
    work.await.unwrap();
}
#[tokio::test]
async fn dropping_caller_owned_task_releases_provider_and_invalidates_prepared_display() {
    let _serial = SERIAL.lock().await;
    for pending in [true, false] {
        let input = lease().await;
        let (provider, mut started) = provider(pending, false);
        let mut flow =
            LiveTranslation::new(identity(), TranslationQuality::Standard, &provider).unwrap();
        flow.ingest(event(1)).unwrap();
        let (mut owner, task) = LiveTextPipeline::new(identity(), TranslationQuality::Standard);
        owner
            .offer(
                Some(input.clone()),
                Some(&event(1)),
                flow.sources_mut().version(),
            )
            .unwrap();
        let mut work = Box::pin(task.run(&provider));
        if pending {
            let token = tokio::select! { token = started.recv() => token.unwrap(), _ = work.as_mut() => panic!("pending provider ended") };
            drop(work);
            assert!(token.reason().is_some());
        } else {
            let done = completion(&mut owner, &mut work.as_mut()).await;
            let mut clock = LivePlaybackClock::new(identity());
            clock.observe(&audio(100, Some(2_040_000)));
            let obs = clock.observation(flow.sources_mut().version());
            assert!(owner.apply(done, &mut flow, obs).unwrap());
            drop(work);
            assert_eq!(
                flow.timeline().items()[0].display_origin(),
                SubtitleSource::LiveAi
            );
        }
        assert_eq!(provider.active.load(Ordering::SeqCst), 0);
        assert!(input.validate(identity()).is_ok());
        assert!(
            tokio::time::timeout(Duration::from_millis(5), owner.changed())
                .await
                .is_err()
        );
    }
}
fn audio(sequence: u64, frame: Option<u64>) -> NormalizedAudio {
    NormalizedAudio {
        stamp: AudioStamp {
            session: identity().session,
            sequence,
            frame_position: frame,
            qpc_100ns: None,
            received: Instant::now(),
            discontinuity: false,
        },
        frames: BLOCK_FRAMES,
        samples: [0.0; BLOCK_FRAMES],
    }
}
#[test]
fn capture_cursor_uses_frame_position_without_wall_clock_extrapolation() {
    let (provider, _) = provider(false, false);
    let mut flow =
        LiveTranslation::new(identity(), TranslationQuality::Standard, &provider).unwrap();
    let mut clock = LivePlaybackClock::new(identity());
    let block = audio(1, Some(2_040_000));
    assert!(!clock.observe(&block));
    let obs = clock.observation(flow.sources_mut().version()).unwrap();
    assert_eq!(obs.capture_position_us, 42_520_000);
    assert_eq!(obs.observed_at, block.stamp.received);
    assert_eq!(obs.sequence, 1);
    assert_eq!(
        clock
            .observation(flow.sources_mut().version())
            .unwrap()
            .capture_position_us,
        obs.capture_position_us
    );
}
#[test]
fn capture_gaps_unknown_timestamps_overflow_and_stale_cursor_require_reset() {
    let (provider, _) = provider(false, false);
    let mut flow =
        LiveTranslation::new(identity(), TranslationQuality::Standard, &provider).unwrap();
    for kind in 0..6 {
        let mut clock = LivePlaybackClock::new(identity());
        clock.observe(&audio(1, Some(1000)));
        let mut next = audio(2, Some(1960));
        match kind {
            0 => next.stamp.discontinuity = true,
            1 => next.stamp.sequence = 4,
            2 => next.stamp.frame_position = None,
            3 => next.stamp.frame_position = Some(u64::MAX),
            4 => {
                next.stamp.received -=
                    crate::audio::capture::MAX_AUDIO_AGE + Duration::from_millis(1)
            }
            _ => next.stamp.session = SessionId(941),
        }
        assert!(clock.observe(&next));
        if kind >= 2 {
            assert!(clock.observation(flow.sources_mut().version()).is_none());
        }
    }
}
#[test]
fn lyrics_ui_reports_preparation_and_adoption_without_leaking_text_or_changed_destination() {
    let base = UiLyricsStatus {
        disclosure: Some(ContentDisclosure {
            service: "合成歌詞服務",
            transport: ContentTransport::Remote,
        }),
        state: UiLyricsState::Available,
    };
    for (state, expected) in [
        (TextPipelineStatus::Verifying, "核對"),
        (TextPipelineStatus::Preparing, "所選翻譯服務"),
        (TextPipelineStatus::Localizing, "本機處理臺灣用語"),
        (TextPipelineStatus::Unmatched, "沒有可靠"),
    ] {
        let label = base.with_pipeline(state, false).label();
        assert!(label.contains(expected));
        assert!(label.contains("歌曲資料將送至此服務"));
    }
    assert!(
        base.with_pipeline(TextPipelineStatus::Ready, true)
            .label()
            .contains("顯示已定位歌詞")
    );
    let mut disabled = base;
    disabled.state = UiLyricsState::Stopped;
    assert_eq!(
        disabled
            .with_pipeline(TextPipelineStatus::Preparing, true)
            .state,
        UiLyricsState::Stopped
    );
}
