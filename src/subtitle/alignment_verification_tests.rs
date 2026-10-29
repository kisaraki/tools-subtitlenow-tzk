use super::*;
use crate::{
    audio::{
        capture::{AudioStamp, BLOCK_FRAMES, SAMPLE_RATE, SessionId},
        preprocess::NormalizedAudio,
    },
    resolver::contracts::{ResolverCancellation, TextCue},
    session::playback_clock::LivePlaybackClock,
    subtitle::{
        fuzzy_alignment::locate_fuzzy_cue,
        fuzzy_track::{
            FuzzyTrack,
            tests::{Fixture, scope},
        },
    },
};
fn observe(clock: &mut LivePlaybackClock, end: u64, sequence: u64, discontinuity: bool) {
    let frames = end * u64::from(SAMPLE_RATE) / 1_000_000;
    clock.observe(&NormalizedAudio {
        stamp: AudioStamp {
            session: scope().capture_identity().session,
            sequence,
            frame_position: Some(frames - BLOCK_FRAMES as u64),
            qpc_100ns: None,
            received: Instant::now(),
            discontinuity,
        },
        frames: BLOCK_FRAMES,
        samples: [0.0; BLOCK_FRAMES],
    });
}
fn accepted(
    count: usize,
    drift: u64,
    errors: usize,
    rival: bool,
) -> (FuzzyTrack, FuzzyCueProposal) {
    let fixture = Fixture {
        step: 1_000_000,
        ..Fixture::default()
    };
    let mut document = fixture.document();
    for (index, cue) in document.cues.iter_mut().enumerate() {
        cue.start_us = cue.start_us.map(|at| at + drift * index as u64);
    }
    if rival {
        document.cues.push(TextCue {
            start_us: Some(900_000_000),
            duration_us: Some(1_000_000),
            text: format!("{}xxx", "c".repeat(29)).into(),
        });
    }
    let mut track = FuzzyTrack::new(scope(), ResolverCancellation::default());
    let mut last = None;
    for index in 0..count {
        let mut event = fixture.event(index);
        if index == 0 && errors > 0 {
            event.text = format!("{}{}", "x".repeat(errors), "a".repeat(32 - errors)).into();
        }
        let proposal = locate_fuzzy_cue(
            scope(),
            &document,
            &event,
            fixture.stamp(index),
            &ResolverCancellation::default(),
        )
        .unwrap();
        track.admit(&proposal, Instant::now()).unwrap();
        last = Some(proposal);
    }
    (track, last.unwrap())
}
fn native_cursor(latest: &FuzzyCueProposal) -> (LivePlaybackClock, CaptureCursor) {
    let mut clock = LivePlaybackClock::new(scope().capture_identity());
    let end = latest.capture_range_us().end;
    for index in 0..250 {
        observe(
            &mut clock,
            end - 4_980_000 + index * 20_000,
            751 + index,
            false,
        );
    }
    let cursor = clock.cursor().unwrap();
    (clock, cursor)
}
#[test]
fn alignment_verification_requires_three_points_not_one_high_scoring_hint() {
    for count in 1..3 {
        assert!(
            accepted(count, 0, 0, false)
                .0
                .proposal(Instant::now())
                .is_none()
        );
    }
    let (track, latest) = accepted(3, 0, 0, false);
    let (_clock, cursor) = native_cursor(&latest);
    let verified = AlignmentVerifier::default()
        .verify(track.proposal(Instant::now()).unwrap(), &latest, cursor)
        .unwrap();
    assert_eq!(
        verified.position().unwrap(),
        ObservedMediaPosition {
            capture_us: 45_000_000,
            media_us: 603_000_000
        }
    );
    assert_eq!(verified.offset_us().unwrap(), 558_000_000);
    assert_eq!(verified.change(), CalibrationChange::Acquired);
}
#[test]
fn alignment_verification_enforces_stricter_agreement_and_runner_margin_across_every_point() {
    for (errors, rival) in [(2, false), (0, true)] {
        let (track, latest) = accepted(3, 0, errors, rival);
        let (_clock, cursor) = native_cursor(&latest);
        assert_eq!(
            AlignmentVerifier::default()
                .verify(track.proposal(Instant::now()).unwrap(), &latest, cursor)
                .err(),
            Some(AlignmentVerificationError::InsufficientEvidence)
        );
    }
}
#[test]
fn alignment_verification_rejects_a_lexically_consistent_but_loose_time_track() {
    let (track, latest) = accepted(3, 120_000, 0, false);
    let (_clock, cursor) = native_cursor(&latest);
    assert_eq!(
        AlignmentVerifier::default()
            .verify(track.proposal(Instant::now()).unwrap(), &latest, cursor)
            .err(),
        Some(AlignmentVerificationError::InsufficientEvidence)
    );
}
#[test]
fn alignment_verification_preserves_negative_offsets_without_unsigned_wrap() {
    let fixture = Fixture {
        media: 1_000_000,
        ..Fixture::default()
    };
    let document = fixture.document();
    let mut track = FuzzyTrack::new(scope(), ResolverCancellation::default());
    let mut latest = None;
    for index in 0..3 {
        let proposal = locate_fuzzy_cue(
            scope(),
            &document,
            &fixture.event(index),
            fixture.stamp(index),
            &ResolverCancellation::default(),
        )
        .unwrap();
        track.admit(&proposal, Instant::now()).unwrap();
        latest = Some(proposal);
    }
    let latest = latest.unwrap();
    let (_clock, cursor) = native_cursor(&latest);
    let verified = AlignmentVerifier::default()
        .verify(track.proposal(Instant::now()).unwrap(), &latest, cursor)
        .unwrap();
    assert_eq!(verified.offset_us().unwrap(), -41_000_000);
}
#[test]
fn alignment_verification_checks_every_endpoint_not_just_an_aggregate_rate() {
    let fixture = Fixture::default();
    let mut document = fixture.document();
    document.cues[1].start_us = document.cues[1].start_us.map(|value| value + 80_000);
    let mut track = FuzzyTrack::new(scope(), ResolverCancellation::default());
    let mut last = None;
    for index in 0..3 {
        let proposal = locate_fuzzy_cue(
            scope(),
            &document,
            &fixture.event(index),
            fixture.stamp(index),
            &ResolverCancellation::default(),
        )
        .unwrap();
        track.admit(&proposal, Instant::now()).unwrap();
        last = Some(proposal);
    }
    let latest = last.unwrap();
    let (_clock, cursor) = native_cursor(&latest);
    assert_eq!(
        AlignmentVerifier::default()
            .verify(track.proposal(Instant::now()).unwrap(), &latest, cursor)
            .err(),
        Some(AlignmentVerificationError::InsufficientEvidence)
    );
}
#[test]
fn alignment_verification_silence_and_continued_capture_never_advance_media_position() {
    let (track, latest) = accepted(3, 0, 0, false);
    let (mut clock, cursor) = native_cursor(&latest);
    let verified = AlignmentVerifier::default()
        .verify(track.proposal(Instant::now()).unwrap(), &latest, cursor)
        .unwrap();
    let position = verified.position().unwrap();
    for index in 1..5 {
        observe(
            &mut clock,
            position.capture_us + index * 20_000,
            1000 + index,
            false,
        );
    }
    assert_eq!(verified.position().unwrap(), position);
}
#[test]
fn alignment_verification_gap_and_clock_drop_revoke_old_cursors_and_verified_positions() {
    for drop_clock in [false, true] {
        let (track, latest) = accepted(3, 0, 0, false);
        let (mut clock, cursor) = native_cursor(&latest);
        let old_cursor = cursor.clone();
        let verified = AlignmentVerifier::default()
            .verify(track.proposal(Instant::now()).unwrap(), &latest, cursor)
            .unwrap();
        if drop_clock {
            drop(clock);
        } else {
            observe(
                &mut clock,
                latest.capture_range_us().end + 20_000,
                1001,
                true,
            );
        }
        assert!(!old_cursor.is_fresh());
        assert_eq!(
            verified.position(),
            Err(AlignmentVerificationError::InvalidCapture)
        );
    }
}
#[test]
fn alignment_verification_rejects_wrong_identity_ahead_or_lagging_cursors() {
    for delta in [-20_000i64, 2_020_000] {
        let (track, latest) = accepted(3, 0, 0, false);
        let mut clock = LivePlaybackClock::new(scope().capture_identity());
        observe(
            &mut clock,
            (latest.capture_range_us().end as i64 + delta) as u64,
            1000,
            false,
        );
        assert_eq!(
            AlignmentVerifier::default()
                .verify(
                    track.proposal(Instant::now()).unwrap(),
                    &latest,
                    clock.cursor().unwrap()
                )
                .err(),
            Some(AlignmentVerificationError::InvalidCapture)
        );
    }
    let (track, latest) = accepted(3, 0, 0, false);
    let mut different = scope().capture_identity();
    different.session = SessionId(903);
    let mut clock = LivePlaybackClock::new(different);
    observe(&mut clock, latest.capture_range_us().end, 1000, false);
    assert!(clock.cursor().is_none());
    assert!(track.proposal(Instant::now()).is_some());
}
#[test]
fn alignment_verification_checks_epoch_binding_and_rejects_duplicate_calibration() {
    let (track, latest) = accepted(3, 0, 0, false);
    let (_clock, cursor) = native_cursor(&latest);
    let mut verifier = AlignmentVerifier::default();
    let verified = verifier
        .verify(
            track.proposal(Instant::now()).unwrap(),
            &latest,
            cursor.clone(),
        )
        .unwrap();
    assert!(
        verifier
            .verify(track.proposal(Instant::now()).unwrap(), &latest, cursor)
            .is_err()
    );
    let next =
        AlignmentScope::new(scope().document_identity(), scope().capture_identity(), 2).unwrap();
    assert!(verified.validate(next, verified.binding()).is_err());
    let mut binding = verified.binding();
    binding.revision += 1;
    assert!(verified.validate(scope(), binding).is_err());
    verifier.reset(next);
    let (_clock2, cursor2) = native_cursor(&latest);
    assert!(
        verifier
            .verify(track.proposal(Instant::now()).unwrap(), &latest, cursor2)
            .is_err()
    );
}
#[test]
fn alignment_verification_track_retirement_and_revision_revoke_delivered_evidence() {
    let (mut track, latest) = accepted(3, 0, 0, false);
    let (_clock, cursor) = native_cursor(&latest);
    let verified = AlignmentVerifier::default()
        .verify(track.proposal(Instant::now()).unwrap(), &latest, cursor)
        .unwrap();
    track.withdraw_utterance(1);
    assert!(verified.position().is_err());
}
