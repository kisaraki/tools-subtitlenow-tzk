use super::*;
use crate::{
    domain::TranslationQuality,
    resolver::contracts::SubtitleSource,
    session::aligned_playback::tests::{Clock, MockTranslator, attachment_fixture, capture, event},
};
use std::{future::Future, time::Duration};
fn lane<F: Future>(future: F) -> F::Output {
    let _lane = crate::media::work::TEST_OWNERS.lock().unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}
async fn deliver(
    owner: &mut PreparedLiveAlignment,
    clock: &mut Clock,
    flow: &mut LiveTranslation<'_>,
    index: u64,
) {
    let event = event(index, index + 1);
    clock.advance(event.start_us.unwrap() + event.duration_us.unwrap());
    flow.ingest(event.clone()).unwrap();
    owner.offer(event, &clock.inner).await.unwrap();
    // Production UI ticks can consume completion before select(changed). This
    // must still verify and adopt exactly once, not lose the completed result.
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            owner.reconcile(&clock.inner, Some(flow)).await.unwrap();
            if index < 2 {
                if matches!(owner.state(), AlignedPlaybackState::Collecting(count) if count >= (index+1) as usize) {break;}
            } else if matches!(owner.state(), AlignedPlaybackState::Positioned {..}) {break;}
            tokio::task::yield_now().await;
        }
    }).await.unwrap();
}
#[test]
fn alignment_live_production_poll_path_adopts_even_when_ui_ticks_consume_completion_first() {
    let (_directory, _attachment, source) = attachment_fixture(true);
    lane(async {
        let mut owner = PreparedLiveAlignment::new(
            source,
            capture(),
            AlignmentCommand {
                running: true,
                reset: 0,
            },
        )
        .await
        .unwrap();
        let mut flow =
            LiveTranslation::new(capture(), TranslationQuality::Standard, &MockTranslator).unwrap();
        let mut clock = Clock::new();
        for index in 0..3 {
            deliver(&mut owner, &mut clock, &mut flow, index).await;
        }
        assert_eq!(
            flow.timeline().items().last().unwrap().display_origin(),
            SubtitleSource::PreparedTranslation
        );
        assert!(
            !owner
                .reconcile(&clock.inner, Some(&mut flow))
                .await
                .unwrap()
        );
        owner
            .command(AlignmentCommand {
                running: false,
                reset: 1,
            })
            .await
            .unwrap();
        owner
            .reconcile(&clock.inner, Some(&mut flow))
            .await
            .unwrap();
        assert_eq!(owner.state(), AlignedPlaybackState::Paused);
        assert_eq!(
            flow.timeline().items().last().unwrap().display_origin(),
            SubtitleSource::LiveAi
        );
        owner
            .command(AlignmentCommand {
                running: true,
                reset: 1,
            })
            .await
            .unwrap();
        assert!(matches!(owner.state(), AlignedPlaybackState::Collecting(0)));
        owner.stop().await.unwrap();
    });
}
#[test]
fn alignment_live_source_clear_and_gap_keep_live_ai_available_and_retire_ready_work() {
    let (_directory, attachment, source) = attachment_fixture(false);
    lane(async {
        let mut owner = PreparedLiveAlignment::new(
            source,
            capture(),
            AlignmentCommand {
                running: true,
                reset: 0,
            },
        )
        .await
        .unwrap();
        let mut flow =
            LiveTranslation::new(capture(), TranslationQuality::Standard, &MockTranslator).unwrap();
        let mut clock = Clock::new();
        for index in 0..3 {
            deliver(&mut owner, &mut clock, &mut flow, index).await;
        }
        drop(attachment);
        owner.audio_gap().await.unwrap();
        owner
            .reconcile(&clock.inner, Some(&mut flow))
            .await
            .unwrap();
        assert_eq!(owner.state(), AlignedPlaybackState::Stopped);
        assert_eq!(
            flow.timeline().items().last().unwrap().display_origin(),
            SubtitleSource::LiveAi
        );
        owner
            .command(AlignmentCommand {
                running: true,
                reset: 2,
            })
            .await
            .unwrap();
        assert_eq!(owner.state(), AlignedPlaybackState::Stopped);
        owner.stop().await.unwrap();
    });
}
#[test]
fn alignment_live_missing_actual_capture_is_visible_and_never_creates_a_fake_window() {
    let (_directory, _attachment, source) = attachment_fixture(false);
    lane(async {
        let mut owner = PreparedLiveAlignment::new(
            source,
            capture(),
            AlignmentCommand {
                running: true,
                reset: 0,
            },
        )
        .await
        .unwrap();
        let clock = LivePlaybackClock::new(capture());
        owner.offer(event(0, 1), &clock).await.unwrap();
        assert!(matches!(owner.state(), AlignedPlaybackState::Failed(_)));
        assert!(owner.engine.verified().is_none());
        owner.reconcile(&clock, None).await.unwrap();
        assert!(matches!(owner.state(), AlignedPlaybackState::Failed(_)));
        let mut clock = Clock::new();
        clock.advance(43_000_000);
        let mut invalid = event(0, 1);
        invalid.start_us = Some(80_000_000);
        owner.offer(invalid, &clock.inner).await.unwrap();
        owner.reconcile(&clock.inner, None).await.unwrap();
        assert!(matches!(owner.state(), AlignedPlaybackState::Failed(_)));
        owner.offer(event(0, 1), &clock.inner).await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), owner.changed())
            .await
            .unwrap()
            .unwrap();
        owner.reconcile(&clock.inner, None).await.unwrap();
        assert_eq!(owner.state(), AlignedPlaybackState::Collecting(1));
        owner.stop().await.unwrap();
    });
}
