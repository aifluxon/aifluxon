use super::chat_completions::ChatCompletionsTurnAssembler;
use super::responses::ResponsesTurnAssembler;
use super::{OpenAiApiMode, OpenAiWireResponse};
use crate::common::{IncrementalSseParser, SseEvent};
use aifluxon_core::{ModelEventSink, ModelTurn, ProviderError};
use serde_json::Value;
use std::sync::Arc;

pub fn decode_chat_response(
    response: &OpenAiWireResponse,
    sink: Arc<dyn ModelEventSink>,
) -> Result<ModelTurn, ProviderError> {
    decode_chat_response_with(response, sink, false)
}

pub fn decode_chat_response_with(
    response: &OpenAiWireResponse,
    sink: Arc<dyn ModelEventSink>,
    allow_cumulative_delta: bool,
) -> Result<ModelTurn, ProviderError> {
    feed_buffered(
        OpenAiApiMode::ChatCompletions,
        response,
        sink,
        allow_cumulative_delta,
    )
}

pub fn decode_responses_response(
    response: &OpenAiWireResponse,
    sink: Arc<dyn ModelEventSink>,
) -> Result<ModelTurn, ProviderError> {
    decode_responses_response_with(response, sink, false)
}

pub fn decode_responses_response_with(
    response: &OpenAiWireResponse,
    sink: Arc<dyn ModelEventSink>,
    allow_cumulative_delta: bool,
) -> Result<ModelTurn, ProviderError> {
    feed_buffered(
        OpenAiApiMode::Responses,
        response,
        sink,
        allow_cumulative_delta,
    )
}

pub(crate) fn feed_buffered(
    mode: OpenAiApiMode,
    response: &OpenAiWireResponse,
    sink: Arc<dyn ModelEventSink>,
    allow_cumulative_delta: bool,
) -> Result<ModelTurn, ProviderError> {
    let mut decoder =
        LiveStreamDecoder::new(mode, allow_cumulative_delta, response.content_type.clone());
    for chunk in &response.chunks {
        decoder.push(chunk, sink.as_ref())?;
    }
    decoder.finish(sink.as_ref())
}

pub(crate) struct LiveStreamDecoder {
    mode: OpenAiApiMode,
    content_type: Option<String>,
    parser: IncrementalSseParser,
    head: Vec<u8>,
    raw: Vec<u8>,
    sse: Option<bool>,
    chat: ChatCompletionsTurnAssembler,
    responses: ResponsesTurnAssembler,
}

impl LiveStreamDecoder {
    pub(crate) fn new(
        mode: OpenAiApiMode,
        allow_cumulative_delta: bool,
        content_type: Option<String>,
    ) -> Self {
        Self {
            mode,
            content_type,
            parser: IncrementalSseParser::default(),
            head: Vec::new(),
            raw: Vec::new(),
            sse: None,
            chat: ChatCompletionsTurnAssembler::new(allow_cumulative_delta),
            responses: ResponsesTurnAssembler::new(allow_cumulative_delta),
        }
    }

    pub(crate) fn push(
        &mut self,
        chunk: &[u8],
        sink: &dyn ModelEventSink,
    ) -> Result<usize, ProviderError> {
        if self.sse == Some(false) {
            self.raw.extend_from_slice(chunk);
            return Ok(0);
        }
        if self.sse.is_none() {
            self.head.extend_from_slice(chunk);
            match detect_sse(&self.content_type, &self.head) {
                None => return Ok(0),
                Some(false) => {
                    self.sse = Some(false);
                    self.raw.append(&mut self.head);
                    return Ok(0);
                }
                Some(true) => {
                    self.sse = Some(true);
                    let head = std::mem::take(&mut self.head);
                    return self.push_sse(&head, sink);
                }
            }
        }
        self.push_sse(chunk, sink)
    }

    fn push_sse(
        &mut self,
        chunk: &[u8],
        sink: &dyn ModelEventSink,
    ) -> Result<usize, ProviderError> {
        let events = self.parser.push(chunk);
        self.apply_sse_events(events, sink)
    }

    fn apply_sse_events(
        &mut self,
        events: Vec<SseEvent>,
        sink: &dyn ModelEventSink,
    ) -> Result<usize, ProviderError> {
        let mut applied = 0;
        for event in events {
            if apply_sse_event(self.mode, &mut self.chat, &mut self.responses, event, sink)? {
                applied += 1;
            }
        }
        Ok(applied)
    }

    pub(crate) fn finish(mut self, sink: &dyn ModelEventSink) -> Result<ModelTurn, ProviderError> {
        if self.sse.is_none() {
            match detect_sse(&self.content_type, &self.head) {
                Some(false) | None => {
                    self.sse = Some(false);
                    self.raw.append(&mut self.head);
                }
                Some(true) => {
                    self.sse = Some(true);
                    let head = std::mem::take(&mut self.head);
                    self.push_sse(&head, sink)?;
                }
            }
        }
        if self.sse == Some(true) {
            let rest = self.parser.finish();
            self.apply_sse_events(rest, sink)?;
            return match self.mode {
                OpenAiApiMode::ChatCompletions => Ok(self.chat.finish()),
                OpenAiApiMode::Responses => self.responses.finish(),
            };
        }
        if self.raw.is_empty() {
            return match self.mode {
                OpenAiApiMode::ChatCompletions => Ok(self.chat.finish()),
                OpenAiApiMode::Responses => self.responses.finish(),
            };
        }
        let value = serde_json::from_slice::<Value>(&self.raw).map_err(|error| {
            ProviderError::message(format!("Provider returned invalid JSON: {error}"))
        })?;
        match self.mode {
            OpenAiApiMode::ChatCompletions => {
                self.chat.apply_value(&value, sink);
                Ok(self.chat.finish())
            }
            OpenAiApiMode::Responses => {
                self.responses.apply_value(&value, sink)?;
                self.responses.finish()
            }
        }
    }
}

fn apply_sse_event(
    mode: OpenAiApiMode,
    chat: &mut ChatCompletionsTurnAssembler,
    responses: &mut ResponsesTurnAssembler,
    event: SseEvent,
    sink: &dyn ModelEventSink,
) -> Result<bool, ProviderError> {
    if event.is_done() {
        return Ok(false);
    }
    let data = event.data.trim();
    if data.is_empty() {
        return Ok(false);
    }
    let Ok(value) = serde_json::from_str::<Value>(data) else {
        return Ok(false);
    };
    match mode {
        OpenAiApiMode::ChatCompletions => chat.apply_value(&value, sink),
        OpenAiApiMode::Responses => responses.apply_value(&value, sink)?,
    }
    Ok(true)
}

fn detect_sse(content_type: &Option<String>, bytes: &[u8]) -> Option<bool> {
    if content_type
        .as_deref()
        .is_some_and(|value| value.to_ascii_lowercase().contains("text/event-stream"))
    {
        return Some(true);
    }
    let start = String::from_utf8_lossy(bytes);
    let start = start.trim_start();
    if start.is_empty() {
        return None;
    }
    if start.starts_with("data:") || start.starts_with("event:") || start.starts_with("id:") {
        return Some(true);
    }
    if start.starts_with('{') {
        return Some(false);
    }
    None
}

#[cfg(test)]
mod live_tests {
    use super::*;
    use aifluxon_core::ModelEventSink;
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecordingSink(Mutex<Vec<String>>);

    impl ModelEventSink for RecordingSink {
        fn on_text_delta(&self, delta: &str) {
            self.0.lock().unwrap().push(delta.to_string());
        }
    }

    #[test]
    fn chat_deltas_are_emitted_as_each_sse_chunk_arrives() {
        let sink = Arc::new(RecordingSink::default());
        let mut decoder = LiveStreamDecoder::new(
            OpenAiApiMode::ChatCompletions,
            false,
            Some("text/event-stream".to_string()),
        );
        decoder
            .push(
                br#"data: {"choices":[{"delta":{"content":"Hel"}}]}"#,
                sink.as_ref(),
            )
            .unwrap();
        decoder.push(b"\n\n", sink.as_ref()).unwrap();
        assert_eq!(*sink.0.lock().unwrap(), vec!["Hel".to_string()]);
        decoder
            .push(
                br#"data: {"choices":[{"delta":{"content":"lo"}}]}"#,
                sink.as_ref(),
            )
            .unwrap();
        decoder.push(b"\n\n", sink.as_ref()).unwrap();
        assert_eq!(
            *sink.0.lock().unwrap(),
            vec!["Hel".to_string(), "lo".to_string()]
        );
        let turn = decoder.finish(sink.as_ref()).unwrap();
        assert_eq!(turn.text, "Hello");
    }

    #[test]
    fn reasoning_deltas_are_emitted_before_visible_text() {
        let sink = Arc::new(RecordingSink::default());
        let mut decoder = LiveStreamDecoder::new(
            OpenAiApiMode::ChatCompletions,
            false,
            Some("text/event-stream".to_string()),
        );
        let reasoning = concat!(
            r#"data: {"choices":[{"delta":{"reasoning_content":"plan"}}]}"#,
            "\n\n"
        );
        decoder.push(reasoning.as_bytes(), sink.as_ref()).unwrap();
        assert!(sink.0.lock().unwrap().is_empty());
        let text = concat!(
            r#"data: {"choices":[{"delta":{"content":"answer"}}]}"#,
            "\n\n"
        );
        decoder.push(text.as_bytes(), sink.as_ref()).unwrap();
        assert_eq!(*sink.0.lock().unwrap(), vec!["answer".to_string()]);
        let turn = decoder.finish(sink.as_ref()).unwrap();
        assert_eq!(turn.reasoning, "plan");
        assert_eq!(turn.text, "answer");
        assert_eq!(turn.opaque["reasoning_content"], "plan");
    }
}
