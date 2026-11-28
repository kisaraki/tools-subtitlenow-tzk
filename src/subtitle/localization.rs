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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn taiwan_output_gate_normalization_handles_mixed_scripts_and_is_idempotent() {
        let expected =
            "記憶體、軟體、伺服器、網路、資料夾、滑鼠、計程車、印表機、硬碟，Rust 1.98。";
        for text in [
            "内存、软件、服务器、网络、文件夹、鼠标、出租车、打印机、硬盘，Rust 1.98。",
            "內存、軟件、服務器、網絡、文件夾、鼠標、出租車、打印機、硬盤，Rust 1.98。",
            expected,
        ] {
            assert_eq!(taiwan_phrase_conversion(text).unwrap(), expected);
        }
    }
}
