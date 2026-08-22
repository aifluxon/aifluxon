use super::{
    build_chat_completions_body, build_responses_body, decorate, validate_deepseek_image_request,
    ApiFamily, OpenAiApiMode, OpenAiCompatibleConfig, OpenAiCompatibleProvider, OpenAiTransport,
    OpenAiWireRequest, OpenAiWireResponse,
};
use aifluxon_core::{
    ContentPart, ContinuationReason, ImageContent, Message, MessageRole, ModelProvider,
    ModelTurnRequest, NoopModelEventSink, ProviderId, ProviderSessionKey, ProviderTerminal, RunId,
};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

struct RecordingTransport {
    responses: Mutex<Vec<OpenAiWireResponse>>,
    requests: Mutex<Vec<OpenAiWireRequest>>,
}

#[async_trait::async_trait]
impl OpenAiTransport for RecordingTransport {
    async fn execute(
        &self,
        request: OpenAiWireRequest,
    ) -> Result<OpenAiWireResponse, aifluxon_core::ProviderError> {
        self.requests.lock().unwrap().push(request);
        let mut responses = self.responses.lock().unwrap();
        if responses.is_empty() {
            return Err(aifluxon_core::ProviderError::message(
                "Recording transport has no remaining responses.",
            ));
        }
        Ok(responses.remove(0))
    }
}

fn request_with(model: &str, features: aifluxon_core::ProviderFeatureRequest) -> ModelTurnRequest {
    ModelTurnRequest {
        model: model.to_string(),
        messages: vec![Message {
            role: MessageRole::User,
            content: vec![ContentPart::Text("hello".to_string())],
            tool_calls: Vec::new(),
            tool_call_id: None,
            provider_state: None,
        }],
        tools: vec![aifluxon_core::ToolDescriptor {
            name: "read_file".to_string(),
            description: "read".to_string(),
            input_schema: json!({ "type": "object" }),
            effect: aifluxon_core::ToolEffect::PureRead,
            required_capabilities: Vec::new(),
            parallel_safe: true,
        }],
        session_key: ProviderSessionKey::from_cache_session("session-a"),
        run_id: RunId::new(),
        opaque_state: None,
        features,
    }
}

fn config(family: ApiFamily, mode: OpenAiApiMode) -> OpenAiCompatibleConfig {
    let mut config = OpenAiCompatibleConfig::new(
        match family {
            ApiFamily::OpenAi => "openai",
            ApiFamily::DeepSeek => "deepseek",
            ApiFamily::Qwen => "qwen",
            ApiFamily::Kimi => "kimi",
            ApiFamily::Gemini => "gemini",
            ApiFamily::Codex => "codex",
            ApiFamily::Custom => "custom",
        },
        "https://provider.invalid/v1",
        "credential",
        mode,
        family == ApiFamily::Custom,
    );
    config.family = family;
    config
}

fn decorated_chat(family: ApiFamily, request: &ModelTurnRequest) -> Value {
    let config = config(family, OpenAiApiMode::ChatCompletions);
    let mut body = build_chat_completions_body(request);
    decorate::decorate_turn_body(&mut body, &config, OpenAiApiMode::ChatCompletions, request);
    body
}

fn decorated_responses(family: ApiFamily, request: &ModelTurnRequest) -> Value {
    let config = config(family, OpenAiApiMode::Responses);
    let mut body = build_responses_body(request);
    decorate::decorate_turn_body(&mut body, &config, OpenAiApiMode::Responses, request);
    body
}

fn add_image(request: &mut ModelTurnRequest, role: MessageRole, reference: &str, mime: &str) {
    request.messages.push(Message {
        role,
        content: vec![
            ContentPart::Text("inspect".to_string()),
            ContentPart::Image(ImageContent::new(reference, mime)),
        ],
        tool_calls: Vec::new(),
        tool_call_id: None,
        provider_state: (role == MessageRole::Tool).then(|| json!({ "call_id": "call-image" })),
    });
}

#[test]
fn deepseek_vision_serializes_chat_responses_and_tool_output_images() {
    let mut chat = request_with("deepseek-v4-flash-vision-exp", Default::default());
    add_image(
        &mut chat,
        MessageRole::User,
        "file-api-image-1",
        "image/png",
    );
    let chat_body = build_chat_completions_body(&chat);
    assert_eq!(chat_body["messages"][1]["content"][1]["type"], "file");
    assert_eq!(
        chat_body["messages"][1]["content"][1]["file_id"],
        "file-api-image-1"
    );

    let mut responses = request_with("deepseek-v4-flash-vision-exp", Default::default());
    add_image(
        &mut responses,
        MessageRole::User,
        "data:image/gif;base64,R0lGODlh",
        "image/gif",
    );
    add_image(
        &mut responses,
        MessageRole::Tool,
        "https://example.com/tool-output.webp",
        "image/webp",
    );
    let responses_body = build_responses_body(&responses);
    assert_eq!(
        responses_body["input"][1]["content"][1]["type"],
        "input_image"
    );
    assert_eq!(
        responses_body["input"][1]["content"][1]["image_url"],
        "data:image/gif;base64,R0lGODlh"
    );
    assert_eq!(responses_body["input"][2]["type"], "function_call_output");
    assert_eq!(
        responses_body["input"][2]["output"][1]["type"],
        "input_image"
    );
    assert_eq!(
        responses_body["input"][2]["output"][1]["image_url"],
        "https://example.com/tool-output.webp"
    );
}

#[test]
fn deepseek_vision_validation_rejects_wrong_models_roles_and_sources() {
    let config = config(ApiFamily::DeepSeek, OpenAiApiMode::Responses);
    let mut wrong_model = request_with("deepseek-v4-flash", Default::default());
    add_image(
        &mut wrong_model,
        MessageRole::User,
        "data:image/png;base64,iVBORw0KGgo=",
        "image/png",
    );
    assert!(
        validate_deepseek_image_request(&config, OpenAiApiMode::Responses, &wrong_model)
            .unwrap_err()
            .message
            .contains("does not support image input")
    );

    let mut wrong_role = request_with("deepseek-v4-flash-vision-exp", Default::default());
    add_image(
        &mut wrong_role,
        MessageRole::Assistant,
        "https://example.com/image.png",
        "image/png",
    );
    assert!(
        validate_deepseek_image_request(&config, OpenAiApiMode::Responses, &wrong_role)
            .unwrap_err()
            .message
            .contains("only in user messages")
    );

    let mut local_path = request_with("deepseek-v4-flash-vision-exp", Default::default());
    add_image(
        &mut local_path,
        MessageRole::User,
        "C:\\images\\plot.png",
        "image/png",
    );
    assert!(
        validate_deepseek_image_request(&config, OpenAiApiMode::Responses, &local_path)
            .unwrap_err()
            .message
            .contains("must be supported base64 data URLs")
    );
}

#[test]
fn openai_responses_required_models_do_not_stay_on_chat() {
    let config = config(ApiFamily::OpenAi, OpenAiApiMode::ChatCompletions);
    assert_eq!(
        decorate::effective_api_mode(&config, "gpt-5.4-codex"),
        OpenAiApiMode::Responses
    );
    assert_eq!(
        decorate::effective_api_mode(&config, "gpt-4.1"),
        OpenAiApiMode::ChatCompletions
    );
}

#[test]
fn openai_prompt_cache_key_is_copied_from_features() {
    let mut features = aifluxon_core::ProviderFeatureRequest::default();
    features.prompt_cache_key = Some("easyphy-cache".to_string());
    features.reasoning_effort = Some("high".to_string());
    let body = decorated_chat(ApiFamily::OpenAi, &request_with("gpt-5.1", features));
    assert_eq!(body["prompt_cache_key"], "easyphy-cache");
    assert_eq!(body["reasoning_effort"], "high");
}

#[test]
fn deepseek_chat_thinking_and_tool_choice_stay_family_specific() {
    let mut features = aifluxon_core::ProviderFeatureRequest::default();
    features.thinking_mode = Some("enabled".to_string());
    features.reasoning_effort = Some("low".to_string());
    let body = decorated_chat(
        ApiFamily::DeepSeek,
        &request_with("deepseek-v4-flash", features),
    );
    assert_eq!(body["thinking"]["type"], "enabled");
    assert_eq!(body["reasoning_effort"], "low");
    assert!(body.get("tool_choice").is_none());
}

#[test]
fn deepseek_v4_flash_keeps_responses_and_native_search_shape() {
    let mut features = aifluxon_core::ProviderFeatureRequest::default();
    features.web_search = true;
    features.thinking_mode = Some("enabled".to_string());
    let body = decorated_responses(
        ApiFamily::DeepSeek,
        &request_with("deepseek-v4-flash", features),
    );
    assert!(body.get("store").is_none());
    assert_eq!(body["reasoning"]["effort"], "high");
    assert_eq!(body["tools"][1], json!({ "type": "web_search" }));
}

#[test]
fn deepseek_v4_pro_keeps_requested_responses_and_maps_low_effort() {
    let config = config(ApiFamily::DeepSeek, OpenAiApiMode::Responses);
    assert_eq!(
        decorate::effective_api_mode(&config, "deepseek-v4-pro"),
        OpenAiApiMode::Responses
    );
    assert_eq!(
        decorate::effective_api_mode(&config, "deepseek-chat"),
        OpenAiApiMode::ChatCompletions
    );
    let mut features = aifluxon_core::ProviderFeatureRequest::default();
    features.thinking_mode = Some("enabled".to_string());
    features.reasoning_effort = Some("low".to_string());
    let body = decorated_responses(
        ApiFamily::DeepSeek,
        &request_with("deepseek-v4-pro", features),
    );
    assert_eq!(body["reasoning"]["effort"], "high");
}

#[test]
fn qwen_chat_thinking_budget_and_explicit_cache_markers_are_applied() {
    let mut features = aifluxon_core::ProviderFeatureRequest::default();
    features.thinking_mode = Some("enabled".to_string());
    features.thinking_budget = Some("8192".to_string());
    features.explicit_cache = true;
    let mut request = request_with("qwen3-max", features);
    request.messages[0].content = vec![ContentPart::Text("x".repeat(5000))];
    let body = decorated_chat(ApiFamily::Qwen, &request);
    assert_eq!(body["enable_thinking"], true);
    assert_eq!(body["thinking_budget"], 8192);
    assert_eq!(
        body["messages"][0]["content"][0]["cache_control"]["type"],
        "ephemeral"
    );
}

#[test]
fn deepseek_replays_reasoning_content_on_assistant_history() {
    let mut request = request_with("deepseek-chat", Default::default());
    let call_id = aifluxon_core::ToolInvocationId::from_stable_key("call-1");
    request.messages.insert(
        0,
        Message {
            role: MessageRole::Assistant,
            content: Vec::new(),
            tool_calls: vec![aifluxon_core::ToolCall {
                id: call_id,
                name: "read_file".to_string(),
                arguments: json!({ "path": "src/lib.rs" }),
                provider_call_id: Some("call-1".to_string()),
            }],
            tool_call_id: None,
            provider_state: Some(json!({
                "protocol": "chat_completions",
                "reasoning_content": "I should inspect the file first.",
            })),
        },
    );
    let body = decorated_chat(ApiFamily::DeepSeek, &request);
    assert_eq!(
        body["messages"][0]["reasoning_content"],
        "I should inspect the file first."
    );
    assert_eq!(body["messages"][0]["role"], "assistant");
    assert_eq!(body["messages"][0]["tool_calls"][0]["id"], "call-1");
}

#[test]
fn openai_does_not_replay_reasoning_content_on_assistant_history() {
    let mut request = request_with("gpt-4.1", Default::default());
    request.messages.insert(
        0,
        Message {
            role: MessageRole::Assistant,
            content: vec![ContentPart::Text("ok".to_string())],
            tool_calls: Vec::new(),
            tool_call_id: None,
            provider_state: Some(json!({
                "reasoning_content": "should stay local",
            })),
        },
    );
    let body = decorated_chat(ApiFamily::OpenAi, &request);
    assert!(body["messages"][0].get("reasoning_content").is_none());
}

#[test]
fn kimi_session_cache_and_thinking_stay_on_the_kimi_family() {
    let mut features = aifluxon_core::ProviderFeatureRequest::default();
    features.prompt_cache_key = Some("easyphy-kimi-stable".to_string());
    let body = decorated_chat(ApiFamily::Kimi, &request_with("kimi-k2.6", features));
    assert_eq!(body["prompt_cache_key"], "easyphy-kimi-stable");
    assert_eq!(body["thinking"]["type"], "enabled");
    assert_eq!(body["thinking"]["keep"], "all");
    assert_eq!(body["max_completion_tokens"], 32768);
    assert!(body.get("parallel_tool_calls").is_none());
}

#[test]
fn gemini_chat_normalizes_reasoning_effort() {
    let mut features = aifluxon_core::ProviderFeatureRequest::default();
    features.reasoning_effort = Some("xhigh".to_string());
    let body = decorated_chat(
        ApiFamily::Gemini,
        &request_with("gemini-2.5-flash", features),
    );
    assert_eq!(body["reasoning_effort"], "high");
}

#[test]
fn codex_responses_contract_and_hosted_search_are_preserved() {
    let mut features = aifluxon_core::ProviderFeatureRequest::default();
    features.reasoning_effort = Some("medium".to_string());
    features.web_search = true;
    features.image_generation = true;
    let body = decorated_responses(
        ApiFamily::Codex,
        &request_with("codex-mini-latest", features),
    );
    assert_eq!(body["include"][0], "reasoning.encrypted_content");
    assert_eq!(body["text"]["verbosity"], "medium");
    assert!(body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| tool.get("type").and_then(Value::as_str) == Some("web_search")));
    assert!(body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| tool.get("type").and_then(Value::as_str) == Some("image_generation")));
}

#[tokio::test]
async fn deepseek_tool_mode_retry_stays_inside_one_logical_turn() {
    let transport = Arc::new(RecordingTransport {
        responses: Mutex::new(vec![
            OpenAiWireResponse {
                status: 400,
                content_type: Some("application/json".to_string()),
                chunks: vec![br#"{"error":"tool mode"}"#.to_vec()],
            },
            OpenAiWireResponse {
                status: 200,
                content_type: Some("application/json".to_string()),
                chunks: vec![
                    br#"{"choices":[{"message":{"content":"fallback"},"finish_reason":"stop"}]}"#
                        .to_vec(),
                ],
            },
        ]),
        requests: Mutex::new(Vec::new()),
    });
    let mut config = config(ApiFamily::DeepSeek, OpenAiApiMode::ChatCompletions);
    config.retry_without_tools_on_tool_mode_error = true;
    let provider = OpenAiCompatibleProvider::with_transport(config, transport.clone());
    let turn = provider
        .next_turn(
            request_with("deepseek-chat", Default::default()),
            Arc::new(NoopModelEventSink),
        )
        .await
        .unwrap();
    assert_eq!(turn.text, "fallback");
    let sent = transport.requests.lock().unwrap();
    assert_eq!(sent.len(), 2);
    assert!(sent[0].body.get("tools").is_some());
    assert!(sent[1].body.get("tools").is_none());
}

#[test]
fn custom_family_is_not_flattened_to_openai_official_defaults() {
    let mut features = aifluxon_core::ProviderFeatureRequest::default();
    features.prompt_cache_key = Some("portable".to_string());
    let body = decorated_chat(ApiFamily::Custom, &request_with("custom-model", features));
    assert_eq!(body["prompt_cache_key"], "portable");
    assert_eq!(
        OpenAiCompatibleConfig::new(
            ProviderId::new("third_party_gateway"),
            "https://gateway.invalid/v1",
            "key",
            OpenAiApiMode::Responses,
            true,
        )
        .family,
        ApiFamily::Custom
    );
}

fn chat_sse(content: &str) -> OpenAiWireResponse {
    OpenAiWireResponse {
        status: 200,
        content_type: Some("text/event-stream".to_string()),
        chunks: vec![format!(
            "data: {{\"choices\":[{{\"delta\":{{\"content\":{content}}},\"finish_reason\":\"stop\"}}]}}\n\ndata: [DONE]\n\n",
            content = serde_json::to_string(content).unwrap()
        )
        .into_bytes()],
    }
}

async fn next_turn_for(
    family: ApiFamily,
    mode: OpenAiApiMode,
    response: OpenAiWireResponse,
    request: ModelTurnRequest,
) -> aifluxon_core::ModelTurn {
    let transport = Arc::new(RecordingTransport {
        responses: Mutex::new(vec![response]),
        requests: Mutex::new(Vec::new()),
    });
    let provider = OpenAiCompatibleProvider::with_transport(config(family, mode), transport);
    provider
        .next_turn(request, Arc::new(NoopModelEventSink))
        .await
        .unwrap()
}

#[tokio::test]
async fn qwen_summary_only_turn_requests_continuation() {
    let turn = next_turn_for(
        ApiFamily::Qwen,
        OpenAiApiMode::ChatCompletions,
        chat_sse("<summary>internal recap</summary>"),
        request_with("qwen3-max", Default::default()),
    )
    .await;
    assert_eq!(
        turn.terminal,
        ProviderTerminal::Continue(ContinuationReason::SummaryOnly)
    );
    assert_eq!(turn.opaque["hidden_context"], "internal recap");
    assert!(turn.text.trim().is_empty());
}

#[tokio::test]
async fn qwen_normal_final_answer_does_not_continue() {
    let turn = next_turn_for(
        ApiFamily::Qwen,
        OpenAiApiMode::ChatCompletions,
        chat_sse("已完成，开发服务器已启动。"),
        request_with("qwen3-max", Default::default()),
    )
    .await;
    assert_eq!(turn.terminal, ProviderTerminal::Stop);
    assert_eq!(turn.text, "已完成，开发服务器已启动。");
}

#[tokio::test]
async fn openai_incomplete_no_tool_turn_continues_when_tools_are_enabled() {
    let turn = next_turn_for(
        ApiFamily::OpenAi,
        OpenAiApiMode::ChatCompletions,
        chat_sse("I'll inspect the file with rg first."),
        request_with("gpt-4.1", Default::default()),
    )
    .await;
    assert_eq!(
        turn.terminal,
        ProviderTerminal::Continue(ContinuationReason::Incomplete)
    );
}

#[tokio::test]
async fn openai_normal_answer_without_tools_does_not_continue() {
    let turn = next_turn_for(
        ApiFamily::OpenAi,
        OpenAiApiMode::ChatCompletions,
        chat_sse("The answer is 42."),
        request_with("gpt-4.1", Default::default()),
    )
    .await;
    assert_eq!(turn.terminal, ProviderTerminal::Stop);
}

#[tokio::test]
async fn deepseek_promised_tool_text_does_not_continue() {
    let turn = next_turn_for(
        ApiFamily::DeepSeek,
        OpenAiApiMode::ChatCompletions,
        chat_sse("我先查看当前文件。"),
        request_with("deepseek-chat", Default::default()),
    )
    .await;
    assert_eq!(turn.terminal, ProviderTerminal::Stop);
}

#[tokio::test]
async fn codex_non_terminal_end_turn_requests_continuation() {
    let response = OpenAiWireResponse {
        status: 200,
        content_type: Some("text/event-stream".to_string()),
        chunks: vec![concat!(
            "data: {\"type\":\"response.output_text.delta\",\"sequence_number\":1,\"delta\":\"partial\"}\n\n",
            "data: {\"type\":\"response.completed\",\"sequence_number\":2,\"response\":{\"end_turn\":false}}\n\n"
        )
        .as_bytes()
        .to_vec()],
    };
    let turn = next_turn_for(
        ApiFamily::Codex,
        OpenAiApiMode::Responses,
        response,
        request_with("codex-mini-latest", Default::default()),
    )
    .await;
    assert_eq!(
        turn.terminal,
        ProviderTerminal::Continue(ContinuationReason::ProviderRequested)
    );
    assert_eq!(turn.opaque["end_turn"], false);
}

#[tokio::test]
async fn codex_terminal_end_turn_stops() {
    let response = OpenAiWireResponse {
        status: 200,
        content_type: Some("text/event-stream".to_string()),
        chunks: vec![concat!(
            "data: {\"type\":\"response.output_text.delta\",\"sequence_number\":1,\"delta\":\"done\"}\n\n",
            "data: {\"type\":\"response.completed\",\"sequence_number\":2,\"response\":{\"end_turn\":true}}\n\n"
        )
        .as_bytes()
        .to_vec()],
    };
    let turn = next_turn_for(
        ApiFamily::Codex,
        OpenAiApiMode::Responses,
        response,
        request_with("codex-mini-latest", Default::default()),
    )
    .await;
    assert_eq!(turn.terminal, ProviderTerminal::Stop);
}

#[test]
fn responses_input_replays_provider_state_items() {
    let mut request = request_with("codex-mini-latest", Default::default());
    request.messages.push(Message {
        role: MessageRole::Assistant,
        content: Vec::new(),
        tool_calls: Vec::new(),
        tool_call_id: None,
        provider_state: Some(json!({
            "response_items": [{ "type": "reasoning", "id": "rs-1" }]
        })),
    });
    let body = build_responses_body(&request);
    assert_eq!(body["input"][1]["type"], "reasoning");
    assert_eq!(body["input"][1]["id"], "rs-1");
}

#[test]
fn chat_input_preserves_provider_call_id_for_matching_tool_results() {
    let call_id = aifluxon_core::ToolInvocationId::from_stable_key("call_upstream_1");
    let mut request = request_with("custom-model", Default::default());
    request.messages.push(Message {
        role: MessageRole::Assistant,
        content: Vec::new(),
        tool_calls: vec![aifluxon_core::ToolCall {
            id: call_id,
            name: "read_file".to_string(),
            arguments: json!({ "path": "src/lib.rs" }),
            provider_call_id: Some("call_upstream_1".to_string()),
        }],
        tool_call_id: None,
        provider_state: None,
    });
    request.messages.push(Message {
        role: MessageRole::Tool,
        content: vec![ContentPart::Text("ok".to_string())],
        tool_calls: Vec::new(),
        tool_call_id: Some(call_id),
        provider_state: Some(json!({ "call_id": "call_upstream_1" })),
    });

    let body = build_chat_completions_body(&request);
    let messages = body["messages"].as_array().expect("chat messages");
    assert_eq!(messages[1]["tool_calls"][0]["id"], "call_upstream_1");
    assert_eq!(messages[2]["tool_call_id"], "call_upstream_1");
}

#[test]
fn responses_input_maps_tool_messages_to_function_call_items() {
    let call_id = aifluxon_core::ToolInvocationId::from_stable_key("call_shell");
    let mut request = request_with("deepseek-v4-pro", Default::default());
    request.messages.push(Message {
        role: MessageRole::Assistant,
        content: Vec::new(),
        tool_calls: vec![aifluxon_core::ToolCall {
            id: call_id,
            name: "shell".to_string(),
            arguments: json!({ "command": "rg --files" }),
            provider_call_id: Some("call_shell".to_string()),
        }],
        tool_call_id: None,
        provider_state: Some(json!({
            "protocol": "responses",
            "response_items": [{
                "type": "reasoning",
                "id": "rs_1",
                "content": [{ "type": "reasoning_text", "text": "inspect first" }]
            }],
            "terminal_output": [
                {
                    "type": "reasoning",
                    "id": "rs_1",
                    "content": [{ "type": "reasoning_text", "text": "inspect first" }]
                },
                {
                    "type": "function_call",
                    "call_id": "call_shell",
                    "name": "shell",
                    "arguments": "{\"command\":\"rg --files\"}"
                }
            ]
        })),
    });
    request.messages.push(Message {
        role: MessageRole::Tool,
        content: vec![ContentPart::Text("src/main.ts".to_string())],
        tool_calls: Vec::new(),
        tool_call_id: Some(call_id),
        provider_state: Some(json!({ "call_id": "call_shell" })),
    });

    let body = build_responses_body(&request);
    let input = body["input"].as_array().expect("responses input");
    assert!(input.iter().all(|item| {
        item.get("role").and_then(Value::as_str) != Some("tool") && item.get("tool_calls").is_none()
    }));
    assert_eq!(input[1]["type"], "reasoning");
    assert_eq!(input[2]["type"], "function_call");
    assert_eq!(input[2]["call_id"], "call_shell");
    assert_eq!(input[2]["name"], "shell");
    assert_eq!(input[3]["type"], "function_call_output");
    assert_eq!(input[3]["call_id"], "call_shell");
    assert_eq!(input[3]["output"], "src/main.ts");
}

#[test]
fn responses_input_reconstructs_function_call_items_without_terminal_output() {
    let call_id = aifluxon_core::ToolInvocationId::from_stable_key("call_read");
    let mut request = request_with("deepseek-v4-flash", Default::default());
    request.messages.push(Message {
        role: MessageRole::Assistant,
        content: vec![ContentPart::Text("checking".to_string())],
        tool_calls: vec![aifluxon_core::ToolCall {
            id: call_id,
            name: "read_file".to_string(),
            arguments: json!({ "path": "src/lib.rs" }),
            provider_call_id: Some("call_read".to_string()),
        }],
        tool_call_id: None,
        provider_state: None,
    });
    request.messages.push(Message {
        role: MessageRole::Tool,
        content: vec![ContentPart::Text("ok".to_string())],
        tool_calls: Vec::new(),
        tool_call_id: Some(call_id),
        provider_state: None,
    });

    let body = build_responses_body(&request);
    let input = body["input"].as_array().expect("responses input");
    assert_eq!(input[1]["role"], "assistant");
    assert_eq!(input[1]["content"], "checking");
    assert_eq!(input[2]["type"], "function_call");
    assert_eq!(input[2]["call_id"], "call_read");
    assert_eq!(input[3]["type"], "function_call_output");
    assert_eq!(input[3]["call_id"], "call_read");
    assert_eq!(input[3]["output"], "ok");
}

#[test]
fn codex_responses_input_preserves_encrypted_reasoning_and_rebuilds_tools() {
    let call_id = aifluxon_core::ToolInvocationId::from_stable_key("call_1");
    let mut request = request_with("codex-mini-latest", Default::default());
    request.messages.insert(
        0,
        Message {
            role: MessageRole::System,
            content: vec![ContentPart::Text("Stable Codex instructions".to_string())],
            tool_calls: Vec::new(),
            tool_call_id: None,
            provider_state: None,
        },
    );
    request.messages.push(Message {
        role: MessageRole::Assistant,
        content: Vec::new(),
        tool_calls: vec![aifluxon_core::ToolCall {
            id: call_id,
            name: "shell".to_string(),
            arguments: json!({ "command": "rg --files" }),
            provider_call_id: Some("call_1".to_string()),
        }],
        tool_call_id: None,
        provider_state: Some(json!({
            "protocol": "responses",
            "response_items": [{
                "type": "reasoning",
                "id": "rs_codex",
                "encrypted_content": "encrypted-blob",
                "summary": []
            }],
            "terminal_output": [
                {
                    "type": "reasoning",
                    "id": "rs_codex",
                    "encrypted_content": "encrypted-blob",
                    "summary": []
                },
                {
                    "type": "message",
                    "id": "msg_should_not_replace_replay",
                    "role": "assistant",
                    "content": [{ "type": "output_text", "text": "commentary" }]
                },
                {
                    "type": "function_call",
                    "id": "fc_1",
                    "call_id": "call_1",
                    "name": "shell",
                    "arguments": "{\"command\":\"rg --files\"}"
                }
            ]
        })),
    });
    request.messages.push(Message {
        role: MessageRole::Tool,
        content: vec![ContentPart::Text("src/main.ts".to_string())],
        tool_calls: Vec::new(),
        tool_call_id: Some(call_id),
        provider_state: Some(json!({ "call_id": "call_1" })),
    });

    let body = decorated_responses(ApiFamily::Codex, &request);
    let input = body["input"].as_array().expect("responses input");
    assert_eq!(body["instructions"], "Stable Codex instructions");
    assert_eq!(body["include"][0], "reasoning.encrypted_content");
    assert_eq!(body["text"]["verbosity"], "medium");
    assert!(input.iter().all(|item| {
        item.get("role").and_then(Value::as_str) != Some("tool") && item.get("tool_calls").is_none()
    }));
    assert_eq!(input[0]["role"], "user");
    assert_eq!(input[1]["type"], "reasoning");
    assert_eq!(input[1]["encrypted_content"], "encrypted-blob");
    assert!(input.iter().all(|item| {
        item.get("id").and_then(Value::as_str) != Some("msg_should_not_replace_replay")
    }));
    assert!(!input.iter().any(|item| {
        item.get("type").and_then(Value::as_str) == Some("reasoning")
            && item.pointer("/content/0/type") == Some(&json!("reasoning_text"))
    }));
    assert_eq!(input[2]["type"], "function_call");
    assert_eq!(input[2]["call_id"], "call_1");
    assert!(input[2].get("id").is_none());
    assert_eq!(input[3]["type"], "function_call_output");
    assert_eq!(input[3]["call_id"], "call_1");
    assert_eq!(input[3]["output"], "src/main.ts");
}
