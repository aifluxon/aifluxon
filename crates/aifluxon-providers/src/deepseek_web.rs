#![allow(dead_code)]

use aifluxon_core::{
    ModelEventSink, ModelProvider, ModelTurn, ModelTurnRequest, ProviderCapabilities,
    ProviderError, ProviderId, ProviderSessionKey,
};
use std::sync::Arc;

pub fn deepseek_web_capabilities() -> ProviderCapabilities {
    DeepSeekWebProvider::capabilities_static()
}

pub struct DeepSeekWebProvider;

impl DeepSeekWebProvider {
    pub fn capabilities_static() -> ProviderCapabilities {
        ProviderCapabilities::deepseek_web()
    }

    pub fn binding_key_from_session(session_id: &str) -> ProviderSessionKey {
        ProviderSessionKey::from_cache_session(session_id)
    }
}

#[async_trait::async_trait]
impl ModelProvider for DeepSeekWebProvider {
    fn id(&self) -> &ProviderId {
        static ID: std::sync::OnceLock<ProviderId> = std::sync::OnceLock::new();
        ID.get_or_init(|| ProviderId::new("deepseek_web"))
    }

    fn capabilities(&self) -> ProviderCapabilities {
        Self::capabilities_static()
    }

    async fn next_turn(
        &self,
        request: ModelTurnRequest,
        _sink: Arc<dyn ModelEventSink>,
    ) -> Result<ModelTurn, ProviderError> {
        let _ = request;
        Err(ProviderError::message(
            "DeepSeek Web next_turn stays on the existing web protocol; AgentCoordinator owns local tool execution.",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bug_agent_031_deepseek_web_binding_stays_session_scoped() {
        assert_eq!(DeepSeekWebProvider.id().as_str(), "deepseek_web");
        let first = DeepSeekWebProvider::binding_key_from_session("session-a");
        let second = DeepSeekWebProvider::binding_key_from_session("session-b");
        assert_eq!(first.as_str(), "session-a");
        assert_ne!(first, second);
        assert!(
            DeepSeekWebProvider
                .capabilities()
                .supports_remote_conversation
        );
        assert!(!DeepSeekWebProvider.capabilities().supports_hosted_tools);
    }
}
