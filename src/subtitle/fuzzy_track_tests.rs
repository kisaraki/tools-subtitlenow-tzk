//! Synthetic independent cues, admitted through the real lexical matcher.
use super::*;
use crate::{
    ai::speech::{SpeechLanguage, StreamIdentity, TranscriptEvent, TranscriptKind},
    audio::capture::SessionId,
    resolver::contracts::{
        ExistingText, MAX_WINDOW_AGE, SubtitleSource, TextCue, TextLanguage, WindowStamp,
    },
    subtitle::{fuzzy_alignment::locate_fuzzy_cue, fuzzy_worker::FuzzyCueLocator},
};
use std::time::Duration;

pub(crate) fn scope() -> AlignmentScope {
    AlignmentScope::new(
        StreamIdentity {
            session: SessionId(901),
            generation: 1,
        },
        StreamIdentity {
            session: SessionId(902),
            generation: 1,
        },
        1,
    )
    .unwrap()
}
pub(crate) struct Fixture {
    pub media: u64,
    pub capture: u64,
    pub step: u64,
    pub duration: u64,
}
impl Default for Fixture {
    fn default() -> Self {
        Self {
            media: 600_000_000,
            capture: 42_000_000,
            step: 2_000_000,
            duration: 1_000_000,
        }
    }
}
impl Fixture {
    pub(crate) fn document(&self) -> ExistingText {
        ExistingText {
            identity: scope().document_identity(),
            source: SubtitleSource::ExistingSubtitle,
            content_id: "synthetic-track-document".into(),
            language: TextLanguage::Source(SpeechLanguage::English),
            cues: (0..16)
                .map(|index| TextCue {
                    start_us: Some(self.media + index * self.step),
                    duration_us: Some(self.duration),
                    text: char::from(b'a' + index as u8).to_string().repeat(32).into(),
                })
                .collect(),
        }
    }
    pub(crate) fn event(&self, index: usize) -> TranscriptEvent {
        TranscriptEvent {
            identity: scope().capture_identity(),
            utterance: index as u64 + 1,
            sequence: index as u64 + 1,
            revision: 1,
            kind: TranscriptKind::Final,
            language: Some(SpeechLanguage::English),
            start_us: Some(self.capture + index as u64 * self.step),
            duration_us: Some(self.duration),
            text: char::from(b'a' + index as u8).to_string().repeat(32).into(),
        }
    }
    pub(crate) fn stamp(&self, index: usize) -> WindowStamp {
        WindowStamp {
            identity: scope().capture_identity(),
            sequence: index as u64 + 100,
            ended_at: Instant::now(),
            start_us: Some((self.capture + index as u64 * self.step).saturating_sub(5_000_000)),
            duration_us: 12_000_000,
        }
    }
    fn matched(
        &self,
        index: usize,
        event: &TranscriptEvent,
        cancel: &ResolverCancellation,
    ) -> FuzzyCueProposal {
        locate_fuzzy_cue(scope(), &self.document(), event, self.stamp(index), cancel).unwrap()
    }
    fn proposal(&self, index: usize) -> FuzzyCueProposal {
        self.matched(index, &self.event(index), &ResolverCancellation::default())
    }
    pub(crate) fn locator(&self) -> FuzzyCueLocator {
        FuzzyCueLocator::new(scope(), self.document()).unwrap()
    }
}
fn track() -> FuzzyTrack {
    FuzzyTrack::new(scope(), ResolverCancellation::default())
}
fn admit(track: &mut FuzzyTrack, fixture: &Fixture, index: usize) {
    assert_eq!(
        track.admit(&fixture.proposal(index), Instant::now()),
        Ok(true)
    );
}
fn consistent() -> (Fixture, FuzzyTrack) {
    let fixture = Fixture::default();
    let mut track = track();
    for index in 0..3 {
        admit(&mut track, &fixture, index);
    }
    (fixture, track)
}

#[test]
fn adjacent_pair_tracks_store_complete_disjoint_cue_ranges() {
    let fixture = Fixture {
        step: 1_000_000,
        ..Fixture::default()
    };
    let mut track = track();
    for index in [0, 2, 4] {
        let mut event = fixture.event(index);
        event.duration_us = Some(2_000_000);
        event.text = format!("{} {}", event.text, fixture.event(index + 1).text).into();
        let proposal = fixture.matched(index, &event, &ResolverCancellation::default());
        assert_eq!(proposal.cue_range(), index..index + 2);
        track.admit(&proposal, Instant::now()).unwrap();
    }
    assert_eq!(
        track
            .points
            .iter()
            .map(|point| point.cues.clone())
            .collect::<Vec<_>>(),
        vec![0..2, 2..4, 4..6]
    );
    assert_eq!(
        track.status(Instant::now()),
        FuzzyTrackStatus::Consistent(3)
    );
    assert_eq!(
        track
            .proposal(Instant::now())
            .unwrap()
            .observed_media_span_us(),
        fixture.media..fixture.media + 6_000_000
    );
}

#[test]
fn overlapping_adjacent_pairs_cannot_be_independent_track_support() {
    let fixture = Fixture {
        step: 1_000_000,
        ..Fixture::default()
    };
    let mut track = track();
    for index in 0..3 {
        let mut event = fixture.event(index);
        event.duration_us = Some(2_000_000);
        event.text = format!("{} {}", event.text, fixture.event(index + 1).text).into();
        let proposal = fixture.matched(index, &event, &ResolverCancellation::default());
        track.admit(&proposal, Instant::now()).unwrap();
        assert_eq!(track.points.len(), 1);
    }
    assert_eq!(track.status(Instant::now()), FuzzyTrackStatus::Restarted(1));
    assert!(track.proposal(Instant::now()).is_none());
}

#[test]
fn three_distinct_nonoverlapping_cues_produce_only_revocable_observed_spans() {
    let fixture = Fixture::default();
    let mut track = track();
    for index in 0..2 {
        admit(&mut track, &fixture, index);
        assert!(track.proposal(Instant::now()).is_none());
    }
    admit(&mut track, &fixture, 2);
    assert_eq!(
        track.status(Instant::now()),
        FuzzyTrackStatus::Consistent(3)
    );
    let proposal = track.proposal(Instant::now()).unwrap();
    assert_eq!(proposal.scope(), scope());
    assert_eq!(proposal.origin(), FuzzyTextOrigin::ExistingText);
    assert_eq!(proposal.observations(), 3);
    assert_eq!(proposal.endpoint_spread_us(), 0);
    assert_eq!(proposal.observed_capture_span_us(), 42_000_000..47_000_000);
    assert_eq!(proposal.observed_media_span_us(), 600_000_000..605_000_000);
    proposal.validate(scope()).unwrap();
}

#[test]
fn duplicate_new_window_does_not_refresh_observation_or_delivered_track() {
    let (fixture, mut track) = consistent();
    let proposal = track.proposal(Instant::now()).unwrap();
    let expiry = track.next_expiry();
    assert_eq!(track.admit(&fixture.proposal(2), Instant::now()), Ok(false));
    assert_eq!(track.next_expiry(), expiry);
    assert_eq!(track.points.len(), 3);
    proposal.validate(scope()).unwrap();
}

#[test]
fn replacement_revision_does_not_count_twice_and_revokes_old_snapshot() {
    let (fixture, mut track) = consistent();
    let old = track.proposal(Instant::now()).unwrap();
    let mut event = fixture.event(2);
    event.revision = 2;
    assert_eq!(
        track.admit(
            &fixture.matched(2, &event, &ResolverCancellation::default()),
            Instant::now()
        ),
        Ok(true)
    );
    assert_eq!(
        track.status(Instant::now()),
        FuzzyTrackStatus::Consistent(3)
    );
    assert_eq!(old.validate(scope()), Err(FuzzyAlignmentError::Obsolete));
}

#[test]
fn endpoint_spread_exact_limit_is_allowed_one_microsecond_more_restarts() {
    for extra in [0, 1] {
        let fixture = Fixture::default();
        let mut track = track();
        for index in 0..3 {
            let mut event = fixture.event(index);
            if index == 2 {
                event.start_us = event
                    .start_us
                    .map(|start| start + MAX_TRACK_ENDPOINT_SPREAD_US + extra);
            }
            track
                .admit(
                    &fixture.matched(index, &event, &ResolverCancellation::default()),
                    Instant::now(),
                )
                .unwrap();
        }
        if extra == 0 {
            assert_eq!(
                track.proposal(Instant::now()).unwrap().endpoint_spread_us(),
                MAX_TRACK_ENDPOINT_SPREAD_US
            );
        } else {
            assert_eq!(track.status(Instant::now()), FuzzyTrackStatus::Restarted(1));
            assert!(track.proposal(Instant::now()).is_none());
        }
    }
}

#[test]
fn incompatible_start_end_offsets_reject_even_an_exact_lexical_match() {
    let (fixture, mut track) = consistent();
    let old = track.proposal(Instant::now()).unwrap();
    let mut event = fixture.event(3);
    event.duration_us = Some(2_000_000);
    assert_eq!(
        track.admit(
            &fixture.matched(3, &event, &ResolverCancellation::default()),
            Instant::now()
        ),
        Err(FuzzyAlignmentError::TimingMismatch)
    );
    assert_eq!(
        track.status(Instant::now()),
        FuzzyTrackStatus::Rejected(FuzzyAlignmentError::TimingMismatch)
    );
    assert_eq!(old.validate(scope()), Err(FuzzyAlignmentError::Obsolete));
}

#[test]
fn cue_repeat_backwards_and_overlapping_capture_restart_without_seek_claims() {
    for variant in 0..3 {
        let (fixture, mut track) = consistent();
        let old = track.proposal(Instant::now()).unwrap();
        let index = if variant == 0 {
            2
        } else if variant == 1 {
            1
        } else {
            3
        };
        let mut event = fixture.event(index);
        event.sequence = 9;
        event.utterance = 9;
        if variant == 2 {
            event.start_us = Some(fixture.capture + fixture.step * 2 + 500_000);
        }
        track
            .admit(
                &fixture.matched(index, &event, &ResolverCancellation::default()),
                Instant::now(),
            )
            .unwrap();
        assert_eq!(track.status(Instant::now()), FuzzyTrackStatus::Restarted(1));
        assert_eq!(old.validate(scope()), Err(FuzzyAlignmentError::Obsolete));
    }
}

#[test]
fn tiny_disjoint_observations_require_one_second_of_observed_coverage() {
    let fixture = Fixture {
        step: 200_000,
        duration: 100_000,
        ..Fixture::default()
    };
    let mut track = track();
    for index in 0..5 {
        admit(&mut track, &fixture, index);
        assert!(track.proposal(Instant::now()).is_none());
    }
    admit(&mut track, &fixture, 5);
    assert_eq!(
        track.status(Instant::now()),
        FuzzyTrackStatus::Consistent(6)
    );
    for span in [MIN_TRACK_CAPTURE_SPAN_US - 1, MIN_TRACK_CAPTURE_SPAN_US] {
        let fixture = Fixture {
            step: 400_000,
            duration: span - 800_000,
            ..Fixture::default()
        };
        let mut track = super::FuzzyTrack::new(scope(), ResolverCancellation::default());
        for index in 0..3 {
            admit(&mut track, &fixture, index);
        }
        assert_eq!(
            track.proposal(Instant::now()).is_some(),
            span == MIN_TRACK_CAPTURE_SPAN_US
        );
    }
}

#[test]
fn gradual_pairwise_small_changes_cannot_hide_whole_track_drift() {
    let fixture = Fixture::default();
    let mut track = track();
    for index in 0..4 {
        let mut event = fixture.event(index);
        event.start_us = event.start_us.map(|start| start + index as u64 * 100_000);
        track
            .admit(
                &fixture.matched(index, &event, &ResolverCancellation::default()),
                Instant::now(),
            )
            .unwrap();
        if index == 2 {
            assert_eq!(
                track.status(Instant::now()),
                FuzzyTrackStatus::Consistent(3)
            );
        }
    }
    assert_eq!(track.status(Instant::now()), FuzzyTrackStatus::Restarted(1));
}

#[test]
fn overlapping_media_and_capture_cannot_count_even_with_identical_endpoint_differences() {
    let fixture = Fixture {
        duration: 2_500_000,
        ..Fixture::default()
    };
    let mut track = track();
    admit(&mut track, &fixture, 0);
    admit(&mut track, &fixture, 1);
    assert_eq!(track.status(Instant::now()), FuzzyTrackStatus::Restarted(1));
    assert!(track.proposal(Instant::now()).is_none());
}

#[test]
fn rolling_storage_is_eight_metadata_records_and_keeps_latest_support() {
    let fixture = Fixture::default();
    let mut track = track();
    for index in 0..16 {
        admit(&mut track, &fixture, index);
        assert!(track.points.len() <= MAX_TRACK_OBSERVATIONS);
    }
    let proposal = track.proposal(Instant::now()).unwrap();
    assert_eq!(proposal.observations(), 8);
    assert_eq!(
        proposal.observed_capture_span_us().start,
        fixture.capture + fixture.step * 8
    );
    assert_eq!(
        proposal.observed_media_span_us().end,
        fixture.media + fixture.step * 15 + fixture.duration
    );
}

#[test]
fn expiry_prunes_old_support_and_revokes_snapshots_without_refreshing_newer_points() {
    let fixture = Fixture::default();
    let mut track = track();
    for index in 0..4 {
        admit(&mut track, &fixture, index);
    }
    let now = Instant::now();
    track.points[0].expires = now + Duration::from_millis(10);
    let old = track.proposal(now).unwrap();
    let future = now + Duration::from_millis(11);
    assert_eq!(
        old.validate_at(scope(), future),
        Err(ResolverError::Stale.into())
    );
    track.refresh(future);
    assert_eq!(old.validate(scope()), Err(FuzzyAlignmentError::Obsolete));
    assert_eq!(track.status(future), FuzzyTrackStatus::Consistent(3));
    track.refresh(now + MAX_WINDOW_AGE + Duration::from_millis(1));
    assert_eq!(
        track.status(Instant::now()),
        FuzzyTrackStatus::Collecting(0)
    );
}

#[test]
fn withdrawn_utterance_and_new_valid_support_revoke_prior_snapshots() {
    let (fixture, mut track) = consistent();
    let old = track.proposal(Instant::now()).unwrap();
    track.withdraw_utterance(1);
    assert_eq!(
        track.status(Instant::now()),
        FuzzyTrackStatus::Collecting(2)
    );
    assert_eq!(old.validate(scope()), Err(FuzzyAlignmentError::Obsolete));
    admit(&mut track, &fixture, 3);
    assert_eq!(
        track.status(Instant::now()),
        FuzzyTrackStatus::Consistent(3)
    );
    let current = track.proposal(Instant::now()).unwrap();
    admit(&mut track, &fixture, 4);
    assert_eq!(
        current.validate(scope()),
        Err(FuzzyAlignmentError::Obsolete)
    );
}

#[test]
fn source_cancel_and_drop_stop_snapshots_without_cancelling_parent() {
    let parent = ResolverCancellation::default();
    let mut track = FuzzyTrack::new(scope(), parent.clone());
    let fixture = Fixture::default();
    for index in 0..3 {
        admit(&mut track, &fixture, index);
    }
    let old = track.proposal(Instant::now()).unwrap();
    drop(track);
    assert_eq!(old.validate(scope()), Err(FuzzyAlignmentError::Obsolete));
    assert!(!parent.is_cancelled());
    let mut track = FuzzyTrack::new(scope(), parent.clone());
    for index in 0..3 {
        admit(&mut track, &fixture, index);
    }
    let old = track.proposal(Instant::now()).unwrap();
    parent.cancel();
    assert_eq!(old.validate(scope()), Err(ResolverError::Cancelled.into()));
    assert_eq!(track.status(Instant::now()), FuzzyTrackStatus::Cancelled);
}

#[test]
fn superseded_job_token_does_not_revoke_committed_independent_final_metadata() {
    let fixture = Fixture::default();
    let mut track = track();
    let job = ResolverCancellation::default();
    let lexical = fixture.matched(0, &fixture.event(0), &job);
    track.admit(&lexical, Instant::now()).unwrap();
    job.cancel();
    assert!(lexical.validate(scope(), lexical.binding()).is_err());
    admit(&mut track, &fixture, 1);
    admit(&mut track, &fixture, 2);
    track
        .proposal(Instant::now())
        .unwrap()
        .validate(scope())
        .unwrap();
    track.stop();
    assert_eq!(track.status(Instant::now()), FuzzyTrackStatus::Stopped);
}

#[test]
fn wrong_epoch_or_document_cannot_admit_or_validate() {
    let (fixture, mut track) = consistent();
    let old = track.proposal(Instant::now()).unwrap();
    let other =
        AlignmentScope::new(scope().document_identity(), scope().capture_identity(), 2).unwrap();
    assert_eq!(old.validate(other), Err(FuzzyAlignmentError::Obsolete));
    let mut wrong = FuzzyTrack::new(other, ResolverCancellation::default());
    assert_eq!(
        wrong.admit(&fixture.proposal(0), Instant::now()),
        Err(FuzzyAlignmentError::Obsolete)
    );
    old.validate(scope()).unwrap();
    admit(&mut track, &fixture, 3);
}

#[test]
fn changed_geometry_under_same_binding_rejects_and_withdraws_support() {
    let (fixture, mut track) = consistent();
    let old = track.proposal(Instant::now()).unwrap();
    let mut changed = fixture.event(2);
    changed.start_us = changed.start_us.map(|start| start + 1);
    assert_eq!(
        track.admit(
            &fixture.matched(2, &changed, &ResolverCancellation::default()),
            Instant::now()
        ),
        Err(ResolverError::InvalidData.into())
    );
    assert_eq!(old.validate(scope()), Err(FuzzyAlignmentError::Obsolete));
    assert!(track.proposal(Instant::now()).is_none());
}

#[test]
fn signed_endpoint_math_handles_media_before_capture_and_large_u64_coordinates() {
    for fixture in [
        Fixture {
            media: 1_000_000,
            capture: 600_000_000,
            ..Fixture::default()
        },
        Fixture {
            media: u64::MAX - 1_000_000_000,
            capture: 42_000_000,
            ..Fixture::default()
        },
    ] {
        let mut track = track();
        for index in 0..3 {
            admit(&mut track, &fixture, index);
        }
        assert_eq!(
            track.proposal(Instant::now()).unwrap().endpoint_spread_us(),
            0
        );
    }
}
