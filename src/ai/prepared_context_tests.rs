//! Explicit generated summaries and translation mocks, never cloud-quality claims.
use super::*;
use crate::ai::{
    prepared_context::{MAX_PREPARED_SUMMARY_BYTES, PreparedTranslationContext},
    prepared_translation::translate_document,
    translation::{
        TranslationCapabilities, TranslationEmitter, TranslationFuture, TranslationPass,
        TranslationRequest,
    },
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

struct ContextProvider {
    windows: AtomicUsize,
    translations: AtomicUsize,
    pointers: Mutex<Vec<usize>>,
    invalid_window: u64,
}
impl ContextProvider {
    fn new(invalid_window: u64) -> Self {
        Self {
            windows: AtomicUsize::new(0),
            translations: AtomicUsize::new(0),
            pointers: Mutex::new(vec![]),
            invalid_window,
        }
    }
}
impl PreparedGlossaryAnalyzer for ContextProvider {
    fn analyze<'a>(
        &'a self,
        request: &'a PreparedGlossaryRequest,
        _: TranslationCancellation,
    ) -> PreparedGlossaryFuture<'a> {
        Box::pin(async move {
            self.windows.fetch_add(1, Ordering::SeqCst);
            let start = request.cues()[0].cue_index;
            let end = request.cues().last().unwrap().cue_index + 1;
            assert_eq!(
                request.prior_summary(),
                if start == 0 {
                    String::new()
                } else {
                    format!("軟體主題 {start}")
                }
            );
            Ok(PreparedGlossaryResponse {
                identity: request.identity(),
                sequence: request.sequence(),
                proposals: vec![],
                context_summary: if request.sequence() == self.invalid_window {
                    "".into()
                } else {
                    format!("软件主题 {end}").into()
                },
            })
        })
    }
}
impl Translator for ContextProvider {
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
        _: TranslationCancellation,
    ) -> TranslationFuture<'a> {
        Box::pin(async move {
            // Every window completes before even the first draft; every draft
            // and review receives the same final immutable summary allocation.
            assert_eq!(self.windows.load(Ordering::SeqCst), 3);
            let context = request
                .prepared_context
                .as_ref()
                .unwrap()
                .document
                .as_ref()
                .unwrap();
            assert_eq!(context.summary(), "軟體主題 129");
            assert_eq!(context.cue_count(), 129);
            self.pointers
                .lock()
                .unwrap()
                .push(Arc::as_ptr(context) as usize);
            self.translations.fetch_add(1, Ordering::SeqCst);
            Ok(if request.pass == TranslationPass::Review {
                "校訂譯文".into()
            } else {
                "草稿譯文".into()
            })
        })
    }
}

#[test]
fn prepared_context_complete_analysis_precedes_every_draft_and_review_for_all_qualities() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    runtime().block_on(async {
        for quality in [
            TranslationQuality::Fast,
            TranslationQuality::Standard,
            TranslationQuality::High,
        ] {
            let original = document(129, 20);
            let provider = ContextProvider::new(0);
            let cancel = ResolverCancellation::default();
            let analysis = analyze_glossary(
                PreparedTranslationInput::Existing(&original),
                quality,
                &SessionGlossary::new(identity()),
                &provider,
                &cancel,
            )
            .await
            .unwrap();
            let weak = Arc::downgrade(&analysis.context);
            let translated = translate_document(&original, quality, &analysis, &provider, &cancel)
                .await
                .unwrap();
            assert_eq!(translated.len(), 129);
            assert_eq!(
                provider.translations.load(Ordering::SeqCst),
                if quality == TranslationQuality::Fast {
                    129
                } else {
                    258
                }
            );
            let pointers = provider.pointers.lock().unwrap();
            assert!(pointers.iter().all(|pointer| *pointer == pointers[0]));
            assert_eq!(
                analysis.document_context().unwrap().summary(),
                "軟體主題 129"
            );
            drop(analysis);
            assert!(weak.upgrade().is_none());
            assert!(!cancel.is_cancelled());
        }
    });
}
#[test]
fn prepared_context_invalid_later_summary_never_reaches_translation() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    runtime().block_on(async {
        let original = document(129, 20);
        let provider = ContextProvider::new(2);
        let cancel = ResolverCancellation::default();
        let result = analyze_glossary(
            PreparedTranslationInput::Existing(&original),
            TranslationQuality::Standard,
            &SessionGlossary::new(identity()),
            &provider,
            &cancel,
        )
        .await;
        assert!(matches!(result, Err(TranslationError::InvalidData)));
        assert_eq!(provider.windows.load(Ordering::SeqCst), 2);
        assert_eq!(provider.translations.load(Ordering::SeqCst), 0);
        assert!(!cancel.is_cancelled()); // A typed analysis failure is not a user stop.
    });
}
#[test]
fn prepared_context_summary_utf8_byte_and_control_limits_are_enforced() {
    let request = request("Aurora Engine");
    for invalid in [
        "".to_owned(),
        " \n\t".to_owned(),
        "a\0b".to_owned(),
        "字".repeat(1366),
        "x".repeat(MAX_PREPARED_SUMMARY_BYTES + 1),
    ] {
        let mut result = response(&request, vec![]);
        result.context_summary = invalid.into();
        assert!(matches!(
            result.validate(&request),
            Err(TranslationError::InvalidData)
        ));
    }
    let exact = "x".repeat(MAX_PREPARED_SUMMARY_BYTES);
    let mut result = response(&request, vec![]);
    result.context_summary = exact.clone().into();
    assert!(result.validate(&request).is_ok());
    assert!(super::request("source").with_prior_summary(&exact).is_ok());
    assert!(
        super::request("source")
            .with_prior_summary(&"x".repeat(MAX_PREPARED_SUMMARY_BYTES + 1))
            .is_err()
    );
    assert!(
        super::request("source")
            .with_prior_summary("bad\0data")
            .is_err()
    );
}
#[test]
fn prepared_context_foreign_session_language_quality_or_document_count_fails_before_provider() {
    runtime().block_on(async {
        let original = document(129, 20);
        let provider = ContextProvider::new(0);
        let mut foreign = identity();
        foreign.generation += 1;
        for (id, language, quality, count, error) in [
            (
                foreign,
                SpeechLanguage::English,
                TranslationQuality::Standard,
                129,
                TranslationError::Stale,
            ),
            (
                identity(),
                SpeechLanguage::Japanese,
                TranslationQuality::Standard,
                129,
                TranslationError::InvalidData,
            ),
            (
                identity(),
                SpeechLanguage::English,
                TranslationQuality::High,
                129,
                TranslationError::InvalidData,
            ),
            (
                identity(),
                SpeechLanguage::English,
                TranslationQuality::Standard,
                128,
                TranslationError::InvalidData,
            ),
        ] {
            let analysis = PreparedTextAnalysis {
                glossary: SessionGlossary::new(identity()),
                context: Arc::new(
                    PreparedDocumentContext::new(id, language, quality, count, "合成脈絡".into())
                        .unwrap(),
                ),
            };
            let result = translate_document(
                &original,
                TranslationQuality::Standard,
                &analysis,
                &provider,
                &ResolverCancellation::default(),
            )
            .await;
            assert!(matches!(result,Err(actual) if actual==error));
        }
        assert_eq!(provider.translations.load(Ordering::SeqCst), 0);
    });
}
