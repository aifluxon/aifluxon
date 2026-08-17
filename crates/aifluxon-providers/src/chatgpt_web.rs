#![allow(dead_code)]

use aifluxon_core::{
    ModelEventSink, ModelProvider, ModelTurn, ModelTurnRequest, ProviderCapabilities,
    ProviderError, ProviderId,
};
use std::sync::Arc;

pub fn chatgpt_web_capabilities() -> ProviderCapabilities {
    ChatGptWebProvider::capabilities_static()
}

pub struct ChatGptWebProvider;

impl ChatGptWebProvider {
    pub fn capabilities_static() -> ProviderCapabilities {
        ProviderCapabilities::chatgpt_web()
    }
}

#[async_trait::async_trait]
impl ModelProvider for ChatGptWebProvider {
    fn id(&self) -> &ProviderId {
        static ID: std::sync::OnceLock<ProviderId> = std::sync::OnceLock::new();
        ID.get_or_init(|| ProviderId::new("chatgpt_web"))
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
            "ChatGPT Web next_turn stays on the existing web protocol; AgentCoordinator owns local tool execution.",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bug_agent_030_chatgpt_web_is_not_openai_compatible() {
        assert_eq!(ChatGptWebProvider.id().as_str(), "chatgpt_web");
        let capabilities = ChatGptWebProvider.capabilities();
        assert!(capabilities.supports_hosted_tools);
        assert!(capabilities.supports_remote_conversation);
        assert!(capabilities.supports_session_restore);
        assert!(!capabilities.supports_prompt_cache_key);
        assert!(!capabilities.supports_images);
    }
}
