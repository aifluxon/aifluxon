#![allow(dead_code)]

use aifluxon_core::{ToolCall, ToolInvocationId};
use serde_json::Value;

#[derive(Default)]
struct PendingToolCall {
    id: String,
    name: String,
    arguments: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StreamDeltaKind {
    UsageOnly,
    Reasoning,
    VisibleAnswer,
    ToolCall,
    Empty,
}

#[derive(Default)]
pub struct ToolCallAssembler {
    calls: Vec<PendingToolCall>,
}

impl ToolCallAssembler {
    pub fn apply_chat_delta(
        &mut self,
        delta_tool_call: &Value,
        fallback_position: usize,
        is_snapshot: bool,
    ) {
        let index = streamed_tool_call_index(delta_tool_call, fallback_position, self.calls.len());
        while self.calls.len() <= index {
            self.calls.push(PendingToolCall::default());
        }
        let accumulator = &mut self.calls[index];
        if let Some(id) = delta_tool_call.get("id").and_then(Value::as_str) {
            if is_snapshot {
                accumulator.id = id.to_string();
            } else {
                accumulator.id.push_str(id);
            }
        }
        if let Some(function) = delta_tool_call.get("function") {
            if let Some(name) = function.get("name").and_then(Value::as_str) {
                if is_snapshot {
                    accumulator.name = name.to_string();
                } else {
                    accumulator.name.push_str(name);
                }
            }
            if let Some(arguments) = function.get("arguments").and_then(Value::as_str) {
                if is_snapshot {
                    accumulator.arguments = arguments.to_string();
                } else {
                    accumulator.arguments.push_str(arguments);
                }
            }
        }
        if let Some(name) = delta_tool_call.get("name").and_then(Value::as_str) {
            if is_snapshot {
                accumulator.name = name.to_string();
            } else if accumulator.name.is_empty() {
                accumulator.name.push_str(name);
            }
        }
        if let Some(arguments) = delta_tool_call.get("arguments").and_then(Value::as_str) {
            if is_snapshot {
                accumulator.arguments = arguments.to_string();
            } else if delta_tool_call.get("function").is_none() {
                accumulator.arguments.push_str(arguments);
            }
        }
    }

    pub fn finish(self) -> Vec<ToolCall> {
        self.calls
            .into_iter()
            .map(|call| {
                let provider_call_id = if call.id.trim().is_empty() {
                    None
                } else {
                    Some(call.id.clone())
                };
                let stable_key = provider_call_id
                    .clone()
                    .unwrap_or_else(|| format!("{}:{}", call.name, call.arguments));
                let arguments = serde_json::from_str(&call.arguments)
                    .unwrap_or_else(|_| Value::String(call.arguments));
                ToolCall {
                    id: ToolInvocationId::from_stable_key(&stable_key),
                    name: call.name,
                    arguments,
                    provider_call_id,
                }
            })
            .collect()
    }
}

pub fn streamed_tool_call_index(
    delta_tool_call: &Value,
    fallback_position: usize,
    tool_calls_len: usize,
) -> usize {
    delta_tool_call
        .get("index")
        .and_then(Value::as_u64)
        .map(|index| index as usize)
        .unwrap_or_else(|| {
            if fallback_position < tool_calls_len {
                fallback_position
            } else {
                tool_calls_len
            }
        })
}

pub fn classify_chat_completion_chunk(value: &Value) -> StreamDeltaKind {
    let delta = value
        .pointer("/choices/0/delta")
        .or_else(|| value.get("delta"));
    let has_tool_calls = delta
        .and_then(|delta| delta.get("tool_calls"))
        .and_then(Value::as_array)
        .is_some_and(|calls| !calls.is_empty());
    if has_tool_calls {
        return StreamDeltaKind::ToolCall;
    }
    let content = delta
        .and_then(|delta| delta.get("content"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let reasoning = delta
        .and_then(|delta| {
            delta
                .get("reasoning_content")
                .or_else(|| delta.get("reasoning"))
        })
        .and_then(Value::as_str)
        .unwrap_or("");
    if !reasoning.trim().is_empty() && content.trim().is_empty() {
        return StreamDeltaKind::Reasoning;
    }
    if !content.trim().is_empty() {
        return StreamDeltaKind::VisibleAnswer;
    }
    if value.get("usage").is_some() {
        return StreamDeltaKind::UsageOnly;
    }
    StreamDeltaKind::Empty
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bug_agent_023_tool_argument_deltas_assemble_in_order() {
        let mut assembler = ToolCallAssembler::default();
        assembler.apply_chat_delta(
            &json!({ "index": 0, "id": "call_", "function": { "name": "she" } }),
            0,
            false,
        );
        assembler.apply_chat_delta(
            &json!({ "index": 1, "id": "call_b", "function": { "name": "read_file" } }),
            1,
            false,
        );
        assembler.apply_chat_delta(
            &json!({ "index": 0, "id": "a", "function": { "name": "ll", "arguments": "{\"com" } }),
            0,
            false,
        );
        assembler.apply_chat_delta(
            &json!({ "index": 0, "function": { "arguments": "mand\":\"echo \\\"{ok}\\\"\"}" } }),
            0,
            false,
        );
        assembler.apply_chat_delta(
            &json!({ "index": 1, "function": { "arguments": "{\"path\":\"src/lib.rs\"}" } }),
            1,
            false,
        );
        let calls = assembler.finish();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "shell");
        assert_eq!(calls[0].arguments, json!({ "command": "echo \"{ok}\"" }));
        assert_eq!(calls[1].name, "read_file");
        assert_eq!(calls[1].arguments, json!({ "path": "src/lib.rs" }));
        assert_ne!(calls[0].id, calls[1].id);
        assert_eq!(calls[0].provider_call_id.as_deref(), Some("call_a"));
        assert_eq!(calls[1].provider_call_id.as_deref(), Some("call_b"));
    }

    #[test]
    fn bug_agent_024_usage_only_deltas_do_not_fake_activity() {
        let kind = classify_chat_completion_chunk(&json!({
            "choices": [{ "delta": {}, "finish_reason": null }],
            "usage": { "prompt_tokens": 12, "completion_tokens": 0 }
        }));
        assert_eq!(kind, StreamDeltaKind::UsageOnly);
        assert_ne!(kind, StreamDeltaKind::VisibleAnswer);
        assert_ne!(kind, StreamDeltaKind::Reasoning);
    }

    #[test]
    fn bug_agent_025_reasoning_deltas_do_not_leak_as_answers() {
        let kind = classify_chat_completion_chunk(&json!({
            "choices": [{
                "delta": { "reasoning_content": "I should inspect the file first." }
            }]
        }));
        assert_eq!(kind, StreamDeltaKind::Reasoning);
        assert_ne!(kind, StreamDeltaKind::VisibleAnswer);
    }
}
