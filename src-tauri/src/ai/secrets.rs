use std::collections::HashMap;
use std::fmt;
use std::sync::{Mutex, OnceLock};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{LocalFree, HLOCAL};
use windows::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
};

const KEYRING_SERVICE: &str = "fern-ai";
const DPAPI_ENTROPY: &[u8] = b"fern-ai-dpapi-v1::credential-vault";

static SESSION_STORE: OnceLock<Mutex<HashMap<String, Vec<u8>>>> = OnceLock::new();

fn session_store() -> &'static Mutex<HashMap<String, Vec<u8>>> {
    SESSION_STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn zeroize_bytes(bytes: &mut [u8]) {
    for byte in bytes.iter_mut() {
        // SAFETY: `byte` is a valid, aligned, writable reference to a `u8` within the slice.
        unsafe {
            std::ptr::write_volatile(byte, 0);
        }
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

pub struct SecretString(Vec<u8>);

impl SecretString {
    pub fn new(s: String) -> Self {
        Self(s.into_bytes())
    }

    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.0).unwrap_or("")
    }
}

impl Drop for SecretString {
    fn drop(&mut self) {
        zeroize_bytes(&mut self.0);
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

pub fn dpapi_encrypt(plain: &[u8], entropy: &[u8]) -> Result<Vec<u8>, String> {
    let cb_data = u32::try_from(plain.len()).map_err(|_| "Input exceeds DPAPI size limit".to_string())?;
    let cb_entropy =
        u32::try_from(entropy.len()).map_err(|_| "Entropy exceeds DPAPI size limit".to_string())?;

    let in_blob = CRYPT_INTEGER_BLOB {
        cbData: cb_data,
        pbData: plain.as_ptr() as *mut u8,
    };
    let entropy_blob = CRYPT_INTEGER_BLOB {
        cbData: cb_entropy,
        pbData: entropy.as_ptr() as *mut u8,
    };
    let entropy_ptr = if entropy.is_empty() {
        None
    } else {
        Some(&entropy_blob as *const CRYPT_INTEGER_BLOB)
    };

    let mut out_blob = CRYPT_INTEGER_BLOB::default();
    // SAFETY: `in_blob` and optional `entropy_blob` point to valid read-only buffers for the
    // duration of the Win32 call; `out_blob` is allocated by DPAPI and freed below via `LocalFree`.
    unsafe {
        CryptProtectData(
            &in_blob,
            PCWSTR::null(),
            entropy_ptr,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out_blob,
        )
        .map_err(|e| format!("DPAPI encrypt failed: {}", e))?;

        let result = if out_blob.cbData == 0 || out_blob.pbData.is_null() {
            Vec::new()
        } else {
            std::slice::from_raw_parts(out_blob.pbData, out_blob.cbData as usize).to_vec()
        };

        if !out_blob.pbData.is_null() {
            let _ = LocalFree(Some(HLOCAL(out_blob.pbData.cast())));
        }

        Ok(result)
    }
}

pub fn dpapi_decrypt(cipher: &[u8], entropy: &[u8]) -> Result<Vec<u8>, String> {
    if cipher.is_empty() {
        return Err("DPAPI decrypt failed: empty ciphertext".to_string());
    }
    let cb_data =
        u32::try_from(cipher.len()).map_err(|_| "Ciphertext exceeds DPAPI size limit".to_string())?;
    let cb_entropy =
        u32::try_from(entropy.len()).map_err(|_| "Entropy exceeds DPAPI size limit".to_string())?;

    let in_blob = CRYPT_INTEGER_BLOB {
        cbData: cb_data,
        pbData: cipher.as_ptr() as *mut u8,
    };
    let entropy_blob = CRYPT_INTEGER_BLOB {
        cbData: cb_entropy,
        pbData: entropy.as_ptr() as *mut u8,
    };
    let entropy_ptr = if entropy.is_empty() {
        None
    } else {
        Some(&entropy_blob as *const CRYPT_INTEGER_BLOB)
    };

    let mut out_blob = CRYPT_INTEGER_BLOB::default();
    // SAFETY: `in_blob` and optional `entropy_blob` point to valid read-only buffers for the
    // duration of the Win32 call; `out_blob` is allocated by DPAPI, wiped, and freed via `LocalFree`.
    unsafe {
        CryptUnprotectData(
            &in_blob,
            None,
            entropy_ptr,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out_blob,
        )
        .map_err(|e| format!("DPAPI decrypt failed: {}", e))?;

        let result = if out_blob.cbData == 0 || out_blob.pbData.is_null() {
            Vec::new()
        } else {
            let slice = std::slice::from_raw_parts_mut(out_blob.pbData, out_blob.cbData as usize);
            let copied = slice.to_vec();
            zeroize_bytes(slice);
            copied
        };

        if !out_blob.pbData.is_null() {
            let _ = LocalFree(Some(HLOCAL(out_blob.pbData.cast())));
        }

        Ok(result)
    }
}

fn keyring_entry(provider: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYRING_SERVICE, provider)
        .map_err(|e| format!("Keyring entry error: {}", e))
}

fn delete_keyring_entry(provider: &str) -> Result<(), String> {
    let entry = keyring_entry(provider)?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("Failed to delete keyring credential: {}", e)),
    }
}

fn decrypt_to_secret(cipher: &[u8]) -> Option<SecretString> {
    let plain = dpapi_decrypt(cipher, DPAPI_ENTROPY).ok()?;
    match String::from_utf8(plain) {
        Ok(s) => Some(SecretString::new(s)),
        Err(err) => {
            let mut bytes = err.into_bytes();
            zeroize_bytes(&mut bytes);
            None
        }
    }
}

pub fn set_key(provider: &str, key: &str, session_only: bool) -> Result<(), String> {
    let provider = provider.trim();
    if provider.is_empty() {
        return Err("Provider name cannot be empty".to_string());
    }
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return Err("API key cannot be empty".to_string());
    }

    let encrypted = dpapi_encrypt(trimmed.as_bytes(), DPAPI_ENTROPY)?;

    if session_only {
        {
            let mut guard = session_store().lock().unwrap_or_else(|e| e.into_inner());
            guard.insert(provider.to_string(), encrypted);
        }
        delete_keyring_entry(provider)?;
        Ok(())
    } else {
        let encoded = STANDARD.encode(&encrypted);
        let entry = keyring_entry(provider)?;
        entry
            .set_password(&encoded)
            .map_err(|e| format!("Failed to store keyring credential: {}", e))?;
        let mut guard = session_store().lock().unwrap_or_else(|e| e.into_inner());
        guard.remove(provider);
        Ok(())
    }
}

pub fn get_key(provider: &str) -> Option<SecretString> {
    let provider = provider.trim();
    if provider.is_empty() {
        return None;
    }

    let session_cipher = {
        let guard = session_store().lock().unwrap_or_else(|e| e.into_inner());
        guard.get(provider).cloned()
    };
    if let Some(cipher) = session_cipher {
        return decrypt_to_secret(&cipher);
    }

    let encoded = keyring_entry(provider).ok()?.get_password().ok()?;
    let cipher = STANDARD.decode(encoded).ok()?;
    decrypt_to_secret(&cipher)
}

pub fn has_key(provider: &str) -> bool {
    get_key(provider).is_some()
}

pub fn delete_key(provider: &str) -> Result<(), String> {
    let provider = provider.trim();
    if provider.is_empty() {
        return Ok(());
    }

    {
        let mut guard = session_store().lock().unwrap_or_else(|e| e.into_inner());
        guard.remove(provider);
    }
    delete_keyring_entry(provider)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dpapi_roundtrip_and_wrong_entropy_rejected() {
        let secret = b"sk-test-dpapi-roundtrip-1234567890";
        let entropy = b"fern-test-entropy-a";
        let wrong_entropy = b"fern-test-entropy-b";

        let encrypted = dpapi_encrypt(secret, entropy).expect("dpapi_encrypt should succeed");
        assert_ne!(encrypted.as_slice(), secret);

        let decrypted = dpapi_decrypt(&encrypted, entropy).expect("dpapi_decrypt should succeed");
        assert_eq!(decrypted.as_slice(), secret);

        let wrong = dpapi_decrypt(&encrypted, wrong_entropy);
        assert!(wrong.is_err(), "decryption with wrong entropy must fail");
    }

    #[test]
    fn secret_string_redacts_debug_and_session_store_works() {
        let provider = "__fern_test_session_provider__";
        let _ = delete_key(provider);

        let raw = "sk-ant-secret-value-xyz".to_string();
        let secret = SecretString::new(raw.clone());
        assert_eq!(secret.as_str(), raw.as_str());
        assert_eq!(format!("{:?}", secret), "[REDACTED]");
        assert!(!format!("{:?}", secret).contains("sk-ant"));

        set_key(provider, "sk-session-only-98765", true).expect("set_key session_only should succeed");
        assert!(has_key(provider));

        let loaded = get_key(provider).expect("get_key should return stored session key");
        assert_eq!(loaded.as_str(), "sk-session-only-98765");
        assert_eq!(format!("{:?}", loaded), "[REDACTED]");

        delete_key(provider).expect("delete_key should succeed");
        assert!(!has_key(provider));
        assert!(get_key(provider).is_none());
    }

    #[test]
    fn rejects_empty_key() {
        let provider = "__fern_test_empty_provider__";
        assert!(set_key(provider, "", true).is_err());
        assert!(set_key(provider, "   \t\n  ", true).is_err());
        assert!(set_key(provider, "", false).is_err());
        assert!(set_key(provider, "   ", false).is_err());
        assert!(!has_key(provider));
    }
}
