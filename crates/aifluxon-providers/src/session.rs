use aifluxon_core::{content_hash, ProviderSessionKey};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct ProviderSessionContext {
    pub key: ProviderSessionKey,
    pub profile_id: String,
    pub project_scope: Option<PathBuf>,
    pub conversation_scope: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ProviderRequestDiagnostics {
    pub provider: String,
    pub session_key_hash: String,
    pub cache_strategy: String,
    pub stable_prefix_hash: String,
    pub context_hash: String,
}

pub fn provider_session_key(
    primary_session_id: Option<&str>,
    fallback_session_id: Option<&str>,
) -> ProviderSessionKey {
    if let Some(session_id) = primary_session_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return ProviderSessionKey::from_cache_session(session_id);
    }
    if let Some(session_id) = fallback_session_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return ProviderSessionKey::from_cache_session(session_id);
    }
    ProviderSessionKey::from_cache_session("ephemeral")
}

pub fn hashed_session_key(key: &ProviderSessionKey) -> String {
    content_hash(key.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn provider_session_key_prefers_primary_and_has_no_run_identity_input() {
        let primary = provider_session_key(Some("session-primary"), Some("session-fallback"));
        let fallback = provider_session_key(None, Some("session-fallback"));
        let ephemeral = provider_session_key(None, None);

        assert_eq!(primary.as_str(), "session-primary");
        assert_eq!(fallback.as_str(), "session-fallback");
        assert_eq!(ephemeral.as_str(), "ephemeral");
    }
}
