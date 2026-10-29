//! Synthetic source text and alignment only; real Session, planner and UI snapshot.
use super::*;
use crate::{
    ai::{
        speech::{SpeechLanguage, TranscriptKind},
        translation::{
            TranslationCapabilities, TranslationEmitter, TranslationFuture, TranslationPass,
            TranslationRequest,
        },
    },
    audio::capture::SessionId,
    domain::DisplayMode,
    resolver::contracts::{
        Confidence, ExistingText, ResolverCancellation, SubtitleSource, TextCue, TextLanguage,
        TextQuery, WindowStamp,
    },
    subtitle::{
        existing_text::ChineseTextLocalizer,
        source_arbiter::{CandidateEvidence, CueAlignment},
        timeline::{MAX_TIMELINE_ITEMS, Stability},
    },
    ui::presentation::{Presentation, TimelineSnapshot},
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
const CAPTURE_START: u64 = 42_000_000;
const CUE_DURATION: u64 = 1_000_000;
fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(909),
        generation: 1,
    }
}
fn event(utterance: u64) -> TranscriptEvent {
    TranscriptEvent {
        identity: identity(),
        utterance,
        sequence: utterance,
        revision: 1,
        start_us: Some(CAPTURE_START),
        duration_us: Some(CUE_DURATION),
        language: Some(SpeechLanguage::English),
        kind: TranscriptKind::Final,
        text: "original synthetic ASR".into(),
    }
}
struct MockTranslator {
    calls: AtomicUsize,
}
impl MockTranslator {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }
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
        _: &'a TranslationRequest,
        output: &'a mut TranslationEmitter,
        _: TranslationCancellation,
    ) -> TranslationFuture<'a> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::Relaxed);
            output.emit("即時測試譯文")?;
            Ok("即時測試譯文".into())
        })
    }
}
fn flow(provider: &dyn Translator) -> LiveTranslation<'_> {
    LiveTranslation::new(identity(), TranslationQuality::Standard, provider).unwrap()
}
#[test]
fn phase11_native_captions_respect_manual_clear_without_fabricating_source_or_reviving_old_sessions()
 {
    use crate::ai::native_caption::NativeCaption;
    let mut live = LiveTranslation::native(identity(), TranslationQuality::Standard, None).unwrap();
    let mut view = Presentation::new(DisplayMode::Dual);
    view.select_session(identity());
    for (version, (sequence, revision, kind)) in [
        (1, 1, TranscriptKind::Partial),
        (1, 2, TranscriptKind::Final),
        (2, 1, TranscriptKind::Final),
    ]
    .into_iter()
    .enumerate()
    {
        live.ingest_native(NativeCaption {
            identity: identity(),
            sequence,
            revision,
            kind,
            text: "软件".into(),
            received: Instant::now(),
        })
        .unwrap();
        view.apply(TimelineSnapshot::from_timeline(
            live.timeline(),
            version as u64 + 1,
        ));
        if sequence == 1 && revision == 1 {
            assert_eq!(view.overlay_text(), "軟體");
            view.clear_subtitles();
        }
        if sequence == 1 && revision == 2 {
            assert_eq!(view.rows().count(), 0);
        }
    }
    assert_eq!(view.overlay_text(), "軟體");
    assert_eq!(view.rows().count(), 1);
    assert!(view.side_text().contains("僅譯文"));
    assert_eq!(live.pending_requests(), 0);
    let mut foreign = identity();
    foreign.generation += 1;
    assert!(
        !live
            .ingest_native(NativeCaption {
                identity: foreign,
                sequence: 3,
                revision: 1,
                kind: TranscriptKind::Final,
                text: "舊資料".into(),
                received: Instant::now()
            })
            .unwrap()
    );
}

#[tokio::test]
async fn phase11_incomplete_translation_retirement_rejects_superseded_jobs_without_false_expiry() {
    struct IncompleteOnly;
    impl Translator for IncompleteOnly {
        fn capabilities(&self) -> TranslationCapabilities {
            MockTranslator::new().capabilities()
        }
        fn translate<'a>(
            &'a self,
            _: &'a TranslationRequest,
            _: &'a mut TranslationEmitter,
            _: TranslationCancellation,
        ) -> TranslationFuture<'a> {
            Box::pin(async { Err(TranslationError::Incomplete) })
        }
    }
    use crate::performance::live::{Count, LiveDiagnostics};
    let provider = IncompleteOnly;
    let metrics = LiveDiagnostics::new(identity());
    let mut core = TranslationCoordinator::diagnosed(
        identity(),
        TranslationQuality::Standard,
        provider.capabilities(),
        Some(metrics.clone()),
    )
    .unwrap();
    let mut initial = event(1);
    initial.kind = TranscriptKind::Partial;
    assert!(core.ingest(initial).unwrap());
    tokio::time::sleep(
        crate::ai::translation::QualityPolicy::for_quality(TranslationQuality::Standard).debounce,
    )
    .await;
    let old_job = core.take_ready(Instant::now()).unwrap();
    let mut newer = event(1);
    newer.revision = 2;
    assert!(core.ingest(newer).unwrap());
    let new_job = core.take_ready(Instant::now()).unwrap();
    let (old_key, old_result) = old_job.run_live(&provider).await;
    assert_eq!(old_result, Err(TranslationError::Cancelled));
    assert_eq!(core.reject_incomplete_live_job(old_key), None);
    let (new_key, new_result) = new_job.run_live(&provider).await;
    assert_eq!(new_result, Err(TranslationError::Incomplete));
    assert_eq!(core.reject_incomplete_live_job(new_key), Some(1));
    assert_eq!(core.reject_incomplete_live_job(new_key), None);
    assert!(!core.completed_live_revision(1));
    assert_eq!(core.active_requests(), 0);
    assert_eq!(core.cancellation().reason(), None);
    assert_eq!(metrics.count(Count::TranslationExpired), 0);
    assert_eq!(metrics.count(Count::FinalSegmentsExpired), 0);
}

#[tokio::test]
async fn phase11_incomplete_translation_keeps_live_source_and_later_segments_running() {
    struct IncompleteOnce;
    impl Translator for IncompleteOnce {
        fn capabilities(&self) -> TranslationCapabilities {
            MockTranslator::new().capabilities()
        }
        fn translate<'a>(
            &'a self,
            request: &'a TranslationRequest,
            output: &'a mut TranslationEmitter,
            _: TranslationCancellation,
        ) -> TranslationFuture<'a> {
            Box::pin(async move {
                if request.source.utterance == 2 {
                    output.emit("暫定但未完整")?;
                    Err(TranslationError::Incomplete)
                } else {
                    assert!(
                        request
                            .previous_stable
                            .iter()
                            .all(|entry| entry.sequence != 2)
                    );
                    Ok("完整測試譯文".into())
                }
            })
        }
    }
    let provider = IncompleteOnce;
    let mut live = flow(&provider);
    live.ingest(event(1)).unwrap();
    while live.timeline().items()[0].stability != Stability::Final {
        live.changed().await.unwrap();
    }
    live.ingest(event(2)).unwrap();
    while live.notice().is_none() {
        live.changed().await.unwrap();
    }
    assert_eq!(live.cancellation().reason(), None);
    assert_eq!(live.active_requests(), 0);
    assert_eq!(live.notice().unwrap().cause, TranslationError::Incomplete);
    assert_eq!(live.notice().unwrap().skipped, 1);
    assert_eq!(
        live.timeline().items()[0].translated.as_ref(),
        "完整測試譯文"
    );
    assert_eq!(live.timeline().items()[1].stability, Stability::Tentative);
    live.ingest(event(3)).unwrap();
    while live.notice().unwrap().consecutive != 0 {
        live.changed().await.unwrap();
    }
    assert_eq!(live.timeline().items()[2].stability, Stability::Final);
    assert_eq!(
        live.timeline().items()[2].translated.as_ref(),
        "完整測試譯文"
    );
    assert_eq!(live.notice().unwrap().skipped, 1);
    assert_eq!(live.cancellation().reason(), None);
}

#[tokio::test]
async fn phase11_one_translation_timeout_does_not_stop_later_live_segments() {
    struct OccasionallySlowTranslator(AtomicUsize);
    impl Translator for OccasionallySlowTranslator {
        fn capabilities(&self) -> TranslationCapabilities {
            MockTranslator::new().capabilities()
        }
        fn translate<'a>(
            &'a self,
            _: &'a TranslationRequest,
            _: &'a mut TranslationEmitter,
            _: TranslationCancellation,
        ) -> TranslationFuture<'a> {
            Box::pin(async move {
                if self.0.fetch_add(1, Ordering::Relaxed) == 0 {
                    Err(TranslationError::Timeout)
                } else {
                    Ok("後續測試譯文".into())
                }
            })
        }
    }
    let provider = OccasionallySlowTranslator(AtomicUsize::new(0));
    let mut live = flow(&provider);
    live.ingest(event(1)).unwrap();
    assert_eq!(live.changed().await, Ok(None));
    assert_eq!(live.cancellation().reason(), None);
    assert_eq!(live.active_requests(), 0);
    live.ingest(event(2)).unwrap();
    while live.timeline().items()[1].translated.is_empty() {
        live.changed().await.unwrap();
    }
    assert_eq!(
        live.timeline().items()[1].translated.as_ref(),
        "後續測試譯文"
    );
    assert!(live.timeline().items()[0].translated.is_empty());
    while live.notice().unwrap().consecutive != 0 {
        live.changed().await.unwrap();
    }
    assert_eq!(live.notice().unwrap().skipped, 1);
    assert_eq!(live.notice().unwrap().sequence, 1);
    assert_eq!(live.cancellation().reason(), None);
}

#[tokio::test]
async fn phase11_persistent_translation_timeouts_escalate_but_credentials_fail_immediately() {
    struct FailingTranslator(TranslationError);
    impl Translator for FailingTranslator {
        fn capabilities(&self) -> TranslationCapabilities {
            MockTranslator::new().capabilities()
        }
        fn translate<'a>(
            &'a self,
            _: &'a TranslationRequest,
            _: &'a mut TranslationEmitter,
            _: TranslationCancellation,
        ) -> TranslationFuture<'a> {
            Box::pin(async move { Err(self.0) })
        }
    }
    for cause in [
        TranslationError::Timeout,
        TranslationError::Stale,
        TranslationError::Incomplete,
    ] {
        let provider = FailingTranslator(cause);
        let mut live = flow(&provider);
        for sequence in 1..=3 {
            live.ingest(event(sequence)).unwrap();
            assert_eq!(
                live.changed().await,
                if sequence == 3 { Err(cause) } else { Ok(None) }
            );
            assert_eq!(live.notice().unwrap().skipped, sequence);
            assert_eq!(live.active_requests(), 0);
        }
        assert_eq!(live.cancellation().reason(), Some(cause));
        assert_eq!(live.changed().await, Err(cause));
    }
    for cause in [
        TranslationError::Credential,
        TranslationError::Network,
        TranslationError::InvalidData,
        TranslationError::Refused,
    ] {
        let provider = FailingTranslator(cause);
        let mut live = flow(&provider);
        live.ingest(event(1)).unwrap();
        assert_eq!(live.changed().await, Err(cause));
        assert!(live.notice().is_none());
    }
}

#[tokio::test]
async fn phase11_expired_pending_live_text_is_skipped_without_request_or_deadline_reset() {
    let provider = MockTranslator::new();
    let mut live = flow(&provider);
    live.ingest(event(1)).unwrap();
    let now = Instant::now() + MAX_TRANSLATION_AGE + Duration::from_millis(1);
    let expired = live.core.expire_live_pending(now);
    for sequence in expired {
        live.record_skip(sequence, TranslationError::Stale).unwrap();
    }
    assert!(live.core.take_ready(now).is_none());
    assert_eq!(live.pending_requests(), 0);
    assert_eq!(provider.calls.load(Ordering::Relaxed), 0);
    assert_eq!(live.cancellation().reason(), None);
    live.ingest(event(2)).unwrap();
    while live.timeline().items()[1].translated.is_empty() {
        live.changed().await.unwrap();
    }
}

#[tokio::test]
async fn phase11_intermittent_timeouts_across_one_hundred_segments_remain_bounded() {
    struct IntermittentTranslator;
    impl Translator for IntermittentTranslator {
        fn capabilities(&self) -> TranslationCapabilities {
            MockTranslator::new().capabilities()
        }
        fn translate<'a>(
            &'a self,
            request: &'a TranslationRequest,
            _: &'a mut TranslationEmitter,
            _: TranslationCancellation,
        ) -> TranslationFuture<'a> {
            Box::pin(async move {
                if request.source.sequence.is_multiple_of(7) {
                    Err(TranslationError::Timeout)
                } else {
                    Ok("合成穩定譯文".into())
                }
            })
        }
    }
    let provider = IntermittentTranslator;
    let mut live = flow(&provider);
    tokio::time::timeout(Duration::from_secs(3), async {
        for sequence in 1..=100 {
            live.ingest(event(sequence)).unwrap();
            if sequence.is_multiple_of(7) {
                assert_eq!(live.changed().await, Ok(None));
                assert_eq!(live.notice().unwrap().skipped, sequence / 7);
            } else {
                while live
                    .timeline()
                    .items()
                    .last()
                    .unwrap()
                    .translated
                    .is_empty()
                {
                    live.changed().await.unwrap();
                }
                assert!(live.notice().is_none_or(|notice| notice.consecutive == 0));
            }
            assert!(live.timeline().items().len() <= MAX_TIMELINE_ITEMS);
            assert_eq!(live.cancellation().reason(), None);
            assert_eq!(live.active_requests(), 0);
            assert_eq!(live.pending_requests(), 0);
        }
    })
    .await
    .expect("bounded mock requests must not stall the Live flow");
    assert_eq!(live.notice().unwrap().skipped, 14);
    assert_eq!(live.timeline().items().last().unwrap().source.sequence, 100);
    live.cancellation().cancel();
    assert_eq!(live.changed().await, Err(TranslationError::Cancelled));
}

#[tokio::test]
async fn phase11_continuous_partial_revisions_show_chinese_before_speech_pauses() {
    struct SlowStreamingTranslator;
    impl Translator for SlowStreamingTranslator {
        fn capabilities(&self) -> TranslationCapabilities {
            MockTranslator::new().capabilities()
        }
        fn translate<'a>(
            &'a self,
            _: &'a TranslationRequest,
            output: &'a mut TranslationEmitter,
            cancel: TranslationCancellation,
        ) -> TranslationFuture<'a> {
            Box::pin(async move {
                tokio::select! { biased;
                    reason=cancel.cancelled()=>return Err(reason),
                    _=tokio::time::sleep(Duration::from_millis(240))=>{},
                }
                output.emit("持續語音的合成測試譯文")?;
                Ok("持續語音的合成測試譯文".into())
            })
        }
    }
    let provider = SlowStreamingTranslator;
    let mut live = flow(&provider);
    let mut revision = 0;
    let mut tick = tokio::time::interval(Duration::from_millis(40));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    tokio::time::timeout(Duration::from_millis(1200), async {
        loop {
            tokio::select! {
                _=tick.tick()=>{
                    revision+=1;
                    let mut source=event(1);
                    source.kind=TranscriptKind::Partial;
                    source.revision=revision;
                    source.start_us=None;
                    source.duration_us=None;
                    source.text="synthetic ".repeat(revision as usize).into();
                    live.ingest(source).unwrap();
                },
                update=live.changed()=>{
                    update.unwrap();
                    if live.timeline().items().iter().any(|item| !item.translated.is_empty()) {
                        assert!(revision>=4);
                        assert_eq!(live.timeline().items()[0].source.kind,TranscriptKind::Partial);
                        assert_eq!(live.timeline().items()[0].source.start_us,None);
                        break;
                    }
                }
            }
        }
    })
    .await
    .expect("continuous partials must show Chinese before any Final or pause");
}

#[tokio::test]
async fn phase11_deferred_partials_keep_only_latest_and_final_or_rollback_cancels_old_work() {
    for final_event in [false, true] {
        let provider = MockTranslator::new();
        let mut live = flow(&provider);
        let mut first = event(1);
        first.kind = TranscriptKind::Partial;
        first.text = "base".into();
        live.ingest(first.clone()).unwrap();
        let old_job = live
            .core
            .take_ready(Instant::now() + Duration::from_secs(1))
            .unwrap();
        for revision in 2..=30 {
            let mut next = first.clone();
            next.revision = revision;
            next.text = format!("base extension {revision}").into();
            assert!(!live.ingest(next).unwrap());
            assert_eq!(live.deferred.len(), 1);
        }
        let mut stale = first.clone();
        stale.revision = 10;
        stale.text = "base stale".into();
        assert!(!live.ingest(stale).unwrap());
        assert_eq!(live.deferred[0].event.revision, 30);
        let mut replacement = first;
        replacement.revision = 31;
        replacement.text = if final_event {
            "final corrected fixture"
        } else {
            "rollback fixture"
        }
        .into();
        replacement.kind = if final_event {
            TranscriptKind::Final
        } else {
            TranscriptKind::Partial
        };
        assert!(live.ingest(replacement).unwrap());
        assert!(live.deferred.is_empty());
        assert_eq!(
            old_job.run(&provider).await,
            Err(TranslationError::Cancelled)
        );
        let latest = live
            .core
            .take_ready(Instant::now() + Duration::from_secs(1))
            .unwrap();
        assert_eq!(latest.request().source.revision, 31);
        latest.run(&provider).await.unwrap();
        live.core.drain_updates().unwrap();
        assert_eq!(live.timeline().items()[0].source.revision, 31);
        assert_eq!(
            live.timeline().items()[0].translated.as_ref(),
            "即時測試譯文"
        );
    }
}

#[tokio::test]
async fn phase11_deferred_partial_deadline_is_not_reset_and_stop_never_releases_it() {
    for stopped in [false, true] {
        let provider = MockTranslator::new();
        let mut live = flow(&provider);
        let mut first = event(1);
        first.kind = TranscriptKind::Partial;
        first.text = "base".into();
        live.ingest(first.clone()).unwrap();
        let job = live
            .core
            .take_ready(Instant::now() + Duration::from_secs(1))
            .unwrap();
        let mut latest = first;
        latest.revision = 2;
        latest.text = "base extension".into();
        live.ingest(latest).unwrap();
        live.deferred[0].expires = Instant::now() - Duration::from_millis(1);
        job.run(&provider).await.unwrap();
        live.core.drain_updates().unwrap();
        if stopped {
            live.cancellation().cancel();
        }
        let expected = if stopped {
            Err(TranslationError::Cancelled)
        } else {
            Ok(None)
        };
        assert_eq!(live.changed().await, expected);
        assert_eq!(live.timeline().items()[0].source.revision, 1);
        if !stopped {
            assert_eq!(live.notice().unwrap().cause, TranslationError::Stale);
            assert_eq!(live.cancellation().reason(), None);
            let mut duplicate = live.timeline().items()[0].source.clone();
            duplicate.revision = 2;
            duplicate.text = "base extension".into();
            assert!(!live.ingest(duplicate.clone()).unwrap());
            assert!(live.deferred.is_empty());
            assert_eq!(live.pending_requests(), 0);
            duplicate.revision = 3;
            duplicate.text = "base extension fresh".into();
            assert!(live.ingest(duplicate).unwrap());
        }
        assert_eq!(provider.calls.load(Ordering::Relaxed), 1);
    }
}

#[tokio::test]
async fn phase11_partial_stream_finishes_before_coalesced_source_is_admitted() {
    struct ChunkedTranslator {
        finish: tokio::sync::Notify,
    }
    impl Translator for ChunkedTranslator {
        fn capabilities(&self) -> TranslationCapabilities {
            MockTranslator::new().capabilities()
        }
        fn translate<'a>(
            &'a self,
            _: &'a TranslationRequest,
            output: &'a mut TranslationEmitter,
            cancel: TranslationCancellation,
        ) -> TranslationFuture<'a> {
            Box::pin(async move {
                output.emit("測試")?;
                tokio::select! {biased;reason=cancel.cancelled()=>return Err(reason),_=self.finish.notified()=>{}}
                output.emit("測試完整譯文")?;
                Ok("測試完整譯文".into())
            })
        }
    }
    let provider = ChunkedTranslator {
        finish: tokio::sync::Notify::new(),
    };
    let mut live = flow(&provider);
    let mut first = event(1);
    first.kind = TranscriptKind::Partial;
    first.text = "base".into();
    live.ingest(first.clone()).unwrap();
    let job = live
        .core
        .take_ready(Instant::now() + Duration::from_secs(1))
        .unwrap();
    let running = job.run(&provider);
    tokio::pin!(running);
    tokio::select! {biased;
        update=live.core.changed()=>{update.unwrap();},
        result=&mut running=>panic!("stream completed before release: {result:?}"),
    }
    assert_eq!(live.timeline().items()[0].translated.as_ref(), "測試");
    for revision in 2..=3 {
        let mut next = first.clone();
        next.revision = revision;
        next.text = format!("base revision {revision}").into();
        live.ingest(next).unwrap();
    }
    live.flush_deferred().unwrap();
    assert_eq!(live.timeline().items()[0].source.revision, 1);
    assert_eq!(live.deferred.len(), 1);
    provider.finish.notify_one();
    running.await.unwrap();
    live.core.drain_updates().unwrap();
    assert_eq!(
        live.timeline().items()[0].translated.as_ref(),
        "測試完整譯文"
    );
    live.flush_deferred().unwrap();
    assert_eq!(live.timeline().items()[0].source.revision, 3);
    assert_eq!(
        live.timeline().items()[0].translated.as_ref(),
        "測試完整譯文"
    );
    assert_eq!(live.timeline().items()[0].stability, Stability::Tentative);
    assert!(live.deferred.is_empty());
}
async fn candidate(
    flow: &mut LiveTranslation<'_>,
    sequence: u64,
    source: SubtitleSource,
) -> ResolverCancellation {
    let query = TextQuery {
        stamp: WindowStamp {
            identity: identity(),
            sequence,
            ended_at: Instant::now(),
            start_us: Some(40_000_000),
            duration_us: 12_000_000,
        },
        source,
        content_id: "synthetic document".into(),
    };
    let text = ExistingText {
        identity: identity(),
        source,
        content_id: query.content_id.clone(),
        language: TextLanguage::SimplifiedChinese,
        cues: vec![TextCue {
            start_us: Some(5_000_000),
            duration_us: Some(CUE_DURATION),
            text: "软件候選".into(),
        }],
    };
    let text = ChineseTextLocalizer::new(identity())
        .unwrap()
        .normalize(query, text, ResolverCancellation::default())
        .await
        .unwrap();
    let cancel = ResolverCancellation::default();
    let evidence = CandidateEvidence {
        content_confidence: Confidence::new(950).unwrap(),
        alignment: CueAlignment {
            version: flow.sources_mut().version(),
            stamp: text.stamp(),
            text_sequence: sequence,
            content_id: text.original().content_id.clone(),
            source,
            matched_original: text.original().cues[0].text.clone(),
            cue_index: 0,
            media_start_us: 5_000_000,
            capture_start_us: CAPTURE_START,
            confidence: Confidence::new(950).unwrap(),
        },
        availability: cancel.clone(),
        lookup_availability: None,
        preparation_availability: None,
    };
    flow.sources_mut()
        .submit_localized(text, evidence, Instant::now())
        .unwrap();
    cancel
}
fn adopt(
    flow: &mut LiveTranslation<'_>,
    source: &TranscriptEvent,
    sequence: u64,
    position: u64,
) -> Result<bool, SourceAdoptionError> {
    let observation = PlaybackObservation {
        version: flow.sources_mut().version(),
        sequence,
        observed_at: Instant::now(),
        capture_position_us: position,
    };
    flow.arbitrate(TimelineBinding::from_event(source), observation)
}
async fn finish_live(flow: &mut LiveTranslation<'_>) {
    tokio::time::timeout(Duration::from_secs(1), async {
        while flow.timeline().items().last().unwrap().stability != Stability::Final {
            flow.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn selected_text_and_live_backup_share_one_item_and_one_snapshot_for_both_views() {
    let provider = MockTranslator::new();
    let mut flow = flow(&provider);
    let source = event(1);
    flow.ingest(source.clone()).unwrap();
    finish_live(&mut flow).await;
    let cancel = candidate(&mut flow, 1, SubtitleSource::ExistingSubtitle).await;
    assert!(adopt(&mut flow, &source, 1, CAPTURE_START).unwrap());
    assert!(!adopt(&mut flow, &source, 2, CAPTURE_START).unwrap());
    let item = &flow.timeline().items()[0];
    assert_eq!(item.source.text.as_ref(), "original synthetic ASR");
    assert_eq!(item.translated.as_ref(), "即時測試譯文");
    assert_eq!(item.display_translation(), "軟體候選");
    assert_eq!(item.display_origin(), SubtitleSource::ExistingSubtitle);
    assert_eq!(
        flow.commit_prefix(1, "即時測試譯文".len()),
        Err(TranslationError::PrefixBusy)
    );
    let item = &flow.timeline().items()[0];
    let provenance = item.existing_display().unwrap();
    assert_eq!(provenance.content_id(), "synthetic document");
    assert_eq!(provenance.document_cue_index(), 0);
    assert_eq!(provenance.media_start_us(), 5_000_000);
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
    let frame = inbox.poll().unwrap();
    assert!(frame.status == crate::ui::bridge::UiStatus::UsingExistingText);
    assert!(frame.status_text().contains("即時辨識仍使用所選雲端服務"));
    let snapshot = frame.snapshot.as_ref().unwrap().clone();
    for mode in [
        DisplayMode::SideChat,
        DisplayMode::BottomSubtitle,
        DisplayMode::Dual,
    ] {
        let mut view = Presentation::new(mode);
        view.select_session(identity());
        assert!(view.apply_shared(snapshot.clone()));
        assert_eq!(view.rows().count(), 1);
        let row = view.rows().next().unwrap();
        assert_eq!(row.source.as_ref(), "软件候選");
        assert_eq!(row.translation.as_ref(), "軟體候選");
        assert_eq!(row.origin, SubtitleSource::ExistingSubtitle);
        assert_eq!(row.timestamp_us, Some(CAPTURE_START));
        assert_eq!(view.overlay_text(), "軟體候選");
    }
    cancel.cancel();
    // Snapshot validity does not depend on waiting for the next owner poll.
    let mut immediate = Presentation::new(DisplayMode::Dual);
    immediate.select_session(identity());
    immediate.apply(TimelineSnapshot::from_timeline(flow.timeline(), 2));
    assert_eq!(immediate.overlay_text(), "即時測試譯文");
    publisher.publish(flow.timeline(), true).unwrap();
    assert!(inbox.poll().unwrap().status == crate::ui::bridge::UiStatus::Translating);
    assert_eq!(flow.reconcile_sources(), Some(1));
    assert_eq!(
        flow.timeline().items()[0].display_translation(),
        "即時測試譯文"
    );
    assert_eq!(flow.pending_requests(), 0);
    assert_eq!(provider.calls.load(Ordering::Relaxed), 1);
}
#[tokio::test]
async fn adopted_candidate_suppresses_pending_cloud_work_and_gap_resumes_with_original_budget() {
    let provider = MockTranslator::new();
    let mut flow = flow(&provider);
    let source = event(1);
    flow.ingest(source.clone()).unwrap();
    candidate(&mut flow, 1, SubtitleSource::ExistingLyrics).await;
    adopt(&mut flow, &source, 1, CAPTURE_START).unwrap();
    assert_eq!(flow.pending_requests(), 0);
    assert!(
        tokio::time::timeout(Duration::from_millis(30), flow.changed())
            .await
            .is_err()
    );
    assert_eq!(provider.calls.load(Ordering::Relaxed), 0);
    assert!(adopt(&mut flow, &source, 2, CAPTURE_START + CUE_DURATION).unwrap());
    assert_eq!(flow.pending_requests(), 1);
    finish_live(&mut flow).await;
    assert_eq!(flow.timeline().items().len(), 1);
    assert_eq!(
        flow.timeline().items()[0].display_translation(),
        "即時測試譯文"
    );
}
struct PendingTranslator {
    active: Arc<AtomicUsize>,
    cancel: Mutex<Option<TranslationCancellation>>,
}
struct Active(Arc<AtomicUsize>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
impl Translator for PendingTranslator {
    fn capabilities(&self) -> TranslationCapabilities {
        MockTranslator::new().capabilities()
    }
    fn translate<'a>(
        &'a self,
        _: &'a TranslationRequest,
        _: &'a mut TranslationEmitter,
        cancel: TranslationCancellation,
    ) -> TranslationFuture<'a> {
        Box::pin(async move {
            self.active.fetch_add(1, Ordering::SeqCst);
            let _active = Active(self.active.clone());
            *self.cancel.lock().unwrap() = Some(cancel);
            std::future::pending().await
        })
    }
}
#[tokio::test]
async fn active_request_is_cancelled_on_adoption_and_released_when_owner_polls() {
    let provider = PendingTranslator {
        active: Arc::new(AtomicUsize::new(0)),
        cancel: Mutex::new(None),
    };
    let mut flow = flow(&provider);
    let source = event(1);
    flow.ingest(source.clone()).unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(30), flow.changed())
            .await
            .is_err()
    );
    assert_eq!(provider.active.load(Ordering::SeqCst), 1);
    candidate(&mut flow, 1, SubtitleSource::ExistingSubtitle).await;
    adopt(&mut flow, &source, 1, CAPTURE_START).unwrap();
    assert_eq!(
        provider.cancel.lock().unwrap().as_ref().unwrap().reason(),
        Some(TranslationError::Cancelled)
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(30), flow.changed())
            .await
            .is_err()
    );
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
    assert_eq!(flow.active_requests(), 0);
    assert_eq!(flow.cancellation().reason(), None);
}
#[tokio::test]
async fn queued_completion_from_cancelled_attempt_is_rejected_after_same_revision_resumes() {
    let provider = MockTranslator::new();
    let mut flow = flow(&provider);
    let source = event(1);
    flow.ingest(source.clone()).unwrap();
    let job = flow.core.take_ready(Instant::now()).unwrap();
    // Complete an old request, but leave its messages unread.
    job.run(&provider).await.unwrap();
    let cancel = candidate(&mut flow, 1, SubtitleSource::ExistingSubtitle).await;
    adopt(&mut flow, &source, 1, CAPTURE_START).unwrap();
    cancel.cancel();
    flow.reconcile_sources();
    let new_job = flow.core.take_ready(Instant::now()).unwrap();
    assert_eq!(flow.core.drain_updates().unwrap(), 0);
    assert!(flow.timeline().items()[0].translated.is_empty());
    new_job.run(&provider).await.unwrap();
    flow.core.drain_updates().unwrap();
    assert_eq!(
        flow.timeline().items()[0].translated.as_ref(),
        "即時測試譯文"
    );
    assert_eq!(flow.cancellation().reason(), None);
}
#[tokio::test]
async fn invalid_binding_is_rejected_before_advancing_arbiter_observation() {
    let provider = MockTranslator::new();
    let mut flow = flow(&provider);
    let source = event(1);
    flow.ingest(source.clone()).unwrap();
    candidate(&mut flow, 1, SubtitleSource::ExistingSubtitle).await;
    for case in 0..4 {
        let mut wrong = source.clone();
        match case {
            0 => wrong.identity.generation += 1,
            1 => wrong.revision += 1,
            2 => wrong.sequence += 1,
            _ => wrong.utterance += 1,
        }
        assert_eq!(
            adopt(&mut flow, &wrong, 1, CAPTURE_START),
            Err(SourceAdoptionError::InvalidBinding)
        );
    }
    assert_eq!(flow.pending_requests(), 1);
    assert!(adopt(&mut flow, &source, 1, CAPTURE_START).unwrap());
}
#[tokio::test]
async fn partial_unknown_timing_outside_interval_and_duplicate_cue_cannot_be_adopted() {
    let provider = MockTranslator::new();
    for case in 0..4 {
        let mut flow = flow(&provider);
        let mut source = event(1);
        match case {
            0 => source.kind = TranscriptKind::Partial,
            1 => source.duration_us = None,
            2 => source.start_us = Some(CAPTURE_START - 1),
            _ => source.duration_us = Some(CUE_DURATION + 1),
        }
        flow.ingest(source.clone()).unwrap();
        candidate(&mut flow, 1, SubtitleSource::ExistingSubtitle).await;
        assert_eq!(
            adopt(&mut flow, &source, 1, CAPTURE_START),
            Err(SourceAdoptionError::InvalidBinding)
        );
        assert_eq!(
            flow.timeline().items()[0].display_origin(),
            SubtitleSource::LiveAi
        );
    }
    let mut flow = flow(&provider);
    let first = event(1);
    let second = event(2);
    flow.ingest(first.clone()).unwrap();
    flow.ingest(second.clone()).unwrap();
    candidate(&mut flow, 1, SubtitleSource::ExistingSubtitle).await;
    adopt(&mut flow, &first, 1, CAPTURE_START).unwrap();
    assert_eq!(
        adopt(&mut flow, &second, 2, CAPTURE_START),
        Err(SourceAdoptionError::InvalidBinding)
    );
}
#[tokio::test]
async fn expired_prepared_lease_restores_backup_without_reuploading_stale_asr() {
    let provider = MockTranslator::new();
    let mut flow = flow(&provider);
    let source = event(1);
    flow.ingest(source.clone()).unwrap();
    candidate(&mut flow, 1, SubtitleSource::ExistingSubtitle).await;
    adopt(&mut flow, &source, 1, CAPTURE_START).unwrap();
    let now = Instant::now() + crate::resolver::contracts::MAX_WINDOW_AGE + Duration::from_secs(1);
    assert_eq!(flow.core.reconcile_sources(now), Some(1));
    assert_eq!(flow.pending_requests(), 0);
    assert!(flow.core.take_ready(now).is_none());
    assert_eq!(flow.cancellation().reason(), None);
    assert_eq!(
        flow.timeline().items()[0].display_origin(),
        SubtitleSource::LiveAi
    );
    let mut next = event(2);
    next.start_us = Some(CAPTURE_START + CUE_DURATION);
    flow.ingest(next).unwrap();
    finish_live(&mut flow).await;
    assert_eq!(provider.calls.load(Ordering::Relaxed), 1);
}
#[tokio::test]
async fn invalidation_is_reported_by_changed_and_stopped_session_rejects_new_adoption() {
    let provider = MockTranslator::new();
    let mut flow = flow(&provider);
    let source = event(1);
    flow.ingest(source.clone()).unwrap();
    candidate(&mut flow, 1, SubtitleSource::ExistingSubtitle).await;
    adopt(&mut flow, &source, 1, CAPTURE_START).unwrap();
    flow.sources_mut().invalidate().unwrap();
    assert_eq!(flow.changed().await.unwrap(), Some(1));
    assert_eq!(
        flow.timeline().items()[0].display_origin(),
        SubtitleSource::LiveAi
    );
    flow.cancellation().cancel();
    assert_eq!(
        adopt(&mut flow, &source, 2, CAPTURE_START),
        Err(SourceAdoptionError::Translation(
            TranslationError::Cancelled
        ))
    );
    assert_eq!(flow.changed().await, Err(TranslationError::Cancelled));
}
#[tokio::test]
async fn slow_snapshot_consumer_observes_latest_fallback_and_old_session_never_returns() {
    let provider = MockTranslator::new();
    let mut flow = flow(&provider);
    let source = event(1);
    flow.ingest(source.clone()).unwrap();
    finish_live(&mut flow).await;
    let cancel = candidate(&mut flow, 1, SubtitleSource::ExistingSubtitle).await;
    adopt(&mut flow, &source, 1, CAPTURE_START).unwrap();
    let old = TimelineSnapshot::from_timeline(flow.timeline(), 1);
    cancel.cancel();
    flow.reconcile_sources();
    let latest = TimelineSnapshot::from_timeline(flow.timeline(), 2);
    let mut view = Presentation::new(DisplayMode::Dual);
    view.select_session(identity());
    assert!(view.apply(latest));
    assert!(!view.apply(old));
    assert_eq!(view.overlay_text(), "即時測試譯文");
    let mut next = identity();
    next.generation += 1;
    view.select_session(next);
    assert!(!view.apply(TimelineSnapshot::from_timeline(flow.timeline(), 3)));
    assert_eq!(view.rows().count(), 1);
    assert_eq!(view.identity(), Some(next));
    assert!(!view.latest_caption_visible());
    assert!(view.side_text().contains("先前字幕"));
}
#[tokio::test]
async fn adopted_item_retirement_and_owner_drop_release_session_text_without_extra_rows() {
    let provider = MockTranslator::new();
    let mut flow = flow(&provider);
    let source = event(1);
    flow.ingest(source.clone()).unwrap();
    let cancel = candidate(&mut flow, 1, SubtitleSource::ExistingSubtitle).await;
    adopt(&mut flow, &source, 1, CAPTURE_START).unwrap();
    for utterance in 2..=MAX_TIMELINE_ITEMS as u64 + 1 {
        flow.ingest(event(utterance)).unwrap();
    }
    assert_eq!(flow.timeline().items().len(), MAX_TIMELINE_ITEMS);
    assert!(
        !flow
            .timeline()
            .items()
            .iter()
            .any(|item| item.source.utterance == 1)
    );
    assert_eq!(
        adopt(&mut flow, &source, 2, CAPTURE_START),
        Err(SourceAdoptionError::InvalidBinding)
    );
    drop(flow);
    assert!(cancel.is_cancelled());
}

#[tokio::test]
async fn unrelated_live_utterance_continues_while_verified_text_is_active() {
    let provider = MockTranslator::new();
    let mut flow = flow(&provider);
    let first = event(1);
    let mut gap = event(2);
    gap.start_us = Some(CAPTURE_START + CUE_DURATION);
    flow.ingest(first.clone()).unwrap();
    candidate(&mut flow, 1, SubtitleSource::ExistingSubtitle).await;
    adopt(&mut flow, &first, 1, CAPTURE_START).unwrap();
    flow.ingest(gap).unwrap();
    finish_live(&mut flow).await;
    assert_eq!(provider.calls.load(Ordering::Relaxed), 1);
    assert_eq!(flow.timeline().items().len(), 2);
    assert_eq!(
        flow.timeline().items()[0].display_origin(),
        SubtitleSource::ExistingSubtitle
    );
    assert_eq!(
        flow.timeline().items()[1].display_origin(),
        SubtitleSource::LiveAi
    );
    let mut view = Presentation::new(DisplayMode::Dual);
    view.select_session(identity());
    view.apply(TimelineSnapshot::from_timeline(flow.timeline(), 1));
    assert_eq!(view.rows().count(), 2);
    assert_eq!(view.overlay_text(), "即時測試譯文");
}

#[tokio::test]
async fn high_review_resumes_with_draft_and_new_request_order_after_source_withdrawal() {
    struct ReviewTranslator(Mutex<Vec<TranslationPass>>);
    impl Translator for ReviewTranslator {
        fn capabilities(&self) -> TranslationCapabilities {
            MockTranslator::new().capabilities()
        }
        fn translate<'a>(
            &'a self,
            request: &'a TranslationRequest,
            output: &'a mut TranslationEmitter,
            _: TranslationCancellation,
        ) -> TranslationFuture<'a> {
            Box::pin(async move {
                self.0.lock().unwrap().push(request.pass);
                let text = if request.pass == TranslationPass::Draft {
                    "草稿軟體"
                } else {
                    assert_eq!(request.draft.as_deref(), Some("草稿軟體"));
                    "校訂軟體"
                };
                output.emit(text)?;
                Ok(text.into())
            })
        }
    }
    let provider = ReviewTranslator(Mutex::new(Vec::new()));
    let mut flow = LiveTranslation::new(identity(), TranslationQuality::High, &provider).unwrap();
    flow.glossary_mut()
        .set(identity(), "software", "軟體")
        .unwrap();
    let source = event(1);
    flow.ingest(source.clone()).unwrap();
    flow.core
        .take_ready(Instant::now())
        .unwrap()
        .run(&provider)
        .await
        .unwrap();
    flow.core.drain_updates().unwrap();
    assert_eq!(flow.timeline().items()[0].stability, Stability::Stable);
    let old_review = flow.core.take_ready(Instant::now()).unwrap();
    old_review.run(&provider).await.unwrap();
    let cancel = candidate(&mut flow, 1, SubtitleSource::ExistingSubtitle).await;
    adopt(&mut flow, &source, 1, CAPTURE_START).unwrap();
    cancel.cancel();
    flow.reconcile_sources();
    let new_review = flow.core.take_ready(Instant::now()).unwrap();
    assert_eq!(flow.core.drain_updates().unwrap(), 0);
    new_review.run(&provider).await.unwrap();
    flow.core.drain_updates().unwrap();
    assert_eq!(flow.timeline().items()[0].stability, Stability::Final);
    assert_eq!(flow.timeline().items()[0].display_translation(), "校訂軟體");
    assert_eq!(
        *provider.0.lock().unwrap(),
        [
            TranslationPass::Draft,
            TranslationPass::Review,
            TranslationPass::Review
        ]
    );
}
