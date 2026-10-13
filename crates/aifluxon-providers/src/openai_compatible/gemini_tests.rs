use super::*;
use aifluxon_core::{
    Message, NoopModelEventSink, ProviderFeatureRequest, ProviderSessionKey, RunId,
};
use serde_json::json;
use std::sync::Mutex;

fn request() -> ModelTurnRequest {
    ModelTurnRequest {
        model: "gemini-3-flash-preview".into(),
        messages: vec![Message {
            role: MessageRole::User,
            content: vec![ContentPart::Text("inspect".into())],
            tool_calls: vec![],
            tool_call_id: None,
            provider_state: None,
        }],
        tools: vec![],
        session_key: ProviderSessionKey::from_cache_session("gemini-test"),
        run_id: RunId::new(),
        opaque_state: None,
        features: ProviderFeatureRequest {
            reasoning_effort: Some("minimal".into()),
            ..Default::default()
        },
    }
}

fn response(values: Vec<Value>) -> OpenAiWireResponse {
    let mut wire = values
        .into_iter()
        .map(|value| format!("data: {value}\n\n"))
        .collect::<String>();
    wire.push_str("data: [DONE]\n\n");
    OpenAiWireResponse {
        status: 200,
        content_type: Some("text/event-stream".into()),
        chunks: vec![wire.into_bytes()],
    }
}

fn tool_response(id: &str, signature: &str, parallel: bool) -> OpenAiWireResponse {
    let mut calls = vec![json!({"index": 0, "id": id, "type": "function",
        "function": {"name": "shell", "arguments": "{\"command\":\"echo ok\"}"}})];
    if parallel {
        calls.push(
            json!({"index": 1, "id": "parallel-unsigned", "type": "function",
            "function": {"name": "read_file", "arguments": "{\"path\":\"a.txt\"}"}}),
        );
    }
    response(vec![
        json!({"choices": [{"delta": {"tool_calls": calls}}]}),
        // The signature can arrive later in a metadata-only chunk.
        json!({"choices": [{"delta": {"tool_calls": [{"index": 0,
            "extra_content": {"google": {"thought_signature": signature}}}]}}]}),
        json!({"choices": [{"delta": {}, "finish_reason": "tool_calls"}]}),
    ])
}

fn append_turn(request: &mut ModelTurnRequest, turn: ModelTurn) {
    let message = Message {
        role: MessageRole::Assistant,
        content: vec![ContentPart::Text(turn.text)],
        tool_calls: turn.tool_calls.clone(),
        tool_call_id: None,
        provider_state: Some(turn.opaque),
    };
    // History persistence must keep provider metadata opaque and lossless.
    request
        .messages
        .push(serde_json::from_str(&serde_json::to_string(&message).unwrap()).unwrap());
    for call in turn.tool_calls {
        request.messages.push(Message {
            role: MessageRole::Tool,
            content: vec![ContentPart::Text("ok".into())],
            tool_calls: vec![],
            tool_call_id: Some(call.id),
            provider_state: Some(json!({"call_id": call.wire_call_id()})),
        });
    }
}

#[test]
fn gemini_signature_survives_every_stream_byte_split_and_parallel_calls() {
    let response = tool_response("native-call-a", " opaque+/== ", true);
    let bytes = &response.chunks[0];
    for split in 0..=bytes.len() {
        let fragmented = OpenAiWireResponse {
            chunks: vec![bytes[..split].to_vec(), bytes[split..].to_vec()],
            ..response.clone()
        };
        let turn = decode_chat_response(&fragmented, Arc::new(NoopModelEventSink)).unwrap();
        let mut request = request();
        append_turn(&mut request, turn);
        let body = build_chat_completions_body(&request);
        assert_eq!(
            body["messages"][1]["tool_calls"][0]["extra_content"]["google"]["thought_signature"],
            " opaque+/== "
        );
        assert!(body["messages"][1]["tool_calls"][1]
            .get("extra_content")
            .is_none());
        assert_eq!(body["messages"][2]["tool_call_id"], "native-call-a");
    }
}

struct RecordingTransport {
    requests: Mutex<Vec<OpenAiWireRequest>>,
    responses: Mutex<Vec<OpenAiWireResponse>>,
}

#[async_trait::async_trait]
impl OpenAiTransport for RecordingTransport {
    async fn execute(
        &self,
        request: OpenAiWireRequest,
    ) -> Result<OpenAiWireResponse, ProviderError> {
        self.requests.lock().unwrap().push(request);
        Ok(self.responses.lock().unwrap().remove(0))
    }
}

#[tokio::test]
async fn gemini_sequential_tool_rounds_replay_each_signature_and_thinking_config() {
    let transport = Arc::new(RecordingTransport {
        requests: Mutex::new(vec![]),
        responses: Mutex::new(vec![
            tool_response("call-a", "signature-A", true),
            tool_response("call-b", "signature-B", false),
            response(vec![
                json!({"choices": [{"delta": {"content": "done"}, "finish_reason": "stop"}]}),
            ]),
        ]),
    });
    let provider = OpenAiCompatibleProvider::with_transport(
        OpenAiCompatibleConfig::new(
            "gemini",
            "https://example.invalid/v1beta/openai",
            "test",
            OpenAiApiMode::ChatCompletions,
            false,
        ),
        transport.clone(),
    );
    let mut request = request();
    for _ in 0..2 {
        let turn = provider
            .next_turn(request.clone(), Arc::new(NoopModelEventSink))
            .await
            .unwrap();
        append_turn(&mut request, turn);
    }
    let final_turn = provider
        .next_turn(request, Arc::new(NoopModelEventSink))
        .await
        .unwrap();
    assert_eq!(final_turn.text, "done");
    let requests = transport.requests.lock().unwrap();
    for wire in requests.iter() {
        assert_eq!(
            wire.body["extra_body"]["google"]["thinking_config"]["thinking_level"],
            "minimal"
        );
        assert!(wire.body.get("reasoning_effort").is_none());
    }
    let messages = requests[2].body["messages"].as_array().unwrap();
    assert_eq!(
        messages[1]["tool_calls"][0]["extra_content"]["google"]["thought_signature"],
        "signature-A"
    );
    assert_eq!(
        messages[4]["tool_calls"][0]["extra_content"]["google"]["thought_signature"],
        "signature-B"
    );
    assert!(messages[1]["tool_calls"][1].get("extra_content").is_none());
}

#[test]
fn gemini_nonstreaming_and_message_signatures_are_preserved() {
    let signature = json!({"google": {"thought_signature": "message-signature"}});
    let wire = json!({"choices": [{"message": {"content": "done", "extra_content": signature}, "finish_reason": "stop"}]});
    let response = OpenAiWireResponse {
        status: 200,
        content_type: Some("application/json".into()),
        chunks: vec![wire.to_string().into_bytes()],
    };
    let turn = decode_chat_response(&response, Arc::new(NoopModelEventSink)).unwrap();
    let mut request = request();
    append_turn(&mut request, turn);
    assert_eq!(
        build_chat_completions_body(&request)["messages"][1]["extra_content"],
        signature
    );
}

#[test]
fn gemini_thinking_matches_model_capabilities_and_default_stays_dynamic() {
    for (model, effort, key, expected) in [
        (
            "gemini-3-flash-preview",
            "minimal",
            "thinking_level",
            json!("minimal"),
        ),
        (
            "gemini-3-pro-preview",
            "medium",
            "thinking_level",
            json!("low"),
        ),
        (
            "gemini-3.1-pro-preview",
            "medium",
            "thinking_level",
            json!("medium"),
        ),
        (
            "gemini-3.7-flash",
            "minimal",
            "thinking_level",
            json!("low"),
        ),
        ("gemini-2.5-flash", "none", "thinking_budget", json!(0)),
        ("gemini-2.5-pro", "none", "thinking_budget", json!(1024)),
        ("gemini-2.5-pro", "medium", "thinking_budget", json!(8192)),
    ] {
        let mut body = json!({});
        crate::gemini::apply_chat_reasoning(&mut body, model, Some(effort));
        assert_eq!(
            body["extra_body"]["google"]["thinking_config"][key], expected,
            "{model}"
        );
        assert!(body.get("reasoning_effort").is_none());
    }
    let mut body = json!({});
    crate::gemini::apply_chat_reasoning(&mut body, "gemini-3.5-flash", None);
    assert_eq!(
        body["extra_body"]["google"]["thinking_config"],
        json!({"include_thoughts": true})
    );
}
