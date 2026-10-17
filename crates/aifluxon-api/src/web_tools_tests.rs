use crate::*;
use aifluxon_providers::openai_compatible::{OpenAiApiMode, OpenAiCompatibleConfig};
use aifluxon_providers::{deepseek::DeepSeekWebClient, OpenAiCompatibleProvider};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Reply {
    path: &'static str,
    status: u16,
    content_type: &'static str,
    body: String,
}

fn json_reply(path: &'static str, status: u16, value: Value) -> Reply {
    Reply {
        path,
        status,
        content_type: "application/json",
        body: value.to_string(),
    }
}

fn server(replies: Vec<Reply>) -> (String, std::thread::JoinHandle<Vec<Value>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let worker = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for reply in replies {
            let deadline = Instant::now() + Duration::from_secs(5);
            let socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            Instant::now() < deadline,
                            "missing request for {}",
                            reply.path
                        );
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("{error}"),
                }
            };
            socket.set_nonblocking(false).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut reader = BufReader::new(socket);
            let mut headers = String::new();
            loop {
                let mut line = String::new();
                assert!(reader.read_line(&mut line).unwrap() > 0);
                if line == "\r\n" {
                    break;
                }
                headers.push_str(&line.to_ascii_lowercase());
            }
            assert!(headers.starts_with(&format!("post {} http/1.1", reply.path)));
            assert!(headers.contains("authorization: bearer web-test-key\r\n"));
            let length: usize = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length: "))
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            requests.push(serde_json::from_slice(&body).unwrap());
            write!(reader.get_mut(), "HTTP/1.1 {} OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\nRetry-After: 0\r\n\r\n{}",
                reply.status, reply.content_type, reply.body.len(), reply.body).unwrap();
        }
        requests
    });
    (url, worker)
}

fn model_reply(mode: OpenAiApiMode, name: Option<&str>, arguments: Value) -> Reply {
    let body = match mode {
        OpenAiApiMode::ChatCompletions => {
            let (delta, reason) = match name {
                Some(name) => (
                    json!({ "tool_calls": [{ "index": 0, "id": name, "type": "function", "function": { "name": name, "arguments": arguments.to_string() } }] }),
                    "tool_calls",
                ),
                None => (
                    json!({ "content": "Answer from https://example.com/source" }),
                    "stop",
                ),
            };
            format!(
                "data: {}\n\ndata: [DONE]\n\n",
                json!({ "choices": [{ "index": 0, "delta": delta, "finish_reason": reason }] })
            )
        }
        OpenAiApiMode::Responses => {
            let output = match name {
                Some(name) => {
                    json!([{ "type": "function_call", "id": name, "call_id": name, "name": name, "arguments": arguments.to_string() }])
                }
                None => {
                    json!([{ "type": "message", "role": "assistant", "content": [{ "type": "output_text", "text": "Answer from https://example.com/source" }] }])
                }
            };
            format!(
                "data: {}\n\n",
                json!({ "type": "response.completed", "response": { "status": "completed", "output": output } })
            )
        }
    };
    Reply {
        path: if mode == OpenAiApiMode::Responses {
            "/responses"
        } else {
            "/chat/completions"
        },
        status: 200,
        content_type: "text/event-stream",
        body,
    }
}

async fn web_roundtrip(mode: OpenAiApiMode) {
    let query = json!({ "query": "physics site:example.com lang:en after:2026", "count": 2, "need_content": true, "ori_question": "original", "localization": "" });
    let (url, worker) = server(vec![
        model_reply(mode, Some("web_search"), query.clone()),
        json_reply(
            "/v1/tools/search",
            200,
            json!({ "results": [{ "title": "Source", "url": "https://example.com/source", "summary": "Evidence", "content": "details" }] }),
        ),
        model_reply(
            mode,
            Some("web_fetch"),
            json!({ "url": "https://example.com/source" }),
        ),
        json_reply(
            "/v1/tools/fetch",
            200,
            json!({ "title": "Source", "content": "# Verified source\nEvidence." }),
        ),
        model_reply(mode, None, Value::Null),
    ]);
    let providers = ProviderRegistry::new();
    providers
        .register(
            ProviderId::new("deepseek"),
            OpenAiCompatibleProvider::configured(OpenAiCompatibleConfig::new(
                "deepseek",
                &url,
                "web-test-key",
                mode,
                false,
            )),
        )
        .unwrap();
    let tools = ToolRegistry::new();
    register_deepseek_web_tools(&tools, &format!("{url}/v1/"), "web-test-key").unwrap();
    let backend = Aifluxon::builder()
        .provider_registry(providers)
        .tool_registry(tools)
        .tool_policy(Arc::new(AllowAllToolPolicy))
        .event_sink(Arc::new(NoopRunEventSink))
        .workspace(Arc::new(EmptyWorkspace))
        .build()
        .unwrap();
    let mut request = user_prompt_request(
        ProviderId::new("deepseek"),
        "custom-future-alias",
        "search and read the source",
        None,
        RunLimits::unlimited(),
    );
    request.features.web_search = true;
    let mut handle = backend.start(request).await.unwrap();
    let mut tools_started = Vec::new();
    let mut completed = 0;
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(envelope) = handle.events().next().await {
            match envelope.event {
                RunEvent::ToolStarted { name, .. } => tools_started.push(name),
                RunEvent::ToolFinished { result, .. } => assert_eq!(result["ok"], true),
                RunEvent::Completed { .. } => completed += 1,
                RunEvent::Failed { message } => panic!("{message}"),
                _ => {}
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(completed, 1);
    assert_eq!(tools_started, ["web_search", "web_fetch"]);
    let requests = worker.join().unwrap();
    assert_eq!(requests[1], query);
    assert_eq!(requests[3]["url"], "https://example.com/source");
    for request in [&requests[0], &requests[2], &requests[4]] {
        assert_eq!(request["model"], "custom-future-alias");
        assert_eq!(request["tools"].as_array().unwrap().len(), 2);
        assert!(request["tools"]
            .as_array()
            .unwrap()
            .iter()
            .all(|tool| tool["type"] == "function"));
    }
    let replay = requests[4].to_string();
    assert!(replay.contains("Verified source"));
    assert!(replay.contains("https://example.com/source"));
}

#[tokio::test]
async fn search_fetch_and_answer_use_chat_completions_for_unknown_model() {
    web_roundtrip(OpenAiApiMode::ChatCompletions).await;
}

#[tokio::test]
async fn search_fetch_and_answer_use_responses_for_unknown_model() {
    web_roundtrip(OpenAiApiMode::Responses).await;
}

#[tokio::test]
async fn web_errors_are_redacted_without_retrying_invalid_requests() {
    let (url, worker) = server(vec![json_reply(
        "/v1/tools/fetch",
        403,
        json!({ "error": { "message": "denied web-test-key" } }),
    )]);
    let client = DeepSeekWebClient::new(&url, "web-test-key").unwrap();
    let error = client
        .execute("web_fetch", json!({ "url": "https://example.com" }))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("403"));
    assert!(!error.contains("web-test-key"));
    assert_eq!(worker.join().unwrap().len(), 1);
}

#[tokio::test]
async fn web_search_retries_rate_limits_and_preserves_empty_results() {
    let (url, worker) = server(vec![
        json_reply(
            "/v1/tools/search",
            429,
            json!({ "error": { "message": "slow down" } }),
        ),
        json_reply("/v1/tools/search", 200, json!({ "results": [] })),
    ]);
    let client = DeepSeekWebClient::new(&url, "web-test-key").unwrap();
    assert_eq!(
        client
            .execute("web_search", json!({ "query": "test" }))
            .await
            .unwrap(),
        json!({ "results": [] })
    );
    let requests = worker.join().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0], requests[1]);
}

#[tokio::test]
async fn invalid_web_response_is_not_returned_as_search_results() {
    let (url, worker) = server(vec![json_reply(
        "/v1/tools/search",
        200,
        json!({ "unexpected": [] }),
    )]);
    let client = DeepSeekWebClient::new(&url, "web-test-key").unwrap();
    assert!(client
        .execute("web_search", json!({ "query": "test" }))
        .await
        .is_err());
    worker.join().unwrap();
}

#[tokio::test]
async fn provider_rejection_does_not_switch_the_selected_api_mode() {
    for mode in [OpenAiApiMode::ChatCompletions, OpenAiApiMode::Responses] {
        let path = if mode == OpenAiApiMode::Responses {
            "/responses"
        } else {
            "/chat/completions"
        };
        let (url, worker) = server(vec![json_reply(
            path,
            400,
            json!({ "error": { "message": "unsupported model for this endpoint" } }),
        )]);
        let registry = ProviderRegistry::new();
        registry
            .register(
                ProviderId::new("deepseek"),
                OpenAiCompatibleProvider::configured(OpenAiCompatibleConfig::new(
                    "deepseek",
                    url,
                    "web-test-key",
                    mode,
                    false,
                )),
            )
            .unwrap();
        let backend = Aifluxon::builder()
            .provider_registry(registry)
            .tool_registry(ToolRegistry::new())
            .tool_policy(Arc::new(AllowAllToolPolicy))
            .event_sink(Arc::new(NoopRunEventSink))
            .workspace(Arc::new(EmptyWorkspace))
            .build()
            .unwrap();
        let mut handle = backend
            .start(user_prompt_request(
                ProviderId::new("deepseek"),
                "future-alias",
                "hello",
                None,
                RunLimits::unlimited(),
            ))
            .await
            .unwrap();
        let mut failure = None;
        tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(event) = handle.events().next().await {
                if let RunEvent::Failed { message } = event.event {
                    failure = Some(message);
                }
            }
        })
        .await
        .unwrap();
        assert!(failure
            .unwrap()
            .contains("unsupported model for this endpoint"));
        assert_eq!(worker.join().unwrap().len(), 1);
    }
}
