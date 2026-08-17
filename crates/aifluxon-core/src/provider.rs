use crate::ids::{RunId, SessionId};
use crate::message::{Message, ToolCall};
use crate::tool::ToolDescriptor;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock;

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ProviderId(pub String);

impl ProviderId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for ProviderId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for ProviderId {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ModelRef {
    pub provider: ProviderId,
    pub model: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct ProviderSessionKey(String);

impl ProviderSessionKey {
    pub fn from_cache_session(cache_session_id: &str) -> Self {
        Self(cache_session_id.trim().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn from_session_id(session_id: &SessionId) -> Self {
        Self(session_id.hyphenated())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProviderCapabilities {
    pub supports_remote_conversation: bool,
    pub supports_prompt_cache_key: bool,
    pub supports_reasoning_stream: bool,
    pub supports_parallel_tool_calls: bool,
    pub supports_hosted_tools: bool,
    pub supports_images: bool,
    pub supports_web_search: bool,
    pub supports_session_restore: bool,
}

impl ProviderCapabilities {
    pub fn openai_compatible() -> Self {
        Self {
            supports_remote_conversation: false,
            supports_prompt_cache_key: true,
            supports_reasoning_stream: true,
            supports_parallel_tool_calls: true,
            supports_hosted_tools: false,
            supports_images: true,
            supports_web_search: true,
            supports_session_restore: false,
        }
    }

    pub fn chatgpt_web() -> Self {
        Self {
            supports_remote_conversation: true,
            supports_prompt_cache_key: false,
            supports_reasoning_stream: true,
            supports_parallel_tool_calls: false,
            supports_hosted_tools: true,
            supports_images: false,
            supports_web_search: true,
            supports_session_restore: true,
        }
    }

    pub fn deepseek_web() -> Self {
        Self {
            supports_remote_conversation: true,
            supports_prompt_cache_key: false,
            supports_reasoning_stream: true,
            supports_parallel_tool_calls: false,
            supports_hosted_tools: false,
            supports_images: false,
            supports_web_search: true,
            supports_session_restore: true,
        }
    }
}

/// Host-computed, product-neutral turn options. Project/profile paths stay Host-owned;
/// only the resulting cache key and capability toggles cross this boundary.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProviderFeatureRequest {
    pub web_search: bool,
    pub image_generation: bool,
    pub reasoning_effort: Option<String>,
    pub thinking_mode: Option<String>,
    pub thinking_budget: Option<String>,
    pub prompt_cache_key: Option<String>,
    pub explicit_cache: bool,
}

#[derive(Clone, Debug)]
pub struct ModelTurnRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDescriptor>,
    pub session_key: ProviderSessionKey,
    pub run_id: RunId,
    pub opaque_state: Option<Value>,
    pub features: ProviderFeatureRequest,
}

/// Product-neutral reason a provider asked Runtime to run another model turn.
///
/// Vendor protocol details stay in the provider. Runtime only sees these reasons
/// so it can apply bounded continuation without provider-name branches.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContinuationReason {
    /// Visible text promised unfinished tool work but emitted no tool call.
    Incomplete,
    /// The remote protocol said this turn is not a valid terminal.
    ProviderRequested,
    /// The model produced only hidden/internal summary, not a user-facing answer.
    SummaryOnly,
}

impl ContinuationReason {
    pub const fn default_limit(self) -> u32 {
        match self {
            Self::Incomplete => 2,
            Self::SummaryOnly => 1,
            Self::ProviderRequested => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderTerminal {
    Stop,
    ToolCalls,
    Continue(ContinuationReason),
    Cancelled,
    Error,
}

impl ProviderTerminal {
    pub fn continuation_reason(self) -> Option<ContinuationReason> {
        match self {
            Self::Continue(reason) => Some(reason),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ModelTurn {
    pub text: String,
    pub reasoning: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Option<Value>,
    pub terminal: ProviderTerminal,
    pub opaque: Value,
}

pub trait ModelEventSink: Send + Sync {
    fn on_text_delta(&self, _delta: &str) {}
    fn on_reasoning_delta(&self, _delta: &str) {}
    fn on_usage(&self, _usage: &Value) {}
}

pub struct NoopModelEventSink;

impl ModelEventSink for NoopModelEventSink {}

#[async_trait::async_trait]
pub trait ModelProvider: Send + Sync {
    fn id(&self) -> &ProviderId;
    fn capabilities(&self) -> ProviderCapabilities;

    async fn next_turn(
        &self,
        request: ModelTurnRequest,
        sink: Arc<dyn ModelEventSink>,
    ) -> Result<ModelTurn, crate::error::ProviderError>;
}

#[derive(Clone, Default)]
pub struct ProviderRegistry {
    providers: Arc<RwLock<HashMap<ProviderId, Arc<dyn ModelProvider>>>>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<P>(
        &self,
        id: impl Into<ProviderId>,
        provider: P,
    ) -> Result<(), ProviderRegistryError>
    where
        P: ModelProvider + 'static,
    {
        self.register_shared(id, Arc::new(provider))
    }

    pub fn register_shared(
        &self,
        id: impl Into<ProviderId>,
        provider: Arc<dyn ModelProvider>,
    ) -> Result<(), ProviderRegistryError> {
        let id = id.into();
        if id.as_str().trim().is_empty() {
            return Err(ProviderRegistryError::EmptyId);
        }
        if provider.id() != &id {
            return Err(ProviderRegistryError::IdMismatch {
                registered: id,
                provider: provider.id().clone(),
            });
        }
        let mut providers = self
            .providers
            .write()
            .map_err(|_| ProviderRegistryError::Unavailable)?;
        if providers.contains_key(&id) {
            return Err(ProviderRegistryError::Duplicate(id));
        }
        providers.insert(id, provider);
        Ok(())
    }

    pub fn resolve(&self, id: &ProviderId) -> Option<Arc<dyn ModelProvider>> {
        self.providers
            .read()
            .ok()
            .and_then(|providers| providers.get(id).cloned())
    }

    pub fn resolve_required(
        &self,
        id: &ProviderId,
    ) -> Result<Arc<dyn ModelProvider>, ProviderRegistryError> {
        self.resolve(id)
            .ok_or_else(|| ProviderRegistryError::Unknown(id.clone()))
    }

    pub fn contains(&self, id: &ProviderId) -> bool {
        self.resolve(id).is_some()
    }

    pub fn is_empty(&self) -> bool {
        self.providers
            .read()
            .map(|providers| providers.is_empty())
            .unwrap_or(true)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ProviderRegistryError {
    #[error("Provider identifiers must not be empty.")]
    EmptyId,
    #[error("The provider identifier is already registered.")]
    Duplicate(ProviderId),
    #[error("The provider is not registered.")]
    Unknown(ProviderId),
    #[error("The registered provider id does not match the provider implementation id.")]
    IdMismatch {
        registered: ProviderId,
        provider: ProviderId,
    },
    #[error("The provider registry is unavailable.")]
    Unavailable,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_provider_id_is_accepted_without_a_fixed_kind() {
        let provider = ProviderId::new("acme_private_gateway");
        let model = ModelRef {
            provider: provider.clone(),
            model: "custom-model-v1".to_string(),
        };

        assert_eq!(provider.as_str(), "acme_private_gateway");
        assert_eq!(model.provider, provider);
    }

    #[test]
    fn provider_session_key_follows_session_not_run() {
        let session = SessionId::new();
        let first_run = RunId::new();
        let second_run = RunId::new();
        let first_key = ProviderSessionKey::from_session_id(&session);
        let resumed_key = ProviderSessionKey::from_session_id(&session);

        assert_ne!(first_run, second_run);
        assert_eq!(first_key, resumed_key);
        assert_ne!(first_key.as_str(), first_run.hyphenated());
        assert_ne!(first_key.as_str(), second_run.hyphenated());
    }

    #[test]
    fn continuation_reasons_are_product_neutral_and_bounded() {
        assert_eq!(ContinuationReason::Incomplete.default_limit(), 2);
        assert_eq!(ContinuationReason::SummaryOnly.default_limit(), 1);
        assert_eq!(ContinuationReason::ProviderRequested.default_limit(), 4);
        for reason in [
            ContinuationReason::Incomplete,
            ContinuationReason::SummaryOnly,
            ContinuationReason::ProviderRequested,
        ] {
            let name = format!("{reason:?}").to_ascii_lowercase();
            assert!(!name.contains("qwen"));
            assert!(!name.contains("codex"));
            assert!(!name.contains("deepseek"));
        }
    }
}
