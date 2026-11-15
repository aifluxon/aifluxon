use super::*;
use aifluxon_core::{NoopModelEventSink, ProviderFeatureRequest, ProviderSessionKey, RunId};
use serde_json::json;
use std::collections::VecDeque;
use std::sync::Mutex;

struct Script {
    replies: Mutex<VecDeque<(u16, String)>>,
    count: Mutex<usize>,
    keep_open: bool,
    retry_after: Option<Duration>,
}
#[async_trait::async_trait]
impl OpenAiTransport for Script {
    async fn execute(&self, _: OpenAiWireRequest) -> Result<OpenAiWireResponse, ProviderError> {
        unreachable!()
    }
    async fn stream(
        &self,
        _: OpenAiWireRequest,
    ) -> Result<(OpenAiStreamHead, OpenAiBodyStream), ProviderError> {
        *self.count.lock().unwrap() += 1;
        let (status, body) = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected replay");
        let stream = futures_util::stream::iter(vec![Ok(body.into_bytes())]);
        let stream: OpenAiBodyStream = if self.keep_open {
            Box::pin(stream.chain(futures_util::stream::pending()))
        } else {
            Box::pin(stream)
        };
        Ok((
            OpenAiStreamHead {
                status,
                content_type: Some("text/event-stream".into()),
                retry_after: self.retry_after,
            },
            stream,
        ))
    }
}
fn event(value: Value) -> String {
    format!("data: {value}\n\n")
}
fn tool(mode: OpenAiApiMode, args: &str, finish: bool) -> String {
    match mode {
        OpenAiApiMode::ChatCompletions => {
            let mut body = event(
                json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"echo","arguments":args}}]},"finish_reason": if finish { Some("tool_calls") } else { None }}]}),
            );
            if finish {
                body.push_str("data: [DONE]\n\n");
            }
            body
        }
        OpenAiApiMode::Responses => {
            let mut body = event(
                json!({"type":"response.output_item.done","output_index":2,"item":{"type":"function_call","call_id":"call_1","name":"echo","arguments":args}}),
            );
            if finish {
                body +=
                    &event(json!({"type":"response.completed","response":{"status":"completed"}}));
            }
            body
        }
    }
}
fn setup(
    family: &str,
    mode: OpenAiApiMode,
    replies: Vec<(u16, String)>,
    keep_open: bool,
    retry_after: Option<Duration>,
) -> (OpenAiCompatibleProvider, Arc<Script>, ModelTurnRequest) {
    let script = Arc::new(Script {
        replies: Mutex::new(replies.into()),
        count: Mutex::new(0),
        keep_open,
        retry_after,
    });
    let config =
        OpenAiCompatibleConfig::new(family, "https://provider.invalid/v1", "secret", mode, false);
    let model = match family {
        "deepseek" => "deepseek-v4-flash",
        "qwen" => "qwen3-max",
        "kimi" => "kimi-k2.6",
        "gemini" => "gemini-3-pro",
        _ => "gpt-5.4",
    };
    let request = ModelTurnRequest {
        model: model.into(),
        messages: vec![],
        tools: vec![],
        session_key: ProviderSessionKey::from_cache_session("reliability-test"),
        run_id: RunId::new(),
        opaque_state: None,
        features: ProviderFeatureRequest::default(),
    };
    (
        OpenAiCompatibleProvider::with_transport(config, script.clone()),
        script,
        request,
    )
}
fn routes() -> Vec<(&'static str, OpenAiApiMode)> {
    use OpenAiApiMode::*;
    vec![
        ("openai", ChatCompletions),
        ("openai", Responses),
        ("deepseek", ChatCompletions),
        ("deepseek", Responses),
        ("qwen", ChatCompletions),
        ("qwen", Responses),
        ("kimi", ChatCompletions),
        ("gemini", ChatCompletions),
        ("codex", Responses),
        ("custom", ChatCompletions),
        ("custom", Responses),
    ]
}

#[tokio::test]
async fn every_api_family_recovers_unpublished_truncation_then_returns_at_terminal() {
    for (family, mode) in routes() {
        let (provider, script, request) = setup(
            family,
            mode,
            vec![(200, tool(mode, "{", false)), (200, tool(mode, "{}", true))],
            false,
            None,
        );
        let turn = provider
            .next_turn(request, Arc::new(NoopModelEventSink))
            .await
            .unwrap();
        assert_eq!(turn.tool_calls[0].arguments, json!({}), "{family}");
        assert_eq!(*script.count.lock().unwrap(), 2, "{family}");
        let (provider, _, request) = setup(
            family,
            mode,
            vec![(200, tool(mode, "{}", true))],
            true,
            None,
        );
        tokio::time::timeout(
            Duration::from_millis(200),
            provider.next_turn(request, Arc::new(NoopModelEventSink)),
        )
        .await
        .expect("terminal must release open socket")
        .unwrap();
    }
}

#[tokio::test]
async fn every_api_family_rejects_invalid_tool_arguments_with_bounded_retries() {
    for (family, mode) in routes() {
        let (provider, script, request) = setup(
            family,
            mode,
            vec![(200, tool(mode, "{", true)); 3],
            false,
            None,
        );
        let error = provider
            .next_turn(request, Arc::new(NoopModelEventSink))
            .await
            .unwrap_err();
        assert!(
            error.message.starts_with("PROVIDER_INVALID_TOOL_CALL:"),
            "{family}: {error}"
        );
        assert_eq!(*script.count.lock().unwrap(), 3);
    }
}

#[tokio::test]
async fn throttling_is_bounded_and_does_not_replay_hosted_work_or_long_retry_after() {
    for (status, hosted, hint, attempts) in [
        (429, false, None, 3),
        (503, true, None, 1),
        (400, false, None, 1),
        (429, false, Some(Duration::from_secs(61)), 1),
    ] {
        let (provider, script, mut request) = setup(
            "openai",
            OpenAiApiMode::Responses,
            vec![(status, String::new()); attempts],
            false,
            hint,
        );
        request.features.web_search = hosted;
        assert!(provider
            .next_turn(request, Arc::new(NoopModelEventSink))
            .await
            .is_err());
        assert_eq!(*script.count.lock().unwrap(), attempts);
    }
}

#[tokio::test]
async fn published_reasoning_is_never_replayed() {
    let body = event(json!({"choices":[{"delta":{"reasoning_content":"thinking"}}]}));
    let (provider, script, request) = setup(
        "openai",
        OpenAiApiMode::ChatCompletions,
        vec![(200, body)],
        false,
        None,
    );
    assert!(provider
        .next_turn(request, Arc::new(NoopModelEventSink))
        .await
        .is_err());
    assert_eq!(*script.count.lock().unwrap(), 1);
}

#[test]
fn conflicting_responses_terminal_and_incomplete_json_are_rejected() {
    for value in [
        json!({"type":"response.completed","response":{"status":"incomplete","output_text":"partial"}}),
        json!({"object":"response","status":"in_progress","output_text":"partial"}),
    ] {
        let response = OpenAiWireResponse {
            status: 200,
            content_type: Some("application/json".into()),
            chunks: vec![value.to_string().into_bytes()],
        };
        assert!(decode_responses_response(&response, Arc::new(NoopModelEventSink)).is_err());
    }
    let response = OpenAiWireResponse {
        status: 200,
        content_type: Some("application/json".into()),
        chunks: vec![
            json!({"choices":[{"message":{"content":"partial"},"finish_reason":"length"}]})
                .to_string()
                .into_bytes(),
        ],
    };
    assert!(
        decode_chat_response(&response, Arc::new(NoopModelEventSink))
            .unwrap_err()
            .message
            .starts_with("PROVIDER_OUTPUT_LIMIT:")
    );
}

#[test]
fn responses_rejects_non_boolean_native_end_turn() {
    let source = event(json!({"type":"response.completed", "response":{
        "status":"completed", "end_turn":"false", "output":[{
            "type":"message", "content":[{"type":"output_text", "text":"visible"}]
        }]
    }}));
    let mut decoder = OpenAiStreamDecoder::new(
        OpenAiApiMode::Responses,
        false,
        Some("text/event-stream".into()),
    );
    let error = decoder
        .push(source.as_bytes(), &NoopModelEventSink)
        .unwrap_err();
    assert!(error.message.starts_with("PROVIDER_MALFORMED_RESPONSE:"));
}

#[test]
fn wire_diagnostics_contain_only_termination_facts_and_counts() {
    for mode in [OpenAiApiMode::Responses, OpenAiApiMode::ChatCompletions] {
        let mut decoder = OpenAiStreamDecoder::new(mode, false, Some("text/event-stream".into()));
        decoder
            .push(
                tool(mode, r#"{"private_argument":"do-not-log"}"#, true).as_bytes(),
                &NoopModelEventSink,
            )
            .unwrap();
        let turn = decoder.finish(&NoopModelEventSink).unwrap();
        let diagnostic = &turn.opaque["provider_diagnostics"];
        assert_eq!(diagnostic["decoded_tool_calls"], 1);
        assert_eq!(diagnostic["observed_tool_calls"], 1);
        assert!(!diagnostic.to_string().contains("do-not-log"));
        if mode == OpenAiApiMode::Responses {
            assert_eq!(diagnostic["terminal_event"], "response.completed");
        } else {
            assert_eq!(diagnostic["terminal_status"], "tool_calls");
        }
    }
}

#[test]
fn retry_and_diagnostic_classification_ignore_message_prefixes() {
    use aifluxon_core::ProviderErrorCode;
    let spoofed = ProviderError::message("PROVIDER_TRANSIENT_TRANSPORT: remote text");
    assert!(!reliability::retryable_error(&spoofed));
    assert_eq!(
        diagnostics::failure_code(&spoofed),
        "PROVIDER_RESPONSE_ERROR"
    );
    let structured = ProviderError::new(ProviderErrorCode::StreamClosed, "localized text");
    assert!(reliability::retryable_error(&structured));
    assert_eq!(
        diagnostics::failure_code(&structured),
        "PROVIDER_STREAM_CLOSED"
    );
    assert!(!reliability::retryable_error(&ProviderError::cancelled()));
}

#[tokio::test]
async fn http_failure_metadata_survives_redaction_and_bounded_retries() {
    use aifluxon_core::ProviderErrorCode;
    for (status, code) in [
        (429, ProviderErrorCode::RateLimited),
        (503, ProviderErrorCode::TransientHttp),
        (401, ProviderErrorCode::Http),
    ] {
        let (provider, script, request) = setup(
            "custom",
            OpenAiApiMode::ChatCompletions,
            vec![(status, "secret upstream body".into())],
            false,
            Some(Duration::from_secs(61)),
        );
        let error = provider
            .next_turn(request, Arc::new(NoopModelEventSink))
            .await
            .unwrap_err();
        assert_eq!(error.code, code);
        assert_eq!(error.http_status, Some(status));
        assert_eq!(error.retry_after, Some(Duration::from_secs(61)));
        assert!(!error.message.contains("secret"));
        assert_eq!(*script.count.lock().unwrap(), 1);
    }
}

#[tokio::test]
async fn instance_proxy_configuration_routes_requests_and_redacts_credentials() {
    use std::io::{BufRead, BufReader, Write};
    let mut servers = Vec::new();
    let mut providers = Vec::new();
    for label in ["first", "second"] {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let mut config = OpenAiCompatibleConfig::new(
            "custom",
            "http://provider.invalid/v1",
            label,
            OpenAiApiMode::ChatCompletions,
            false,
        );
        config.http_client_tuning.proxy_url = Some(format!(
            "http://proxy-user:proxy-password@{}",
            listener.local_addr().unwrap()
        ));
        config.http_client_tuning.http1_only = label == "first";
        assert!(!format!("{config:?}").contains("proxy-password"));
        servers.push(std::thread::spawn(move || {
            let (socket, _) = listener.accept().unwrap();
            socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            let mut reader = BufReader::new(socket);
            let mut headers = String::new();
            loop {
                let mut line = String::new();
                assert!(reader.read_line(&mut line).unwrap() > 0);
                if line == "\r\n" { break; }
                headers.push_str(&line.to_ascii_lowercase());
            }
            assert!(headers.starts_with("post http://provider.invalid/"));
            assert!(headers.contains(&format!("authorization: bearer {label}\r\n")));
            assert!(headers.contains("proxy-authorization: basic "));
            let body = r#"{"choices":[{"message":{"content":"ok"},"finish_reason":"stop"}]}"#;
            write!(reader.get_mut(), "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        }));
        providers.push(OpenAiCompatibleProvider::configured(config));
    }
    for provider in providers {
        let (_, _, request) = setup(
            "custom",
            OpenAiApiMode::ChatCompletions,
            vec![],
            false,
            None,
        );
        assert_eq!(
            provider
                .next_turn(request, Arc::new(NoopModelEventSink))
                .await
                .unwrap()
                .text,
            "ok"
        );
    }
    for server in servers {
        server.join().unwrap();
    }
    let tuning = crate::HttpClientTuning {
        proxy_url: Some("http://secret-user:secret-password@[invalid".into()),
        ..Default::default()
    };
    assert!(!format!("{tuning:?}").contains("secret-password"));
    let error = ReqwestOpenAiTransport::new(tuning).client().unwrap_err();
    assert!(!error.message.contains("secret-user"));
    assert!(!error.message.contains("secret-password"));
}

#[tokio::test]
async fn instance_read_timeout_is_applied_to_response_body() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut buffer = [0; 4096];
        assert!(socket.read(&mut buffer).unwrap() > 0);
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1024\r\n\r\n")
            .unwrap();
        socket.flush().unwrap();
        std::thread::sleep(Duration::from_millis(150));
    });
    let transport = ReqwestOpenAiTransport::new(crate::HttpClientTuning {
        read_timeout: Duration::from_millis(25),
        ..Default::default()
    });
    let error = transport
        .execute(OpenAiWireRequest {
            url,
            api_key: "secret".into(),
            extra_headers: vec![],
            body: json!({}),
        })
        .await
        .unwrap_err();
    assert_eq!(
        error.code,
        aifluxon_core::ProviderErrorCode::TransientTransport
    );
    assert!(!error.message.contains("secret"));
    server.join().unwrap();
}
