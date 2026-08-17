use crate::{AifluxonError, AifluxonErrorKind};
use aifluxon_core::{ContentPart, Message, MessageRole, RunId, RunState, SessionId};

/// Canonical final outcome of a run. Values come from Runtime terminal state, not client-side
/// token accumulation.
#[derive(Clone, Debug, PartialEq)]
pub struct RunResult {
    pub run_id: RunId,
    pub session_id: Option<SessionId>,
    pub state: RunState,
    pub output: Vec<Message>,
    pub text: String,
    pub usage: Option<serde_json::Value>,
}

pub fn assistant_text(output: &[Message]) -> String {
    output
        .iter()
        .rev()
        .find(|message| message.role == MessageRole::Assistant)
        .map(|message| {
            message
                .content
                .iter()
                .filter_map(|part| match part {
                    ContentPart::Text(text) => Some(text.as_str()),
                    ContentPart::Image(_) => None,
                })
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

pub fn terminal_result(
    run_id: RunId,
    session_id: Option<SessionId>,
    state: RunState,
    output: Vec<Message>,
    usage: Option<serde_json::Value>,
    failure: Option<String>,
) -> Result<RunResult, AifluxonError> {
    match state {
        RunState::Cancelled => Err(AifluxonError::new(
            AifluxonErrorKind::Cancelled,
            "The run was cancelled.",
        )),
        RunState::Failed => Err(AifluxonError::new(
            AifluxonErrorKind::Failed,
            failure.unwrap_or_else(|| "The run failed.".to_string()),
        )),
        RunState::Completed => Ok(RunResult {
            run_id,
            session_id,
            state,
            text: assistant_text(&output),
            output,
            usage,
        }),
        _ => Err(AifluxonError::new(
            AifluxonErrorKind::Internal,
            "The run event stream ended without a terminal outcome.",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aifluxon_core::ContentPart;

    #[test]
    fn assistant_text_uses_the_last_assistant_message() {
        let output = vec![
            Message {
                role: MessageRole::User,
                content: vec![ContentPart::Text("hello".to_string())],
                tool_calls: Vec::new(),
                tool_call_id: None,
                provider_state: None,
            },
            Message {
                role: MessageRole::Assistant,
                content: vec![ContentPart::Text("one".to_string())],
                tool_calls: Vec::new(),
                tool_call_id: None,
                provider_state: None,
            },
            Message {
                role: MessageRole::Assistant,
                content: vec![ContentPart::Text("two".to_string())],
                tool_calls: Vec::new(),
                tool_call_id: None,
                provider_state: None,
            },
        ];
        assert_eq!(assistant_text(&output), "two");
    }
}
