use brail_core::error::{BrailError, BrailResult};
use uuid::Uuid;
use windows::core::{HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::FILETIME;
use windows::Win32::Security::Credentials::{
    CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE,
    CRED_TYPE_GENERIC,
};

/// Namespaces every credential this app writes so `CredEnumerateW` (used
/// during account cleanup/uninstall) can find exactly this app's entries
/// and nothing else on the user's system.
const TARGET_PREFIX: &str = "BrailRecorder_StreamKey_";

pub struct CredentialVault;

impl CredentialVault {
    /// Encrypts and stores a stream key, keyed by the owning
    /// `StreamProfile`'s UUID so renaming a profile's display name never
    /// orphans its stored key.
    pub fn store_stream_key(profile_id: Uuid, stream_key: &str) -> BrailResult<()> {
        let target_name = HSTRING::from(format!("{TARGET_PREFIX}{profile_id}"));
        // CredentialBlob is stored as raw bytes with no implied encoding;
        // UTF-16 is used here (rather than UTF-8) purely so the same bytes
        // round-trip cleanly through Windows' own credential tooling
        // (Credential Manager's UI) if the user ever inspects the entry.
        let mut blob: Vec<u16> = stream_key.encode_utf16().collect();

        let mut credential = CREDENTIALW {
            Flags: Default::default(),
            Type: CRED_TYPE_GENERIC,
            TargetName: PWSTR(target_name.as_ptr() as *mut u16),
            Comment: PWSTR::null(),
            LastWritten: FILETIME::default(),
            CredentialBlobSize: (blob.len() * 2) as u32,
            CredentialBlob: blob.as_mut_ptr() as *mut u8,
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            AttributeCount: 0,
            Attributes: std::ptr::null_mut(),
            TargetAlias: PWSTR::null(),
            UserName: PWSTR::null(),
        };

        unsafe {
            CredWriteW(&mut credential, 0)
                .map_err(|e| BrailError::SecureStorageError(format!("CredWriteW failed: {e}")))?;
        }

        Ok(())
    }

    pub fn load_stream_key(profile_id: Uuid) -> BrailResult<Option<String>> {
        let target_name = HSTRING::from(format!("{TARGET_PREFIX}{profile_id}"));

        unsafe {
            let mut credential_ptr: *mut CREDENTIALW = std::ptr::null_mut();
            let result = CredReadW(
                PCWSTR(target_name.as_ptr()),
                CRED_TYPE_GENERIC,
                None,
                &mut credential_ptr,
            );

            match result {
                Ok(()) => {
                    let credential = &*credential_ptr;
                    let bytes = std::slice::from_raw_parts(
                        credential.CredentialBlob,
                        credential.CredentialBlobSize as usize,
                    );
                    // Reinterpret the raw blob bytes as UTF-16 (see the
                    // encoding note in `store_stream_key`).
                    let utf16: Vec<u16> = bytes
                        .chunks_exact(2)
                        .map(|c| u16::from_ne_bytes([c[0], c[1]]))
                        .collect();
                    let key = String::from_utf16_lossy(&utf16);

                    CredFree(credential_ptr as *const _);
                    Ok(Some(key))
                }
                Err(e) if e.code() == windows::Win32::Foundation::ERROR_NOT_FOUND.to_hresult() => Ok(None),
                Err(e) => Err(BrailError::SecureStorageError(format!("CredReadW failed: {e}"))),
            }
        }
    }

    pub fn delete_stream_key(profile_id: Uuid) -> BrailResult<()> {
        let target_name = HSTRING::from(format!("{TARGET_PREFIX}{profile_id}"));
        unsafe {
            match CredDeleteW(PCWSTR(target_name.as_ptr()), CRED_TYPE_GENERIC, None) {
                Ok(()) => Ok(()),
                Err(e) if e.code() == windows::Win32::Foundation::ERROR_NOT_FOUND.to_hresult() => Ok(()),
                Err(e) => Err(BrailError::SecureStorageError(format!("CredDeleteW failed: {e}"))),
            }
        }
    }
}
