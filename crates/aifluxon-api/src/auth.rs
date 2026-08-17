use crate::{
    AifluxonError, AifluxonErrorKind, ModelProvider, ProviderBinding, ProviderId, ProviderRegistry,
};
use aifluxon_auth::codex::{CodexAuthManager, CodexLoginAttempt as NativeLoginAttempt};
use aifluxon_auth::{AuthError, AuthErrorKind};
use aifluxon_providers::OpenAiCompatibleProvider;
use std::sync::Arc;

pub use aifluxon_auth::codex::{CodexAccount, CodexAuthState, CodexAuthStatus};
pub use aifluxon_auth::{
    AuthErrorKind as AifluxonAuthErrorKind, EncryptedFileSecretStore, MemorySecretStore,
    SecretStore, SecretString, SystemKeyringStore, DEFAULT_SERVICE_NAME,
};
pub use aifluxon_providers::codex::OAUTH_BASE_URL as CODEX_OAUTH_BASE_URL;

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("{message}")]
pub struct AifluxonAuthError {
    kind: AuthErrorKind,
    message: String,
}

impl AifluxonAuthError {
    pub fn kind(&self) -> AuthErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl From<AuthError> for AifluxonAuthError {
    fn from(error: AuthError) -> Self {
        Self {
            kind: error.kind(),
            message: error.message().to_string(),
        }
    }
}

impl From<AifluxonAuthError> for AifluxonError {
    fn from(error: AifluxonAuthError) -> Self {
        AifluxonError::new(AifluxonErrorKind::Provider, error.message)
    }
}

#[derive(Default)]
pub struct CodexAuthBuilder {
    secret_store: Option<Arc<dyn SecretStore>>,
}

impl CodexAuthBuilder {
    pub fn secret_store(mut self, store: impl SecretStore + 'static) -> Self {
        self.secret_store = Some(Arc::new(store));
        self
    }

    pub fn secret_store_shared(mut self, store: Arc<dyn SecretStore>) -> Self {
        self.secret_store = Some(store);
        self
    }

    pub fn build(self) -> Result<CodexAuth, AifluxonAuthError> {
        let store = self.secret_store.ok_or_else(|| {
            AifluxonAuthError::from(AuthError::new(
                AuthErrorKind::Configuration,
                "CodexAuth requires a secret store.",
            ))
        })?;
        Ok(CodexAuth {
            inner: Arc::new(CodexAuthManager::new(store)),
        })
    }
}

#[derive(Clone)]
pub struct CodexAuth {
    inner: Arc<CodexAuthManager>,
}

impl CodexAuth {
    pub fn builder() -> CodexAuthBuilder {
        CodexAuthBuilder::default()
    }

    pub fn system_default() -> Result<Self, AifluxonAuthError> {
        Self::builder()
            .secret_store(SystemKeyringStore::new(DEFAULT_SERVICE_NAME))
            .build()
    }

    pub async fn begin_login(&self) -> Result<CodexLoginAttempt, AifluxonAuthError> {
        Ok(CodexLoginAttempt {
            inner: self.inner.begin_login().await?,
        })
    }

    pub fn accounts_sync(&self) -> Result<Vec<CodexAccount>, AifluxonAuthError> {
        Ok(self.inner.accounts()?)
    }

    pub async fn accounts(&self) -> Result<Vec<CodexAccount>, AifluxonAuthError> {
        self.accounts_sync()
    }

    pub fn status_sync(&self, account_id: &str) -> Result<CodexAuthStatus, AifluxonAuthError> {
        Ok(self.inner.status(account_id)?)
    }

    pub async fn status(&self, account_id: &str) -> Result<CodexAuthStatus, AifluxonAuthError> {
        self.status_sync(account_id)
    }

    pub async fn refresh(&self, account_id: &str) -> Result<CodexAuthStatus, AifluxonAuthError> {
        Ok(self.inner.refresh(Some(account_id), true).await?)
    }

    pub fn logout_sync(&self, account_id: &str) -> Result<(), AifluxonAuthError> {
        Ok(self.inner.logout(account_id)?)
    }

    pub async fn logout(&self, account_id: &str) -> Result<(), AifluxonAuthError> {
        self.logout_sync(account_id)
    }

    pub fn resolve_account_id_sync(
        &self,
        account_id: Option<&str>,
    ) -> Result<String, AifluxonAuthError> {
        Ok(self.inner.resolve_account_id(account_id)?)
    }

    pub fn provider(
        &self,
        model: impl Into<String>,
        account_id: Option<String>,
    ) -> Result<CodexProviderHandle, AifluxonAuthError> {
        let account_id = self.inner.resolve_account_id(account_id.as_deref())?;
        let source = Arc::new(self.inner.credential_source(Some(&account_id))?);
        let provider = OpenAiCompatibleProvider::configured(
            aifluxon_providers::codex::oauth_config(source, account_id.clone()),
        );
        Ok(CodexProviderHandle {
            provider: Arc::new(provider),
            model: model.into(),
            account_id,
        })
    }

    pub fn seed_account_for_tests(
        &self,
        account_id: &str,
        access_token: &str,
        refresh_token: &str,
        id_token: &str,
    ) -> Result<CodexAccount, AifluxonAuthError> {
        Ok(self.inner.seed_account(
            account_id,
            access_token.to_string(),
            refresh_token.to_string(),
            id_token.to_string(),
        )?)
    }
}

pub struct CodexLoginAttempt {
    inner: NativeLoginAttempt,
}

impl CodexLoginAttempt {
    pub fn authorization_url(&self) -> &str {
        self.inner.authorization_url()
    }

    pub async fn wait(self) -> Result<CodexAccount, AifluxonAuthError> {
        Ok(self.inner.wait().await?)
    }

    pub async fn cancel(self) {
        self.inner.cancel().await;
    }
}

#[derive(Clone)]
pub struct CodexProviderHandle {
    provider: Arc<OpenAiCompatibleProvider>,
    model: String,
    account_id: String,
}

impl CodexProviderHandle {
    pub fn provider_id(&self) -> ProviderId {
        ProviderId::new("codex")
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    pub fn model_provider(&self) -> Arc<dyn ModelProvider> {
        self.provider.clone()
    }

    pub fn compatible_provider(&self) -> Arc<OpenAiCompatibleProvider> {
        self.provider.clone()
    }

    pub fn register(&self, registry: &ProviderRegistry) -> Result<ProviderBinding, AifluxonError> {
        let provider_id = self.provider_id();
        registry
            .register_shared(provider_id.clone(), self.model_provider())
            .map_err(|error| {
                AifluxonError::new(AifluxonErrorKind::InvalidConfiguration, error.to_string())
            })?;
        Ok(ProviderBinding {
            provider_id,
            model: self.model.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jwt(account_id: &str) -> String {
        use base64::{engine::general_purpose, Engine as _};
        let payload = serde_json::json!({
            "https://api.openai.com/auth": { "chatgpt_account_id": account_id },
            "email": format!("{account_id}@example.com"),
            "exp": 4_000_000_000_u64
        });
        format!(
            "aaa.{}.sig",
            general_purpose::URL_SAFE_NO_PAD.encode(payload.to_string())
        )
    }

    #[tokio::test]
    async fn provider_handle_does_not_serialize_tokens() {
        let auth = CodexAuth::builder()
            .secret_store(MemorySecretStore::new())
            .build()
            .unwrap();
        auth.seed_account_for_tests(
            "acct-1",
            "access-secret-token",
            "refresh-secret-token",
            &jwt("acct-1"),
        )
        .unwrap();
        let handle = auth
            .provider("gpt-5.6-codex", Some("acct-1".to_string()))
            .unwrap();
        let debug = format!("{handle:?}");
        assert!(!debug.contains("access-secret-token"));
        assert!(!debug.contains("refresh-secret-token"));
        assert_eq!(handle.account_id(), "acct-1");
        assert_eq!(handle.model(), "gpt-5.6-codex");
    }

    #[tokio::test]
    async fn multiple_accounts_require_explicit_selection() {
        let auth = CodexAuth::builder()
            .secret_store(MemorySecretStore::new())
            .build()
            .unwrap();
        auth.seed_account_for_tests("acct-a", "a1", "r1", &jwt("acct-a"))
            .unwrap();
        auth.seed_account_for_tests("acct-b", "a2", "r2", &jwt("acct-b"))
            .unwrap();
        let error = auth.provider("gpt-5.6-codex", None).unwrap_err();
        assert_eq!(error.kind(), AuthErrorKind::AccountSelectionRequired);
    }
}

impl std::fmt::Debug for CodexProviderHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodexProviderHandle")
            .field("provider_id", &"codex")
            .field("model", &self.model)
            .field("account_id", &self.account_id)
            .finish()
    }
}

pub fn unlock_encrypted_store(
    store: &EncryptedFileSecretStore,
    password: &str,
) -> Result<(), AifluxonAuthError> {
    store
        .unlock(&SecretString::new(password))
        .map_err(AifluxonAuthError::from)
}
