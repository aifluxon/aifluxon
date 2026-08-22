use super::tools::descriptor_to_openai_tool;
use crate::common::{TextDeltaReconciler, ToolCallAssembler};
use aifluxon_core::{
    ContentPart, Message, MessageRole, ModelEventSink, ModelTurn, ModelTurnRequest,
    ProviderTerminal, ToolCall,
};
use serde_json::{json, Value};

pub fn build_chat_completions_body(request: &ModelTurnRequest) -> Value {
    let mut body = json!({
        "model": request.model,
        "messages": request.messages.iter().map(message_to_wire).collect::<Vec<_>>(),
        "stream": true,
        "stream_options": { "include_usage": true },
    });
    if !request.tools.is_empty() {
        body["tools"] = Value::Array(
            request
                .tools
                .iter()
                .map(descriptor_to_openai_tool)
                .collect(),
        );
        body["tool_choice"] = json!("auto");
    }
    body
}

pub(crate) fn message_to_wire(message: &Message) -> Value {
    let role = match message.role {
        MessageRole::System => "system",
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::Tool => "tool",
    };
    let mut wire = json!({
        "role": role,
        "content": content_to_wire(message),
    });
    if !message.tool_calls.is_empty() {
        wire["tool_calls"] = Value::Array(
            message
                .tool_calls
                .iter()
                .map(|call| {
                    json!({
                        "id": call.id.hyphenated(),
                        "type": "function",
                        "function": {
                            "name": call.name,
                            "arguments": call.arguments.to_string(),
                        }
                    })
                })
                .collect(),
        );
    }
    if let Some(tool_call_id) = message.tool_call_id {
        wire["tool_call_id"] = json!(tool_call_id.hyphenated());
    }
    wire
}

fn content_to_wire(message: &Message) -> Value {
    if message.content.len() == 1 {
        if let ContentPart::Text(text) = &message.content[0] {
            return json!(text);
        }
    }
    Value::Array(
        message
            .content
            .iter()
            .map(|part| match part {
                ContentPart::Text(text) => json!({ "type": "text", "text": text }),
                ContentPart::Image(image) => image_to_chat_wire(image),
            })
            .collect(),
    )
}

pub(crate) fn image_is_file_id(reference: &str) -> bool {
    reference.trim().starts_with("file-")
}

pub(crate) fn image_to_chat_wire(image: &aifluxon_core::ImageContent) -> Value {
    if image_is_file_id(image.artifact.as_str()) {
        json!({
            "type": "file",
            "file_id": image.artifact.as_str(),
        })
    } else {
        json!({
            "type": "image_url",
            "image_url": { "url": image.artifact.as_str() },
        })
    }
}

pub struct ChatCompletionsTurnAssembler {
    allow_cumulative_delta: bool,
    text: TextDeltaReconciler,
    reasoning: TextDeltaReconciler,
    tools: ToolCallAssembler,
    usage: Option<Value>,
    finish_reason: Option<String>,
}

impl ChatCompletionsTurnAssembler {
    pub fn new(allow_cumulative_delta: bool) -> Self {
        Self {
            allow_cumulative_delta,
            text: TextDeltaReconciler::default(),
            reasoning: TextDeltaReconciler::default(),
            tools: ToolCallAssembler::default(),
            usage: None,
            finish_reason: None,
        }
    }

    pub fn apply_value(&mut self, value: &Value, sink: &dyn ModelEventSink) {
        if let Some(usage) = usage_value(value) {
            self.usage = Some(usage.clone());
            sink.on_usage(&usage);
        }

        let Some(choice) = value
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
        else {
            return;
        };
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            if !reason.is_empty() && reason != "null" {
                self.finish_reason = Some(reason.to_string());
            }
        }

        let is_snapshot = choice.get("delta").is_none();
        let Some(delta_value) = choice.get("delta").or_else(|| choice.get("message")) else {
            return;
        };

        if let Some(reasoning_delta) = delta_value
            .get("reasoning_content")
            .or_else(|| delta_value.get("reasoning"))
            .and_then(Value::as_str)
        {
            if let Some(delta) = self.reasoning.push_compatible(
                reasoning_delta,
                is_snapshot,
                self.allow_cumulative_delta,
            ) {
                sink.on_reasoning_delta(&delta);
            }
        }

        if let Some(content) = delta_value.get("content").and_then(Value::as_str) {
            if let Some(delta) =
                self.text
                    .push_compatible(content, is_snapshot, self.allow_cumulative_delta)
            {
                sink.on_text_delta(&delta);
            }
        }

        if let Some(delta_tool_calls) = delta_value.get("tool_calls").and_then(Value::as_array) {
            for (fallback_position, delta_tool_call) in delta_tool_calls.iter().enumerate() {
                self.tools
                    .apply_chat_delta(delta_tool_call, fallback_position, is_snapshot);
            }
        }
    }

    pub fn finish(self) -> ModelTurn {
        let tool_calls = finished_tool_calls(self.tools);
        let terminal = if !tool_calls.is_empty()
            || matches!(
                self.finish_reason.as_deref(),
                Some("tool_calls" | "function_call")
            ) {
            ProviderTerminal::ToolCalls
        } else {
            ProviderTerminal::Stop
        };
        let reasoning = self.reasoning.emitted().to_string();
        let mut opaque = json!({
            "protocol": "chat_completions",
            "finish_reason": self.finish_reason,
        });
        if !reasoning.is_empty() {
            opaque["reasoning_content"] = json!(reasoning);
        }
        ModelTurn {
            text: self.text.emitted().to_string(),
            reasoning,
            tool_calls,
            usage: self.usage,
            terminal,
            opaque,
        }
    }
}

pub(crate) fn finished_tool_calls(assembler: ToolCallAssembler) -> Vec<ToolCall> {
    assembler
        .finish()
        .into_iter()
        .filter(|call| !call.name.trim().is_empty())
        .collect()
}

pub(crate) fn usage_value(value: &Value) -> Option<Value> {
    find_usage_value(value)
}

fn find_usage_value(value: &Value) -> Option<Value> {
    let usage = value.get("usage").filter(|usage| {
        usage.is_object()
            && !usage
                .as_object()
                .map(|object| object.is_empty())
                .unwrap_or(false)
    });
    if let Some(usage) = usage {
        return Some(usage.clone());
    }

    match value {
        Value::Array(items) => items.iter().find_map(find_usage_value),
        Value::Object(object) => object
            .iter()
            .filter(|(key, _)| key.as_str() != "raw_usage" && key.as_str() != "rawUsage")
            .find_map(|(_, nested)| find_usage_value(nested)),
        _ => None,
    }
}
