//! Synthetic caption scans and deterministic monotonic clocks; no native apps,
//! credentials, private window titles, or network requests are read here.
use super::*;
use crate::{audio::capture::SessionId, resolver::contracts::MAX_METADATA_BYTES};

const TITLE: &str = "Synthetic Lantern - Fixture Player";
fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(1401),
        generation: 4,
    }
}
fn stamp_at(now: Instant) -> WindowStamp {
    WindowStamp {
        identity: identity(),
        sequence: 71,
        ended_at: now,
        start_us: Some(4_000_000),
        duration_us: 12_000_000,
    }
}
fn publish(buffer: &SourceMetadataBuffer, sequence: u64, at: Instant, text: &str) {
    buffer
        .publish_at(sequence, at, Ok(text.into()), at)
        .unwrap();
}
fn snapshot(buffer: &SourceMetadataBuffer, now: Instant) -> MovieHintMetadata {
    buffer.metadata_at(stamp_at(now), now).unwrap()
}
fn validity(snapshot: &MovieHintMetadata) -> &MovieMetadataValidity {
    snapshot.validity.as_ref().unwrap()
}
fn assert_preserved(buffer: &SourceMetadataBuffer, original: &MovieHintMetadata, now: Instant) {
    let latest = snapshot(buffer, now);
    assert_eq!(latest.window_title, original.window_title);
    assert!(
        validity(&latest)
            .cancellation
            .same_scope(&validity(original).cancellation)
    );
    assert_eq!(validity(&latest).expires_at, validity(original).expires_at);
    assert!(!validity(original).cancellation.is_cancelled());
}

#[test]
fn exact_requested_stamp_and_only_actual_title_are_returned() {
    let buffer = SourceMetadataBuffer::new(identity());
    let at = Instant::now();
    publish(&buffer, 1, at, TITLE);
    let request = stamp_at(at);
    let result = buffer.metadata_at(request, at).unwrap();
    assert_eq!(result.window_title.as_deref(), Some(TITLE));
    assert!(result.filename.is_none());
    assert!(result.year.is_none());
    assert!(result.duration_us.is_none());
    assert_eq!(result.stamp.identity, request.identity);
    assert_eq!(result.stamp.sequence, request.sequence);
    assert_eq!(result.stamp.ended_at, request.ended_at);
    assert_eq!(result.stamp.start_us, request.start_us);
    assert_eq!(result.stamp.duration_us, request.duration_us);
    assert_eq!(validity(&result).expires_at, at + MAX_SOURCE_METADATA_AGE);
}

#[test]
fn public_ram_interfaces_publish_and_read_without_inference() {
    let buffer = SourceMetadataBuffer::new(identity());
    let at = Instant::now();
    buffer.publish(1, at, Ok(TITLE.into())).unwrap();
    let result = buffer.metadata(stamp_at(Instant::now())).unwrap();
    assert_eq!(result.window_title.as_deref(), Some(TITLE));
    assert!(result.filename.is_none());
    assert!(result.year.is_none());
    assert!(result.duration_us.is_none());
}

#[test]
fn foreign_session_or_generation_does_not_clear_current_title() {
    let buffer = SourceMetadataBuffer::new(identity());
    let at = Instant::now();
    publish(&buffer, 1, at, TITLE);
    let original = snapshot(&buffer, at);
    for foreign in [
        StreamIdentity {
            session: SessionId(1402),
            ..identity()
        },
        StreamIdentity {
            generation: 5,
            ..identity()
        },
    ] {
        let mut request = stamp_at(at);
        request.identity = foreign;
        assert!(matches!(
            buffer.metadata_at(request, at),
            Err(ResolverError::InvalidData)
        ));
        assert_preserved(&buffer, &original, at);
    }
}

#[test]
fn missing_caption_is_explicitly_unavailable() {
    let buffer = SourceMetadataBuffer::new(identity());
    let at = Instant::now();
    assert!(matches!(
        buffer.metadata_at(stamp_at(at), at),
        Err(ResolverError::SourceMetadataUnavailable)
    ));
}

#[test]
fn stale_or_invalid_capture_stamp_preserves_valid_source_publication() {
    let buffer = SourceMetadataBuffer::new(identity());
    let at = Instant::now();
    publish(&buffer, 1, at, TITLE);
    let original = snapshot(&buffer, at);
    for invalid in 0..4 {
        let mut request = stamp_at(at);
        let expected = match invalid {
            0 => {
                request.duration_us = 0;
                ResolverError::InvalidData
            }
            1 => {
                request.start_us = Some(u64::MAX);
                ResolverError::InvalidData
            }
            2 => {
                request.ended_at = at + Duration::from_millis(1);
                ResolverError::Stale
            }
            _ => {
                request.ended_at = at - Duration::from_secs(6);
                ResolverError::Stale
            }
        };
        assert!(matches!(buffer.metadata_at(request, at), Err(error) if error == expected));
        assert_preserved(&buffer, &original, at);
    }
}

#[test]
fn original_scan_age_is_independent_of_fresh_capture_and_ui_receipt() {
    let buffer = SourceMetadataBuffer::new(identity());
    let at = Instant::now();
    let now = at + MAX_SOURCE_METADATA_AGE + Duration::from_millis(1);
    assert_eq!(
        buffer.publish_at(1, at, Ok(TITLE.into()), now),
        Err(ResolverError::Stale)
    );
    assert!(matches!(
        buffer.metadata_at(stamp_at(now), now),
        Err(ResolverError::Stale)
    ));
    publish(&buffer, 2, now, TITLE);
    let original = snapshot(&buffer, now);
    let expired = now + MAX_SOURCE_METADATA_AGE + Duration::from_millis(1);
    assert!(matches!(
        buffer.metadata_at(stamp_at(expired), expired),
        Err(ResolverError::Stale)
    ));
    assert!(validity(&original).cancellation.is_cancelled());
    assert!(matches!(
        buffer.metadata_at(stamp_at(expired), expired),
        Err(ResolverError::Stale)
    ));
}

#[test]
fn exact_age_boundary_is_accepted_then_one_millisecond_later_expires() {
    let buffer = SourceMetadataBuffer::new(identity());
    let at = Instant::now();
    let boundary = at + MAX_SOURCE_METADATA_AGE;
    buffer
        .publish_at(1, at, Ok(TITLE.into()), boundary)
        .unwrap();
    let original = snapshot(&buffer, boundary);
    assert!(!validity(&original).cancellation.is_cancelled());
    let now = boundary + Duration::from_millis(1);
    assert!(matches!(
        buffer.metadata_at(stamp_at(now), now),
        Err(ResolverError::Stale)
    ));
    assert!(validity(&original).cancellation.is_cancelled());
}

#[test]
fn future_publication_clears_without_poisoning_recovery_observation_time() {
    let buffer = SourceMetadataBuffer::new(identity());
    let at = Instant::now();
    publish(&buffer, 1, at, TITLE);
    let original = snapshot(&buffer, at);
    assert_eq!(
        buffer.publish_at(2, at + Duration::from_millis(1), Ok(TITLE.into()), at),
        Err(ResolverError::Stale)
    );
    assert!(validity(&original).cancellation.is_cancelled());
    assert!(matches!(
        buffer.metadata_at(stamp_at(at), at),
        Err(ResolverError::Stale)
    ));
    publish(&buffer, 3, at, TITLE);
    assert!(!validity(&snapshot(&buffer, at)).cancellation.is_cancelled());
}

#[test]
fn duplicate_or_lower_sequence_and_regressed_observation_preserve_current() {
    let buffer = SourceMetadataBuffer::new(identity());
    let at = Instant::now();
    publish(&buffer, 10, at, TITLE);
    let original = snapshot(&buffer, at);
    for sequence in [9, 10] {
        for text in [
            Ok("Different Synthetic Caption".into()),
            Err(ResolverError::Network),
        ] {
            assert_eq!(
                buffer.publish_at(sequence, at + Duration::from_millis(1), text, at),
                Err(ResolverError::Stale)
            );
            assert_preserved(&buffer, &original, at);
        }
    }
    assert_eq!(
        buffer.publish_at(
            11,
            at - Duration::from_millis(1),
            Err(ResolverError::Network),
            at
        ),
        Err(ResolverError::Stale)
    );
    assert_preserved(&buffer, &original, at);
    // Rejected arrival did not consume the next valid scan's sequence.
    publish(&buffer, 11, at, "Changed Synthetic Caption");
    assert!(validity(&original).cancellation.is_cancelled());
}

#[test]
fn malformed_or_missing_new_caption_clears_and_advances_ordering() {
    for text in [
        "".to_owned(),
        "   \t ".to_owned(),
        "Synthetic\0Caption".to_owned(),
        "Synthetic\nCaption".to_owned(),
        "x".repeat(MAX_METADATA_BYTES + 1),
    ] {
        let buffer = SourceMetadataBuffer::new(identity());
        let at = Instant::now();
        publish(&buffer, 10, at, TITLE);
        let original = snapshot(&buffer, at);
        assert_eq!(
            buffer.publish_at(11, at, Ok(text.into()), at),
            Err(ResolverError::InvalidData)
        );
        assert!(validity(&original).cancellation.is_cancelled());
        assert!(matches!(
            buffer.metadata_at(stamp_at(at), at),
            Err(ResolverError::InvalidData)
        ));
        assert_eq!(
            buffer.publish_at(11, at, Ok(TITLE.into()), at),
            Err(ResolverError::Stale)
        );
        publish(&buffer, 12, at, TITLE);
        assert_eq!(snapshot(&buffer, at).window_title.as_deref(), Some(TITLE));
    }
}

#[test]
fn maximum_multibyte_caption_is_retained_without_language_or_filename_inference() {
    for text in [
        "x".repeat(MAX_METADATA_BYTES),
        format!("{}x", "燈".repeat((MAX_METADATA_BYTES - 1) / 3)),
        "Synthetic 2024 한국어 日本語 English \u{2003} - fake.mkv".to_owned(),
    ] {
        let buffer = SourceMetadataBuffer::new(identity());
        let at = Instant::now();
        publish(&buffer, 1, at, &text);
        let result = snapshot(&buffer, at);
        assert_eq!(result.window_title.as_deref(), Some(text.as_str()));
        assert!(result.filename.is_none());
        assert!(result.year.is_none());
        assert!(result.duration_us.is_none());
    }
}

#[test]
fn same_caption_fresh_scan_renews_only_new_lease_deadline() {
    let buffer = SourceMetadataBuffer::new(identity());
    let at = Instant::now();
    publish(&buffer, 1, at, TITLE);
    let original = snapshot(&buffer, at);
    let scanned_again = at + Duration::from_secs(1);
    publish(&buffer, 2, scanned_again, TITLE);
    let current = snapshot(&buffer, scanned_again);
    assert!(
        validity(&current)
            .cancellation
            .same_scope(&validity(&original).cancellation)
    );
    assert_eq!(validity(&original).expires_at, at + MAX_SOURCE_METADATA_AGE);
    assert_eq!(
        validity(&current).expires_at,
        scanned_again + MAX_SOURCE_METADATA_AGE
    );
    let after_old_deadline = validity(&original).expires_at + Duration::from_millis(1);
    let new_lease = snapshot(&buffer, after_old_deadline);
    assert_eq!(
        validity(&new_lease).expires_at,
        validity(&current).expires_at
    );
    assert!(!validity(&original).cancellation.is_cancelled());
}

#[test]
fn same_caption_after_gap_cannot_resurrect_expired_publication_token() {
    let buffer = SourceMetadataBuffer::new(identity());
    let at = Instant::now();
    publish(&buffer, 1, at, TITLE);
    let original = snapshot(&buffer, at);
    let next = at + MAX_SOURCE_METADATA_AGE + Duration::from_millis(1);
    publish(&buffer, 2, next, TITLE);
    let current = snapshot(&buffer, next);
    assert!(validity(&original).cancellation.is_cancelled());
    assert!(
        !validity(&current)
            .cancellation
            .same_scope(&validity(&original).cancellation)
    );
    assert!(!validity(&current).cancellation.is_cancelled());
}

#[test]
fn changed_caption_and_error_scan_retire_handed_out_publications_immediately() {
    let buffer = SourceMetadataBuffer::new(identity());
    let at = Instant::now();
    publish(&buffer, 1, at, TITLE);
    let original = snapshot(&buffer, at);
    publish(&buffer, 2, at, "Changed Synthetic Caption");
    assert!(validity(&original).cancellation.is_cancelled());
    let changed = snapshot(&buffer, at);
    assert_eq!(
        buffer.publish_at(3, at, Err(ResolverError::SourceMetadataUnavailable), at),
        Err(ResolverError::SourceMetadataUnavailable)
    );
    assert!(validity(&changed).cancellation.is_cancelled());
    assert!(matches!(
        buffer.metadata_at(stamp_at(at), at),
        Err(ResolverError::SourceMetadataUnavailable)
    ));
    publish(&buffer, 4, at, TITLE);
    let restored = snapshot(&buffer, at);
    assert_eq!(
        buffer.publish_at(5, at, Err(ResolverError::WorkerPanicked), at),
        Err(ResolverError::WorkerPanicked)
    );
    assert!(validity(&restored).cancellation.is_cancelled());
    assert!(matches!(
        buffer.metadata_at(stamp_at(at), at),
        Err(ResolverError::WorkerPanicked)
    ));
}

#[test]
fn fatal_failure_and_invalid_data_are_retained_until_a_valid_new_scan_recovers() {
    let buffer = SourceMetadataBuffer::new(identity());
    let at = Instant::now();
    publish(&buffer, 10, at, TITLE);
    let original = snapshot(&buffer, at);
    buffer.fail(ResolverError::WorkerPanicked);
    assert!(validity(&original).cancellation.is_cancelled());
    assert!(matches!(
        buffer.metadata_at(stamp_at(at), at),
        Err(ResolverError::WorkerPanicked)
    ));
    assert_eq!(
        buffer.publish_at(10, at, Err(ResolverError::Network), at),
        Err(ResolverError::Stale)
    );
    assert!(matches!(
        buffer.metadata_at(stamp_at(at), at),
        Err(ResolverError::WorkerPanicked)
    ));
    publish(&buffer, 11, at, TITLE);
    assert_eq!(snapshot(&buffer, at).window_title.as_deref(), Some(TITLE));
    assert_eq!(
        buffer.publish_at(12, at, Ok("Synthetic\nCaption".into()), at),
        Err(ResolverError::InvalidData)
    );
    assert!(matches!(
        buffer.metadata_at(stamp_at(at), at),
        Err(ResolverError::InvalidData)
    ));
    assert_eq!(
        buffer.publish_at(11, at, Ok(TITLE.into()), at),
        Err(ResolverError::Stale)
    );
    assert!(matches!(
        buffer.metadata_at(stamp_at(at), at),
        Err(ResolverError::InvalidData)
    ));
    publish(&buffer, 13, at, TITLE);
    assert_eq!(snapshot(&buffer, at).window_title.as_deref(), Some(TITLE));
    buffer.stop();
    buffer.fail(ResolverError::WorkerPanicked);
    assert!(matches!(
        buffer.metadata_at(stamp_at(at), at),
        Err(ResolverError::Cancelled)
    ));
}

#[test]
fn later_expired_publication_retires_current_before_any_query() {
    let buffer = SourceMetadataBuffer::new(identity());
    let at = Instant::now();
    publish(&buffer, 1, at, TITLE);
    let original = snapshot(&buffer, at);
    let now = at + MAX_SOURCE_METADATA_AGE + Duration::from_millis(1);
    assert_eq!(
        buffer.publish_at(2, at, Ok(TITLE.into()), now),
        Err(ResolverError::Stale)
    );
    assert!(validity(&original).cancellation.is_cancelled());
    assert_eq!(
        buffer.publish_at(2, now, Ok(TITLE.into()), now),
        Err(ResolverError::Stale)
    );
    publish(&buffer, 3, now, TITLE);
    assert_eq!(snapshot(&buffer, now).window_title.as_deref(), Some(TITLE));
}

#[test]
fn stop_is_permanent_and_drop_cancels_external_snapshots() {
    let at = Instant::now();
    let buffer = SourceMetadataBuffer::new(identity());
    publish(&buffer, 1, at, TITLE);
    let original = snapshot(&buffer, at);
    buffer.stop();
    buffer.stop();
    assert!(validity(&original).cancellation.is_cancelled());
    assert_eq!(
        buffer.publish_at(2, at, Ok(TITLE.into()), at),
        Err(ResolverError::Cancelled)
    );
    assert!(matches!(
        buffer.metadata_at(stamp_at(at), at),
        Err(ResolverError::Cancelled)
    ));
    let dropped = {
        let owner = SourceMetadataBuffer::new(identity());
        publish(&owner, 1, at, TITLE);
        snapshot(&owner, at)
    };
    assert!(validity(&dropped).cancellation.is_cancelled());
}

#[test]
fn repeated_scan_retains_only_latest_bounded_caption_and_retires_prior_copies() {
    let buffer = SourceMetadataBuffer::new(identity());
    let at = Instant::now();
    let mut previous: Option<MovieHintMetadata> = None;
    for sequence in 1..=256 {
        let text = format!("Synthetic Fixture {sequence}");
        publish(&buffer, sequence, at, &text);
        if let Some(previous) = previous.take() {
            assert!(validity(&previous).cancellation.is_cancelled());
        }
        let current = snapshot(&buffer, at);
        assert_eq!(current.window_title.as_deref(), Some(text.as_str()));
        assert!(current.window_title.as_ref().unwrap().len() <= MAX_METADATA_BYTES);
        previous = Some(current);
    }
    buffer.stop();
    assert!(
        validity(previous.as_ref().unwrap())
            .cancellation
            .is_cancelled()
    );
}
