//! Explicit test translators. No network, API key input, or real translation claims.
use super::*;
use crate::{ai::speech::SpeechLanguage, audio::capture::SessionId};
use std::sync::atomic::{AtomicUsize, Ordering};
fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(4000),
        generation: 1,
    }
}
fn caps() -> TranslationCapabilities {
    TranslationCapabilities {
        streaming: true,
        cancellation: true,
        supported_languages: [true; 5],
        max_concurrent: 2,
    }
}
#[test]
fn phase11_native_caption_uses_the_shared_timeline_without_asr_or_translation_jobs() {
    use crate::ai::native_caption::NativeCaption;
    let mut core = coordinator(TranslationQuality::Standard);
    for (revision, kind) in [(1, TranscriptKind::Partial), (2, TranscriptKind::Final)] {
        assert!(
            core.ingest_native(NativeCaption {
                identity: identity(),
                sequence: 1,
                revision,
                kind,
                text: "软件与打印机".into(),
                received: Instant::now()
            })
            .unwrap()
        );
    }
    let row = &core.timeline().items()[0];
    assert!(row.source.text.is_empty());
    assert_eq!(row.source.start_us, None);
    assert_eq!(row.source.duration_us, None);
    assert_eq!(row.source.language, None);
    assert_eq!(row.translated.as_ref(), "軟體與印表機");
    assert_eq!(row.stability, Stability::Final);
    assert_eq!(core.pending_requests(), 0);
    assert_eq!(core.active_requests(), 0);
    assert!(core.take_ready(Instant::now()).is_none());
    let mut foreign = identity();
    foreign.generation += 1;
    assert!(
        !core
            .ingest_native(NativeCaption {
                identity: foreign,
                sequence: 2,
                revision: 1,
                kind: TranscriptKind::Final,
                text: "synthetic foreign".into(),
                received: Instant::now()
            })
            .unwrap()
    );
    assert_eq!(
        core.ingest_native(NativeCaption {
            identity: identity(),
            sequence: 2,
            revision: 1,
            kind: TranscriptKind::Final,
            text: "synthetic stale".into(),
            received: Instant::now() - MAX_TRANSLATION_AGE - Duration::from_millis(1)
        }),
        Err(TranslationError::Stale)
    );
    assert_eq!(core.timeline().items().len(), 1);
    core.cancellation().cancel();
    assert_eq!(
        core.ingest_native(NativeCaption {
            identity: identity(),
            sequence: 2,
            revision: 1,
            kind: TranscriptKind::Final,
            text: "synthetic stopped".into(),
            received: Instant::now()
        }),
        Err(TranslationError::Cancelled)
    );
}
fn source(utterance: u64, revision: u64, kind: TranscriptKind, text: &str) -> TranscriptEvent {
    TranscriptEvent {
        identity: identity(),
        utterance,
        revision,
        sequence: utterance,
        start_us: Some(utterance * 100000),
        duration_us: Some(50000),
        language: Some(SpeechLanguage::English),
        kind,
        text: text.into(),
    }
}
fn coordinator(quality: TranslationQuality) -> TranslationCoordinator {
    TranslationCoordinator::new(identity(), quality, caps()).unwrap()
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
        caps()
    }
    fn translate<'a>(
        &'a self,
        request: &'a TranslationRequest,
        output: &'a mut TranslationEmitter,
        _cancel: TranslationCancellation,
    ) -> TranslationFuture<'a> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::Relaxed);
            assert_eq!(request.target_language(), "zh-TW");
            assert!(request.style_guide().contains("臺灣用語"));
            let text = if request.pass == TranslationPass::Review {
                "測試校訂文字"
            } else {
                "測試翻譯文字"
            };
            output.emit("測試")?;
            output.emit(text)?;
            Ok(text.into())
        })
    }
}
#[tokio::test]
async fn quality_changes_debounce_context_and_high_review_work() {
    let provider = MockTranslator::new();
    for (quality, count) in [
        (TranslationQuality::Fast, 1),
        (TranslationQuality::Standard, 3),
        (TranslationQuality::High, 8),
    ] {
        let mut core = coordinator(quality);
        core.glossary_mut()
            .set(identity(), "software", "軟體")
            .unwrap();
        for utterance in 1..=9 {
            core.ingest(source(
                utterance,
                1,
                TranscriptKind::Final,
                "fixture sentence",
            ))
            .unwrap();
            let job = core.take_ready(Instant::now()).unwrap();
            assert_eq!(
                job.request().previous_stable.len(),
                ((utterance - 1) as usize).min(count)
            );
            assert_eq!(
                job.request().glossary[0].taiwan_translation.as_ref(),
                "軟體"
            );
            job.run(&provider).await.unwrap();
            core.drain_updates().unwrap();
            if quality == TranslationQuality::High {
                assert_eq!(
                    core.timeline().items().last().unwrap().stability,
                    Stability::Stable
                );
                let review = core.take_ready(Instant::now()).unwrap();
                assert_eq!(review.request().pass, TranslationPass::Review);
                assert!(review.request().draft.is_some());
                review.run(&provider).await.unwrap();
                core.drain_updates().unwrap();
            } else {
                assert!(core.take_ready(Instant::now()).is_none());
            }
            assert_eq!(
                core.timeline().items().last().unwrap().stability,
                Stability::Final
            );
        }
        core.ingest(source(10, 1, TranscriptKind::Partial, "tail"))
            .unwrap();
        assert!(core.take_ready(Instant::now()).is_none());
        assert!(
            core.take_ready(Instant::now() + core.policy.debounce + Duration::from_millis(1))
                .is_some()
        );
    }
    assert_eq!(provider.calls.load(Ordering::Relaxed), 36);
}
#[tokio::test]
async fn source_revisions_cancel_old_jobs_and_coalesce_partial_tail() {
    let mut core = coordinator(TranslationQuality::Standard);
    let provider = MockTranslator::new();
    core.ingest(source(1, 1, TranscriptKind::Partial, "old long sentence"))
        .unwrap();
    let old = core
        .take_ready(Instant::now() + Duration::from_secs(1))
        .unwrap();
    core.ingest(source(1, 2, TranscriptKind::Partial, "short"))
        .unwrap();
    assert_eq!(old.run(&provider).await, Err(TranslationError::Cancelled));
    assert_eq!(provider.calls.load(Ordering::Relaxed), 0);
    core.ingest(source(
        1,
        3,
        TranscriptKind::Final,
        "final corrected source",
    ))
    .unwrap();
    let newest = core.take_ready(Instant::now()).unwrap();
    assert_eq!(newest.request().source.revision, 3);
    newest.run(&provider).await.unwrap();
    core.drain_updates().unwrap();
    assert_eq!(core.timeline().items().len(), 1);
    assert_eq!(
        core.timeline().items()[0].source.text.as_ref(),
        "final corrected source"
    );
    assert!(
        !core
            .ingest(source(1, 4, TranscriptKind::Partial, "late"))
            .unwrap()
    );
}

#[tokio::test]
async fn phase11_unknown_capture_times_still_translate_standard_partial_and_final() {
    let mut core = coordinator(TranslationQuality::Standard);
    let provider = MockTranslator::new();
    for (revision, kind) in [(1, TranscriptKind::Partial), (2, TranscriptKind::Final)] {
        let mut event = source(1, revision, kind, "synthetic source fixture");
        event.start_us = None;
        event.duration_us = None;
        core.ingest(event).unwrap();
        let job = core
            .take_ready(Instant::now() + core.policy.debounce)
            .unwrap();
        job.run(&provider).await.unwrap();
        core.drain_updates().unwrap();
        let item = &core.timeline().items()[0];
        assert_eq!(item.translated.as_ref(), "測試翻譯文字");
        assert_eq!(item.source.start_us, None);
        assert_eq!(item.source.duration_us, None);
        assert_eq!(item.source.kind, kind);
    }
    assert_eq!(provider.calls.load(Ordering::Relaxed), 2);
    assert_eq!(core.timeline().items()[0].stability, Stability::Final);
}
#[tokio::test]
async fn out_of_order_completion_uses_one_source_ordered_timeline() {
    let mut core = coordinator(TranslationQuality::Fast);
    let provider = MockTranslator::new();
    core.ingest(source(1, 1, TranscriptKind::Final, "first"))
        .unwrap();
    core.ingest(source(2, 1, TranscriptKind::Final, "second"))
        .unwrap();
    let first = core.take_ready(Instant::now()).unwrap();
    let second = core.take_ready(Instant::now()).unwrap();
    assert_eq!(core.active_requests(), 2);
    assert!(core.take_ready(Instant::now()).is_none());
    second.run(&provider).await.unwrap();
    core.drain_updates().unwrap();
    first.run(&provider).await.unwrap();
    core.drain_updates().unwrap();
    assert_eq!(core.timeline().items()[0].source.utterance, 1);
    assert_eq!(core.timeline().items()[1].source.utterance, 2);
    assert_eq!(core.active_requests(), 0);
}
#[test]
fn session_generation_and_retired_history_cannot_reappear() {
    let mut core = coordinator(TranslationQuality::Fast);
    let mut foreign = source(1, 1, TranscriptKind::Final, "foreign");
    foreign.identity.generation = 0;
    assert!(!core.ingest(foreign).unwrap());
    for utterance in 1..=100 {
        core.ingest(source(
            utterance,
            1,
            TranscriptKind::Final,
            "bounded history",
        ))
        .unwrap();
    }
    assert_eq!(core.timeline().items().len(), MAX_TIMELINE_ITEMS);
    assert_eq!(core.work.len(), MAX_TIMELINE_ITEMS);
    assert!(
        !core
            .ingest(source(1, 2, TranscriptKind::Final, "retired"))
            .unwrap()
    );
    assert_eq!(core.timeline().items()[0].source.utterance, 37);
}
#[test]
fn glossary_is_bounded_session_owned_and_updates_existing_terms() {
    let mut glossary = SessionGlossary::new(identity());
    glossary.set(identity(), "software", "軟體").unwrap();
    glossary.set(identity(), "software", "應用軟體").unwrap();
    assert_eq!(glossary.entries().len(), 1);
    assert_eq!(
        glossary.entries()[0].taiwan_translation.as_ref(),
        "應用軟體"
    );
    let mut foreign = identity();
    foreign.generation += 1;
    assert_eq!(
        glossary.set(foreign, "network", "網路"),
        Err(TranslationError::InvalidData)
    );
    for index in 1..MAX_GLOSSARY_ENTRIES {
        glossary
            .set(identity(), &format!("fixture-{index}"), "測試")
            .unwrap();
    }
    assert_eq!(
        glossary.set(identity(), "overflow", "測試"),
        Err(TranslationError::Overloaded)
    );
    assert!(glossary.set(identity(), "bad\nterm", "測試").is_err());
    assert!(SessionGlossary::new(foreign).entries().is_empty());
}
#[test]
fn stable_prefix_is_utf8_safe_and_only_mutable_tail_can_roll_back() {
    let mut timeline = TranscriptTimeline::new(identity());
    timeline
        .ingest(source(1, 1, TranscriptKind::Partial, "source"))
        .unwrap();
    let mut update = TranslationUpdate {
        job_id: 0,
        identity: identity(),
        utterance: 1,
        source_revision: 1,
        pass: TranslationPass::Draft,
        revision: 1,
        text: "軟體正在啟動".into(),
        stability: Stability::Tentative,
        stable_prefix_bytes: 0,
        complete: false,
    };
    assert!(timeline.apply(&update).unwrap());
    assert!(timeline.commit_prefix(1, 1).is_err());
    timeline.commit_prefix(1, "軟體".len()).unwrap();
    update.revision = 2;
    update.text = "軟體已停止".into();
    update.stable_prefix_bytes = "軟體".len();
    assert!(timeline.apply(&update).unwrap());
    update.revision = 3;
    update.text = "程式已停止".into();
    assert_eq!(timeline.apply(&update), Err(TranslationError::InvalidData));
    assert_eq!(timeline.items()[0].translated.as_ref(), "軟體已停止");
}
struct PendingTranslator;
impl Translator for PendingTranslator {
    fn capabilities(&self) -> TranslationCapabilities {
        caps()
    }
    fn translate<'a>(
        &'a self,
        _: &'a TranslationRequest,
        _: &'a mut TranslationEmitter,
        _: TranslationCancellation,
    ) -> TranslationFuture<'a> {
        Box::pin(std::future::pending())
    }
}
#[tokio::test]
async fn coordinator_drop_cancels_pending_provider_and_releases_permit() {
    let mut core = coordinator(TranslationQuality::Fast);
    core.ingest(source(1, 1, TranscriptKind::Final, "pending"))
        .unwrap();
    let job = core.take_ready(Instant::now()).unwrap();
    let cancel = core.cancellation();
    let future = job.run(&PendingTranslator);
    tokio::pin!(future);
    assert!(
        tokio::time::timeout(Duration::from_millis(10), &mut future)
            .await
            .is_err()
    );
    drop(core);
    assert_eq!(future.await, Err(TranslationError::Cancelled));
    assert_eq!(cancel.reason(), Some(TranslationError::Cancelled));
}
struct FailingTranslator;
impl Translator for FailingTranslator {
    fn capabilities(&self) -> TranslationCapabilities {
        caps()
    }
    fn translate<'a>(
        &'a self,
        _: &'a TranslationRequest,
        _: &'a mut TranslationEmitter,
        _: TranslationCancellation,
    ) -> TranslationFuture<'a> {
        Box::pin(async { Err(TranslationError::Credential) })
    }
}
#[tokio::test]
async fn provider_failure_keeps_safe_error_and_clears_queued_work() {
    let mut core = coordinator(TranslationQuality::Fast);
    core.ingest(source(1, 1, TranscriptKind::Final, "PRIVATE_TEXT"))
        .unwrap();
    let job = core.take_ready(Instant::now()).unwrap();
    assert_eq!(
        job.run(&FailingTranslator).await,
        Err(TranslationError::Credential)
    );
    assert_eq!(core.drain_updates(), Err(TranslationError::Credential));
    assert!(core.take_ready(Instant::now()).is_none());
    assert!(
        !TranslationError::Credential
            .to_string()
            .contains("PRIVATE_TEXT")
    );
    assert_eq!(core.active_requests(), 0);
}
struct FloodTranslator;
impl Translator for FloodTranslator {
    fn capabilities(&self) -> TranslationCapabilities {
        caps()
    }
    fn translate<'a>(
        &'a self,
        _: &'a TranslationRequest,
        output: &'a mut TranslationEmitter,
        _: TranslationCancellation,
    ) -> TranslationFuture<'a> {
        Box::pin(async move {
            for _ in 0..=TRANSLATION_QUEUE_ITEMS {
                output.emit("測試")?;
            }
            Ok("測試".into())
        })
    }
}
#[tokio::test]
async fn slow_consumer_cancels_at_bounded_update_queue() {
    let mut core = coordinator(TranslationQuality::Fast);
    core.ingest(source(1, 1, TranscriptKind::Final, "source"))
        .unwrap();
    assert_eq!(
        core.take_ready(Instant::now())
            .unwrap()
            .run(&FloodTranslator)
            .await,
        Err(TranslationError::Overloaded)
    );
    assert_eq!(core.receiver.len(), TRANSLATION_QUEUE_ITEMS);
    assert_eq!(core.drain_updates(), Err(TranslationError::Overloaded));
    assert_eq!(core.receiver.len(), 0);
}
#[test]
fn capability_limits_and_invalid_time_are_rejected() {
    let mut invalid = caps();
    invalid.max_concurrent = MAX_CONCURRENT_TRANSLATIONS + 1;
    assert!(
        TranslationCoordinator::new(identity(), TranslationQuality::Standard, invalid).is_err()
    );
    invalid = caps();
    invalid.supported_languages[4] = false;
    assert!(
        TranslationCoordinator::new(identity(), TranslationQuality::Standard, invalid).is_err()
    );
    let mut core = coordinator(TranslationQuality::Standard);
    let mut invalid = source(1, 1, TranscriptKind::Partial, "source");
    invalid.start_us = Some(u64::MAX);
    assert_eq!(core.ingest(invalid), Err(TranslationError::InvalidData));
}

#[tokio::test]
async fn queued_old_revision_and_previous_generation_updates_are_filtered() {
    let provider = MockTranslator::new();
    let mut core = coordinator(TranslationQuality::Fast);
    core.ingest(source(1, 1, TranscriptKind::Partial, "old"))
        .unwrap();
    core.take_ready(Instant::now() + Duration::from_secs(1))
        .unwrap()
        .run(&provider)
        .await
        .unwrap();
    core.ingest(source(1, 2, TranscriptKind::Final, "new"))
        .unwrap();
    assert_eq!(core.drain_updates().unwrap(), 0);
    assert!(core.timeline().items()[0].translated.is_empty());
    core.take_ready(Instant::now())
        .unwrap()
        .run(&provider)
        .await
        .unwrap();
    core.drain_updates().unwrap();
    let mut foreign = identity();
    foreign.generation = 0;
    assert!(
        !core
            .apply(TranslationUpdate {
                job_id: 0,
                identity: foreign,
                utterance: 1,
                source_revision: 2,
                pass: TranslationPass::Review,
                revision: 999,
                text: "舊工作階段".into(),
                stability: Stability::Final,
                stable_prefix_bytes: "舊工作階段".len(),
                complete: true
            })
            .unwrap()
    );
    assert_eq!(
        core.timeline().items()[0].translated.as_ref(),
        "測試翻譯文字"
    );
}
#[test]
fn expired_pending_work_stops_before_requesting_old_text() {
    let mut core = coordinator(TranslationQuality::Fast);
    core.ingest(source(1, 1, TranscriptKind::Partial, "stale"))
        .unwrap();
    assert!(
        core.take_ready(Instant::now() + MAX_TRANSLATION_AGE + Duration::from_millis(1))
            .is_none()
    );
    assert_eq!(core.drain_updates(), Err(TranslationError::Stale));
    assert_eq!(core.active_requests(), 0);
}

#[tokio::test]
async fn configured_concurrency_cannot_exceed_actual_provider_limits() {
    let mut capabilities = caps();
    capabilities.max_concurrent = 4;
    let mut core =
        TranslationCoordinator::new(identity(), TranslationQuality::Fast, capabilities).unwrap();
    let provider = MockTranslator::new();
    core.ingest(source(1, 1, TranscriptKind::Final, "source"))
        .unwrap();
    assert_eq!(
        core.take_ready(Instant::now())
            .unwrap()
            .run(&provider)
            .await,
        Err(TranslationError::UnsupportedCapability)
    );
    assert_eq!(provider.calls.load(Ordering::Relaxed), 0);
    assert_eq!(core.active_requests(), 0);
}

#[tokio::test]
async fn active_request_deadline_cancels_a_provider_that_never_finishes() {
    let mut core = coordinator(TranslationQuality::Standard);
    core.ingest(source(1, 1, TranscriptKind::Final, "source"))
        .unwrap();
    let mut job = core.take_ready(Instant::now()).unwrap();
    job.expires = Instant::now() + Duration::from_millis(20);
    assert_eq!(
        job.run(&PendingTranslator).await,
        Err(TranslationError::Timeout)
    );
    assert_eq!(core.active_requests(), 0);
    assert_eq!(core.drain_updates(), Err(TranslationError::Timeout));
}

#[tokio::test]
async fn phase11_live_deadline_releases_permit_and_rejects_queued_late_partial() {
    struct EmitsThenStalls;
    impl Translator for EmitsThenStalls {
        fn capabilities(&self) -> TranslationCapabilities {
            caps()
        }
        fn translate<'a>(
            &'a self,
            _: &'a TranslationRequest,
            output: &'a mut TranslationEmitter,
            _: TranslationCancellation,
        ) -> TranslationFuture<'a> {
            Box::pin(async move {
                output.emit("未完成測試譯文")?;
                std::future::pending().await
            })
        }
    }
    for applied_before_timeout in [false, true] {
        let mut core = coordinator(TranslationQuality::Standard);
        core.ingest(source(1, 1, TranscriptKind::Final, "synthetic"))
            .unwrap();
        let mut job = core.take_ready(Instant::now()).unwrap();
        job.expires = Instant::now() + Duration::from_millis(30);
        let running = job.run_live(&EmitsThenStalls);
        tokio::pin!(running);
        if applied_before_timeout {
            tokio::select! {biased;
                update=core.changed()=>{ assert_eq!(update, Ok(Some(1))); },
                _=&mut running=>panic!("provider must emit before timing out"),
            }
        }
        let (key, result) = running.await;
        assert_eq!(result, Err(TranslationError::Timeout));
        assert_eq!(core.active_requests(), 0);
        assert_eq!(core.expire_live_job(key), Some(1));
        assert_eq!(core.expire_live_job(key), None);
        assert_eq!(core.drain_updates(), Ok(0));
        assert_eq!(core.cancellation().reason(), None);
        assert_eq!(core.timeline().items()[0].stability, Stability::Tentative);
        assert_eq!(
            core.timeline().items()[0].translated.is_empty(),
            !applied_before_timeout
        );
        core.ingest(source(2, 1, TranscriptKind::Final, "next synthetic"))
            .unwrap();
        core.take_ready(Instant::now())
            .unwrap()
            .run(&MockTranslator::new())
            .await
            .unwrap();
        assert_eq!(core.drain_updates(), Ok(3));
        assert_eq!(core.timeline().items()[1].stability, Stability::Final);
    }
}

#[tokio::test]
async fn phase11_expired_job_key_cannot_retire_new_revision_or_foreign_identity() {
    let provider = PendingTranslator;
    let mut core = coordinator(TranslationQuality::Standard);
    core.ingest(source(1, 1, TranscriptKind::Partial, "synthetic"))
        .unwrap();
    let mut job = core
        .take_ready(Instant::now() + Duration::from_secs(1))
        .unwrap();
    job.expires = Instant::now() + Duration::from_millis(15);
    let (key, result) = job.run_live(&provider).await;
    assert_eq!(result, Err(TranslationError::Timeout));
    let mut foreign = key;
    foreign.identity.generation += 1;
    assert_eq!(core.expire_live_job(foreign), None);
    assert!(
        core.ingest(source(1, 2, TranscriptKind::Final, "new synthetic"))
            .unwrap()
    );
    assert_eq!(core.expire_live_job(key), None);
    assert_eq!(core.pending_requests(), 1);
    core.take_ready(Instant::now())
        .unwrap()
        .run(&MockTranslator::new())
        .await
        .unwrap();
    assert_eq!(core.drain_updates(), Ok(3));
    assert_eq!(core.timeline().items()[0].source.revision, 2);
    assert_eq!(core.timeline().items()[0].stability, Stability::Final);
}

#[test]
fn rapid_partial_revisions_do_not_postpone_the_initial_due_time() {
    let mut core = coordinator(TranslationQuality::Standard);
    core.ingest(source(1, 1, TranscriptKind::Partial, "first"))
        .unwrap();
    let first_due = core.work[0].due;
    for revision in 2..=20 {
        core.ingest(source(
            1,
            revision,
            TranscriptKind::Partial,
            "latest partial",
        ))
        .unwrap();
        assert_eq!(core.work[0].due, first_due);
    }
    assert_eq!(
        core.take_ready(first_due)
            .unwrap()
            .request()
            .source
            .revision,
        20
    );
}

#[tokio::test]
async fn prefix_commit_waits_for_applied_completion_without_cancelling_session() {
    let provider = MockTranslator::new();
    let mut core = coordinator(TranslationQuality::Fast);
    core.ingest(source(1, 1, TranscriptKind::Partial, "source"))
        .unwrap();
    assert_eq!(core.commit_prefix(1, 0), Err(TranslationError::PrefixBusy));
    let job = core
        .take_ready(Instant::now() + Duration::from_secs(1))
        .unwrap();
    assert_eq!(core.commit_prefix(1, 0), Err(TranslationError::PrefixBusy));
    job.run(&provider).await.unwrap();
    // Even though its network job is finished, its queued results still own
    // the old prefix until the completion is consumed.
    assert_eq!(core.commit_prefix(1, 0), Err(TranslationError::PrefixBusy));
    assert_eq!(core.cancellation().reason(), None);
    core.drain_updates().unwrap();
    assert_eq!(core.timeline().items()[0].stable_prefix_bytes, 0);
    core.commit_prefix(1, "測試".len()).unwrap();
    core.ingest(source(1, 2, TranscriptKind::Final, "changed source"))
        .unwrap();
    let job = core.take_ready(Instant::now()).unwrap();
    assert_eq!(job.request().committed_prefix.as_ref(), "測試");
    assert_eq!(
        core.commit_prefix(1, "測試翻譯".len()),
        Err(TranslationError::PrefixBusy)
    );
    job.run(&provider).await.unwrap();
    core.drain_updates().unwrap();
    assert_eq!(core.timeline().items()[0].stability, Stability::Final);
    assert_eq!(core.cancellation().reason(), None);
}

#[tokio::test]
async fn rejected_or_cancelled_prefix_commit_does_not_mutate_timeline() {
    let mut core = coordinator(TranslationQuality::Fast);
    core.ingest(source(1, 1, TranscriptKind::Partial, "source"))
        .unwrap();
    core.take_ready(Instant::now() + Duration::from_secs(1))
        .unwrap()
        .run(&MockTranslator::new())
        .await
        .unwrap();
    core.drain_updates().unwrap();
    for (utterance, bytes) in [(1, 1), (1, MAX_TRANSCRIPT_BYTES + 1), (2, 0)] {
        assert_eq!(
            core.commit_prefix(utterance, bytes),
            Err(TranslationError::InvalidData)
        );
        assert_eq!(core.timeline().items()[0].stable_prefix_bytes, 0);
        assert_eq!(core.timeline().items()[0].stability, Stability::Tentative);
    }
    core.cancellation().cancel();
    assert_eq!(
        core.commit_prefix(1, "測試".len()),
        Err(TranslationError::Cancelled)
    );
    assert_eq!(core.timeline().items()[0].stable_prefix_bytes, 0);
}

#[tokio::test]
async fn superseded_provider_failure_does_not_fail_the_live_session() {
    struct CancelThenFail;
    impl Translator for CancelThenFail {
        fn capabilities(&self) -> TranslationCapabilities {
            caps()
        }
        fn translate<'a>(
            &'a self,
            _: &'a TranslationRequest,
            _: &'a mut TranslationEmitter,
            cancel: TranslationCancellation,
        ) -> TranslationFuture<'a> {
            Box::pin(async move {
                cancel.cancel();
                Err(TranslationError::Network)
            })
        }
    }
    let mut core = coordinator(TranslationQuality::Standard);
    core.ingest(source(1, 1, TranscriptKind::Final, "synthetic source"))
        .unwrap();
    assert_eq!(
        core.take_ready(Instant::now())
            .unwrap()
            .run(&CancelThenFail)
            .await,
        Err(TranslationError::Cancelled)
    );
    assert_eq!(core.cancellation().reason(), None);
    assert_eq!(core.active_requests(), 0);
    assert_eq!(core.drain_updates().unwrap(), 0);
}

#[test]
fn request_identity_exhaustion_fails_without_leaking_permits() {
    let mut core = coordinator(TranslationQuality::Standard);
    core.ingest(source(1, 1, TranscriptKind::Final, "synthetic source"))
        .unwrap();
    core.next_job_id = u64::MAX;
    assert!(core.take_ready(Instant::now()).is_none());
    assert_eq!(
        core.cancellation().reason(),
        Some(TranslationError::InvalidData)
    );
    assert_eq!(core.active_requests(), 0);
}

#[test]
fn phase10_unpolled_job_holds_a_permit_without_claiming_a_provider_operation() {
    let metrics = LiveDiagnostics::new(identity());
    let mut core = TranslationCoordinator::diagnosed(
        identity(),
        TranslationQuality::Standard,
        caps(),
        Some(metrics.clone()),
    )
    .unwrap();
    core.ingest(source(1, 1, TranscriptKind::Final, "explicit mock source"))
        .unwrap();
    let job = core.take_ready(Instant::now()).unwrap();
    assert_eq!(core.active_requests(), 1);
    assert_eq!(metrics.gauge(Activity::TranslationPermits).current, 1);
    assert_eq!(metrics.gauge(Activity::TranslationOperation).current, 0);
    drop(core);
    // Dropping the coordinator cancels the caller-owned job; the permit is
    // still genuinely held until that job is dropped, even if never polled.
    assert_eq!(metrics.count(Count::FinalSegmentsUnfinishedAtStop), 1);
    assert_eq!(metrics.gauge(Activity::TranslationPermits).current, 1);
    drop(job);
    assert_eq!(metrics.gauge(Activity::TranslationPermits).current, 0);
    assert_eq!(
        metrics
            .distribution(crate::performance::live::Latency::TranslationRequestFinal)
            .samples,
        0
    );
}

#[tokio::test]
async fn phase10_real_emitter_and_consumer_share_all_successful_output_markers() {
    let metrics = LiveDiagnostics::new(identity());
    let mut core = TranslationCoordinator::diagnosed(
        identity(),
        TranslationQuality::Standard,
        caps(),
        Some(metrics.clone()),
    )
    .unwrap();
    core.ingest(source(1, 1, TranscriptKind::Final, "explicit mock source"))
        .unwrap();
    core.take_ready(Instant::now())
        .unwrap()
        .run(&MockTranslator::new())
        .await
        .unwrap();
    assert_eq!(core.drain_updates().unwrap(), 3);
    assert_eq!(
        metrics
            .distribution(crate::performance::live::Latency::TranslationApply)
            .samples,
        3
    );
    assert_eq!(metrics.count(Count::CorrelationMissing), 0);
    assert_eq!(metrics.gauge(Activity::TranslationOperation).current, 0);
    assert_eq!(metrics.gauge(Activity::TranslationPermits).current, 0);
}
