//! Bounded descriptive tags only. No returned text, images, URLs or metadata
//! model: validation borrows the parent's short-lived moov, then discards it.
use super::{MediaError, ResolverCancellation, check, children, get, require, u32_at};

// The entire moov still has its independent 1 MiB cap. These per-value limits
// bound UTF validation work and forbid one huge cover monopolizing that budget.
const MAX_TEXT_BYTES: usize = 64 * 1024;
const MAX_COVER_BYTES: usize = 512 * 1024;
const MAX_METADATA_BYTES: usize = 768 * 1024;
const UTF8_TYPE: u32 = 1;
const JPEG_TYPE: u32 = 13;
const PNG_TYPE: u32 = 14;

pub(super) fn validate(data: &[u8], cancel: &ResolverCancellation) -> Result<(), MediaError> {
    check(cancel)?;
    require(data.len() <= MAX_METADATA_BYTES)?;
    let items = children(
        data,
        &[
            b"\xa9nam", b"\xa9ART", b"aART", b"\xa9alb", b"\xa9gen", b"\xa9wrt", b"\xa9day",
            b"\xa9cmt", b"\xa9too", b"cprt", b"covr", b"trkn", b"disk", b"tmpo", b"cpil",
        ],
        cancel,
    )?;
    for item in items {
        check(cancel)?;
        // Exactly one type/locale-specific value per known item. Unknown
        // nested atoms, references, freeform and multiple representations fail.
        let values = children(item.body, &[b"data"], cancel)?;
        let value = get(&values, b"data")?;
        require(value.len() >= 8)?;
        let kind = u32_at(value, 0)?; // Version must be zero; full 24-bit type.
        let payload = &value[8..]; // Four-byte locale is descriptive, not a pointer.
        if item.kind == b"covr" {
            require(!payload.is_empty() && payload.len() <= MAX_COVER_BYTES)?;
            let known_header = match kind {
                JPEG_TYPE => payload.starts_with(&[0xff, 0xd8, 0xff]),
                PNG_TYPE => payload.starts_with(b"\x89PNG\r\n\x1a\n"),
                _ => false,
            };
            require(known_header)?;
            // No image codec, allocation or dimension interpretation. This is
            // an inert bounded cover value, not proof of a decodable picture.
        } else if matches!(item.kind, b"trkn" | b"disk") {
            let length = if item.kind == b"trkn" { 8 } else { 6 };
            require(kind == 0 && payload.len() == length && payload[..2] == [0, 0])?;
            if length == 8 {
                require(payload[6..] == [0, 0])?;
            }
            let index = u16::from_be_bytes([payload[2], payload[3]]);
            let total = u16::from_be_bytes([payload[4], payload[5]]);
            require(total == 0 || index <= total)?;
        } else if matches!(item.kind, b"tmpo" | b"cpil") {
            // Conventional integer tempo and compilation flags. No timestamps
            // or decoder trimming semantics are accepted from user metadata.
            require(kind == 21)?;
            if item.kind == b"tmpo" {
                require(payload.len() == 2 && payload[0] & 0x80 == 0)?;
            } else {
                require(payload.len() == 1 && matches!(payload[0], 0 | 1))?;
            }
        } else {
            require(kind == UTF8_TYPE && payload.len() <= MAX_TEXT_BYTES)?;
            require(std::str::from_utf8(payload).is_ok())?;
        }
        check(cancel)?;
    }
    check(cancel)
}

#[cfg(test)]
#[path = "local_mp4_metadata_tests.rs"]
mod tests;
