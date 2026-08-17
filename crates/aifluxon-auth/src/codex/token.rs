use crate::error::{AuthError, AuthErrorKind};
use crate::secret::SecretString;
use base64::{engine::general_purpose, Engine as _};
use serde::Deserialize;
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub(crate) struct CodexCredentials {
    pub access_token: SecretString,
    pub refresh_token: SecretString,
    pub id_token: SecretString,
    pub last_refresh_at: u64,
}

impl std::fmt::Debug for CodexCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodexCredentials")
            .field("access_token", &self.access_token)
            .field("refresh_token", &self.refresh_token)
            .field("id_token", &self.id_token)
            .field("last_refresh_at", &self.last_refresh_at)
            .finish()
    }
}

#[derive(Clone, Debug, Default, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CodexCredentialMetadata {
    pub last_refresh_at: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct TokenResponse {
    pub id_token: Option<String>,
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

pub fn jwt_payload(token: &str) -> Option<Value> {
    let encoded = token.split('.').nth(1)?;
    let decoded = general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .or_else(|_| general_purpose::URL_SAFE.decode(encoded))
        .ok()?;
    serde_json::from_slice(&decoded).ok()
}

pub fn access_token_expiry(access_token: &str) -> Option<u64> {
    jwt_payload(access_token)?.get("exp")?.as_u64()
}

pub fn id_token_auth_claim(id_token: &str, key: &str) -> Option<String> {
    jwt_payload(id_token)?
        .get("https://api.openai.com/auth")?
        .get(key)?
        .as_str()
        .map(str::to_string)
}

pub fn id_token_email(id_token: &str) -> Option<String> {
    let payload = jwt_payload(id_token)?;
    payload
        .get("email")
        .and_then(Value::as_str)
        .or_else(|| {
            payload
                .get("https://api.openai.com/profile")
                .and_then(|profile| profile.get("email"))
                .and_then(Value::as_str)
        })
        .map(str::to_string)
}

pub fn credentials_from_tokens(
    tokens: TokenResponse,
    fallback: Option<&CodexCredentials>,
) -> Result<CodexCredentials, AuthError> {
    let access_token = tokens
        .access_token
        .filter(|value| !value.trim().is_empty())
        .or_else(|| fallback.map(|current| current.access_token.expose().to_string()))
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            AuthError::new(
                AuthErrorKind::TokenExchange,
                "Codex OAuth response is missing an access token.",
            )
        })?;
    let refresh_token = tokens
        .refresh_token
        .filter(|value| !value.trim().is_empty())
        .or_else(|| fallback.map(|current| current.refresh_token.expose().to_string()))
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            AuthError::new(
                AuthErrorKind::TokenExchange,
                "Codex OAuth response is missing a refresh token.",
            )
        })?;
    let id_token = tokens
        .id_token
        .filter(|value| !value.trim().is_empty())
        .or_else(|| fallback.map(|current| current.id_token.expose().to_string()))
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            AuthError::new(
                AuthErrorKind::TokenExchange,
                "Codex OAuth response is missing an ID token.",
            )
        })?;
    if id_token_auth_claim(&id_token, "chatgpt_account_id").is_none() {
        return Err(AuthError::new(
            AuthErrorKind::TokenExchange,
            "Codex ID token is missing a ChatGPT account id.",
        ));
    }
    Ok(CodexCredentials {
        access_token: SecretString::new(access_token),
        refresh_token: SecretString::new(refresh_token),
        id_token: SecretString::new(id_token),
        last_refresh_at: unix_now(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jwt_with_payload(payload: &Value) -> String {
        let encoded = general_purpose::URL_SAFE_NO_PAD.encode(payload.to_string());
        format!("aaa.{encoded}.sig")
    }

    #[test]
    fn id_token_claims_follow_codex_namespaced_shape() {
        let token = jwt_with_payload(&serde_json::json!({
            "email": "user@example.com",
            "https://api.openai.com/auth": { "chatgpt_account_id": "acct-1" }
        }));
        assert_eq!(
            id_token_auth_claim(&token, "chatgpt_account_id").as_deref(),
            Some("acct-1")
        );
        assert_eq!(id_token_email(&token).as_deref(), Some("user@example.com"));
    }

    #[test]
    fn codex_credentials_debug_never_contains_tokens() {
        let credentials = CodexCredentials {
            access_token: SecretString::new("access-secret"),
            refresh_token: SecretString::new("refresh-secret"),
            id_token: SecretString::new("id-secret"),
            last_refresh_at: 1,
        };
        let debug = format!("{credentials:?}");
        assert!(!debug.contains("access-secret"));
        assert!(!debug.contains("refresh-secret"));
        assert!(!debug.contains("id-secret"));
    }
}
