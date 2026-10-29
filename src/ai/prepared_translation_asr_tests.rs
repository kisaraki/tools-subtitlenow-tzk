//! Sealed ASR transcripts and explicit mock translations. No subtitle
//! publication, file, key, service connection or playback evidence is created.
use super::*;
use crate::{
    ai::{
        prepared_speech::{
            MAX_ASR_CUES_PER_CHUNK, PreparedChunkStamp, PreparedSpeechChunk, PreparedSpeechCue,
            PreparedSpeechRequest, PreparedSpeechResponse,
        },
        prepared_transcript::PreparedTranscriptMerger,
        translation::{SessionGlossary, TranslationCapabilities, TranslationFuture},
    },
    audio::capture::SessionId,
    domain::SourceLanguage,
    media::prepared_audio::PreparedSampleRate,
};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::{Semaphore, mpsc};

fn identity() -> super::super::speech::StreamIdentity {
    super::super::speech::StreamIdentity {
        session: SessionId(908),
        generation: 3,
    }
}
fn selected(language: SpeechLanguage) -> SourceLanguage {
    match language {
        SpeechLanguage::English => SourceLanguage::English,
        SpeechLanguage::Japanese => SourceLanguage::Japanese,
        SpeechLanguage::French => SourceLanguage::French,
        SpeechLanguage::German => SourceLanguage::German,
        SpeechLanguage::Korean => SourceLanguage::Korean,
    }
}
fn transcript(language: SpeechLanguage, count: usize) -> PreparedTranscript {
    let mut merger = PreparedTranscriptMerger::new(identity(), selected(language)).unwrap();
    for batch in (0..count).step_by(MAX_ASR_CUES_PER_CHUNK) {
        let start = 1_000_000 + (batch / MAX_ASR_CUES_PER_CHUNK) as u64 * 10_000_000;
        let stamp = PreparedChunkStamp {
            identity: identity(),
            sequence: batch as u64 / MAX_ASR_CUES_PER_CHUNK as u64 + 1,
            run_id: 1,
            rate: PreparedSampleRate::Hz16000,
            audio_start_us: start,
            body_start_us: start,
            body_end_us: start + 10_000_000,
            discontinuity: false,
        };
        let request = PreparedSpeechRequest::new(
            PreparedSpeechChunk::new(stamp, vec![0; 160_000]).unwrap(),
            selected(language),
            Vec::new(),
        )
        .unwrap();
        merger.register(&request).unwrap();
        merger
            .complete(
                &request,
                PreparedSpeechResponse {
                    identity: identity(),
                    sequence: stamp.sequence,
                    run_id: 1,
                    language: Some(language),
                    cues: (batch..(batch + MAX_ASR_CUES_PER_CHUNK).min(count))
                        .map(|index| PreparedSpeechCue {
                            start_us: (index - batch) as u64 * 2_000,
                            duration_us: 1_000,
                            text: format!("ASR source {index}").into_boxed_str(),
                        })
                        .collect(),
                },
            )
            .unwrap();
    }
    merger.finish().unwrap()
}

#[derive(Clone, Copy)]
enum Mode {
    Immediate,
    Gated,
    Pending,
    Fail,
    Blank,
    Controls,
    Oversized,
    Expanded,
    Panic,
    CapabilityPanic,
    CancelSuccess,
}
struct Seen {
    source: TranscriptEvent,
    quality: TranslationQuality,
    pass: TranslationPass,
    previous: Vec<Box<str>>,
    following: Vec<Box<str>>,
    neighbors: Vec<(u64, Box<str>, Box<str>)>,
    stable: Vec<StableContext>,
    glossary: Vec<GlossaryEntry>,
    draft: Option<Box<str>>,
}
struct Mock {
    mode: Mode,
    concurrency: usize,
    seen: Mutex<Vec<Seen>>,
    tokens: Mutex<Vec<TranslationCancellation>>,
    active: AtomicUsize,
    peak: AtomicUsize,
    gates: Vec<Semaphore>,
    entered: mpsc::Sender<usize>,
    root: ResolverCancellation,
}
struct Active<'a>(&'a AtomicUsize);
impl Drop for Active<'_> {
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
        _: &'a mut TranslationEmitter,
        cancel: TranslationCancellation,
    ) -> TranslationFuture<'a> {
        Box::pin(async move {
            let count = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            let _active = Active(&self.active);
            self.peak.fetch_max(count, Ordering::SeqCst);
            let context = request.prepared_context.as_ref().unwrap();
            let index = request.source.sequence as usize;
            self.tokens.lock().unwrap().push(cancel);
            self.seen.lock().unwrap().push(Seen {
                source: request.source.as_ref().clone(),
                quality: request.quality,
                pass: request.pass,
                previous: context.previous_source.clone(),
                following: context.following_source.clone(),
                neighbors: context
                    .neighboring_drafts
                    .iter()
                    .map(|draft| {
                        (
                            draft.sequence,
                            draft.source.clone(),
                            draft.translation.clone(),
                        )
                    })
                    .collect(),
                stable: request.previous_stable.clone(),
                glossary: request.glossary.clone(),
                draft: request.draft.clone(),
            });
            let _ = self.entered.try_send(index);
            match self.mode {
                Mode::Gated => self.gates[index].acquire().await.unwrap().forget(),
                Mode::Pending => std::future::pending::<()>().await,
                Mode::Fail if index == 1 => return Err(TranslationError::Refused),
                Mode::Blank => return Ok(" \n".into()),
                Mode::Controls => return Ok("bad\0text".into()),
                Mode::Oversized => return Ok("x".repeat(MAX_CUE_BYTES + 1).into_boxed_str()),
                Mode::Expanded => return Ok("x".repeat(MAX_CUE_BYTES).into_boxed_str()),
                Mode::Panic => panic!("synthetic request panic"),
                Mode::CancelSuccess => self.root.cancel(),
                _ => {}
            }
            Ok(format!(
                "{} {index}",
                if request.pass == TranslationPass::Draft {
                    "草稿"
                } else {
                    "校訂"
                }
            )
            .into_boxed_str())
        })
    }
}
fn mock(
    mode: Mode,
    concurrency: usize,
    count: usize,
    root: ResolverCancellation,
) -> (Mock, mpsc::Receiver<usize>) {
    let (entered, receiver) = mpsc::channel(32);
    (
        Mock {
            mode,
            concurrency,
            seen: Mutex::new(Vec::new()),
            tokens: Mutex::new(Vec::new()),
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            gates: (0..count).map(|_| Semaphore::new(0)).collect(),
            entered,
            root,
        },
        receiver,
    )
}

#[tokio::test]
async fn five_asr_languages_keep_original_identity_native_time_and_source_text() {
    for language in SpeechLanguage::ALL {
        let original = transcript(language, 3);
        let before: Vec<_> = original
            .cues()
            .iter()
            .map(|cue| (cue.start_us, cue.duration_us, cue.text.clone()))
            .collect();
        let cancel = ResolverCancellation::default();
        let (provider, _) = mock(Mode::Immediate, 2, 3, cancel.clone());
        let result = translate_transcript(
            &original,
            TranslationQuality::Fast,
            &SessionGlossary::new(identity()),
            &provider,
            &cancel,
        )
        .await
        .unwrap();
        assert_eq!(
            result.iter().map(|text| text.as_ref()).collect::<Vec<_>>(),
            ["草稿 0", "草稿 1", "草稿 2"]
        );
        for seen in provider.seen.lock().unwrap().iter() {
            let index = seen.source.sequence as usize;
            assert_eq!(seen.source.identity, identity());
            assert_eq!(seen.source.language, Some(language));
            assert_eq!(seen.source.start_us, Some(before[index].0));
            assert_eq!(seen.source.duration_us, Some(before[index].1));
            assert_eq!(seen.source.text, before[index].2);
            assert_eq!(seen.source.kind, TranscriptKind::Final);
        }
        assert_eq!(original.identity(), identity());
        assert_eq!(original.language(), language);
        assert_eq!(original.cues()[2].text, before[2].2);
    }
}

#[tokio::test]
async fn quality_changes_actual_asr_context_and_full_review_passes() {
    for (quality, window, passes) in [
        (TranslationQuality::Fast, 1, 1),
        (TranslationQuality::Standard, 3, 2),
        (TranslationQuality::High, 8, 2),
    ] {
        let original = transcript(SpeechLanguage::English, 12);
        let cancel = ResolverCancellation::default();
        let (provider, _) = mock(Mode::Immediate, 2, 12, cancel.clone());
        let mut glossary = SessionGlossary::new(identity());
        glossary.set(identity(), "ASR", "語音辨識").unwrap();
        let result = translate_transcript(&original, quality, &glossary, &provider, &cancel)
            .await
            .unwrap();
        assert_eq!(
            result[5].as_ref(),
            if passes == 1 { "草稿 5" } else { "校訂 5" }
        );
        let seen = provider.seen.lock().unwrap();
        assert_eq!(seen.len(), 12 * passes);
        let draft = seen
            .iter()
            .find(|row| row.pass == TranslationPass::Draft && row.source.sequence == 5)
            .unwrap();
        assert_eq!(draft.quality, quality);
        assert_eq!(draft.previous.len(), window.min(5));
        assert_eq!(draft.following.len(), window.min(6));
        assert_eq!(draft.glossary[0].taiwan_translation.as_ref(), "語音辨識");
        assert!(draft.neighbors.is_empty());
        if passes == 2 {
            let first_review = seen
                .iter()
                .position(|row| row.pass == TranslationPass::Review)
                .unwrap();
            assert_eq!(first_review, 12);
            let review = seen
                .iter()
                .find(|row| row.pass == TranslationPass::Review && row.source.sequence == 5)
                .unwrap();
            assert_eq!(review.draft.as_deref(), Some("草稿 5"));
            assert_eq!(review.neighbors.len(), window.min(5) + window.min(6));
            for (index, source, draft) in &review.neighbors {
                assert_eq!(source.as_ref(), format!("ASR source {index}"));
                assert_eq!(draft.as_ref(), format!("草稿 {index}"));
            }
            assert!(
                review
                    .stable
                    .iter()
                    .all(|row| row.translation.starts_with("校訂"))
            );
        }
    }
}

#[tokio::test]
async fn out_of_order_asr_translations_restore_order_and_keep_next_batch_unscheduled() {
    let original = transcript(SpeechLanguage::English, 4);
    let glossary = SessionGlossary::new(identity());
    let cancel = ResolverCancellation::default();
    let (provider, mut entered) = mock(Mode::Gated, 2, 4, cancel.clone());
    let work = translate_transcript(
        &original,
        TranslationQuality::Fast,
        &glossary,
        &provider,
        &cancel,
    );
    tokio::pin!(work);
    let mut first = Vec::new();
    for _ in 0..2 {
        tokio::select! {
            result = &mut work => panic!("gate unexpectedly completed: {}",result.is_ok()),
            index = entered.recv() => first.push(index.unwrap()),
        }
    }
    first.sort_unstable();
    assert_eq!(first, [0, 1]);
    provider.gates[1].add_permits(1);
    tokio::select! {
        result = &mut work => panic!("first gate still closed: {}",result.is_ok()),
        _ = tokio::time::sleep(Duration::from_millis(10)) => {},
    }
    assert_eq!(provider.seen.lock().unwrap().len(), 2);
    provider.gates[0].add_permits(1);
    for _ in 0..2 {
        tokio::select! {
            result = &mut work => panic!("second batch still closed: {}",result.is_ok()),
            index = entered.recv() => assert!((2..=3).contains(&index.unwrap())),
        }
    }
    {
        let seen = provider.seen.lock().unwrap();
        let row = seen.iter().find(|row| row.source.sequence == 2).unwrap();
        assert_eq!(row.stable.len(), 1);
        assert_eq!(row.stable[0].source.as_ref(), "ASR source 1");
        assert_eq!(row.stable[0].translation.as_ref(), "草稿 1");
    }
    provider.gates[2].add_permits(1);
    provider.gates[3].add_permits(1);
    let result = work.await.unwrap();
    assert_eq!(
        result.iter().map(|text| text.as_ref()).collect::<Vec<_>>(),
        ["草稿 0", "草稿 1", "草稿 2", "草稿 3"]
    );
    assert_eq!(provider.peak.load(Ordering::SeqCst), 2);
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn wrong_session_glossary_or_capability_starts_no_asr_translation_requests() {
    let original = transcript(SpeechLanguage::English, 1);
    let cancel = ResolverCancellation::default();
    let (provider, _) = mock(Mode::Immediate, 1, 1, cancel.clone());
    let mut foreign = identity();
    foreign.generation += 1;
    assert_eq!(
        translate_transcript(
            &original,
            TranslationQuality::Fast,
            &SessionGlossary::new(foreign),
            &provider,
            &cancel
        )
        .await
        .unwrap_err(),
        TranslationError::InvalidData
    );
    assert!(provider.seen.lock().unwrap().is_empty());
    let (invalid, _) = mock(Mode::Immediate, 0, 1, cancel.clone());
    assert_eq!(
        translate_transcript(
            &original,
            TranslationQuality::Fast,
            &SessionGlossary::new(identity()),
            &invalid,
            &cancel
        )
        .await
        .unwrap_err(),
        TranslationError::UnsupportedCapability
    );
    assert!(invalid.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn cancellation_and_future_drop_release_all_pending_asr_translation_requests() {
    for drop_future in [false, true] {
        let original = transcript(SpeechLanguage::English, 3);
        let glossary = SessionGlossary::new(identity());
        let cancel = ResolverCancellation::default();
        let (provider, mut entered) = mock(Mode::Pending, 2, 3, cancel.clone());
        let mut work = Box::pin(translate_transcript(
            &original,
            TranslationQuality::Standard,
            &glossary,
            &provider,
            &cancel,
        ));
        for _ in 0..2 {
            tokio::select! {
                result = &mut work => panic!("pending fixture completed: {}",result.is_ok()),
                index = entered.recv() => { index.unwrap(); },
            }
        }
        if drop_future {
            drop(work);
        } else {
            cancel.cancel();
            assert_eq!(work.await.unwrap_err(), TranslationError::Cancelled);
        }
        assert_eq!(provider.active.load(Ordering::SeqCst), 0);
        assert_eq!(provider.seen.lock().unwrap().len(), 2);
        assert!(
            provider
                .tokens
                .lock()
                .unwrap()
                .iter()
                .all(|token| token.reason() == Some(TranslationError::Cancelled))
        );
    }
}

#[tokio::test]
async fn asr_provider_failure_never_starts_review_or_returns_partial_text() {
    let original = transcript(SpeechLanguage::English, 4);
    let cancel = ResolverCancellation::default();
    let (provider, _) = mock(Mode::Fail, 2, 4, cancel.clone());
    assert_eq!(
        translate_transcript(
            &original,
            TranslationQuality::High,
            &SessionGlossary::new(identity()),
            &provider,
            &cancel
        )
        .await
        .unwrap_err(),
        TranslationError::Refused
    );
    let seen = provider.seen.lock().unwrap();
    assert!(
        seen.iter()
            .all(|row| row.source.sequence < 2 && row.pass == TranslationPass::Draft)
    );
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn malformed_and_expanded_asr_translation_results_keep_existing_limits() {
    for mode in [Mode::Blank, Mode::Controls, Mode::Oversized] {
        let original = transcript(SpeechLanguage::English, 1);
        let cancel = ResolverCancellation::default();
        let (provider, _) = mock(mode, 1, 1, cancel.clone());
        assert_eq!(
            translate_transcript(
                &original,
                TranslationQuality::Fast,
                &SessionGlossary::new(identity()),
                &provider,
                &cancel
            )
            .await
            .unwrap_err(),
            TranslationError::InvalidData
        );
    }
    let original = transcript(SpeechLanguage::English, 513);
    let cancel = ResolverCancellation::default();
    let (provider, _) = mock(Mode::Expanded, 2, 513, cancel.clone());
    assert_eq!(
        translate_transcript(
            &original,
            TranslationQuality::Fast,
            &SessionGlossary::new(identity()),
            &provider,
            &cancel
        )
        .await
        .unwrap_err(),
        TranslationError::InvalidData
    );
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn asr_provider_panics_and_late_success_after_cancel_are_typed() {
    for mode in [Mode::Panic, Mode::CapabilityPanic, Mode::CancelSuccess] {
        let original = transcript(SpeechLanguage::English, 1);
        let cancel = ResolverCancellation::default();
        let (provider, _) = mock(mode, 1, 1, cancel.clone());
        let expected = if matches!(mode, Mode::CancelSuccess) {
            TranslationError::Cancelled
        } else {
            TranslationError::InvalidData
        };
        assert_eq!(
            translate_transcript(
                &original,
                TranslationQuality::Fast,
                &SessionGlossary::new(identity()),
                &provider,
                &cancel
            )
            .await
            .unwrap_err(),
            expected
        );
        assert_eq!(provider.active.load(Ordering::SeqCst), 0);
        assert!(
            provider
                .tokens
                .lock()
                .unwrap()
                .iter()
                .all(|token| token.reason() == Some(TranslationError::Cancelled))
        );
    }
}

#[tokio::test]
async fn pre_cancelled_asr_translation_queries_no_provider() {
    let original = transcript(SpeechLanguage::English, 1);
    let cancel = ResolverCancellation::default();
    cancel.cancel();
    let (provider, _) = mock(Mode::CapabilityPanic, 1, 1, cancel.clone());
    assert_eq!(
        translate_transcript(
            &original,
            TranslationQuality::Fast,
            &SessionGlossary::new(identity()),
            &provider,
            &cancel
        )
        .await
        .unwrap_err(),
        TranslationError::Cancelled
    );
    assert!(provider.seen.lock().unwrap().is_empty());
}
