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
