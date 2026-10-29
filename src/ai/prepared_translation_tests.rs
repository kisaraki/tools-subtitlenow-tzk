//! Synthetic documents and explicit offline adapters; no media, keys or network.
use super::*;
use crate::{
    ai::{
        speech::StreamIdentity,
        translation::{SessionGlossary, TranslationCapabilities, TranslationFuture},
    },
    audio::capture::SessionId,
    resolver::contracts::{MAX_TEXT_CUES, TextCue},
};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::sync::{mpsc, watch};

#[derive(Clone, Copy)]
enum Mode {
    Immediate,
    Gated,
    FailGated,
    Pending,
    Empty,
    Controls,
    Oversized,
    Expanded,
    Partials,
    SlowBeyondLive,
    CancelRoot,
    CancelRequest,
    SyncPanic,
    FuturePanic,
    CapabilityPanic,
}
struct Seen {
    source: TranscriptEvent,
    quality: TranslationQuality,
    pass: TranslationPass,
    stable: Vec<StableContext>,
    glossary: Vec<GlossaryEntry>,
    draft: Option<Box<str>>,
    previous: Vec<Box<str>>,
    following: Vec<Box<str>>,
    neighboring: Vec<PreparedDraftContext>,
}
struct Entered {
    sequence: usize,
    cancel: TranslationCancellation,
}
struct Mock {
    mode: Mode,
    concurrency: usize,
    seen: Mutex<Vec<Seen>>,
    active: Arc<AtomicUsize>,
    peak: AtomicUsize,
    entered: mpsc::Sender<Entered>,
    finished: mpsc::Sender<usize>,
    gates: Vec<watch::Sender<bool>>,
    root_cancel: ResolverCancellation,
}
struct Active(Arc<AtomicUsize>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
impl Translator for Mock {
    fn capabilities(&self) -> TranslationCapabilities {
        if matches!(self.mode, Mode::CapabilityPanic) {
            panic!("synthetic capability panic");
        }
        TranslationCapabilities {
            streaming: true,
            cancellation: true,
            supported_languages: [true; 5],
            max_concurrent: self.concurrency,
        }
    }
    fn translate<'a>(
        &'a self,
        request: &'a TranslationRequest,
        output: &'a mut TranslationEmitter,
        cancel: TranslationCancellation,
    ) -> TranslationFuture<'a> {
        if matches!(self.mode, Mode::SyncPanic) {
            let _ = self.entered.try_send(Entered {
                sequence: request.source.sequence as usize,
                cancel,
            });
            panic!("synthetic synchronous provider panic");
        }
        Box::pin(async move {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            let _active = Active(self.active.clone());
            self.peak.fetch_max(active, Ordering::SeqCst);
            let context = request.prepared_context.as_ref().unwrap();
            self.seen.lock().unwrap().push(Seen {
                source: request.source.as_ref().clone(),
                quality: request.quality,
                pass: request.pass,
                stable: request.previous_stable.clone(),
                glossary: request.glossary.clone(),
                draft: request.draft.clone(),
                previous: context.previous_source.clone(),
                following: context.following_source.clone(),
                neighboring: context
                    .neighboring_drafts
                    .iter()
                    .map(|draft| PreparedDraftContext {
                        sequence: draft.sequence,
                        source: draft.source.clone(),
                        translation: draft.translation.clone(),
                    })
                    .collect(),
            });
            let index = request.source.sequence as usize;
            let _ = self.entered.try_send(Entered {
                sequence: index,
                cancel: cancel.clone(),
            });
            assert_eq!(request.target_language(), "zh-TW");
            assert_eq!(request.source.kind, TranscriptKind::Final);
            assert!(request.committed_prefix.is_empty());
            match self.mode {
                Mode::Gated | Mode::FailGated => {
                    let mut gate = self.gates[index].subscribe();
                    while !*gate.borrow_and_update() {
                        gate.changed().await.unwrap();
                    }
                    if matches!(self.mode, Mode::FailGated) && index == 1 {
                        return Err(TranslationError::Refused);
                    }
                }
                Mode::Pending => std::future::pending::<()>().await,
                Mode::Empty => return Ok(" \n".into()),
                Mode::Controls => return Ok("invalid\0translation".into()),
                Mode::Oversized => return Ok("x".repeat(MAX_CUE_BYTES + 1).into()),
                Mode::Expanded => return Ok("x".repeat(MAX_CUE_BYTES).into()),
                Mode::FuturePanic => panic!("synthetic asynchronous provider panic"),
                Mode::SlowBeyondLive => {
                    tokio::time::sleep(
                        crate::ai::translation::MAX_TRANSLATION_AGE + Duration::from_millis(20),
                    )
                    .await;
                }
                Mode::CancelRoot => self.root_cancel.cancel(),
                Mode::CancelRequest => cancel.cancel(),
                Mode::Partials => {
                    for _ in 0..100 {
                        output.emit("合成暫定譯文")?;
                    }
                }
                _ => {}
            }
            let _ = self.finished.try_send(index);
            Ok(format!(
                "{}{index}",
                if request.pass == TranslationPass::Review {
                    "校訂"
                } else {
                    "譯文"
                }
            )
            .into())
        })
    }
}
fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(41),
        generation: 1,
    }
}
fn document(count: usize) -> ExistingText {
    ExistingText {
        identity: identity(),
        content_id: "synthetic-prepared-document".into(),
        source: SubtitleSource::ExistingSubtitle,
        language: TextLanguage::Source(SpeechLanguage::English),
        cues: (0..count)
            .map(|index| TextCue {
                start_us: Some(600_000_000 + index as u64 * 60_000_000),
                duration_us: Some(10_000_000),
                text: format!("source {index}").into(),
            })
            .collect(),
    }
}
fn mock(
    mode: Mode,
    concurrency: usize,
    count: usize,
) -> (Mock, mpsc::Receiver<Entered>, mpsc::Receiver<usize>) {
    let (entered, receiver) = mpsc::channel(64);
    let (finished, completions) = mpsc::channel(64);
    (
        Mock {
            mode,
            concurrency,
            seen: Mutex::new(Vec::new()),
            active: Arc::new(AtomicUsize::new(0)),
            peak: AtomicUsize::new(0),
            entered,
            finished,
            gates: (0..count).map(|_| watch::channel(false).0).collect(),
            root_cancel: ResolverCancellation::default(),
        },
        receiver,
        completions,
    )
}
async fn observe<F: Future, T>(work: &mut Pin<Box<F>>, receiver: &mut mpsc::Receiver<T>) -> T {
    tokio::select! {
        item = receiver.recv() => item.unwrap(),
        _ = work => panic!("document finished before expected synthetic event"),
    }
}

#[tokio::test]
async fn out_of_order_batches_restore_order_and_bound_active_requests() {
    let original = document(6);
    let glossary = SessionGlossary::new(identity());
    let cancel = ResolverCancellation::default();
    let (provider, mut entered, mut finished) = mock(Mode::Gated, 3, 6);
    let mut work = Box::pin(translate_document(
        &original,
        TranslationQuality::Fast,
        &glossary,
        &provider,
        &cancel,
    ));
    let mut tokens = Vec::new();
    for index in 0..3 {
        let event = observe(&mut work, &mut entered).await;
        assert_eq!(event.sequence, index);
        tokens.push(event.cancel);
    }
    assert_eq!(provider.active.load(Ordering::SeqCst), 3);
    provider.gates[2].send_replace(true);
    assert_eq!(observe(&mut work, &mut finished).await, 2);
    assert!(entered.try_recv().is_err());
    for index in [0, 1] {
        provider.gates[index].send_replace(true);
        assert_eq!(observe(&mut work, &mut finished).await, index);
    }
    for index in 3..6 {
        let event = observe(&mut work, &mut entered).await;
        assert_eq!(event.sequence, index);
        tokens.push(event.cancel);
    }
    for index in [5, 4] {
        provider.gates[index].send_replace(true);
        assert_eq!(observe(&mut work, &mut finished).await, index);
    }
    provider.gates[3].send_replace(true);
    let result = work.await.unwrap();
    assert_eq!(
        result.iter().map(AsRef::as_ref).collect::<Vec<&str>>(),
        ["譯文0", "譯文1", "譯文2", "譯文3", "譯文4", "譯文5"]
    );
    assert_eq!(provider.peak.load(Ordering::SeqCst), 3);
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
    assert!(
        tokens
            .iter()
            .all(|token| token.reason() == Some(TranslationError::Cancelled))
    );
}

#[tokio::test]
async fn review_receives_whole_draft_neighbors_and_cross_batch_context() {
    let original = document(7);
    let mut glossary = SessionGlossary::new(identity());
    glossary.set(identity(), "source", "來源").unwrap();
    let (provider, _, _) = mock(Mode::Immediate, 2, 7);
    let result = translate_document(
        &original,
        TranslationQuality::Standard,
        &glossary,
        &provider,
        &ResolverCancellation::default(),
    )
    .await
    .unwrap();
    assert_eq!(result[6].as_ref(), "校訂6");
    let calls = provider.seen.lock().unwrap();
    assert_eq!(calls.len(), 14);
    assert!(
        calls[..7]
            .iter()
            .all(|call| call.pass == TranslationPass::Draft)
    );
    assert!(
        calls[7..]
            .iter()
            .all(|call| call.pass == TranslationPass::Review)
    );
    let draft2 = &calls[2];
    assert_eq!(
        draft2
            .stable
            .iter()
            .map(|cue| cue.sequence)
            .collect::<Vec<_>>(),
        [0, 1]
    );
    assert_eq!(
        draft2
            .previous
            .iter()
            .map(AsRef::as_ref)
            .collect::<Vec<&str>>(),
        ["source 0", "source 1"]
    );
    assert_eq!(
        draft2
            .following
            .iter()
            .map(AsRef::as_ref)
            .collect::<Vec<&str>>(),
        ["source 3", "source 4", "source 5"]
    );
    assert_eq!(
        calls[4]
            .stable
            .iter()
            .map(|cue| cue.sequence)
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(calls[5].previous.last().unwrap().as_ref(), "source 4");
    let review1 = &calls[8];
    assert_eq!(review1.draft.as_deref(), Some("譯文1"));
    assert_eq!(
        review1
            .neighboring
            .iter()
            .map(|cue| cue.sequence)
            .collect::<Vec<_>>(),
        [0, 2, 3, 4]
    );
    assert_eq!(
        review1.neighboring.last().unwrap().translation.as_ref(),
        "譯文4"
    );
    assert_eq!(calls[11].stable[0].translation.as_ref(), "校訂1");
    assert!(calls.iter().all(|call| call.glossary.len() == 1
        && call.glossary[0].source.as_ref() == "source"
        && call.glossary[0].taiwan_translation.as_ref() == "來源"));
}

#[tokio::test]
async fn quality_modes_change_pass_count_and_actual_context_window() {
    for (quality, passes, window) in [
        (TranslationQuality::Fast, 1, 1),
        (TranslationQuality::Standard, 2, 3),
        (TranslationQuality::High, 2, 8),
    ] {
        let original = document(12);
        let (provider, _, _) = mock(Mode::Immediate, 4, 12);
        translate_document(
            &original,
            quality,
            &SessionGlossary::new(identity()),
            &provider,
            &ResolverCancellation::default(),
        )
        .await
        .unwrap();
        let calls = provider.seen.lock().unwrap();
        assert_eq!(calls.len(), 12 * passes);
        assert!(calls.iter().all(|call| call.quality == quality
            && call.previous.len() <= window
            && call.following.len() <= window
            && call.stable.len() <= window
            && call.neighboring.len() <= 2 * window));
        assert_eq!(calls[9].previous.len(), window);
        assert_eq!(calls[0].following.len(), window);
        assert_eq!(calls[0].neighboring.len(), 0);
        if passes == 2 {
            assert_eq!(calls[12].neighboring.len(), window);
        }
    }
}

#[tokio::test]
async fn five_languages_preserve_subtitle_time_sequence_and_session() {
    for language in SpeechLanguage::ALL {
        let mut original = document(5);
        original.language = TextLanguage::Source(language);
        let (provider, _, _) = mock(Mode::Immediate, 2, 5);
        translate_document(
            &original,
            TranslationQuality::Fast,
            &SessionGlossary::new(identity()),
            &provider,
            &ResolverCancellation::default(),
        )
        .await
        .unwrap();
        for call in provider.seen.lock().unwrap().iter() {
            let index = call.source.sequence as usize;
            assert_eq!(call.source.identity, identity());
            assert_eq!(call.source.utterance, index as u64 + 1);
            assert_eq!(call.source.revision, 1);
            assert_eq!(call.source.start_us, original.cues[index].start_us);
            assert_eq!(call.source.duration_us, original.cues[index].duration_us);
            assert_eq!(call.source.text, original.cues[index].text);
            assert_eq!(call.source.language, Some(language));
        }
    }
}

#[tokio::test]
async fn prepared_document_and_requests_do_not_use_live_window_or_expiry() {
    let original = document(1100);
    let (provider, _, _) = mock(Mode::Immediate, 4, 1100);
    let result = translate_document(
        &original,
        TranslationQuality::Fast,
        &SessionGlossary::new(identity()),
        &provider,
        &ResolverCancellation::default(),
    )
    .await
    .unwrap();
    assert_eq!(result.len(), 1100);
    assert_eq!(result[1099].as_ref(), "譯文1099");
    let one = document(1);
    let (slow, _, _) = mock(Mode::SlowBeyondLive, 1, 1);
    let result = translate_document(
        &one,
        TranslationQuality::Fast,
        &SessionGlossary::new(identity()),
        &slow,
        &ResolverCancellation::default(),
    )
    .await
    .unwrap();
    assert_eq!(result[0].as_ref(), "譯文0");
}

#[tokio::test]
async fn session_stop_cancels_every_started_uncooperative_request() {
    let original = document(8);
    let glossary = SessionGlossary::new(identity());
    let (provider, mut entered, _) = mock(Mode::Pending, 4, 8);
    let cancel = ResolverCancellation::default();
    let mut work = Box::pin(translate_document(
        &original,
        TranslationQuality::High,
        &glossary,
        &provider,
        &cancel,
    ));
    let mut tokens = Vec::new();
    for _ in 0..4 {
        tokens.push(observe(&mut work, &mut entered).await.cancel);
    }
    cancel.cancel();
    assert_eq!(work.await.unwrap_err(), TranslationError::Cancelled);
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
    assert_eq!(provider.seen.lock().unwrap().len(), 4);
    assert!(
        tokens
            .iter()
            .all(|token| token.reason() == Some(TranslationError::Cancelled))
    );
}

#[tokio::test]
async fn dropping_document_future_releases_requests_and_cancels_tokens() {
    let original = document(5);
    let glossary = SessionGlossary::new(identity());
    let (provider, mut entered, _) = mock(Mode::Pending, 3, 5);
    let cancel = ResolverCancellation::default();
    let mut work = Box::pin(translate_document(
        &original,
        TranslationQuality::Standard,
        &glossary,
        &provider,
        &cancel,
    ));
    let mut tokens = Vec::new();
    for _ in 0..3 {
        tokens.push(observe(&mut work, &mut entered).await.cancel);
    }
    drop(work);
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
    assert!(!cancel.is_cancelled());
    assert!(
        tokens
            .iter()
            .all(|token| token.reason() == Some(TranslationError::Cancelled))
    );
}

#[tokio::test]
async fn provider_failure_abandons_batch_and_never_starts_review_or_next_batch() {
    let original = document(9);
    let glossary = SessionGlossary::new(identity());
    let cancel = ResolverCancellation::default();
    let (provider, mut entered, _) = mock(Mode::FailGated, 3, 9);
    let mut work = Box::pin(translate_document(
        &original,
        TranslationQuality::Standard,
        &glossary,
        &provider,
        &cancel,
    ));
    let mut tokens = Vec::new();
    for _ in 0..3 {
        tokens.push(observe(&mut work, &mut entered).await.cancel);
    }
    provider.gates[1].send_replace(true);
    assert_eq!(work.await.unwrap_err(), TranslationError::Refused);
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
    assert_eq!(provider.seen.lock().unwrap().len(), 3);
    assert!(
        tokens
            .iter()
            .all(|token| token.reason() == Some(TranslationError::Cancelled))
    );
}

#[tokio::test]
async fn malformed_provider_output_and_total_expansion_never_return_partial_document() {
    for mode in [Mode::Empty, Mode::Controls, Mode::Oversized] {
        let (provider, _, _) = mock(mode, 2, 3);
        assert_eq!(
            translate_document(
                &document(3),
                TranslationQuality::High,
                &SessionGlossary::new(identity()),
                &provider,
                &ResolverCancellation::default()
            )
            .await
            .unwrap_err(),
            TranslationError::InvalidData
        );
        assert!(
            provider
                .seen
                .lock()
                .unwrap()
                .iter()
                .all(|call| call.pass == TranslationPass::Draft)
        );
    }
    let (provider, _, _) = mock(Mode::Expanded, 4, 513);
    assert_eq!(
        translate_document(
            &document(513),
            TranslationQuality::Fast,
            &SessionGlossary::new(identity()),
            &provider,
            &ResolverCancellation::default()
        )
        .await
        .unwrap_err(),
        TranslationError::InvalidData
    );
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn invalid_document_glossary_or_capability_starts_no_provider_request() {
    let (provider, _, _) = mock(Mode::Immediate, 2, 1);
    let cancel = ResolverCancellation::default();
    let glossary = SessionGlossary::new(identity());
    for mut original in [document(0), document(MAX_TEXT_CUES + 1), document(1)] {
        if original.cues.len() == 1 {
            original.cues[0].duration_us = Some(0);
        }
        assert_eq!(
            translate_document(
                &original,
                TranslationQuality::Standard,
                &glossary,
                &provider,
                &cancel
            )
            .await
            .unwrap_err(),
            TranslationError::InvalidData
        );
    }
    let mut wrong = identity();
    wrong.generation += 1;
    assert_eq!(
        translate_document(
            &document(1),
            TranslationQuality::Fast,
            &SessionGlossary::new(wrong),
            &provider,
            &cancel
        )
        .await
        .unwrap_err(),
        TranslationError::InvalidData
    );
    let mut chinese = document(1);
    chinese.language = TextLanguage::TaiwanTraditional;
    assert_eq!(
        translate_document(
            &chinese,
            TranslationQuality::Fast,
            &glossary,
            &provider,
            &cancel
        )
        .await
        .unwrap_err(),
        TranslationError::UnsupportedCapability
    );
    assert!(provider.seen.lock().unwrap().is_empty());
    for concurrency in [0, 5] {
        let (invalid, _, _) = mock(Mode::Immediate, concurrency, 1);
        assert_eq!(
            translate_document(
                &document(1),
                TranslationQuality::Fast,
                &glossary,
                &invalid,
                &cancel
            )
            .await
            .unwrap_err(),
            TranslationError::UnsupportedCapability
        );
        assert!(invalid.seen.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn provider_panics_are_typed_and_drop_owned_request_tokens() {
    for mode in [Mode::SyncPanic, Mode::FuturePanic, Mode::CapabilityPanic] {
        let (provider, mut entered, _) = mock(mode, 2, 2);
        assert_eq!(
            translate_document(
                &document(2),
                TranslationQuality::High,
                &SessionGlossary::new(identity()),
                &provider,
                &ResolverCancellation::default()
            )
            .await
            .unwrap_err(),
            TranslationError::InvalidData
        );
        assert_eq!(provider.active.load(Ordering::SeqCst), 0);
        while let Ok(event) = entered.try_recv() {
            assert_eq!(event.cancel.reason(), Some(TranslationError::Cancelled));
        }
    }
}

#[tokio::test]
async fn prepared_partials_are_validated_without_an_extra_update_queue() {
    let (provider, _, _) = mock(Mode::Partials, 2, 3);
    let result = translate_document(
        &document(3),
        TranslationQuality::Standard,
        &SessionGlossary::new(identity()),
        &provider,
        &ResolverCancellation::default(),
    )
    .await
    .unwrap();
    assert_eq!(result[2].as_ref(), "校訂2");
}

#[tokio::test]
async fn cancellation_precedes_ready_provider_completion_and_pre_cancel_starts_nothing() {
    let (provider, _, _) = mock(Mode::CancelRoot, 3, 5);
    assert_eq!(
        translate_document(
            &document(5),
            TranslationQuality::High,
            &SessionGlossary::new(identity()),
            &provider,
            &provider.root_cancel
        )
        .await
        .unwrap_err(),
        TranslationError::Cancelled
    );
    assert_eq!(provider.seen.lock().unwrap().len(), 1);
    let (fresh, _, _) = mock(Mode::Immediate, 3, 5);
    let cancel = ResolverCancellation::default();
    cancel.cancel();
    assert_eq!(
        translate_document(
            &document(5),
            TranslationQuality::High,
            &SessionGlossary::new(identity()),
            &fresh,
            &cancel
        )
        .await
        .unwrap_err(),
        TranslationError::Cancelled
    );
    assert!(fresh.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn provider_cancel_precedes_its_uncooperative_ready_result() {
    let (provider, _, _) = mock(Mode::CancelRequest, 3, 5);
    assert_eq!(
        translate_document(
            &document(5),
            TranslationQuality::High,
            &SessionGlossary::new(identity()),
            &provider,
            &ResolverCancellation::default()
        )
        .await
        .unwrap_err(),
        TranslationError::Cancelled
    );
    assert_eq!(provider.seen.lock().unwrap().len(), 1);
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn request_timeout_releases_uncooperative_future_and_cancels_token() {
    let original = document(1);
    let settings = PassSettings {
        quality: TranslationQuality::Fast,
        language: SpeechLanguage::English,
        concurrency: 1,
        context: 1,
    };
    let request = make_request(
        PreparedTranslationInput::Existing(&original),
        settings,
        &[],
        &[],
        None,
        0,
        None,
    );
    let (provider, mut entered, _) = mock(Mode::Pending, 1, 1);
    let (_, result) = run_request(
        0,
        request,
        &provider,
        &ResolverCancellation::default(),
        Duration::from_millis(20),
    )
    .await;
    assert_eq!(result.unwrap_err(), TranslationError::Timeout);
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
    assert_eq!(
        entered.try_recv().unwrap().cancel.reason(),
        Some(TranslationError::Cancelled)
    );
}
