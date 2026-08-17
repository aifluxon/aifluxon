use super::chat_completions::{finished_tool_calls, message_to_wire, usage_value};
use super::tools::descriptor_to_openai_tool;
use crate::common::{reconcile_terminal_text, TextDeltaReconciler, ToolCallAssembler};
use aifluxon_core::{
    Message, ModelEventSink, ModelTurn, ModelTurnRequest, ProviderError, ProviderTerminal,
};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

pub fn build_responses_body(request: &ModelTurnRequest) -> Value {
    let mut input = Vec::new();
    for message in &request.messages {
        if let Some(items) = replay_items_from_message(message) {
            input.extend(items);
        }
        let has_content = !message.content.is_empty();
        let has_tools = !message.tool_calls.is_empty();
        if has_content || has_tools || message.tool_call_id.is_some() {
            input.push(message_to_wire(message));
        }
    }
    let mut body = json!({
        "model": request.model,
        "input": input,
        "stream": true,
        "store": false,
    });
    if !request.tools.is_empty() {
        body["tools"] = Value::Array(
            request
                .tools
                .iter()
                .map(|descriptor| {
                    let function = descriptor_to_openai_tool(descriptor);
                    json!({
                        "type": "function",
                        "name": function["function"]["name"],
                        "description": function["function"]["description"],
                        "parameters": function["function"]["parameters"],
                    })
                })
                .collect(),
        );
        body["tool_choice"] = json!("auto");
    }
    body
}

fn replay_items_from_message(message: &Message) -> Option<Vec<Value>> {
    message.provider_state.as_ref().and_then(|state| {
        state
            .get("response_items")
            .or_else(|| state.get("responses_replay_items"))
            .and_then(Value::as_array)
            .cloned()
    })
}

#[derive(Default)]
pub struct ReasoningPartState {
    inner: TextDeltaReconciler,
}

impl ReasoningPartState {
    fn push(&mut self, delta: &str, allow_cumulative: bool) -> Option<String> {
        self.inner.push_compatible(delta, false, allow_cumulative)
    }

    fn complete(&mut self, text: &str) -> Result<Option<String>, ProviderError> {
        self.inner.complete(text).map_err(ProviderError::message)
    }
}

#[derive(Default)]
pub struct OutputPartState {
    inner: TextDeltaReconciler,
}

impl OutputPartState {
    fn push(&mut self, delta: &str, allow_cumulative: bool) -> Option<String> {
        self.inner.push_compatible(delta, false, allow_cumulative)
    }

    fn complete(&mut self, text: &str) -> Result<Option<String>, ProviderError> {
        self.inner.complete(text).map_err(ProviderError::message)
    }
}

#[derive(Default)]
pub struct ReplayState {
    items: Vec<Value>,
}

impl ReplayState {
    fn upsert(&mut self, item: &Value) {
        let Some(key) = replay_item_key(item) else {
            if !self.items.iter().any(|existing| existing == item) {
                self.items.push(item.clone());
            }
            return;
        };
        if let Some(existing) = self
            .items
            .iter_mut()
            .find(|existing| replay_item_key(existing).as_deref() == Some(key.as_str()))
        {
            *existing = item.clone();
        } else {
            self.items.push(item.clone());
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeneratedArtifact {
    pub id: Option<String>,
    pub result: String,
    pub revised_prompt: Option<String>,
}

#[derive(Default)]
pub struct GeneratedArtifactState {
    images: Vec<GeneratedArtifact>,
    seen: HashSet<String>,
}

impl GeneratedArtifactState {
    fn capture(&mut self, item: &Value) {
        if item.get("type").and_then(Value::as_str) != Some("image_generation_call") {
            return;
        }
        let Some(result) = item
            .get("result")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|result| !result.is_empty())
        else {
            return;
        };
        let id = item
            .get("id")
            .or_else(|| item.get("call_id"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_string);
        let key = id.clone().unwrap_or_else(|| {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            result.hash(&mut hasher);
            format!("result-{:016x}", hasher.finish())
        });
        if !self.seen.insert(key) {
            return;
        }
        self.images.push(GeneratedArtifact {
            id,
            result: result.to_string(),
            revised_prompt: item
                .get("revised_prompt")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|prompt| !prompt.is_empty())
                .map(str::to_string),
        });
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ResponsesTerminalStatus {
    Completed,
    Incomplete,
    Failed,
}

pub struct ResponsesTurnAssembler {
    allow_cumulative_delta: bool,
    state: ResponsesTurnState,
}

pub struct ResponsesTurnState {
    last_sequence_number: Option<u64>,
    seen_event_keys: HashSet<String>,
    reasoning_parts: HashMap<String, ReasoningPartState>,
    reasoning_summary_parts: HashMap<String, ReasoningPartState>,
    output_parts: HashMap<String, OutputPartState>,
    tools: ToolCallAssembler,
    replay: ReplayState,
    artifacts: GeneratedArtifactState,
    usage: Option<Value>,
    terminal: Option<ResponsesTerminalStatus>,
    incomplete_reason: Option<String>,
    end_turn: Option<bool>,
    saw_output_done: bool,
    terminal_output_items: Vec<Value>,
    text: String,
    reasoning: String,
}

impl ResponsesTurnAssembler {
    pub fn new(allow_cumulative_delta: bool) -> Self {
        Self {
            allow_cumulative_delta,
            state: ResponsesTurnState {
                last_sequence_number: None,
                seen_event_keys: HashSet::new(),
                reasoning_parts: HashMap::new(),
                reasoning_summary_parts: HashMap::new(),
                output_parts: HashMap::new(),
                tools: ToolCallAssembler::default(),
                replay: ReplayState::default(),
                artifacts: GeneratedArtifactState::default(),
                usage: None,
                terminal: None,
                incomplete_reason: None,
                end_turn: None,
                saw_output_done: false,
                terminal_output_items: Vec::new(),
                text: String::new(),
                reasoning: String::new(),
            },
        }
    }

    pub fn apply_value(
        &mut self,
        value: &Value,
        sink: &dyn ModelEventSink,
    ) -> Result<(), ProviderError> {
        if !looks_like_responses_stream_value(value) {
            return Ok(());
        }
        if !self.accept_event(value)? {
            return Ok(());
        }

        if let Some(status) = terminal_status(value) {
            self.state.terminal = Some(status);
        }
        if value
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|event_type| {
                event_type == "response.failed"
                    || event_type == "response.cancelled"
                    || event_type == "error"
            })
        {
            let message = value
                .pointer("/response/error/message")
                .or_else(|| value.pointer("/error/message"))
                .or_else(|| value.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("Responses stream failed.");
            return Err(ProviderError::message(message));
        }

        if self.state.terminal.is_some() {
            let terminal = value.get("response").unwrap_or(value);
            if let Some(end_turn) = terminal.get("end_turn").and_then(Value::as_bool) {
                self.state.end_turn = Some(end_turn);
            }
            if self.state.terminal == Some(ResponsesTerminalStatus::Incomplete) {
                self.state.incomplete_reason = terminal
                    .pointer("/incomplete_details/reason")
                    .or_else(|| terminal.pointer("/error/message"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| Some("incomplete".to_string()));
            }
        }

        if let Some(usage) = usage_from_responses(value) {
            self.state.usage = Some(usage.clone());
            sink.on_usage(&usage);
        }

        let output_index = value
            .get("output_index")
            .and_then(Value::as_u64)
            .map(|index| index as usize);
        match value.get("type").and_then(Value::as_str) {
            Some("response.reasoning_summary_part.added") => {
                let text = value
                    .pointer("/part/text")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if !text.is_empty() {
                    self.push_reasoning_summary(value, text, sink);
                }
            }
            Some("response.reasoning_summary_text.delta") => {
                if let Some(delta) = value.get("delta").and_then(Value::as_str) {
                    self.push_reasoning_summary(value, delta, sink);
                }
            }
            Some("response.reasoning_summary_text.done") => {
                if let Some(text) = value.get("text").and_then(Value::as_str) {
                    self.complete_reasoning_summary(value, text, sink)?;
                }
            }
            Some("response.reasoning_text.delta") => {
                if let Some(delta) = value.get("delta").and_then(Value::as_str) {
                    self.push_reasoning(value, delta, sink);
                }
            }
            Some("response.reasoning_text.done") => {
                if let Some(text) = value.get("text").and_then(Value::as_str) {
                    self.complete_reasoning(value, text, sink)?;
                }
            }
            Some("response.output_text.delta" | "response.refusal.delta") => {
                if let Some(delta) = value.get("delta").and_then(Value::as_str) {
                    self.push_output(value, delta, sink);
                }
            }
            Some("response.output_text.done" | "response.refusal.done") => {
                self.state.saw_output_done = true;
                if let Some(text) = value
                    .get("text")
                    .or_else(|| value.get("refusal"))
                    .and_then(Value::as_str)
                {
                    self.complete_output(value, text, sink)?;
                }
            }
            Some("response.output_item.added") => {
                if let (Some(index), Some(item)) = (output_index, value.get("item")) {
                    self.apply_function_call_item(index, item, false);
                }
            }
            Some("response.function_call_arguments.delta") => {
                if let (Some(index), Some(delta)) =
                    (output_index, value.get("delta").and_then(Value::as_str))
                {
                    self.state.tools.apply_chat_delta(
                        &json!({ "index": index, "function": { "arguments": delta } }),
                        index,
                        false,
                    );
                }
            }
            Some("response.function_call_arguments.done") => {
                if let (Some(index), Some(arguments)) =
                    (output_index, value.get("arguments").and_then(Value::as_str))
                {
                    self.state.tools.apply_chat_delta(
                        &json!({ "index": index, "function": { "arguments": arguments } }),
                        index,
                        true,
                    );
                }
            }
            Some("response.output_item.done") => {
                if let (Some(index), Some(item)) = (output_index, value.get("item")) {
                    self.state.artifacts.capture(item);
                    match item.get("type").and_then(Value::as_str) {
                        Some("reasoning" | "web_search_call") => self.state.replay.upsert(item),
                        Some("function_call") => self.apply_function_call_item(index, item, true),
                        _ => {}
                    }
                }
            }
            _ => {
                if let Some(response) = terminal_snapshot(value) {
                    self.apply_terminal_snapshot(response, sink)?;
                }
            }
        }
        Ok(())
    }

    pub fn finish(self) -> Result<ModelTurn, ProviderError> {
        match self.state.terminal {
            Some(ResponsesTerminalStatus::Completed) => {}
            Some(ResponsesTerminalStatus::Incomplete) => {
                let reason = self
                    .state
                    .incomplete_reason
                    .unwrap_or_else(|| "incomplete".to_string());
                return Err(ProviderError::message(format!(
                    "Responses stream ended incomplete: {reason}"
                )));
            }
            Some(ResponsesTerminalStatus::Failed) => {
                return Err(ProviderError::message(
                    "Responses stream ended with a failed terminal.",
                ));
            }
            None => {
                return Err(ProviderError::message(
                    "Responses stream ended without an explicit terminal event.",
                ));
            }
        }

        let tool_calls = finished_tool_calls(self.state.tools);
        let generated_images = self
            .state
            .artifacts
            .images
            .iter()
            .map(|image| {
                json!({
                    "id": image.id,
                    "result": image.result,
                    "revised_prompt": image.revised_prompt,
                })
            })
            .collect::<Vec<_>>();
        Ok(ModelTurn {
            text: self.state.text,
            reasoning: self.state.reasoning,
            terminal: if tool_calls.is_empty() {
                ProviderTerminal::Stop
            } else {
                ProviderTerminal::ToolCalls
            },
            tool_calls,
            usage: self.state.usage,
            opaque: json!({
                "protocol": "responses",
                "response_items": self.state.replay.items,
                "terminal_output": self.state.terminal_output_items,
                "generated_images": generated_images,
                "end_turn": self.state.end_turn,
                "incomplete_reason": self.state.incomplete_reason,
            }),
        })
    }

    fn accept_event(&mut self, value: &Value) -> Result<bool, ProviderError> {
        if let Some(sequence) = value.get("sequence_number").and_then(Value::as_u64) {
            match self.state.last_sequence_number {
                Some(previous) if sequence == previous => return Ok(false),
                Some(previous) if sequence < previous => {
                    return Err(ProviderError::message(format!(
                        "Responses stream sequence_number {sequence} arrived after {previous}."
                    )));
                }
                _ => self.state.last_sequence_number = Some(sequence),
            }
            return Ok(true);
        }
        if let Some(key) = event_key(value) {
            return Ok(self.state.seen_event_keys.insert(key));
        }
        Ok(true)
    }

    fn push_reasoning_summary(&mut self, value: &Value, delta: &str, sink: &dyn ModelEventSink) {
        let key = text_part_key(value);
        let allow = self.allow_cumulative_delta;
        if let Some(delta) = self
            .state
            .reasoning_summary_parts
            .entry(key)
            .or_default()
            .push(delta, allow)
        {
            self.state.reasoning.push_str(&delta);
            sink.on_reasoning_delta(&delta);
        }
    }

    fn complete_reasoning_summary(
        &mut self,
        value: &Value,
        text: &str,
        sink: &dyn ModelEventSink,
    ) -> Result<(), ProviderError> {
        let key = text_part_key(value);
        if let Some(suffix) = self
            .state
            .reasoning_summary_parts
            .entry(key)
            .or_default()
            .complete(text)?
        {
            self.state.reasoning.push_str(&suffix);
            sink.on_reasoning_delta(&suffix);
        }
        Ok(())
    }

    fn push_reasoning(&mut self, value: &Value, delta: &str, sink: &dyn ModelEventSink) {
        let key = text_part_key(value);
        let allow = self.allow_cumulative_delta;
        if let Some(delta) = self
            .state
            .reasoning_parts
            .entry(key)
            .or_default()
            .push(delta, allow)
        {
            self.state.reasoning.push_str(&delta);
            sink.on_reasoning_delta(&delta);
        }
    }

    fn complete_reasoning(
        &mut self,
        value: &Value,
        text: &str,
        sink: &dyn ModelEventSink,
    ) -> Result<(), ProviderError> {
        let key = text_part_key(value);
        if let Some(suffix) = self
            .state
            .reasoning_parts
            .entry(key)
            .or_default()
            .complete(text)?
        {
            self.state.reasoning.push_str(&suffix);
            sink.on_reasoning_delta(&suffix);
        }
        Ok(())
    }

    fn push_output(&mut self, value: &Value, delta: &str, sink: &dyn ModelEventSink) {
        let key = text_part_key(value);
        let allow = self.allow_cumulative_delta;
        if let Some(delta) = self
            .state
            .output_parts
            .entry(key)
            .or_default()
            .push(delta, allow)
        {
            self.state.text.push_str(&delta);
            sink.on_text_delta(&delta);
        }
    }

    fn complete_output(
        &mut self,
        value: &Value,
        text: &str,
        sink: &dyn ModelEventSink,
    ) -> Result<(), ProviderError> {
        let key = text_part_key(value);
        if let Some(suffix) = self
            .state
            .output_parts
            .entry(key)
            .or_default()
            .complete(text)?
        {
            self.state.text.push_str(&suffix);
            sink.on_text_delta(&suffix);
        }
        Ok(())
    }

    fn apply_function_call_item(&mut self, index: usize, item: &Value, snapshot: bool) {
        if item.get("type").and_then(Value::as_str) != Some("function_call") {
            return;
        }
        self.state.tools.apply_chat_delta(
            &json!({
                "index": index,
                "id": item.get("call_id").and_then(Value::as_str).unwrap_or_default(),
                "function": {
                    "name": item.get("name").and_then(Value::as_str).unwrap_or_default(),
                    "arguments": item.get("arguments").and_then(Value::as_str).unwrap_or_default(),
                }
            }),
            index,
            snapshot,
        );
    }

    fn apply_terminal_snapshot(
        &mut self,
        response: &Value,
        sink: &dyn ModelEventSink,
    ) -> Result<(), ProviderError> {
        if let Some(output) = response.get("output").and_then(Value::as_array) {
            self.state.terminal_output_items = output.clone();
        }
        let complete_text = extract_responses_output_text(response);
        match reconcile_terminal_text(&self.state.text, &complete_text, self.state.saw_output_done)
        {
            Ok(Some(suffix)) => {
                self.state.text.push_str(&suffix);
                sink.on_text_delta(&suffix);
            }
            Ok(None) => {}
            Err(message) => return Err(ProviderError::message(message)),
        }
        merge_snapshot_items(
            response,
            &mut self.state.tools,
            &mut self.state.replay,
            &mut self.state.artifacts,
        );
        Ok(())
    }
}

fn merge_snapshot_items(
    response: &Value,
    tools: &mut ToolCallAssembler,
    replay: &mut ReplayState,
    artifacts: &mut GeneratedArtifactState,
) {
    let Some(output) = response.get("output").and_then(Value::as_array) else {
        return;
    };
    for (index, item) in output.iter().enumerate() {
        artifacts.capture(item);
        match item.get("type").and_then(Value::as_str) {
            Some("reasoning" | "web_search_call") => replay.upsert(item),
            Some("function_call") => {
                tools.apply_chat_delta(
                    &json!({
                        "index": index,
                        "id": item.get("call_id").and_then(Value::as_str).unwrap_or_default(),
                        "function": {
                            "name": item.get("name").and_then(Value::as_str).unwrap_or_default(),
                            "arguments": item.get("arguments").and_then(Value::as_str).unwrap_or_default(),
                        }
                    }),
                    index,
                    true,
                );
            }
            _ => {}
        }
    }
}

pub fn looks_like_responses_stream_value(value: &Value) -> bool {
    value
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|event_type| event_type == "error" || event_type.starts_with("response."))
        || value.get("output").and_then(Value::as_array).is_some()
        || value.get("object").and_then(Value::as_str) == Some("response")
}

fn event_key(value: &Value) -> Option<String> {
    let sequence = value
        .get("sequence_number")
        .or_else(|| value.get("event_id"))?;
    Some(sequence.to_string())
}

fn text_part_key(value: &Value) -> String {
    let item = value
        .get("item_id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| {
            format!(
                "output:{}",
                value
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
            )
        });
    let content_index = value
        .get("summary_index")
        .or_else(|| value.get("content_index"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    format!("{item}:{content_index}")
}

fn terminal_snapshot(value: &Value) -> Option<&Value> {
    if value.get("output").and_then(Value::as_array).is_some() {
        return Some(value);
    }
    match value.get("type").and_then(Value::as_str) {
        Some("response.completed" | "response.done" | "response.incomplete") => {
            value.get("response")
        }
        _ => None,
    }
}

fn terminal_status(value: &Value) -> Option<ResponsesTerminalStatus> {
    match value.get("type").and_then(Value::as_str) {
        Some("response.completed" | "response.done") => {
            return Some(ResponsesTerminalStatus::Completed)
        }
        Some("response.incomplete") => return Some(ResponsesTerminalStatus::Incomplete),
        Some("response.failed" | "response.cancelled" | "error") => {
            return Some(ResponsesTerminalStatus::Failed)
        }
        _ => {}
    }

    let response = value.get("response").unwrap_or(value);
    if response.get("object").and_then(Value::as_str) != Some("response") {
        return None;
    }
    match response.get("status").and_then(Value::as_str) {
        Some("completed") => Some(ResponsesTerminalStatus::Completed),
        Some("incomplete") => Some(ResponsesTerminalStatus::Incomplete),
        Some("failed" | "cancelled") => Some(ResponsesTerminalStatus::Failed),
        _ => None,
    }
}

fn replay_item_key(item: &Value) -> Option<String> {
    let item_type = item.get("type").and_then(Value::as_str)?;
    let id = item
        .get("id")
        .or_else(|| item.get("call_id"))
        .and_then(Value::as_str)?;
    Some(format!("{item_type}:{id}"))
}

fn usage_from_responses(value: &Value) -> Option<Value> {
    value
        .get("response")
        .and_then(|response| response.get("usage"))
        .filter(|usage| !usage.is_null())
        .cloned()
        .or_else(|| usage_value(value))
}

fn extract_responses_output_text(value: &Value) -> String {
    if let Some(output_text) = value.get("output_text").and_then(Value::as_str) {
        return output_text.to_string();
    }

    value
        .get("output")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("message"))
        .flat_map(|item| {
            item.get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter(|content| {
            matches!(
                content.get("type").and_then(Value::as_str),
                Some("output_text" | "refusal")
            )
        })
        .filter_map(|content| {
            content
                .get("text")
                .or_else(|| content.get("output_text"))
                .or_else(|| content.get("refusal"))
                .and_then(Value::as_str)
        })
        .collect::<Vec<_>>()
        .join("")
}
