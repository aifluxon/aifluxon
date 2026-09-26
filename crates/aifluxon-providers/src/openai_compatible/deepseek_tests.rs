use super::*;
use aifluxon_core::{
    NoopModelEventSink, ProviderFeatureRequest, ProviderSessionKey, ProviderTerminal, RunId,
};
use serde_json::json;
use std::collections::VecDeque;
use std::sync::Mutex;

fn decode(source: &str) -> Result<ModelTurn, ProviderError> {
    let mut decoder = LiveStreamDecoder::new(
        OpenAiApiMode::ChatCompletions,
        false,
        Some("text/event-stream".into()),
    )
    .with_deepseek_contract();
    // One-byte reads exercise UTF-8, framing, and argument splits together.
    for byte in source.as_bytes() {
        decoder.push(&[*byte], &NoopModelEventSink)?;
    }
    decoder.finish(&NoopModelEventSink)
}

fn event(value: Value) -> String {
    format!("data: {value}\n\n")
}
fn finish(reason: &str) -> String {
    event(json!({"choices": [{"delta": {}, "finish_reason": reason}]})) + "data: [DONE]\n\n"
}
fn tool(arguments: &str) -> String {
    event(
        json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "id": "call_1", "function": {"name": "read_file", "arguments": arguments}}]}}]}),
    )
}

#[test]
fn deepseek_repeated_tool_metadata_and_unicode_arguments_are_lossless() {
    let source = tool("{\"path\":\"") + &tool("模型.txt\"}") + &finish("tool_calls");
    let turn = decode(&source).unwrap();
    assert_eq!(turn.tool_calls.len(), 1);
    assert_eq!(turn.tool_calls[0].name, "read_file");
    assert_eq!(
        turn.tool_calls[0].provider_call_id.as_deref(),
        Some("call_1")
    );
    assert_eq!(turn.tool_calls[0].arguments, json!({"path": "模型.txt"}));
    assert_eq!(turn.terminal, ProviderTerminal::ToolCalls);
}

#[test]
fn deepseek_never_dispatches_truncated_or_malformed_tool_calls() {
    for (source, code) in [
        (tool("{}"), "PROVIDER_STREAM_CLOSED"),
        (
            tool("{}") + finish("tool_calls").trim_end(),
            "PROVIDER_STREAM_CLOSED",
        ),
        (tool("{}") + "data: [DONE]\n\n", "PROVIDER_STREAM_CLOSED"),
        (
            tool("{\"path\":") + &finish("tool_calls"),
            "PROVIDER_INVALID_TOOL_CALL",
        ),
        (tool("{}") + &finish("length"), "PROVIDER_OUTPUT_LIMIT"),
        (
            tool("{}") + &finish("content_filter"),
            "PROVIDER_FINISH_ERROR",
        ),
        (
            "data: {bad}\n\n".to_string() + &finish("stop"),
            "PROVIDER_MALFORMED_RESPONSE",
        ),
        (
            "data: {\"error\":{\"message\":\"secret upstream detail\"}}\n\n".to_string(),
            "PROVIDER_FINISH_ERROR",
        ),
    ] {
        let error = decode(&source).unwrap_err();
        assert!(error.message.starts_with(code), "{error}");
        assert!(!error.message.contains("secret upstream detail"));
    }
}

#[test]
fn deepseek_requires_an_answer_after_reasoning_and_preserves_trailing_usage() {
    let reasoning = event(json!({"choices": [{"delta": {"reasoning_content": " plan\n"}}]}));
    assert!(decode(&(reasoning.clone() + &finish("stop")))
        .unwrap_err()
        .message
        .starts_with("PROVIDER_EMPTY_RESPONSE"));
    let source = reasoning
        + &tool("{}")
        + &event(json!({"choices": [{"delta": {}, "finish_reason": "tool_calls"}]}))
        + &event(json!({"choices": [], "usage": {"total_tokens": 23}}))
        + "data: [DONE]\n\n";
    let turn = decode(&source).unwrap();
    assert_eq!(turn.usage.unwrap()["total_tokens"], 23);
    assert_eq!(turn.reasoning, " plan\n");
}

struct ScriptedTransport {
    replies: Mutex<VecDeque<String>>,
    requests: Mutex<Vec<OpenAiWireRequest>>,
    keep_open: bool,
}

#[async_trait::async_trait]
impl OpenAiTransport for ScriptedTransport {
    async fn execute(&self, _: OpenAiWireRequest) -> Result<OpenAiWireResponse, ProviderError> {
        unreachable!()
    }
    async fn stream(
        &self,
        request: OpenAiWireRequest,
    ) -> Result<(OpenAiStreamHead, OpenAiBodyStream), ProviderError> {
        self.requests.lock().unwrap().push(request);
        let source = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected retry");
        let stream = futures_util::stream::iter(vec![Ok(source.into_bytes())]);
        let body: OpenAiBodyStream = if self.keep_open {
            Box::pin(stream.chain(futures_util::stream::pending()))
        } else {
            Box::pin(stream)
        };
        Ok((
            OpenAiStreamHead {
                retry_after: None,
                status: 200,
                content_type: Some("text/event-stream".into()),
            },
            body,
        ))
    }
}

fn setup(
    replies: Vec<String>,
    keep_open: bool,
) -> (
    OpenAiCompatibleProvider,
    Arc<ScriptedTransport>,
    ModelTurnRequest,
) {
    let transport = Arc::new(ScriptedTransport {
        replies: Mutex::new(replies.into()),
        requests: Mutex::new(Vec::new()),
        keep_open,
    });
    let config = OpenAiCompatibleConfig::new(
        "deepseek",
        "https://provider.invalid",
        "secret",
        OpenAiApiMode::ChatCompletions,
        false,
    );
    let request = ModelTurnRequest {
        model: "deepseek-v4-flash".into(),
        messages: Vec::new(),
        tools: Vec::new(),
        session_key: ProviderSessionKey::from_cache_session("contract-test"),
        run_id: RunId::new(),
        opaque_state: None,
        features: ProviderFeatureRequest::default(),
    };
    (
        OpenAiCompatibleProvider::with_transport(config, transport.clone()),
        transport,
        request,
    )
}

#[tokio::test]
async fn deepseek_recovers_before_tool_dispatch_and_bounds_retries() {
    let truncated = tool("{\"path\":");
    let (provider, transport, request) = setup(
        vec![truncated.clone(), tool("{}") + &finish("tool_calls")],
        false,
    );
    let turn = provider
        .next_turn(request, Arc::new(NoopModelEventSink))
        .await
        .unwrap();
    assert_eq!(turn.tool_calls.len(), 1);
    {
        let requests = transport.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].body, requests[1].body);
        assert_eq!(requests[0].body["max_tokens"], 256_000);
    }
    let (provider, transport, request) = setup(vec![truncated; 3], false);
    assert!(provider
        .next_turn(request, Arc::new(NoopModelEventSink))
        .await
        .is_err());
    assert_eq!(transport.requests.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn deepseek_returns_at_done_without_waiting_for_socket_close() {
    let (provider, _, request) = setup(vec![tool("{}") + &finish("tool_calls")], true);
    let turn = tokio::time::timeout(
        Duration::from_millis(200),
        provider.next_turn(request, Arc::new(NoopModelEventSink)),
    )
    .await
    .expect("must not wait for HTTP EOF")
    .unwrap();
    assert_eq!(turn.tool_calls.len(), 1);
}

#[tokio::test]
async fn deepseek_does_not_replay_visible_output_or_hosted_work() {
    let partial = event(json!({"choices": [{"delta": {"content": "partial answer"}}]}));
    let (provider, transport, request) = setup(vec![partial], false);
    assert!(provider
        .next_turn(request, Arc::new(NoopModelEventSink))
        .await
        .is_err());
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
    let (provider, transport, mut request) = setup(vec![tool("{")], false);
    request.features.web_search = true;
    assert!(provider
        .next_turn(request, Arc::new(NoopModelEventSink))
        .await
        .is_err());
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn deepseek_retry_delay_is_cancelled_with_the_provider_future() {
    let (provider, transport, request) = setup(vec![tool("{")], false);
    assert!(tokio::time::timeout(
        Duration::from_millis(30),
        provider.next_turn(request, Arc::new(NoopModelEventSink))
    )
    .await
    .is_err());
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn deepseek_responses_also_returns_at_terminal_without_socket_close() {
    let source = event(
        json!({"type": "response.completed", "response": {"status": "completed", "output": [{"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "完成"}]}]}}),
    );
    let (mut provider, transport, request) = setup(vec![source], true);
    provider.config.api_mode = OpenAiApiMode::Responses;
    let turn = tokio::time::timeout(
        Duration::from_millis(200),
        provider.next_turn(request, Arc::new(NoopModelEventSink)),
    )
    .await
    .expect("must not wait for HTTP EOF")
    .unwrap();
    assert_eq!(turn.text, "完成");
    assert_eq!(
        transport.requests.lock().unwrap()[0].body["max_output_tokens"],
        256_000
    );
}

#[test]
fn deepseek_responses_accepts_tool_after_reasoning_but_rejects_incomplete_arguments() {
    for arguments in ["{}", "{\"path\":"] {
        let mut decoder = LiveStreamDecoder::new(
            OpenAiApiMode::Responses,
            false,
            Some("text/event-stream".into()),
        )
        .with_deepseek_contract();
        let source = event(json!({"type": "response.completed", "response": {
            "output": [
                {"type": "reasoning", "id": "reasoning-1", "status": "completed"},
                {"type": "function_call", "id": "fc-1", "call_id": "call-1", "name": "read_file", "arguments": arguments}
            ]
        }}));
        decoder
            .push(source.as_bytes(), &NoopModelEventSink)
            .unwrap();
        let result = decoder.finish(&NoopModelEventSink);
        if arguments == "{}" {
            let turn = result.unwrap();
            assert_eq!(turn.terminal, ProviderTerminal::ToolCalls);
            assert_eq!(turn.tool_calls.len(), 1);
            assert_eq!(
                turn.tool_calls[0].provider_call_id.as_deref(),
                Some("call-1")
            );
        } else {
            assert!(result
                .unwrap_err()
                .message
                .starts_with("PROVIDER_INVALID_TOOL_CALL"));
        }
    }
}

#[tokio::test]
async fn pooled_transport_reuses_connection_without_reusing_credentials() {
    use std::io::{BufRead, BufReader, Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let (socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut reader = BufReader::new(socket);
        for expected in ["first-credential", "second-credential"] {
            let mut headers = String::new();
            loop {
                let mut line = String::new();
                assert!(reader.read_line(&mut line).unwrap() > 0);
                if line == "\r\n" {
                    break;
                }
                headers.push_str(&line.to_ascii_lowercase());
            }
            assert!(headers.contains(&format!("authorization: bearer {expected}\r\n")));
            let length: usize = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length: "))
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            reader.read_exact(&mut vec![0; length]).unwrap();
            let body = r#"{"choices":[{"message":{"content":"ok"},"finish_reason":"stop"}]}"#;
            write!(
                reader.get_mut(),
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
            reader.get_mut().flush().unwrap();
        }
    });
    for credential in ["first-credential", "second-credential"] {
        let provider = OpenAiCompatibleProvider::configured(OpenAiCompatibleConfig::new(
            "deepseek",
            &endpoint,
            credential,
            OpenAiApiMode::ChatCompletions,
            false,
        ));
        let (_, _, request) = setup(Vec::new(), false);
        assert_eq!(
            provider
                .next_turn(request, Arc::new(NoopModelEventSink))
                .await
                .unwrap()
                .text,
            "ok"
        );
    }
    server.join().unwrap();
}
