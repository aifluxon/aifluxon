use crate::codex::token::{
    access_token_expiry, id_token_auth_claim, id_token_email, CodexCredentialMetadata,
    CodexCredentials,
};
use crate::error::{AuthError, AuthErrorKind};
use crate::secret::SecretString;
use crate::secret_store::SecretStore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub const ACCOUNT_INDEX_ACCOUNT: &str = "codex-oauth:account-index";
pub const LEGACY_ACCESS_TOKEN_ACCOUNT: &str = "codex-oauth:access-token";
pub const LEGACY_REFRESH_TOKEN_ACCOUNT: &str = "codex-oauth:refresh-token";
pub const LEGACY_ID_TOKEN_ACCOUNT: &str = "codex-oauth:id-token";
pub const LEGACY_METADATA_ACCOUNT: &str = "codex-oauth:metadata";

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CodexAccountRecord {
    pub account_id: String,
    pub account_email: Option<String>,
}

#[derive(Default)]
pub(crate) struct CodexRuntimeCredentialCache {
    account_records: Option<Vec<CodexAccountRecord>>,
    credentials: HashMap<String, CodexCredentials>,
}

impl CodexRuntimeCredentialCache {
    fn account_records(&self) -> Option<Vec<CodexAccountRecord>> {
        self.account_records.clone()
    }

    fn cache_account_records(&mut self, records: &[CodexAccountRecord]) {
        self.account_records = Some(records.to_vec());
    }

    fn credentials(&self, account_id: &str) -> Option<CodexCredentials> {
        self.credentials.get(account_id).cloned()
    }

    fn cache_credentials(&mut self, account_id: &str, credentials: &CodexCredentials) {
        self.credentials
            .insert(account_id.to_string(), credentials.clone());
    }

    fn remove_credentials(&mut self, account_id: &str) {
        self.credentials.remove(account_id);
    }
}

pub(crate) struct CodexCredentialStore {
    secrets: Arc<dyn SecretStore>,
    cache: Mutex<CodexRuntimeCredentialCache>,
    migrated: Mutex<bool>,
}

impl CodexCredentialStore {
    pub(crate) fn new(secrets: Arc<dyn SecretStore>) -> Self {
        Self {
            secrets,
            cache: Mutex::new(CodexRuntimeCredentialCache::default()),
            migrated: Mutex::new(false),
        }
    }

    fn cache(&self) -> std::sync::MutexGuard<'_, CodexRuntimeCredentialCache> {
        self.cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn account_secret_account(account_id: &str, kind: &str) -> String {
        let fingerprint = hex::encode(Sha256::digest(account_id.as_bytes()));
        format!("codex-oauth:account:{}:{kind}", &fingerprint[..32])
    }

    fn get_text(&self, key: &str) -> Result<Option<String>, AuthError> {
        Ok(self
            .secrets
            .get(key)?
            .map(|value| value.expose().to_string())
            .filter(|value| !value.trim().is_empty()))
    }

    fn set_text(&self, key: &str, value: &str) -> Result<(), AuthError> {
        self.secrets.set(key, &SecretString::new(value))
    }

    pub(crate) fn read_account_index(&self) -> Result<Vec<CodexAccountRecord>, AuthError> {
        if let Some(records) = self.cache().account_records() {
            return Ok(records);
        }
        let Some(value) = self.get_text(ACCOUNT_INDEX_ACCOUNT)? else {
            self.cache().cache_account_records(&[]);
            return Ok(Vec::new());
        };
        let records: Vec<CodexAccountRecord> = serde_json::from_str(&value).map_err(|_| {
            AuthError::new(
                AuthErrorKind::CredentialCorrupted,
                "Codex account index is invalid.",
            )
        })?;
        let mut normalized = Vec::new();
        for mut record in records {
            record.account_id = record.account_id.trim().to_string();
            record.account_email = record
                .account_email
                .as_deref()
                .map(str::trim)
                .filter(|email| !email.is_empty())
                .map(str::to_string);
            if !record.account_id.is_empty()
                && !normalized
                    .iter()
                    .any(|current: &CodexAccountRecord| current.account_id == record.account_id)
            {
                normalized.push(record);
            }
        }
        self.cache().cache_account_records(&normalized);
        Ok(normalized)
    }

    pub(crate) fn write_account_index(
        &self,
        records: &[CodexAccountRecord],
    ) -> Result<(), AuthError> {
        if records.is_empty() {
            self.secrets.delete(ACCOUNT_INDEX_ACCOUNT)?;
        } else {
            let encoded = serde_json::to_string(records).map_err(|error| {
                AuthError::new(AuthErrorKind::CredentialStore, error.to_string())
            })?;
            self.set_text(ACCOUNT_INDEX_ACCOUNT, &encoded)?;
        }
        self.cache().cache_account_records(records);
        Ok(())
    }

    pub(crate) fn upsert_account_record(
        &self,
        record: CodexAccountRecord,
    ) -> Result<(), AuthError> {
        let mut records = self.read_account_index()?;
        if let Some(current) = records
            .iter_mut()
            .find(|current| current.account_id == record.account_id)
        {
            *current = record;
        } else {
            records.push(record);
        }
        self.write_account_index(&records)
    }

    pub(crate) fn remove_account_record(&self, account_id: &str) -> Result<(), AuthError> {
        let mut records = self.read_account_index()?;
        records.retain(|record| record.account_id != account_id);
        self.write_account_index(&records)
    }

    fn read_metadata(&self, account_id: &str) -> CodexCredentialMetadata {
        self.get_text(&Self::account_secret_account(account_id, "metadata"))
            .ok()
            .flatten()
            .and_then(|value| serde_json::from_str(&value).ok())
            .unwrap_or_default()
    }

    pub(crate) fn read_scoped_credentials(
        &self,
        account_id: &str,
    ) -> Result<CodexCredentials, AuthError> {
        let missing = || {
            AuthError::new(
                AuthErrorKind::AuthenticationRequired,
                "The selected Codex account is not authorized.",
            )
        };
        Ok(CodexCredentials {
            access_token: SecretString::new(
                self.get_text(&Self::account_secret_account(account_id, "access-token"))?
                    .ok_or_else(missing)?,
            ),
            refresh_token: SecretString::new(
                self.get_text(&Self::account_secret_account(account_id, "refresh-token"))?
                    .ok_or_else(missing)?,
            ),
            id_token: SecretString::new(
                self.get_text(&Self::account_secret_account(account_id, "id-token"))?
                    .ok_or_else(missing)?,
            ),
            last_refresh_at: self.read_metadata(account_id).last_refresh_at,
        })
    }

    pub(crate) fn store_scoped_credentials(
        &self,
        account_id: &str,
        credentials: &CodexCredentials,
    ) -> Result<(), AuthError> {
        self.set_text(
            &Self::account_secret_account(account_id, "access-token"),
            credentials.access_token.expose(),
        )?;
        self.set_text(
            &Self::account_secret_account(account_id, "refresh-token"),
            credentials.refresh_token.expose(),
        )?;
        self.set_text(
            &Self::account_secret_account(account_id, "id-token"),
            credentials.id_token.expose(),
        )?;
        self.set_text(
            &Self::account_secret_account(account_id, "metadata"),
            &serde_json::to_string(&CodexCredentialMetadata {
                last_refresh_at: credentials.last_refresh_at,
            })
            .map_err(|error| AuthError::new(AuthErrorKind::CredentialStore, error.to_string()))?,
        )
    }

    pub(crate) fn delete_account_credentials(&self, account_id: &str) -> Result<(), AuthError> {
        self.cache().remove_credentials(account_id);
        let mut first_error = None;
        for kind in ["access-token", "refresh-token", "id-token", "metadata"] {
            if let Err(error) = self
                .secrets
                .delete(&Self::account_secret_account(account_id, kind))
            {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    pub(crate) fn migrate_legacy_credentials(&self) -> Result<(), AuthError> {
        let Some(id_token) = self.get_text(LEGACY_ID_TOKEN_ACCOUNT)? else {
            return Ok(());
        };
        let Some(access_token) = self.get_text(LEGACY_ACCESS_TOKEN_ACCOUNT)? else {
            return Ok(());
        };
        let Some(refresh_token) = self.get_text(LEGACY_REFRESH_TOKEN_ACCOUNT)? else {
            return Ok(());
        };
        let account_id = id_token_auth_claim(&id_token, "chatgpt_account_id").ok_or_else(|| {
            AuthError::new(
                AuthErrorKind::CredentialCorrupted,
                "Legacy Codex credentials are missing a ChatGPT account id.",
            )
        })?;
        let credentials = CodexCredentials {
            access_token: SecretString::new(access_token),
            refresh_token: SecretString::new(refresh_token),
            id_token: SecretString::new(id_token),
            last_refresh_at: self
                .get_text(LEGACY_METADATA_ACCOUNT)?
                .and_then(|value| serde_json::from_str::<CodexCredentialMetadata>(&value).ok())
                .unwrap_or_default()
                .last_refresh_at,
        };
        self.store_scoped_credentials(&account_id, &credentials)?;
        self.upsert_account_record(CodexAccountRecord {
            account_email: id_token_email(credentials.id_token.expose()),
            account_id,
        })?;
        self.secrets.delete(LEGACY_ACCESS_TOKEN_ACCOUNT)?;
        self.secrets.delete(LEGACY_REFRESH_TOKEN_ACCOUNT)?;
        self.secrets.delete(LEGACY_ID_TOKEN_ACCOUNT)?;
        self.secrets.delete(LEGACY_METADATA_ACCOUNT)?;
        Ok(())
    }

    pub(crate) fn ensure_legacy_credentials_migrated(&self) -> Result<(), AuthError> {
        let mut migrated = self
            .migrated
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if *migrated {
            return Ok(());
        }
        self.migrate_legacy_credentials()?;
        *migrated = true;
        Ok(())
    }

    pub(crate) fn account_records(&self) -> Result<Vec<CodexAccountRecord>, AuthError> {
        self.ensure_legacy_credentials_migrated()?;
        self.read_account_index()
    }

    pub(crate) fn resolve_account_id(
        &self,
        requested_account_id: Option<&str>,
    ) -> Result<String, AuthError> {
        let records = self.account_records()?;
        if let Some(account_id) = requested_account_id
            .map(str::trim)
            .filter(|account_id| !account_id.is_empty())
        {
            if records.iter().any(|record| record.account_id == account_id) {
                return Ok(account_id.to_string());
            }
            return Err(AuthError::new(
                AuthErrorKind::AccountNotFound,
                "The requested Codex account is not signed in.",
            ));
        }
        match records.as_slice() {
            [record] => Ok(record.account_id.clone()),
            [] => Err(AuthError::new(
                AuthErrorKind::AuthenticationRequired,
                "No Codex account is signed in.",
            )),
            _ => Err(AuthError::new(
                AuthErrorKind::AccountSelectionRequired,
                "Multiple Codex accounts are signed in; select an account explicitly.",
            )),
        }
    }

    pub(crate) fn read_credentials(&self, account_id: &str) -> Result<CodexCredentials, AuthError> {
        self.ensure_legacy_credentials_migrated()?;
        if let Some(credentials) = self.cache().credentials(account_id) {
            return Ok(credentials);
        }
        let credentials = self.read_scoped_credentials(account_id)?;
        self.cache().cache_credentials(account_id, &credentials);
        Ok(credentials)
    }

    pub(crate) fn store_credentials(
        &self,
        account_id: &str,
        credentials: &CodexCredentials,
    ) -> Result<(), AuthError> {
        let token_account_id =
            id_token_auth_claim(credentials.id_token.expose(), "chatgpt_account_id").ok_or_else(
                || {
                    AuthError::new(
                        AuthErrorKind::CredentialCorrupted,
                        "Codex ID token is missing a ChatGPT account id.",
                    )
                },
            )?;
        if token_account_id != account_id {
            return Err(AuthError::new(
                AuthErrorKind::CredentialCorrupted,
                "Codex OAuth credentials do not match the selected account.",
            ));
        }
        self.store_scoped_credentials(account_id, credentials)?;
        self.upsert_account_record(CodexAccountRecord {
            account_id: account_id.to_string(),
            account_email: id_token_email(credentials.id_token.expose()),
        })?;
        self.cache().cache_credentials(account_id, credentials);
        Ok(())
    }

    pub(crate) fn status_from_storage(
        &self,
        record: &CodexAccountRecord,
    ) -> Result<super::CodexAuthStatus, AuthError> {
        let account_id = record.account_id.as_str();
        let access_token =
            self.get_text(&Self::account_secret_account(account_id, "access-token"))?;
        let refresh_token =
            self.get_text(&Self::account_secret_account(account_id, "refresh-token"))?;
        let id_token = self.get_text(&Self::account_secret_account(account_id, "id-token"))?;
        let expires_at = access_token.as_deref().and_then(access_token_expiry);
        let now = crate::codex::token::unix_now();
        let id_matches_account = id_token
            .as_deref()
            .and_then(|token| id_token_auth_claim(token, "chatgpt_account_id"))
            .is_some_and(|token_account_id| token_account_id == account_id);
        let complete = access_token.is_some()
            && refresh_token.is_some()
            && id_token.is_some()
            && id_matches_account;
        let state = if complete {
            if expires_at.is_some_and(|expiry| expiry <= now) {
                super::CodexAuthState::RefreshRequired
            } else {
                super::CodexAuthState::Authenticated
            }
        } else if access_token.is_some() || refresh_token.is_some() || id_token.is_some() {
            super::CodexAuthState::Expired
        } else {
            super::CodexAuthState::Missing
        };
        Ok(super::CodexAuthStatus {
            account: super::CodexAccount {
                id: account_id.to_string(),
                email: id_token
                    .as_deref()
                    .and_then(id_token_email)
                    .or_else(|| record.account_email.clone()),
                expires_at,
            },
            state,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secret_store::MemorySecretStore;

    #[test]
    fn credential_entries_are_stable_and_isolated_per_codex_account() {
        let account_a_access =
            CodexCredentialStore::account_secret_account("account-a", "access-token");
        let account_a_refresh =
            CodexCredentialStore::account_secret_account("account-a", "refresh-token");
        let account_b_access =
            CodexCredentialStore::account_secret_account("account-b", "access-token");
        assert_eq!(
            account_a_access,
            CodexCredentialStore::account_secret_account("account-a", "access-token")
        );
        assert_ne!(account_a_access, account_a_refresh);
        assert_ne!(account_a_access, account_b_access);
        assert!(account_a_access.starts_with("codex-oauth:account:"));
    }

    #[test]
    fn account_index_contains_no_secret() {
        let encoded = serde_json::to_string(&[CodexAccountRecord {
            account_id: "acct-1".to_string(),
            account_email: Some("user@example.com".to_string()),
        }])
        .unwrap();
        assert!(!encoded.contains("access"));
        assert!(!encoded.contains("refresh"));
        assert!(!encoded.contains("token"));
    }

    #[test]
    fn memory_accounts_are_isolated() {
        let store = CodexCredentialStore::new(Arc::new(MemorySecretStore::new()));
        store
            .write_account_index(&[CodexAccountRecord {
                account_id: "a".to_string(),
                account_email: None,
            }])
            .unwrap();
        let other = CodexCredentialStore::new(Arc::new(MemorySecretStore::new()));
        assert!(other.read_account_index().unwrap().is_empty());
    }
}
