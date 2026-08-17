use crate::error::{AuthError, AuthErrorKind};
use crate::secret::SecretString;
use crate::secret_store::SecretStore;
use base64::{engine::general_purpose, Engine as _};
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::sync::Mutex;

const KEYRING_CHUNK_MAX_UTF16_UNITS: usize = 1_024;
#[cfg(test)]
const WINDOWS_CREDENTIAL_BLOB_MAX_BYTES: usize = 2_560;
const KEYRING_CHUNK_MAX_COUNT: usize = 64;
const KEYRING_CHUNK_MANIFEST_PREFIX: &str = "easyphy-keyring-chunks:v1:";

pub const DEFAULT_SERVICE_NAME: &str = "AIFLUXON";

#[derive(Clone, Debug, Eq, PartialEq)]
struct SecretChunkManifest {
    generation: String,
    chunk_count: usize,
    sha256: String,
}

pub struct SystemKeyringStore {
    service: String,
    backend: Mutex<Box<dyn RawSecretBackend>>,
}

trait RawSecretBackend: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>, AuthError>;
    fn set(&self, key: &str, value: &str) -> Result<(), AuthError>;
    fn delete(&self, key: &str) -> Result<(), AuthError>;
}

struct KeyringBackend {
    service: String,
}

impl RawSecretBackend for KeyringBackend {
    fn get(&self, key: &str) -> Result<Option<String>, AuthError> {
        match keyring::Entry::new(&self.service, key)
            .map_err(map_keyring)?
            .get_password()
        {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(map_keyring(error)),
        }
    }

    fn set(&self, key: &str, value: &str) -> Result<(), AuthError> {
        keyring::Entry::new(&self.service, key)
            .map_err(map_keyring)?
            .set_password(value)
            .map_err(map_keyring)
    }

    fn delete(&self, key: &str) -> Result<(), AuthError> {
        match keyring::Entry::new(&self.service, key)
            .map_err(map_keyring)?
            .delete_password()
        {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(map_keyring(error)),
        }
    }
}

#[cfg(test)]
struct MemoryRawBackend {
    values: Mutex<std::collections::HashMap<String, String>>,
}

#[cfg(test)]
impl Default for MemoryRawBackend {
    fn default() -> Self {
        Self {
            values: Mutex::new(std::collections::HashMap::new()),
        }
    }
}

#[cfg(test)]
impl RawSecretBackend for MemoryRawBackend {
    fn get(&self, key: &str) -> Result<Option<String>, AuthError> {
        Ok(self
            .values
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(key)
            .cloned())
    }

    fn set(&self, key: &str, value: &str) -> Result<(), AuthError> {
        self.values
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(key.to_string(), value.to_string());
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<(), AuthError> {
        self.values
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(key);
        Ok(())
    }
}

impl SystemKeyringStore {
    pub fn new(service_name: impl Into<String>) -> Self {
        let service = service_name.into();
        Self {
            backend: Mutex::new(Box::new(KeyringBackend {
                service: service.clone(),
            })),
            service,
        }
    }

    pub fn service_name(&self) -> &str {
        &self.service
    }

    #[cfg(test)]
    fn with_memory_backend(service_name: impl Into<String>) -> Self {
        Self {
            service: service_name.into(),
            backend: Mutex::new(Box::new(MemoryRawBackend::default())),
        }
    }

    fn backend(&self) -> std::sync::MutexGuard<'_, Box<dyn RawSecretBackend>> {
        self.backend
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn map_keyring(error: keyring::Error) -> AuthError {
    let message = error.to_string();
    let kind = if message.to_ascii_lowercase().contains("no")
        && (message.to_ascii_lowercase().contains("backend")
            || message.to_ascii_lowercase().contains("platform"))
    {
        AuthErrorKind::CredentialStoreUnavailable
    } else {
        AuthErrorKind::CredentialStore
    };
    AuthError::new(kind, format!("Credential store failed: {message}"))
}

fn secret_sha256(secret: &str) -> String {
    hex::encode(Sha256::digest(secret.as_bytes()))
}

fn secret_utf16_units(secret: &str) -> usize {
    secret.encode_utf16().count()
}

fn split_secret_chunks(secret: &str) -> Vec<String> {
    if secret.is_empty() {
        return vec![String::new()];
    }
    let mut chunks = Vec::new();
    let mut chunk_start = 0;
    let mut chunk_utf16_units = 0;
    for (index, character) in secret.char_indices() {
        let character_utf16_units = character.len_utf16();
        if chunk_utf16_units + character_utf16_units > KEYRING_CHUNK_MAX_UTF16_UNITS
            && index > chunk_start
        {
            chunks.push(secret[chunk_start..index].to_string());
            chunk_start = index;
            chunk_utf16_units = 0;
        }
        chunk_utf16_units += character_utf16_units;
    }
    chunks.push(secret[chunk_start..].to_string());
    chunks
}

fn chunk_account(account: &str, generation: &str, index: usize) -> String {
    format!("{account}:chunk:{generation}:{index}")
}

fn encode_chunk_manifest(manifest: &SecretChunkManifest) -> String {
    format!(
        "{KEYRING_CHUNK_MANIFEST_PREFIX}{}:{}:{}",
        manifest.generation, manifest.chunk_count, manifest.sha256
    )
}

fn parse_chunk_manifest(value: &str) -> Result<Option<SecretChunkManifest>, AuthError> {
    let Some(encoded) = value.strip_prefix(KEYRING_CHUNK_MANIFEST_PREFIX) else {
        return Ok(None);
    };
    let mut parts = encoded.split(':');
    let generation = parts.next().unwrap_or_default();
    let chunk_count = parts
        .next()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|count| (1..=KEYRING_CHUNK_MAX_COUNT).contains(count));
    let sha256 = parts.next().unwrap_or_default();
    if parts.next().is_some()
        || generation.is_empty()
        || !generation
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
        || chunk_count.is_none()
        || sha256.len() != 64
        || !sha256
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err(AuthError::new(
            AuthErrorKind::CredentialCorrupted,
            "Credential chunk manifest is invalid.",
        ));
    }
    Ok(Some(SecretChunkManifest {
        generation: generation.to_string(),
        chunk_count: chunk_count.unwrap_or_default(),
        sha256: sha256.to_ascii_lowercase(),
    }))
}

fn delete_manifest_chunks(
    backend: &dyn RawSecretBackend,
    account: &str,
    manifest: &SecretChunkManifest,
) -> Result<(), AuthError> {
    let mut first_error = None;
    for index in 0..manifest.chunk_count {
        if let Err(error) = backend.delete(&chunk_account(account, &manifest.generation, index)) {
            first_error.get_or_insert(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

impl SecretStore for SystemKeyringStore {
    fn get(&self, key: &str) -> Result<Option<SecretString>, AuthError> {
        let backend = self.backend();
        let Some(stored) = backend.get(key)? else {
            return Ok(None);
        };
        let Some(manifest) = parse_chunk_manifest(&stored)? else {
            return if stored.trim().is_empty() {
                Ok(None)
            } else {
                Ok(Some(SecretString::new(stored)))
            };
        };
        let mut secret = String::new();
        for index in 0..manifest.chunk_count {
            let part = backend
                .get(&chunk_account(key, &manifest.generation, index))?
                .ok_or_else(|| {
                    AuthError::new(
                        AuthErrorKind::CredentialCorrupted,
                        "Credential chunk is missing.",
                    )
                })?;
            secret.push_str(&part);
        }
        if secret_sha256(&secret) != manifest.sha256 {
            return Err(AuthError::new(
                AuthErrorKind::CredentialCorrupted,
                "Credential chunk hash mismatch.",
            ));
        }
        Ok(Some(SecretString::new(secret)))
    }

    fn set(&self, key: &str, value: &SecretString) -> Result<(), AuthError> {
        let backend = self.backend();
        let secret = value.expose();
        let previous_manifest = backend
            .get(key)?
            .as_deref()
            .and_then(|value| parse_chunk_manifest(value).ok().flatten());
        if secret_utf16_units(secret) <= KEYRING_CHUNK_MAX_UTF16_UNITS {
            backend.set(key, secret)?;
            if let Some(previous_manifest) = previous_manifest {
                let _ = delete_manifest_chunks(backend.as_ref(), key, &previous_manifest);
            }
            return Ok(());
        }
        let chunks = split_secret_chunks(secret);
        if chunks.len() > KEYRING_CHUNK_MAX_COUNT {
            return Err(AuthError::new(
                AuthErrorKind::CredentialStore,
                "Secret exceeds the credential chunk limit.",
            ));
        }
        let mut generation_bytes = [0_u8; 12];
        rand::thread_rng().fill_bytes(&mut generation_bytes);
        let manifest = SecretChunkManifest {
            generation: general_purpose::URL_SAFE_NO_PAD.encode(generation_bytes),
            chunk_count: chunks.len(),
            sha256: secret_sha256(secret),
        };
        let mut written: Vec<String> = Vec::new();
        for (index, chunk) in chunks.iter().enumerate() {
            let part = chunk_account(key, &manifest.generation, index);
            if let Err(error) = backend.set(&part, chunk) {
                for written_key in &written {
                    let _ = backend.delete(written_key);
                }
                return Err(error);
            }
            written.push(part);
        }
        if let Err(error) = backend.set(key, &encode_chunk_manifest(&manifest)) {
            for written_key in &written {
                let _ = backend.delete(written_key);
            }
            return Err(error);
        }
        if let Some(previous_manifest) = previous_manifest {
            let _ = delete_manifest_chunks(backend.as_ref(), key, &previous_manifest);
        }
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<(), AuthError> {
        let backend = self.backend();
        let manifest = backend
            .get(key)?
            .as_deref()
            .and_then(|value| parse_chunk_manifest(value).ok().flatten());
        let mut first_error = backend.delete(key).err();
        if let Some(manifest) = manifest {
            if let Err(error) = delete_manifest_chunks(backend.as_ref(), key, &manifest) {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_secret_roundtrip() {
        let store = SystemKeyringStore::with_memory_backend("test");
        store.set("k", &SecretString::new("short")).unwrap();
        assert_eq!(store.get("k").unwrap().unwrap().expose(), "short");
    }

    #[test]
    fn long_secret_roundtrip() {
        let store = SystemKeyringStore::with_memory_backend("test");
        let secret = format!(
            "{}.{}.{}",
            "a".repeat(3_200),
            "b".repeat(2_900),
            "c".repeat(2_700)
        );
        store.set("k", &SecretString::new(&secret)).unwrap();
        assert_eq!(store.get("k").unwrap().unwrap().expose(), secret);
    }

    #[test]
    fn long_unicode_secret_roundtrip() {
        let store = SystemKeyringStore::with_memory_backend("test");
        let secret = "令牌-token-".repeat(700);
        store.set("k", &SecretString::new(&secret)).unwrap();
        assert_eq!(store.get("k").unwrap().unwrap().expose(), secret);
    }

    #[test]
    fn chunk_manifest_roundtrip() {
        let secret = "令牌-token-".repeat(700);
        let chunks = split_secret_chunks(&secret);
        let manifest = SecretChunkManifest {
            generation: "generation_123".to_string(),
            chunk_count: chunks.len(),
            sha256: secret_sha256(&secret),
        };
        assert_eq!(
            parse_chunk_manifest(&encode_chunk_manifest(&manifest)).unwrap(),
            Some(manifest)
        );
    }

    #[test]
    fn chunk_hash_mismatch_fails_closed() {
        let store = SystemKeyringStore::with_memory_backend("test");
        let secret = "x".repeat(3_000);
        store.set("k", &SecretString::new(&secret)).unwrap();
        let backend = store.backend();
        let stored = backend.get("k").unwrap().unwrap();
        let manifest = parse_chunk_manifest(&stored).unwrap().unwrap();
        let first = chunk_account("k", &manifest.generation, 0);
        backend.set(&first, "tampered").unwrap();
        drop(backend);
        let error = store.get("k").unwrap_err();
        assert_eq!(error.kind(), AuthErrorKind::CredentialCorrupted);
        let _ = manifest;
    }

    #[test]
    fn missing_chunk_fails_closed() {
        let store = SystemKeyringStore::with_memory_backend("test");
        store
            .set("k", &SecretString::new(&"x".repeat(3_000)))
            .unwrap();
        let backend = store.backend();
        let stored = backend.get("k").unwrap().unwrap();
        let manifest = parse_chunk_manifest(&stored).unwrap().unwrap();
        backend
            .delete(&chunk_account("k", &manifest.generation, 0))
            .unwrap();
        drop(backend);
        assert_eq!(
            store.get("k").unwrap_err().kind(),
            AuthErrorKind::CredentialCorrupted
        );
    }

    #[test]
    fn invalid_manifest_fails_closed() {
        assert!(parse_chunk_manifest("easyphy-keyring-chunks:v1:bad:0:nope").is_err());
    }

    #[test]
    fn failed_chunk_write_cleans_partial_generation() {
        let chunks = split_secret_chunks(&"x".repeat(3_000));
        assert!(chunks.len() > 1);
        assert!(chunks
            .iter()
            .all(|chunk| secret_utf16_units(chunk) <= KEYRING_CHUNK_MAX_UTF16_UNITS));
        assert!(chunks
            .iter()
            .all(|chunk| secret_utf16_units(chunk) * 2 <= WINDOWS_CREDENTIAL_BLOB_MAX_BYTES));
    }

    #[test]
    fn replacing_long_secret_cleans_old_generation() {
        let store = SystemKeyringStore::with_memory_backend("test");
        store
            .set("k", &SecretString::new(&"a".repeat(3_000)))
            .unwrap();
        let first_manifest = {
            let backend = store.backend();
            parse_chunk_manifest(&backend.get("k").unwrap().unwrap())
                .unwrap()
                .unwrap()
        };
        store
            .set("k", &SecretString::new(&"b".repeat(3_000)))
            .unwrap();
        let backend = store.backend();
        assert!(backend
            .get(&chunk_account("k", &first_manifest.generation, 0))
            .unwrap()
            .is_none());
    }

    #[test]
    fn replacing_long_with_short_cleans_old_generation() {
        let store = SystemKeyringStore::with_memory_backend("test");
        store
            .set("k", &SecretString::new(&"a".repeat(3_000)))
            .unwrap();
        let first_manifest = {
            let backend = store.backend();
            parse_chunk_manifest(&backend.get("k").unwrap().unwrap())
                .unwrap()
                .unwrap()
        };
        store.set("k", &SecretString::new("short")).unwrap();
        let backend = store.backend();
        assert_eq!(backend.get("k").unwrap().as_deref(), Some("short"));
        assert!(backend
            .get(&chunk_account("k", &first_manifest.generation, 0))
            .unwrap()
            .is_none());
    }

    #[test]
    fn deleting_long_secret_deletes_manifest_and_chunks() {
        let store = SystemKeyringStore::with_memory_backend("test");
        store
            .set("k", &SecretString::new(&"a".repeat(3_000)))
            .unwrap();
        let manifest = {
            let backend = store.backend();
            parse_chunk_manifest(&backend.get("k").unwrap().unwrap())
                .unwrap()
                .unwrap()
        };
        store.delete("k").unwrap();
        let backend = store.backend();
        assert!(backend.get("k").unwrap().is_none());
        assert!(backend
            .get(&chunk_account("k", &manifest.generation, 0))
            .unwrap()
            .is_none());
    }

    #[test]
    fn chunk_count_limit_is_enforced() {
        let huge = "a".repeat(KEYRING_CHUNK_MAX_UTF16_UNITS * (KEYRING_CHUNK_MAX_COUNT + 2));
        let chunks = split_secret_chunks(&huge);
        assert!(chunks.len() > KEYRING_CHUNK_MAX_COUNT);
        let store = SystemKeyringStore::with_memory_backend("test");
        assert_eq!(
            store.set("k", &SecretString::new(huge)).unwrap_err().kind(),
            AuthErrorKind::CredentialStore
        );
    }
}
