use std::fmt::{Display, Formatter};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthErrorKind {
    Configuration,
    Browser,
    CallbackBind,
    CallbackTimeout,
    CallbackProtocol,
    StateMismatch,
    TokenExchange,
    TokenRefresh,
    AuthenticationRequired,
    AccountSelectionRequired,
    AccountNotFound,
    CredentialStore,
    CredentialStoreUnavailable,
    CredentialStoreLocked,
    CredentialCorrupted,
    UnsupportedPlatform,
    Cancelled,
    Network,
}

impl AuthErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Configuration => "Configuration",
            Self::Browser => "Browser",
            Self::CallbackBind => "CallbackBind",
            Self::CallbackTimeout => "CallbackTimeout",
            Self::CallbackProtocol => "CallbackProtocol",
            Self::StateMismatch => "StateMismatch",
            Self::TokenExchange => "TokenExchange",
            Self::TokenRefresh => "TokenRefresh",
            Self::AuthenticationRequired => "AuthenticationRequired",
            Self::AccountSelectionRequired => "AccountSelectionRequired",
            Self::AccountNotFound => "AccountNotFound",
            Self::CredentialStore => "CredentialStore",
            Self::CredentialStoreUnavailable => "CredentialStoreUnavailable",
            Self::CredentialStoreLocked => "CredentialStoreLocked",
            Self::CredentialCorrupted => "CredentialCorrupted",
            Self::UnsupportedPlatform => "UnsupportedPlatform",
            Self::Cancelled => "Cancelled",
            Self::Network => "Network",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub struct AuthError {
    kind: AuthErrorKind,
    message: String,
}

impl AuthError {
    pub fn new(kind: AuthErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: sanitize_secrets(message.into()),
        }
    }

    pub fn kind(&self) -> AuthErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl Display for AuthError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

pub(crate) fn sanitize_secrets(message: String) -> String {
    let mut sanitized = message;
    for needle in [
        "Bearer ",
        "refresh_token",
        "access_token",
        "id_token",
        "code_verifier",
        "authorization_code",
    ] {
        if sanitized.contains(needle) && needle != "Bearer " {
            sanitized = sanitized.replace(needle, "[redacted]");
        }
    }
    sanitized
}

pub(crate) fn redact_known(message: impl Into<String>, secrets: &[&str]) -> String {
    let mut sanitized = message.into();
    for secret in secrets {
        let secret = secret.trim();
        if secret.len() >= 8 {
            sanitized = sanitized.replace(secret, "[redacted]");
        }
    }
    sanitize_secrets(sanitized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_error_redacts_known_secret_material() {
        let error = AuthError::new(
            AuthErrorKind::Network,
            redact_known(
                "failed Authorization: Bearer secret-token-value-1234",
                &["secret-token-value-1234"],
            ),
        );
        let text = error.to_string();
        assert!(!text.contains("secret-token-value-1234"));
        assert!(!text.contains("Bearer secret-token-value-1234"));
    }
}
