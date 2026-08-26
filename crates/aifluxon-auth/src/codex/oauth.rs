use crate::codex::token::{credentials_from_tokens, CodexCredentials, TokenResponse};
use crate::error::{redact_known, AuthError, AuthErrorKind};
use reqwest::StatusCode;
use serde_json::{json, Value};
use std::time::Duration;

pub const AUTHORIZATION_URL: &str = "https://auth.openai.com/oauth/authorize";
pub const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
pub const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub const OAUTH_SCOPE: &str =
    "openid profile email offline_access api.connectors.read api.connectors.invoke";
pub const AUTHORIZE_ORIGINATOR: &str = "codex_cli_rs";

#[derive(Clone)]
pub struct TokenHttpResponse {
    pub status: u16,
    pub body: String,
}

#[async_trait::async_trait]
pub trait TokenTransport: Send + Sync {
    async fn post_form(
        &self,
        url: &str,
        form: &[(&str, &str)],
    ) -> Result<TokenHttpResponse, AuthError>;
    async fn post_json(&self, url: &str, body: &Value) -> Result<TokenHttpResponse, AuthError>;
}

pub struct ReqwestTokenTransport;

#[async_trait::async_trait]
impl TokenTransport for ReqwestTokenTransport {
    async fn post_form(
        &self,
        url: &str,
        form: &[(&str, &str)],
    ) -> Result<TokenHttpResponse, AuthError> {
        let response = oauth_client()?
            .post(url)
            .form(form)
            .send()
            .await
            .map_err(|error| network(error, &[]))?;
        let status = response.status().as_u16();
        let body = response.text().await.map_err(|error| network(error, &[]))?;
        Ok(TokenHttpResponse { status, body })
    }

    async fn post_json(&self, url: &str, body: &Value) -> Result<TokenHttpResponse, AuthError> {
        let response = oauth_client()?
            .post(url)
            .json(body)
            .send()
            .await
            .map_err(|error| network(error, &[]))?;
        let status = response.status().as_u16();
        let body = response.text().await.map_err(|error| network(error, &[]))?;
        Ok(TokenHttpResponse { status, body })
    }
}

fn oauth_client() -> Result<reqwest::Client, AuthError> {
    reqwest::Client::builder()
        .use_rustls_tls()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|error| {
            AuthError::new(
                AuthErrorKind::Network,
                format!("Could not build the Codex OAuth HTTP client: {error}"),
            )
        })
}

fn network(error: reqwest::Error, secrets: &[&str]) -> AuthError {
    AuthError::new(
        AuthErrorKind::Network,
        redact_known(format!("Codex OAuth request failed: {error}"), secrets),
    )
}

pub fn authorization_url(
    redirect_uri: &str,
    challenge: &str,
    state: &str,
) -> Result<url::Url, AuthError> {
    let mut url = url::Url::parse(AUTHORIZATION_URL)
        .map_err(|error| AuthError::new(AuthErrorKind::Configuration, error.to_string()))?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", CLIENT_ID)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("scope", OAUTH_SCOPE)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("id_token_add_organizations", "true")
        .append_pair("codex_cli_simplified_flow", "true")
        .append_pair("state", state)
        .append_pair("originator", AUTHORIZE_ORIGINATOR);
    Ok(url)
}

pub fn oauth_error(status: u16, body: &str, kind: AuthErrorKind) -> AuthError {
    let error = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .and_then(Value::as_str)
                .map(|error| error.to_ascii_lowercase())
        })
        .unwrap_or_default();
    let message = if error.contains("access_denied") {
        "Codex authorization was not completed.".to_string()
    } else if error.contains("invalid_grant") {
        "Codex authorization is expired or already used.".to_string()
    } else if status == StatusCode::TOO_MANY_REQUESTS.as_u16() {
        "Codex authorization is rate limited. Try again later.".to_string()
    } else if (500..600).contains(&status) {
        "Codex authorization is temporarily unavailable.".to_string()
    } else if status == 401 || status == 403 {
        "Codex authorization is no longer valid.".to_string()
    } else {
        "Codex authorization failed.".to_string()
    };
    AuthError::new(kind, message)
}

pub async fn exchange_authorization_code(
    transport: &dyn TokenTransport,
    code: &str,
    redirect_uri: &str,
    verifier: &str,
) -> Result<CodexCredentials, AuthError> {
    let response = transport
        .post_form(
            TOKEN_URL,
            &[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", redirect_uri),
                ("client_id", CLIENT_ID),
                ("code_verifier", verifier),
            ],
        )
        .await
        .map_err(|error| {
            AuthError::new(
                error.kind(),
                redact_known(error.message(), &[code, verifier]),
            )
        })?;
    if !(200..300).contains(&response.status) {
        return Err(oauth_error(
            response.status,
            &response.body,
            AuthErrorKind::TokenExchange,
        ));
    }
    let tokens: TokenResponse = serde_json::from_str(&response.body).map_err(|_| {
        AuthError::new(
            AuthErrorKind::TokenExchange,
            "Codex OAuth response is invalid.",
        )
    })?;
    credentials_from_tokens(tokens, None)
}

pub async fn refresh_tokens(
    transport: &dyn TokenTransport,
    current: &CodexCredentials,
) -> Result<CodexCredentials, AuthError> {
    let refresh = current.refresh_token.expose().to_string();
    let response = transport
        .post_json(
            TOKEN_URL,
            &json!({
                "client_id": CLIENT_ID,
                "grant_type": "refresh_token",
                "refresh_token": refresh,
            }),
        )
        .await
        .map_err(|error| {
            AuthError::new(
                error.kind(),
                redact_known(error.message(), &[refresh.as_str()]),
            )
        })?;
    if !(200..300).contains(&response.status) {
        return Err(oauth_error(
            response.status,
            &response.body,
            AuthErrorKind::TokenRefresh,
        ));
    }
    let tokens: TokenResponse = serde_json::from_str(&response.body).map_err(|_| {
        AuthError::new(
            AuthErrorKind::TokenRefresh,
            "Codex OAuth refresh response is invalid.",
        )
    })?;
    credentials_from_tokens(tokens, Some(current))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorization_url_contains_expected_client_id() {
        let url =
            authorization_url("http://localhost:1455/auth/callback", "challenge", "state").unwrap();
        let query = url.query().unwrap_or_default();
        assert!(query.contains(&format!("client_id={CLIENT_ID}")));
        assert!(query.contains("code_challenge=challenge"));
        assert!(query.contains("state=state"));
        assert!(query.contains("redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback"));
        assert!(query.contains("code_challenge_method=S256"));
    }
}
