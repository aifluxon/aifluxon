use crate::error::AuthError;
use crate::secret::SecretString;
use std::sync::Arc;

#[derive(Clone)]
pub struct BearerCredential {
    token: SecretString,
}

impl BearerCredential {
    pub fn new(token: impl Into<SecretString>) -> Self {
        Self {
            token: token.into(),
        }
    }

    pub fn token(&self) -> &str {
        self.token.expose()
    }
}

impl std::fmt::Debug for BearerCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BearerCredential")
            .field("token", &self.token)
            .finish()
    }
}

#[async_trait::async_trait]
pub trait CredentialSource: Send + Sync {
    async fn bearer(&self) -> Result<BearerCredential, AuthError>;
    async fn force_refresh(&self) -> Result<BearerCredential, AuthError> {
        self.bearer().await
    }
    fn supports_refresh(&self) -> bool {
        false
    }
}

pub struct StaticBearerCredential {
    token: SecretString,
}

impl StaticBearerCredential {
    pub fn new(token: impl Into<SecretString>) -> Self {
        Self {
            token: token.into(),
        }
    }
}

impl std::fmt::Debug for StaticBearerCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StaticBearerCredential")
            .field("token", &self.token)
            .finish()
    }
}

#[async_trait::async_trait]
impl CredentialSource for StaticBearerCredential {
    async fn bearer(&self) -> Result<BearerCredential, AuthError> {
        Ok(BearerCredential::new(self.token.clone()))
    }
}

#[async_trait::async_trait]
impl CredentialSource for Arc<dyn CredentialSource> {
    async fn bearer(&self) -> Result<BearerCredential, AuthError> {
        (**self).bearer().await
    }

    async fn force_refresh(&self) -> Result<BearerCredential, AuthError> {
        (**self).force_refresh().await
    }

    fn supports_refresh(&self) -> bool {
        (**self).supports_refresh()
    }
}
