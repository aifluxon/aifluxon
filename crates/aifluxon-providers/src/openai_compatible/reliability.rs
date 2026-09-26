use aifluxon_core::{ModelEventSink, ModelTurn, ProviderError, ProviderTerminal};
use serde_json::Value;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

pub(crate) struct AttemptSink {
    inner: Arc<dyn ModelEventSink>,
    visible: AtomicBool,
}

impl AttemptSink {
    pub(crate) fn new(inner: Arc<dyn ModelEventSink>) -> Self {
        Self {
            inner,
            visible: AtomicBool::new(false),
        }
    }

    pub(crate) fn has_visible_output(&self) -> bool {
        self.visible.load(Ordering::Relaxed)
    }
}

impl ModelEventSink for AttemptSink {
    fn on_text_delta(&self, delta: &str) {
        if !delta.is_empty() {
            self.visible.store(true, Ordering::Relaxed);
        }
        self.inner.on_text_delta(delta);
    }

    fn on_reasoning_delta(&self, delta: &str) {
        if !delta.is_empty() {
            self.visible.store(true, Ordering::Relaxed);
        }
        self.inner.on_reasoning_delta(delta);
    }

    fn on_usage(&self, usage: &Value) {
        self.inner.on_usage(usage);
    }
}

pub(crate) fn retryable_error(error: &ProviderError) -> bool {
    [
        "PROVIDER_STREAM_CLOSED:",
        "PROVIDER_INVALID_TOOL_CALL:",
        "PROVIDER_EMPTY_RESPONSE:",
        "PROVIDER_MALFORMED_RESPONSE:",
        "PROVIDER_RESOURCE_UNAVAILABLE:",
        "PROVIDER_TRANSIENT_HTTP:",
        "PROVIDER_TRANSIENT_TRANSPORT:",
    ]
    .iter()
    .any(|code| error.message.starts_with(code))
}

pub(crate) fn validate_turn(turn: &ModelTurn) -> Result<(), ProviderError> {
    let has_artifact = turn
        .opaque
        .get("generated_images")
        .and_then(Value::as_array)
        .is_some_and(|images| !images.is_empty());
    if turn.tool_calls.is_empty()
        && turn.text.trim().is_empty()
        && !has_artifact
        && turn.terminal.continuation_reason().is_none()
    {
        return Err(ProviderError::message(
            "PROVIDER_EMPTY_RESPONSE: Provider returned no answer or executable tool calls.",
        ));
    }
    if turn.terminal == ProviderTerminal::ToolCalls && turn.tool_calls.is_empty() {
        return Err(ProviderError::message(
            "PROVIDER_INVALID_TOOL_CALL: Provider ended in tool mode without a tool call.",
        ));
    }
    let mut ids = std::collections::HashSet::new();
    if turn.tool_calls.iter().any(|call| {
        call.name.trim().is_empty()
            || !call.arguments.is_object()
            || call
                .provider_call_id
                .as_deref()
                .is_none_or(|id| id.trim().is_empty() || !ids.insert(id))
    }) {
        return Err(ProviderError::message(
            "PROVIDER_INVALID_TOOL_CALL: Tool identity or JSON arguments are incomplete; no tools were dispatched.",
        ));
    }
    Ok(())
}

pub(super) struct TurnAttemptError {
    pub error: ProviderError,
    pub retry_after: Option<std::time::Duration>,
}
impl From<ProviderError> for TurnAttemptError {
    fn from(error: ProviderError) -> Self {
        Self {
            error,
            retry_after: None,
        }
    }
}
