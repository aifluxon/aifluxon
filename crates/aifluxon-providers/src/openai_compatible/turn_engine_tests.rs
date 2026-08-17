use super::{
    decode_chat_response, decode_chat_response_with, decode_responses_response, OpenAiWireResponse,
};
use aifluxon_core::{ModelEventSink, ModelTurn, ProviderTerminal};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq)]
enum RecordedEvent {
    Text(String),
    Reasoning(String),
    Usage(Value),
}

#[derive(Default)]
struct RecordingSink {
    events: Mutex<Vec<RecordedEvent>>,
}

impl ModelEventSink for RecordingSink {
    fn on_text_delta(&self, delta: &str) {
        self.events
            .lock()
            .unwrap()
            .push(RecordedEvent::Text(delta.to_string()));
    }

    fn on_reasoning_delta(&self, delta: &str) {
        self.events
            .lock()
            .unwrap()
            .push(RecordedEvent::Reasoning(delta.to_string()));
    }

    fn on_usage(&self, usage: &Value) {
        self.events
            .lock()
            .unwrap()
            .push(RecordedEvent::Usage(usage.clone()));
    }
}

fn sse(source: &str) -> OpenAiWireResponse {
    OpenAiWireResponse {
        status: 200,
        content_type: Some("text/event-stream".to_string()),
        chunks: vec![source.as_bytes().to_vec()],
    }
}

fn sse_chunks(chunks: Vec<Vec<u8>>) -> OpenAiWireResponse {
    OpenAiWireResponse {
        status: 200,
        content_type: Some("text/event-stream".to_string()),
        chunks,
    }
}

fn decode_chat(source: &str) -> (ModelTurn, Vec<RecordedEvent>) {
    let sink = Arc::new(RecordingSink::default());
    let turn = decode_chat_response(&sse(source), sink.clone()).unwrap();
    let events = sink.events.lock().unwrap().clone();
    (turn, events)
}

fn decode_responses(source: &str) -> (ModelTurn, Vec<RecordedEvent>) {
    let sink = Arc::new(RecordingSink::default());
    let turn = decode_responses_response(&sse(source), sink.clone()).unwrap();
    let events = sink.events.lock().unwrap().clone();
    (turn, events)
}

fn assert_stable_across_byte_splits(
    source: &str,
    decode: impl Fn(&OpenAiWireResponse) -> ModelTurn,
    expected: &ModelTurn,
) {
    let bytes = source.as_bytes();
    let mid = source
        .find('模')
        .map(|index| index + 1)
        .unwrap_or(source.len() / 2);
    for split in [0, mid.min(bytes.len()), bytes.len()] {
        let response = sse_chunks(vec![bytes[..split].to_vec(), bytes[split..].to_vec()]);
        let turn = decode(&response);
        assert_eq!(turn.text, expected.text, "split {split}");
        assert_eq!(turn.reasoning, expected.reasoning, "split {split}");
        assert_eq!(turn.tool_calls, expected.tool_calls, "split {split}");
        assert_eq!(turn.usage, expected.usage, "split {split}");
        assert_eq!(turn.terminal, expected.terminal, "split {split}");
        assert_eq!(turn.opaque, expected.opaque, "split {split}");
    }
}

const CHAT_VISIBLE_REASONING_USAGE: &str = concat!(
    "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"先看文件\"}}]}\n\n",
    "data: {\"choices\":[{\"delta\":{\"content\":\"模型\"}}]}\n\n",
    "data: {\"choices\":[{\"delta\":{\"content\":\"输出\"},\"finish_reason\":\"stop\"}]}\n\n",
    "data: {\"choices\":[{\"delta\":{}}],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":2,\"total_tokens\":5}}\n\n",
    "data: [DONE]\n\n"
);

const CHAT_SPLIT_TOOL_ARGS: &str = concat!(
    "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_\",\"function\":{\"name\":\"she\"}}]}}]}\n\n",
    "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":1,\"id\":\"call_b\",\"function\":{\"name\":\"read_file\"}}]}}]}\n\n",
    "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"a\",\"function\":{\"name\":\"ll\",\"arguments\":\"{\\\"com\"}}]}}]}\n\n",
    "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"mand\\\":\\\"echo \\\\\\\"{ok}\\\\\\\"\\\"}\"}}]}}]}\n\n",
    "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":1,\"function\":{\"arguments\":\"{\\\"path\\\":\\\"src/lib.rs\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
    "data: [DONE]\n\n"
);

const CHAT_MALFORMED_THEN_TEXT: &str = concat!(
    "data: {bad}\n\n",
    "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}]}\n\n",
    "data: [DONE]\n\n"
);

const CHAT_CUMULATIVE: &str = concat!(
    "data: {\"choices\":[{\"delta\":{\"content\":\"模型\"}}]}\n\n",
    "data: {\"choices\":[{\"delta\":{\"content\":\"模型正在\"}}]}\n\n",
    "data: {\"choices\":[{\"delta\":{\"content\":\"模型正在输出\"},\"finish_reason\":\"stop\"}]}\n\n",
    "data: [DONE]\n\n"
);

const RESPONSES_FULL_TURN: &str = concat!(
    "data: {\"type\":\"response.reasoning_summary_text.delta\",\"sequence_number\":1,\"delta\":\"think\"}\n\n",
    "data: {\"type\":\"response.reasoning_text.delta\",\"sequence_number\":2,\"item_id\":\"rs-1\",\"delta\":\"先分析\"}\n\n",
    "data: {\"type\":\"response.output_text.delta\",\"sequence_number\":3,\"delta\":\"done\"}\n\n",
    "data: {\"type\":\"response.output_item.done\",\"sequence_number\":4,\"output_index\":0,\"item\":{\"type\":\"function_call\",\"call_id\":\"call-1\",\"name\":\"act\",\"arguments\":\"{\\\"x\\\":1}\"}}\n\n",
    "data: {\"type\":\"response.completed\",\"sequence_number\":5,\"response\":{\"usage\":{\"total_tokens\":3}}}\n\n"
);

const RESPONSES_REFUSAL: &str = concat!(
    "data: {\"type\":\"response.refusal.delta\",\"sequence_number\":1,\"delta\":\"I cannot\"}\n\n",
    "data: {\"type\":\"response.refusal.done\",\"sequence_number\":2,\"refusal\":\"I cannot\"}\n\n",
    "data: {\"type\":\"response.completed\",\"sequence_number\":3,\"response\":{\"usage\":{\"total_tokens\":1},\"output\":[{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"refusal\",\"refusal\":\"I cannot\"}]}]}}\n\n"
);

const RESPONSES_REPLAY_AND_HOSTED: &str = concat!(
    "data: {\"type\":\"response.output_item.done\",\"sequence_number\":1,\"output_index\":0,\"item\":{\"type\":\"reasoning\",\"id\":\"rs-1\",\"status\":\"completed\"}}\n\n",
    "data: {\"type\":\"response.output_item.done\",\"sequence_number\":2,\"output_index\":1,\"item\":{\"type\":\"web_search_call\",\"id\":\"ws-1\",\"status\":\"completed\"}}\n\n",
    "data: {\"type\":\"response.output_text.delta\",\"sequence_number\":3,\"delta\":\"cited\"}\n\n",
    "data: {\"type\":\"response.completed\",\"sequence_number\":4,\"response\":{\"usage\":{\"total_tokens\":8}}}\n\n"
);

const RESPONSES_IMAGE: &str = concat!(
    "data: {\"type\":\"response.output_item.done\",\"sequence_number\":1,\"output_index\":0,\"item\":{\"type\":\"image_generation_call\",\"id\":\"ig_123\",\"result\":\"aW1hZ2U=\",\"revised_prompt\":\"A clearer prompt\"}}\n\n",
    "data: {\"type\":\"response.output_item.done\",\"sequence_number\":1,\"output_index\":0,\"item\":{\"type\":\"image_generation_call\",\"id\":\"ig_123\",\"result\":\"aW1hZ2U=\",\"revised_prompt\":\"A clearer prompt\"}}\n\n",
    "data: {\"type\":\"response.completed\",\"sequence_number\":2,\"response\":{\"id\":\"resp-1\"}}\n\n"
);

const RESPONSES_INCOMPLETE: &str = concat!(
    "data: {\"type\":\"response.output_text.delta\",\"sequence_number\":1,\"delta\":\"partial\"}\n\n",
    "data: {\"type\":\"response.incomplete\",\"sequence_number\":2,\"response\":{\"incomplete_details\":{\"reason\":\"max_output_tokens\"}}}\n\n"
);

const RESPONSES_CANCELLED_STREAM: &str = concat!(
    "data: {\"type\":\"response.output_text.delta\",\"sequence_number\":1,\"delta\":\"partial\"}\n\n"
);

#[test]
fn chat_fixture_preserves_visible_reasoning_usage_terminal_and_event_order() {
    let (turn, events) = decode_chat(CHAT_VISIBLE_REASONING_USAGE);
    assert_eq!(turn.text, "模型输出");
    assert_eq!(turn.reasoning, "先看文件");
    assert_eq!(turn.terminal, ProviderTerminal::Stop);
    assert_eq!(turn.usage.unwrap()["total_tokens"], 5);
    assert_eq!(turn.opaque["protocol"], "chat_completions");
    assert_eq!(turn.opaque["finish_reason"], "stop");
    assert_eq!(turn.opaque["reasoning_content"], "先看文件");
    assert_eq!(
        events,
        vec![
            RecordedEvent::Reasoning("先看文件".to_string()),
            RecordedEvent::Text("模型".to_string()),
            RecordedEvent::Text("输出".to_string()),
            RecordedEvent::Usage(json!({"prompt_tokens":3,"completion_tokens":2,"total_tokens":5})),
        ]
    );
}

#[test]
fn chat_fixture_assembles_split_and_multiple_tool_calls() {
    let (turn, _) = decode_chat(CHAT_SPLIT_TOOL_ARGS);
    assert_eq!(turn.terminal, ProviderTerminal::ToolCalls);
    assert_eq!(turn.tool_calls.len(), 2);
    assert_eq!(turn.tool_calls[0].name, "shell");
    assert_eq!(
        turn.tool_calls[0].arguments,
        json!({ "command": "echo \"{ok}\"" })
    );
    assert_eq!(turn.tool_calls[1].name, "read_file");
    assert_eq!(
        turn.tool_calls[1].arguments,
        json!({ "path": "src/lib.rs" })
    );
    assert_ne!(turn.tool_calls[0].id, turn.tool_calls[1].id);
}

#[test]
fn chat_malformed_chunk_is_skipped_without_losing_later_text() {
    let (turn, _) = decode_chat(CHAT_MALFORMED_THEN_TEXT);
    assert_eq!(turn.text, "ok");
    assert_eq!(turn.terminal, ProviderTerminal::Stop);
}

#[test]
fn chat_cumulative_compatible_deltas_only_append_missing_suffix() {
    let sink = Arc::new(RecordingSink::default());
    let turn = decode_chat_response_with(&sse(CHAT_CUMULATIVE), sink.clone(), true).unwrap();
    assert_eq!(turn.text, "模型正在输出");
    let events = sink.events.lock().unwrap().clone();
    let texts: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            RecordedEvent::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(texts, ["模型", "正在", "输出"]);
}

#[test]
fn chat_utf8_and_chunk_splits_match_recorded_result() {
    let (expected, _) = decode_chat(CHAT_VISIBLE_REASONING_USAGE);
    assert_stable_across_byte_splits(
        CHAT_VISIBLE_REASONING_USAGE,
        |response| {
            decode_chat_response(response, Arc::new(aifluxon_core::NoopModelEventSink)).unwrap()
        },
        &expected,
    );
}

#[test]
fn responses_fixture_preserves_reasoning_text_tools_usage_and_terminal() {
    let (turn, events) = decode_responses(RESPONSES_FULL_TURN);
    assert_eq!(turn.text, "done");
    assert_eq!(turn.reasoning, "think先分析");
    assert_eq!(turn.tool_calls.len(), 1);
    assert_eq!(turn.tool_calls[0].name, "act");
    assert_eq!(turn.tool_calls[0].arguments, json!({ "x": 1 }));
    assert_eq!(turn.terminal, ProviderTerminal::ToolCalls);
    assert_eq!(turn.usage.unwrap()["total_tokens"], 3);
    assert_eq!(
        events,
        vec![
            RecordedEvent::Reasoning("think".to_string()),
            RecordedEvent::Reasoning("先分析".to_string()),
            RecordedEvent::Text("done".to_string()),
            RecordedEvent::Usage(json!({"total_tokens":3})),
        ]
    );
}

#[test]
fn responses_refusal_is_visible_text_not_a_local_tool_call() {
    let (turn, _) = decode_responses(RESPONSES_REFUSAL);
    assert_eq!(turn.text, "I cannot");
    assert!(turn.tool_calls.is_empty());
    assert_eq!(turn.terminal, ProviderTerminal::Stop);
}

#[test]
fn responses_hosted_activity_stays_in_replay_state_not_local_tool_calls() {
    let (turn, _) = decode_responses(RESPONSES_REPLAY_AND_HOSTED);
    assert_eq!(turn.text, "cited");
    assert!(turn.tool_calls.is_empty());
    assert_eq!(turn.terminal, ProviderTerminal::Stop);
    assert_eq!(turn.opaque["response_items"].as_array().unwrap().len(), 2);
    assert_eq!(turn.opaque["response_items"][0]["type"], "reasoning");
    assert_eq!(turn.opaque["response_items"][1]["type"], "web_search_call");
}

#[test]
fn responses_generated_images_are_captured_once_without_host_materialization() {
    let (turn, _) = decode_responses(RESPONSES_IMAGE);
    assert_eq!(turn.opaque["generated_images"].as_array().unwrap().len(), 1);
    assert_eq!(turn.opaque["generated_images"][0]["id"], "ig_123");
    assert_eq!(turn.opaque["generated_images"][0]["result"], "aW1hZ2U=");
    assert!(turn.tool_calls.is_empty());
}

#[test]
fn responses_duplicate_sequence_numbers_are_deduplicated() {
    let duplicated = concat!(
        "data: {\"type\":\"response.output_text.delta\",\"sequence_number\":7,\"delta\":\"片段\"}\n\n",
        "data: {\"type\":\"response.output_text.delta\",\"sequence_number\":7,\"delta\":\"片段\"}\n\n",
        "data: {\"type\":\"response.completed\",\"sequence_number\":8}\n\n"
    );
    let (turn, events) = decode_responses(duplicated);
    assert_eq!(turn.text, "片段");
    let texts: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            RecordedEvent::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(texts, ["片段"]);
}

#[test]
fn responses_out_of_order_sequence_fails_closed() {
    let source = concat!(
        "data: {\"type\":\"response.output_text.delta\",\"sequence_number\":2,\"delta\":\"later\"}\n\n",
        "data: {\"type\":\"response.output_text.delta\",\"sequence_number\":1,\"delta\":\"earlier\"}\n\n",
        "data: {\"type\":\"response.completed\",\"sequence_number\":3}\n\n"
    );
    let error =
        decode_responses_response(&sse(source), Arc::new(aifluxon_core::NoopModelEventSink))
            .expect_err("out of order");
    assert!(error.message.contains("sequence_number 1 arrived after 2"));
}

#[test]
fn responses_incomplete_and_cancelled_streams_do_not_fabricate_a_terminal() {
    let incomplete = decode_responses_response(
        &sse(RESPONSES_INCOMPLETE),
        Arc::new(aifluxon_core::NoopModelEventSink),
    )
    .expect_err("incomplete");
    assert!(incomplete.message.contains("incomplete"));
    assert!(incomplete.message.contains("max_output_tokens"));

    let cancelled = decode_responses_response(
        &sse(RESPONSES_CANCELLED_STREAM),
        Arc::new(aifluxon_core::NoopModelEventSink),
    )
    .expect_err("cancelled");
    assert!(cancelled
        .message
        .contains("without an explicit terminal event"));

    let malformed = decode_responses_response(
        &sse("data: {bad}\n\n"),
        Arc::new(aifluxon_core::NoopModelEventSink),
    )
    .expect_err("malformed");
    assert!(malformed
        .message
        .contains("without an explicit terminal event"));
}

#[test]
fn responses_utf8_and_chunk_splits_match_recorded_result() {
    let (expected, _) = decode_responses(RESPONSES_FULL_TURN);
    assert_stable_across_byte_splits(
        RESPONSES_FULL_TURN,
        |response| {
            decode_responses_response(response, Arc::new(aifluxon_core::NoopModelEventSink))
                .unwrap()
        },
        &expected,
    );
}

#[test]
fn split_sse_preserves_reasoning_text_tools_usage_and_terminal() {
    let source = RESPONSES_FULL_TURN;
    for split in 0..=source.len() {
        let response = sse_chunks(vec![
            source.as_bytes()[..split].to_vec(),
            source.as_bytes()[split..].to_vec(),
        ]);
        let turn =
            decode_responses_response(&response, Arc::new(aifluxon_core::NoopModelEventSink))
                .unwrap();
        assert_eq!(turn.text, "done");
        assert_eq!(turn.reasoning, "think先分析");
        assert_eq!(turn.tool_calls.len(), 1);
        assert_eq!(turn.terminal, ProviderTerminal::ToolCalls);
        assert_eq!(turn.usage.unwrap()["total_tokens"], 3);
    }
}
