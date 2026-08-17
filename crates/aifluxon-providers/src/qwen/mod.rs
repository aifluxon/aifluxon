use crate::strategy::{model_is_family, ModelApiCapabilities};
use aifluxon_core::{ContinuationReason, ModelEventSink, ModelTurn, ProviderTerminal};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

pub const EXPLICIT_CACHE_MIN_CHARS: usize = 4096;
pub const IMAGE_RAW_BYTES_SOFT_LIMIT: usize = 1_500_000;
pub const IMAGE_MAX_SIDE: u32 = 2400;
pub const DOCUMENT_MODEL_DEFAULT: &str = "qwen-long";
pub const FILE_EXTRACT_PURPOSE: &str = "file-extract";

pub fn is_document_upload_file(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    [".xlsx", ".xls", ".pdf", ".doc", ".docx"]
        .iter()
        .any(|extension| lower.ends_with(extension))
}

pub fn is_document_extraction_file(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    [".pdf", ".doc", ".docx"]
        .iter()
        .any(|extension| lower.ends_with(extension))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThinkingSupport {
    Unsupported,
    Hybrid,
    Always,
}

pub fn capabilities(model: &str) -> ModelApiCapabilities {
    let model = model.trim().to_ascii_lowercase();
    let supports = [
        "qwen3.7-max",
        "qwen3-max",
        "qwen3.7-plus",
        "qwen3.6-plus",
        "qwen3.5-plus",
        "qwen3.6-flash",
        "qwen3.5-flash",
        "qwen3.6-35b-a3b",
        "qwen3.5-397b-a17b",
        "qwen3.5-122b-a10b",
        "qwen3.5-27b",
        "qwen3.5-35b-a3b",
        "qwen-plus",
        "qwen-flash",
        "qwen3-coder-plus",
        "qwen3-coder-flash",
        "qwen3.5-ocr",
        "qwen-plus-character",
        "qwen-flash-character",
    ]
    .iter()
    .any(|family| model_is_family(&model, family));
    if supports {
        ModelApiCapabilities::CHAT_AND_RESPONSES
    } else {
        ModelApiCapabilities::CHAT_ONLY
    }
}

pub fn thinking_support(model: &str) -> ThinkingSupport {
    let model = model.trim().to_ascii_lowercase();
    if model.contains("-thinking") || model.starts_with("qwq") {
        ThinkingSupport::Always
    } else if model.contains("-instruct")
        || model_is_family(&model, "qwen3-coder-next")
        || model_is_family(&model, "qwen3-coder-plus")
    {
        ThinkingSupport::Unsupported
    } else if model.starts_with("qwen3")
        || model_is_family(&model, "qwen-plus")
        || model_is_family(&model, "qwen-flash")
        || model_is_family(&model, "qwen-turbo")
    {
        ThinkingSupport::Hybrid
    } else {
        ThinkingSupport::Unsupported
    }
}

pub fn apply_chat_thinking(body: &mut Value, model: &str, mode: &str, budget: &str) {
    let support = thinking_support(model);
    match support {
        ThinkingSupport::Unsupported => return,
        ThinkingSupport::Always => {}
        ThinkingSupport::Hybrid => match mode.trim() {
            "enabled" => body["enable_thinking"] = json!(true),
            "disabled" => body["enable_thinking"] = json!(false),
            _ => {}
        },
    }
    if (support == ThinkingSupport::Always || mode.trim() == "enabled")
        && budget.trim().parse::<u32>().is_ok_and(|value| value > 0)
    {
        body["thinking_budget"] = json!(budget.trim().parse::<u32>().unwrap());
    }
}

pub fn apply_responses_reasoning(body: &mut Value, mode: &str) {
    match mode {
        "disabled" => body["reasoning"] = json!({ "effort": "none" }),
        "enabled" => body["reasoning"] = json!({ "effort": "medium" }),
        _ => {}
    }
}

pub fn text_part(explicit_cache: bool, text: impl Into<String>) -> Value {
    let text = text.into();
    let mut part = json!({ "type": "text", "text": text });
    if explicit_cache
        && part["text"]
            .as_str()
            .is_some_and(|text| text.len() >= EXPLICIT_CACHE_MIN_CHARS)
    {
        part["cache_control"] = json!({ "type": "ephemeral" });
    }
    part
}

fn marker_text_part(text: impl Into<String>) -> Value {
    json!({
        "type": "text",
        "text": text.into(),
        "cache_control": { "type": "ephemeral" }
    })
}

fn content_text_len(content: &Value) -> usize {
    if let Some(text) = content.as_str() {
        return text.len();
    }
    content
        .as_array()
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .map(str::len)
                .sum()
        })
        .unwrap_or_default()
}

fn mark_content(content: &mut Value) {
    if let Some(text) = content.as_str() {
        *content = Value::Array(vec![marker_text_part(text.to_string())]);
        return;
    }
    if let Some(parts) = content.as_array_mut() {
        if let Some(part) = parts
            .iter_mut()
            .rev()
            .find(|part| part.get("type").and_then(Value::as_str) == Some("text"))
        {
            part["cache_control"] = json!({ "type": "ephemeral" });
        } else {
            parts.push(marker_text_part(""));
        }
    }
}

pub fn mark_message_if_large(message: &mut Value, explicit_cache: bool) {
    if !explicit_cache {
        return;
    }
    if let Some(content) = message.get_mut("content") {
        if content_text_len(content) >= EXPLICIT_CACHE_MIN_CHARS {
            mark_content(content);
        }
    }
}

pub fn mark_latest_visible_user(messages: &mut [Value], explicit_cache: bool) {
    if !explicit_cache {
        return;
    }
    let Some(index) = messages
        .iter()
        .rposition(|message| message.get("role").and_then(Value::as_str) == Some("user"))
    else {
        return;
    };
    let prefix_len = messages[..=index]
        .iter()
        .filter_map(|message| message.get("content"))
        .map(content_text_len)
        .sum::<usize>();
    if prefix_len >= EXPLICIT_CACHE_MIN_CHARS {
        if let Some(content) = messages[index].get_mut("content") {
            mark_content(content);
        }
    }
}

pub fn refresh_cache_markers(messages: &mut [Value], explicit_cache: bool) {
    if !explicit_cache || messages.is_empty() {
        return;
    }
    for message in messages.iter_mut().skip(1) {
        if let Some(parts) = message.get_mut("content").and_then(Value::as_array_mut) {
            for part in parts {
                if let Some(part) = part.as_object_mut() {
                    part.remove("cache_control");
                }
            }
        }
    }
    let last_index = messages.len() - 1;
    let prefix_len = messages[..=last_index]
        .iter()
        .filter_map(|message| message.get("content"))
        .map(content_text_len)
        .sum::<usize>();
    if prefix_len < EXPLICIT_CACHE_MIN_CHARS {
        return;
    }
    match messages[last_index].get_mut("content") {
        Some(content) if content.is_null() => *content = Value::Array(vec![marker_text_part("")]),
        Some(content) => mark_content(content),
        None => messages[last_index]["content"] = Value::Array(vec![marker_text_part("")]),
    }
}

pub struct SummaryFilter {
    enabled: bool,
    mode: SummaryFilterMode,
    prefix_buffer: String,
    summary_buffer: String,
}

#[derive(PartialEq)]
enum SummaryFilterMode {
    Undecided,
    Passthrough,
    SuppressingSummary,
}

impl SummaryFilter {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            mode: SummaryFilterMode::Undecided,
            prefix_buffer: String::new(),
            summary_buffer: String::new(),
        }
    }

    pub fn push_delta(&mut self, delta: &str) -> Option<String> {
        if !self.enabled {
            return Some(delta.to_string());
        }

        match self.mode {
            SummaryFilterMode::Passthrough => Some(delta.to_string()),
            SummaryFilterMode::SuppressingSummary => {
                self.summary_buffer.push_str(delta);
                self.finish_summary_if_closed()
            }
            SummaryFilterMode::Undecided => {
                self.prefix_buffer.push_str(delta);
                let trimmed = self.prefix_buffer.trim_start();
                let lower = trimmed.to_ascii_lowercase();
                const TAG: &str = "<summary>";
                if TAG.starts_with(&lower) {
                    return None;
                }
                if lower.starts_with(TAG) {
                    self.mode = SummaryFilterMode::SuppressingSummary;
                    self.summary_buffer.push_str(&self.prefix_buffer);
                    self.prefix_buffer.clear();
                    return self.finish_summary_if_closed();
                }

                self.mode = SummaryFilterMode::Passthrough;
                Some(std::mem::take(&mut self.prefix_buffer))
            }
        }
    }

    fn finish_summary_if_closed(&mut self) -> Option<String> {
        let lower = self.summary_buffer.to_ascii_lowercase();
        let end_index = lower.find("</summary>")?;

        let after_start = end_index + "</summary>".len();
        let after = self.summary_buffer[after_start..].to_string();
        self.summary_buffer.truncate(after_start);
        self.mode = SummaryFilterMode::Passthrough;
        if after.is_empty() {
            None
        } else {
            Some(after)
        }
    }

    pub fn summary_text(&self) -> Option<&str> {
        let trimmed = self.summary_buffer.trim();
        if trimmed.is_empty() {
            return None;
        }
        let without_open = trimmed
            .strip_prefix("<summary>")
            .or_else(|| trimmed.strip_prefix("<SUMMARY>"))
            .unwrap_or(trimmed);
        let without_close = without_open
            .strip_suffix("</summary>")
            .or_else(|| without_open.strip_suffix("</SUMMARY>"))
            .unwrap_or(without_open)
            .trim();
        if without_close.is_empty() {
            None
        } else {
            Some(without_close)
        }
    }

    pub fn finish(&mut self) -> Option<String> {
        if self.mode == SummaryFilterMode::Undecided && !self.prefix_buffer.is_empty() {
            self.mode = SummaryFilterMode::Passthrough;
            return Some(std::mem::take(&mut self.prefix_buffer));
        }
        None
    }
}

pub struct SummaryFilterSink {
    inner: Arc<dyn ModelEventSink>,
    filter: Mutex<SummaryFilter>,
}

impl SummaryFilterSink {
    pub fn new(inner: Arc<dyn ModelEventSink>) -> Self {
        Self {
            inner,
            filter: Mutex::new(SummaryFilter::new(true)),
        }
    }
}

impl ModelEventSink for SummaryFilterSink {
    fn on_text_delta(&self, delta: &str) {
        let visible = self
            .filter
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push_delta(delta);
        if let Some(visible) = visible {
            if !visible.is_empty() {
                self.inner.on_text_delta(&visible);
            }
        }
    }

    fn on_reasoning_delta(&self, delta: &str) {
        self.inner.on_reasoning_delta(delta);
    }

    fn on_usage(&self, usage: &Value) {
        self.inner.on_usage(usage);
    }
}

pub fn apply_summary_to_turn(turn: &mut ModelTurn) {
    let mut filter = SummaryFilter::new(true);
    let mut visible = String::new();
    if let Some(delta) = filter.push_delta(&turn.text) {
        visible.push_str(&delta);
    }
    if let Some(tail) = filter.finish() {
        visible.push_str(&tail);
    }
    if let Some(summary) = filter.summary_text() {
        match turn.opaque.as_object_mut() {
            Some(object) => {
                object.insert("hidden_context".to_string(), json!(summary));
            }
            None => {
                turn.opaque = json!({ "hidden_context": summary });
            }
        }
        if visible.trim().is_empty() && turn.tool_calls.is_empty() {
            turn.terminal = ProviderTerminal::Continue(ContinuationReason::SummaryOnly);
        }
    }
    turn.text = visible;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn thinking_contract_follows_model_support() {
        let mut hybrid = json!({});
        apply_chat_thinking(&mut hybrid, "qwen3-max", "enabled", "8192");
        assert_eq!(hybrid["enable_thinking"], true);
        assert_eq!(hybrid["thinking_budget"], 8192);
        let mut unsupported = json!({});
        apply_chat_thinking(&mut unsupported, "qwen3-coder-plus", "enabled", "8192");
        assert_eq!(unsupported, json!({}));
    }

    #[test]
    fn qwen_summary_only_turn_requests_continuation() {
        let mut turn = ModelTurn {
            text: "<summary>\ninternal recap\n</summary>".to_string(),
            reasoning: String::new(),
            tool_calls: Vec::new(),
            usage: None,
            terminal: ProviderTerminal::Stop,
            opaque: json!({ "protocol": "chat_completions" }),
        };
        apply_summary_to_turn(&mut turn);
        assert_eq!(turn.text.trim(), "");
        assert_eq!(turn.opaque["hidden_context"], "internal recap");
        assert_eq!(
            turn.terminal,
            ProviderTerminal::Continue(ContinuationReason::SummaryOnly)
        );
    }

    #[test]
    fn qwen_normal_final_answer_does_not_continue() {
        let mut turn = ModelTurn {
            text: "已完成，开发服务器已启动。".to_string(),
            reasoning: String::new(),
            tool_calls: Vec::new(),
            usage: None,
            terminal: ProviderTerminal::Stop,
            opaque: json!({ "protocol": "chat_completions" }),
        };
        apply_summary_to_turn(&mut turn);
        assert_eq!(turn.text, "已完成，开发服务器已启动。");
        assert!(turn.opaque.get("hidden_context").is_none());
        assert_eq!(turn.terminal, ProviderTerminal::Stop);
    }

    #[test]
    fn qwen_summary_then_visible_answer_does_not_continue() {
        let mut turn = ModelTurn {
            text: "<summary>internal</summary>\n\n已完成。".to_string(),
            reasoning: String::new(),
            tool_calls: Vec::new(),
            usage: None,
            terminal: ProviderTerminal::Stop,
            opaque: json!({ "protocol": "chat_completions" }),
        };
        apply_summary_to_turn(&mut turn);
        assert_eq!(turn.text, "\n\n已完成。");
        assert_eq!(turn.opaque["hidden_context"], "internal");
        assert_eq!(turn.terminal, ProviderTerminal::Stop);
    }
}
