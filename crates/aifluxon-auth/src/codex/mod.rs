mod callback;
mod oauth;
mod pkce;
mod store;
mod token;

use crate::credential::{BearerCredential, CredentialSource};
use crate::error::{AuthError, AuthErrorKind};
use crate::secret::SecretString;
use crate::secret_store::SecretStore;
use callback::{
    bind_callback_listener, wait_for_callback, write_callback_response, CALLBACK_PATH,
    LOGIN_TIMEOUT_SECS,
};
use oauth::{
    authorization_url, exchange_authorization_code, refresh_tokens, ReqwestTokenTransport,
    TokenTransport,
};
use pkce::{generate_pkce, generate_state};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use store::CodexCredentialStore;
use token::{access_token_expiry, id_token_auth_claim, CodexCredentials};
use tokio::sync::Mutex as AsyncMutex;

pub use oauth::{AUTHORIZATION_URL, CLIENT_ID, OAUTH_SCOPE, TOKEN_URL};
pub use store::CodexAccountRecord;

pub const REFRESH_WINDOW_SECS: u64 = 300;
pub const UNKNOWN_EXPIRY_REFRESH_SECS: u64 = 8 * 24 * 60 * 60;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexAccount {
    pub id: String,
    pub email: Option<String>,
    pub expires_at: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodexAuthState {
    Authenticated,
    RefreshRequired,
    Expired,
    Missing,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexAuthStatus {
    pub account: CodexAccount,
    pub state: CodexAuthState,
}

pub struct CodexAuthManager {
    store: CodexCredentialStore,
    transport: Arc<dyn TokenTransport>,
    refresh_locks: Mutex<HashMap<String, Arc<AsyncMutex<()>>>>,
    login_in_progress: AtomicBool,
}

impl CodexAuthManager {
    pub fn new(secrets: Arc<dyn SecretStore>) -> Self {
        Self::with_transport(secrets, Arc::new(ReqwestTokenTransport))
    }

    pub fn with_transport(
        secrets: Arc<dyn SecretStore>,
        transport: Arc<dyn TokenTransport>,
    ) -> Self {
        Self {
            store: CodexCredentialStore::new(secrets),
            transport,
            refresh_locks: Mutex::new(HashMap::new()),
            login_in_progress: AtomicBool::new(false),
        }
    }

    fn account_lock(&self, account_id: &str) -> Arc<AsyncMutex<()>> {
        let mut locks = self
            .refresh_locks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        locks
            .entry(account_id.to_string())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone()
    }

    pub async fn begin_login(self: &Arc<Self>) -> Result<CodexLoginAttempt, AuthError> {
        if self
            .login_in_progress
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(AuthError::new(
                AuthErrorKind::Browser,
                "A Codex login is already in progress.",
            ));
        }
        let (listener, port) = match bind_callback_listener().await {
            Ok(bound) => bound,
            Err(error) => {
                self.login_in_progress.store(false, Ordering::SeqCst);
                return Err(error);
            }
        };
        let redirect_uri = format!("http://localhost:{port}{CALLBACK_PATH}");
        let (verifier, challenge) = generate_pkce();
        let state = generate_state();
        let authorize_url = match authorization_url(&redirect_uri, &challenge, &state) {
            Ok(url) => url,
            Err(error) => {
                self.login_in_progress.store(false, Ordering::SeqCst);
                return Err(error);
            }
        };
        Ok(CodexLoginAttempt {
            manager: self.clone(),
            authorization_url: authorize_url.to_string(),
            redirect_uri,
            verifier: SecretString::new(verifier),
            state,
            listener: Some(listener),
            timeout: Duration::from_secs(LOGIN_TIMEOUT_SECS),
        })
    }

    pub fn accounts(&self) -> Result<Vec<CodexAccount>, AuthError> {
        self.store
            .account_records()?
            .iter()
            .map(|record| Ok(self.store.status_from_storage(record)?.account))
            .collect()
    }

    pub fn status(&self, account_id: &str) -> Result<CodexAuthStatus, AuthError> {
        let records = self.store.account_records()?;
        let record = records
            .iter()
            .find(|record| record.account_id == account_id)
            .ok_or_else(|| {
                AuthError::new(
                    AuthErrorKind::AccountNotFound,
                    "The requested Codex account is not signed in.",
                )
            })?;
        self.store.status_from_storage(record)
    }

    pub fn logout(&self, account_id: &str) -> Result<(), AuthError> {
        let account_id = account_id.trim();
        if account_id.is_empty() {
            return Err(AuthError::new(
                AuthErrorKind::AccountNotFound,
                "Select a Codex account to sign out.",
            ));
        }
        self.store.delete_account_credentials(account_id)?;
        self.store.remove_account_record(account_id)
    }

    pub fn resolve_account_id(&self, account_id: Option<&str>) -> Result<String, AuthError> {
        self.store.resolve_account_id(account_id)
    }

    pub async fn refresh(
        self: &Arc<Self>,
        account_id: Option<&str>,
        force: bool,
    ) -> Result<CodexAuthStatus, AuthError> {
        let account_id = self.store.resolve_account_id(account_id)?;
        self.refresh_credentials(Some(&account_id), force).await?;
        self.status(&account_id)
    }

    pub fn seed_account(
        &self,
        account_id: &str,
        access_token: impl Into<SecretString>,
        refresh_token: impl Into<SecretString>,
        id_token: impl Into<SecretString>,
    ) -> Result<CodexAccount, AuthError> {
        let credentials = CodexCredentials {
            access_token: access_token.into(),
            refresh_token: refresh_token.into(),
            id_token: id_token.into(),
            last_refresh_at: token::unix_now(),
        };
        self.store.store_credentials(account_id, &credentials)?;
        Ok(self.status(account_id)?.account)
    }

    pub(crate) async fn refresh_credentials(
        self: &Arc<Self>,
        account_id: Option<&str>,
        force: bool,
    ) -> Result<CodexCredentials, AuthError> {
        let account_id = self.store.resolve_account_id(account_id)?;
        let lock = self.account_lock(&account_id);
        let _guard = lock.lock().await;
        let current = self.store.read_credentials(&account_id)?;
        if !force && !should_refresh(&current) {
            return Ok(current);
        }
        let refreshed = refresh_tokens(self.transport.as_ref(), &current).await?;
        let refreshed_account_id =
            id_token_auth_claim(refreshed.id_token.expose(), "chatgpt_account_id");
        if refreshed_account_id.as_deref() != Some(account_id.as_str()) {
            return Err(AuthError::new(
                AuthErrorKind::TokenRefresh,
                "Refreshed Codex credentials do not match the selected account.",
            ));
        }
        self.store.store_credentials(&account_id, &refreshed)?;
        Ok(refreshed)
    }

    pub fn credential_source(
        self: &Arc<Self>,
        account_id: Option<&str>,
    ) -> Result<CodexOAuthCredentialSource, AuthError> {
        let account_id = self.store.resolve_account_id(account_id)?;
        Ok(CodexOAuthCredentialSource {
            manager: self.clone(),
            account_id,
        })
    }
}

pub struct CodexLoginAttempt {
    manager: Arc<CodexAuthManager>,
    authorization_url: String,
    redirect_uri: String,
    verifier: SecretString,
    state: String,
    listener: Option<tokio::net::TcpListener>,
    timeout: Duration,
}

impl CodexLoginAttempt {
    pub fn authorization_url(&self) -> &str {
        &self.authorization_url
    }

    pub async fn wait(mut self) -> Result<CodexAccount, AuthError> {
        let listener = self.listener.take().ok_or_else(|| {
            self.manager
                .login_in_progress
                .store(false, Ordering::SeqCst);
            AuthError::new(
                AuthErrorKind::Cancelled,
                "Codex login attempt is no longer active.",
            )
        })?;
        let result = self.complete(listener).await;
        self.manager
            .login_in_progress
            .store(false, Ordering::SeqCst);
        result
    }

    async fn complete(&self, listener: tokio::net::TcpListener) -> Result<CodexAccount, AuthError> {
        let (code, mut callback_stream) =
            wait_for_callback(listener, &self.state, self.timeout).await?;
        let credentials = match exchange_authorization_code(
            self.manager.transport.as_ref(),
            &code,
            &self.redirect_uri,
            self.verifier.expose(),
        )
        .await
        {
            Ok(credentials) => credentials,
            Err(error) => {
                write_callback_response(&mut callback_stream, false).await;
                return Err(error);
            }
        };
        let account_id = id_token_auth_claim(credentials.id_token.expose(), "chatgpt_account_id")
            .ok_or_else(|| {
            AuthError::new(
                AuthErrorKind::TokenExchange,
                "Codex ID token is missing a ChatGPT account id.",
            )
        })?;
        if let Err(error) = self
            .manager
            .store
            .store_credentials(&account_id, &credentials)
        {
            write_callback_response(&mut callback_stream, false).await;
            return Err(error);
        }
        write_callback_response(&mut callback_stream, true).await;
        Ok(self.manager.status(&account_id)?.account)
    }

    pub async fn cancel(mut self) {
        self.listener.take();
        self.manager
            .login_in_progress
            .store(false, Ordering::SeqCst);
    }
}

impl std::fmt::Debug for CodexLoginAttempt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodexLoginAttempt")
            .field("authorization_url", &self.authorization_url)
            .finish()
    }
}

pub struct CodexOAuthCredentialSource {
    manager: Arc<CodexAuthManager>,
    account_id: String,
}

impl CodexOAuthCredentialSource {
    pub fn account_id(&self) -> &str {
        &self.account_id
    }
}

#[async_trait::async_trait]
impl CredentialSource for CodexOAuthCredentialSource {
    async fn bearer(&self) -> Result<BearerCredential, AuthError> {
        let credentials = self
            .manager
            .refresh_credentials(Some(&self.account_id), false)
            .await?;
        Ok(BearerCredential::new(credentials.access_token))
    }

    async fn force_refresh(&self) -> Result<BearerCredential, AuthError> {
        let credentials = self
            .manager
            .refresh_credentials(Some(&self.account_id), true)
            .await?;
        Ok(BearerCredential::new(credentials.access_token))
    }

    fn supports_refresh(&self) -> bool {
        true
    }
}

fn should_refresh(credentials: &CodexCredentials) -> bool {
    let now = token::unix_now();
    access_token_expiry(credentials.access_token.expose())
        .map(|expiry| expiry <= now.saturating_add(REFRESH_WINDOW_SECS))
        .unwrap_or_else(|| {
            credentials.last_refresh_at == 0
                || now.saturating_sub(credentials.last_refresh_at) >= UNKNOWN_EXPIRY_REFRESH_SECS
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex::oauth::TokenHttpResponse;
    use crate::secret_store::MemorySecretStore;
    use base64::{engine::general_purpose, Engine as _};
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn jwt(account_id: &str, exp: Option<u64>) -> String {
        let mut payload = json!({
            "https://api.openai.com/auth": { "chatgpt_account_id": account_id },
            "email": format!("{account_id}@example.com")
        });
        if let Some(exp) = exp {
            payload["exp"] = json!(exp);
        }
        format!(
            "aaa.{}.sig",
            general_purpose::URL_SAFE_NO_PAD.encode(payload.to_string())
        )
    }

    struct FakeTransport {
        exchange: Mutex<Option<TokenHttpResponse>>,
        refresh: Mutex<Option<TokenHttpResponse>>,
        refresh_count: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl TokenTransport for FakeTransport {
        async fn post_form(
            &self,
            _url: &str,
            _form: &[(&str, &str)],
        ) -> Result<TokenHttpResponse, AuthError> {
            self.exchange
                .lock()
                .unwrap()
                .take()
                .ok_or_else(|| AuthError::new(AuthErrorKind::TokenExchange, "no exchange"))
        }

        async fn post_json(
            &self,
            _url: &str,
            _body: &serde_json::Value,
        ) -> Result<TokenHttpResponse, AuthError> {
            self.refresh_count.fetch_add(1, Ordering::SeqCst);
            let refresh = self.refresh.lock().unwrap();
            refresh
                .clone()
                .ok_or_else(|| AuthError::new(AuthErrorKind::TokenRefresh, "no refresh"))
        }
    }

    fn manager_with(transport: FakeTransport) -> Arc<CodexAuthManager> {
        Arc::new(CodexAuthManager::with_transport(
            Arc::new(MemorySecretStore::new()),
            Arc::new(transport),
        ))
    }

    #[tokio::test]
    async fn successful_login_persists_account_and_credentials() {
        let id = jwt("acct-1", Some(token::unix_now() + 3600));
        let transport = FakeTransport {
            exchange: Mutex::new(Some(TokenHttpResponse {
                status: 200,
                body: json!({
                    "access_token": "access-1",
                    "refresh_token": "refresh-1",
                    "id_token": id
                })
                .to_string(),
            })),
            refresh: Mutex::new(None),
            refresh_count: Arc::new(AtomicUsize::new(0)),
        };
        let manager = manager_with(transport);
        let credentials = exchange_authorization_code(
            manager.transport.as_ref(),
            "code",
            "http://localhost:1455/auth/callback",
            "verifier",
        )
        .await
        .unwrap();
        manager
            .store
            .store_credentials("acct-1", &credentials)
            .unwrap();
        let accounts = manager.accounts().unwrap();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].id, "acct-1");
        assert_eq!(accounts[0].email.as_deref(), Some("acct-1@example.com"));
        let source = manager.credential_source(None).unwrap();
        assert_eq!(source.bearer().await.unwrap().token(), "access-1");
    }

    #[tokio::test]
    async fn second_process_style_auth_instance_can_reload_account() {
        let secrets = Arc::new(MemorySecretStore::new());
        let id = jwt("acct-1", Some(token::unix_now() + 3600));
        let first = Arc::new(CodexAuthManager::new(secrets.clone()));
        first
            .store
            .store_credentials(
                "acct-1",
                &CodexCredentials {
                    access_token: SecretString::new("access-1"),
                    refresh_token: SecretString::new("refresh-1"),
                    id_token: SecretString::new(id),
                    last_refresh_at: token::unix_now(),
                },
            )
            .unwrap();
        let second = Arc::new(CodexAuthManager::new(secrets));
        assert_eq!(second.accounts().unwrap()[0].id, "acct-1");
    }

    #[tokio::test]
    async fn logout_removes_persistent_credentials() {
        let id = jwt("acct-1", Some(token::unix_now() + 3600));
        let manager = Arc::new(CodexAuthManager::new(Arc::new(MemorySecretStore::new())));
        manager
            .store
            .store_credentials(
                "acct-1",
                &CodexCredentials {
                    access_token: SecretString::new("access-1"),
                    refresh_token: SecretString::new("refresh-1"),
                    id_token: SecretString::new(id),
                    last_refresh_at: token::unix_now(),
                },
            )
            .unwrap();
        manager.logout("acct-1").unwrap();
        assert!(manager.accounts().unwrap().is_empty());
    }

    #[tokio::test]
    async fn multiple_accounts_without_selection_fails_closed() {
        let manager = Arc::new(CodexAuthManager::new(Arc::new(MemorySecretStore::new())));
        for id in ["acct-a", "acct-b"] {
            manager
                .store
                .store_credentials(
                    id,
                    &CodexCredentials {
                        access_token: SecretString::new(format!("access-{id}")),
                        refresh_token: SecretString::new(format!("refresh-{id}")),
                        id_token: SecretString::new(jwt(id, Some(token::unix_now() + 3600))),
                        last_refresh_at: token::unix_now(),
                    },
                )
                .unwrap();
        }
        assert_eq!(
            manager.resolve_account_id(None).unwrap_err().kind(),
            AuthErrorKind::AccountSelectionRequired
        );
        assert_eq!(
            manager.resolve_account_id(Some("acct-a")).unwrap(),
            "acct-a"
        );
    }

    #[tokio::test]
    async fn parallel_runs_share_one_refresh_request() {
        let refresh_count = Arc::new(AtomicUsize::new(0));
        let transport = FakeTransport {
            exchange: Mutex::new(None),
            refresh: Mutex::new(Some(TokenHttpResponse {
                status: 200,
                body: json!({
                    "access_token": "access-2",
                    "refresh_token": "refresh-2",
                    "id_token": jwt("acct-1", Some(token::unix_now() + 3600))
                })
                .to_string(),
            })),
            refresh_count: refresh_count.clone(),
        };
        let manager = manager_with(transport);
        manager
            .store
            .store_credentials(
                "acct-1",
                &CodexCredentials {
                    access_token: SecretString::new("access-1"),
                    refresh_token: SecretString::new("refresh-1"),
                    id_token: SecretString::new(jwt("acct-1", Some(token::unix_now() + 1))),
                    last_refresh_at: 1,
                },
            )
            .unwrap();
        let source = Arc::new(manager.credential_source(Some("acct-1")).unwrap());
        let mut tasks = Vec::new();
        for _ in 0..20 {
            let source = source.clone();
            tasks.push(tokio::spawn(async move {
                source.bearer().await.unwrap().token().to_string()
            }));
        }
        let mut tokens = Vec::new();
        for task in tasks {
            tokens.push(task.await.unwrap());
        }
        assert!(tokens.iter().all(|token| token == "access-2"));
        assert_eq!(refresh_count.load(Ordering::SeqCst), 1);
    }
}
