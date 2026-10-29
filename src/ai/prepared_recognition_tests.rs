//! Explicit mock speech and synthetic RAM PCM only; no keys, files or cloud.
use super::*;
use crate::{
    ai::{
        prepared_speech::{PreparedChunkStamp, PreparedSpeechCapabilities, PreparedSpeechCue},
        speech::SpeechLanguage,
    },
    audio::capture::SessionId,
};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::Semaphore;

fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(605),
        generation: 1,
    }
}
fn rate() -> PreparedSampleRate {
    PreparedSampleRate::Hz16000
}
fn chunk(sequence: u64) -> PreparedSpeechChunk {
    let body = 1_000_000 + (sequence - 1) * 10_000_000;
    custom_chunk(sequence, 1, body, false)
}
fn custom_chunk(sequence: u64, run_id: u64, body: u64, discontinuity: bool) -> PreparedSpeechChunk {
    let prefix = if sequence == 1 || discontinuity {
        0
    } else {
        500_000
    };
    let stamp = PreparedChunkStamp {
        identity: identity(),
        sequence,
        run_id,
        rate: rate(),
        audio_start_us: body - prefix,
        body_start_us: body,
        body_end_us: body + 10_000_000,
        discontinuity,
    };
    let frames = (10_500_000 + prefix) * 16_000 / 1_000_000;
    PreparedSpeechChunk::new(stamp, vec![0; frames as usize]).unwrap()
}
fn tail(chunk: PreparedSpeechChunk) -> PreparedSpeechChunk {
    let stamp = chunk.stamp();
    let frames =
        (stamp.body_end_us - stamp.audio_start_us) * u64::from(stamp.rate.hz()) / 1_000_000;
    PreparedSpeechChunk::new(stamp, chunk.samples()[..frames as usize].to_vec()).unwrap()
}
fn caps(concurrency: usize) -> PreparedSpeechCapabilities {
    PreparedSpeechCapabilities {
        cancellation: true,
        timestamped_finals: true,
        supported_languages: [true; 5],
        constrained_auto_detection: true,
        sample_rates: [true; 2],
        max_concurrent: concurrency,
    }
}
fn reply(request: &PreparedSpeechRequest) -> PreparedSpeechResponse {
    let stamp = request.chunk().stamp();
    PreparedSpeechResponse {
        identity: stamp.identity,
        sequence: stamp.sequence,
        run_id: stamp.run_id,
        language: Some(
            SpeechLanguage::selected(request.language()).unwrap_or(SpeechLanguage::English),
        ),
        cues: vec![PreparedSpeechCue {
            start_us: stamp.body_start_us - stamp.audio_start_us + 100_000,
            duration_us: 200_000,
            text: format!("source-{}", stamp.sequence).into_boxed_str(),
        }],
    }
}
#[derive(Clone, Copy)]
enum Mode {
    Success,
    OrderedGate,
    Pending,
    Credential,
    CancelSuccess,
    ParentCancelSuccess,
    SyncPanic,
    FuturePanic,
    CapabilityPanic,
    Mixed,
    Silence,
    Boundary,
}
struct Mock {
    mode: Mode,
    capabilities: PreparedSpeechCapabilities,
    calls: AtomicUsize,
    active: AtomicUsize,
    peak: AtomicUsize,
    contexts: Mutex<Vec<(u64, Vec<String>)>>,
    tokens: Mutex<Vec<ResolverCancellation>>,
    first: Semaphore,
    third: Semaphore,
    started: Semaphore,
    parent: Option<ResolverCancellation>,
}
impl Mock {
    fn new(mode: Mode, concurrency: usize) -> Self {
        Self {
            mode,
            capabilities: caps(concurrency),
            calls: AtomicUsize::new(0),
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            contexts: Mutex::new(Vec::new()),
            tokens: Mutex::new(Vec::new()),
            first: Semaphore::new(0),
            third: Semaphore::new(0),
            started: Semaphore::new(0),
            parent: None,
        }
    }
}
struct Active<'a>(&'a Mock);
impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
    }
}
impl PreparedSpeechRecognizer for Mock {
    fn capabilities(&self) -> PreparedSpeechCapabilities {
        if matches!(self.mode, Mode::CapabilityPanic) {
            panic!("explicit mock capability panic");
        }
        self.capabilities
    }
    fn recognize<'a>(
        &'a self,
        request: &'a PreparedSpeechRequest,
        cancel: ResolverCancellation,
    ) -> super::super::prepared_speech::PreparedSpeechFuture<'a> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if matches!(self.mode, Mode::SyncPanic) {
            panic!("explicit mock method panic");
        }
        Box::pin(async move {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(active, Ordering::SeqCst);
            let _guard = Active(self);
            let stamp = request.chunk().stamp();
            self.contexts.lock().unwrap().push((
                stamp.sequence,
                request
                    .previous_source()
                    .iter()
                    .map(|text| text.to_string())
                    .collect(),
            ));
            self.tokens.lock().unwrap().push(cancel.clone());
            self.started.add_permits(1);
            match self.mode {
                Mode::Pending => {
                    cancel.cancelled().await;
                    return Err(PreparedSpeechError::Cancelled);
                }
                Mode::Credential => return Err(PreparedSpeechError::Credential),
                Mode::FuturePanic => panic!("explicit mock future panic"),
                Mode::OrderedGate if stamp.sequence == 1 => {
                    tokio::select! {biased;_=cancel.cancelled()=>return Err(PreparedSpeechError::Cancelled),permit=self.first.acquire()=>{permit.unwrap().forget();}}
                }
                Mode::OrderedGate if stamp.sequence == 3 => self.third.add_permits(1),
                _ => {}
            }
            let mut result = reply(request);
            match self.mode {
                Mode::CancelSuccess => cancel.cancel(),
                Mode::ParentCancelSuccess => self.parent.as_ref().unwrap().cancel(),
                Mode::Mixed if stamp.sequence > 1 => {
                    result.language = Some(SpeechLanguage::Japanese)
                }
                Mode::Silence => {
                    result.language = None;
                    result.cues.clear();
                }
                Mode::Boundary => result.cues[0].duration_us = 10_000_000,
                _ => {}
            }
            Ok(result)
        })
    }
}
fn session(language: SourceLanguage, cancel: &ResolverCancellation) -> PreparedRecognitionSession {
    PreparedRecognitionSession::new(identity(), language, rate(), cancel).unwrap()
}

#[tokio::test]
async fn out_of_order_finals_restore_input_order_and_never_exceed_provider_limit() {
    let cancel = ResolverCancellation::default();
    let mut session = session(SourceLanguage::English, &cancel);
    let provider = Mock::new(Mode::OrderedGate, 2);
    let orchestrate = async {
        tokio::time::timeout(Duration::from_secs(5), provider.third.acquire())
            .await
            .unwrap()
            .unwrap()
            .forget();
        assert_eq!(provider.peak.load(Ordering::SeqCst), 2);
        provider.first.add_permits(1);
    };
    let (result, ()) = tokio::join!(
        session.recognize_batch(
            (1..=4)
                .map(|seq| if seq == 4 {
                    tail(chunk(seq))
                } else {
                    chunk(seq)
                })
                .collect(),
            &provider
        ),
        orchestrate
    );
    result.unwrap();
    assert_eq!(provider.calls.load(Ordering::SeqCst), 4);
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
    assert!(!cancel.is_cancelled());
    let document = session.finish().unwrap();
    assert_eq!(document.identity(), identity());
    assert_eq!(document.language(), SpeechLanguage::English);
    assert_eq!(
        document
            .cues()
            .iter()
            .map(|cue| cue.text.as_ref())
            .collect::<Vec<_>>(),
        vec!["source-1", "source-2", "source-3", "source-4"]
    );
    assert_eq!(document.cues()[3].start_us, 31_100_000);
}

#[tokio::test]
async fn successful_batches_preserve_committed_source_context_and_clear_it_at_gap() {
    let cancel = ResolverCancellation::default();
    let mut session = session(SourceLanguage::Auto, &cancel);
    let provider = Mock::new(Mode::Success, 1);
    session
        .recognize_batch((1..=4).map(chunk).collect(), &provider)
        .await
        .unwrap();
    session
        .recognize_batch(vec![tail(chunk(5))], &provider)
        .await
        .unwrap();
    session
        .recognize_batch(vec![tail(custom_chunk(6, 2, 100_000_000, true))], &provider)
        .await
        .unwrap();
    let contexts = provider.contexts.lock().unwrap();
    assert_eq!(
        contexts.iter().find(|(seq, _)| *seq == 5).unwrap().1,
        vec!["source-1", "source-2", "source-3", "source-4"]
    );
    assert!(
        contexts
            .iter()
            .find(|(seq, _)| *seq == 6)
            .unwrap()
            .1
            .is_empty()
    );
    drop(contexts);
    assert_eq!(session.finish().unwrap().cues().len(), 6);
}

#[tokio::test]
async fn all_five_selected_languages_reach_only_the_supplied_provider() {
    for language in [
        SourceLanguage::English,
        SourceLanguage::Japanese,
        SourceLanguage::French,
        SourceLanguage::German,
        SourceLanguage::Korean,
    ] {
        let cancel = ResolverCancellation::default();
        let mut session = session(language, &cancel);
        let provider = Mock::new(Mode::Success, 1);
        session
            .recognize_batch(vec![tail(chunk(1))], &provider)
            .await
            .unwrap();
        assert_eq!(
            session.finish().unwrap().language(),
            SpeechLanguage::selected(language).unwrap()
        );
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn dropping_unpolled_batch_cancels_and_immediately_clears_existing_source_text() {
    let cancel = ResolverCancellation::default();
    let mut session = session(SourceLanguage::English, &cancel);
    let provider = Mock::new(Mode::Success, 1);
    session
        .recognize_batch(vec![chunk(1)], &provider)
        .await
        .unwrap();
    assert_eq!(
        session.merger.as_ref().unwrap().context(1)[0].as_ref(),
        "source-1"
    );
    let future = session.recognize_batch(vec![chunk(2)], &provider);
    drop(future);
    assert!(cancel.is_cancelled());
    assert!(session.merger.is_none());
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn batch_ownership_discards_excess_capacity_and_rejects_oversized_pcm_before_polling() {
    let mut chunks = Vec::with_capacity(100);
    chunks.push(chunk(1));
    let bounded = bounded_batch(chunks).unwrap().into_vec();
    assert_eq!(bounded.len(), 1);
    assert_eq!(bounded.capacity(), 1);
    assert_eq!(
        bounded_batch((1..=5).map(chunk).collect()).err(),
        Some(PreparedSpeechError::InvalidData)
    );
}

#[tokio::test]
async fn dropping_started_batch_cancels_remote_tokens_and_clears_all_private_rows() {
    let cancel = ResolverCancellation::default();
    let mut session = session(SourceLanguage::English, &cancel);
    let provider = Mock::new(Mode::Pending, 2);
    let mut future = Box::pin(session.recognize_batch(vec![chunk(1), chunk(2)], &provider));
    tokio::select! { result=&mut future=>panic!("mock request completed: {result:?}"), permit=provider.started.acquire()=>permit.unwrap().forget() }
    drop(future);
    assert!(cancel.is_cancelled());
    assert!(session.merger.is_none());
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
    assert!(
        provider
            .tokens
            .lock()
            .unwrap()
            .iter()
            .all(ResolverCancellation::is_cancelled)
    );
}

#[tokio::test]
async fn external_cancel_rejects_pending_batch_without_transcript() {
    let cancel = ResolverCancellation::default();
    let mut session = session(SourceLanguage::English, &cancel);
    let provider = Mock::new(Mode::Pending, 2);
    let control = async {
        provider.started.acquire().await.unwrap().forget();
        cancel.cancel();
    };
    let (result, ()) = tokio::join!(session.recognize_batch(vec![chunk(1)], &provider), control);
    assert_eq!(result, Err(PreparedSpeechError::Cancelled));
    assert!(session.merger.is_none());
    assert_eq!(session.finish().err(), Some(PreparedSpeechError::Cancelled));
}

#[tokio::test]
async fn offline_timeout_has_separate_budget_and_cancels_request_scope() {
    let cancel = ResolverCancellation::default();
    let mut session = session(SourceLanguage::English, &cancel);
    let provider = Mock::new(Mode::Pending, 1);
    let result = session
        .batch_with_timeout(vec![chunk(1)], &provider, Duration::from_millis(20))
        .await;
    assert_eq!(result, Err(PreparedSpeechError::Timeout));
    assert!(cancel.is_cancelled());
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
    assert!(
        provider
            .tokens
            .lock()
            .unwrap()
            .iter()
            .all(ResolverCancellation::is_cancelled)
    );
    assert_eq!(PREPARED_RECOGNITION_TIMEOUT, Duration::from_secs(60));
}

#[tokio::test]
async fn provider_failure_after_previous_batch_removes_all_source_rows() {
    let cancel = ResolverCancellation::default();
    let mut session = session(SourceLanguage::English, &cancel);
    session
        .recognize_batch(vec![chunk(1)], &Mock::new(Mode::Success, 1))
        .await
        .unwrap();
    let provider = Mock::new(Mode::Credential, 2);
    assert_eq!(
        session
            .recognize_batch(vec![chunk(2), chunk(3)], &provider)
            .await,
        Err(PreparedSpeechError::Credential)
    );
    assert!(session.merger.is_none());
    assert!(cancel.is_cancelled());
    assert_eq!(session.finish().err(), Some(PreparedSpeechError::Cancelled));
}

#[tokio::test]
async fn request_cancelling_itself_then_returning_success_is_never_committed() {
    let cancel = ResolverCancellation::default();
    let mut session = session(SourceLanguage::English, &cancel);
    assert_eq!(
        session
            .recognize_batch(vec![chunk(1)], &Mock::new(Mode::CancelSuccess, 1))
            .await,
        Err(PreparedSpeechError::Cancelled)
    );
    assert!(session.merger.is_none());
}

#[tokio::test]
async fn parent_cancel_immediately_before_success_rejects_late_response_and_releases_children() {
    let cancel = ResolverCancellation::default();
    let mut session = session(SourceLanguage::English, &cancel);
    session
        .recognize_batch(vec![chunk(1)], &Mock::new(Mode::Success, 1))
        .await
        .unwrap();
    let mut provider = Mock::new(Mode::ParentCancelSuccess, 2);
    provider.parent = Some(cancel.clone());
    assert_eq!(
        session
            .recognize_batch(vec![chunk(2), chunk(3)], &provider)
            .await,
        Err(PreparedSpeechError::Cancelled)
    );
    assert!(cancel.is_cancelled());
    assert!(session.merger.is_none());
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
    assert!(
        provider
            .tokens
            .lock()
            .unwrap()
            .iter()
            .all(ResolverCancellation::is_cancelled)
    );
}

#[tokio::test]
async fn synchronous_future_and_capability_panics_are_typed_without_partial_result() {
    for mode in [Mode::SyncPanic, Mode::FuturePanic, Mode::CapabilityPanic] {
        let cancel = ResolverCancellation::default();
        let mut session = session(SourceLanguage::English, &cancel);
        assert_eq!(
            session
                .recognize_batch(vec![chunk(1)], &Mock::new(mode, 1))
                .await,
            Err(PreparedSpeechError::InvalidData)
        );
        assert!(cancel.is_cancelled());
        assert!(session.merger.is_none());
    }
}

#[tokio::test]
async fn unconfigured_timing_or_auto_capability_never_invokes_provider() {
    for variant in 0..3 {
        let cancel = ResolverCancellation::default();
        let mut session = session(SourceLanguage::Auto, &cancel);
        let mut provider = Mock::new(Mode::Success, 1);
        match variant {
            0 => provider.capabilities.timestamped_finals = false,
            1 => provider.capabilities.constrained_auto_detection = false,
            _ => provider.capabilities.max_concurrent = 5,
        }
        let expected = if variant == 1 {
            PreparedSpeechError::UnsupportedLanguage
        } else {
            PreparedSpeechError::UnsupportedCapability
        };
        assert_eq!(
            session.recognize_batch(vec![chunk(1)], &provider).await,
            Err(expected)
        );
        assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn empty_oversized_or_wrong_session_batches_fail_before_service_work() {
    for kind in 0..3 {
        let cancel = ResolverCancellation::default();
        let mut session = session(SourceLanguage::English, &cancel);
        let provider = Mock::new(Mode::Success, 2);
        let chunks = match kind {
            0 => vec![],
            1 => (1..=5).map(chunk).collect(),
            _ => {
                let stamp = PreparedChunkStamp {
                    identity: StreamIdentity {
                        session: SessionId(606),
                        generation: 1,
                    },
                    ..chunk(1).stamp()
                };
                vec![PreparedSpeechChunk::new(stamp, vec![0; 168_000]).unwrap()]
            }
        };
        let expected = if kind == 2 {
            PreparedSpeechError::Stale
        } else {
            PreparedSpeechError::InvalidData
        };
        assert_eq!(
            session.recognize_batch(chunks, &provider).await,
            Err(expected)
        );
        assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn invalid_input_order_is_rejected_before_starting_any_request() {
    let cancel = ResolverCancellation::default();
    let mut session = session(SourceLanguage::English, &cancel);
    let provider = Mock::new(Mode::Success, 2);
    assert_eq!(
        session
            .recognize_batch(vec![chunk(1), chunk(3)], &provider)
            .await,
        Err(PreparedSpeechError::InvalidData)
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn auto_mixed_language_and_crossing_sentence_boundaries_cannot_become_transcript() {
    for (mode, expected) in [
        (Mode::Mixed, PreparedSpeechError::MixedLanguages),
        (Mode::Boundary, PreparedSpeechError::BoundaryMismatch),
    ] {
        let cancel = ResolverCancellation::default();
        let mut session = session(SourceLanguage::Auto, &cancel);
        assert_eq!(
            session
                .recognize_batch(vec![chunk(1), chunk(2)], &Mock::new(mode, 2))
                .await,
            Err(expected)
        );
        assert!(session.merger.is_none());
    }
}

#[tokio::test]
async fn complete_silence_is_no_speech_and_never_fake_success() {
    let cancel = ResolverCancellation::default();
    let mut session = session(SourceLanguage::Auto, &cancel);
    session
        .recognize_batch(vec![tail(chunk(1))], &Mock::new(Mode::Silence, 1))
        .await
        .unwrap();
    assert_eq!(session.finish().err(), Some(PreparedSpeechError::NoSpeech));
    assert!(cancel.is_cancelled());
}

#[test]
fn session_drop_and_explicit_cancel_release_scope_and_source_owner() {
    let cancel = ResolverCancellation::default();
    let session = session(SourceLanguage::English, &cancel);
    drop(session);
    assert!(cancel.is_cancelled());
    let second = ResolverCancellation::default();
    let mut session = super::PreparedRecognitionSession::new(
        identity(),
        SourceLanguage::English,
        rate(),
        &second,
    )
    .unwrap();
    session.cancel();
    assert!(second.is_cancelled());
    assert!(session.merger.is_none());
}
