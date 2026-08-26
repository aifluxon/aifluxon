use crate::error::{AuthError, AuthErrorKind};
use crate::secret::SecretString;
use crate::secret_store::SecretStore;
use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    Key, XChaCha20Poly1305, XNonce,
};
use rand::RngCore;
use serde_json::{json, Map, Value};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use zeroize::Zeroize;

const MAGIC: &[u8; 8] = b"AFLXCRD1";
const FORMAT_VERSION: u16 = 1;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
const KEY_LEN: usize = 32;
const M_COST: u32 = 64 * 1024;
const T_COST: u32 = 3;
const P_COST: u32 = 1;

pub struct EncryptedFileSecretStore {
    path: PathBuf,
    state: Mutex<VaultState>,
}

struct VaultState {
    key: Option<[u8; KEY_LEN]>,
    salt: Option<[u8; SALT_LEN]>,
    m_cost: u32,
    t_cost: u32,
    p_cost: u32,
    values: Option<Map<String, Value>>,
}

impl EncryptedFileSecretStore {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: expand_path(path.as_ref()),
            state: Mutex::new(VaultState {
                key: None,
                salt: None,
                m_cost: M_COST,
                t_cost: T_COST,
                p_cost: P_COST,
                values: None,
            }),
        }
    }

    pub fn default_path() -> PathBuf {
        directories::ProjectDirs::from("", "", "AIFLUXON")
            .map(|dirs| dirs.data_dir().join("credentials.vault"))
            .unwrap_or_else(|| PathBuf::from("credentials.vault"))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn is_unlocked(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .key
            .is_some()
    }

    pub fn unlock(&self, password: &SecretString) -> Result<(), AuthError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.path.exists() {
            let bytes = fs::read(&self.path).map_err(store_io)?;
            let parsed = parse_vault(&bytes)?;
            let key = derive_key(
                password.expose().as_bytes(),
                &parsed.salt,
                parsed.m_cost,
                parsed.t_cost,
                parsed.p_cost,
            )?;
            let cipher = XChaCha20Poly1305::new(Key::from_slice(&key));
            let plaintext = cipher
                .decrypt(
                    XNonce::from_slice(&parsed.nonce),
                    parsed.ciphertext.as_ref(),
                )
                .map_err(|_| {
                    AuthError::new(
                        AuthErrorKind::CredentialCorrupted,
                        "Encrypted credential vault could not be unlocked.",
                    )
                })?;
            let values: Value = serde_json::from_slice(&plaintext).map_err(|_| {
                AuthError::new(
                    AuthErrorKind::CredentialCorrupted,
                    "Encrypted credential vault is corrupted.",
                )
            })?;
            let object = values.as_object().cloned().ok_or_else(|| {
                AuthError::new(
                    AuthErrorKind::CredentialCorrupted,
                    "Encrypted credential vault is corrupted.",
                )
            })?;
            state.key = Some(key);
            state.salt = Some(parsed.salt);
            state.m_cost = parsed.m_cost;
            state.t_cost = parsed.t_cost;
            state.p_cost = parsed.p_cost;
            state.values = Some(object);
            Ok(())
        } else {
            let mut salt = [0_u8; SALT_LEN];
            rand::thread_rng().fill_bytes(&mut salt);
            let key = derive_key(password.expose().as_bytes(), &salt, M_COST, T_COST, P_COST)?;
            state.key = Some(key);
            state.salt = Some(salt);
            state.m_cost = M_COST;
            state.t_cost = T_COST;
            state.p_cost = P_COST;
            state.values = Some(Map::new());
            drop(state);
            self.persist()
        }
    }

    pub fn lock(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(key) = state.key.as_mut() {
            key.zeroize();
        }
        state.key = None;
        state.salt = None;
        state.values = None;
    }

    fn persist(&self) -> Result<(), AuthError> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let key = state.key.ok_or_else(locked)?;
        let salt = state.salt.ok_or_else(locked)?;
        let values = Value::Object(state.values.clone().unwrap_or_default());
        let plaintext = serde_json::to_vec(&values)
            .map_err(|error| AuthError::new(AuthErrorKind::CredentialStore, error.to_string()))?;
        let mut nonce = [0_u8; NONCE_LEN];
        rand::thread_rng().fill_bytes(&mut nonce);
        let cipher = XChaCha20Poly1305::new(Key::from_slice(&key));
        let ciphertext = cipher
            .encrypt(XNonce::from_slice(&nonce), plaintext.as_ref())
            .map_err(|_| {
                AuthError::new(
                    AuthErrorKind::CredentialStore,
                    "Encrypted credential vault could not be written.",
                )
            })?;
        let path = self.path.clone();
        let m_cost = state.m_cost;
        let t_cost = state.t_cost;
        let p_cost = state.p_cost;
        drop(state);
        write_vault(&path, &salt, m_cost, t_cost, p_cost, &nonce, &ciphertext)
    }
}

impl std::fmt::Debug for EncryptedFileSecretStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EncryptedFileSecretStore")
            .field("path", &self.path)
            .field("unlocked", &self.is_unlocked())
            .finish()
    }
}

fn locked() -> AuthError {
    AuthError::new(
        AuthErrorKind::CredentialStoreLocked,
        "Encrypted credential vault is locked.",
    )
}

fn store_io(error: std::io::Error) -> AuthError {
    AuthError::new(
        AuthErrorKind::CredentialStore,
        format!("Credential vault I/O failed: {error}"),
    )
}

fn expand_path(path: &Path) -> PathBuf {
    if let Some(stripped) = path.to_str().and_then(|value| value.strip_prefix("~/")) {
        if let Some(home) = directories::BaseDirs::new() {
            return home.home_dir().join(stripped);
        }
    }
    path.to_path_buf()
}

fn derive_key(
    password: &[u8],
    salt: &[u8],
    m_cost: u32,
    t_cost: u32,
    p_cost: u32,
) -> Result<[u8; KEY_LEN], AuthError> {
    let params = Params::new(m_cost, t_cost, p_cost, Some(KEY_LEN)).map_err(|_| {
        AuthError::new(
            AuthErrorKind::Configuration,
            "Encrypted credential vault KDF parameters are invalid.",
        )
    })?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0_u8; KEY_LEN];
    argon2
        .hash_password_into(password, salt, &mut key)
        .map_err(|_| {
            AuthError::new(
                AuthErrorKind::CredentialStore,
                "Encrypted credential vault key derivation failed.",
            )
        })?;
    Ok(key)
}

struct ParsedVault {
    salt: [u8; SALT_LEN],
    m_cost: u32,
    t_cost: u32,
    p_cost: u32,
    nonce: [u8; NONCE_LEN],
    ciphertext: Vec<u8>,
}

fn parse_vault(bytes: &[u8]) -> Result<ParsedVault, AuthError> {
    let header_len = 8 + 2 + SALT_LEN + 12 + NONCE_LEN;
    if bytes.len() < header_len || bytes.get(..8) != Some(MAGIC.as_slice()) {
        return Err(AuthError::new(
            AuthErrorKind::CredentialCorrupted,
            "Encrypted credential vault header is invalid.",
        ));
    }
    let version = u16::from_le_bytes(bytes[8..10].try_into().unwrap());
    if version != FORMAT_VERSION {
        return Err(AuthError::new(
            AuthErrorKind::CredentialCorrupted,
            "Encrypted credential vault version is unsupported.",
        ));
    }
    let mut salt = [0_u8; SALT_LEN];
    salt.copy_from_slice(&bytes[10..26]);
    let m_cost = u32::from_le_bytes(bytes[26..30].try_into().unwrap());
    let t_cost = u32::from_le_bytes(bytes[30..34].try_into().unwrap());
    let p_cost = u32::from_le_bytes(bytes[34..38].try_into().unwrap());
    let mut nonce = [0_u8; NONCE_LEN];
    nonce.copy_from_slice(&bytes[38..62]);
    Ok(ParsedVault {
        salt,
        m_cost,
        t_cost,
        p_cost,
        nonce,
        ciphertext: bytes[62..].to_vec(),
    })
}

fn encode_vault(
    salt: &[u8; SALT_LEN],
    m_cost: u32,
    t_cost: u32,
    p_cost: u32,
    nonce: &[u8; NONCE_LEN],
    ciphertext: &[u8],
) -> Vec<u8> {
    let mut out = Vec::with_capacity(62 + ciphertext.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    out.extend_from_slice(salt);
    out.extend_from_slice(&m_cost.to_le_bytes());
    out.extend_from_slice(&t_cost.to_le_bytes());
    out.extend_from_slice(&p_cost.to_le_bytes());
    out.extend_from_slice(nonce);
    out.extend_from_slice(ciphertext);
    out
}

fn write_vault(
    path: &Path,
    salt: &[u8; SALT_LEN],
    m_cost: u32,
    t_cost: u32,
    p_cost: u32,
    nonce: &[u8; NONCE_LEN],
    ciphertext: &[u8],
) -> Result<(), AuthError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(store_io)?;
    }
    let encoded = encode_vault(salt, m_cost, t_cost, p_cost, nonce, ciphertext);
    let tmp = path.with_extension("vault.tmp");
    {
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp)
            .map_err(store_io)?;
        file.write_all(&encoded).map_err(store_io)?;
        file.sync_all().map_err(store_io)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600)).map_err(|error| {
            let _ = fs::remove_file(&tmp);
            store_io(error)
        })?;
    }
    fs::rename(&tmp, path).map_err(|error| {
        let _ = fs::remove_file(&tmp);
        store_io(error)
    })
}

impl SecretStore for EncryptedFileSecretStore {
    fn get(&self, key: &str) -> Result<Option<SecretString>, AuthError> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.key.is_none() {
            return Err(locked());
        }
        Ok(state
            .values
            .as_ref()
            .and_then(|values| values.get(key))
            .and_then(Value::as_str)
            .map(SecretString::new))
    }

    fn set(&self, key: &str, value: &SecretString) -> Result<(), AuthError> {
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if state.key.is_none() {
                return Err(locked());
            }
            state
                .values
                .get_or_insert_with(Map::new)
                .insert(key.to_string(), json!(value.expose()));
        }
        self.persist()
    }

    fn delete(&self, key: &str) -> Result<(), AuthError> {
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if state.key.is_none() {
                return Err(locked());
            }
            if let Some(values) = state.values.as_mut() {
                values.remove(key);
            }
        }
        self.persist()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn encrypted_vault_create_and_unlock() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("credentials.vault");
        let store = EncryptedFileSecretStore::new(&path);
        store.unlock(&SecretString::new("pw-one")).unwrap();
        store.set("token", &SecretString::new("secret-a")).unwrap();
        store.lock();
        store.unlock(&SecretString::new("pw-one")).unwrap();
        assert_eq!(store.get("token").unwrap().unwrap().expose(), "secret-a");
    }

    #[test]
    fn encrypted_vault_wrong_password_rejected() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("credentials.vault");
        let store = EncryptedFileSecretStore::new(&path);
        store.unlock(&SecretString::new("pw-one")).unwrap();
        store.lock();
        let error = store.unlock(&SecretString::new("pw-two")).unwrap_err();
        assert_eq!(error.kind(), AuthErrorKind::CredentialCorrupted);
    }

    #[test]
    fn tampered_ciphertext_rejected() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("credentials.vault");
        let store = EncryptedFileSecretStore::new(&path);
        store.unlock(&SecretString::new("pw-one")).unwrap();
        store.set("token", &SecretString::new("secret-a")).unwrap();
        store.lock();
        let mut bytes = fs::read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0x5a;
        fs::write(&path, bytes).unwrap();
        let error = store.unlock(&SecretString::new("pw-one")).unwrap_err();
        assert_eq!(error.kind(), AuthErrorKind::CredentialCorrupted);
    }

    #[test]
    fn tampered_header_rejected() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("credentials.vault");
        let store = EncryptedFileSecretStore::new(&path);
        store.unlock(&SecretString::new("pw-one")).unwrap();
        store.lock();
        let mut bytes = fs::read(&path).unwrap();
        bytes[0] = b'Z';
        fs::write(&path, bytes).unwrap();
        assert_eq!(
            store
                .unlock(&SecretString::new("pw-one"))
                .unwrap_err()
                .kind(),
            AuthErrorKind::CredentialCorrupted
        );
    }

    #[test]
    fn vault_file_does_not_contain_plaintext_token() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("credentials.vault");
        let store = EncryptedFileSecretStore::new(&path);
        store.unlock(&SecretString::new("pw-one")).unwrap();
        store
            .set("token", &SecretString::new("unique-access-token-xyz"))
            .unwrap();
        let bytes = fs::read(&path).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains("unique-access-token-xyz"));
        assert!(!text.contains("user@example.com"));
    }

    #[test]
    fn locked_store_rejects_reads() {
        let dir = tempdir().unwrap();
        let store = EncryptedFileSecretStore::new(dir.path().join("credentials.vault"));
        store.unlock(&SecretString::new("pw-one")).unwrap();
        store.lock();
        assert_eq!(
            store.get("token").unwrap_err().kind(),
            AuthErrorKind::CredentialStoreLocked
        );
    }

    #[test]
    fn random_salt_per_vault() {
        let dir = tempdir().unwrap();
        let a = EncryptedFileSecretStore::new(dir.path().join("a.vault"));
        let b = EncryptedFileSecretStore::new(dir.path().join("b.vault"));
        a.unlock(&SecretString::new("pw")).unwrap();
        b.unlock(&SecretString::new("pw")).unwrap();
        let left = fs::read(a.path()).unwrap();
        let right = fs::read(b.path()).unwrap();
        assert_ne!(&left[10..26], &right[10..26]);
    }

    #[cfg(unix)]
    #[test]
    fn encrypted_vault_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir().unwrap();
        let path = dir.path().join("credentials.vault");
        let store = EncryptedFileSecretStore::new(&path);
        store.unlock(&SecretString::new("pw")).unwrap();

        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
