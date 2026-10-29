//! App-owned credential names only. Keys are neither serializable nor clonable.
use super::provider::SpeechProvider;
use zeroize::Zeroizing;

pub const MAX_KEY_BLOB_BYTES: usize = 2560;
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CredentialError {
    #[error("尚未設定所選服務的 API 金鑰。請在主視窗輸入，或先儲存至 Windows 認證管理員。")]
    Missing,
    #[error("無法讀取或儲存 API 金鑰，請確認 Windows 登入工作階段。")]
    Unavailable,
    #[error("API 金鑰格式無效；請輸入不含空白或控制字元的有效 API 金鑰。")]
    InvalidEncoding,
    #[error("API 憑證讀取目前只支援 Windows。")]
    UnsupportedPlatform,
    #[error("金鑰寫入要求已完成，但無法確認本機讀取；請按「檢查本機」確認。")]
    VerificationUnavailable,
}

pub struct ApiKey(Zeroizing<String>);
impl std::fmt::Debug for ApiKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ApiKey([REDACTED])")
    }
}
impl ApiKey {
    pub fn from_text(text: &str) -> Result<Self, CredentialError> {
        if text.is_empty()
            || text.len() > MAX_KEY_BLOB_BYTES / 2
            || !text.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
        {
            return Err(CredentialError::InvalidEncoding);
        }
        Ok(Self(Zeroizing::new(text.to_owned())))
    }
    /// Application-defined generic credential format: UTF-16LE, no terminator.
    /// Printable ASCII excludes control characters and header line breaks.
    /// Transport must still encode keys correctly when constructing URLs.
    pub fn from_utf16le(blob: &[u8]) -> Result<Self, CredentialError> {
        if blob.is_empty() || blob.len() > MAX_KEY_BLOB_BYTES || !blob.len().is_multiple_of(2) {
            return Err(CredentialError::InvalidEncoding);
        }
        let mut key = Zeroizing::new(String::with_capacity(blob.len() / 2));
        for pair in blob.as_chunks::<2>().0 {
            let character = u16::from_le_bytes([pair[0], pair[1]]);
            if !(0x21..=0x7e).contains(&character) {
                return Err(CredentialError::InvalidEncoding);
            }
            key.push(char::from(character as u8));
        }
        Ok(Self(key))
    }
    /// Restrict use to the selected provider's validated transport. Never log this.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

pub trait CredentialStore {
    fn read(&self, provider: SpeechProvider) -> Result<ApiKey, CredentialError>;
}
pub struct WindowsCredentialStore;
impl WindowsCredentialStore {
    /// Explicit Save only: verify the selected app-owned entry before reporting success.
    pub fn write_verified(
        &self,
        provider: SpeechProvider,
        key: &ApiKey,
    ) -> Result<(), CredentialError> {
        self.write(provider, key)?;
        verify_readback(self.read(provider), key)
    }
    /// Called only by an explicit setup command or UI Save action; never persists media content.
    pub fn write(&self, provider: SpeechProvider, key: &ApiKey) -> Result<(), CredentialError> {
        #[cfg(windows)]
        {
            crate::win32::credentials::write(provider, key)
        }
        #[cfg(not(windows))]
        {
            let _ = (provider, key);
            Err(CredentialError::UnsupportedPlatform)
        }
    }
}
fn verify_readback(
    readback: Result<ApiKey, CredentialError>,
    expected: &ApiKey,
) -> Result<(), CredentialError> {
    match readback {
        Ok(actual) if actual.expose() == expected.expose() => Ok(()),
        _ => Err(CredentialError::VerificationUnavailable),
    }
}
impl CredentialStore for WindowsCredentialStore {
    fn read(&self, provider: SpeechProvider) -> Result<ApiKey, CredentialError> {
        #[cfg(windows)]
        {
            crate::win32::credentials::read(provider)
        }
        #[cfg(not(windows))]
        {
            let _ = provider;
            Err(CredentialError::UnsupportedPlatform)
        }
    }
}
impl SpeechProvider {
    pub const fn credential_target(self) -> &'static str {
        match self {
            Self::OpenAi => "SubtitleNow/OpenAI/ApiKey",
            Self::Gemini => "SubtitleNow/Gemini/ApiKey",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn phase11_credential_save_requires_matching_readback_and_never_claims_missing_after_write() {
        let key = ApiKey::from_text("SYNTHETIC_EXPECTED_KEY").unwrap();
        assert_eq!(
            verify_readback(ApiKey::from_text("SYNTHETIC_EXPECTED_KEY"), &key),
            Ok(())
        );
        for result in [
            ApiKey::from_text("SYNTHETIC_DIFFERENT_KEY"),
            Err(CredentialError::Missing),
            Err(CredentialError::Unavailable),
            Err(CredentialError::InvalidEncoding),
        ] {
            assert_eq!(
                verify_readback(result, &key),
                Err(CredentialError::VerificationUnavailable)
            );
        }
        assert!(
            !CredentialError::VerificationUnavailable
                .to_string()
                .contains("SYNTHETIC")
        );
    }
    fn bytes(text: &str) -> Zeroizing<Vec<u8>> {
        Zeroizing::new(text.encode_utf16().flat_map(u16::to_le_bytes).collect())
    }
    #[test]
    fn keys_are_decoded_without_debug_or_error_disclosure() {
        let key = ApiKey::from_utf16le(&bytes("SYNTHETIC_TEST_KEY")).unwrap();
        assert_eq!(key.expose(), "SYNTHETIC_TEST_KEY");
        assert_eq!(format!("{key:?}"), "ApiKey([REDACTED])");
        assert!(
            !CredentialError::InvalidEncoding
                .to_string()
                .contains(key.expose())
        );
    }
    #[test]
    fn malformed_and_injection_payloads_are_rejected() {
        for text in ["", "with space", "line\r\nbreak", "key\0", "金鑰"] {
            assert!(ApiKey::from_utf16le(&bytes(text)).is_err());
            assert!(ApiKey::from_text(text).is_err());
        }
        assert!(ApiKey::from_text(&"A".repeat(MAX_KEY_BLOB_BYTES / 2 + 1)).is_err());
        assert!(ApiKey::from_utf16le(&[65]).is_err());
        assert!(ApiKey::from_utf16le(&vec![65; MAX_KEY_BLOB_BYTES + 2]).is_err());
    }
    #[test]
    fn named_credentials_are_separate_and_not_web_login_tokens() {
        assert_ne!(
            SpeechProvider::OpenAi.credential_target(),
            SpeechProvider::Gemini.credential_target()
        );
        assert!(
            SpeechProvider::OpenAi
                .credential_target()
                .starts_with("SubtitleNow/")
        );
        assert!(
            SpeechProvider::Gemini
                .credential_target()
                .starts_with("SubtitleNow/")
        );
    }
}
