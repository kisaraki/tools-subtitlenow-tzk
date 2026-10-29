//! Real lookup/Live owner/CPU/arbiter path, synthetic services and evidence only.
use super::*;
use crate::{
    ai::{
        speech::{SpeechLanguage, TranscriptEvent, TranscriptKind},
        translation::{
            TranslationCancellation, TranslationCapabilities, TranslationEmitter, TranslationError,
            TranslationFuture, TranslationRequest,
        },
    },
    app::lyrics::{LiveLyrics, LyricsConfiguration, LyricsServiceFactory, LyricsSongBinder},
    audio::capture::SessionId,
    resolver::{
        candidates::{Resolution, SongState},
        contracts::{ContentType, ExistingTextProvider, SongCandidate, SubtitleSource, TextFuture},
    },
    subtitle::{
        source_arbiter::{CueAlignment, PlaybackObservation},
        timeline::TimelineBinding,
    },
    ui::{
        content::{ContentDisclosure, ContentTransport},
        lyrics::UiLyricsState,
    },
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::mpsc;
fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(910),
        generation: 1,
    }
}
fn stamp(sequence: u64) -> WindowStamp {
    WindowStamp {
        identity: identity(),
        sequence,
        ended_at: Instant::now(),
        start_us: Some(40_000_000),
        duration_us: 12_000_000,
    }
}
fn song(confidence: u16) -> Resolution {
    Resolution {
        kind: ContentType::Music,
        song_state: SongState::Confirmed,
        song: Some(Arc::new(SongCandidate {
            id: "synthetic work".into(),
            title: "synthetic title".into(),
            artist: Some("synthetic artist".into()),
            confidence: Confidence::new(confidence).unwrap(),
            match_offset_us: None,
        })),
        movies: vec![],
    }
}
struct MockLookup {
    language: TextLanguage,
    calls: Arc<AtomicUsize>,
}
impl LyricsServiceFactory for MockLookup {
    fn create(&self, _: StreamIdentity) -> Result<Arc<dyn LyricsSongBinder>, ResolverError> {
        Ok(Arc::new(Self {
            language: self.language,
            calls: self.calls.clone(),
        }))
    }
}
impl LyricsSongBinder for MockLookup {
    fn bind(&self, _: &Resolution) -> Result<Arc<dyn ExistingTextProvider>, ResolverError> {
        Ok(Arc::new(Self {
            language: self.language,
            calls: self.calls.clone(),
        }))
    }
}
impl ExistingTextProvider for MockLookup {
    fn supports(&self, source: SubtitleSource) -> bool {
        source == SubtitleSource::ExistingLyrics
    }
    fn lookup<'a>(&'a self, query: &'a TextQuery, _: ResolverCancellation) -> TextFuture<'a> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let strings = match self.language {
                TextLanguage::TaiwanTraditional => ["軟體", "伺服器"],
                TextLanguage::OtherTraditional => ["軟件", "服務器"],
                TextLanguage::SimplifiedChinese => ["软件", "服务器"],
                TextLanguage::Source(_) => ["synthetic source one", "synthetic source two"],
            };
            Ok(Some(ExistingText {
                identity: query.stamp.identity,
                source: query.source,
                content_id: query.content_id.clone(),
                language: self.language,
                cues: strings
                    .into_iter()
                    .enumerate()
                    .map(|(index, text)| TextCue {
                        start_us: Some(5_000_000 + index as u64 * 1_000_000),
                        duration_us: Some(1_000_000),
                        text: text.into(),
                    })
                    .collect(),
            }))
        })
    }
}
async fn setup(language: TextLanguage) -> (LiveLyrics, ExistingTextLease, Arc<AtomicUsize>) {
    setup_at(language, stamp(1)).await
}
async fn setup_at(
    language: TextLanguage,
    initial: WindowStamp,
) -> (LiveLyrics, ExistingTextLease, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let config = LyricsConfiguration::new(
        ContentDisclosure {
            service: "合成文字測試",
            transport: ContentTransport::Local,
        },
        Arc::new(MockLookup {
            language,
            calls: calls.clone(),
        }),
    )
    .unwrap();
    let mut lyrics = LiveLyrics::new(identity(), Some(&config));
    lyrics.select(initial, &song(900)).unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), lyrics.changed())
            .await
            .unwrap(),
        UiLyricsState::Available
    );
    let input = lyrics.ready_text().unwrap();
    (lyrics, input, calls)
}
enum Mode {
    Ok,
    Pending,
    Error,
    Panic,
}
struct MockTranslator {
    mode: Mode,
    calls: AtomicUsize,
    active: Arc<AtomicUsize>,
    started: mpsc::Sender<TranslationCancellation>,
    contexts: Mutex<Vec<usize>>,
}
struct Active(Arc<AtomicUsize>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
fn translator(mode: Mode) -> (MockTranslator, mpsc::Receiver<TranslationCancellation>) {
    let (started, receiver) = mpsc::channel(8);
    (
        MockTranslator {
            mode,
            calls: AtomicUsize::new(0),
            active: Arc::new(AtomicUsize::new(0)),
            started,
            contexts: Mutex::new(Vec::new()),
        },
        receiver,
    )
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
            self.contexts
                .lock()
                .unwrap()
                .push(request.previous_stable.len());
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.active.fetch_add(1, Ordering::SeqCst);
            let _active = Active(self.active.clone());
            let _ = self.started.try_send(cancel);
            match self.mode {
                Mode::Ok => Ok("软件譯文".into()),
                Mode::Pending => std::future::pending().await,
                Mode::Error => Err(TranslationError::Network),
                Mode::Panic => panic!("synthetic translator panic"),
            }
        })
    }
}
fn evidence(
    flow: &mut LiveTranslation<'_>,
    candidate: &PreparedTextCandidate,
) -> CandidateEvidence {
    let text = candidate.localized();
    let original = text.original();
    CandidateEvidence {
        content_confidence: Confidence::new(1000).unwrap(),
        alignment: CueAlignment {
            version: flow.sources_mut().version(),
            stamp: text.stamp(),
            text_sequence: text.stamp().sequence,
            content_id: original.content_id.clone(),
            source: original.source,
            matched_original: original.cues[0].text.clone(),
            cue_index: 0,
            media_start_us: original.cues[0].start_us.unwrap(),
            capture_start_us: 42_000_000,
            confidence: Confidence::new(950).unwrap(),
        },
        availability: ResolverCancellation::default(),
        lookup_availability: None,
        preparation_availability: None,
    }
}
fn adopt(flow: &mut LiveTranslation<'_>, sequence: u64) {
    let event = TranscriptEvent {
        identity: identity(),
        utterance: 1,
        sequence: 1,
        revision: 1,
        start_us: Some(42_000_000),
        duration_us: Some(1_000_000),
        kind: TranscriptKind::Final,
        language: Some(SpeechLanguage::English),
        text: "synthetic ASR".into(),
    };
    flow.ingest(event.clone()).unwrap();
    let obs = PlaybackObservation {
        version: flow.sources_mut().version(),
        sequence,
        observed_at: Instant::now(),
        capture_position_us: 42_000_000,
    };
    flow.arbitrate(TimelineBinding::from_event(&event), obs)
        .unwrap();
}
fn matched_event(language: SpeechLanguage) -> TranscriptEvent {
    TranscriptEvent {
        identity: identity(),
        utterance: 1,
        sequence: 1,
        revision: 1,
        start_us: Some(42_000_000),
        duration_us: Some(950_000),
        kind: TranscriptKind::Final,
        language: Some(language),
        text: "SYNTHETIC source two".into(),
    }
}
static EXACT_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
#[tokio::test]
async fn exact_asr_proof_prepares_only_matched_cue_and_adopts_shared_timeline() {
    let _serial = EXACT_TEST_LOCK.lock().await;
    use crate::subtitle::exact_alignment::ExactCueVerifier;
    for language in SpeechLanguage::ALL {
        let (mut lyrics, input, _) = setup(TextLanguage::Source(language)).await;
        let (provider, _) = translator(Mode::Ok);
        let mut flow =
            LiveTranslation::new(identity(), TranslationQuality::Standard, &provider).unwrap();
        let event = matched_event(language);
        flow.ingest(event.clone()).unwrap();
        let mut verifier = ExactCueVerifier::new(flow.sources_mut().version());
        let matched = verifier
            .verify(input, event, stamp(1), ResolverCancellation::default())
            .await
            .unwrap();
        assert_eq!(matched.timing_agreement().permille(), 950);
        let mut prep =
            TextPreparationSession::new(identity(), TranslationQuality::Standard, &provider)
                .unwrap();
        let (candidate, matched) = prep
            .prepare_matched(matched, ResolverCancellation::default())
            .await
            .unwrap();
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
        let binding = candidate.submit_matched(&mut flow, matched).unwrap();
        let observation = PlaybackObservation {
            version: flow.sources_mut().version(),
            sequence: 1,
            observed_at: Instant::now(),
            capture_position_us: 42_500_000,
        };
        assert!(flow.arbitrate(binding, observation).unwrap());
        assert_eq!(flow.timeline().items().len(), 1);
        let display = flow.timeline().items()[0].existing_display().unwrap();
        assert_eq!(display.document_cue_index(), 1);
        assert_eq!(display.media_start_us(), 6_000_000);
        assert_eq!(
            flow.timeline().items()[0].display_origin(),
            SubtitleSource::ExistingLyrics
        );
        lyrics.clear(None);
        assert_eq!(
            flow.timeline().items()[0].display_origin(),
            SubtitleSource::LiveAi
        );
    }
}
#[tokio::test]
async fn exact_proof_rejects_obsolete_asr_binding_or_arbiter_version_before_submission() {
    let _serial = EXACT_TEST_LOCK.lock().await;
    use crate::subtitle::exact_alignment::{ExactAlignmentError, ExactCueVerifier};
    for reset in [false, true] {
        let (_lyrics, input, _) = setup(TextLanguage::Source(SpeechLanguage::English)).await;
        let (provider, _) = translator(Mode::Ok);
        let mut flow =
            LiveTranslation::new(identity(), TranslationQuality::Standard, &provider).unwrap();
        let event = matched_event(SpeechLanguage::English);
        if reset {
            flow.ingest(event.clone()).unwrap();
        }
        let mut verifier = ExactCueVerifier::new(flow.sources_mut().version());
        let matched = verifier
            .verify(
                input,
                event.clone(),
                stamp(1),
                ResolverCancellation::default(),
            )
            .await
            .unwrap();
        let mut prep =
            TextPreparationSession::new(identity(), TranslationQuality::Standard, &provider)
                .unwrap();
        let candidate = prep
            .prepare(
                matched.input(),
                Some(matched.source_range()),
                ResolverCancellation::default(),
            )
            .await
            .unwrap();
        if reset {
            flow.sources_mut().invalidate().unwrap();
        } else {
            let mut revised = event;
            revised.revision += 1;
            revised.sequence += 1;
            revised.text = "changed synthetic source".into();
            flow.ingest(revised).unwrap();
        }
        assert!(matches!(
            candidate.submit_matched(&mut flow, matched),
            Err(TextPreparationError::Alignment(
                ExactAlignmentError::Obsolete
            ))
        ));
        assert_eq!(flow.sources_mut().candidate_count(), 0);
        assert_eq!(
            flow.timeline().items()[0].display_origin(),
            SubtitleSource::LiveAi
        );
    }
}
#[tokio::test]
async fn matched_preparation_checks_verifier_before_cloud_and_cancels_active_request() {
    let _serial = EXACT_TEST_LOCK.lock().await;
    use crate::subtitle::exact_alignment::ExactCueVerifier;
    for before in [true, false] {
        let (_lyrics, input, _) = setup(TextLanguage::Source(SpeechLanguage::English)).await;
        let (provider, mut started) = translator(Mode::Pending);
        let mut flow =
            LiveTranslation::new(identity(), TranslationQuality::Standard, &provider).unwrap();
        let event = matched_event(SpeechLanguage::English);
        flow.ingest(event.clone()).unwrap();
        let mut verifier = ExactCueVerifier::new(flow.sources_mut().version());
        let matched = verifier
            .verify(
                input.clone(),
                event,
                stamp(1),
                ResolverCancellation::default(),
            )
            .await
            .unwrap();
        let mut prep =
            TextPreparationSession::new(identity(), TranslationQuality::Standard, &provider)
                .unwrap();
        if before {
            verifier.stop();
        }
        let work = prep.prepare_matched(matched, ResolverCancellation::default());
        tokio::pin!(work);
        if !before {
            let active = tokio::select! {
                active = started.recv() => active.unwrap(),
                _ = &mut work => panic!("pending mock finished before verifier stop"),
            };
            verifier.stop();
            assert!(work.await.is_err());
            assert!(active.reason().is_some());
        } else {
            assert!(work.await.is_err());
        }
        assert_eq!(provider.calls.load(Ordering::SeqCst), usize::from(!before));
        assert_eq!(provider.active.load(Ordering::SeqCst), 0);
        assert!(input.validate(identity()).is_ok());
    }
}
#[tokio::test]
async fn exact_proof_rejects_wrong_prepared_range_or_stopped_verifier() {
    let _serial = EXACT_TEST_LOCK.lock().await;
    use crate::subtitle::exact_alignment::ExactCueVerifier;
    for stopped in [false, true] {
        let (_lyrics, input, _) = setup(TextLanguage::Source(SpeechLanguage::English)).await;
        let (provider, _) = translator(Mode::Ok);
        let mut flow =
            LiveTranslation::new(identity(), TranslationQuality::Standard, &provider).unwrap();
        let event = matched_event(SpeechLanguage::English);
        flow.ingest(event.clone()).unwrap();
        let mut verifier = ExactCueVerifier::new(flow.sources_mut().version());
        let matched = verifier
            .verify(input, event, stamp(1), ResolverCancellation::default())
            .await
            .unwrap();
        let mut prep =
            TextPreparationSession::new(identity(), TranslationQuality::Standard, &provider)
                .unwrap();
        let range = if stopped {
            matched.source_range()
        } else {
            0..1
        };
        let candidate = prep
            .prepare(
                matched.input(),
                Some(range),
                ResolverCancellation::default(),
            )
            .await
            .unwrap();
        if stopped {
            verifier.stop();
        }
        assert!(candidate.submit_matched(&mut flow, matched).is_err());
        assert_eq!(flow.sources_mut().candidate_count(), 0);
    }
}
#[tokio::test]
async fn three_chinese_paths_prepare_without_cloud_and_keep_original_time() {
    for language in [
        TextLanguage::TaiwanTraditional,
        TextLanguage::OtherTraditional,
        TextLanguage::SimplifiedChinese,
    ] {
        let (_lyrics, input, _) = setup(language).await;
        let (provider, _) = translator(Mode::Ok);
        let mut prep =
            TextPreparationSession::new(identity(), TranslationQuality::Standard, &provider)
                .unwrap();
        let output = prep
            .prepare(input, None, ResolverCancellation::default())
            .await
            .unwrap();
        assert_eq!(output.localized().translated()[0].as_ref(), "軟體");
        assert_eq!(
            output.localized().original().cues[0].start_us,
            Some(5_000_000)
        );
        assert_eq!(output.document_range(), 0..2);
        assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    }
}
#[tokio::test]
async fn five_source_languages_use_the_selected_provider_and_preserve_window_index() {
    for language in [
        SpeechLanguage::English,
        SpeechLanguage::Japanese,
        SpeechLanguage::French,
        SpeechLanguage::German,
        SpeechLanguage::Korean,
    ] {
        let (_lyrics, input, _) = setup(TextLanguage::Source(language)).await;
        let (provider, _) = translator(Mode::Ok);
        let mut prep =
            TextPreparationSession::new(identity(), TranslationQuality::Standard, &provider)
                .unwrap();
        let output = prep
            .prepare(input, Some(1..2), ResolverCancellation::default())
            .await
            .unwrap();
        assert_eq!(output.document_range(), 1..2);
        assert_eq!(output.localized().translated()[0].as_ref(), "軟體譯文");
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
        let mut flow =
            LiveTranslation::new(identity(), TranslationQuality::Standard, &provider).unwrap();
        let proof = evidence(&mut flow, &output);
        output.submit(&mut flow, proof).unwrap();
        adopt(&mut flow, 1);
        assert_eq!(
            flow.timeline().items()[0]
                .existing_display()
                .unwrap()
                .document_cue_index(),
            1
        );
    }
}
#[tokio::test]
async fn lookup_retirement_invalidates_queued_and_already_adopted_candidates() {
    for submitted in [false, true] {
        let (mut lyrics, input, _) = setup(TextLanguage::SimplifiedChinese).await;
        let (provider, _) = translator(Mode::Ok);
        let mut prep =
            TextPreparationSession::new(identity(), TranslationQuality::Standard, &provider)
                .unwrap();
        let output = prep
            .prepare(input, None, ResolverCancellation::default())
            .await
            .unwrap();
        let mut flow =
            LiveTranslation::new(identity(), TranslationQuality::Standard, &provider).unwrap();
        let proof = evidence(&mut flow, &output);
        if submitted {
            output.submit(&mut flow, proof).unwrap();
            adopt(&mut flow, 1);
            assert_eq!(
                flow.timeline().items()[0].display_origin(),
                SubtitleSource::ExistingLyrics
            );
            lyrics.clear(None);
            assert_eq!(
                flow.timeline().items()[0].display_origin(),
                SubtitleSource::LiveAi
            );
            assert_eq!(flow.reconcile_sources(), Some(1));
        } else {
            lyrics.clear(None);
            assert!(matches!(
                output.submit(&mut flow, proof),
                Err(TextPreparationError::Lookup(ResolverError::Cancelled))
            ));
            assert_eq!(flow.sources_mut().candidate_count(), 0);
        }
    }
}
#[tokio::test]
async fn candidate_retirement_does_not_cancel_the_shared_lookup_publication() {
    let (mut lyrics, input, _) = setup(TextLanguage::SimplifiedChinese).await;
    let (provider, _) = translator(Mode::Ok);
    let mut prep =
        TextPreparationSession::new(identity(), TranslationQuality::Standard, &provider).unwrap();
    let output = prep
        .prepare(input.clone(), None, ResolverCancellation::default())
        .await
        .unwrap();
    let mut flow =
        LiveTranslation::new(identity(), TranslationQuality::Standard, &provider).unwrap();
    let proof = evidence(&mut flow, &output);
    output.submit(&mut flow, proof).unwrap();
    flow.sources_mut().invalidate().unwrap();
    input.validate(identity()).unwrap();
    assert!(lyrics.ready_text().is_some());
    lyrics.stop();
    assert_eq!(input.validate(identity()), Err(ResolverError::Cancelled));
}
#[tokio::test]
async fn clearing_lookup_during_translation_cancels_and_releases_request() {
    let (mut lyrics, input, _) = setup(TextLanguage::Source(SpeechLanguage::English)).await;
    let (provider, mut started) = translator(Mode::Pending);
    let mut prep =
        TextPreparationSession::new(identity(), TranslationQuality::Standard, &provider).unwrap();
    let work = prep.prepare(input, Some(0..1), ResolverCancellation::default());
    tokio::pin!(work);
    let token = tokio::select! { token = started.recv() => token.unwrap(), result = &mut work => panic!("unexpected early result: {}", result.is_ok()) };
    lyrics.clear(None);
    assert!(matches!(
        work.await,
        Err(TextPreparationError::Lookup(ResolverError::Cancelled))
    ));
    assert_eq!(token.reason(), Some(TranslationError::Cancelled));
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn dropping_preparation_future_releases_provider_without_stopping_lookup() {
    let (_lyrics, input, _) = setup(TextLanguage::Source(SpeechLanguage::English)).await;
    let (provider, mut started) = translator(Mode::Pending);
    let mut prep =
        TextPreparationSession::new(identity(), TranslationQuality::Standard, &provider).unwrap();
    let mut work =
        Box::pin(prep.prepare(input.clone(), Some(0..1), ResolverCancellation::default()));
    let token = tokio::select! { token = started.recv() => token.unwrap(), _ = &mut work => panic!("expected pending request") };
    drop(work);
    assert_eq!(token.reason(), Some(TranslationError::Cancelled));
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
    input.validate(identity()).unwrap();
}
#[tokio::test]
async fn explicit_selection_foreign_session_and_cancelled_input_reject_before_cloud() {
    let (_lyrics, input, _) = setup(TextLanguage::Source(SpeechLanguage::English)).await;
    let (provider, _) = translator(Mode::Ok);
    let mut prep =
        TextPreparationSession::new(identity(), TranslationQuality::Standard, &provider).unwrap();
    assert!(matches!(
        prep.prepare(input.clone(), None, ResolverCancellation::default())
            .await,
        Err(TextPreparationError::Selection)
    ));
    let mut foreign = identity();
    foreign.generation += 1;
    let mut other =
        TextPreparationSession::new(foreign, TranslationQuality::Standard, &provider).unwrap();
    assert!(matches!(
        other
            .prepare(input.clone(), Some(0..1), ResolverCancellation::default())
            .await,
        Err(TextPreparationError::Lookup(ResolverError::InvalidData))
    ));
    let cancel = ResolverCancellation::default();
    cancel.cancel();
    assert!(prep.prepare(input, Some(0..1), cancel).await.is_err());
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn fresh_confirmed_selection_renews_new_input_without_refetch_or_mutating_old_stamp() {
    let (mut lyrics, input, lookups) = setup(TextLanguage::Source(SpeechLanguage::English)).await;
    let (provider, _) = translator(Mode::Ok);
    let mut prep =
        TextPreparationSession::new(identity(), TranslationQuality::Standard, &provider).unwrap();
    let first = prep
        .prepare(input.clone(), Some(0..1), ResolverCancellation::default())
        .await
        .unwrap();
    assert!(
        prep.prepare(input.clone(), Some(0..1), ResolverCancellation::default())
            .await
            .is_err()
    );
    assert!(!lyrics.select(stamp(2), &song(900)).unwrap());
    let next = lyrics.ready_text().unwrap();
    assert_eq!(next.query().stamp.sequence, 2);
    assert_eq!(input.query().stamp.sequence, 1);
    assert_eq!(lookups.load(Ordering::SeqCst), 1);
    let second = prep
        .prepare(next, Some(1..2), ResolverCancellation::default())
        .await
        .unwrap();
    assert_eq!(first.localized().stamp().sequence, 1);
    assert_eq!(second.localized().stamp().sequence, 2);
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    assert_eq!(*provider.contexts.lock().unwrap(), [0, 1]);
}
#[tokio::test]
async fn confidence_downgrade_revokes_old_inputs_and_cannot_be_overridden_by_caller() {
    let (mut lyrics, input, _) = setup(TextLanguage::SimplifiedChinese).await;
    let (provider, _) = translator(Mode::Ok);
    let mut prep =
        TextPreparationSession::new(identity(), TranslationQuality::Standard, &provider).unwrap();
    let output = prep
        .prepare(input.clone(), None, ResolverCancellation::default())
        .await
        .unwrap();
    let mut flow =
        LiveTranslation::new(identity(), TranslationQuality::Standard, &provider).unwrap();
    let mut proof = evidence(&mut flow, &output);
    proof.content_confidence = Confidence::new(800).unwrap();
    assert!(output.submit(&mut flow, proof).is_err());
    assert!(!lyrics.select(stamp(2), &song(810)).unwrap());
    assert_eq!(input.validate(identity()), Err(ResolverError::Cancelled));
    let weak = lyrics.ready_text().unwrap();
    assert!(matches!(
        prep.prepare(weak, None, ResolverCancellation::default())
            .await,
        Err(TextPreparationError::Arbitration(
            ArbitrationError::WeakEvidence
        ))
    ));
}
#[tokio::test]
async fn provider_failure_and_panic_return_typed_error_without_poisoning_live() {
    for mode in [Mode::Error, Mode::Panic] {
        let (_lyrics, input, _) = setup(TextLanguage::Source(SpeechLanguage::English)).await;
        let (provider, _) = translator(mode);
        let mut prep =
            TextPreparationSession::new(identity(), TranslationQuality::Standard, &provider)
                .unwrap();
        assert!(
            prep.prepare(input.clone(), Some(0..1), ResolverCancellation::default())
                .await
                .is_err()
        );
        input.validate(identity()).unwrap();
        assert_eq!(provider.active.load(Ordering::SeqCst), 0);
        let flow =
            LiveTranslation::new(identity(), TranslationQuality::Standard, &provider).unwrap();
        assert_eq!(flow.cancellation().reason(), None);
    }
}

#[tokio::test]
async fn preparation_stop_revokes_unsubmitted_and_adopted_results_without_stopping_lookup() {
    for submitted in [false, true] {
        let (lyrics, input, _) = setup(TextLanguage::SimplifiedChinese).await;
        let (provider, _) = translator(Mode::Ok);
        let mut prep =
            TextPreparationSession::new(identity(), TranslationQuality::Standard, &provider)
                .unwrap();
        let output = prep
            .prepare(input.clone(), None, ResolverCancellation::default())
            .await
            .unwrap();
        let mut flow =
            LiveTranslation::new(identity(), TranslationQuality::Standard, &provider).unwrap();
        let proof = evidence(&mut flow, &output);
        if submitted {
            output.submit(&mut flow, proof).unwrap();
            adopt(&mut flow, 1);
            prep.stop();
            assert_eq!(
                flow.timeline().items()[0].display_origin(),
                SubtitleSource::LiveAi
            );
            assert_eq!(flow.reconcile_sources(), Some(1));
        } else {
            drop(prep);
            assert!(matches!(
                output.submit(&mut flow, proof),
                Err(TextPreparationError::Lookup(ResolverError::Cancelled))
            ));
        }
        input.validate(identity()).unwrap();
        assert!(lyrics.ready_text().is_some());
    }
}

#[tokio::test]
async fn original_window_deadline_cancels_slow_preparation_without_poll_renewal() {
    let mut old = stamp(1);
    old.ended_at -= Duration::from_secs(4);
    let (_lyrics, input, _) = setup_at(TextLanguage::Source(SpeechLanguage::English), old).await;
    let (provider, mut started) = translator(Mode::Pending);
    let mut prep =
        TextPreparationSession::new(identity(), TranslationQuality::Standard, &provider).unwrap();
    let work = prep.prepare(input.clone(), Some(0..1), ResolverCancellation::default());
    tokio::pin!(work);
    let token = tokio::select! { token = started.recv() => token.unwrap(), _ = &mut work => panic!("expected pending request") };
    assert!(
        tokio::time::timeout(Duration::from_secs(2), work)
            .await
            .unwrap()
            .is_err()
    );
    assert_eq!(token.reason(), Some(TranslationError::Cancelled));
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
    assert_eq!(input.validate(identity()), Err(ResolverError::Stale));
}
