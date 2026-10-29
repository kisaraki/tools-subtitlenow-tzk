//! Real sealed ASR source owners and explicit synthetic translation providers.
//! No keys, network, media files, Ready owner, subtitle publication or playback.
use super::*;
use crate::{
    ai::{
        prepared_speech::{
            PreparedChunkStamp, PreparedSpeechChunk, PreparedSpeechCue, PreparedSpeechRequest,
            PreparedSpeechResponse,
        },
        prepared_transcript::PreparedTranscriptMerger,
        speech::{SpeechLanguage, TranscriptKind},
        translation::{
            GlossaryEntry, QualityPolicy, SessionGlossary, TranslationCancellation,
            TranslationCapabilities, TranslationEmitter, TranslationFuture, TranslationPass,
            TranslationRequest,
        },
    },
    audio::capture::SessionId,
    domain::SourceLanguage,
    media::prepared_audio::PreparedSampleRate,
};
use std::{
    future::Future,
    sync::{
        Condvar, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Notify, mpsc, oneshot};

const TEST_TIMEOUT: Duration = Duration::from_secs(5);
const WORKER_GATE_TIMEOUT: Duration = Duration::from_secs(10);
const SOURCE_ANCHOR_US: u64 = 600_000_000;
const SOURCE_CUE_SPACING_US: u64 = 10_000;
const SOURCE_CUE_DURATION_US: u64 = 5_000;
const SOURCE_CUES_PER_CHUNK: usize = 512;

fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(818),
        generation: 4,
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
    for first in (0..count).step_by(SOURCE_CUES_PER_CHUNK) {
        let sequence = first / SOURCE_CUES_PER_CHUNK + 1;
        let anchor = SOURCE_ANCHOR_US + (sequence - 1) as u64 * 10_000_000;
        let request = PreparedSpeechRequest::new(
            PreparedSpeechChunk::new(
                PreparedChunkStamp {
                    identity: identity(),
                    sequence: sequence as u64,
                    run_id: 1,
                    rate: PreparedSampleRate::Hz16000,
                    audio_start_us: anchor,
                    body_start_us: anchor,
                    body_end_us: anchor + 10_000_000,
                    discontinuity: false,
                },
                vec![0; 160_000],
            )
            .unwrap(),
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
                    sequence: sequence as u64,
                    run_id: 1,
                    language: Some(language),
                    cues: (first..(first + SOURCE_CUES_PER_CHUNK).min(count))
                        .map(|index| PreparedSpeechCue {
                            start_us: (index - first) as u64 * SOURCE_CUE_SPACING_US + 123,
                            duration_us: SOURCE_CUE_DURATION_US,
                            text: format!("synthetic source {index}").into(),
                        })
                        .collect(),
                },
            )
            .unwrap();
    }
    merger.finish().unwrap()
}
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}
/// The sync test holds the global fixture lock outside the async future.
fn with_shared_cpu<T>(work: impl Future<Output = T>) -> T {
    let _fixtures = crate::media::work::TEST_OWNERS.lock().unwrap();
    runtime().block_on(work)
}

#[derive(Clone, Copy)]
enum Mode {
    Immediate,
    Gated,
    Pending,
    CancelRoot,
    Fail,
    Empty,
    Controls,
    Oversized,
    Maximum,
}
struct Seen {
    identity: StreamIdentity,
    language: SpeechLanguage,
    sequence: u64,
    start_us: u64,
    duration_us: u64,
    quality: TranslationQuality,
    pass: TranslationPass,
    draft: Option<Box<str>>,
    previous: Vec<Box<str>>,
    following: Vec<Box<str>>,
    neighboring: usize,
    glossary: Vec<GlossaryEntry>,
}
struct MockTranslator {
    mode: Mode,
    root: ResolverCancellation,
    seen: Mutex<Vec<Seen>>,
    calls: AtomicUsize,
    active: Arc<AtomicUsize>,
    entered: mpsc::Sender<TranslationCancellation>,
    release: Notify,
}
struct Active(Arc<AtomicUsize>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
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
        request: &'a TranslationRequest,
        _: &'a mut TranslationEmitter,
        cancel: TranslationCancellation,
    ) -> TranslationFuture<'a> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.active.fetch_add(1, Ordering::SeqCst);
            let _active = Active(self.active.clone());
            let context = request.prepared_context.as_ref().unwrap();
            assert_eq!(request.target_language(), "zh-TW");
            assert_eq!(request.source.kind, TranscriptKind::Final);
            assert!(request.committed_prefix.is_empty());
            self.seen.lock().unwrap().push(Seen {
                identity: request.source.identity,
                language: request.source.language.unwrap(),
                sequence: request.source.sequence,
                start_us: request
                    .source
                    .start_us
                    .ok_or(TranslationError::InvalidData)?,
                duration_us: request
                    .source
                    .duration_us
                    .ok_or(TranslationError::InvalidData)?,
                quality: request.quality,
                pass: request.pass,
                draft: request.draft.clone(),
                previous: context.previous_source.clone(),
                following: context.following_source.clone(),
                neighboring: context.neighboring_drafts.len(),
                glossary: request.glossary.clone(),
            });
            let _ = self.entered.try_send(cancel);
            match self.mode {
                Mode::Gated => self.release.notified().await,
                Mode::Pending => std::future::pending::<()>().await,
                Mode::CancelRoot => self.root.cancel(),
                Mode::Fail => return Err(TranslationError::RateLimited),
                Mode::Empty => return Ok(" \n".into()),
                Mode::Controls => return Ok("invalid\0output".into()),
                Mode::Oversized => return Ok("x".repeat(MAX_CUE_BYTES + 1).into()),
                Mode::Maximum => return Ok("x".repeat(MAX_CUE_BYTES).into()),
                Mode::Immediate => {}
            }
            Ok(format!(
                "{}软件打印机 {}",
                if request.pass == TranslationPass::Review {
                    "校訂"
                } else {
                    "草稿"
                },
                request.source.sequence
            )
            .into())
        })
    }
}
fn mock(
    mode: Mode,
    root: ResolverCancellation,
) -> (MockTranslator, mpsc::Receiver<TranslationCancellation>) {
    let (entered, receiver) = mpsc::channel(32);
    (
        MockTranslator {
            mode,
            root,
            seen: Mutex::new(Vec::new()),
            calls: AtomicUsize::new(0),
            active: Arc::new(AtomicUsize::new(0)),
            entered,
            release: Notify::new(),
        },
        receiver,
    )
}

#[test]
fn five_language_sources_move_original_ownership_and_keep_native_times() {
    with_shared_cpu(async {
        for language in SpeechLanguage::ALL {
            let cancel = ResolverCancellation::default();
            let (provider, _) = mock(Mode::Immediate, cancel.clone());
            let mut glossary = SessionGlossary::new(identity());
            glossary.set(identity(), "synthetic", "合成").unwrap();
            let original = transcript(language, 3);
            let allocation = original.cues().as_ptr();
            let text_allocations: Vec<_> = original
                .cues()
                .iter()
                .map(|cue| cue.text.as_ptr())
                .collect();
            let translated = translate_recognized_transcript(
                original,
                TranslationQuality::Fast,
                &glossary,
                &provider,
                cancel.clone(),
            )
            .await
            .unwrap();
            assert_eq!(translated.identity(), identity());
            assert_eq!(translated.original().language(), language);
            assert_eq!(translated.original().cues().as_ptr(), allocation);
            assert_eq!(translated.quality(), TranslationQuality::Fast);
            assert_eq!(translated.target_language(), "zh-TW");
            for (index, cue) in translated.original().cues().iter().enumerate() {
                assert_eq!(cue.text.as_ptr(), text_allocations[index]);
                assert_eq!(
                    cue.start_us,
                    SOURCE_ANCHOR_US + index as u64 * SOURCE_CUE_SPACING_US + 123
                );
                assert_eq!(cue.duration_us, SOURCE_CUE_DURATION_US);
                assert_eq!(cue.text.as_ref(), format!("synthetic source {index}"));
                assert_eq!(
                    translated.translated()[index].as_ref(),
                    format!("草稿軟體印表機 {index}")
                );
            }
            let seen = provider.seen.lock().unwrap();
            assert_eq!(seen.len(), 3);
            assert!(seen.iter().all(|request| request.identity == identity()
                && request.language == language
                && request.glossary.len() == 1
                && request.glossary[0].source.as_ref() == "synthetic"
                && request.glossary[0].taiwan_translation.as_ref() == "合成"));
            assert!(!cancel.is_cancelled());
        }
    });
}

#[test]
fn fast_standard_and_high_apply_real_context_and_review_strategy() {
    with_shared_cpu(async {
        for quality in [
            TranslationQuality::Fast,
            TranslationQuality::Standard,
            TranslationQuality::High,
        ] {
            let cancel = ResolverCancellation::default();
            let glossary = SessionGlossary::new(identity());
            let (provider, _) = mock(Mode::Immediate, cancel.clone());
            let translated = translate_recognized_transcript(
                transcript(SpeechLanguage::English, 9),
                quality,
                &glossary,
                &provider,
                cancel,
            )
            .await
            .unwrap();
            assert_eq!(translated.quality(), quality);
            let review = quality != TranslationQuality::Fast;
            let seen = provider.seen.lock().unwrap();
            assert_eq!(seen.len(), if review { 18 } else { 9 });
            let radius = QualityPolicy::for_quality(quality).context_sentences;
            for request in seen.iter() {
                let index = request.sequence as usize;
                assert_eq!(request.quality, quality);
                assert_eq!(
                    request.start_us,
                    SOURCE_ANCHOR_US + index as u64 * SOURCE_CUE_SPACING_US + 123
                );
                assert_eq!(request.duration_us, SOURCE_CUE_DURATION_US);
                assert_eq!(request.previous.len(), index.min(radius));
                assert_eq!(request.following.len(), (8 - index).min(radius));
                if request.pass == TranslationPass::Review {
                    assert!(review && request.draft.is_some());
                    assert_eq!(
                        request.neighboring,
                        index.min(radius) + (8 - index).min(radius)
                    );
                } else {
                    assert!(request.draft.is_none());
                    assert_eq!(request.neighboring, 0);
                }
            }
            assert!(
                translated
                    .translated()
                    .iter()
                    .enumerate()
                    .all(|(index, text)| text.as_ref()
                        == format!("{}軟體印表機 {index}", if review { "校訂" } else { "草稿" }))
            );
        }
    });
}

#[tokio::test]
async fn stale_glossary_is_rejected_before_provider_or_localization() {
    for stale in [
        StreamIdentity {
            session: SessionId(819),
            generation: 4,
        },
        StreamIdentity {
            session: identity().session,
            generation: identity().generation + 1,
        },
    ] {
        let cancel = ResolverCancellation::default();
        let (provider, _) = mock(Mode::Immediate, cancel.clone());
        let glossary = SessionGlossary::new(stale);
        assert_eq!(
            translate_recognized_transcript(
                transcript(SpeechLanguage::English, 1),
                TranslationQuality::Fast,
                &glossary,
                &provider,
                cancel.clone()
            )
            .await
            .err(),
            Some(TranslationError::InvalidData)
        );
        assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
        assert!(cancel.is_cancelled());
    }
}

#[test]
fn unpolled_owner_drop_cancels_without_provider_work() {
    let cancel = ResolverCancellation::default();
    let glossary = SessionGlossary::new(identity());
    let (provider, _) = mock(Mode::Immediate, cancel.clone());
    drop(translate_recognized_transcript(
        transcript(SpeechLanguage::English, 1),
        TranslationQuality::Fast,
        &glossary,
        &provider,
        cancel.clone(),
    ));
    assert!(cancel.is_cancelled());
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn pending_future_drop_cancels_request_and_releases_provider_owner() {
    let cancel = ResolverCancellation::default();
    let glossary = SessionGlossary::new(identity());
    let (provider, mut entered) = mock(Mode::Pending, cancel.clone());
    let mut work = Box::pin(translate_recognized_transcript(
        transcript(SpeechLanguage::Japanese, 1),
        TranslationQuality::Fast,
        &glossary,
        &provider,
        cancel.clone(),
    ));
    let token = tokio::select! {
        result = &mut work => panic!("pending synthetic provider unexpectedly finished: {:?}", result.err()),
        token = tokio::time::timeout(TEST_TIMEOUT, entered.recv()) => token.unwrap().unwrap(),
    };
    assert_eq!(provider.active.load(Ordering::SeqCst), 1);
    drop(work);
    assert!(cancel.is_cancelled());
    assert_eq!(token.reason(), Some(TranslationError::Cancelled));
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn success_returned_after_root_cancel_cannot_become_a_document() {
    let cancel = ResolverCancellation::default();
    let glossary = SessionGlossary::new(identity());
    let (provider, _) = mock(Mode::CancelRoot, cancel.clone());
    assert_eq!(
        translate_recognized_transcript(
            transcript(SpeechLanguage::French, 1),
            TranslationQuality::Fast,
            &glossary,
            &provider,
            cancel.clone()
        )
        .await
        .err(),
        Some(TranslationError::Cancelled)
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
    assert!(cancel.is_cancelled());
}

#[tokio::test]
async fn typed_provider_failure_returns_no_partial_translated_owner() {
    let cancel = ResolverCancellation::default();
    let glossary = SessionGlossary::new(identity());
    let (provider, _) = mock(Mode::Fail, cancel.clone());
    assert_eq!(
        translate_recognized_transcript(
            transcript(SpeechLanguage::German, 3),
            TranslationQuality::Standard,
            &glossary,
            &provider,
            cancel.clone()
        )
        .await
        .err(),
        Some(TranslationError::RateLimited)
    );
    assert!(cancel.is_cancelled());
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn malformed_complete_output_never_reaches_localized_document() {
    for mode in [Mode::Empty, Mode::Controls, Mode::Oversized] {
        let cancel = ResolverCancellation::default();
        let glossary = SessionGlossary::new(identity());
        let (provider, _) = mock(mode, cancel.clone());
        assert_eq!(
            translate_recognized_transcript(
                transcript(SpeechLanguage::Korean, 1),
                TranslationQuality::Fast,
                &glossary,
                &provider,
                cancel.clone()
            )
            .await
            .err(),
            Some(TranslationError::InvalidData)
        );
        assert!(cancel.is_cancelled());
    }
}

#[test]
fn provider_wait_holds_no_shared_cpu_capacity() {
    with_shared_cpu(async {
        let cpu = crate::media::work::workers();
        let available = cpu.available_permits() as u32;
        assert!(available > 0);
        let cancel = ResolverCancellation::default();
        let glossary = SessionGlossary::new(identity());
        let (provider, mut entered) = mock(Mode::Gated, cancel.clone());
        let mut work = Box::pin(translate_recognized_transcript(
            transcript(SpeechLanguage::English, 1),
            TranslationQuality::Fast,
            &glossary,
            &provider,
            cancel.clone(),
        ));
        tokio::select! {
            result = &mut work => panic!("gated provider completed early: {:?}", result.err()),
            token = tokio::time::timeout(TEST_TIMEOUT, entered.recv()) => { token.unwrap().unwrap(); },
        }
        let all_capacity = cpu.clone().try_acquire_many_owned(available).unwrap();
        assert_eq!(cpu.available_permits(), 0);
        drop(all_capacity);
        provider.release.notify_one();
        let result = tokio::time::timeout(TEST_TIMEOUT, work)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.translated()[0].as_ref(), "草稿軟體印表機 0");
        assert_eq!(cpu.available_permits(), available as usize);
        assert!(!cancel.is_cancelled());
    });
}

#[test]
fn final_localization_overload_is_explicit_and_does_not_queue_or_return_draft() {
    with_shared_cpu(async {
        let cpu = crate::media::work::workers();
        let cancel = ResolverCancellation::default();
        let glossary = SessionGlossary::new(identity());
        let (provider, mut entered) = mock(Mode::Gated, cancel.clone());
        let mut work = Box::pin(translate_recognized_transcript(
            transcript(SpeechLanguage::English, 1),
            TranslationQuality::Fast,
            &glossary,
            &provider,
            cancel.clone(),
        ));
        tokio::select! {
            result = &mut work => panic!("gated provider completed early: {:?}", result.err()),
            token = tokio::time::timeout(TEST_TIMEOUT, entered.recv()) => { token.unwrap().unwrap(); },
        }
        let all_capacity = cpu
            .clone()
            .try_acquire_many_owned(cpu.available_permits() as u32)
            .unwrap();
        provider.release.notify_one();
        assert_eq!(
            tokio::time::timeout(TEST_TIMEOUT, work)
                .await
                .unwrap()
                .err(),
            Some(TranslationError::Overloaded)
        );
        assert!(cancel.is_cancelled());
        assert_eq!(cpu.available_permits(), 0);
        drop(all_capacity);
    });
}

#[test]
fn finalizer_requires_exact_cue_count_and_rejects_invalid_output() {
    let cancel = ResolverCancellation::default();
    for output in [
        Vec::new(),
        vec!["one".into(), "two".into()],
        vec!["\0".into()],
        vec!["x".repeat(MAX_CUE_BYTES + 1).into()],
    ] {
        assert_eq!(
            finish(
                transcript(SpeechLanguage::English, 1),
                output,
                TranslationQuality::Fast,
                &cancel
            )
            .err(),
            Some(TranslationError::InvalidData)
        );
    }
}

#[test]
fn finalizer_keeps_exact_per_cue_byte_limit_and_original_times() {
    let cancel = ResolverCancellation::default();
    let original = transcript(SpeechLanguage::English, 1);
    let source_pointer = original.cues()[0].text.as_ptr();
    let text = "x".repeat(MAX_CUE_BYTES);
    let document = finish(
        original,
        vec![text.into()],
        TranslationQuality::High,
        &cancel,
    )
    .unwrap();
    assert_eq!(document.translated()[0].len(), MAX_CUE_BYTES);
    assert_eq!(document.original().cues()[0].text.as_ptr(), source_pointer);
    assert_eq!(
        document.original().cues()[0].start_us,
        SOURCE_ANCHOR_US + 123
    );
    assert_eq!(
        document.original().cues()[0].duration_us,
        SOURCE_CUE_DURATION_US
    );
    assert_eq!(document.quality(), TranslationQuality::High);
}

#[tokio::test]
async fn full_output_budget_cannot_be_exceeded_by_many_valid_individual_cues() {
    let cancel = ResolverCancellation::default();
    let glossary = SessionGlossary::new(identity());
    let (provider, _) = mock(Mode::Maximum, cancel.clone());
    assert_eq!(
        translate_recognized_transcript(
            transcript(SpeechLanguage::English, MAX_TEXT_BYTES / MAX_CUE_BYTES + 1),
            TranslationQuality::Fast,
            &glossary,
            &provider,
            cancel.clone()
        )
        .await
        .err(),
        Some(TranslationError::InvalidData)
    );
    assert!(cancel.is_cancelled());
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
}

#[test]
fn cancellation_before_finalizer_returns_no_owner() {
    let cancel = ResolverCancellation::default();
    cancel.cancel();
    assert_eq!(
        finish(
            transcript(SpeechLanguage::English, 1),
            vec!["軟體".into()],
            TranslationQuality::Fast,
            &cancel
        )
        .err(),
        Some(TranslationError::Cancelled)
    );
}

#[test]
fn taiwan_localization_expansion_respects_per_cue_and_document_budgets() {
    let cancel = ResolverCancellation::default();
    // Six UTF-8 bytes of source become nine bytes of Taiwan terminology.
    assert_eq!(taiwan_phrase_conversion("内存").unwrap(), "記憶體");
    let per_cue = "内存".repeat(1365);
    assert_eq!(per_cue.len(), 8190);
    assert_eq!("記憶體".repeat(1365).len(), 12_285);
    assert_eq!(
        finish(
            transcript(SpeechLanguage::English, 1),
            vec![per_cue.into()],
            TranslationQuality::Fast,
            &cancel
        )
        .err(),
        Some(TranslationError::InvalidData)
    );

    let count = 518;
    let text = "内存".repeat(900);
    let localized = "記憶體".repeat(900);
    assert_eq!(text.len(), 5400);
    assert_eq!(localized.len(), 8100);
    assert!(text.len() * count < MAX_TEXT_BYTES);
    assert!(localized.len() <= MAX_CUE_BYTES);
    assert!(localized.len() * count > MAX_TEXT_BYTES);
    let translated = (0..count).map(|_| text.clone().into_boxed_str()).collect();
    assert_eq!(
        finish(
            transcript(SpeechLanguage::English, count),
            translated,
            TranslationQuality::Standard,
            &cancel
        )
        .err(),
        Some(TranslationError::InvalidData)
    );
}

#[derive(Default)]
struct FinishGate {
    released: Mutex<bool>,
    wake: Condvar,
}
impl FinishGate {
    fn wait(&self) -> Result<(), TranslationError> {
        let released = self.released.lock().unwrap();
        let (released, _) = self
            .wake
            .wait_timeout_while(released, WORKER_GATE_TIMEOUT, |released| !*released)
            .unwrap();
        if *released {
            Ok(())
        } else {
            Err(TranslationError::Timeout)
        }
    }
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.wake.notify_all();
    }
}
struct ReleaseFinishGate(Arc<FinishGate>);
impl Drop for ReleaseFinishGate {
    fn drop(&mut self) {
        self.0.release();
    }
}

#[tokio::test]
async fn started_localization_future_drop_retains_capacity_until_real_worker_exit() {
    let cpu = Arc::new(Semaphore::new(1));
    let cancel = ResolverCancellation::default();
    let gate = Arc::new(FinishGate::default());
    let _release_on_failure = ReleaseFinishGate(gate.clone());
    let worker_gate = gate.clone();
    let completed = Arc::new(AtomicBool::new(false));
    let worker_completed = completed.clone();
    let cancelled_finish = Arc::new(AtomicBool::new(false));
    let worker_cancelled_finish = cancelled_finish.clone();
    let (entered, entered_receiver) = oneshot::channel();
    let original = transcript(SpeechLanguage::English, 1);
    let raw = vec!["软件".into()];
    let mut work = Box::pin(finish_on_worker(&cancel, cpu.clone(), move |scope| {
        let _ = entered.send(());
        worker_gate.wait()?;
        let result = finish(original, raw, TranslationQuality::Standard, scope);
        worker_cancelled_finish.store(
            matches!(&result, Err(TranslationError::Cancelled)),
            Ordering::SeqCst,
        );
        worker_completed.store(true, Ordering::SeqCst);
        result
    }));
    tokio::select! {
        result = &mut work => panic!("blocked localization worker returned early: {:?}", result.err()),
        entered = tokio::time::timeout(TEST_TIMEOUT, entered_receiver) => { entered.unwrap().unwrap(); },
    }
    assert_eq!(cpu.available_permits(), 0);
    assert!(!completed.load(Ordering::SeqCst));
    drop(work);
    assert!(cancel.is_cancelled());
    assert_eq!(cpu.available_permits(), 0);
    assert!(!completed.load(Ordering::SeqCst));
    gate.release();
    // Semaphore wake is actual capacity release; no sleep or abort assumption.
    let returned_capacity = tokio::time::timeout(TEST_TIMEOUT, cpu.clone().acquire_owned())
        .await
        .unwrap()
        .unwrap();
    assert!(completed.load(Ordering::SeqCst));
    assert!(cancelled_finish.load(Ordering::SeqCst));
    drop(returned_capacity);
    assert_eq!(cpu.available_permits(), 1);
}
