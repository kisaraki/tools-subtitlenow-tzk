//! Real local Ready preparation, synthetic source text and an explicit translator mock.
use super::*;
use crate::{
    ai::{
        speech::{SpeechLanguage, StreamIdentity, TranscriptKind},
        translation::{
            TranslationCancellation, TranslationCapabilities, TranslationEmitter,
            TranslationFuture, TranslationRequest, Translator,
        },
    },
    app::state::AppState,
    audio::capture::SessionId,
    domain::TranslationQuality,
    media::{
        input::{LocalSubtitleInput, MediaInput},
        planning::PreparedRequest,
    },
    resolver::{contracts::TextLanguage, timed_subtitle::SubtitleFormat},
    session::prepared::{PreparedSession, PreparedState},
};
use std::{
    fs::{self, OpenOptions},
    path::PathBuf,
    sync::{
        Weak,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    task::Poll,
    time::Duration,
};
use tokio::sync::oneshot;

const LINE: &str = "the synthetic subtitle belongs to this document";

#[test]
fn fuzzy_alignment_control_ready_pause_and_source_withdrawal_release_media_and_text() {
    use crate::session::{
        alignment_control::{AlignmentControlState, AlignmentController},
        fuzzy_proposals::ProposalState,
    };
    lane(async {
        let fixture = Fixture::new();
        let translator = MockTranslator::default();
        let mut ready = fixture
            .ready(
                TextLanguage::Source(SpeechLanguage::English),
                SubtitleFormat::Srt,
                &[LINE],
                &translator,
            )
            .await;
        let borrowed = locator(&ready, Arc::new(Semaphore::new(1)));
        let text = weak(&borrowed);
        drop(borrowed);
        let calls = translator.calls.load(Ordering::SeqCst);
        let mut owner = AlignmentController::from_prepared(&ready, capture()).unwrap();
        assert_eq!(owner.state(), AlignmentControlState::Paused);
        let first = owner.resume().await.unwrap();
        owner
            .offer(event(SpeechLanguage::English, LINE), stamp())
            .unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), owner.changed())
                .await
                .unwrap()
                .unwrap(),
            ProposalState::Proposed
        );
        assert_eq!(owner.proposal().unwrap().scope(), first);
        owner.pause().await.unwrap();
        assert_eq!(ready.app_state(), AppState::PreparedReady);
        assert!(text.upgrade().is_some());
        let second = owner.resume().await.unwrap();
        assert_ne!(first.epoch(), second.epoch());
        owner.pause().await.unwrap();
        ready.stop();
        assert!(text.upgrade().is_none());
        #[cfg(windows)]
        assert!(fixture.writable());
        assert_eq!(
            owner.resume().await,
            Err(FuzzyAlignmentError::Invalid(ResolverError::Cancelled).into())
        );
        owner.stop().await.unwrap();
        assert_eq!(translator.calls.load(Ordering::SeqCst), calls);
    });
}

#[test]
fn fuzzy_adjacent_pair_ready_uses_original_text_without_new_services_and_revokes_on_stop() {
    lane(async {
        let fixture = Fixture::new();
        let lines = [
            "first synthetic adjacent sentence",
            "second synthetic adjacent sentence",
        ];
        let request = fixture.request(
            TextLanguage::Source(SpeechLanguage::English),
            SubtitleFormat::Srt,
            &lines,
        );
        fs::write(
            fixture.directory.path().join("explicit-subtitle.data"),
            format!(
                "1\n00:10:00,000 --> 00:10:01,000\n{}\n\n2\n00:10:01,000 --> 00:10:02,000\n{}\n",
                lines[0], lines[1]
            ),
        )
        .unwrap();
        let translator = MockTranslator::default();
        let mut session = PreparedSession::new();
        let completion = session
            .start(request)
            .unwrap()
            .run_with_translator(&translator, TranslationQuality::Standard, None)
            .await;
        assert!(session.apply(completion).unwrap());
        let calls = translator.calls.load(Ordering::SeqCst);
        let locator = locator(&session, Arc::new(Semaphore::new(1)));
        let text = weak(&locator);
        let proposal = locator
            .locate(
                event(SpeechLanguage::English, &lines.join(" ")),
                stamp(),
                ResolverCancellation::default(),
            )
            .await
            .unwrap();
        assert_eq!(proposal.cue_range(), 0..2);
        assert_eq!(proposal.media_range_us(), 600_000_000..602_000_000);
        assert_eq!(translator.calls.load(Ordering::SeqCst), calls);
        session.stop();
        assert!(text.upgrade().is_none());
        #[cfg(windows)]
        assert!(fixture.writable());
        assert_eq!(
            proposal.validate(locator.scope(), proposal.binding()),
            Err(ResolverError::Cancelled.into())
        );
    });
}

#[test]
fn fuzzy_track_ready_withdrawal_revokes_metadata_without_retaining_text_or_media() {
    use crate::{
        session::fuzzy_proposals::{FuzzyProposalSession, ProposalState},
        subtitle::fuzzy_track::FuzzyTrackStatus,
    };
    lane(async {
        let fixture = Fixture::new();
        let lines: Vec<String> = ['a', 'b', 'c']
            .iter()
            .map(|letter| letter.to_string().repeat(32))
            .collect();
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let translator = MockTranslator::default();
        let mut ready = fixture
            .ready(
                TextLanguage::Source(SpeechLanguage::English),
                SubtitleFormat::Srt,
                &refs,
                &translator,
            )
            .await;
        let calls = translator.calls.load(Ordering::SeqCst);
        let locator = locator(&ready, Arc::new(Semaphore::new(1)));
        let text = weak(&locator);
        let (mut owner, task) = FuzzyProposalSession::new(locator).unwrap();
        let driver = tokio::spawn(task.run());
        for (index, line) in lines.iter().enumerate() {
            // This SRT fixture spaces cues ten minutes apart. These are
            // synthetic coordinates, not elapsed playback or capture proof.
            let step_us = index as u64 * 600_000_000;
            let mut input = event(SpeechLanguage::English, line);
            input.utterance = index as u64 + 1;
            input.sequence = index as u64 + 1;
            input.start_us = Some(42_000_000 + step_us);
            let mut window = stamp();
            window.sequence += index as u64;
            window.start_us = Some(40_000_000 + step_us);
            owner.offer(input, window).unwrap();
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(2), owner.changed())
                    .await
                    .unwrap(),
                ProposalState::Proposed
            );
        }
        assert_eq!(owner.track_status(), FuzzyTrackStatus::Consistent(3));
        let track = owner.track_proposal().unwrap();
        track.validate(owner.scope()).unwrap();
        assert_eq!(translator.calls.load(Ordering::SeqCst), calls);
        ready.stop();
        assert!(text.upgrade().is_none());
        #[cfg(windows)]
        assert!(fixture.writable());
        assert_eq!(
            track.validate(owner.scope()),
            Err(ResolverError::Cancelled.into())
        );
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), owner.changed())
                .await
                .unwrap(),
            ProposalState::Cancelled
        );
        assert_eq!(driver.await.unwrap(), Err(ResolverError::Cancelled.into()));
    });
}

#[test]
fn fuzzy_proposals_ready_revocation_wakes_idle_pending_and_delivered_owners() {
    use crate::session::fuzzy_proposals::{FuzzyProposalSession, ProposalState};
    lane(async {
        for mode in 0..3 {
            let fixture = Fixture::new();
            let mut ready = fixture
                .ready(
                    TextLanguage::Source(SpeechLanguage::English),
                    SubtitleFormat::Srt,
                    &[LINE],
                    &MockTranslator::default(),
                )
                .await;
            let locator = locator(&ready, Arc::new(Semaphore::new(1)));
            let text = weak(&locator);
            let (mut owner, task) = FuzzyProposalSession::new(locator).unwrap();
            if mode > 0 {
                owner
                    .offer(event(SpeechLanguage::English, LINE), stamp())
                    .unwrap();
            }
            let mut driver = if mode == 2 {
                Some(tokio::spawn(task.run()))
            } else {
                None
            };
            // Idle and unpolled cases keep only the revocable weak text lease.
            // For mode 2 the active driver must preserve the same ancestry bound.
            if mode == 2 {
                assert_eq!(
                    tokio::time::timeout(Duration::from_secs(2), owner.changed())
                        .await
                        .unwrap(),
                    ProposalState::Proposed
                );
                assert!(owner.proposal().is_some());
            }
            ready.stop();
            assert!(text.upgrade().is_none());
            #[cfg(windows)]
            assert!(fixture.writable());
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(2), owner.changed())
                    .await
                    .unwrap(),
                ProposalState::Cancelled
            );
            assert!(owner.proposal().is_none() && owner.binding().is_none());
            if let Some(driver) = driver.take() {
                assert_eq!(driver.await.unwrap(), Err(ResolverError::Cancelled.into()));
            }
        }
    });
}

#[test]
fn fuzzy_proposals_stop_does_not_cancel_prepared_ready_or_another_locator() {
    use crate::session::fuzzy_proposals::{FuzzyProposalSession, ProposalState};
    lane(async {
        let fixture = Fixture::new();
        let ready = fixture
            .ready(
                TextLanguage::Source(SpeechLanguage::English),
                SubtitleFormat::Srt,
                &[LINE],
                &MockTranslator::default(),
            )
            .await;
        let first = locator(&ready, Arc::new(Semaphore::new(1)));
        let second = locator(&ready, Arc::new(Semaphore::new(1)));
        let (mut owner, task) = FuzzyProposalSession::new(first).unwrap();
        owner
            .offer(event(SpeechLanguage::English, LINE), stamp())
            .unwrap();
        owner.stop();
        assert_eq!(owner.state(), ProposalState::Stopped);
        assert_eq!(task.run().await, Ok(()));
        assert_eq!(ready.app_state(), AppState::PreparedReady);
        assert!(
            second
                .locate(
                    event(SpeechLanguage::English, LINE),
                    stamp(),
                    ResolverCancellation::default()
                )
                .await
                .is_ok()
        );
    });
}
fn capture() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(991_117),
        generation: 2,
    }
}
fn event(language: SpeechLanguage, text: &str) -> TranscriptEvent {
    TranscriptEvent {
        identity: capture(),
        utterance: 2,
        sequence: 7,
        revision: 1,
        start_us: Some(42_000_000),
        duration_us: Some(2_000_000),
        language: Some(language),
        kind: TranscriptKind::Final,
        text: text.into(),
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
fn lane<F: Future>(future: F) -> F::Output {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}
struct Fixture {
    directory: tempfile::TempDir,
    media: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let media = directory.path().join("synthetic-private.media");
        fs::write(&media, b"synthetic opaque media; no audio decoding").unwrap();
        Self { directory, media }
    }
    fn request(
        &self,
        language: TextLanguage,
        format: SubtitleFormat,
        lines: &[&str],
    ) -> PreparedRequest {
        let path = self.directory.path().join("explicit-subtitle.data");
        let mut body = if format == SubtitleFormat::WebVtt {
            "WEBVTT\n\n".to_owned()
        } else {
            String::new()
        };
        for (index, text) in lines.iter().enumerate() {
            if format == SubtitleFormat::Srt {
                body.push_str(&format!("{}\n", index + 1));
            }
            let separator = if format == SubtitleFormat::Srt {
                ','
            } else {
                '.'
            };
            body.push_str(&format!(
                "00:{:02}:00{separator}000 --> 00:{:02}:02{separator}000\n{text}\n\n",
                10 + index * 10,
                10 + index * 10
            ));
        }
        fs::write(&path, body).unwrap();
        PreparedRequest::new(
            MediaInput::local(self.media.clone()).unwrap(),
            vec![LocalSubtitleInput::new(path, format, language).unwrap()],
        )
        .unwrap()
    }
    async fn ready(
        &self,
        language: TextLanguage,
        format: SubtitleFormat,
        lines: &[&str],
        translator: &MockTranslator,
    ) -> PreparedSession {
        let mut session = PreparedSession::new();
        let task = session
            .start(self.request(language, format, lines))
            .unwrap();
        let completion = task
            .run_with_translator(translator, TranslationQuality::Standard, None)
            .await;
        assert!(session.apply(completion).unwrap());
        assert_eq!(session.app_state(), AppState::PreparedReady);
        session
    }
    #[cfg(windows)]
    fn writable(&self) -> bool {
        OpenOptions::new().write(true).open(&self.media).is_ok()
    }
}
#[derive(Default)]
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
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok("合成字幕測試譯文。".into()) })
    }
}
fn locator(session: &PreparedSession, cpu: Arc<Semaphore>) -> FuzzyCueLocator {
    let mut locator = session.fuzzy_locator(capture(), 1).unwrap();
    locator.workers = cpu;
    locator
}
fn weak(locator: &FuzzyCueLocator) -> Weak<PreparedTextOwner> {
    match locator.document.as_ref().unwrap() {
        AlignmentDocument::Prepared(lease) => lease.text.clone(),
        AlignmentDocument::Owned(_)
        | AlignmentDocument::Paired(_)
        | AlignmentDocument::Lookup(_) => {
            panic!("expected Ready fixture")
        }
    }
}
async fn pending<F: Future>(mut future: std::pin::Pin<&mut F>) {
    std::future::poll_fn(|cx| {
        assert!(future.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
}
fn gate() -> (
    oneshot::Sender<()>,
    oneshot::Receiver<()>,
    mpsc::SyncSender<()>,
    mpsc::Receiver<()>,
) {
    let (entered, entered_rx) = oneshot::channel();
    let (release, release_rx) = mpsc::sync_channel(1);
    (entered, entered_rx, release, release_rx)
}
fn gated(
    entered: oneshot::Sender<()>,
    release: mpsc::Receiver<()>,
) -> impl FnOnce(&WorkInput) -> Result<FuzzyCueProposal, FuzzyAlignmentError> + Send {
    move |input| {
        entered.send(()).unwrap();
        release.recv_timeout(Duration::from_secs(5)).unwrap();
        run_match(input)
    }
}
async fn reclaim(cpu: Arc<Semaphore>) {
    drop(
        tokio::time::timeout(Duration::from_secs(2), cpu.acquire_owned())
            .await
            .unwrap()
            .unwrap(),
    );
}

#[test]
fn five_languages_srt_vtt_ready_borrows_original_without_copy_or_playback() {
    lane(async {
        for (language, line) in [
            (SpeechLanguage::English, LINE),
            (
                SpeechLanguage::Japanese,
                "これは文字の編集距離を検証するための合成字幕です",
            ),
            (
                SpeechLanguage::French,
                "cette réplique synthétique vérifie les caractères",
            ),
            (
                SpeechLanguage::German,
                "diese künstliche untertitelzeile prüft die zeichen",
            ),
            (
                SpeechLanguage::Korean,
                "이것은문자편집거리를검증하기위한합성자막문장입니다",
            ),
        ] {
            for format in [SubtitleFormat::Srt, SubtitleFormat::WebVtt] {
                let fixture = Fixture::new();
                let translator = MockTranslator::default();
                let mut session = fixture
                    .ready(TextLanguage::Source(language), format, &[line], &translator)
                    .await;
                let original_ptr = session.ready().unwrap().original().existing().unwrap().cues[0]
                    .text
                    .as_ptr() as usize;
                let locator = locator(&session, Arc::new(Semaphore::new(1)));
                let weak = weak(&locator);
                assert_eq!(weak.strong_count(), 1);
                let proposal = locator
                    .locate_using(
                        event(language, &format!("{line}!")),
                        stamp(),
                        ResolverCancellation::default(),
                        move |input| {
                            assert_eq!(
                                input
                                    .document
                                    .original()
                                    .unwrap()
                                    .cue(0)
                                    .unwrap()
                                    .text
                                    .as_ptr() as usize,
                                original_ptr
                            );
                            run_match(input)
                        },
                    )
                    .await
                    .unwrap();
                assert_eq!(
                    proposal.scope().document_identity(),
                    session.identity().unwrap()
                );
                assert_ne!(proposal.scope().document_identity(), capture());
                assert_eq!(
                    proposal.origin(),
                    crate::subtitle::fuzzy_alignment::FuzzyTextOrigin::ExistingText
                );
                assert_eq!(proposal.scope().capture_identity(), capture());
                assert_eq!(proposal.media_range_us(), 600_000_000..602_000_000);
                assert_eq!(proposal.capture_range_us(), 42_000_000..44_000_000);
                assert_eq!(proposal.edit_distance(), 1);
                assert_eq!(weak.strong_count(), 1);
                assert_eq!(session.app_state(), AppState::PreparedReady);
                assert_eq!(
                    session.ready().unwrap().translated()[0].as_ref(),
                    "合成字幕測試譯文。"
                );
                assert!(translator.calls.load(Ordering::SeqCst) > 0);
                session.stop();
                assert!(weak.upgrade().is_none());
            }
        }
    });
}

#[test]
fn no_ready_bridge_before_exact_completion_or_after_stop() {
    lane(async {
        let fixture = Fixture::new();
        let translator = MockTranslator::default();
        let mut session = PreparedSession::new();
        assert_eq!(
            session.fuzzy_locator(capture(), 1).err(),
            Some(FuzzyAlignmentError::NotReady)
        );
        let task = session
            .start(fixture.request(
                TextLanguage::Source(SpeechLanguage::English),
                SubtitleFormat::Srt,
                &[LINE],
            ))
            .unwrap();
        assert_eq!(
            session.fuzzy_locator(capture(), 1).err(),
            Some(FuzzyAlignmentError::NotReady)
        );
        let completion = task
            .run_with_translator(&translator, TranslationQuality::Standard, None)
            .await;
        assert_eq!(
            session.fuzzy_locator(capture(), 1).err(),
            Some(FuzzyAlignmentError::NotReady)
        );
        session.apply(completion).unwrap();
        assert!(session.fuzzy_locator(capture(), 1).is_ok());
        session.stop();
        assert_eq!(
            session.fuzzy_locator(capture(), 1).err(),
            Some(FuzzyAlignmentError::NotReady)
        );
    });
}

#[test]
fn chinese_ready_is_not_original_language_alignment_evidence() {
    lane(async {
        for language in [
            TextLanguage::TaiwanTraditional,
            TextLanguage::OtherTraditional,
            TextLanguage::SimplifiedChinese,
        ] {
            let fixture = Fixture::new();
            let translator = MockTranslator::default();
            let session = fixture
                .ready(
                    language,
                    SubtitleFormat::Srt,
                    &["软件与服务器都是合成字幕的测试内容"],
                    &translator,
                )
                .await;
            assert_eq!(
                session.fuzzy_locator(capture(), 1).err(),
                Some(FuzzyAlignmentError::UnsupportedEvidence)
            );
            assert_eq!(translator.calls.load(Ordering::SeqCst), 0);
        }
    });
}

#[test]
fn ready_stop_revokes_delivered_proposal_and_frees_text_and_media_with_locator_alive() {
    lane(async {
        let fixture = Fixture::new();
        let mut session = fixture
            .ready(
                TextLanguage::Source(SpeechLanguage::English),
                SubtitleFormat::Srt,
                &[LINE],
                &MockTranslator::default(),
            )
            .await;
        let locator = locator(&session, Arc::new(Semaphore::new(1)));
        let text = weak(&locator);
        let proposal = locator
            .locate(
                event(SpeechLanguage::English, LINE),
                stamp(),
                ResolverCancellation::default(),
            )
            .await
            .unwrap();
        #[cfg(windows)]
        assert!(!fixture.writable());
        session.stop();
        assert!(text.upgrade().is_none());
        #[cfg(windows)]
        assert!(fixture.writable());
        assert_eq!(
            proposal.validate(locator.scope(), proposal.binding()),
            Err(ResolverError::Cancelled.into())
        );
        assert_eq!(
            locator
                .locate(
                    event(SpeechLanguage::English, LINE),
                    stamp(),
                    ResolverCancellation::default()
                )
                .await
                .err(),
            Some(ResolverError::Cancelled.into())
        );
    });
}

#[test]
fn unpolled_prepared_future_holds_no_text_or_media_after_session_stop() {
    lane(async {
        let fixture = Fixture::new();
        let mut session = fixture
            .ready(
                TextLanguage::Source(SpeechLanguage::English),
                SubtitleFormat::Srt,
                &[LINE],
                &MockTranslator::default(),
            )
            .await;
        let locator = locator(&session, Arc::new(Semaphore::new(1)));
        let text = weak(&locator);
        let external = ResolverCancellation::default();
        let request = locator.locate(
            event(SpeechLanguage::English, LINE),
            stamp(),
            external.clone(),
        );
        assert_eq!(text.strong_count(), 1);
        session.stop();
        assert!(text.upgrade().is_none());
        #[cfg(windows)]
        assert!(fixture.writable());
        assert_eq!(request.await.err(), Some(ResolverError::Cancelled.into()));
        assert!(!external.is_cancelled());
    });
}

#[test]
fn replacing_ready_session_cancels_started_worker_and_releases_media_before_text_cleanup() {
    lane(async {
        let first = Fixture::new();
        let second = Fixture::new();
        let translator = MockTranslator::default();
        let mut session = first
            .ready(
                TextLanguage::Source(SpeechLanguage::English),
                SubtitleFormat::Srt,
                &[LINE],
                &translator,
            )
            .await;
        let cpu = Arc::new(Semaphore::new(1));
        let old_locator = locator(&session, cpu.clone());
        let old_text = weak(&old_locator);
        let old_identity = old_locator.scope().document_identity();
        let (entered, entered_rx, release, release_rx) = gate();
        let request = old_locator.locate_using(
            event(SpeechLanguage::English, LINE),
            stamp(),
            ResolverCancellation::default(),
            gated(entered, release_rx),
        );
        tokio::pin!(request);
        pending(request.as_mut()).await;
        entered_rx.await.unwrap();
        let task = session
            .start(second.request(
                TextLanguage::Source(SpeechLanguage::English),
                SubtitleFormat::Srt,
                &[LINE],
            ))
            .unwrap();
        pending(request.as_mut()).await;
        assert_eq!(cpu.available_permits(), 0);
        assert!(old_text.upgrade().is_some());
        #[cfg(windows)]
        assert!(first.writable());
        release.send(()).unwrap();
        assert_eq!(request.await.err(), Some(ResolverError::Cancelled.into()));
        assert!(old_text.upgrade().is_none());
        session
            .apply(
                task.run_with_translator(&translator, TranslationQuality::Standard, None)
                    .await,
            )
            .unwrap();
        let new_locator = session.fuzzy_locator(capture(), 2).unwrap();
        assert_ne!(new_locator.scope().document_identity(), old_identity);
        assert_eq!(new_locator.scope().epoch(), 2);
        assert!(
            old_locator
                .locate(
                    event(SpeechLanguage::English, LINE),
                    stamp(),
                    ResolverCancellation::default()
                )
                .await
                .is_err()
        );
        assert!(
            new_locator
                .locate(
                    event(SpeechLanguage::English, LINE),
                    stamp(),
                    ResolverCancellation::default()
                )
                .await
                .is_ok()
        );
    });
}

#[test]
fn session_drop_cancels_worker_and_cannot_keep_native_media_pinned() {
    lane(async {
        let fixture = Fixture::new();
        let session = fixture
            .ready(
                TextLanguage::Source(SpeechLanguage::English),
                SubtitleFormat::Srt,
                &[LINE],
                &MockTranslator::default(),
            )
            .await;
        let cpu = Arc::new(Semaphore::new(1));
        let locator = locator(&session, cpu.clone());
        let text = weak(&locator);
        let (entered, entered_rx, release, release_rx) = gate();
        let request = locator.locate_using(
            event(SpeechLanguage::English, LINE),
            stamp(),
            ResolverCancellation::default(),
            gated(entered, release_rx),
        );
        tokio::pin!(request);
        pending(request.as_mut()).await;
        entered_rx.await.unwrap();
        drop(session);
        pending(request.as_mut()).await;
        assert!(text.upgrade().is_some());
        #[cfg(windows)]
        assert!(fixture.writable());
        release.send(()).unwrap();
        assert_eq!(request.await.err(), Some(ResolverError::Cancelled.into()));
        assert!(text.upgrade().is_none());
    });
}

#[test]
fn prepared_future_drop_cancels_job_without_revoking_ready_or_parent() {
    lane(async {
        let fixture = Fixture::new();
        let session = fixture
            .ready(
                TextLanguage::Source(SpeechLanguage::English),
                SubtitleFormat::Srt,
                &[LINE],
                &MockTranslator::default(),
            )
            .await;
        let cpu = Arc::new(Semaphore::new(1));
        let locator = locator(&session, cpu.clone());
        let text = weak(&locator);
        let parent = ResolverCancellation::default();
        let (entered, entered_rx, release, release_rx) = gate();
        let mut request = Box::pin(locator.locate_using(
            event(SpeechLanguage::English, LINE),
            stamp(),
            parent.clone(),
            gated(entered, release_rx),
        ));
        pending(request.as_mut()).await;
        entered_rx.await.unwrap();
        drop(request);
        assert_eq!(cpu.available_permits(), 0);
        assert!(!parent.is_cancelled());
        assert_eq!(session.state(), PreparedState::Ready);
        release.send(()).unwrap();
        reclaim(cpu).await;
        assert_eq!(text.strong_count(), 1);
        assert!(
            locator
                .locate(
                    event(SpeechLanguage::English, LINE),
                    stamp(),
                    ResolverCancellation::default()
                )
                .await
                .is_ok()
        );
    });
}

#[test]
fn stopping_one_locator_preserves_ready_and_another_locator() {
    lane(async {
        let fixture = Fixture::new();
        let session = fixture
            .ready(
                TextLanguage::Source(SpeechLanguage::English),
                SubtitleFormat::Srt,
                &[LINE],
                &MockTranslator::default(),
            )
            .await;
        let cpu = Arc::new(Semaphore::new(1));
        let mut first = locator(&session, cpu.clone());
        let second = locator(&session, cpu);
        let proposal = first
            .locate(
                event(SpeechLanguage::English, LINE),
                stamp(),
                ResolverCancellation::default(),
            )
            .await
            .unwrap();
        first.stop();
        assert_eq!(
            proposal.validate(first.scope(), proposal.binding()),
            Err(ResolverError::Cancelled.into())
        );
        assert_eq!(session.state(), PreparedState::Ready);
        assert!(
            second
                .locate(
                    event(SpeechLanguage::English, LINE),
                    stamp(),
                    ResolverCancellation::default()
                )
                .await
                .is_ok()
        );
        assert_eq!(weak(&second).strong_count(), 1);
    });
}

#[test]
fn prepared_bridge_keeps_full_document_ambiguity_and_capture_evidence_checks() {
    lane(async {
        let fixture = Fixture::new();
        let session = fixture
            .ready(
                TextLanguage::Source(SpeechLanguage::English),
                SubtitleFormat::Srt,
                &[LINE, LINE],
                &MockTranslator::default(),
            )
            .await;
        let locator = locator(&session, Arc::new(Semaphore::new(1)));
        assert_eq!(
            locator
                .locate(
                    event(SpeechLanguage::English, LINE),
                    stamp(),
                    ResolverCancellation::default()
                )
                .await
                .err(),
            Some(FuzzyAlignmentError::NoUniqueMatch)
        );
        let mut partial = event(SpeechLanguage::English, LINE);
        partial.kind = TranscriptKind::Partial;
        assert_eq!(
            locator
                .locate(partial, stamp(), ResolverCancellation::default())
                .await
                .err(),
            Some(FuzzyAlignmentError::UnsupportedEvidence)
        );
        let mut old = stamp();
        old.ended_at -= MAX_WINDOW_AGE + Duration::from_secs(1);
        assert_eq!(
            locator
                .locate(
                    event(SpeechLanguage::English, LINE),
                    old,
                    ResolverCancellation::default()
                )
                .await
                .err(),
            Some(ResolverError::Stale.into())
        );
        let mut wrong = stamp();
        wrong.identity.generation += 1;
        assert_eq!(
            locator
                .locate(
                    event(SpeechLanguage::English, LINE),
                    wrong,
                    ResolverCancellation::default()
                )
                .await
                .err(),
            Some(ResolverError::InvalidData.into())
        );
    });
}

#[test]
fn invalid_capture_scope_does_not_change_prepared_ready() {
    lane(async {
        let fixture = Fixture::new();
        let session = fixture
            .ready(
                TextLanguage::Source(SpeechLanguage::English),
                SubtitleFormat::Srt,
                &[LINE],
                &MockTranslator::default(),
            )
            .await;
        for (identity, epoch) in [
            (
                StreamIdentity {
                    session: SessionId(0),
                    ..capture()
                },
                1,
            ),
            (
                StreamIdentity {
                    generation: 0,
                    ..capture()
                },
                1,
            ),
            (capture(), 0),
        ] {
            assert_eq!(
                session.fuzzy_locator(identity, epoch).err(),
                Some(ResolverError::InvalidData.into())
            );
            assert_eq!(session.state(), PreparedState::Ready);
        }
    });
}

#[test]
fn prepared_request_ancestry_limit_is_explicit_and_never_cancels_ready() {
    lane(async {
        let fixture = Fixture::new();
        let session = fixture
            .ready(
                TextLanguage::Source(SpeechLanguage::English),
                SubtitleFormat::Srt,
                &[LINE],
                &MockTranslator::default(),
            )
            .await;
        let cpu = Arc::new(Semaphore::new(1));
        let locator = locator(&session, cpu.clone());
        let root = ResolverCancellation::default();
        let parent = ResolverCancellation::linked(root.clone()).unwrap();
        let proposal = locator
            .locate(event(SpeechLanguage::English, LINE), stamp(), parent)
            .await
            .unwrap();
        root.cancel();
        assert_eq!(
            proposal.validate(locator.scope(), proposal.binding()),
            Err(ResolverError::Cancelled.into())
        );
        assert_eq!(session.state(), PreparedState::Ready);
        let root = ResolverCancellation::default();
        let deep =
            ResolverCancellation::linked(ResolverCancellation::linked(root.clone()).unwrap())
                .unwrap();
        assert_eq!(
            locator
                .locate(event(SpeechLanguage::English, LINE), stamp(), deep)
                .await
                .err(),
            Some(ResolverError::InvalidData.into())
        );
        assert!(!root.is_cancelled());
        assert_eq!(cpu.available_permits(), 1);
        assert_eq!(session.state(), PreparedState::Ready);
    });
}

#[test]
fn failed_preparation_has_no_alignment_lease_or_fake_asr_fallback() {
    lane(async {
        let fixture = Fixture::new();
        let request = fixture.request(
            TextLanguage::Source(SpeechLanguage::English),
            SubtitleFormat::Srt,
            &[LINE],
        );
        fs::write(
            fixture.directory.path().join("explicit-subtitle.data"),
            "invalid synthetic subtitle",
        )
        .unwrap();
        let translator = MockTranslator::default();
        let mut session = PreparedSession::new();
        let completion = session
            .start(request)
            .unwrap()
            .run_with_translator(&translator, TranslationQuality::Standard, None)
            .await;
        assert!(session.apply(completion).is_err());
        assert!(matches!(session.state(), PreparedState::Failed(_)));
        assert_eq!(
            session.fuzzy_locator(capture(), 1).err(),
            Some(FuzzyAlignmentError::NotReady)
        );
        assert_eq!(translator.calls.load(Ordering::SeqCst), 0);
    });
}
