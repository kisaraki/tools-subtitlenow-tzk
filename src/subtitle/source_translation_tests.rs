//! Original synthetic fixtures and explicit providers only; no cloud or keys.
use super::*;
use crate::{
    ai::translation::{TranslationCapabilities, TranslationFuture},
    audio::capture::SessionId,
};
use std::{
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

#[derive(Clone, Copy)]
enum Mode {
    Ok,
    Pending,
    Error,
    Empty,
    Oversized,
    Controls,
    SyncPanic,
    FuturePanic,
    Unsupported,
    CapabilityPanic,
    SecondError,
    Expansion,
    Blocking,
}
struct Seen {
    pass: TranslationPass,
    language: Option<SpeechLanguage>,
    context: Vec<StableContext>,
    draft: Option<Box<str>>,
    terms: usize,
}
struct Mock {
    mode: Mode,
    calls: Mutex<Vec<Seen>>,
    active: Arc<AtomicUsize>,
    entered: tokio::sync::mpsc::Sender<TranslationCancellation>,
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
            cancellation: !matches!(self.mode, Mode::Unsupported),
            supported_languages: [true; 5],
            max_concurrent: 1,
        }
    }
    fn translate<'a>(
        &'a self,
        request: &'a TranslationRequest,
        output: &'a mut TranslationEmitter,
        cancel: TranslationCancellation,
    ) -> TranslationFuture<'a> {
        if matches!(self.mode, Mode::SyncPanic) {
            panic!("synthetic synchronous translator panic");
        }
        Box::pin(async move {
            self.active.fetch_add(1, Ordering::SeqCst);
            let _active = Active(self.active.clone());
            self.calls.lock().unwrap().push(Seen {
                pass: request.pass,
                language: request.source.language,
                context: request.previous_stable.clone(),
                draft: request.draft.clone(),
                terms: request.glossary.len(),
            });
            let _ = self.entered.try_send(cancel);
            assert_eq!(request.target_language(), "zh-TW");
            assert_eq!(request.source.kind, TranscriptKind::Final);
            if matches!(self.mode, Mode::SecondError) && self.calls.lock().unwrap().len() == 2 {
                return Err(TranslationError::Refused);
            }
            match self.mode {
                Mode::Pending => std::future::pending().await,
                Mode::Error => Err(TranslationError::RateLimited),
                Mode::Empty => Ok(" \n".into()),
                Mode::Oversized => Ok("x".repeat(MAX_TRANSCRIPT_BYTES + 1).into()),
                Mode::Controls => Ok("invalid\0text".into()),
                Mode::FuturePanic => panic!("synthetic asynchronous translator panic"),
                Mode::Expansion => Ok("内存".repeat(1000).into()),
                Mode::Blocking => {
                    // Deliberately non-cooperative test adapter. A ready result after
                    // the budget must not bypass the deadline or start another pass.
                    std::thread::sleep(MAX_TRANSLATION_AGE + Duration::from_millis(20));
                    Ok("synthetic late result".into())
                }
                _ => {
                    // More than the live event queue capacity: final-only emission
                    // must not create an unconsumed queue or another timeline.
                    for _ in 0..100 {
                        output.emit("合成暫定譯文")?;
                    }
                    Ok(if request.pass == TranslationPass::Review {
                        "校訂软件".into()
                    } else {
                        "软件".into()
                    })
                }
            }
        })
    }
}
fn mock(mode: Mode) -> (Mock, tokio::sync::mpsc::Receiver<TranslationCancellation>) {
    let (entered, receiver) = tokio::sync::mpsc::channel(16);
    (
        Mock {
            mode,
            calls: Mutex::new(Vec::new()),
            active: Arc::new(AtomicUsize::new(0)),
            entered,
        },
        receiver,
    )
}
fn id() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(907),
        generation: 1,
    }
}
fn fixture(sequence: u64, language: SpeechLanguage) -> (TextQuery, ExistingText) {
    let query = TextQuery {
        stamp: WindowStamp {
            identity: id(),
            sequence,
            ended_at: Instant::now(),
            start_us: None,
            duration_us: 12_000_000,
        },
        source: SubtitleSource::ExistingSubtitle,
        content_id: "synthetic document".into(),
    };
    let text = ExistingText {
        identity: id(),
        source: query.source,
        content_id: query.content_id.clone(),
        language: TextLanguage::Source(language),
        cues: (0..12)
            .map(|index| TextCue {
                start_us: Some(index * 1_000_000),
                duration_us: Some(900_000),
                text: format!("original synthetic cue {index}").into(),
            })
            .collect(),
    };
    (query, text)
}
async fn translate(
    owner: &mut SourceTextTranslator<'_>,
    sequence: u64,
    text: &ExistingText,
    range: Range<usize>,
) -> Result<TranslatedTextWindow, SourceTranslationError> {
    let (mut query, _) = fixture(sequence, SpeechLanguage::English);
    query.content_id = text.content_id.clone();
    query.source = text.source;
    tokio::time::timeout(
        Duration::from_secs(5),
        owner.translate_window(query, text, range, ResolverCancellation::default()),
    )
    .await
    .unwrap()
}
#[tokio::test]
async fn five_languages_use_selected_provider_and_preserve_original_timing() {
    for language in SpeechLanguage::ALL {
        let (provider, _) = mock(Mode::Ok);
        let mut owner =
            SourceTextTranslator::new(id(), TranslationQuality::Standard, &provider).unwrap();
        let (_, text) = fixture(1, language);
        let result = translate(&mut owner, 1, &text, 2..4).await.unwrap();
        assert_eq!(result.range(), 2..4);
        assert_eq!(result.quality(), TranslationQuality::Standard);
        assert_eq!(
            result.localized().translated(),
            &[Box::<str>::from("軟體"), "軟體".into()]
        );
        assert_eq!(
            result.localized().original().cues[0].text,
            text.cues[2].text
        );
        assert_eq!(
            result.localized().original().cues[0].start_us,
            Some(2_000_000)
        );
        assert_eq!(
            result.localized().original().cues[1].duration_us,
            Some(900_000)
        );
        assert!(result.localized().stamp().start_us.is_none());
        assert!(
            matches!(result.localized().original().language, TextLanguage::Source(found) if found == language)
        );
        assert_eq!(provider.calls.lock().unwrap()[0].language, Some(language));
    }
}
#[tokio::test]
async fn quality_changes_context_size_and_high_runs_review_with_draft() {
    for (quality, context_limit, passes) in [
        (TranslationQuality::Fast, 1, 1),
        (TranslationQuality::Standard, 3, 1),
        (TranslationQuality::High, 8, 2),
    ] {
        let (provider, _) = mock(Mode::Ok);
        let mut owner = SourceTextTranslator::new(id(), quality, &provider).unwrap();
        owner
            .glossary_mut()
            .set(id(), "synthetic term", "合成詞彙")
            .unwrap();
        let (_, text) = fixture(1, SpeechLanguage::English);
        for (sequence, range) in [(1, 0..4), (2, 4..8), (3, 8..9)] {
            let result = translate(&mut owner, sequence, &text, range).await.unwrap();
            assert_eq!(
                result.localized().translated()[0].as_ref(),
                if quality == TranslationQuality::High {
                    "校訂軟體"
                } else {
                    "軟體"
                }
            );
        }
        let seen = provider.calls.lock().unwrap();
        assert_eq!(seen.len(), 9 * passes);
        assert_eq!(seen.last().unwrap().context.len(), context_limit);
        assert!(seen.iter().all(|call| call.terms == 1));
        assert!(
            seen.last()
                .unwrap()
                .context
                .iter()
                .all(|entry| entry.translation.contains("軟體"))
        );
        if quality == TranslationQuality::High {
            for pair in seen.as_chunks::<2>().0 {
                assert_eq!(pair[0].pass, TranslationPass::Draft);
                assert_eq!(pair[1].pass, TranslationPass::Review);
                assert_eq!(pair[1].draft.as_deref(), Some("软件"));
            }
        }
    }
}
#[tokio::test]
async fn seek_gap_document_revision_and_content_changes_do_not_reuse_wrong_context() {
    let (provider, _) = mock(Mode::Ok);
    let mut owner = SourceTextTranslator::new(id(), TranslationQuality::High, &provider).unwrap();
    let (_, mut text) = fixture(1, SpeechLanguage::English);
    translate(&mut owner, 1, &text, 0..2).await.unwrap();
    translate(&mut owner, 2, &text, 4..5).await.unwrap(); // non-adjacent gap
    assert!(provider.calls.lock().unwrap()[4].context.is_empty());
    text.cues[4].text = "revised synthetic cue".into();
    translate(&mut owner, 3, &text, 5..6).await.unwrap(); // changed previous original
    assert!(provider.calls.lock().unwrap()[6].context.is_empty());
    owner
        .glossary_mut()
        .set(id(), "old work term", "舊作品詞彙")
        .unwrap();
    text.content_id = "different synthetic work".into();
    translate(&mut owner, 4, &text, 6..7).await.unwrap();
    let seen = provider.calls.lock().unwrap();
    assert!(seen[8].context.is_empty());
    assert_eq!(seen[8].terms, 0);
}
#[tokio::test]
async fn unsupported_chinese_foreign_session_and_bad_ranges_do_not_send_requests() {
    let (provider, _) = mock(Mode::Ok);
    let mut owner =
        SourceTextTranslator::new(id(), TranslationQuality::Standard, &provider).unwrap();
    for range in [0..0, 0..5, 11..13, Range { start: 5, end: 4 }] {
        let (query, text) = fixture(1, SpeechLanguage::English);
        assert!(matches!(
            owner
                .translate_window(query, &text, range, ResolverCancellation::default())
                .await,
            Err(SourceTranslationError::WindowSize)
        ));
    }
    let (mut query, mut text) = fixture(1, SpeechLanguage::English);
    query.stamp.identity.generation += 1;
    assert!(matches!(
        owner
            .translate_window(query, &text, 0..1, ResolverCancellation::default())
            .await,
        Err(SourceTranslationError::Invalid(ResolverError::InvalidData))
    ));
    text.language = TextLanguage::TaiwanTraditional;
    assert!(matches!(
        translate(&mut owner, 1, &text, 0..1).await,
        Err(SourceTranslationError::Translation(
            TranslationError::UnsupportedCapability
        ))
    ));
    assert!(provider.calls.lock().unwrap().is_empty());
}
#[tokio::test]
async fn stale_future_and_reused_sequence_are_rejected_before_translation() {
    let (provider, _) = mock(Mode::Ok);
    let mut owner =
        SourceTextTranslator::new(id(), TranslationQuality::Standard, &provider).unwrap();
    let (_, text) = fixture(2, SpeechLanguage::English);
    translate(&mut owner, 2, &text, 0..1).await.unwrap();
    for sequence in [1, 2, 3, 4] {
        let (mut query, _) = fixture(sequence, SpeechLanguage::English);
        if sequence == 3 {
            query.stamp.ended_at -= Duration::from_secs(6);
        }
        if sequence == 4 {
            query.stamp.ended_at += Duration::from_secs(1);
        }
        assert!(matches!(
            owner
                .translate_window(query, &text, 1..2, ResolverCancellation::default())
                .await,
            Err(SourceTranslationError::Invalid(ResolverError::Stale))
        ));
    }
    assert_eq!(provider.calls.lock().unwrap().len(), 1);
}
#[tokio::test]
async fn errors_malformed_results_and_panics_never_commit_partial_context() {
    for mode in [
        Mode::Error,
        Mode::Empty,
        Mode::Oversized,
        Mode::Controls,
        Mode::SyncPanic,
        Mode::FuturePanic,
    ] {
        let (provider, _) = mock(mode);
        let mut owner =
            SourceTextTranslator::new(id(), TranslationQuality::High, &provider).unwrap();
        let (_, text) = fixture(1, SpeechLanguage::English);
        assert!(translate(&mut owner, 1, &text, 0..2).await.is_err());
        assert!(owner.context.is_empty());
        assert_eq!(provider.active.load(Ordering::SeqCst), 0);
        assert!(provider.calls.lock().unwrap().len() <= 1); // no retries or Review after failure
    }
}
#[tokio::test]
async fn request_and_session_cancellation_drop_provider_work() {
    for session in [false, true] {
        let (provider, mut entered) = mock(Mode::Pending);
        let mut owner =
            SourceTextTranslator::new(id(), TranslationQuality::Standard, &provider).unwrap();
        let external = ResolverCancellation::default();
        let signal = if session {
            owner.cancellation()
        } else {
            external.clone()
        };
        let (query, text) = fixture(1, SpeechLanguage::English);
        let mut job = Box::pin(owner.translate_window(query, &text, 0..2, external));
        let request = tokio::select! {
            _ = &mut job => panic!("expected pending request"),
            token = tokio::time::timeout(Duration::from_secs(2), entered.recv()) => token.unwrap().unwrap(),
        };
        signal.cancel();
        assert!(matches!(
            job.await,
            Err(SourceTranslationError::Translation(
                TranslationError::Cancelled
            ))
        ));
        assert_eq!(request.reason(), Some(TranslationError::Cancelled));
        assert_eq!(provider.active.load(Ordering::SeqCst), 0);
        assert_eq!(provider.calls.lock().unwrap().len(), 1);
        assert!(owner.context.is_empty());
    }
}
#[tokio::test]
async fn dropped_window_cancels_request_and_releases_future_without_detaching() {
    let (provider, mut entered) = mock(Mode::Pending);
    let mut owner =
        SourceTextTranslator::new(id(), TranslationQuality::Standard, &provider).unwrap();
    let (query, text) = fixture(1, SpeechLanguage::English);
    let mut job =
        Box::pin(owner.translate_window(query, &text, 0..2, ResolverCancellation::default()));
    let request = tokio::select! {
        _ = &mut job => panic!("expected pending request"),
        token = tokio::time::timeout(Duration::from_secs(2), entered.recv()) => token.unwrap().unwrap(),
    };
    drop(job);
    assert_eq!(request.reason(), Some(TranslationError::Cancelled));
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
    owner.stop();
    assert!(owner.context.is_empty());
    assert!(matches!(
        translate(&mut owner, 2, &text, 0..1).await,
        Err(SourceTranslationError::Translation(
            TranslationError::Cancelled
        ))
    ));
}
#[tokio::test]
async fn total_window_deadline_cancels_a_slow_provider_and_does_not_schedule_more_cues() {
    let (provider, mut entered) = mock(Mode::Pending);
    let mut owner = SourceTextTranslator::new(id(), TranslationQuality::High, &provider).unwrap();
    let (mut query, text) = fixture(1, SpeechLanguage::English);
    // Original freshness, not a reset deadline for every cue or pass.
    query.stamp.ended_at -= MAX_WINDOW_AGE - Duration::from_millis(200);
    assert!(matches!(
        owner
            .translate_window(query, &text, 0..4, ResolverCancellation::default())
            .await,
        Err(SourceTranslationError::Translation(
            TranslationError::Timeout
        ))
    ));
    assert_eq!(
        entered.recv().await.unwrap().reason(),
        Some(TranslationError::Cancelled)
    );
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
    assert_eq!(provider.calls.lock().unwrap().len(), 1);
}
#[tokio::test]
async fn unknown_lyrics_times_stay_unknown_and_partial_updates_are_not_candidates() {
    let (provider, _) = mock(Mode::Ok);
    let mut owner = SourceTextTranslator::new(id(), TranslationQuality::Fast, &provider).unwrap();
    let (_, mut text) = fixture(1, SpeechLanguage::Japanese);
    text.source = SubtitleSource::ExistingLyrics;
    text.cues[0].start_us = None;
    text.cues[0].duration_us = None;
    let result = translate(&mut owner, 1, &text, 0..1).await.unwrap();
    assert!(result.localized().original().cues[0].start_us.is_none());
    assert_eq!(result.localized().translated()[0].as_ref(), "軟體");
}

#[test]
fn unsupported_or_panicking_capabilities_cannot_start_an_owner() {
    let (unsupported, _) = mock(Mode::Unsupported);
    assert!(matches!(
        SourceTextTranslator::new(id(), TranslationQuality::Standard, &unsupported),
        Err(SourceTranslationError::Translation(
            TranslationError::UnsupportedCapability
        ))
    ));
    let (panic, _) = mock(Mode::CapabilityPanic);
    assert!(matches!(
        SourceTextTranslator::new(id(), TranslationQuality::Standard, &panic),
        Err(SourceTranslationError::WorkerPanicked)
    ));
}
#[tokio::test]
async fn later_cue_or_review_failure_rolls_back_the_entire_window_context() {
    for quality in [TranslationQuality::Standard, TranslationQuality::High] {
        let (provider, _) = mock(Mode::SecondError);
        let mut owner = SourceTextTranslator::new(id(), quality, &provider).unwrap();
        let (_, text) = fixture(1, SpeechLanguage::French);
        assert!(matches!(
            translate(&mut owner, 1, &text, 0..2).await,
            Err(SourceTranslationError::Translation(
                TranslationError::Refused
            ))
        ));
        assert!(owner.context.is_empty());
        assert_eq!(owner.binding.as_ref().unwrap().end, 0);
        assert_eq!(provider.calls.lock().unwrap().len(), 2);
    }
}
#[tokio::test]
async fn cpu_localization_expansion_is_rejected_without_committing_source_context() {
    let (provider, _) = mock(Mode::Expansion);
    let mut owner =
        SourceTextTranslator::new(id(), TranslationQuality::Standard, &provider).unwrap();
    let (_, text) = fixture(1, SpeechLanguage::German);
    assert!(matches!(
        translate(&mut owner, 1, &text, 0..1).await,
        Err(SourceTranslationError::Localization(
            TextLocalizationError::Invalid(ResolverError::InvalidData)
        ))
    ));
    assert!(owner.context.is_empty());
}
#[tokio::test]
async fn ready_result_after_budget_never_starts_review_or_becomes_a_candidate() {
    let (provider, _) = mock(Mode::Blocking);
    let mut owner = SourceTextTranslator::new(id(), TranslationQuality::High, &provider).unwrap();
    let (_, text) = fixture(1, SpeechLanguage::Korean);
    assert!(matches!(
        translate(&mut owner, 1, &text, 0..2).await,
        Err(SourceTranslationError::Translation(
            TranslationError::Timeout
        ))
    ));
    assert_eq!(provider.calls.lock().unwrap().len(), 1);
    assert!(owner.context.is_empty());
}
