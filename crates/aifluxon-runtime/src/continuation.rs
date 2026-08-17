use aifluxon_core::{
    ContentPart, ContinuationReason, Message, MessageRole, ModelTurn, ModelTurnRequest,
};

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ContinuationCounts {
    incomplete: u32,
    summary_only: u32,
    provider_requested: u32,
}

impl ContinuationCounts {
    pub(crate) fn try_begin(&mut self, reason: ContinuationReason) -> bool {
        let slot = match reason {
            ContinuationReason::Incomplete => &mut self.incomplete,
            ContinuationReason::SummaryOnly => &mut self.summary_only,
            ContinuationReason::ProviderRequested => &mut self.provider_requested,
        };
        if *slot >= reason.default_limit() {
            return false;
        }
        *slot += 1;
        true
    }

    pub(crate) fn reset_incomplete(&mut self) {
        self.incomplete = 0;
    }
}

pub(crate) fn apply_continuation(
    request: &mut ModelTurnRequest,
    turn: &ModelTurn,
    reason: ContinuationReason,
) {
    request.opaque_state = Some(turn.opaque.clone());
    match reason {
        ContinuationReason::Incomplete => {
            request.messages.push(assistant_continuation_message(turn));
            request.messages.push(continuation_prompt(reason, turn));
        }
        ContinuationReason::SummaryOnly => {
            request.messages.push(continuation_prompt(reason, turn));
        }
        ContinuationReason::ProviderRequested => {
            if !turn.text.trim().is_empty() || has_replay_items(turn) {
                request.messages.push(assistant_continuation_message(turn));
            } else {
                request.messages.push(continuation_prompt(reason, turn));
            }
        }
    }
}

fn assistant_continuation_message(turn: &ModelTurn) -> Message {
    Message {
        role: MessageRole::Assistant,
        content: (!turn.text.is_empty())
            .then(|| ContentPart::Text(turn.text.clone()))
            .into_iter()
            .collect(),
        tool_calls: turn.tool_calls.clone(),
        tool_call_id: None,
        provider_state: (!turn.opaque.is_null()).then(|| turn.opaque.clone()),
    }
}

fn continuation_prompt(reason: ContinuationReason, turn: &ModelTurn) -> Message {
    Message {
        role: MessageRole::System,
        content: vec![ContentPart::Text(continuation_prompt_text(reason, turn))],
        tool_calls: Vec::new(),
        tool_call_id: None,
        provider_state: None,
    }
}

fn continuation_prompt_text(reason: ContinuationReason, turn: &ModelTurn) -> String {
    match reason {
        ContinuationReason::Incomplete => {
            "The previous assistant message promised or prepared to inspect, search, read, run, verify, or edit, but it did not include a tool call. Continue the same user request now by calling the necessary tool or tools. Do not restate intent and do not give a final answer until the required tool work is actually complete."
                .to_string()
        }
        ContinuationReason::SummaryOnly => {
            let hidden = turn
                .opaque
                .get("hidden_context")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .trim();
            format!(
                "The previous response contained only an internal summary, not a user-facing answer. Treat the following summary as hidden context and continue the same user request by giving a concise final answer. Do not include internal recap sections in the user-visible answer.\n\nHidden summary:\n{hidden}"
            )
        }
        ContinuationReason::ProviderRequested => {
            "Continue the same turn; the previous response was not terminal.".to_string()
        }
    }
}

fn has_replay_items(turn: &ModelTurn) -> bool {
    turn.opaque
        .get("response_items")
        .or_else(|| turn.opaque.get("responses_replay_items"))
        .and_then(serde_json::Value::as_array)
        .is_some_and(|items| !items.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aifluxon_core::ProviderTerminal;
    use serde_json::json;

    fn turn(text: &str, opaque: serde_json::Value) -> ModelTurn {
        ModelTurn {
            text: text.to_string(),
            reasoning: String::new(),
            tool_calls: Vec::new(),
            usage: None,
            terminal: ProviderTerminal::Continue(ContinuationReason::Incomplete),
            opaque,
        }
    }

    #[test]
    fn continuation_prompts_are_product_neutral() {
        for reason in [
            ContinuationReason::Incomplete,
            ContinuationReason::SummaryOnly,
            ContinuationReason::ProviderRequested,
        ] {
            let text = continuation_prompt_text(reason, &turn("", json!({})));
            let lower = text.to_ascii_lowercase();
            assert!(!lower.contains("qwen"));
            assert!(!lower.contains("codex"));
            assert!(!lower.contains("end_turn"));
        }
    }

    #[test]
    fn continuation_counts_are_bounded_per_reason() {
        let mut counts = ContinuationCounts::default();
        assert!(counts.try_begin(ContinuationReason::Incomplete));
        assert!(counts.try_begin(ContinuationReason::Incomplete));
        assert!(!counts.try_begin(ContinuationReason::Incomplete));
        assert!(counts.try_begin(ContinuationReason::SummaryOnly));
        assert!(!counts.try_begin(ContinuationReason::SummaryOnly));
        for _ in 0..4 {
            assert!(counts.try_begin(ContinuationReason::ProviderRequested));
        }
        assert!(!counts.try_begin(ContinuationReason::ProviderRequested));
    }
}
