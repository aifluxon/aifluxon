use aifluxon_core::{
    ModelEventSink, ModelProvider, ModelTurn, ModelTurnRequest, ProviderCapabilities,
    ProviderError, ProviderId, ProviderTerminal, ToolCall, ToolInvocationId,
};
use serde_json::json;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Deterministic in-process provider for tests and offline examples.
///
/// This is a stable embedding seam. It does not perform network I/O.
#[derive(Clone)]
pub struct ControlledProvider {
    id: ProviderId,
    turns: Arc<Mutex<VecDeque<ModelTurn>>>,
    delay: std::time::Duration,
}

impl ControlledProvider {
    pub fn new(id: impl Into<ProviderId>, turns: Vec<ModelTurn>) -> Self {
        Self {
            id: id.into(),
            turns: Arc::new(Mutex::new(VecDeque::from(turns))),
            delay: std::time::Duration::ZERO,
        }
    }

    pub fn with_delay(mut self, delay: std::time::Duration) -> Self {
        self.delay = delay;
        self
    }

    pub fn from_text_responses(
        id: impl Into<ProviderId>,
        responses: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        let turns = responses
            .into_iter()
            .map(|text| text_turn(text.into()))
            .collect();
        Self::new(id, turns)
    }
}

#[async_trait::async_trait]
impl ModelProvider for ControlledProvider {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::openai_compatible()
    }

    async fn next_turn(
        &self,
        _request: ModelTurnRequest,
        sink: Arc<dyn ModelEventSink>,
    ) -> Result<ModelTurn, ProviderError> {
        if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
        let turn = self
            .turns
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pop_front()
            .ok_or_else(|| ProviderError::message("Controlled provider has no remaining turns."))?;
        if !turn.text.is_empty() {
            sink.on_text_delta(&turn.text);
        }
        Ok(turn)
    }
}

pub fn text_turn(text: String) -> ModelTurn {
    ModelTurn {
        text,
        reasoning: String::new(),
        tool_calls: Vec::new(),
        usage: None,
        terminal: ProviderTerminal::Stop,
        opaque: json!({}),
    }
}

pub fn tool_turn(name: &str, id: &str, arguments: serde_json::Value) -> ModelTurn {
    ModelTurn {
        text: String::new(),
        reasoning: String::new(),
        tool_calls: vec![ToolCall {
            id: ToolInvocationId::from_stable_key(id),
            name: name.to_string(),
            arguments,
            provider_call_id: Some(id.to_string()),
        }],
        usage: None,
        terminal: ProviderTerminal::ToolCalls,
        opaque: json!({}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aifluxon_core::NoopModelEventSink;

    #[tokio::test]
    async fn controlled_provider_yields_turns_in_fifo_order() {
        let provider = ControlledProvider::from_text_responses("controlled", ["one", "two"]);
        let sink: Arc<dyn ModelEventSink> = Arc::new(NoopModelEventSink);
        let first = provider
            .next_turn(empty_request(), sink.clone())
            .await
            .unwrap();
        let second = provider.next_turn(empty_request(), sink).await.unwrap();
        assert_eq!(first.text, "one");
        assert_eq!(second.text, "two");
        assert_eq!(first.terminal, ProviderTerminal::Stop);
    }

    fn empty_request() -> ModelTurnRequest {
        ModelTurnRequest {
            model: "controlled-model".to_string(),
            messages: Vec::new(),
            tools: Vec::new(),
            session_key: aifluxon_core::ProviderSessionKey::from_cache_session("session"),
            run_id: aifluxon_core::RunId::new(),
            opaque_state: None,
            features: Default::default(),
        }
    }
}
