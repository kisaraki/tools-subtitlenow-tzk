//! Narrow app-owned WinCred boundary. Never enumerates other applications' credentials.
use crate::ai::{
    credentials::{ApiKey, CredentialError, MAX_KEY_BLOB_BYTES},
    provider::SpeechProvider,
};
use std::ptr;
use windows::{
    Win32::{
        Foundation::ERROR_NOT_FOUND,
        Security::Credentials::{CRED_TYPE_GENERIC, CREDENTIALW, CredFree, CredReadW},
    },
    core::{HRESULT, PCWSTR},
};
use zeroize::Zeroize;

pub(crate) fn write(provider: SpeechProvider, key: &ApiKey) -> Result<(), CredentialError> {
    write_target(provider.credential_target(), key)
}
fn write_target(target_name: &str, key: &ApiKey) -> Result<(), CredentialError> {
    use windows::{
        Win32::Security::Credentials::{CRED_PERSIST_LOCAL_MACHINE, CredWriteW},
        core::PWSTR,
    };
    let mut target: Vec<u16> = target_name.encode_utf16().chain(Some(0)).collect();
    // ApiKey is ASCII, so exact capacity avoids reallocating an unwiped old blob.
    let mut blob = zeroize::Zeroizing::new(Vec::with_capacity(key.expose().len() * 2));
    blob.extend(key.expose().encode_utf16().flat_map(u16::to_le_bytes));
    let credential = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(target.as_mut_ptr()),
        CredentialBlobSize: blob.len() as u32,
        CredentialBlob: blob.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        ..Default::default()
    };
    // SAFETY: terminated target and initialized UTF-16LE blob stay alive for this
    // synchronous call; lengths are validated by ApiKey. WinCred copies the data.
    // No pointers or ownership escape; blob is wiped after success or failure.
    unsafe { CredWriteW(&credential, 0) }.map_err(|_| CredentialError::Unavailable)
}

struct OwnedCredential(*mut CREDENTIALW);
impl Drop for OwnedCredential {
    fn drop(&mut self) {
        // SAFETY: created only from a successful CredReadW allocation. The single
        // block and its blob remain owned by this guard, with no live borrowed slices
        // when Drop runs. Validate size/null before wiping; CredFree is called once.
        unsafe {
            let credential = &*self.0;
            let size = credential.CredentialBlobSize as usize;
            if size > 0 && size <= MAX_KEY_BLOB_BYTES && !credential.CredentialBlob.is_null() {
                std::slice::from_raw_parts_mut(credential.CredentialBlob, size).zeroize();
            }
            CredFree(self.0.cast());
        }
    }
}

pub(crate) fn read(provider: SpeechProvider) -> Result<ApiKey, CredentialError> {
    read_target(provider.credential_target())
}
fn read_target(target_name: &str) -> Result<ApiKey, CredentialError> {
    let target: Vec<u16> = target_name.encode_utf16().chain(Some(0)).collect();
    let mut pointer = ptr::null_mut();
    // SAFETY: target is terminated UTF-16 and lives through this synchronous call;
    // pointer is valid writable output storage. Flags are zero. Success transfers
    // ownership of the WinCred allocation to the guard below on this same thread.
    unsafe {
        CredReadW(
            PCWSTR(target.as_ptr()),
            CRED_TYPE_GENERIC,
            Some(0),
            &mut pointer,
        )
    }
    .map_err(|error| {
        if error.code() == HRESULT::from_win32(ERROR_NOT_FOUND.0) {
            CredentialError::Missing
        } else {
            CredentialError::Unavailable
        }
    })?;
    if pointer.is_null() {
        return Err(CredentialError::Unavailable);
    }
    let credential = OwnedCredential(pointer);
    // SAFETY: CredReadW guarantees an initialized CREDENTIALW plus its blob in one
    // allocation. Guard retains ownership for the whole borrow. The documented blob
    // length is validated before constructing a slice; no pointer escapes this call.
    let blob = unsafe {
        let value = &*credential.0;
        let size = value.CredentialBlobSize as usize;
        if size == 0 || size > MAX_KEY_BLOB_BYTES || value.CredentialBlob.is_null() {
            return Err(CredentialError::InvalidEncoding);
        }
        std::slice::from_raw_parts(value.CredentialBlob, size)
    };
    ApiKey::from_utf16le(blob)
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Security::Credentials::CredDeleteW;
    struct TestCredential(String);
    impl Drop for TestCredential {
        fn drop(&mut self) {
            let target: Vec<u16> = self.0.encode_utf16().chain(Some(0)).collect();
            // SAFETY: terminated UTF-16 target lives for the synchronous call.
            // Only this test's unique app-owned credential is deleted, including on panic.
            let _ = unsafe { CredDeleteW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, Some(0)) };
        }
    }
    #[test]
    fn native_app_owned_test_credential_round_trips_and_is_removed() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let target = format!("SubtitleNow/Test/{}/{nonce}", std::process::id());
        assert!(matches!(
            read_target(&target),
            Err(CredentialError::Missing)
        ));
        let owned = TestCredential(target);
        write_target(&owned.0, &ApiKey::from_text("SYNTHETIC_KEY_A").unwrap()).unwrap();
        assert_eq!(read_target(&owned.0).unwrap().expose(), "SYNTHETIC_KEY_A");
        write_target(&owned.0, &ApiKey::from_text("SYNTHETIC_KEY_B").unwrap()).unwrap();
        assert_eq!(read_target(&owned.0).unwrap().expose(), "SYNTHETIC_KEY_B");
        let target = owned.0.clone();
        drop(owned);
        assert!(matches!(
            read_target(&target),
            Err(CredentialError::Missing)
        ));
    }
}
