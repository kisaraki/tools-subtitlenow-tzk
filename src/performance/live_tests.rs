use super::*;
use crate::{
    ai::speech::SpeechLanguage,
    audio::capture::{AudioStamp, BLOCK_FRAMES, SessionId},
    subtitle::timeline::{Stability, TranscriptTimeline},
};
use std::time::Duration;
fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(98_001),
        generation: 2,
    }
}
pub(crate) fn diagnostic_fixture() -> (Arc<LiveDiagnostics>, TranscriptTimeline) {
    let metrics = LiveDiagnostics::new(identity());
    let timeline = translated(&metrics, event(1, 1, TranscriptKind::Final));
    (metrics, timeline)
}
fn event(utterance: u64, revision: u64, kind: TranscriptKind) -> TranscriptEvent {
    TranscriptEvent {
        identity: identity(),
        utterance,
        sequence: utterance,
        revision,
        start_us: Some(0),
        duration_us: Some(20_000),
        language: Some(SpeechLanguage::English),
        kind,
        text: "private synthetic text never retained in metrics".into(),
    }
}
fn audio() -> NormalizedAudio {
    NormalizedAudio {
        stamp: AudioStamp {
            session: identity().session,
            sequence: 1,
            frame_position: Some(0),
            qpc_100ns: None,
            received: Instant::now() - Duration::from_millis(2),
            discontinuity: false,
        },
        frames: BLOCK_FRAMES,
        samples: [0.0; BLOCK_FRAMES],
    }
}
fn translated(metrics: &Arc<LiveDiagnostics>, source: TranscriptEvent) -> TranscriptTimeline {
    metrics.observe_audio(&audio());
    metrics.speech_emitted(&source);
    let mut timeline = TranscriptTimeline::new(identity());
    timeline.ingest(source.clone()).unwrap();
    metrics.translation_emitted(
        &source,
        TranslationObservation {
            job: 1,
            pass: TranslationPass::Draft,
            revision: 1,
            started: Instant::now(),
            first: true,
            complete: true,
        },
    );
    let update = TranslationUpdate {
        identity: identity(),
        utterance: source.utterance,
        source_revision: source.revision,
        pass: TranslationPass::Draft,
        revision: 1,
        text: "臺灣合成測試".into(),
        stability: Stability::Final,
        stable_prefix_bytes: "臺灣合成測試".len(),
        complete: true,
        job_id: 1,
    };
    timeline.apply(&update).unwrap();
    metrics.translation_applied(&update, &source);
    timeline
}
#[test]
fn phase10_correlation_first_partial_final_and_duplicate_revision_are_separate() {
    let metrics = LiveDiagnostics::new(identity());
    metrics.observe_audio(&audio());
    let partial = event(1, 1, TranscriptKind::Partial);
    metrics.speech_emitted(&partial);
    metrics.speech_emitted(&partial);
    metrics.speech_emitted(&event(1, 2, TranscriptKind::Final));
    metrics.speech_emitted(&event(1, 3, TranscriptKind::Partial));
    assert_eq!(metrics.distribution(Latency::SpeechFirstPartial).samples, 1);
    assert_eq!(metrics.distribution(Latency::SpeechFinal).samples, 1);
}
#[test]
fn phase10_fast_final_does_not_erase_queued_partial_timing() {
    let metrics = LiveDiagnostics::new(identity());
    metrics.observe_audio(&audio());
    let partial = event(1, 1, TranscriptKind::Partial);
    let final_event = event(1, 2, TranscriptKind::Final);
    metrics.speech_emitted(&partial);
    metrics.speech_emitted(&final_event);
    metrics.speech_consumed(&partial);
    metrics.speech_consumed(&final_event);
    assert_eq!(metrics.distribution(Latency::SpeechOutputQueue).samples, 2);
}
#[test]
fn phase10_correlation_missing_audio_never_uses_session_start_as_fake_origin() {
    let metrics = LiveDiagnostics::new(identity());
    metrics.speech_emitted(&event(1, 1, TranscriptKind::Final));
    assert_eq!(metrics.distribution(Latency::SpeechFinal).samples, 0);
    assert_eq!(metrics.count(Count::CorrelationMissing), 1);
}
#[test]
fn phase10_render_receipts_deduplicate_clones_and_repeated_publish() {
    let metrics = LiveDiagnostics::new(identity());
    let timeline = translated(&metrics, event(1, 1, TranscriptKind::Final));
    let first = metrics.ticket(&timeline.items()[0]).unwrap();
    let second = metrics.ticket(&timeline.items()[0]).unwrap();
    first.submitted();
    first.submitted();
    second.submitted();
    assert_eq!(
        metrics
            .distribution(Latency::TranslationToRenderSubmission)
            .samples,
        1
    );
    assert_eq!(
        metrics
            .distribution(Latency::CaptureToFirstUsefulSubmission)
            .samples,
        1
    );
}
#[test]
fn phase10_retired_session_rejects_delayed_ui_receipt_and_new_audio() {
    let metrics = LiveDiagnostics::new(identity());
    let timeline = translated(&metrics, event(1, 1, TranscriptKind::Final));
    let ticket = metrics.ticket(&timeline.items()[0]).unwrap();
    metrics.retire();
    ticket.submitted();
    metrics.observe_audio(&audio());
    assert_eq!(
        metrics
            .distribution(Latency::TranslationToRenderSubmission)
            .samples,
        0
    );
    assert!(metrics.lock().unwrap().anchors.is_empty());
}
#[test]
fn phase10_session_and_generation_mismatch_never_correlates() {
    let metrics = LiveDiagnostics::new(identity());
    metrics.observe_audio(&audio());
    let mut source = event(1, 1, TranscriptKind::Final);
    source.identity.generation += 1;
    metrics.speech_emitted(&source);
    assert_eq!(metrics.distribution(Latency::SpeechFinal).samples, 0);
    assert!(metrics.lock().unwrap().entries.is_empty());
}
#[test]
fn phase10_numeric_metadata_stays_bounded_during_long_sessions() {
    let metrics = LiveDiagnostics::new(identity());
    for _ in 0..AUDIO_ANCHORS + 20 {
        metrics.observe_audio(&audio());
    }
    for index in 1..=CORRELATIONS as u64 + 10 {
        metrics.speech_emitted(&event(index, 1, TranscriptKind::Final));
    }
    let state = metrics.lock().unwrap();
    assert_eq!(state.anchors.len(), AUDIO_ANCHORS);
    assert_eq!(state.entries.len(), CORRELATIONS);
    assert_eq!(metrics.count(Count::CorrelationEvicted), 10);
}
#[test]
fn phase10_operation_drop_tracks_peak_and_cancel_cleanup_without_task_leaks() {
    let metrics = LiveDiagnostics::new(identity());
    let one = metrics.operation(Activity::TranslationOperation);
    let two = metrics.operation(Activity::TranslationOperation);
    assert_eq!(metrics.gauge(Activity::TranslationOperation).current, 2);
    drop(two);
    drop(one);
    assert_eq!(metrics.gauge(Activity::TranslationOperation).current, 0);
    assert_eq!(
        metrics
            .gauge(Activity::TranslationOperation)
            .observed_high_water,
        2
    );
}
#[test]
fn phase10_new_revision_cannot_claim_previous_translation_was_presented() {
    let metrics = LiveDiagnostics::new(identity());
    let timeline = translated(&metrics, event(1, 1, TranscriptKind::Final));
    let ticket = metrics.ticket(&timeline.items()[0]).unwrap();
    metrics.lock().unwrap().entries[0].binding.revision = 2;
    ticket.submitted();
    assert_eq!(
        metrics
            .distribution(Latency::TranslationToRenderSubmission)
            .samples,
        0
    );
    assert_eq!(metrics.count(Count::UiReceiptRetired), 1);
}
#[test]
fn phase10_scope_drop_retires_only_its_session() {
    let first = LiveDiagnostics::new(identity());
    let second = LiveDiagnostics::new(identity());
    {
        let _scope = first.scope();
    }
    assert!(first.retired.load(Ordering::Acquire));
    assert!(!second.retired.load(Ordering::Acquire));
}
#[test]
fn phase10_ui_mailbox_coalesces_receipts_and_rejects_obsolete_frame_versions() {
    use crate::{
        ai::provider::SpeechProvider,
        ui::bridge::{UiSource, channel},
    };
    let metrics = LiveDiagnostics::new(identity());
    let timeline = translated(&metrics, event(1, 1, TranscriptKind::Final));
    let (mut publisher, mut inbox) = channel();
    publisher
        .begin(
            identity(),
            UiSource {
                pid: 1,
                provider: SpeechProvider::OpenAi,
            },
        )
        .unwrap();
    publisher.attach_diagnostics(Some(metrics.clone())).unwrap();
    publisher.publish(&timeline, true).unwrap();
    let old = inbox.poll().unwrap();
    publisher.publish(&timeline, true).unwrap();
    let current = inbox.poll().unwrap();
    inbox.render_submitted(old.version());
    assert_eq!(
        metrics
            .distribution(Latency::TranslationToRenderSubmission)
            .samples,
        0
    );
    inbox.render_submitted(current.version());
    inbox.render_submitted(current.version());
    assert_eq!(
        metrics
            .distribution(Latency::TranslationToRenderSubmission)
            .samples,
        1
    );
}
#[test]
fn phase10_ui_terminal_state_removes_receipt_and_cannot_report_old_translation() {
    use crate::{
        ai::provider::SpeechProvider,
        ui::bridge::{UiSource, UiStatus, channel},
    };
    let metrics = LiveDiagnostics::new(identity());
    let timeline = translated(&metrics, event(1, 1, TranscriptKind::Final));
    let (mut publisher, mut inbox) = channel();
    publisher
        .begin(
            identity(),
            UiSource {
                pid: 1,
                provider: SpeechProvider::OpenAi,
            },
        )
        .unwrap();
    publisher.attach_diagnostics(Some(metrics.clone())).unwrap();
    publisher.publish(&timeline, true).unwrap();
    let old = inbox.poll().unwrap();
    publisher.clear(UiStatus::Stopped).unwrap();
    inbox.render_submitted(old.version());
    assert_eq!(
        metrics
            .distribution(Latency::TranslationToRenderSubmission)
            .samples,
        0
    );
}
