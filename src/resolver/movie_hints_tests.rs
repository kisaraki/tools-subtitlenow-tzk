//! Synthetic Final ASR and capture stamps only; no media or cloud requests.
use super::*;
use crate::audio::capture::SessionId;
use std::time::Duration;

const DIALOGUE: &str = "A synthetic lantern remains beside this empty doorway";
fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(942),
        generation: 6,
    }
}
fn event(sequence: u64) -> TranscriptEvent {
    TranscriptEvent {
        identity: identity(),
        utterance: sequence,
        revision: 2,
        sequence,
        start_us: Some(3_000_000),
        duration_us: Some(2_000_000),
        language: Some(SpeechLanguage::English),
        kind: TranscriptKind::Final,
        text: DIALOGUE.into(),
    }
}
fn stamp_at(now: Instant) -> WindowStamp {
    WindowStamp {
        identity: identity(),
        sequence: 20,
        ended_at: now,
        start_us: Some(1_000_000),
        duration_us: 12_000_000,
    }
}
fn snapshot(buffer: &MovieHintBuffer) -> MovieDialogueSnapshot {
    buffer.query(stamp_at(Instant::now())).unwrap().unwrap()
}
fn assert_kept(buffer: &MovieHintBuffer, original: &MovieDialogueSnapshot) {
    let current = snapshot(buffer);
    assert_eq!(current.dialogue.as_ref(), DIALOGUE);
    assert!(current.cancellation.same_scope(&original.cancellation));
    assert!(!original.cancellation.is_cancelled());
}

#[test]
fn final_native_dialogue_and_supported_or_unknown_language_are_preserved() {
    for language in SpeechLanguage::ALL.map(Some).into_iter().chain([None]) {
        let buffer = MovieHintBuffer::new(identity());
        let mut source = event(1);
        source.language = language;
        assert_eq!(buffer.observe(&source), Ok(true));
        let result = snapshot(&buffer);
        assert_eq!(result.dialogue, source.text);
        assert_eq!(result.language, language);
        assert!(!result.cancellation.is_cancelled());
    }
}

#[test]
fn foreign_session_or_generation_is_rejected_without_retiring_current() {
    let buffer = MovieHintBuffer::new(identity());
    buffer.observe(&event(1)).unwrap();
    let original = snapshot(&buffer);
    for foreign in [
        StreamIdentity {
            session: SessionId(943),
            ..identity()
        },
        StreamIdentity {
            generation: 7,
            ..identity()
        },
    ] {
        let mut source = event(2);
        source.identity = foreign;
        assert_eq!(buffer.observe(&source), Err(ResolverError::InvalidData));
        let mut stamp = stamp_at(Instant::now());
        stamp.identity = foreign;
        assert!(matches!(
            buffer.query(stamp),
            Err(ResolverError::InvalidData)
        ));
        assert_kept(&buffer, &original);
    }
}

#[test]
fn partials_and_missing_native_timing_do_not_replace_or_advance_watermark() {
    let buffer = MovieHintBuffer::new(identity());
    buffer.observe(&event(1)).unwrap();
    let original = snapshot(&buffer);
    let mut partial = event(99);
    partial.kind = TranscriptKind::Partial;
    partial.text = "x".repeat(MAX_DIALOGUE_BYTES + 1).into();
    assert_eq!(buffer.observe(&partial), Ok(false));
    for absent in [0, 1, 2] {
        let mut source = event(99);
        if absent != 1 {
            source.start_us = None;
        }
        if absent != 0 {
            source.duration_us = None;
        }
        assert_eq!(buffer.observe(&source), Ok(false));
        assert_kept(&buffer, &original);
    }
    assert_eq!(buffer.observe(&event(2)), Ok(true));
    assert!(original.cancellation.is_cancelled());
}

#[test]
fn invalid_text_or_native_interval_preserves_valid_current_dialogue() {
    let buffer = MovieHintBuffer::new(identity());
    buffer.observe(&event(1)).unwrap();
    let original = snapshot(&buffer);
    for invalid in 0..6 {
        let mut source = event(2);
        match invalid {
            0 => source.text = "".into(),
            1 => source.text = " \r\n\t ".into(),
            2 => source.text = "x".repeat(MAX_DIALOGUE_BYTES + 1).into(),
            3 => source.text = "Synthetic\0dialogue".into(),
            4 => source.duration_us = Some(0),
            _ => {
                source.start_us = Some(u64::MAX);
                source.duration_us = Some(1);
            }
        }
        assert_eq!(buffer.observe(&source), Err(ResolverError::InvalidData));
        assert_kept(&buffer, &original);
    }
}

#[test]
fn foreign_or_malformed_partial_is_not_silently_accepted() {
    let buffer = MovieHintBuffer::new(identity());
    let mut source = event(1);
    source.kind = TranscriptKind::Partial;
    source.identity.generation += 1;
    assert_eq!(buffer.observe(&source), Err(ResolverError::InvalidData));
    source.identity = identity();
    source.text = "x".repeat(MAX_TRANSCRIPT_BYTES + 1).into();
    assert_eq!(buffer.observe(&source), Err(ResolverError::InvalidData));
    source.text = DIALOGUE.into();
    source.duration_us = Some(0);
    assert_eq!(buffer.observe(&source), Err(ResolverError::InvalidData));
}

#[test]
fn bounded_final_text_allows_multibyte_and_native_line_breaks() {
    let buffer = MovieHintBuffer::new(identity());
    let mut source = event(1);
    source.text = format!("{}\n\t", "燈".repeat((MAX_DIALOGUE_BYTES - 2) / 3)).into();
    assert!(source.text.len() <= MAX_DIALOGUE_BYTES);
    assert_eq!(buffer.observe(&source), Ok(true));
    assert_eq!(snapshot(&buffer).dialogue, source.text);
    source.sequence += 1;
    source.utterance += 1;
    source.text = "x".repeat(MAX_DIALOGUE_BYTES).into();
    assert_eq!(buffer.observe(&source), Ok(true));
    assert_eq!(snapshot(&buffer).dialogue.len(), MAX_DIALOGUE_BYTES);
}

#[test]
fn duplicate_or_lower_sequence_and_nonadvancing_same_utterance_revision_are_stale() {
    let buffer = MovieHintBuffer::new(identity());
    buffer.observe(&event(10)).unwrap();
    let original = snapshot(&buffer);
    for sequence in [9, 10] {
        let mut source = event(sequence);
        source.revision = 99;
        assert_eq!(buffer.observe(&source), Err(ResolverError::Stale));
        assert_kept(&buffer, &original);
    }
    for revision in [1, 2] {
        let mut source = event(11);
        source.utterance = 10;
        source.revision = revision;
        assert_eq!(buffer.observe(&source), Err(ResolverError::Stale));
        assert_kept(&buffer, &original);
    }
    let mut next = event(11);
    next.utterance = 10;
    next.revision = 3;
    assert_eq!(buffer.observe(&next), Ok(true));
    assert!(original.cancellation.is_cancelled());
}

#[test]
fn entire_native_interval_must_be_contained_and_boundary_equality_is_allowed() {
    let buffer = MovieHintBuffer::new(identity());
    buffer.observe(&event(1)).unwrap();
    let original = snapshot(&buffer);
    for (start, duration, present) in [
        (3_000_000, 2_000_000, true),
        (2_000_000, 3_000_000, true),
        (3_000_001, 2_000_000, false),
        (3_000_000, 1_999_999, false),
        (1_000_000, 1_000_000, false),
        (6_000_000, 1_000_000, false),
    ] {
        let mut stamp = stamp_at(Instant::now());
        stamp.start_us = Some(start);
        stamp.duration_us = duration;
        assert_eq!(buffer.query(stamp).unwrap().is_some(), present);
        assert!(!original.cancellation.is_cancelled());
    }
    let mut stamp = stamp_at(Instant::now());
    stamp.start_us = None;
    assert!(buffer.query(stamp).unwrap().is_none());
    assert_kept(&buffer, &original);
}

#[test]
fn greater_arrival_sequence_cannot_replace_with_regressed_native_start_or_end() {
    let buffer = MovieHintBuffer::new(identity());
    buffer.observe(&event(10)).unwrap();
    let original = snapshot(&buffer);
    for (start_us, duration_us) in [
        (2_000_000, 4_000_000),
        (4_000_000, 500_000),
        (2_000_000, 2_000_000),
    ] {
        let mut late = event(11);
        late.start_us = Some(start_us);
        late.duration_us = Some(duration_us);
        assert_eq!(buffer.observe(&late), Err(ResolverError::Stale));
        assert_kept(&buffer, &original);
        late.utterance = 10;
        late.revision = 3;
        assert_eq!(buffer.observe(&late), Err(ResolverError::Stale));
        assert_kept(&buffer, &original);
    }
    buffer.clear();
    let mut late = event(11);
    late.start_us = Some(2_000_000);
    assert_eq!(buffer.observe(&late), Err(ResolverError::Stale));
    assert!(buffer.query(stamp_at(Instant::now())).unwrap().is_none());
    assert_eq!(buffer.observe(&event(11)), Ok(true));
}

#[test]
fn invalid_actual_capture_interval_cannot_be_used_as_containment() {
    let buffer = MovieHintBuffer::new(identity());
    buffer.observe(&event(1)).unwrap();
    let original = snapshot(&buffer);
    for invalid in 0..2 {
        let mut stamp = stamp_at(Instant::now());
        if invalid == 0 {
            stamp.duration_us = 0;
        } else {
            stamp.start_us = Some(u64::MAX);
            stamp.duration_us = 1;
        }
        assert!(matches!(
            buffer.query(stamp),
            Err(ResolverError::InvalidData)
        ));
        assert_kept(&buffer, &original);
    }
}

#[test]
fn receipt_age_is_independent_of_current_window_age_and_expiry_cancels_lease() {
    let buffer = MovieHintBuffer::new(identity());
    let received = Instant::now();
    buffer.observe_at(&event(1), received).unwrap();
    let original = buffer
        .query_at(stamp_at(received), received)
        .unwrap()
        .unwrap();
    let boundary = received + MAX_WINDOW_AGE;
    assert_eq!(original.expires_at, boundary);
    assert!(
        buffer
            .query_at(stamp_at(boundary), boundary)
            .unwrap()
            .is_some()
    );
    // Use a representable Windows clock interval beyond the exact boundary.
    let expired = boundary + Duration::from_millis(1);
    assert!(matches!(
        buffer.query_at(stamp_at(expired), expired),
        Err(ResolverError::Stale)
    ));
    assert!(original.cancellation.is_cancelled());
    assert!(
        buffer
            .query_at(stamp_at(expired), expired)
            .unwrap()
            .is_none()
    );
    assert_eq!(buffer.observe(&event(1)), Err(ResolverError::Stale));
}

#[test]
fn stale_or_future_window_is_rejected_without_laundering_or_retiring_valid_hint() {
    let buffer = MovieHintBuffer::new(identity());
    let now = Instant::now();
    buffer.observe_at(&event(1), now).unwrap();
    let original = buffer.query_at(stamp_at(now), now).unwrap().unwrap();
    for ended_at in [
        now - MAX_WINDOW_AGE - Duration::from_millis(1),
        now + Duration::from_millis(1),
    ] {
        let mut stamp = stamp_at(now);
        stamp.ended_at = ended_at;
        assert!(matches!(
            buffer.query_at(stamp, now),
            Err(ResolverError::Stale)
        ));
        assert!(!original.cancellation.is_cancelled());
    }
    assert!(buffer.query_at(stamp_at(now), now).unwrap().is_some());
}

#[test]
fn replacement_and_clear_cancel_copies_and_clear_retains_ordering_watermark() {
    let buffer = MovieHintBuffer::new(identity());
    buffer.observe(&event(1)).unwrap();
    let first = snapshot(&buffer);
    buffer.observe(&event(2)).unwrap();
    let second = snapshot(&buffer);
    assert!(first.cancellation.is_cancelled());
    assert!(!second.cancellation.is_cancelled());
    assert!(!first.cancellation.same_scope(&second.cancellation));
    buffer.clear();
    buffer.clear();
    assert!(second.cancellation.is_cancelled());
    assert!(buffer.query(stamp_at(Instant::now())).unwrap().is_none());
    assert_eq!(buffer.observe(&event(1)), Err(ResolverError::Stale));
    assert_eq!(buffer.observe(&event(2)), Err(ResolverError::Stale));
    assert_eq!(buffer.observe(&event(3)), Ok(true));
}

#[test]
fn stop_is_permanent_and_drop_retires_remaining_publication() {
    let buffer = MovieHintBuffer::new(identity());
    buffer.observe(&event(1)).unwrap();
    let first = snapshot(&buffer);
    buffer.stop();
    buffer.clear();
    buffer.stop();
    assert!(first.cancellation.is_cancelled());
    assert_eq!(buffer.observe(&event(2)), Err(ResolverError::Cancelled));
    assert!(matches!(
        buffer.query(stamp_at(Instant::now())),
        Err(ResolverError::Cancelled)
    ));
    let buffer = MovieHintBuffer::new(identity());
    buffer.observe(&event(1)).unwrap();
    let remaining = snapshot(&buffer);
    drop(buffer);
    assert!(remaining.cancellation.is_cancelled());
}

#[test]
fn repeated_latest_replacements_never_accumulate_history() {
    let buffer = MovieHintBuffer::new(identity());
    let mut previous = None::<MovieDialogueSnapshot>;
    for sequence in 0..1000 {
        let mut source = event(sequence);
        source.text = format!("{sequence} {DIALOGUE}").into();
        assert_eq!(buffer.observe(&source), Ok(true));
        let current = snapshot(&buffer);
        assert_eq!(current.dialogue, source.text);
        if let Some(previous) = previous.take() {
            assert!(previous.cancellation.is_cancelled());
        }
        assert_eq!(
            buffer
                .state
                .lock()
                .unwrap()
                .watermark
                .as_ref()
                .unwrap()
                .sequence,
            sequence
        );
        previous = Some(current);
    }
    buffer.clear();
    assert!(buffer.state.lock().unwrap().latest.is_none());
    assert!(previous.unwrap().cancellation.is_cancelled());
}
