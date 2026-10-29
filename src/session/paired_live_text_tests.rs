//! Real bounded Live text task; paired documents, ASR and provider are synthetic.
use super::*;
use crate::{
    ai::{
        speech::SpeechLanguage,
        translation::{
            TranslationCancellation, TranslationCapabilities, TranslationEmitter,
            TranslationFuture, TranslationRequest,
        },
    },
    app::lyrics::{LiveLyrics, LyricsConfiguration, LyricsServiceFactory, LyricsSongBinder},
    audio::capture::SessionId,
    resolver::{
        candidates::{Resolution, SongState},
        contracts::{
            Confidence, ContentType, ExistingText, ExistingTextBundle, ExistingTextProvider,
            SongCandidate, SourceTextReference, SubtitleSource, TextCue, TextFuture, TextLanguage,
            TextQuery, WindowStamp,
        },
        text_session::{ExistingTextSession, TextLookupStatus},
    },
    ui::{
        content::{ContentDisclosure, ContentTransport},
        lyrics::UiLyricsState,
        presentation::TimelineSnapshot,
    },
};
use std::{
    future::Future,
    pin::Pin,
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};

fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(954),
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
fn event(sequence: u64, language: SpeechLanguage, text: &str) -> TranscriptEvent {
    TranscriptEvent {
        identity: identity(),
        sequence,
        utterance: sequence,
        revision: 1,
        kind: TranscriptKind::Final,
        language: Some(language),
        start_us: Some(42_000_000 + (sequence - 1) * 1_000_000),
        duration_us: Some(950_000),
        text: text.into(),
    }
}
#[derive(Clone)]
struct MockPaired {
    target: TextLanguage,
    source: SpeechLanguage,
    source_text: &'static str,
    paired: bool,
    edition: &'static str,
}
impl MockPaired {
    fn primary_text(&self) -> &'static str {
        match self.target {
            TextLanguage::TaiwanTraditional => "軟體與伺服器字幕",
            TextLanguage::OtherTraditional => "軟件與服務器字幕",
            TextLanguage::SimplifiedChinese => "软件与服务器字幕",
            TextLanguage::Source(_) => unreachable!("synthetic Chinese fixture"),
        }
    }
    fn document(&self, query: &TextQuery, language: TextLanguage, primary: bool) -> ExistingText {
        let cues = if primary {
            vec![
                (4_000_000, "較早的合成字幕"),
                (5_000_000, self.primary_text()),
                (6_000_000, "下一句合成字幕"),
            ]
        } else {
            vec![
                (5_000_000, self.source_text),
                (6_000_000, "synthetic source two"),
            ]
        };
        ExistingText {
            identity: query.stamp.identity,
            source: query.source,
            content_id: query.content_id.clone(),
            language,
            cues: cues
                .into_iter()
                .map(|(start, text)| TextCue {
                    start_us: Some(start),
                    duration_us: Some(1_000_000),
                    text: text.into(),
                })
                .collect(),
        }
    }
}
impl ExistingTextProvider for MockPaired {
    fn supports(&self, source: SubtitleSource) -> bool {
        source == SubtitleSource::ExistingLyrics
    }
    fn lookup<'a>(&'a self, _: &'a TextQuery, _: ResolverCancellation) -> TextFuture<'a> {
        Box::pin(async { panic!("paired provider must publish one atomic bundle") })
    }
    fn lookup_bundle<'a>(
        &'a self,
        query: &'a TextQuery,
        _: ResolverCancellation,
    ) -> Pin<Box<dyn Future<Output = Result<Option<ExistingTextBundle>, ResolverError>> + Send + 'a>>
    {
        Box::pin(async move {
            Ok(Some(ExistingTextBundle {
                text: self.document(query, self.target, true),
                source_reference: self.paired.then(|| SourceTextReference {
                    text: self.document(query, TextLanguage::Source(self.source), false),
                    primary_edition_id: self.edition.into(),
                    reference_edition_id: self.edition.into(),
                }),
            }))
        })
    }
}
impl LyricsServiceFactory for MockPaired {
    fn create(&self, _: StreamIdentity) -> Result<Arc<dyn LyricsSongBinder>, ResolverError> {
        Ok(Arc::new(self.clone()))
    }
}
impl LyricsSongBinder for MockPaired {
    fn bind(&self, _: &Resolution) -> Result<Arc<dyn ExistingTextProvider>, ResolverError> {
        Ok(Arc::new(self.clone()))
    }
}
async fn lease(
    target: TextLanguage,
    source: SpeechLanguage,
    source_text: &'static str,
    paired: bool,
    initial: WindowStamp,
) -> ExistingTextLease {
    lease_with_edition(
        target,
        source,
        source_text,
        paired,
        "synthetic edition 1",
        initial,
    )
    .await
}
async fn lease_with_edition(
    target: TextLanguage,
    source: SpeechLanguage,
    source_text: &'static str,
    paired: bool,
    edition: &'static str,
    initial: WindowStamp,
) -> ExistingTextLease {
    let (mut owner, task) = ExistingTextSession::new(identity());
    owner
        .select(
            TextQuery {
                stamp: initial,
                source: SubtitleSource::ExistingLyrics,
                content_id: "synthetic paired work".into(),
            },
            Arc::new(MockPaired {
                target,
                source,
                source_text,
                paired,
                edition,
            }),
        )
        .unwrap();
    let worker = task.run();
    tokio::pin!(worker);
    let mut input = loop {
        tokio::select! {
            result = &mut worker => panic!("synthetic lookup ended: {result:?}"),
            update = owner.next() => {
                let update = update.unwrap();
                if update.status == TextLookupStatus::Available {
                    break ExistingTextLease::from_update(&update).unwrap();
                }
                if let TextLookupStatus::Failed(error) = update.status {
                    panic!("synthetic bundle rejected: {error:?}");
                }
            }
        }
    };
    input.renew(initial, Confidence::new(900).unwrap()).unwrap();
    owner.stop().unwrap();
    worker.await.unwrap();
    input
}
fn provider() -> MockTranslator {
    MockTranslator {
        calls: AtomicUsize::new(0),
    }
}
struct MockTranslator {
    calls: AtomicUsize,
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
        _: &'a TranslationRequest,
        _: &'a mut TranslationEmitter,
        _: TranslationCancellation,
    ) -> TranslationFuture<'a> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok("軟體與伺服器字幕".into())
        })
    }
}
async fn completion<F: Future<Output = Result<(), TextPreparationError>>>(
    owner: &mut LiveTextPipeline,
    worker: &mut Pin<&mut F>,
) -> TextCompletion {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            tokio::select! {
                result = worker.as_mut() => panic!("text task ended: {result:?}"),
                result = owner.changed() => if let Some(done) = result { return done; },
            }
        }
    })
    .await
    .unwrap()
}
fn observation(flow: &mut LiveTranslation<'_>, position: u64) -> PlaybackObservation {
    PlaybackObservation {
        version: flow.sources_mut().version(),
        sequence: 100,
        observed_at: Instant::now(),
        capture_position_us: position,
    }
}

#[tokio::test]
async fn live_lyrics_publishes_atomic_pair_and_clear_withdraws_shared_display() {
    let config = LyricsConfiguration::new(
        ContentDisclosure {
            service: "合成配對歌詞測試",
            transport: ContentTransport::Local,
        },
        Arc::new(MockPaired {
            target: TextLanguage::SimplifiedChinese,
            source: SpeechLanguage::English,
            source_text: "synthetic source one",
            paired: true,
            edition: "synthetic edition 1",
        }),
    )
    .unwrap();
    let resolution = Resolution {
        kind: ContentType::Music,
        song_state: SongState::Confirmed,
        song: Some(Arc::new(SongCandidate {
            id: "synthetic paired work".into(),
            title: "synthetic paired title".into(),
            artist: Some("synthetic paired artist".into()),
            confidence: Confidence::new(900).unwrap(),
            match_offset_us: None,
        })),
        movies: vec![],
    };
    let mut lyrics = LiveLyrics::new(identity(), Some(&config));
    lyrics.select(stamp(1), &resolution).unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(2), lyrics.changed())
            .await
            .unwrap()
            == UiLyricsState::Available
    );
    let input = lyrics.ready_text().unwrap();
    assert!(input.source_reference().is_some());
    assert_eq!(input.content_confidence().unwrap().permille(), 900);
    let translator = provider();
    let mut flow =
        LiveTranslation::new(identity(), TranslationQuality::Standard, &translator).unwrap();
    let source = event(1, SpeechLanguage::English, "synthetic source one");
    flow.ingest(source.clone()).unwrap();
    let (mut owner, task) = LiveTextPipeline::new(identity(), TranslationQuality::Standard);
    owner
        .offer(
            Some(input.clone()),
            Some(&source),
            flow.sources_mut().version(),
        )
        .unwrap();
    let worker = task.isolated().run(&translator);
    tokio::pin!(worker);
    let done = completion(&mut owner, &mut worker).await;
    let obs = observation(&mut flow, 42_500_000);
    assert!(owner.apply(done, &mut flow, Some(obs)).unwrap());
    assert_eq!(flow.timeline().items().len(), 1);
    assert_eq!(
        flow.timeline().items()[0].display_translation(),
        "軟體與伺服器字幕"
    );
    assert_eq!(
        flow.timeline().items()[0].display_origin(),
        SubtitleSource::ExistingLyrics
    );
    assert!(lyrics.clear(None));
    assert!(lyrics.ready_text().is_none());
    assert!(matches!(
        input.validate(identity()),
        Err(ResolverError::Cancelled)
    ));
    assert_eq!(
        flow.timeline().items()[0].display_origin(),
        SubtitleSource::LiveAi
    );
    assert_eq!(flow.reconcile_sources(), Some(source.utterance));
    assert_eq!(flow.timeline().items().len(), 1);
    assert_eq!(translator.calls.load(Ordering::SeqCst), 0);
    owner.stop();
    worker.await.unwrap();
}

#[tokio::test]
async fn older_pair_cannot_replace_current_work_by_changing_edition() {
    let current_stamp = stamp(20);
    let current = lease_with_edition(
        TextLanguage::TaiwanTraditional,
        SpeechLanguage::English,
        "synthetic source one",
        true,
        "synthetic edition 2",
        current_stamp,
    )
    .await;
    let translator = provider();
    let mut flow =
        LiveTranslation::new(identity(), TranslationQuality::Standard, &translator).unwrap();
    let current_final = event(2, SpeechLanguage::English, "synthetic source two");
    flow.ingest(current_final.clone()).unwrap();
    let (mut owner, _task) = LiveTextPipeline::new(identity(), TranslationQuality::Standard);
    assert!(
        owner
            .offer(
                Some(current),
                Some(&current_final),
                flow.sources_mut().version()
            )
            .unwrap()
    );
    let active = owner.active.as_ref().unwrap().clone();
    let ticket = owner.ticket;
    let current_status = owner.status();
    assert!(owner.busy);
    let older_final = event(1, SpeechLanguage::English, "synthetic source one");
    let mut older_revision = current_final.clone();
    older_revision.revision = 0;
    let mut earlier_time = stamp(22);
    earlier_time.ended_at = current_stamp.ended_at - Duration::from_secs(1);
    for (query_stamp, evidence) in [
        (stamp(10), older_final.clone()),
        (stamp(19), current_final.clone()),
        (stamp(21), older_final),
        (stamp(21), older_revision),
        (earlier_time, current_final.clone()),
    ] {
        let older_edition = lease_with_edition(
            TextLanguage::TaiwanTraditional,
            SpeechLanguage::English,
            "synthetic source one",
            true,
            "synthetic edition 1",
            query_stamp,
        )
        .await;
        let error = owner
            .offer(
                Some(older_edition),
                Some(&evidence),
                flow.sources_mut().version(),
            )
            .expect_err("older paired work must be rejected");
        assert!(matches!(
            error,
            TextPreparationError::Lookup(ResolverError::Stale)
        ));
        // The app forwards offer errors through reject; stale delivery must not
        // clear the newer request after its ordering guard has protected it.
        owner.reject(error);
        assert_eq!(owner.ticket, ticket);
        assert!(Arc::ptr_eq(owner.active.as_ref().unwrap(), &active));
        assert!(!active.cancel.is_cancelled());
        assert!(owner.busy);
        assert_eq!(owner.status(), current_status);
    }
    assert_eq!(translator.calls.load(Ordering::SeqCst), 0);
    let invalid = TextPreparationError::Lookup(ResolverError::InvalidData);
    owner.reject(invalid);
    assert!(owner.active.is_none());
    assert!(active.cancel.is_cancelled());
    assert!(!owner.busy);
    assert_eq!(owner.status(), TextPipelineStatus::Failed(invalid.into()));
    owner.stop();
}

#[tokio::test]
async fn different_paired_edition_reprocesses_same_final_but_same_edition_renews_without_work() {
    let mut original = lease(
        TextLanguage::TaiwanTraditional,
        SpeechLanguage::English,
        "synthetic source one",
        true,
        stamp(1),
    )
    .await;
    let translator = provider();
    let mut flow =
        LiveTranslation::new(identity(), TranslationQuality::Standard, &translator).unwrap();
    let source = event(1, SpeechLanguage::English, "synthetic source one");
    flow.ingest(source.clone()).unwrap();
    let (mut owner, task) = LiveTextPipeline::new(identity(), TranslationQuality::Standard);
    owner
        .offer(
            Some(original.clone()),
            Some(&source),
            flow.sources_mut().version(),
        )
        .unwrap();
    let worker = task.isolated().run(&translator);
    tokio::pin!(worker);
    let first = completion(&mut owner, &mut worker).await;
    let obs = observation(&mut flow, 42_500_000);
    assert!(owner.apply(first, &mut flow, Some(obs)).unwrap());

    original
        .renew(stamp(2), Confidence::new(900).unwrap())
        .unwrap();
    assert!(
        !owner
            .offer(
                Some(original.clone()),
                Some(&source),
                flow.sources_mut().version()
            )
            .unwrap()
    );

    let mut replacement = lease_with_edition(
        TextLanguage::TaiwanTraditional,
        SpeechLanguage::English,
        "synthetic source one",
        true,
        "synthetic edition 2",
        stamp(3),
    )
    .await;
    assert!(
        owner
            .offer(
                Some(replacement.clone()),
                Some(&source),
                flow.sources_mut().version()
            )
            .unwrap()
    );
    let replacement_ticket = owner.ticket;
    replacement
        .renew(stamp(4), Confidence::new(900).unwrap())
        .unwrap();
    assert!(
        !owner
            .offer(
                Some(replacement),
                Some(&source),
                flow.sources_mut().version()
            )
            .unwrap()
    );
    assert_eq!(owner.ticket, replacement_ticket);

    // Reference language is part of the pending key even for the same edition.
    // This mismatching synthetic publication is replaced before verification.
    let changed_language = lease_with_edition(
        TextLanguage::TaiwanTraditional,
        SpeechLanguage::French,
        "synthetic source one",
        true,
        "synthetic edition 2",
        stamp(5),
    )
    .await;
    assert!(
        owner
            .offer(
                Some(changed_language),
                Some(&source),
                flow.sources_mut().version()
            )
            .unwrap()
    );
    assert!(owner.ticket > replacement_ticket);

    // Changing Some(reference) to None and back replaces pending same-ASR work.
    // The persistent worker processes only the latest request when polled below.
    let unpaired = lease(
        TextLanguage::TaiwanTraditional,
        SpeechLanguage::English,
        "synthetic source one",
        false,
        stamp(6),
    )
    .await;
    assert!(
        owner
            .offer(Some(unpaired), Some(&source), flow.sources_mut().version())
            .unwrap()
    );
    let latest = lease_with_edition(
        TextLanguage::TaiwanTraditional,
        SpeechLanguage::English,
        "synthetic source one",
        true,
        "synthetic edition 2",
        stamp(7),
    )
    .await;
    assert!(
        owner
            .offer(
                Some(latest.clone()),
                Some(&source),
                flow.sources_mut().version()
            )
            .unwrap()
    );
    let done = completion(&mut owner, &mut worker).await;
    let mut obs = observation(&mut flow, 42_500_000);
    obs.sequence = 101;
    assert!(owner.apply(done, &mut flow, Some(obs)).unwrap());
    assert_eq!(owner.status(), TextPipelineStatus::Ready);
    assert_eq!(flow.timeline().items().len(), 1);
    assert_eq!(translator.calls.load(Ordering::SeqCst), 0);
    // Replacement binds the new atomic publication instead of retaining edition 1.
    original.retire();
    assert_eq!(
        flow.timeline().items()[0].display_origin(),
        SubtitleSource::ExistingLyrics
    );
    latest.retire();
    assert_eq!(
        flow.timeline().items()[0].display_origin(),
        SubtitleSource::LiveAi
    );
    owner.stop();
    worker.await.unwrap();
}

#[tokio::test]
async fn five_languages_and_three_chinese_forms_adopt_without_translation_requests() {
    for (language, text) in [
        (SpeechLanguage::English, "synthetic source one"),
        (SpeechLanguage::Japanese, "これは合成字幕の試験です"),
        (SpeechLanguage::French, "Voici un texte synthétique"),
        (SpeechLanguage::German, "Dies ist ein synthetischer Text"),
        (SpeechLanguage::Korean, "이것은 합성 자막 테스트입니다"),
    ] {
        for target in [
            TextLanguage::TaiwanTraditional,
            TextLanguage::OtherTraditional,
            TextLanguage::SimplifiedChinese,
        ] {
            let input = lease(target, language, text, true, stamp(1)).await;
            let translator = provider();
            let mut flow =
                LiveTranslation::new(identity(), TranslationQuality::Standard, &translator)
                    .unwrap();
            let source = event(1, language, text);
            flow.ingest(source.clone()).unwrap();
            let (mut owner, task) = LiveTextPipeline::new(identity(), TranslationQuality::Standard);
            assert!(
                owner
                    .offer(Some(input), Some(&source), flow.sources_mut().version())
                    .unwrap()
            );
            let worker = task.isolated().run(&translator);
            tokio::pin!(worker);
            let done = completion(&mut owner, &mut worker).await;
            let obs = observation(&mut flow, 42_500_000);
            assert!(owner.apply(done, &mut flow, Some(obs)).unwrap());
            assert_eq!(owner.status(), TextPipelineStatus::Ready);
            assert_eq!(translator.calls.load(Ordering::SeqCst), 0);
            assert_eq!(flow.timeline().items().len(), 1);
            let item = &flow.timeline().items()[0];
            assert_eq!(item.source.text.as_ref(), text);
            assert_eq!(item.display_translation(), "軟體與伺服器字幕");
            assert_eq!(item.display_timestamp_us(), Some(42_000_000));
            assert_eq!(item.existing_display().unwrap().document_cue_index(), 1);
            assert_eq!(item.existing_display().unwrap().media_start_us(), 5_000_000);
            assert_eq!(
                TimelineSnapshot::from_timeline(flow.timeline(), 1).latest_origin(),
                Some(SubtitleSource::ExistingLyrics)
            );
            owner.stop();
            worker.await.unwrap();
        }
    }
}

#[tokio::test]
async fn chinese_localization_never_runs_draft_or_review_at_any_quality() {
    for quality in [
        TranslationQuality::Fast,
        TranslationQuality::Standard,
        TranslationQuality::High,
    ] {
        let input = lease(
            TextLanguage::SimplifiedChinese,
            SpeechLanguage::English,
            "synthetic source one",
            true,
            stamp(1),
        )
        .await;
        let translator = provider();
        let mut flow = LiveTranslation::new(identity(), quality, &translator).unwrap();
        let source = event(1, SpeechLanguage::English, "synthetic source one");
        flow.ingest(source.clone()).unwrap();
        let (mut owner, task) = LiveTextPipeline::new(identity(), quality);
        owner
            .offer(Some(input), Some(&source), flow.sources_mut().version())
            .unwrap();
        let worker = task.isolated().run(&translator);
        tokio::pin!(worker);
        let done = completion(&mut owner, &mut worker).await;
        let obs = observation(&mut flow, 42_500_000);
        assert!(owner.apply(done, &mut flow, Some(obs)).unwrap());
        assert_eq!(translator.calls.load(Ordering::SeqCst), 0);
        owner.stop();
        worker.await.unwrap();
    }
}

#[tokio::test]
async fn matching_live_translation_is_not_a_source_reference() {
    let input = lease(
        TextLanguage::TaiwanTraditional,
        SpeechLanguage::English,
        "synthetic source one",
        false,
        stamp(1),
    )
    .await;
    let translator = provider();
    let mut flow =
        LiveTranslation::new(identity(), TranslationQuality::Standard, &translator).unwrap();
    let source = event(1, SpeechLanguage::English, "synthetic source one");
    flow.ingest(source.clone()).unwrap();
    tokio::time::timeout(Duration::from_secs(1), flow.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        flow.timeline().items()[0].display_translation(),
        "軟體與伺服器字幕"
    );
    assert_eq!(translator.calls.load(Ordering::SeqCst), 1);
    let (mut owner, task) = LiveTextPipeline::new(identity(), TranslationQuality::Standard);
    owner
        .offer(Some(input), Some(&source), flow.sources_mut().version())
        .unwrap();
    let worker = task.isolated().run(&translator);
    tokio::pin!(worker);
    let done = completion(&mut owner, &mut worker).await;
    let obs = observation(&mut flow, 42_500_000);
    assert!(owner.apply(done, &mut flow, Some(obs)).is_err());
    assert_eq!(owner.status(), TextPipelineStatus::Unmatched);
    assert_eq!(flow.sources_mut().candidate_count(), 0);
    assert_eq!(
        flow.timeline().items()[0].display_origin(),
        SubtitleSource::LiveAi
    );
    assert_eq!(translator.calls.load(Ordering::SeqCst), 1);
    owner.stop();
    worker.await.unwrap();
}

#[tokio::test]
async fn retiring_atomic_pair_before_submission_rejects_completed_preparation() {
    let input = lease(
        TextLanguage::TaiwanTraditional,
        SpeechLanguage::English,
        "synthetic source one",
        true,
        stamp(1),
    )
    .await;
    let translator = provider();
    let mut flow =
        LiveTranslation::new(identity(), TranslationQuality::Standard, &translator).unwrap();
    let source = event(1, SpeechLanguage::English, "synthetic source one");
    flow.ingest(source.clone()).unwrap();
    let (mut owner, task) = LiveTextPipeline::new(identity(), TranslationQuality::Standard);
    owner
        .offer(
            Some(input.clone()),
            Some(&source),
            flow.sources_mut().version(),
        )
        .unwrap();
    let worker = task.isolated().run(&translator);
    tokio::pin!(worker);
    let done = completion(&mut owner, &mut worker).await;
    input.retire();
    let obs = observation(&mut flow, 42_500_000);
    assert!(owner.apply(done, &mut flow, Some(obs)).is_err());
    assert_eq!(flow.sources_mut().candidate_count(), 0);
    assert_eq!(
        flow.timeline().items()[0].display_origin(),
        SubtitleSource::LiveAi
    );
    assert_eq!(translator.calls.load(Ordering::SeqCst), 0);
    owner.stop();
    worker.await.unwrap();
}

#[tokio::test]
async fn retiring_pair_after_adoption_restores_live_in_the_same_timeline_item() {
    let input = lease(
        TextLanguage::OtherTraditional,
        SpeechLanguage::English,
        "synthetic source one",
        true,
        stamp(1),
    )
    .await;
    let translator = provider();
    let mut flow =
        LiveTranslation::new(identity(), TranslationQuality::Standard, &translator).unwrap();
    let source = event(1, SpeechLanguage::English, "synthetic source one");
    flow.ingest(source.clone()).unwrap();
    let (mut owner, task) = LiveTextPipeline::new(identity(), TranslationQuality::Standard);
    owner
        .offer(
            Some(input.clone()),
            Some(&source),
            flow.sources_mut().version(),
        )
        .unwrap();
    let worker = task.isolated().run(&translator);
    tokio::pin!(worker);
    let done = completion(&mut owner, &mut worker).await;
    let obs = observation(&mut flow, 42_500_000);
    assert!(owner.apply(done, &mut flow, Some(obs)).unwrap());
    input.retire();
    assert_eq!(
        flow.timeline().items()[0].display_origin(),
        SubtitleSource::LiveAi
    );
    assert_eq!(flow.reconcile_sources(), Some(source.utterance));
    assert_eq!(flow.timeline().items().len(), 1);
    assert_eq!(
        flow.timeline().items()[0].display_original(),
        "synthetic source one"
    );
    assert!(flow.timeline().items()[0].existing_display().is_none());
    assert_eq!(translator.calls.load(Ordering::SeqCst), 0);
    owner.stop();
    worker.await.unwrap();
}

#[tokio::test]
async fn arbiter_epoch_change_rejects_prepared_pair_without_revoking_lookup() {
    let input = lease(
        TextLanguage::TaiwanTraditional,
        SpeechLanguage::English,
        "synthetic source one",
        true,
        stamp(1),
    )
    .await;
    let translator = provider();
    let mut flow =
        LiveTranslation::new(identity(), TranslationQuality::Standard, &translator).unwrap();
    let source = event(1, SpeechLanguage::English, "synthetic source one");
    flow.ingest(source.clone()).unwrap();
    let (mut owner, task) = LiveTextPipeline::new(identity(), TranslationQuality::Standard);
    owner
        .offer(
            Some(input.clone()),
            Some(&source),
            flow.sources_mut().version(),
        )
        .unwrap();
    let worker = task.isolated().run(&translator);
    tokio::pin!(worker);
    let done = completion(&mut owner, &mut worker).await;
    flow.sources_mut().invalidate().unwrap();
    let obs = observation(&mut flow, 42_500_000);
    assert!(owner.apply(done, &mut flow, Some(obs)).is_err());
    assert!(input.validate(identity()).is_ok());
    assert_eq!(flow.sources_mut().candidate_count(), 0);
    assert_eq!(translator.calls.load(Ordering::SeqCst), 0);
    owner.stop();
    worker.await.unwrap();
}

#[tokio::test]
async fn changed_asr_revision_rejects_prepared_pair() {
    let input = lease(
        TextLanguage::TaiwanTraditional,
        SpeechLanguage::English,
        "synthetic source one",
        true,
        stamp(1),
    )
    .await;
    let translator = provider();
    let mut flow =
        LiveTranslation::new(identity(), TranslationQuality::Standard, &translator).unwrap();
    let mut source = event(1, SpeechLanguage::English, "synthetic source one");
    // A pending revision can become a different committed final while bounded
    // preparation runs; already-final timeline rows intentionally cannot mutate.
    let mut pending = source.clone();
    pending.kind = TranscriptKind::Partial;
    pending.revision = 0;
    flow.ingest(pending).unwrap();
    let (mut owner, task) = LiveTextPipeline::new(identity(), TranslationQuality::Standard);
    owner
        .offer(Some(input), Some(&source), flow.sources_mut().version())
        .unwrap();
    let worker = task.isolated().run(&translator);
    tokio::pin!(worker);
    let done = completion(&mut owner, &mut worker).await;
    source.revision = 2;
    source.text = "changed synthetic source".into();
    flow.ingest(source).unwrap();
    let obs = observation(&mut flow, 42_500_000);
    assert!(owner.apply(done, &mut flow, Some(obs)).is_err());
    assert_eq!(flow.sources_mut().candidate_count(), 0);
    assert_eq!(translator.calls.load(Ordering::SeqCst), 0);
    owner.stop();
    worker.await.unwrap();
}

#[tokio::test]
async fn late_old_pair_completion_cannot_override_the_latest_final() {
    let mut input = lease(
        TextLanguage::TaiwanTraditional,
        SpeechLanguage::English,
        "synthetic source one",
        true,
        stamp(1),
    )
    .await;
    let translator = provider();
    let mut flow =
        LiveTranslation::new(identity(), TranslationQuality::Standard, &translator).unwrap();
    let first = event(1, SpeechLanguage::English, "synthetic source one");
    let second = event(2, SpeechLanguage::English, "synthetic source two");
    flow.ingest(first.clone()).unwrap();
    flow.ingest(second.clone()).unwrap();
    let (mut owner, task) = LiveTextPipeline::new(identity(), TranslationQuality::Standard);
    owner
        .offer(
            Some(input.clone()),
            Some(&first),
            flow.sources_mut().version(),
        )
        .unwrap();
    let worker = task.isolated().run(&translator);
    tokio::pin!(worker);
    let old = completion(&mut owner, &mut worker).await;
    input
        .renew(stamp(2), Confidence::new(900).unwrap())
        .unwrap();
    assert!(
        owner
            .offer(Some(input), Some(&second), flow.sources_mut().version())
            .unwrap()
    );
    assert!(!owner.apply(old, &mut flow, None).unwrap());
    assert_eq!(owner.output.max_capacity(), 1);
    let latest = completion(&mut owner, &mut worker).await;
    let obs = observation(&mut flow, 43_500_000);
    assert!(owner.apply(latest, &mut flow, Some(obs)).unwrap());
    assert_eq!(
        flow.timeline().items()[0].display_origin(),
        SubtitleSource::LiveAi
    );
    let current = &flow.timeline().items()[1];
    assert_eq!(current.display_translation(), "下一句合成字幕");
    assert_eq!(current.existing_display().unwrap().document_cue_index(), 2);
    assert_eq!(
        current.existing_display().unwrap().media_start_us(),
        6_000_000
    );
    assert_eq!(translator.calls.load(Ordering::SeqCst), 0);
    owner.stop();
    worker.await.unwrap();
}

#[tokio::test]
async fn stopping_text_worker_releases_adopted_preparation_without_retiring_pair_lookup() {
    let input = lease(
        TextLanguage::TaiwanTraditional,
        SpeechLanguage::English,
        "synthetic source one",
        true,
        stamp(1),
    )
    .await;
    let translator = provider();
    let mut flow =
        LiveTranslation::new(identity(), TranslationQuality::Standard, &translator).unwrap();
    let source = event(1, SpeechLanguage::English, "synthetic source one");
    flow.ingest(source.clone()).unwrap();
    let (mut owner, task) = LiveTextPipeline::new(identity(), TranslationQuality::Standard);
    owner
        .offer(
            Some(input.clone()),
            Some(&source),
            flow.sources_mut().version(),
        )
        .unwrap();
    let worker = task.isolated().run(&translator);
    tokio::pin!(worker);
    let done = completion(&mut owner, &mut worker).await;
    let obs = observation(&mut flow, 42_500_000);
    assert!(owner.apply(done, &mut flow, Some(obs)).unwrap());
    owner.stop();
    worker.await.unwrap();
    assert!(input.validate(identity()).is_ok());
    assert_eq!(
        flow.timeline().items()[0].display_origin(),
        SubtitleSource::LiveAi
    );
    assert_eq!(translator.calls.load(Ordering::SeqCst), 0);
}
