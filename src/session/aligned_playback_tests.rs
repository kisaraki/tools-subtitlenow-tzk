//! Synthetic SRT/opaque media, real Ready/worker/arbiter/timeline, explicit AI mock.
use super::*;
use crate::{
    ai::{
        speech::{SpeechLanguage, TranscriptKind},
        translation::{
            TranslationCancellation, TranslationCapabilities, TranslationEmitter,
            TranslationFuture, TranslationRequest, Translator,
        },
    },
    audio::{
        capture::{AudioStamp, BLOCK_FRAMES, SessionId},
        preprocess::NormalizedAudio,
    },
    domain::TranslationQuality,
    media::{
        input::{LocalSubtitleInput, MediaInput},
        planning::PreparedRequest,
    },
    resolver::{
        contracts::{SubtitleSource, TextLanguage},
        timed_subtitle::SubtitleFormat,
    },
    session::{
        live_translation::LiveTranslation, playback_clock::LivePlaybackClock,
        prepared::PreparedSession,
    },
};
use futures_util::FutureExt;
use std::{
    fs,
    future::Future,
    time::{Duration, Instant},
};
fn lane<F: Future>(future: F) -> F::Output {
    let _lane = crate::media::work::TEST_OWNERS.lock().unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}
pub(crate) struct MockTranslator;
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
        Box::pin(async { Ok("臺灣合成測試譯文。".into()) })
    }
}
struct Fixture {
    // Release the Ready media pin before TempDir removes synthetic files.
    ready: PreparedSession,
    source: AlignmentSource,
    dir: tempfile::TempDir,
}
pub(crate) fn attachment_fixture(
    paired: bool,
) -> (
    tempfile::TempDir,
    crate::session::prepared::PreparedAttachment,
    AlignmentSource,
) {
    lane(async {
        let Fixture { dir, ready, source } =
            Fixture::new(paired, TranslationQuality::Standard, 0).await;
        (dir, ready.into_attachment(), source)
    })
}
fn time(us: u64) -> String {
    format!(
        "{:02}:{:02}:{:02},{:03}",
        us / 3_600_000_000,
        us / 60_000_000 % 60,
        us / 1_000_000 % 60,
        us / 1000 % 1000
    )
}
impl Fixture {
    async fn new(paired: bool, quality: TranslationQuality, drift: u64) -> Self {
        Self::with_rate(paired, quality, i64::try_from(drift).unwrap()).await
    }
    async fn with_rate(paired: bool, quality: TranslationQuality, step: i64) -> Self {
        Self::with_timing(paired, quality, step, 1_000_000).await
    }
    async fn with_timing(
        paired: bool,
        quality: TranslationQuality,
        step: i64,
        capture_duration: u64,
    ) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let media = dir.path().join("synthetic-opaque.media");
        fs::write(&media, b"synthetic opaque media; never decoded or played").unwrap();
        let primary = dir.path().join("primary.srt");
        let reference = dir.path().join("reference.srt");
        let mut body = String::new();
        let mut refs = String::new();
        for index in 0..16 {
            let start = u64::try_from(
                600_000_000i128
                    + i128::from(index) * (i128::from(capture_duration) + i128::from(step)),
            )
            .unwrap();
            let duration = u64::try_from(i128::from(capture_duration) + i128::from(step)).unwrap();
            let timing = format!("{} --> {}", time(start), time(start + duration));
            let text = char::from(b'a' + index as u8).to_string().repeat(32);
            refs.push_str(&format!("{}\n{timing}\n{text}\n\n", index + 1));
            body.push_str(&format!(
                "{}\n{timing}\n{}\n\n",
                index + 1,
                if paired {
                    format!("第{index}段合成中文字幕，影片只在本次記憶體處理。")
                } else {
                    text
                }
            ));
        }
        fs::write(&primary, body).unwrap();
        fs::write(&reference, refs).unwrap();
        let language = if paired {
            TextLanguage::TaiwanTraditional
        } else {
            TextLanguage::Source(SpeechLanguage::English)
        };
        let mut request = PreparedRequest::new(
            MediaInput::local(media).unwrap(),
            vec![LocalSubtitleInput::new(primary, SubtitleFormat::Srt, language).unwrap()],
        )
        .unwrap();
        if paired {
            request = request
                .with_source_reference(
                    LocalSubtitleInput::new(
                        reference,
                        SubtitleFormat::Srt,
                        TextLanguage::Source(SpeechLanguage::English),
                    )
                    .unwrap(),
                    "explicit-synthetic-edition",
                )
                .unwrap();
        }
        let mut ready = PreparedSession::new();
        let task = ready.start(request).unwrap();
        let completion = if paired {
            task.run().await
        } else {
            task.run_with_translator(&MockTranslator, quality, None)
                .await
        };
        assert!(ready.apply(completion).unwrap());
        let source = ready.alignment_source().unwrap();
        Self { dir, ready, source }
    }
    fn writable(&self) -> bool {
        fs::OpenOptions::new()
            .write(true)
            .open(self.dir.path().join("synthetic-opaque.media"))
            .is_ok()
    }
}
pub(crate) fn capture() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(88_001),
        generation: 3,
    }
}
pub(crate) struct Clock {
    pub(crate) inner: LivePlaybackClock,
    end: u64,
    sequence: u64,
}
impl Clock {
    pub(crate) fn new() -> Self {
        Self {
            inner: LivePlaybackClock::new(capture()),
            end: 40_000_000,
            sequence: 0,
        }
    }
    pub(crate) fn advance(&mut self, until: u64) {
        while self.end < until {
            self.sequence += 1;
            self.inner.observe(&NormalizedAudio {
                stamp: AudioStamp {
                    session: capture().session,
                    sequence: self.sequence,
                    frame_position: Some(self.end * 48_000 / 1_000_000),
                    qpc_100ns: None,
                    received: Instant::now(),
                    discontinuity: false,
                },
                frames: BLOCK_FRAMES,
                samples: [0.0; BLOCK_FRAMES],
            });
            self.end += 20_000;
        }
    }
}
pub(crate) fn event(index: u64, utterance: u64) -> TranscriptEvent {
    TranscriptEvent {
        identity: capture(),
        utterance,
        sequence: utterance,
        revision: 1,
        start_us: Some(42_000_000 + (utterance - 1) * 1_000_000),
        duration_us: Some(1_000_000),
        language: Some(SpeechLanguage::English),
        kind: TranscriptKind::Final,
        text: char::from(b'a' + index as u8).to_string().repeat(32).into(),
    }
}
async fn send(
    owner: &mut AlignedPlayback,
    clock: &mut Clock,
    index: u64,
    utterance: u64,
) -> TranscriptEvent {
    let old_scope = owner.control.scope();
    let event = event(index, utterance);
    clock.advance(event.start_us.unwrap() + event.duration_us.unwrap());
    owner
        .offer(
            event.clone(),
            clock.inner.cursor().unwrap().window().unwrap(),
        )
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), owner.changed())
        .await
        .unwrap()
        .unwrap();
    owner.reconcile(clock.inner.cursor()).await.unwrap();
    // A detected jump retires the old epoch and replays the first observation
    // through the new real worker; wait for this bounded replacement as well.
    if matches!(owner.state(), AlignedPlaybackState::Reacquiring(_))
        && owner.control.scope() != old_scope
    {
        tokio::time::timeout(Duration::from_secs(2), owner.changed())
            .await
            .unwrap()
            .unwrap();
        owner.reconcile(clock.inner.cursor()).await.unwrap();
    }
    event
}
async fn seed(
    owner: &mut AlignedPlayback,
    clock: &mut Clock,
    first_cue: u64,
    first_utterance: u64,
) {
    for index in 0..3 {
        send(owner, clock, first_cue + index, first_utterance + index).await;
    }
    assert!(owner.verified().is_some(), "{:?}", owner.state());
}
#[test]
fn alignment_playback_long_utterances_keep_bounded_history_but_expire_current_position() {
    lane(async {
        let fixture =
            Fixture::with_timing(true, TranslationQuality::Standard, 160_000, 4_000_000).await;
        let mut owner = AlignedPlayback::new(&fixture.source, capture()).unwrap();
        let mut clock = Clock::new();
        owner.resume().await.unwrap();
        let scope = owner.control.scope();
        for index in 0..3 {
            if index == 1 {
                tokio::time::sleep(Duration::from_millis(5_100)).await;
                owner.reconcile(None).await.unwrap();
                assert_eq!(owner.state(), AlignedPlaybackState::Collecting(1));
                assert_eq!(owner.control.scope(), scope);
                assert!(
                    tokio::time::timeout(Duration::from_millis(20), owner.changed())
                        .await
                        .is_err()
                );
            }
            let mut input = event(index, index + 1);
            input.start_us = Some(42_000_000 + index * 4_000_000);
            input.duration_us = Some(4_000_000);
            clock.advance(input.start_us.unwrap() + 4_000_000);
            owner
                .offer(input, clock.inner.cursor().unwrap().window().unwrap())
                .unwrap();
            tokio::time::timeout(Duration::from_secs(2), owner.changed())
                .await
                .unwrap()
                .unwrap();
            owner.reconcile(clock.inner.cursor()).await.unwrap();
        }
        let verified = owner.verified().unwrap();
        assert_eq!(verified.position().unwrap().media_us, 612_480_000);
        assert_eq!(verified.rate_ppm().unwrap(), 1_040_000);
        assert_eq!(owner.control.scope(), scope);
        owner.stop().await.unwrap();
        assert!(verified.position().is_err());
    });
}
#[test]
fn alignment_playback_real_ready_worker_and_arbiter_feed_the_single_timeline_for_source_and_chinese()
 {
    lane(async {
        for paired in [false, true] {
            let fixture = Fixture::new(paired, TranslationQuality::Standard, 0).await;
            let mut owner = AlignedPlayback::new(&fixture.source, capture()).unwrap();
            let mut flow =
                LiveTranslation::new(capture(), TranslationQuality::Standard, &MockTranslator)
                    .unwrap();
            let mut clock = Clock::new();
            owner.resume().await.unwrap();
            for index in 0..3 {
                let event = send(&mut owner, &mut clock, index, index + 1).await;
                flow.ingest(event).unwrap();
                if index < 2 {
                    assert!(owner.take_verified().is_none());
                }
            }
            let verified = owner.take_verified().unwrap();
            assert_eq!(verified.offset_us().unwrap(), 558_000_000);
            assert!(
                flow.adopt_prepared(&fixture.source, verified.clone())
                    .unwrap()
            );
            let last = flow.timeline().items().last().unwrap();
            assert_eq!(last.display_origin(), SubtitleSource::PreparedTranslation);
            assert!(last.display_translation().contains(if paired {
                "合成中文字幕"
            } else {
                "合成測試譯文"
            }));
            assert_eq!(last.display_original(), "c".repeat(32));
            assert!(owner.take_verified().is_none());
            owner.pause().await.unwrap();
            assert!(verified.position().is_err());
            assert!(flow.reconcile_sources().is_some());
            assert_eq!(
                flow.timeline().items().last().unwrap().display_origin(),
                SubtitleSource::LiveAi
            );
            owner.stop().await.unwrap();
        }
    });
}
#[test]
fn alignment_playback_forward_backward_chapter_and_restart_withdraw_before_recalibration() {
    lane(async {
        for (initial, jump) in [(0, 8), (8, 1), (0, 0)] {
            let fixture = Fixture::new(false, TranslationQuality::Standard, 0).await;
            let mut owner = AlignedPlayback::new(&fixture.source, capture()).unwrap();
            let mut clock = Clock::new();
            owner.resume().await.unwrap();
            seed(&mut owner, &mut clock, initial, 1).await;
            let previous = owner.verified().unwrap();
            send(&mut owner, &mut clock, jump, 4).await;
            assert!(previous.position().is_err());
            assert!(owner.verified().is_none());
            send(&mut owner, &mut clock, jump + 1, 5).await;
            send(&mut owner, &mut clock, jump + 2, 6).await;
            let current = owner.verified().unwrap();
            assert_ne!(current.scope().epoch(), previous.scope().epoch());
            assert_eq!(
                current.position().unwrap().media_us,
                603_000_000 + jump * 1_000_000
            );
            owner.stop().await.unwrap();
        }
    });
}
#[test]
fn alignment_playback_small_native_drift_recalibrates_from_new_speech_without_wall_time() {
    lane(async {
        let fixture = Fixture::new(false, TranslationQuality::Standard, 20_000).await;
        let mut owner = AlignedPlayback::new(&fixture.source, capture()).unwrap();
        let mut clock = Clock::new();
        owner.resume().await.unwrap();
        seed(&mut owner, &mut clock, 0, 1).await;
        let previous = owner.verified().unwrap();
        send(&mut owner, &mut clock, 3, 4).await;
        let current = owner.verified().unwrap();
        assert_eq!(
            current.change(),
            CalibrationChange::DriftCorrected { delta_us: 20_000 }
        );
        assert_eq!(current.offset_us().unwrap(), 558_080_000);
        assert!(previous.position().is_err());
        owner.stop().await.unwrap();
    });
}
#[test]
fn alignment_playback_advertisement_withdraws_then_reacquires_the_same_document() {
    lane(async {
        let fixture = Fixture::new(false, TranslationQuality::Standard, 0).await;
        let mut owner = AlignedPlayback::new(&fixture.source, capture()).unwrap();
        let mut clock = Clock::new();
        owner.resume().await.unwrap();
        seed(&mut owner, &mut clock, 0, 1).await;
        let previous = owner.verified().unwrap();
        let mut ad = event(0, 4);
        ad.text = "this unrelated synthetic advertisement is not in the subtitle document".into();
        clock.advance(46_000_000);
        owner
            .offer(ad, clock.inner.cursor().unwrap().window().unwrap())
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), owner.changed())
            .await
            .unwrap()
            .unwrap();
        owner.reconcile(clock.inner.cursor()).await.unwrap();
        assert_eq!(owner.state(), AlignedPlaybackState::Unmatched);
        assert!(previous.position().is_err());
        seed(&mut owner, &mut clock, 4, 5).await;
        assert_ne!(
            owner.verified().unwrap().scope().epoch(),
            previous.scope().epoch()
        );
        owner.stop().await.unwrap();
    });
}
#[test]
fn alignment_playback_uniform_positive_and_negative_rate_drift_is_not_a_seek() {
    lane(async {
        for step in [-40_000i64, 40_000] {
            let fixture = Fixture::with_rate(false, TranslationQuality::Standard, step).await;
            let mut owner = AlignedPlayback::new(&fixture.source, capture()).unwrap();
            let mut clock = Clock::new();
            owner.resume().await.unwrap();
            seed(&mut owner, &mut clock, 0, 1).await;
            let scope = owner.verified().unwrap().scope();
            assert_eq!(
                owner.verified().unwrap().rate_ppm().unwrap(),
                u32::try_from(1_000_000 + step).unwrap()
            );
            for index in 3..9 {
                send(&mut owner, &mut clock, index, index + 1).await;
                assert!(!matches!(
                    owner.state(),
                    AlignedPlaybackState::Reacquiring(
                        RealignmentReason::SeekForward | RealignmentReason::SeekBackward
                    )
                ));
            }
            let verified = owner.verified().unwrap();
            assert_eq!(verified.scope(), scope);
            assert_eq!(
                verified.rate_ppm().unwrap(),
                u32::try_from(1_000_000 + step).unwrap()
            );
            assert!(matches!(
                verified.change(),
                CalibrationChange::DriftCorrected { .. }
            ));
            owner.stop().await.unwrap();
        }
    });
}
#[test]
fn alignment_playback_ready_withdrawal_releases_pin_and_revokes_timeline_and_position() {
    lane(async {
        let mut fixture = Fixture::new(true, TranslationQuality::Standard, 0).await;
        let mut owner = AlignedPlayback::new(&fixture.source, capture()).unwrap();
        let mut clock = Clock::new();
        owner.resume().await.unwrap();
        seed(&mut owner, &mut clock, 0, 1).await;
        let verified = owner.verified().unwrap();
        fixture.ready.stop();
        assert!(verified.position().is_err());
        #[cfg(windows)]
        assert!(fixture.writable());
        tokio::time::timeout(Duration::from_secs(2), owner.changed())
            .await
            .unwrap()
            .unwrap();
        owner.reconcile(clock.inner.cursor()).await.unwrap();
        assert_eq!(owner.state(), AlignedPlaybackState::Stopped);
        owner.stop().await.unwrap();
    });
}
#[test]
fn alignment_playback_cancelled_pause_and_resume_do_not_reuse_the_old_epoch() {
    lane(async {
        let fixture = Fixture::new(false, TranslationQuality::Standard, 0).await;
        let mut owner = AlignedPlayback::new(&fixture.source, capture()).unwrap();
        owner.resume().await.unwrap();
        let initial = owner.control.scope().unwrap();
        assert!(owner.pause().now_or_never().is_none());
        assert_eq!(owner.state(), AlignedPlaybackState::Paused);
        owner.resume().await.unwrap();
        assert_ne!(owner.control.scope().unwrap().epoch(), initial.epoch());
        let mut clock = Clock::new();
        seed(&mut owner, &mut clock, 0, 1).await;
        owner.stop().await.unwrap();
    });
}
#[test]
fn alignment_playback_quality_mismatch_is_visible_and_never_degrades_the_required_strategy() {
    lane(async {
        let fixture = Fixture::new(false, TranslationQuality::Fast, 0).await;
        assert_eq!(
            fixture.ready.ready().unwrap().quality(),
            Some(TranslationQuality::Fast)
        );
        let mut owner = AlignedPlayback::new(&fixture.source, capture()).unwrap();
        let mut flow =
            LiveTranslation::new(capture(), TranslationQuality::High, &MockTranslator).unwrap();
        let mut clock = Clock::new();
        owner.resume().await.unwrap();
        for index in 0..3 {
            let event = send(&mut owner, &mut clock, index, index + 1).await;
            flow.ingest(event).unwrap();
        }
        let error = flow
            .adopt_prepared(&fixture.source, owner.take_verified().unwrap())
            .unwrap_err();
        owner.reject_adoption(error);
        assert!(matches!(
            owner.state(),
            AlignedPlaybackState::AdoptionFailed(_)
        ));
        assert_eq!(
            flow.timeline().items().last().unwrap().display_origin(),
            SubtitleSource::LiveAi
        );
        owner.reconcile(clock.inner.cursor()).await.unwrap();
        assert!(matches!(
            owner.state(),
            AlignedPlaybackState::AdoptionFailed(_)
        ));
        owner.stop().await.unwrap();
    });
}
