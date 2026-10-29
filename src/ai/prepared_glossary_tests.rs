//! Synthetic source evidence and explicit mock analysis; no cloud or credentials.
use super::*;
use crate::{
    audio::capture::SessionId,
    resolver::contracts::{ExistingText, SubtitleSource, TextCue, TextLanguage},
};
use std::sync::Mutex;
#[path = "prepared_context_tests.rs"]
mod context_tests;
fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(8800),
        generation: 2,
    }
}
fn request(text: &str) -> PreparedGlossaryRequest {
    PreparedGlossaryRequest::new(
        identity(),
        1,
        SpeechLanguage::English,
        TranslationQuality::Standard,
        vec![PreparedGlossaryCue {
            cue_index: 7,
            text: text.into(),
        }],
        vec![],
    )
    .unwrap()
}
fn proposal(index: usize, start: usize, end: usize, target: &str) -> PreparedGlossaryProposal {
    PreparedGlossaryProposal {
        cue_index: index,
        source_byte_start: start,
        source_byte_end: end,
        taiwan_translation: target.into(),
    }
}
fn response(
    request: &PreparedGlossaryRequest,
    proposals: Vec<PreparedGlossaryProposal>,
) -> PreparedGlossaryResponse {
    PreparedGlossaryResponse {
        identity: request.identity(),
        sequence: request.sequence(),
        proposals,
        context_summary: "合成全文脈絡".into(),
    }
}
fn document(count: usize, bytes: usize) -> ExistingText {
    ExistingText {
        identity: identity(),
        source: SubtitleSource::ExistingSubtitle,
        content_id: "synthetic-glossary".into(),
        language: TextLanguage::Source(SpeechLanguage::English),
        cues: (0..count)
            .map(|index| TextCue {
                start_us: Some(index as u64 * 2_000_000),
                duration_us: Some(1_000_000),
                text: format!("Aurora Engine {}", "x".repeat(bytes.saturating_sub(14))).into(),
            })
            .collect(),
    }
}
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}
#[test]
fn proposal_requires_exact_session_sequence_cue_and_utf8_source_range() {
    let request = request("前言 Aurora Engine");
    assert!(
        response(&request, vec![proposal(7, 7, 20, "极光引擎")])
            .validate(&request)
            .is_ok()
    );
    for entry in [
        proposal(7, 1, 5, "譯名"),
        proposal(7, 8, 7, "譯名"),
        proposal(8, 0, 3, "譯名"),
        proposal(7, 0, 0, "譯名"),
        proposal(7, 0, 999, "譯名"),
        proposal(7, 0, 3, " \t"),
        proposal(7, 0, 3, &"字".repeat(86)),
    ] {
        assert_eq!(
            response(&request, vec![entry]).validate(&request),
            Err(TranslationError::InvalidData)
        );
    }
    let mut stale = response(&request, vec![]);
    stale.identity.generation += 1;
    assert_eq!(stale.validate(&request), Err(TranslationError::Stale));
    stale.identity = identity();
    stale.sequence += 1;
    assert_eq!(stale.validate(&request), Err(TranslationError::Stale));
}
#[test]
fn request_rejects_invalid_identity_limits_noncontiguous_cues_and_duplicate_terms() {
    for (id, sequence, cues, terms) in [
        (
            StreamIdentity {
                session: SessionId(0),
                generation: 2,
            },
            1,
            vec![PreparedGlossaryCue {
                cue_index: 0,
                text: "valid".into(),
            }],
            vec![],
        ),
        (
            identity(),
            0,
            vec![PreparedGlossaryCue {
                cue_index: 0,
                text: "valid".into(),
            }],
            vec![],
        ),
        (identity(), 1, vec![], vec![]),
        (
            identity(),
            1,
            vec![
                PreparedGlossaryCue {
                    cue_index: 0,
                    text: "a".into(),
                },
                PreparedGlossaryCue {
                    cue_index: 2,
                    text: "b".into(),
                },
            ],
            vec![],
        ),
        (
            identity(),
            1,
            vec![PreparedGlossaryCue {
                cue_index: 0,
                text: "a".into(),
            }],
            vec![
                GlossaryEntry {
                    source: "a".into(),
                    taiwan_translation: "甲".into(),
                },
                GlossaryEntry {
                    source: "a".into(),
                    taiwan_translation: "乙".into(),
                },
            ],
        ),
        (
            identity(),
            1,
            (0..9)
                .map(|index| PreparedGlossaryCue {
                    cue_index: index,
                    text: "x".repeat(MAX_CUE_BYTES).into(),
                })
                .collect(),
            vec![],
        ),
    ] {
        assert!(matches!(
            PreparedGlossaryRequest::new(
                id,
                sequence,
                SpeechLanguage::English,
                TranslationQuality::Fast,
                cues,
                terms
            ),
            Err(TranslationError::InvalidData)
        ));
    }
}
enum Mode {
    Terms,
    Empty,
    Conflict,
    Oversized,
    Stale,
    Fail,
    SyncPanic,
    FuturePanic,
    Pending,
    CancelRequest,
    CancelRoot,
}
struct Mock {
    mode: Mode,
    seen: Mutex<Vec<(u64, Vec<usize>, usize)>>,
    token: Mutex<Option<TranslationCancellation>>,
    root: ResolverCancellation,
}
impl Mock {
    fn new(mode: Mode, root: ResolverCancellation) -> Self {
        Self {
            mode,
            seen: Mutex::new(vec![]),
            token: Mutex::new(None),
            root,
        }
    }
}
impl PreparedGlossaryAnalyzer for Mock {
    fn analyze<'a>(
        &'a self,
        request: &'a PreparedGlossaryRequest,
        cancel: TranslationCancellation,
    ) -> PreparedGlossaryFuture<'a> {
        if matches!(self.mode, Mode::SyncPanic) {
            panic!("synthetic analysis call panic");
        }
        Box::pin(async move {
            self.seen.lock().unwrap().push((
                request.sequence(),
                request.cues().iter().map(|cue| cue.cue_index).collect(),
                request.known_terms().len(),
            ));
            *self.token.lock().unwrap() = Some(cancel.clone());
            match self.mode {
                Mode::FuturePanic => panic!("synthetic analysis future panic"),
                Mode::Fail => return Err(TranslationError::Credential),
                Mode::Pending => futures_util::future::pending::<()>().await,
                Mode::CancelRequest => cancel.cancel(),
                Mode::CancelRoot => self.root.cancel(),
                _ => {}
            }
            let proposals = match self.mode {
                Mode::Terms | Mode::Conflict => vec![proposal(
                    request.cues()[0].cue_index,
                    0,
                    13,
                    if matches!(self.mode, Mode::Conflict) && request.sequence() > 1 {
                        "不同譯名"
                    } else {
                        "极光引擎"
                    },
                )],
                Mode::Oversized => (0..65)
                    .map(|_| proposal(request.cues()[0].cue_index, 0, 13, "極光引擎"))
                    .collect(),
                _ => vec![],
            };
            let mut result = response(request, proposals);
            if matches!(self.mode, Mode::Stale) {
                result.identity.generation += 1;
            }
            Ok(result)
        })
    }
}
#[test]
fn complete_document_is_analyzed_before_freezing_one_localized_glossary() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    runtime().block_on(async {
        for (count, bytes) in [(129, 20), (19, 8192)] {
            let original = document(count, bytes);
            let cancel = ResolverCancellation::default();
            let provider = Mock::new(Mode::Terms, cancel.clone());
            let terms = analyze_glossary(
                PreparedTranslationInput::Existing(&original),
                TranslationQuality::High,
                &SessionGlossary::new(identity()),
                &provider,
                &cancel,
            )
            .await
            .unwrap();
            assert_eq!(terms.glossary.entries().len(), 1);
            assert_eq!(terms.glossary.entries()[0].source.as_ref(), "Aurora Engine");
            assert_eq!(
                terms.glossary.entries()[0].taiwan_translation.as_ref(),
                "極光引擎"
            );
            let seen = provider.seen.lock().unwrap();
            assert_eq!(
                seen.iter()
                    .flat_map(|row| row.1.iter().copied())
                    .collect::<Vec<_>>(),
                (0..count).collect::<Vec<_>>()
            );
            assert_eq!(seen[0].2, 0);
            assert!(seen[1..].iter().all(|row| row.2 == 1));
            assert_eq!(seen.len(), 3);
            assert!(!cancel.is_cancelled());
        }
    });
}
#[test]
fn empty_analysis_still_traverses_every_cue_and_manual_terms_are_authoritative() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    runtime().block_on(async {
        let original = document(65, 20);
        let cancel = ResolverCancellation::default();
        for mode in [Mode::Empty, Mode::Terms, Mode::Conflict] {
            let provider = Mock::new(mode, cancel.clone());
            let mut seed = SessionGlossary::new(identity());
            seed.set(identity(), "Aurora Engine", "使用者譯名").unwrap();
            let terms = analyze_glossary(
                PreparedTranslationInput::Existing(&original),
                TranslationQuality::Fast,
                &seed,
                &provider,
                &cancel,
            )
            .await
            .unwrap();
            assert_eq!(
                terms.glossary.entries()[0].taiwan_translation.as_ref(),
                "使用者譯名"
            );
            assert_eq!(provider.seen.lock().unwrap().len(), 2);
            assert_eq!(seed.entries().len(), 1);
        }
    });
}
#[test]
fn generated_conflicts_and_typed_provider_failures_discard_the_pass() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    runtime().block_on(async {
        let original = document(65, 20);
        for (mode, expected) in [
            (Mode::Conflict, TranslationError::InvalidData),
            (Mode::Oversized, TranslationError::Overloaded),
            (Mode::Stale, TranslationError::Stale),
            (Mode::Fail, TranslationError::Credential),
            (Mode::SyncPanic, TranslationError::InvalidData),
            (Mode::FuturePanic, TranslationError::InvalidData),
        ] {
            let cancel = ResolverCancellation::default();
            let provider = Mock::new(mode, cancel.clone());
            let seed = SessionGlossary::new(identity());
            let result = analyze_glossary(
                PreparedTranslationInput::Existing(&original),
                TranslationQuality::Standard,
                &seed,
                &provider,
                &cancel,
            )
            .await;
            assert!(matches!(result, Err(error) if error == expected));
            assert!(seed.entries().is_empty());
            assert!(!cancel.is_cancelled());
            if let Some(token) = provider.token.lock().unwrap().as_ref() {
                assert!(token.reason().is_some());
            }
        }
    });
}
#[test]
fn cancellation_late_success_timeout_and_future_drop_revoke_active_requests() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    runtime().block_on(async {
        let original = document(1, 20);
        let seed = SessionGlossary::new(identity());
        for mode in [Mode::CancelRequest, Mode::CancelRoot, Mode::Pending] {
            let cancel = ResolverCancellation::default();
            let provider = Mock::new(mode, cancel.clone());
            let result = analyze_with_timeout(
                PreparedTranslationInput::Existing(&original),
                TranslationQuality::Fast,
                &seed,
                &provider,
                &cancel,
                Duration::from_millis(10),
            )
            .await;
            assert!(matches!(
                result,
                Err(TranslationError::Cancelled | TranslationError::Timeout)
            ));
            assert!(
                provider
                    .token
                    .lock()
                    .unwrap()
                    .as_ref()
                    .unwrap()
                    .reason()
                    .is_some()
            );
        }
        let cancel = ResolverCancellation::default();
        let provider = Mock::new(Mode::Pending, cancel.clone());
        let mut future = Box::pin(analyze_glossary(
            PreparedTranslationInput::Existing(&original),
            TranslationQuality::Fast,
            &seed,
            &provider,
            &cancel,
        ));
        assert!(futures_util::poll!(&mut future).is_pending());
        drop(future);
        assert_eq!(
            provider.token.lock().unwrap().as_ref().unwrap().reason(),
            Some(TranslationError::Cancelled)
        );
        assert!(!cancel.is_cancelled());
    });
}
#[test]
fn old_session_seed_and_invalid_document_fail_before_provider_calls() {
    runtime().block_on(async {
        let mut original = document(1, 20);
        let cancel = ResolverCancellation::default();
        let provider = Mock::new(Mode::Empty, cancel.clone());
        let seed = SessionGlossary::new(StreamIdentity {
            session: SessionId(8801),
            generation: 2,
        });
        assert!(matches!(
            analyze_glossary(
                PreparedTranslationInput::Existing(&original),
                TranslationQuality::Fast,
                &seed,
                &provider,
                &cancel
            )
            .await,
            Err(TranslationError::InvalidData)
        ));
        original.cues[0].duration_us = Some(0);
        assert!(matches!(
            analyze_glossary(
                PreparedTranslationInput::Existing(&original),
                TranslationQuality::Fast,
                &SessionGlossary::new(identity()),
                &provider,
                &cancel
            )
            .await,
            Err(TranslationError::InvalidData)
        ));
        assert!(provider.seen.lock().unwrap().is_empty());
    });
}
#[test]
fn cpu_saturation_is_explicit_and_does_not_publish_partial_terms() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    runtime().block_on(async {
        let workers = crate::media::work::workers();
        let _busy = workers
            .clone()
            .acquire_many_owned(crate::media::work::worker_limit() as u32)
            .await
            .unwrap();
        let original = document(1, 20);
        let cancel = ResolverCancellation::default();
        let provider = Mock::new(Mode::Terms, cancel.clone());
        assert!(matches!(
            analyze_glossary(
                PreparedTranslationInput::Existing(&original),
                TranslationQuality::Standard,
                &SessionGlossary::new(identity()),
                &provider,
                &cancel
            )
            .await,
            Err(TranslationError::Overloaded)
        ));
    });
}

#[test]
fn maximum_document_is_fully_covered_and_the_65th_unique_term_fails_explicitly() {
    struct Unique;
    impl PreparedGlossaryAnalyzer for Unique {
        fn analyze<'a>(
            &'a self,
            request: &'a PreparedGlossaryRequest,
            _: TranslationCancellation,
        ) -> PreparedGlossaryFuture<'a> {
            Box::pin(async move {
                Ok(response(
                    request,
                    request
                        .cues()
                        .iter()
                        .map(|cue| proposal(cue.cue_index, 0, cue.text.len(), "測試譯名"))
                        .collect(),
                ))
            })
        }
    }
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    runtime().block_on(async {
        let original = document(MAX_TEXT_CUES, 512);
        let cancel = ResolverCancellation::default();
        let empty = Mock::new(Mode::Empty, cancel.clone());
        let result = analyze_glossary(
            PreparedTranslationInput::Existing(&original),
            TranslationQuality::Fast,
            &SessionGlossary::new(identity()),
            &empty,
            &cancel,
        )
        .await
        .unwrap();
        assert!(result.glossary.entries().is_empty());
        assert_eq!(result.context.cue_count(), MAX_TEXT_CUES);
        {
            let seen = empty.seen.lock().unwrap();
            assert_eq!(seen.len(), 128);
            assert_eq!(
                seen.iter()
                    .flat_map(|row| row.1.iter().copied())
                    .collect::<Vec<_>>(),
                (0..MAX_TEXT_CUES).collect::<Vec<_>>()
            );
        }
        let mut original = document(65, 20);
        for (index, cue) in original.cues.iter_mut().enumerate() {
            cue.text = format!("term_{index}").into();
        }
        assert!(matches!(
            analyze_glossary(
                PreparedTranslationInput::Existing(&original),
                TranslationQuality::Standard,
                &SessionGlossary::new(identity()),
                &Unique,
                &cancel
            )
            .await,
            Err(TranslationError::Overloaded)
        ));
    });
}

#[test]
fn request_cancellation_during_queued_cpu_merge_prevents_glossary_delivery() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(async {
        // Occupy only this test runtime's blocking worker. The real merge can
        // acquire its CPU lease but cannot run before the cancellation arrives.
        let (release, wait) = std::sync::mpsc::sync_channel::<()>(1);
        let (started, ready) = tokio::sync::oneshot::channel();
        let blocker = tokio::task::spawn_blocking(move || {
            let _ = started.send(());
            let _ = wait.recv();
        });
        ready.await.unwrap();
        let original = document(1, 20);
        let cancel = ResolverCancellation::default();
        let provider = Mock::new(Mode::Terms, cancel.clone());
        let seed = SessionGlossary::new(identity());
        let mut work = Box::pin(analyze_glossary(
            PreparedTranslationInput::Existing(&original),
            TranslationQuality::Standard,
            &seed,
            &provider,
            &cancel,
        ));
        assert!(futures_util::poll!(&mut work).is_pending());
        provider.token.lock().unwrap().as_ref().unwrap().cancel();
        let result = tokio::time::timeout(Duration::from_millis(100), work).await;
        release.send(()).unwrap();
        blocker.await.unwrap();
        assert!(matches!(result, Ok(Err(TranslationError::Cancelled))));
        assert!(!cancel.is_cancelled());
    });
    drop(runtime); // Reaps the aborted queued closure and its CPU lease.
    assert_eq!(
        crate::media::work::workers().available_permits(),
        crate::media::work::worker_limit()
    );
}
