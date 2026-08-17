use aifluxon_core::{ModelEventSink, ModelTurn};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

pub fn is_official_config(provider: &str, base_url: &str, model: &str) -> bool {
    provider.eq_ignore_ascii_case("kimi")
        || (base_url
            .trim()
            .to_ascii_lowercase()
            .contains("api.moonshot")
            && model.trim().to_ascii_lowercase().starts_with("kimi-"))
}

pub fn apply_chat_thinking(body: &mut Value, model: &str) {
    let model = model.trim().to_ascii_lowercase();
    if model.starts_with("kimi-k2.6") {
        body["thinking"] = json!({ "type": "enabled", "keep": "all" });
    } else if model.starts_with("kimi-k2.5") {
        body["thinking"] = json!({ "type": "enabled" });
    }
}

pub fn apply_chat_limits(body: &mut Value, max_completion_tokens: u32) {
    body["max_completion_tokens"] = json!(max_completion_tokens);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ThinkTagKind {
    Reasoning,
    Visible,
}

struct ThinkTagPart {
    kind: ThinkTagKind,
    text: String,
}

struct ThinkTagFilter {
    reasoning: bool,
    close_tag: &'static str,
    pending: String,
}

impl ThinkTagFilter {
    fn push_delta(&mut self, delta: &str) -> Vec<ThinkTagPart> {
        self.pending.push_str(delta);
        let mut output = Vec::new();
        loop {
            let lower = self.pending.to_ascii_lowercase();
            if self.reasoning {
                if let Some(index) = lower.find(self.close_tag) {
                    take_prefix(
                        &mut self.pending,
                        index,
                        ThinkTagKind::Reasoning,
                        &mut output,
                    );
                    self.pending.drain(..self.close_tag.len());
                    self.reasoning = false;
                    continue;
                }
                let keep = suffix_tag_prefix_len(&lower, &[self.close_tag]);
                let emit_len = self.pending.len().saturating_sub(keep);
                take_prefix(
                    &mut self.pending,
                    emit_len,
                    ThinkTagKind::Reasoning,
                    &mut output,
                );
                break;
            }
            let open_tag = ["<think>", "<thinking>"]
                .iter()
                .filter_map(|tag| lower.find(tag).map(|index| (index, *tag)))
                .min_by_key(|(index, _)| *index);
            if let Some((index, tag)) = open_tag {
                take_prefix(&mut self.pending, index, ThinkTagKind::Visible, &mut output);
                self.pending.drain(..tag.len());
                self.close_tag = if tag == "<thinking>" {
                    "</thinking>"
                } else {
                    "</think>"
                };
                self.reasoning = true;
                continue;
            }
            let keep = suffix_tag_prefix_len(&lower, &["<think>", "<thinking>"]);
            let emit_len = self.pending.len().saturating_sub(keep);
            take_prefix(
                &mut self.pending,
                emit_len,
                ThinkTagKind::Visible,
                &mut output,
            );
            break;
        }
        output
    }

    fn finish(&mut self) -> Vec<ThinkTagPart> {
        if self.pending.is_empty() {
            return Vec::new();
        }
        vec![ThinkTagPart {
            kind: if self.reasoning {
                ThinkTagKind::Reasoning
            } else {
                ThinkTagKind::Visible
            },
            text: std::mem::take(&mut self.pending),
        }]
    }
}

fn take_prefix(
    pending: &mut String,
    end: usize,
    kind: ThinkTagKind,
    output: &mut Vec<ThinkTagPart>,
) {
    if end == 0 {
        return;
    }
    output.push(ThinkTagPart {
        kind,
        text: pending.drain(..end).collect(),
    });
}

fn suffix_tag_prefix_len(text: &str, tags: &[&str]) -> usize {
    text.char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(text.len()))
        .filter_map(|index| {
            let suffix = &text[index..];
            (!suffix.is_empty() && tags.iter().any(|tag| tag.starts_with(suffix)))
                .then_some(text.len() - index)
        })
        .max()
        .unwrap_or(0)
}

pub struct ThinkTagSink {
    inner: Arc<dyn ModelEventSink>,
    filter: Mutex<ThinkTagFilter>,
}

impl ThinkTagSink {
    pub fn new(inner: Arc<dyn ModelEventSink>) -> Self {
        Self {
            inner,
            filter: Mutex::new(ThinkTagFilter {
                reasoning: false,
                close_tag: "</think>",
                pending: String::new(),
            }),
        }
    }

    pub fn flush(&self) {
        let parts = self
            .filter
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .finish();
        emit_parts(self.inner.as_ref(), parts);
    }
}

impl ModelEventSink for ThinkTagSink {
    fn on_text_delta(&self, delta: &str) {
        let parts = self
            .filter
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push_delta(delta);
        emit_parts(self.inner.as_ref(), parts);
    }

    fn on_reasoning_delta(&self, delta: &str) {
        self.inner.on_reasoning_delta(delta);
    }

    fn on_usage(&self, usage: &Value) {
        self.inner.on_usage(usage);
    }
}

fn emit_parts(sink: &dyn ModelEventSink, parts: Vec<ThinkTagPart>) {
    for part in parts {
        if part.text.is_empty() {
            continue;
        }
        match part.kind {
            ThinkTagKind::Reasoning => sink.on_reasoning_delta(&part.text),
            ThinkTagKind::Visible => sink.on_text_delta(&part.text),
        }
    }
}

pub fn apply_think_tags_to_turn(turn: &mut ModelTurn) {
    let mut filter = ThinkTagFilter {
        reasoning: false,
        close_tag: "</think>",
        pending: String::new(),
    };
    let mut visible = String::new();
    let mut reasoning = turn.reasoning.clone();
    for part in filter
        .push_delta(&turn.text)
        .into_iter()
        .chain(filter.finish())
    {
        match part.kind {
            ThinkTagKind::Reasoning => reasoning.push_str(&part.text),
            ThinkTagKind::Visible => visible.push_str(&part.text),
        }
    }
    turn.text = visible;
    turn.reasoning = reasoning;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn moonshot_endpoint_normalizes_to_official_kimi() {
        assert!(is_official_config(
            "custom",
            "https://api.moonshot.cn/v1",
            "kimi-k2.6"
        ));
    }

    #[test]
    fn think_tags_are_split_across_stream_chunks() {
        let mut filter = ThinkTagFilter {
            reasoning: false,
            close_tag: "</think>",
            pending: String::new(),
        };
        let mut reasoning = String::new();
        let mut visible = String::new();
        for delta in ["<thi", "nk>内部", "思考</th", "ink>最终答案"] {
            for part in filter.push_delta(delta) {
                match part.kind {
                    ThinkTagKind::Reasoning => reasoning.push_str(&part.text),
                    ThinkTagKind::Visible => visible.push_str(&part.text),
                }
            }
        }
        for part in filter.finish() {
            match part.kind {
                ThinkTagKind::Reasoning => reasoning.push_str(&part.text),
                ThinkTagKind::Visible => visible.push_str(&part.text),
            }
        }
        assert_eq!(reasoning, "内部思考");
        assert_eq!(visible, "最终答案");
    }
}
