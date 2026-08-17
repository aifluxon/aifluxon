use aifluxon_core::{
    ContentPart, Message, MessageRole, ModelRef, ProviderId, RunLimits, RunRequest, SessionId,
};

pub fn user_prompt_request(
    provider: ProviderId,
    model: impl Into<String>,
    prompt: impl Into<String>,
    session_id: Option<SessionId>,
    limits: RunLimits,
) -> RunRequest {
    RunRequest {
        session_id,
        messages: vec![Message {
            role: MessageRole::User,
            content: vec![ContentPart::Text(prompt.into())],
            tool_calls: Vec::new(),
            tool_call_id: None,
            provider_state: None,
        }],
        model: ModelRef {
            provider,
            model: model.into(),
        },
        session_key: None,
        allowed_tools: None,
        limits,
        features: Default::default(),
        authority: None,
    }
}
