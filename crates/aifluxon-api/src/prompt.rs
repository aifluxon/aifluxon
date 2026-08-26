use aifluxon_core::{
    ContentPart, Message, MessageRole, ModelRef, ProviderFeatureRequest, ProviderId, RunLimits,
    RunRequest, SessionId,
};

pub fn user_prompt_request(
    provider: ProviderId,
    model: impl Into<String>,
    prompt: impl Into<String>,
    session_id: Option<SessionId>,
    limits: RunLimits,
) -> RunRequest {
    user_prompt_request_with_system(
        provider,
        model,
        prompt,
        session_id,
        limits,
        ProviderFeatureRequest::default(),
        None::<String>,
    )
}

pub fn user_prompt_request_with_features(
    provider: ProviderId,
    model: impl Into<String>,
    prompt: impl Into<String>,
    session_id: Option<SessionId>,
    limits: RunLimits,
    features: ProviderFeatureRequest,
) -> RunRequest {
    user_prompt_request_with_system(
        provider,
        model,
        prompt,
        session_id,
        limits,
        features,
        None::<String>,
    )
}

pub fn user_prompt_request_with_system(
    provider: ProviderId,
    model: impl Into<String>,
    prompt: impl Into<String>,
    session_id: Option<SessionId>,
    limits: RunLimits,
    features: ProviderFeatureRequest,
    system_prompt: Option<impl Into<String>>,
) -> RunRequest {
    RunRequest {
        session_id,
        messages: prompt_messages(prompt, system_prompt),
        model: ModelRef {
            provider,
            model: model.into(),
        },
        session_key: None,
        allowed_tools: None,
        limits,
        features,
        authority: None,
    }
}

pub(crate) fn merge_session_and_request_messages(
    session: Vec<Message>,
    request: Vec<Message>,
) -> Vec<Message> {
    let (request_system, request_rest) = split_leading_system(request);
    if request_system.is_empty() {
        let mut merged = session;
        merged.extend(request_rest);
        return merged;
    }
    let (_session_system, session_rest) = split_leading_system(session);
    let mut merged = request_system;
    merged.extend(session_rest);
    merged.extend(request_rest);
    merged
}

fn prompt_messages(
    prompt: impl Into<String>,
    system_prompt: Option<impl Into<String>>,
) -> Vec<Message> {
    let mut messages = Vec::new();
    if let Some(system) = system_prompt {
        let text = system.into();
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            messages.push(text_message(MessageRole::System, trimmed));
        }
    }
    messages.push(text_message(MessageRole::User, prompt.into()));
    messages
}

fn split_leading_system(messages: Vec<Message>) -> (Vec<Message>, Vec<Message>) {
    let count = messages
        .iter()
        .take_while(|message| message.role == MessageRole::System)
        .count();
    let mut rest = messages;
    let system = rest.drain(..count).collect();
    (system, rest)
}

fn text_message(role: MessageRole, text: impl Into<String>) -> Message {
    Message {
        role,
        content: vec![ContentPart::Text(text.into())],
        tool_calls: Vec::new(),
        tool_call_id: None,
        provider_state: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aifluxon_core::ProviderId;

    #[test]
    fn prompt_helper_forwards_thinking_features() {
        let features = ProviderFeatureRequest {
            reasoning_effort: Some("high".to_string()),
            thinking_mode: Some("enabled".to_string()),
            thinking_budget: Some("8192".to_string()),
            ..Default::default()
        };
        let request = user_prompt_request_with_features(
            ProviderId::new("qwen"),
            "qwen-plus",
            "hi",
            None,
            RunLimits::default(),
            features,
        );
        assert_eq!(request.features.reasoning_effort.as_deref(), Some("high"));
        assert_eq!(request.features.thinking_mode.as_deref(), Some("enabled"));
        assert_eq!(request.features.thinking_budget.as_deref(), Some("8192"));
        assert_eq!(request.messages.len(), 1);
        assert_eq!(request.messages[0].role, MessageRole::User);
        assert!(user_prompt_request(
            ProviderId::new("openai"),
            "gpt-5.4",
            "hi",
            None,
            RunLimits::default(),
        )
        .features
        .reasoning_effort
        .is_none());
    }

    #[test]
    fn prompt_helper_prepends_non_empty_system_prompt() {
        let request = user_prompt_request_with_system(
            ProviderId::new("openai"),
            "gpt-5.4",
            "review this",
            None,
            RunLimits::default(),
            ProviderFeatureRequest::default(),
            Some("You are a laboratory reviewer."),
        );
        assert_eq!(request.messages.len(), 2);
        assert_eq!(request.messages[0].role, MessageRole::System);
        assert_eq!(
            request.messages[0].content,
            vec![ContentPart::Text(
                "You are a laboratory reviewer.".to_string()
            )]
        );
        assert_eq!(request.messages[1].role, MessageRole::User);
        assert!(user_prompt_request_with_system(
            ProviderId::new("openai"),
            "gpt-5.4",
            "hi",
            None,
            RunLimits::default(),
            ProviderFeatureRequest::default(),
            Some("   "),
        )
        .messages
        .iter()
        .all(|message| message.role != MessageRole::System));
    }

    #[test]
    fn session_merge_replaces_leading_system_instead_of_stacking() {
        let session = vec![
            text_message(MessageRole::System, "old persona"),
            text_message(MessageRole::User, "first"),
            text_message(MessageRole::Assistant, "ok"),
        ];
        let request = vec![
            text_message(MessageRole::System, "new persona"),
            text_message(MessageRole::User, "second"),
        ];
        let merged = merge_session_and_request_messages(session, request);
        assert_eq!(merged.len(), 4);
        assert_eq!(
            merged[0].content,
            vec![ContentPart::Text("new persona".into())]
        );
        assert_eq!(merged[1].content, vec![ContentPart::Text("first".into())]);
        assert_eq!(merged[2].role, MessageRole::Assistant);
        assert_eq!(merged[3].content, vec![ContentPart::Text("second".into())]);
    }

    #[test]
    fn session_merge_keeps_stored_system_when_request_has_none() {
        let session = vec![
            text_message(MessageRole::System, "stored persona"),
            text_message(MessageRole::User, "first"),
        ];
        let request = vec![text_message(MessageRole::User, "second")];
        let merged = merge_session_and_request_messages(session, request);
        assert_eq!(
            merged[0].content,
            vec![ContentPart::Text("stored persona".into())]
        );
        assert_eq!(merged[2].content, vec![ContentPart::Text("second".into())]);
    }
}
