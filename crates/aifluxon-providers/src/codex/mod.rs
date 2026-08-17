use crate::openai_compatible::{OpenAiApiMode, OpenAiCompatibleConfig};
use crate::strategy::ModelApiCapabilities;
use aifluxon_auth::CredentialSource;
use aifluxon_core::ProviderId;
use serde_json::{json, Value};
use std::sync::Arc;

pub const OAUTH_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
pub const ORIGINATOR: &str = "codex_cli_rs";
pub const CLIENT_VERSION: &str = "0.144.5";

pub fn capabilities() -> ModelApiCapabilities {
    ModelApiCapabilities::RESPONSES_ONLY
}

pub fn apply_responses_contract(body: &mut Value, reasoning_effort: &str) {
    body["include"] = json!(["reasoning.encrypted_content"]);
    if !body.get("reasoning").is_some_and(Value::is_object) {
        body["reasoning"] = json!({});
    }
    body["reasoning"]["summary"] = json!(if reasoning_effort == "none" {
        "none"
    } else {
        "auto"
    });
    if !body.get("text").is_some_and(Value::is_object) {
        body["text"] = json!({});
    }
    body["text"]["verbosity"] = json!("medium");
}

pub fn should_continue_end_turn(opaque: &Value) -> bool {
    opaque
        .get("end_turn")
        .and_then(Value::as_bool)
        .is_some_and(|end_turn| !end_turn)
}

pub fn user_agent() -> String {
    format!(
        "{ORIGINATOR}/{CLIENT_VERSION} ({}; {}) aifluxon/{}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        env!("CARGO_PKG_VERSION")
    )
}

pub fn oauth_config(
    credential: Arc<dyn CredentialSource>,
    account_id: impl Into<String>,
) -> OpenAiCompatibleConfig {
    let account_id = account_id.into();
    let mut config = OpenAiCompatibleConfig::with_credential(
        ProviderId::new("codex"),
        OAUTH_BASE_URL,
        credential,
        OpenAiApiMode::Responses,
        false,
    );
    config.chatgpt_account_id = Some(account_id);
    config
}

pub fn oauth_request_headers(
    account_id: Option<&str>,
    session_key: &str,
    turn_state: Option<&str>,
) -> Vec<(String, String)> {
    let Some(account_id) = account_id.filter(|value| !value.trim().is_empty()) else {
        return Vec::new();
    };
    let session_id = if session_key.trim().is_empty() {
        uuid::Uuid::new_v4().to_string()
    } else {
        session_key.to_string()
    };
    let mut headers = vec![
        ("ChatGPT-Account-ID".to_string(), account_id.to_string()),
        ("version".to_string(), CLIENT_VERSION.to_string()),
        ("originator".to_string(), ORIGINATOR.to_string()),
        ("user-agent".to_string(), user_agent()),
        ("session-id".to_string(), session_id.clone()),
        ("thread-id".to_string(), session_id),
        (
            "x-client-request-id".to_string(),
            uuid::Uuid::new_v4().to_string(),
        ),
        ("accept".to_string(), "text/event-stream".to_string()),
    ];
    if let Some(turn_state) = turn_state.map(str::trim).filter(|value| !value.is_empty()) {
        headers.push(("x-codex-turn-state".to_string(), turn_state.to_string()));
    }
    headers
}

#[cfg(test)]
mod tests {
    use super::*;
    use aifluxon_auth::StaticBearerCredential;

    #[test]
    fn oauth_headers_keep_accounts_isolated() {
        let headers_a = oauth_request_headers(Some("account-a"), "session-a", Some("turn-a"));
        let headers_b = oauth_request_headers(Some("account-b"), "session-b", Some("turn-b"));
        let account_a = headers_a
            .iter()
            .find(|(name, _)| name == "ChatGPT-Account-ID")
            .map(|(_, value)| value.as_str());
        let account_b = headers_b
            .iter()
            .find(|(name, _)| name == "ChatGPT-Account-ID")
            .map(|(_, value)| value.as_str());
        assert_eq!(account_a, Some("account-a"));
        assert_eq!(account_b, Some("account-b"));
        assert!(headers_a
            .iter()
            .any(|(name, value)| name == "originator" && value == ORIGINATOR));
        assert!(headers_a
            .iter()
            .any(|(name, value)| name == "version" && value == CLIENT_VERSION));
        assert!(headers_a
            .iter()
            .any(|(name, value)| name == "user-agent" && value.contains("aifluxon/")));
        assert!(!headers_a
            .iter()
            .any(|(_, value)| value.to_ascii_lowercase().contains("easyphy")));
    }

    #[test]
    fn static_credential_path_does_not_emit_oauth_headers() {
        assert!(oauth_request_headers(None, "session", None).is_empty());
    }

    #[test]
    fn oauth_config_uses_chatgpt_backend() {
        let config = oauth_config(Arc::new(StaticBearerCredential::new("token")), "acct-1");
        assert_eq!(config.base_url, OAUTH_BASE_URL);
        assert_eq!(config.chatgpt_account_id.as_deref(), Some("acct-1"));
        assert_eq!(config.api_mode, OpenAiApiMode::Responses);
    }
}
