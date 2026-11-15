//! Bounded protocol facts; never records request bodies, deltas or credentials.
use super::OpenAiApiMode;
use aifluxon_core::{ModelTurn, ProviderError};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Instant;

pub(super) struct StreamDiagnostics {
    span: tracing::Span,
    started: Instant,
    protocol: &'static str,
    phase: &'static str,
    outcome: &'static str,
    failure_code: String,
    http_status: Option<u16>,
    headers_ms: Option<u64>,
    first_byte_ms: Option<u64>,
    last_byte_ms: Option<u64>,
    chunks: u64,
    bytes: u64,
    events: u64,
    visible_delta_bytes: u64,
    reasoning_delta_bytes: u64,
    event_counts: BTreeMap<String, u64>,
    tool_slots: BTreeMap<u64, (String, String)>,
    detail_count: u64,
    omitted_details: u64,
    terminal_seen: bool,
    decoder_complete: bool,
    done_marker: bool,
}

pub(super) fn failure_code(error: &ProviderError) -> &str {
    error.code.as_str()
}

impl StreamDiagnostics {
    pub(super) fn new(mode: OpenAiApiMode) -> Self {
        Self {
            span: tracing::Span::current(),
            started: Instant::now(),
            protocol: match mode {
                OpenAiApiMode::Responses => "responses",
                OpenAiApiMode::ChatCompletions => "chat_completions",
            },
            phase: "awaiting_headers",
            outcome: "aborted",
            failure_code: String::new(),
            http_status: None,
            headers_ms: None,
            first_byte_ms: None,
            last_byte_ms: None,
            chunks: 0,
            bytes: 0,
            events: 0,
            visible_delta_bytes: 0,
            reasoning_delta_bytes: 0,
            event_counts: BTreeMap::new(),
            tool_slots: BTreeMap::new(),
            detail_count: 0,
            omitted_details: 0,
            terminal_seen: false,
            decoder_complete: false,
            done_marker: false,
        }
    }

    fn elapsed_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    pub(super) fn headers(
        &mut self,
        status: u16,
        content_type: Option<&str>,
        retry_after_ms: Option<u64>,
    ) {
        self.http_status = Some(status);
        self.headers_ms = Some(self.elapsed_ms());
        self.phase = "reading_body";
        let media_type = match content_type {
            Some(value) if value.contains("text/event-stream") => "event_stream",
            Some(value) if value.contains("json") => "json",
            Some(_) => "other",
            None => "missing",
        };
        tracing::info!(
            category = "agent",
            component = "agent.provider",
            event_code = "PROVIDER_RESPONSE_HEADERS",
            outcome = "received",
            http_status = status,
            headers_ms = self.headers_ms.unwrap_or_default(),
            media_type,
            retry_after_present = retry_after_ms.is_some(),
            retry_after_ms = retry_after_ms.unwrap_or_default(),
            "Provider response headers received"
        );
    }

    pub(super) fn chunk(&mut self, bytes: usize) {
        if self.phase == "awaiting_headers" {
            self.phase = "reading_body";
        }
        self.chunks = self.chunks.saturating_add(1);
        self.bytes = self.bytes.saturating_add(bytes as u64);
        if bytes > 0 {
            let elapsed = self.elapsed_ms();
            self.first_byte_ms.get_or_insert(elapsed);
            self.last_byte_ms = Some(elapsed);
        }
    }

    pub(super) fn done(&mut self) {
        self.done_marker = true;
    }

    pub(super) fn decoded_complete(&mut self, complete: bool) {
        self.decoder_complete = complete;
    }

    pub(super) fn observe(&mut self, value: &Value) {
        self.events = self.events.saturating_add(1);
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("chat_chunk");
        let safe_kind = if kind.len() <= 96
            && kind
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        {
            kind
        } else {
            "unknown"
        };
        let key = if self.event_counts.contains_key(safe_kind) || self.event_counts.len() < 32 {
            safe_kind
        } else {
            "other"
        };
        *self.event_counts.entry(key.into()).or_default() += 1;
        if let Some(delta) = value.get("delta").and_then(Value::as_str) {
            match kind {
                "response.output_text.delta" | "response.refusal.delta" => {
                    self.visible_delta_bytes += delta.len() as u64
                }
                "response.reasoning_text.delta" | "response.reasoning_summary_text.delta" => {
                    self.reasoning_delta_bytes += delta.len() as u64
                }
                _ => {}
            }
        }
        if let Some(choice) = value.pointer("/choices/0") {
            if let Some(delta) = choice.get("delta") {
                self.visible_delta_bytes += delta
                    .get("content")
                    .and_then(Value::as_str)
                    .map(str::len)
                    .unwrap_or(0) as u64;
                self.reasoning_delta_bytes += delta
                    .get("reasoning_content")
                    .or_else(|| delta.get("reasoning"))
                    .and_then(Value::as_str)
                    .map(str::len)
                    .unwrap_or(0) as u64;
            }
            if let Some(calls) = choice
                .pointer("/delta/tool_calls")
                .or_else(|| choice.pointer("/message/tool_calls"))
                .and_then(Value::as_array)
            {
                for (position, call) in calls.iter().enumerate() {
                    self.tool(
                        call.get("index")
                            .and_then(Value::as_u64)
                            .unwrap_or(position as u64),
                        call,
                    );
                }
            }
            if choice
                .get("finish_reason")
                .is_some_and(|reason| !reason.is_null())
            {
                self.terminal_seen = true;
                self.detail("chat_finish", choice);
            }
        }
        if matches!(
            kind,
            "response.completed"
                | "response.done"
                | "response.incomplete"
                | "response.failed"
                | "response.cancelled"
                | "error"
        ) {
            self.terminal_seen = true;
            self.detail(safe_kind, value.get("response").unwrap_or(value));
        }
        if let Some(item) = value.get("item") {
            if item.get("type").and_then(Value::as_str) == Some("function_call") {
                self.tool(
                    value
                        .get("output_index")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                    item,
                );
                if kind == "response.output_item.done" {
                    self.detail("function_call_done", item);
                }
            }
        }
        let response = value.get("response").unwrap_or(value);
        if response.get("status").and_then(Value::as_str) == Some("completed") {
            self.terminal_seen = true;
        }
        if let Some(items) = response.get("output").and_then(Value::as_array) {
            for (index, item) in items.iter().enumerate() {
                if item.get("type").and_then(Value::as_str) == Some("function_call") {
                    self.tool(index as u64, item);
                }
            }
        }
    }

    fn tool(&mut self, slot: u64, item: &Value) {
        let id = item
            .get("call_id")
            .or_else(|| item.get("id"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let name = item
            .get("name")
            .or_else(|| item.pointer("/function/name"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !self.tool_slots.contains_key(&slot) && self.tool_slots.len() >= 512 {
            self.omitted_details += 1;
            return;
        }
        let changed = self.tool_slots.get(&slot).is_none_or(|previous| {
            (!id.is_empty() && previous.0 != id) || (!name.is_empty() && previous.1 != name)
        });
        if changed {
            let previous = self.tool_slots.entry(slot).or_default();
            if !id.is_empty() {
                previous.0 = id.chars().take(160).collect();
            }
            if !name.is_empty() {
                previous.1 = name.chars().take(128).collect();
            }
            self.detail("tool_call_observed", item);
        }
    }

    fn detail(&mut self, kind: &str, value: &Value) {
        if self.detail_count >= 128 {
            self.omitted_details += 1;
            return;
        }
        self.detail_count += 1;
        let args = value
            .get("arguments")
            .or_else(|| value.pointer("/function/arguments"));
        let args_state = match args {
            None => "absent",
            Some(Value::String(arguments)) => match serde_json::from_str::<Value>(arguments) {
                Ok(Value::Object(_)) => "object",
                Ok(_) => "non_object",
                Err(_) => "invalid_json",
            },
            Some(_) => "not_string",
        };
        let facts = json!({
            "args_state": args_state,
            "status": value.get("status").and_then(Value::as_str).map(|value| value.chars().take(96).collect::<String>()),
            "finish_reason": value.get("finish_reason").and_then(Value::as_str).map(|value| value.chars().take(96).collect::<String>()),
            "end_turn": value.get("end_turn").filter(|value| value.is_boolean()),
                "end_turn_present": value.get("end_turn").is_some(),
            "item_type": value.get("type").and_then(Value::as_str).map(|value| value.chars().take(96).collect::<String>()),
            "call_id": value.get("call_id").or_else(|| value.get("id")).and_then(Value::as_str).map(|value| value.chars().take(160).collect::<String>()),
            "tool_name": value.get("name").or_else(|| value.pointer("/function/name")).and_then(Value::as_str).map(|value| value.chars().take(128).collect::<String>()),
            "args_bytes": value.get("arguments").or_else(|| value.pointer("/function/arguments")).and_then(Value::as_str).map(str::len),
            "output_count": value.get("output").and_then(Value::as_array).map(Vec::len),
            "incomplete_reason": value.pointer("/incomplete_details/reason").and_then(Value::as_str).map(|value| value.chars().take(96).collect::<String>()),
        });
        tracing::info!(category = "agent", component = "agent.provider",
            event_code = "PROVIDER_WIRE_STATE", outcome = "observed", wire_kind = kind,
            wire_facts = %facts, "Provider native protocol state observed");
    }

    pub(super) fn fail(&mut self, error: &ProviderError) {
        self.outcome = "failed";
        self.failure_code = failure_code(error).to_string();
    }

    pub(super) fn http_error(&mut self) {
        self.outcome = "http_error";
    }

    pub(super) fn finish(&mut self, result: &Result<ModelTurn, ProviderError>) {
        match result {
            Ok(_) => {
                self.outcome = "success";
                self.phase = "validated";
                self.decoder_complete = true;
            }
            Err(error) => self.fail(error),
        }
    }

    pub(super) fn snapshot(&self) -> Value {
        json!({ "protocol": self.protocol, "phase": self.phase, "stream_outcome": self.outcome, "http_status": self.http_status,
            "headers_ms": self.headers_ms, "first_byte_ms": self.first_byte_ms,
            "last_byte_ms": self.last_byte_ms, "chunks": self.chunks, "bytes": self.bytes,
            "events": self.events, "event_counts": self.event_counts.iter().map(|(kind, count)| json!({"event_type":kind, "count":count})).collect::<Vec<_>>(),
            "visible_delta_bytes": self.visible_delta_bytes, "reasoning_delta_bytes": self.reasoning_delta_bytes,
            "observed_tool_slots": self.tool_slots.len(), "details_omitted": self.omitted_details,
            "terminal_seen": self.terminal_seen, "settled": self.decoder_complete, "done_marker": self.done_marker,
            "failure_kind": self.failure_code,
        })
    }
}

impl Drop for StreamDiagnostics {
    fn drop(&mut self) {
        self.span.in_scope(|| {
            tracing::info!(category = "agent", component = "agent.provider",
            event_code = "PROVIDER_STREAM_SUMMARY", outcome = self.outcome,
            duration_ms = self.elapsed_ms(), stream_facts = %self.snapshot(),
            "Provider HTTP exchange and decoder summary")
        });
    }
}
