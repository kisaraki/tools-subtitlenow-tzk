//! Bundled general OpenCC Taiwan phrase dictionaries, not a content cache.
use crate::ai::{speech::MAX_TRANSCRIPT_BYTES, translation::TranslationError};

/// Potential CPU/dictionary initialization work: call on a bounded CPU worker
/// for existing text, never on the capture callback or UI thread.
pub fn taiwan_phrase_conversion(text: &str) -> Result<String, TranslationError> {
    if text.len() > MAX_TRANSCRIPT_BYTES {
        return Err(TranslationError::InvalidData);
    }
    let result = zhconv::zhconv(text, zhconv::Variant::ZhTW);
    if result.len() > MAX_TRANSCRIPT_BYTES {
        return Err(TranslationError::InvalidData);
    }
    Ok(result)
}
