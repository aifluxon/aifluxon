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
                    let mut wire = json!({
                        "id": call.wire_call_id(),
                        "type": "function",
                        "function": {
                            "name": call.name,
                            "arguments": call.arguments.to_string(),
                        }
                    });
                    if let Some(extra) = message
                        .provider_state
                        .as_ref()
                        .and_then(|state| state.get("tool_call_extra_content"))
                        .and_then(|metadata| metadata.get(call.wire_call_id()))
                        .filter(|extra| extra.is_object())
                    {
                        wire["extra_content"] = extra.clone();
                    }
                    wire
                })
                .collect(),
        );
    }
    if message.role == MessageRole::Assistant {
        if let Some(extra) = message
            .provider_state
            .as_ref()
            .and_then(|state| state.get("extra_content"))
            .filter(|extra| extra.is_object())
        {
            wire["extra_content"] = extra.clone();
        }
    }
    if message.role == MessageRole::Tool {
        if let Some(tool_call_id) = tool_message_call_id(message) {
            wire["tool_call_id"] = json!(tool_call_id);
        }
    }
    wire
}

fn tool_message_call_id(message: &Message) -> Option<String> {
    message
        .provider_state
        .as_ref()
        .and_then(|state| state.get("call_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .or_else(|| message.tool_call_id.map(|id| id.hyphenated()))
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
    extra_content: Value,
    gemini_thoughts: Option<crate::gemini::ThoughtStream>,
}

impl ChatCompletionsTurnAssembler {
    pub(crate) fn use_gemini_contract(&mut self) {
        self.tools.use_gemini_contract();
        self.gemini_thoughts.get_or_insert_with(Default::default);
    }

    fn emit_content(
        &mut self,
        visible: &str,
        reasoning: &str,
        snapshot: bool,
        sink: &dyn ModelEventSink,
    ) {
        if !reasoning.is_empty() {
            if let Some(delta) =
                self.reasoning
                    .push_compatible(reasoning, snapshot, self.allow_cumulative_delta)
            {
                sink.on_reasoning_delta(&delta);
            }
        }
        if !visible.is_empty() {
            if let Some(delta) =
                self.text
                    .push_compatible(visible, snapshot, self.allow_cumulative_delta)
            {
                sink.on_text_delta(&delta);
            }
        }
    }

    pub(crate) fn flush_content(&mut self, sink: &dyn ModelEventSink) {
        if let Some(parser) = &mut self.gemini_thoughts {
            let (visible, reasoning) = parser.finish();
            self.emit_content(&visible, &reasoning, false, sink);
        }
    }
    pub(crate) fn use_deepseek_contract(&mut self) {
        self.tools.use_metadata_snapshots();
    }

    pub fn finish_checked(self) -> Result<ModelTurn, aifluxon_core::ProviderError> {
        use aifluxon_core::ProviderError;
        match self.finish_reason.as_deref() {
            Some("stop" | "tool_calls") => {}
            Some("length") => return Err(ProviderError::message(
                "PROVIDER_OUTPUT_LIMIT: Output token limit reached; no tools from this truncated response were dispatched.",
            )),
            Some("insufficient_system_resource") => return Err(ProviderError::message(
                "PROVIDER_RESOURCE_UNAVAILABLE: Provider could not complete this response.",
            )),
            Some(_) => return Err(ProviderError::message(
                "PROVIDER_FINISH_ERROR: Provider did not return a successful finish reason.",
            )),
            None => return Err(ProviderError::message(
                "PROVIDER_STREAM_CLOSED: Response ended without a finish reason.",
            )),
        }
        self.tools.validate()?;
        let turn = self.finish();
        super::reliability::validate_turn(&turn)?;
        Ok(turn)
    }

    pub fn new(allow_cumulative_delta: bool) -> Self {
        Self {
            allow_cumulative_delta,
            text: TextDeltaReconciler::default(),
            reasoning: TextDeltaReconciler::default(),
            tools: ToolCallAssembler::default(),
            usage: None,
            finish_reason: None,
            extra_content: json!({}),
            gemini_thoughts: None,
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
        if let Some(extra) = delta_value
            .get("extra_content")
            .filter(|value| value.is_object())
        {
            if extra.get("google").is_some() {
                self.use_gemini_contract();
            }
            let mut replay_extra = extra.clone();
            if let Some(google) = replay_extra
                .get_mut("google")
                .and_then(Value::as_object_mut)
            {
                google.remove("thought"); // This marks a streamed summary, not replay state.
                if google.is_empty() {
                    replay_extra.as_object_mut().unwrap().remove("google");
                }
            }
            crate::gemini::merge_extra_content(&mut self.extra_content, &replay_extra);
        }

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

        if let Some(content) = delta_value
            .get("content")
            .and_then(Value::as_str)
            .filter(|content| !content.is_empty())
            .or_else(|| delta_value.get("refusal").and_then(Value::as_str))
        {
            if let Some(parser) = &mut self.gemini_thoughts {
                if is_snapshot {
                    *parser = Default::default();
                }
                let (visible, reasoning) = parser.push(
                    content,
                    delta_value
                        .pointer("/extra_content/google/thought")
                        .and_then(Value::as_bool),
                );
                self.emit_content(&visible, &reasoning, is_snapshot, sink);
            } else {
                self.emit_content(content, "", is_snapshot, sink);
            }
        }

        if let Some(delta_tool_calls) = delta_value.get("tool_calls").and_then(Value::as_array) {
            if delta_tool_calls
                .iter()
                .any(|call| call.pointer("/extra_content/google").is_some())
            {
                self.use_gemini_contract();
            }
            for (fallback_position, delta_tool_call) in delta_tool_calls.iter().enumerate() {
                self.tools
                    .apply_chat_delta(delta_tool_call, fallback_position, is_snapshot);
            }
        }
    }

    pub fn finish(mut self) -> ModelTurn {
        self.flush_content(&aifluxon_core::NoopModelEventSink);
        let tool_metadata = self.tools.extra_content_by_call_id();
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
        if tool_metadata
            .as_object()
            .is_some_and(|metadata| !metadata.is_empty())
        {
            opaque["tool_call_extra_content"] = tool_metadata;
        }
        if self
            .extra_content
            .as_object()
            .is_some_and(|extra| !extra.is_empty())
        {
            opaque["extra_content"] = self.extra_content;
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
