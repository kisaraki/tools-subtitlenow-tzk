//! Synthetic WAVE -> explicit ASR/translation mocks -> Ready -> fuzzy worker.
use super::*;
use crate::{
    ai::speech::{TranscriptEvent, TranscriptKind},
    resolver::contracts::WindowStamp,
    session::fuzzy_proposals::{FuzzyProposalSession, ProposalState},
    subtitle::{fuzzy_alignment::FuzzyTextOrigin, fuzzy_worker::FuzzyCueLocator},
};
fn capture() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(991_118),
        generation: 2,
    }
}
fn input(language: SpeechLanguage) -> TranscriptEvent {
    TranscriptEvent {
        identity: capture(),
        utterance: 1,
        sequence: 7,
        revision: 1,
        start_us: Some(42_000_000),
        duration_us: Some(12_000),
        language: Some(language),
        kind: TranscriptKind::Final,
        text: "synthetic recognized dialogue".into(),
    }
}
fn stamp() -> WindowStamp {
    WindowStamp {
        identity: capture(),
        sequence: 9,
        ended_at: Instant::now(),
        start_us: Some(40_000_000),
        duration_us: 12_000_000,
    }
}
async fn ready(language: SpeechLanguage) -> (Fixture, PreparedSession, Recognizer, TranslatorMock) {
    let fixture = Fixture::new();
    let mut owner = session();
    let task = owner.start(fixture.request(Vec::new())).unwrap();
    let recognizer = Recognizer::new(Mode::Immediate, language);
    let translator = TranslatorMock::new(Mode::Immediate);
    let completion = bounded(task.run_with_recognizer(
        &recognizer,
        &translator,
        SourceLanguage::Auto,
        PreparedSampleRate::Hz24000,
        TranslationQuality::Standard,
        None,
    ))
    .await;
    assert!(owner.apply(completion).unwrap());
    (fixture, owner, recognizer, translator)
}
fn locator(owner: &PreparedSession) -> FuzzyCueLocator {
    let mut locator = owner.fuzzy_locator(capture(), 1).unwrap();
    locator.isolate_workers(Arc::new(Semaphore::new(1)));
    locator
}

const CONTRASTING_TRANSLATION: &str =
    "這是供測試使用的合成臺灣中文譯文不可以作為原文辨識定位的證據";
struct ContrastingTranslator(AtomicUsize);
impl Translator for ContrastingTranslator {
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
        self.0.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(CONTRASTING_TRANSLATION.into()) })
    }
}

#[test]
fn fuzzy_recognized_ready_five_languages_borrow_source_and_native_time_without_new_services() {
    with_fixture_lane(async {
        for language in [
            SpeechLanguage::English,
            SpeechLanguage::Japanese,
            SpeechLanguage::French,
            SpeechLanguage::German,
            SpeechLanguage::Korean,
        ] {
            let (fixture, mut owner, recognizer, translator) = ready(language).await;
            let document = owner.ready().unwrap();
            let original = document.original().recognized().unwrap();
            let borrowed = document.body.alignment_source().unwrap();
            assert_eq!(
                borrowed.cue(0).unwrap().text.as_ptr(),
                original.cues()[0].text.as_ptr()
            );
            assert!(document.original().existing().is_none());
            let locator = locator(&owner);
            let proposal =
                bounded(locator.locate(input(language), stamp(), ResolverCancellation::default()))
                    .await
                    .unwrap();
            assert_eq!(proposal.origin(), FuzzyTextOrigin::PreparedRecognition);
            assert_eq!(proposal.scope().document_identity(), document.identity());
            assert_eq!(proposal.scope().capture_identity(), capture());
            assert_ne!(document.identity(), capture());
            assert_eq!(proposal.media_range_us(), 4_000..16_000);
            assert_eq!(proposal.capture_range_us(), 42_000_000..42_012_000);
            assert_eq!(owner.app_state(), AppState::PreparedReady);
            assert!(!fixture.writable());
            assert_eq!(recognizer.calls.load(Ordering::SeqCst), 1);
            assert_eq!(translator.calls.load(Ordering::SeqCst), 2);
            owner.stop();
            assert!(fixture.writable());
            assert_eq!(
                proposal.validate(locator.scope(), proposal.binding()),
                Err(ResolverError::Cancelled.into())
            );
        }
    });
}

#[test]
fn fuzzy_recognized_completion_is_not_alignment_ready_until_applied() {
    with_fixture_lane(async {
        let fixture = Fixture::new();
        let mut owner = session();
        let task = owner.start(fixture.request(Vec::new())).unwrap();
        let recognizer = Recognizer::new(Mode::Immediate, SpeechLanguage::English);
        let translator = TranslatorMock::new(Mode::Immediate);
        let completion = bounded(task.run_with_recognizer(
            &recognizer,
            &translator,
            SourceLanguage::Auto,
            PreparedSampleRate::Hz24000,
            TranslationQuality::Standard,
            None,
        ))
        .await;
        assert_eq!(
            owner.fuzzy_locator(capture(), 1).err(),
            Some(FuzzyAlignmentError::NotReady)
        );
        assert!(owner.apply(completion).unwrap());
        assert!(owner.fuzzy_locator(capture(), 1).is_ok());
    });
}

#[test]
fn fuzzy_recognized_stop_or_drop_with_unpolled_job_releases_text_and_media() {
    with_fixture_lane(async {
        for drop_owner in [false, true] {
            let (fixture, mut owner, _, _) = ready(SpeechLanguage::English).await;
            let text = Arc::downgrade(&owner.ready().unwrap().body);
            let locator = locator(&owner);
            let request = locator.locate(
                input(SpeechLanguage::English),
                stamp(),
                ResolverCancellation::default(),
            );
            assert_eq!(text.strong_count(), 1);
            if drop_owner {
                drop(owner);
            } else {
                owner.stop();
            }
            assert!(text.upgrade().is_none());
            assert!(fixture.writable());
            assert_eq!(request.await.err(), Some(ResolverError::Cancelled.into()));
        }
    });
}

#[test]
fn fuzzy_recognized_replacement_revokes_old_document_and_keeps_new_provenance() {
    with_fixture_lane(async {
        let (fixture, mut owner, _, _) = ready(SpeechLanguage::English).await;
        let old = locator(&owner);
        let proposal = old
            .locate(
                input(SpeechLanguage::English),
                stamp(),
                ResolverCancellation::default(),
            )
            .await
            .unwrap();
        let text = Arc::downgrade(&owner.ready().unwrap().body);
        let task = owner.start(fixture.chinese()).unwrap();
        assert!(text.upgrade().is_none() && fixture.writable());
        assert_eq!(
            proposal.validate(old.scope(), proposal.binding()),
            Err(ResolverError::Cancelled.into())
        );
        let completion = task
            .run_with_translator(
                &TranslatorMock::new(Mode::Forbidden),
                TranslationQuality::Standard,
                None,
            )
            .await;
        assert!(owner.apply(completion).unwrap());
        assert!(owner.ready().unwrap().original().existing().is_some());
        assert!(owner.ready().unwrap().original().recognized().is_none());
        assert_ne!(
            owner.ready().unwrap().identity(),
            old.scope().document_identity()
        );
        assert_eq!(
            owner.fuzzy_locator(capture(), 2).err(),
            Some(FuzzyAlignmentError::UnsupportedEvidence)
        );
        owner.stop();
        assert!(fixture.writable());
    });
}

#[test]
fn fuzzy_recognized_latest_proposal_session_reacts_to_ready_revocation() {
    with_fixture_lane(async {
        let (fixture, mut ready, _, _) = ready(SpeechLanguage::English).await;
        let text = Arc::downgrade(&ready.ready().unwrap().body);
        let (mut owner, task) = FuzzyProposalSession::new(locator(&ready)).unwrap();
        owner
            .offer(input(SpeechLanguage::English), stamp())
            .unwrap();
        let driver = tokio::spawn(task.run());
        assert_eq!(bounded(owner.changed()).await, ProposalState::Proposed);
        assert_eq!(
            owner.track_status(),
            crate::subtitle::fuzzy_track::FuzzyTrackStatus::Collecting(1)
        );
        assert_eq!(
            owner.proposal().unwrap().origin(),
            FuzzyTextOrigin::PreparedRecognition
        );
        assert_eq!(ready.app_state(), AppState::PreparedReady);
        ready.stop();
        assert!(fixture.writable() && text.upgrade().is_none());
        assert_eq!(bounded(owner.changed()).await, ProposalState::Cancelled);
        assert!(owner.proposal().is_none());
        assert_eq!(driver.await.unwrap(), Err(ResolverError::Cancelled.into()));
    });
}

#[test]
fn fuzzy_recognized_ready_never_uses_translation_or_wrong_language_as_source() {
    with_fixture_lane(async {
        let fixture = Fixture::new();
        let mut owner = session();
        let recognizer = Recognizer::new(Mode::Immediate, SpeechLanguage::English);
        let translator = ContrastingTranslator(AtomicUsize::new(0));
        let task = owner.start(fixture.request(Vec::new())).unwrap();
        let completion = bounded(task.run_with_recognizer(
            &recognizer,
            &translator,
            SourceLanguage::Auto,
            PreparedSampleRate::Hz24000,
            TranslationQuality::Standard,
            None,
        ))
        .await;
        assert!(owner.apply(completion).unwrap());
        let locator = locator(&owner);
        let mut translated = input(SpeechLanguage::English);
        translated.text = owner.ready().unwrap().translated()[0].clone();
        assert!(
            translated
                .text
                .chars()
                .filter(|character| character.is_alphanumeric())
                .count()
                >= crate::subtitle::fuzzy_alignment::MIN_MATCH_ALPHANUMERIC
        );
        assert_eq!(
            locator
                .locate(translated, stamp(), ResolverCancellation::default())
                .await
                .err(),
            Some(FuzzyAlignmentError::NoUniqueMatch)
        );
        assert_eq!(
            locator
                .locate(
                    input(SpeechLanguage::French),
                    stamp(),
                    ResolverCancellation::default()
                )
                .await
                .err(),
            Some(FuzzyAlignmentError::UnsupportedEvidence)
        );
        assert!(
            locator
                .locate(
                    input(SpeechLanguage::English),
                    stamp(),
                    ResolverCancellation::default()
                )
                .await
                .is_ok()
        );
        assert_eq!(recognizer.calls.load(Ordering::SeqCst), 1);
        assert_eq!(translator.0.load(Ordering::SeqCst), 2);
    });
}
