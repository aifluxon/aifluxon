#![allow(dead_code)]

use serde_json::Value;

pub const OPAQUE_CONTINUATION_FIELDS: &[&str] = &[
    "reasoning_content",
    "responses_replay_items",
    "response_items",
    "hidden_context",
    "thought_signature",
    "context_signature",
    "conversation_cursor",
    "parent_message_id",
    "binding_generation",
];

#[derive(Clone, Debug, Default)]
pub struct ContextLayers {
    pub stable_prefix: Vec<Value>,
    pub conversation: Vec<Value>,
    pub dynamic: Vec<Value>,
}

pub fn split_context_layers(messages: &[Value]) -> ContextLayers {
    let mut layers = ContextLayers::default();
    let mut in_prefix = true;
    for message in messages {
        let role = message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if in_prefix && role == "system" {
            layers.stable_prefix.push(message.clone());
            continue;
        }
        in_prefix = false;
        if role == "tool" || message.get("tool_call_id").is_some() {
            layers.dynamic.push(message.clone());
        } else {
            layers.conversation.push(message.clone());
        }
    }
    layers
}

pub fn stable_prefix_hash(messages: &[Value]) -> String {
    let prefix = split_context_layers(messages).stable_prefix;
    let text = serde_json::to_string(&serde_json::json!({ "messages": prefix }))
        .unwrap_or_else(|_| prefix.len().to_string());
    aifluxon_core::content_hash(&text)
}

pub fn context_contains_runtime_metadata(messages: &[Value]) -> bool {
    messages.iter().any(message_contains_runtime_metadata)
}

fn message_contains_runtime_metadata(message: &Value) -> bool {
    let content = message
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default();
    [
        "runId",
        "turnId",
        "operationId",
        "\"timestamp\"",
        "telemetry",
    ]
    .iter()
    .any(|needle| content.contains(needle))
}

pub fn prune_preserving_tool_pairs(messages: &[Value], max_messages: usize) -> Vec<Value> {
    if messages.len() <= max_messages {
        return messages.to_vec();
    }
    let layers = split_context_layers(messages);
    let prefix_len = layers.stable_prefix.len();
    let rest = &messages[prefix_len..];
    if rest.len() <= max_messages.saturating_sub(prefix_len) {
        return messages.to_vec();
    }
    let keep_rest = max_messages.saturating_sub(prefix_len).max(1);
    let mut start = rest.len().saturating_sub(keep_rest);
    while start > 0 && is_tool_result(&rest[start]) {
        start -= 1;
        if is_assistant_tool_call(&rest[start]) {
            break;
        }
    }
    if start < rest.len() && is_tool_result(&rest[start]) {
        start = (start + 1).min(rest.len());
        while start < rest.len() && is_tool_result(&rest[start]) {
            start += 1;
        }
    }
    let mut kept = layers.stable_prefix;
    kept.extend(rest[start..].iter().cloned());
    kept
}

fn is_tool_result(message: &Value) -> bool {
    message.get("role").and_then(Value::as_str) == Some("tool")
        || message.get("tool_call_id").is_some()
}

fn is_assistant_tool_call(message: &Value) -> bool {
    message.get("role").and_then(Value::as_str) == Some("assistant")
        && message
            .get("tool_calls")
            .and_then(Value::as_array)
            .is_some_and(|calls| !calls.is_empty())
}

pub fn preserve_opaque_continuation(from: &Value, into: &mut Value) {
    for field in OPAQUE_CONTINUATION_FIELDS {
        if let Some(value) = from.get(*field) {
            into[*field] = value.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bug_agent_027_dynamic_runtime_metadata_stays_out_of_stable_prefix() {
        let first = vec![
            json!({ "role": "system", "content": "stable host policies" }),
            json!({ "role": "user", "content": "hello" }),
        ];
        let second = vec![
            json!({ "role": "system", "content": "stable host policies" }),
            json!({ "role": "user", "content": "hello again" }),
            json!({ "role": "system", "content": "runId=not-in-prefix timestamp=now" }),
        ];
        assert_eq!(stable_prefix_hash(&first), stable_prefix_hash(&second));
        assert!(!context_contains_runtime_metadata(&first));
        assert!(split_context_layers(&second)
            .stable_prefix
            .iter()
            .all(|message| !message_contains_runtime_metadata(message)));
    }

    #[test]
    fn bug_agent_028_context_pruning_keeps_tool_call_and_result_together() {
        let messages = vec![
            json!({ "role": "system", "content": "stable" }),
            json!({ "role": "user", "content": "old" }),
            json!({
                "role": "assistant",
                "content": null,
                "tool_calls": [{ "id": "call-1", "function": { "name": "shell" } }]
            }),
            json!({ "role": "tool", "tool_call_id": "call-1", "content": "ok" }),
            json!({ "role": "user", "content": "latest" }),
        ];
        let pruned = prune_preserving_tool_pairs(&messages, 4);
        let has_call = pruned.iter().any(is_assistant_tool_call);
        let has_result = pruned.iter().any(is_tool_result);
        assert_eq!(has_call, has_result);
        assert!(has_call);
        assert!(pruned
            .iter()
            .any(|message| message.get("content") == Some(&json!("latest"))));
    }

    #[test]
    fn bug_agent_029_opaque_continuation_metadata_round_trips() {
        let from = json!({
            "reasoning_content": "think",
            "thought_signature": "gemini-sig",
            "responses_replay_items": [{ "type": "reasoning" }],
            "context_signature": "chatgpt-cursor",
            "conversation_cursor": "parent-1",
            "parent_message_id": "msg-9",
            "binding_generation": 4,
            "content": "visible"
        });
        let mut into = json!({ "content": "visible" });
        preserve_opaque_continuation(&from, &mut into);
        for field in OPAQUE_CONTINUATION_FIELDS {
            assert_eq!(
                into.get(*field),
                from.get(*field),
                "{field} must round-trip"
            );
        }
        assert_eq!(into["content"], "visible");
    }
}
