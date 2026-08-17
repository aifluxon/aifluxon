use crate::authority::ExecutionAuthority;
use crate::ids::{RunId, SessionId};
use crate::message::Message;
use crate::provider::{ModelRef, ProviderFeatureRequest, ProviderSessionKey};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RunContext {
    pub run_id: RunId,
    pub session_id: Option<SessionId>,
    pub parent_run_id: Option<RunId>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RunLimits {
    pub max_model_rounds: u32,
    pub max_tool_invocations: u32,
}

impl Default for RunLimits {
    fn default() -> Self {
        Self {
            max_model_rounds: 32,
            max_tool_invocations: 64,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunRequest {
    pub session_id: Option<SessionId>,
    pub messages: Vec<Message>,
    pub model: ModelRef,
    pub session_key: Option<ProviderSessionKey>,
    pub allowed_tools: Option<Vec<String>>,
    pub limits: RunLimits,
    pub features: ProviderFeatureRequest,
    pub authority: Option<ExecutionAuthority>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum RunState {
    Running,
    AwaitingOperation,
    Completed,
    Failed,
    Cancelled,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{ContentPart, MessageRole};
    use crate::provider::ProviderId;

    #[test]
    fn generic_parent_child_relationship_uses_only_run_ids() {
        let parent = RunId::new();
        let child = RunContext {
            run_id: RunId::new(),
            session_id: Some(SessionId::new()),
            parent_run_id: Some(parent),
        };

        assert_eq!(child.parent_run_id, Some(parent));
        assert_ne!(child.run_id, parent);
    }

    #[test]
    fn run_request_can_be_built_without_product_context() {
        let request = RunRequest {
            session_id: None,
            messages: vec![Message {
                role: MessageRole::User,
                content: vec![ContentPart::Text("hello".to_string())],
                tool_calls: Vec::new(),
                tool_call_id: None,
                provider_state: None,
            }],
            model: ModelRef {
                provider: ProviderId::new("custom_provider"),
                model: "model-1".to_string(),
            },
            session_key: None,
            allowed_tools: None,
            limits: RunLimits {
                max_model_rounds: 8,
                max_tool_invocations: 16,
            },
            features: Default::default(),
            authority: None,
        };

        assert_eq!(request.model.provider.as_str(), "custom_provider");
        assert!(request.session_key.is_none());
        assert!(request.allowed_tools.is_none());
    }
}
