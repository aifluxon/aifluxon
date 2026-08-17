mod chat_completions;
mod continuation;
mod decorate;
#[cfg(test)]
mod family_tests;
mod responses;
mod streaming;
pub mod tools;
#[cfg(test)]
mod turn_engine_tests;

use aifluxon_auth::{CredentialSource, StaticBearerCredential};
use aifluxon_core::{
    ModelEventSink, ModelProvider, ModelTurn, ModelTurnRequest, ProviderCapabilities,
    ProviderError, ProviderId,
};
use futures_util::{Stream, StreamExt};
use std::pin::Pin;
use std::sync::Arc;
use streaming::LiveStreamDecoder;

pub use chat_completions::{build_chat_completions_body, ChatCompletionsTurnAssembler};
pub use decorate::effective_api_mode;
pub use responses::{build_responses_body, ResponsesTurnAssembler};
pub use streaming::{
    decode_chat_response, decode_chat_response_with, decode_responses_response,
    decode_responses_response_with,
};
pub use tools::descriptor_to_openai_tool;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpenAiApiMode {
    ChatCompletions,
    Responses,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApiFamily {
    OpenAi,
    DeepSeek,
    Qwen,
    Kimi,
    Gemini,
    Codex,
    Custom,
}

impl ApiFamily {
    pub fn from_provider_id(id: &str) -> Self {
        match id.trim().to_ascii_lowercase().as_str() {
            "openai" => Self::OpenAi,
            "deepseek" => Self::DeepSeek,
            "qwen" => Self::Qwen,
            "kimi" => Self::Kimi,
            "gemini" => Self::Gemini,
            "codex" => Self::Codex,
            _ => Self::Custom,
        }
    }
}

#[derive(Clone)]
pub struct OpenAiCompatibleConfig {
    pub provider_id: ProviderId,
    pub base_url: String,
    pub credential_source: Arc<dyn CredentialSource>,
    pub api_mode: OpenAiApiMode,
    pub allow_cumulative_delta: bool,
    pub family: ApiFamily,
    pub retry_without_tools_on_tool_mode_error: bool,
    pub send_tool_choice_auto: bool,
    pub parallel_tool_calls: bool,
    pub include_stream_usage: bool,
    pub chatgpt_account_id: Option<String>,
}

impl std::fmt::Debug for OpenAiCompatibleConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiCompatibleConfig")
            .field("provider_id", &self.provider_id)
            .field("base_url", &self.base_url)
            .field("credential_source", &"CredentialSource([redacted])")
            .field("api_mode", &self.api_mode)
            .field("allow_cumulative_delta", &self.allow_cumulative_delta)
            .field("family", &self.family)
            .field(
                "retry_without_tools_on_tool_mode_error",
                &self.retry_without_tools_on_tool_mode_error,
            )
            .field("send_tool_choice_auto", &self.send_tool_choice_auto)
            .field("parallel_tool_calls", &self.parallel_tool_calls)
            .field("include_stream_usage", &self.include_stream_usage)
            .field("chatgpt_account_id", &self.chatgpt_account_id)
            .finish()
    }
}

impl OpenAiCompatibleConfig {
    pub fn new(
        provider_id: impl Into<ProviderId>,
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        api_mode: OpenAiApiMode,
        allow_cumulative_delta: bool,
    ) -> Self {
        Self::with_credential(
            provider_id,
            base_url,
            Arc::new(StaticBearerCredential::new(api_key.into())),
            api_mode,
            allow_cumulative_delta,
        )
    }

    pub fn with_credential(
        provider_id: impl Into<ProviderId>,
        base_url: impl Into<String>,
        credential_source: Arc<dyn CredentialSource>,
        api_mode: OpenAiApiMode,
        allow_cumulative_delta: bool,
    ) -> Self {
        let provider_id = provider_id.into();
        let family = ApiFamily::from_provider_id(provider_id.as_str());
        Self {
            family,
            send_tool_choice_auto: family != ApiFamily::DeepSeek,
            parallel_tool_calls: !matches!(family, ApiFamily::DeepSeek | ApiFamily::Gemini),
            include_stream_usage: true,
            retry_without_tools_on_tool_mode_error: false,
            provider_id,
            base_url: base_url.into(),
            credential_source,
            api_mode,
            allow_cumulative_delta,
            chatgpt_account_id: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenAiWireRequest {
    pub url: String,
    pub api_key: String,
    pub extra_headers: Vec<(String, String)>,
    pub body: serde_json::Value,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenAiWireResponse {
    pub status: u16,
    pub content_type: Option<String>,
    pub chunks: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenAiStreamHead {
    pub status: u16,
    pub content_type: Option<String>,
}

pub type OpenAiBodyStream = Pin<Box<dyn Stream<Item = Result<Vec<u8>, ProviderError>> + Send>>;

#[async_trait::async_trait]
pub trait OpenAiTransport: Send + Sync {
    async fn execute(
        &self,
        request: OpenAiWireRequest,
    ) -> Result<OpenAiWireResponse, ProviderError>;

    async fn stream(
        &self,
        request: OpenAiWireRequest,
    ) -> Result<(OpenAiStreamHead, OpenAiBodyStream), ProviderError> {
        Ok(buffered_body_stream(self.execute(request).await?))
    }
}

fn buffered_body_stream(response: OpenAiWireResponse) -> (OpenAiStreamHead, OpenAiBodyStream) {
    let head = OpenAiStreamHead {
        status: response.status,
        content_type: response.content_type,
    };
    let body = Box::pin(futures_util::stream::iter(
        response.chunks.into_iter().map(Ok),
    ));
    (head, body)
}

async fn collect_openai_stream(
    (head, mut body): (OpenAiStreamHead, OpenAiBodyStream),
) -> Result<OpenAiWireResponse, ProviderError> {
    let mut chunks = Vec::new();
    while let Some(chunk) = body.next().await {
        chunks.push(chunk?);
    }
    Ok(OpenAiWireResponse {
        status: head.status,
        content_type: head.content_type,
        chunks,
    })
}

async fn send_openai_reqwest(
    request: &OpenAiWireRequest,
) -> Result<reqwest::Response, ProviderError> {
    let client = crate::common::build_http_client(crate::common::HttpClientTuning::default())
        .map_err(ProviderError::message)?;
    let mut builder = client
        .post(&request.url)
        .bearer_auth(&request.api_key)
        .header(reqwest::header::ACCEPT_ENCODING, "identity");
    for (name, value) in &request.extra_headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    builder.json(&request.body).send().await.map_err(|error| {
        ProviderError::message(crate::common::sanitize_provider_error(
            format!("Provider request could not be sent: {error}"),
            &[&request.api_key],
        ))
    })
}

#[derive(Default)]
pub struct ReqwestOpenAiTransport;

#[async_trait::async_trait]
impl OpenAiTransport for ReqwestOpenAiTransport {
    async fn execute(
        &self,
        request: OpenAiWireRequest,
    ) -> Result<OpenAiWireResponse, ProviderError> {
        collect_openai_stream(self.stream(request).await?).await
    }

    async fn stream(
        &self,
        request: OpenAiWireRequest,
    ) -> Result<(OpenAiStreamHead, OpenAiBodyStream), ProviderError> {
        let response = send_openai_reqwest(&request).await?;
        let head = OpenAiStreamHead {
            status: response.status().as_u16(),
            content_type: response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .map(str::to_string),
        };
        let api_key = request.api_key.clone();
        let body = response.bytes_stream().map(move |chunk| {
            chunk.map(|bytes| bytes.to_vec()).map_err(|error| {
                ProviderError::message(crate::common::sanitize_provider_error(
                    format!("Provider response stream failed: {error}"),
                    &[&api_key],
                ))
            })
        });
        Ok((head, Box::pin(body)))
    }
}

pub struct OpenAiCompatibleProvider {
    config: OpenAiCompatibleConfig,
    transport: Arc<dyn OpenAiTransport>,
}

impl OpenAiCompatibleProvider {
    /// Compatibility constructor for registry/boundary tests. A configured transport is required
    /// before `next_turn`; no implicit endpoint or credential is invented.
    pub fn new(provider: impl Into<String>) -> Self {
        Self {
            config: OpenAiCompatibleConfig::new(
                ProviderId::new(provider.into()),
                String::new(),
                String::new(),
                OpenAiApiMode::ChatCompletions,
                false,
            ),
            transport: Arc::new(ReqwestOpenAiTransport),
        }
    }

    pub fn configured(config: OpenAiCompatibleConfig) -> Self {
        Self {
            config,
            transport: Arc::new(ReqwestOpenAiTransport),
        }
    }

    pub fn with_transport(
        config: OpenAiCompatibleConfig,
        transport: Arc<dyn OpenAiTransport>,
    ) -> Self {
        Self { config, transport }
    }

    fn endpoint_for(&self, mode: OpenAiApiMode) -> Result<String, ProviderError> {
        let base = self.config.base_url.trim().trim_end_matches('/');
        if base.is_empty() {
            return Err(ProviderError::message(
                "OpenAI-compatible provider requires an explicit base URL.",
            ));
        }
        Ok(format!(
            "{base}/{}",
            match mode {
                OpenAiApiMode::ChatCompletions => "chat/completions",
                OpenAiApiMode::Responses => "responses",
            }
        ))
    }

    async fn resolve_bearer(&self, force: bool) -> Result<String, ProviderError> {
        let credential = if force {
            self.config.credential_source.force_refresh().await
        } else {
            self.config.credential_source.bearer().await
        };
        credential
            .map(|bearer| bearer.token().to_string())
            .map_err(|error| ProviderError::message(error.to_string()))
    }

    fn wire_request(
        &self,
        mode: OpenAiApiMode,
        body: serde_json::Value,
        token: &str,
        session_key: &str,
        turn_state: Option<&str>,
    ) -> Result<OpenAiWireRequest, ProviderError> {
        Ok(OpenAiWireRequest {
            url: self.endpoint_for(mode)?,
            api_key: token.to_string(),
            extra_headers: crate::codex::oauth_request_headers(
                self.config.chatgpt_account_id.as_deref(),
                session_key,
                turn_state,
            ),
            body,
        })
    }

    pub async fn send_http(
        &self,
        body: serde_json::Value,
        session_key: Option<&str>,
        turn_state: Option<&str>,
    ) -> Result<reqwest::Response, ProviderError> {
        let mode = decorate::effective_api_mode(
            &self.config,
            body.get("model")
                .and_then(|value| value.as_str())
                .unwrap_or(""),
        );
        let mut token = self.resolve_bearer(false).await?;
        let mut retried = false;
        loop {
            let request = self.wire_request(
                mode,
                body.clone(),
                &token,
                session_key.unwrap_or(""),
                turn_state,
            )?;
            let response = send_openai_reqwest(&request).await?;
            if response.status().as_u16() == 401
                && !retried
                && self.config.credential_source.supports_refresh()
            {
                token = self.resolve_bearer(true).await?;
                retried = true;
                continue;
            }
            return Ok(response);
        }
    }

    fn wrap_family_sink(
        &self,
        sink: Arc<dyn ModelEventSink>,
    ) -> (
        Arc<dyn ModelEventSink>,
        Option<Arc<crate::kimi::ThinkTagSink>>,
    ) {
        match self.config.family {
            ApiFamily::Qwen => (Arc::new(crate::qwen::SummaryFilterSink::new(sink)), None),
            ApiFamily::Kimi => {
                let think = Arc::new(crate::kimi::ThinkTagSink::new(sink));
                let sink: Arc<dyn ModelEventSink> = think.clone();
                (sink, Some(think))
            }
            _ => (sink, None),
        }
    }

    async fn stream_turn(
        &self,
        mode: OpenAiApiMode,
        body: serde_json::Value,
        token: &str,
        session_key: &str,
        turn_state: Option<&str>,
        sink: Arc<dyn ModelEventSink>,
    ) -> Result<(u16, String, Option<ModelTurn>), ProviderError> {
        let (head, mut chunks) = self
            .transport
            .stream(self.wire_request(mode, body, token, session_key, turn_state)?)
            .await?;
        if !(200..300).contains(&head.status) {
            let mut raw = Vec::new();
            while let Some(chunk) = chunks.next().await {
                raw.extend(chunk?);
            }
            return Ok((
                head.status,
                String::from_utf8_lossy(&raw).into_owned(),
                None,
            ));
        }
        let (sink, kimi_think) = self.wrap_family_sink(sink);
        let mut decoder =
            LiveStreamDecoder::new(mode, self.config.allow_cumulative_delta, head.content_type);
        while let Some(chunk) = chunks.next().await {
            let applied = decoder.push(&chunk?, sink.as_ref())?;
            if applied > 0 {
                tokio::task::yield_now().await;
            }
        }
        let mut turn = decoder.finish(sink.as_ref())?;
        if let Some(think) = kimi_think {
            think.flush();
            crate::kimi::apply_think_tags_to_turn(&mut turn);
        }
        if self.config.family == ApiFamily::Qwen {
            crate::qwen::apply_summary_to_turn(&mut turn);
        }
        Ok((head.status, String::new(), Some(turn)))
    }

    async fn stream_turn_with_auth(
        &self,
        mode: OpenAiApiMode,
        body: serde_json::Value,
        request: &ModelTurnRequest,
        sink: Arc<dyn ModelEventSink>,
    ) -> Result<(u16, String, Option<ModelTurn>, String), ProviderError> {
        let turn_state = request
            .opaque_state
            .as_ref()
            .and_then(|value| value.get("x-codex-turn-state"))
            .and_then(|value| value.as_str());
        let mut token = self.resolve_bearer(false).await?;
        let mut retried = false;
        loop {
            let streamed = self
                .stream_turn(
                    mode,
                    body.clone(),
                    &token,
                    request.session_key.as_str(),
                    turn_state,
                    sink.clone(),
                )
                .await?;
            if streamed.0 == 401 && !retried && self.config.credential_source.supports_refresh() {
                token = self.resolve_bearer(true).await?;
                retried = true;
                continue;
            }
            return Ok((streamed.0, streamed.1, streamed.2, token));
        }
    }
}

#[async_trait::async_trait]
impl ModelProvider for OpenAiCompatibleProvider {
    fn id(&self) -> &ProviderId {
        &self.config.provider_id
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::openai_compatible()
    }

    async fn next_turn(
        &self,
        mut request: ModelTurnRequest,
        sink: Arc<dyn ModelEventSink>,
    ) -> Result<ModelTurn, ProviderError> {
        let mode = decorate::effective_api_mode(&self.config, &request.model);
        let mut body = match mode {
            OpenAiApiMode::ChatCompletions => build_chat_completions_body(&request),
            OpenAiApiMode::Responses => build_responses_body(&request),
        };
        decorate::decorate_turn_body(&mut body, &self.config, mode, &request);
        let mut streamed = self
            .stream_turn_with_auth(mode, body, &request, sink.clone())
            .await?;
        if self.config.retry_without_tools_on_tool_mode_error
            && !request.tools.is_empty()
            && crate::deepseek::tool_mode_error_status(streamed.0)
        {
            request.tools.clear();
            request.messages.push(aifluxon_core::Message {
                role: aifluxon_core::MessageRole::System,
                content: vec![aifluxon_core::ContentPart::Text(
                    "Tool mode is unavailable for this DeepSeek request. Do not call tools. Answer based on the provided conversation and active file context only.".to_string(),
                )],
                tool_calls: Vec::new(),
                tool_call_id: None,
                provider_state: None,
            });
            let mut fallback = match mode {
                OpenAiApiMode::ChatCompletions => build_chat_completions_body(&request),
                OpenAiApiMode::Responses => build_responses_body(&request),
            };
            decorate::decorate_turn_body(&mut fallback, &self.config, mode, &request);
            streamed = self
                .stream_turn_with_auth(mode, fallback, &request, sink)
                .await?;
        }
        let (status, preview, turn, token) = streamed;
        if !(200..300).contains(&status) {
            return Err(ProviderError::message(
                crate::common::sanitize_provider_error(
                    format!("Provider returned HTTP {status}: {preview}"),
                    &[&token],
                ),
            ));
        }
        let mut turn = turn.ok_or_else(|| {
            ProviderError::message("Provider returned a success status without a model turn.")
        })?;
        continuation::apply_turn_continuation(
            self.config.family,
            !request.tools.is_empty(),
            &mut turn,
        );
        Ok(turn)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aifluxon_core::{ContentPart, Message, MessageRole, ProviderSessionKey, RunId};
    use std::sync::Mutex;

    struct FakeTransport {
        response: OpenAiWireResponse,
        request: Mutex<Option<OpenAiWireRequest>>,
    }

    #[async_trait::async_trait]
    impl OpenAiTransport for FakeTransport {
        async fn execute(
            &self,
            request: OpenAiWireRequest,
        ) -> Result<OpenAiWireResponse, ProviderError> {
            *self.request.lock().unwrap() = Some(request);
            Ok(self.response.clone())
        }
    }

    fn request() -> ModelTurnRequest {
        ModelTurnRequest {
            model: "model-1".to_string(),
            messages: vec![Message {
                role: MessageRole::User,
                content: vec![ContentPart::Text("hello".to_string())],
                tool_calls: Vec::new(),
                tool_call_id: None,
                provider_state: None,
            }],
            tools: Vec::new(),
            session_key: ProviderSessionKey::from_cache_session("session-a"),
            run_id: RunId::new(),
            opaque_state: None,
            features: Default::default(),
        }
    }

    #[tokio::test]
    async fn configured_provider_executes_chat_protocol_through_injected_transport() {
        let transport = Arc::new(FakeTransport {
            response: OpenAiWireResponse {
                status: 200,
                content_type: Some("application/json".to_string()),
                chunks: vec![br#"{"choices":[{"message":{"content":"done"},"finish_reason":"stop"}],"usage":{"total_tokens":2}}"#.to_vec()],
            },
            request: Mutex::new(None),
        });
        let provider = OpenAiCompatibleProvider::with_transport(
            OpenAiCompatibleConfig::new(
                "custom_gateway",
                "https://provider.invalid/v1",
                "credential",
                OpenAiApiMode::ChatCompletions,
                false,
            ),
            transport.clone(),
        );
        let turn = provider
            .next_turn(request(), Arc::new(aifluxon_core::NoopModelEventSink))
            .await
            .unwrap();
        assert_eq!(turn.text, "done");
        assert_eq!(provider.id().as_str(), "custom_gateway");
        let sent = transport.request.lock().unwrap().clone().unwrap();
        assert_eq!(sent.url, "https://provider.invalid/v1/chat/completions");
        assert_eq!(sent.body["model"], "model-1");
    }

    #[tokio::test]
    async fn next_turn_performs_exactly_one_remote_call_even_when_tools_are_returned() {
        let transport = Arc::new(FakeTransport {
            response: OpenAiWireResponse {
                status: 200,
                content_type: Some("text/event-stream".to_string()),
                chunks: vec![concat!(
                    "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_a\",\"function\":{\"name\":\"read_file\",\"arguments\":\"{\\\"path\\\":\\\"a.rs\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
                    "data: [DONE]\n\n"
                ).as_bytes().to_vec()],
            },
            request: Mutex::new(None),
        });
        let provider = OpenAiCompatibleProvider::with_transport(
            OpenAiCompatibleConfig::new(
                "openai",
                "https://provider.invalid/v1",
                "credential",
                OpenAiApiMode::ChatCompletions,
                false,
            ),
            transport.clone(),
        );
        let turn = provider
            .next_turn(request(), Arc::new(aifluxon_core::NoopModelEventSink))
            .await
            .unwrap();
        assert_eq!(turn.terminal, aifluxon_core::ProviderTerminal::ToolCalls);
        assert_eq!(turn.tool_calls.len(), 1);
        assert_eq!(turn.tool_calls[0].name, "read_file");
        assert!(transport.request.lock().unwrap().is_some());
    }

    struct DelayedChunkTransport {
        chunks: Vec<Vec<u8>>,
        delay: std::time::Duration,
    }

    #[async_trait::async_trait]
    impl OpenAiTransport for DelayedChunkTransport {
        async fn execute(
            &self,
            _request: OpenAiWireRequest,
        ) -> Result<OpenAiWireResponse, ProviderError> {
            Ok(OpenAiWireResponse {
                status: 200,
                content_type: Some("text/event-stream".to_string()),
                chunks: self.chunks.clone(),
            })
        }

        async fn stream(
            &self,
            _request: OpenAiWireRequest,
        ) -> Result<(OpenAiStreamHead, OpenAiBodyStream), ProviderError> {
            let chunks = self.chunks.clone();
            let delay = self.delay;
            let body = futures_util::stream::unfold(0usize, move |index| {
                let chunks = chunks.clone();
                async move {
                    if index >= chunks.len() {
                        return None;
                    }
                    if index > 0 {
                        tokio::time::sleep(delay).await;
                    }
                    Some((Ok(chunks[index].clone()), index + 1))
                }
            });
            Ok((
                OpenAiStreamHead {
                    status: 200,
                    content_type: Some("text/event-stream".to_string()),
                },
                Box::pin(body),
            ))
        }
    }

    #[derive(Default)]
    struct TimingSink {
        events: Mutex<Vec<(std::time::Instant, String)>>,
    }

    impl ModelEventSink for TimingSink {
        fn on_text_delta(&self, delta: &str) {
            self.events
                .lock()
                .unwrap()
                .push((std::time::Instant::now(), delta.to_string()));
        }
    }

    #[tokio::test]
    async fn next_turn_emits_text_deltas_before_later_http_chunks_arrive() {
        let first = concat!(r#"data: {"choices":[{"delta":{"content":"Hel"}}]}"#, "\n\n");
        let second = concat!(
            r#"data: {"choices":[{"delta":{"content":"lo"}}]}"#,
            "\n\n",
            "data: [DONE]\n\n"
        );
        let sink = Arc::new(TimingSink::default());
        let provider = OpenAiCompatibleProvider::with_transport(
            OpenAiCompatibleConfig::new(
                "openai",
                "https://provider.invalid/v1",
                "credential",
                OpenAiApiMode::ChatCompletions,
                false,
            ),
            Arc::new(DelayedChunkTransport {
                chunks: vec![first.as_bytes().to_vec(), second.as_bytes().to_vec()],
                delay: std::time::Duration::from_millis(40),
            }),
        );
        let turn = provider.next_turn(request(), sink.clone()).await.unwrap();
        assert_eq!(turn.text, "Hello");
        let events = sink.events.lock().unwrap().clone();
        assert_eq!(
            events
                .iter()
                .map(|(_, delta)| delta.as_str())
                .collect::<Vec<_>>(),
            vec!["Hel", "lo"]
        );
        assert!(events[1].0.duration_since(events[0].0) >= std::time::Duration::from_millis(20));
    }

    #[tokio::test]
    async fn next_turn_emits_reasoning_deltas_before_visible_text() {
        #[derive(Default)]
        struct OrderSink(Mutex<Vec<String>>);
        impl ModelEventSink for OrderSink {
            fn on_text_delta(&self, delta: &str) {
                self.0.lock().unwrap().push(format!("text:{delta}"));
            }
            fn on_reasoning_delta(&self, delta: &str) {
                self.0.lock().unwrap().push(format!("reason:{delta}"));
            }
        }
        let body = concat!(
            r#"data: {"choices":[{"delta":{"reasoning_content":"plan"}}]}"#,
            "\n\n",
            r#"data: {"choices":[{"delta":{"content":"answer"}}]}"#,
            "\n\n",
            "data: [DONE]\n\n"
        );
        let sink = Arc::new(OrderSink::default());
        let provider = OpenAiCompatibleProvider::with_transport(
            OpenAiCompatibleConfig::new(
                "openai",
                "https://provider.invalid/v1",
                "credential",
                OpenAiApiMode::ChatCompletions,
                false,
            ),
            Arc::new(FakeTransport {
                response: OpenAiWireResponse {
                    status: 200,
                    content_type: Some("text/event-stream".to_string()),
                    chunks: vec![body.as_bytes().to_vec()],
                },
                request: Mutex::new(None),
            }),
        );
        let turn = provider.next_turn(request(), sink.clone()).await.unwrap();
        assert_eq!(turn.reasoning, "plan");
        assert_eq!(turn.text, "answer");
        assert_eq!(
            *sink.0.lock().unwrap(),
            vec!["reason:plan".to_string(), "text:answer".to_string()]
        );
    }

    struct SequenceTransport {
        responses: Mutex<Vec<OpenAiWireResponse>>,
        requests: Mutex<Vec<OpenAiWireRequest>>,
    }

    #[async_trait::async_trait]
    impl OpenAiTransport for SequenceTransport {
        async fn execute(
            &self,
            request: OpenAiWireRequest,
        ) -> Result<OpenAiWireResponse, ProviderError> {
            self.requests.lock().unwrap().push(request);
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                return Err(ProviderError::message("no remaining responses"));
            }
            Ok(responses.remove(0))
        }
    }

    struct RotatingCredential {
        tokens: Mutex<Vec<String>>,
        refresh_count: std::sync::atomic::AtomicUsize,
        calls: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl aifluxon_auth::CredentialSource for RotatingCredential {
        async fn bearer(
            &self,
        ) -> Result<aifluxon_auth::BearerCredential, aifluxon_auth::AuthError> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let tokens = self.tokens.lock().unwrap();
            Ok(aifluxon_auth::BearerCredential::new(tokens[0].clone()))
        }

        async fn force_refresh(
            &self,
        ) -> Result<aifluxon_auth::BearerCredential, aifluxon_auth::AuthError> {
            self.refresh_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let mut tokens = self.tokens.lock().unwrap();
            if tokens.len() > 1 {
                tokens.remove(0);
            }
            Ok(aifluxon_auth::BearerCredential::new(tokens[0].clone()))
        }

        fn supports_refresh(&self) -> bool {
            true
        }
    }

    fn success_chat() -> OpenAiWireResponse {
        OpenAiWireResponse {
            status: 200,
            content_type: Some("application/json".to_string()),
            chunks: vec![br#"{"choices":[{"message":{"content":"done"},"finish_reason":"stop"}],"usage":{"total_tokens":2}}"#.to_vec()],
        }
    }

    fn success_responses() -> OpenAiWireResponse {
        OpenAiWireResponse {
            status: 200,
            content_type: Some("text/event-stream".to_string()),
            chunks: vec![concat!(
                r#"data: {"type":"response.output_text.delta","sequence_number":1,"delta":"done"}"#,
                "\n\n",
                r#"data: {"type":"response.completed","sequence_number":2,"response":{"end_turn":true}}"#,
                "\n\n"
            )
            .as_bytes()
            .to_vec()],
        }
    }

    #[tokio::test]
    async fn provider_uses_dynamic_bearer_token() {
        let credential = Arc::new(RotatingCredential {
            tokens: Mutex::new(vec!["token-a".to_string()]),
            refresh_count: std::sync::atomic::AtomicUsize::new(0),
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let transport = Arc::new(SequenceTransport {
            responses: Mutex::new(vec![success_chat()]),
            requests: Mutex::new(Vec::new()),
        });
        let provider = OpenAiCompatibleProvider::with_transport(
            OpenAiCompatibleConfig::with_credential(
                "openai",
                "https://provider.invalid/v1",
                credential,
                OpenAiApiMode::ChatCompletions,
                false,
            ),
            transport.clone(),
        );
        provider
            .next_turn(request(), Arc::new(aifluxon_core::NoopModelEventSink))
            .await
            .unwrap();
        assert_eq!(transport.requests.lock().unwrap()[0].api_key, "token-a");
    }

    #[tokio::test]
    async fn provider_does_not_cache_oauth_token_forever() {
        let credential = Arc::new(RotatingCredential {
            tokens: Mutex::new(vec!["token-a".to_string(), "token-b".to_string()]),
            refresh_count: std::sync::atomic::AtomicUsize::new(0),
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let transport = Arc::new(SequenceTransport {
            responses: Mutex::new(vec![success_chat(), success_chat()]),
            requests: Mutex::new(Vec::new()),
        });
        let provider = OpenAiCompatibleProvider::with_transport(
            OpenAiCompatibleConfig::with_credential(
                "openai",
                "https://provider.invalid/v1",
                credential.clone(),
                OpenAiApiMode::ChatCompletions,
                false,
            ),
            transport.clone(),
        );
        provider
            .next_turn(request(), Arc::new(aifluxon_core::NoopModelEventSink))
            .await
            .unwrap();
        credential.force_refresh().await.unwrap();
        provider
            .next_turn(request(), Arc::new(aifluxon_core::NoopModelEventSink))
            .await
            .unwrap();
        let keys: Vec<String> = transport
            .requests
            .lock()
            .unwrap()
            .iter()
            .map(|request| request.api_key.clone())
            .collect();
        assert_eq!(keys, vec!["token-a".to_string(), "token-b".to_string()]);
        assert_eq!(
            credential.calls.load(std::sync::atomic::Ordering::SeqCst),
            2
        );
    }

    #[tokio::test]
    async fn provider_401_forces_at_most_one_refresh_retry() {
        let credential = Arc::new(RotatingCredential {
            tokens: Mutex::new(vec!["token-a".to_string(), "token-b".to_string()]),
            refresh_count: std::sync::atomic::AtomicUsize::new(0),
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let transport = Arc::new(SequenceTransport {
            responses: Mutex::new(vec![
                OpenAiWireResponse {
                    status: 401,
                    content_type: Some("application/json".to_string()),
                    chunks: vec![br#"{"error":"unauthorized"}"#.to_vec()],
                },
                success_chat(),
            ]),
            requests: Mutex::new(Vec::new()),
        });
        let provider = OpenAiCompatibleProvider::with_transport(
            OpenAiCompatibleConfig::with_credential(
                "openai",
                "https://provider.invalid/v1",
                credential.clone(),
                OpenAiApiMode::ChatCompletions,
                false,
            ),
            transport.clone(),
        );
        let turn = provider
            .next_turn(request(), Arc::new(aifluxon_core::NoopModelEventSink))
            .await
            .unwrap();
        assert_eq!(turn.text, "done");
        assert_eq!(
            credential
                .refresh_count
                .load(std::sync::atomic::Ordering::SeqCst),
            1
        );
        let keys: Vec<String> = transport
            .requests
            .lock()
            .unwrap()
            .iter()
            .map(|request| request.api_key.clone())
            .collect();
        assert_eq!(keys, vec!["token-a".to_string(), "token-b".to_string()]);
    }

    #[tokio::test]
    async fn provider_second_401_surfaces_error() {
        let credential = Arc::new(RotatingCredential {
            tokens: Mutex::new(vec!["token-a".to_string(), "token-b".to_string()]),
            refresh_count: std::sync::atomic::AtomicUsize::new(0),
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let unauthorized = OpenAiWireResponse {
            status: 401,
            content_type: Some("application/json".to_string()),
            chunks: vec![br#"{"error":"unauthorized"}"#.to_vec()],
        };
        let transport = Arc::new(SequenceTransport {
            responses: Mutex::new(vec![unauthorized.clone(), unauthorized]),
            requests: Mutex::new(Vec::new()),
        });
        let provider = OpenAiCompatibleProvider::with_transport(
            OpenAiCompatibleConfig::with_credential(
                "openai",
                "https://provider.invalid/v1",
                credential.clone(),
                OpenAiApiMode::ChatCompletions,
                false,
            ),
            transport.clone(),
        );
        let error = provider
            .next_turn(request(), Arc::new(aifluxon_core::NoopModelEventSink))
            .await
            .unwrap_err();
        assert!(error.message.contains("401"));
        assert_eq!(transport.requests.lock().unwrap().len(), 2);
        assert_eq!(
            credential
                .refresh_count
                .load(std::sync::atomic::Ordering::SeqCst),
            1
        );
    }

    #[tokio::test]
    async fn static_api_key_path_still_works() {
        let transport = Arc::new(FakeTransport {
            response: success_responses(),
            request: Mutex::new(None),
        });
        let provider = OpenAiCompatibleProvider::with_transport(
            OpenAiCompatibleConfig::new(
                "codex",
                "https://api.openai.com/v1",
                "sk-static",
                OpenAiApiMode::Responses,
                false,
            ),
            transport.clone(),
        );
        provider
            .next_turn(request(), Arc::new(aifluxon_core::NoopModelEventSink))
            .await
            .unwrap();
        let sent = transport.request.lock().unwrap().clone().unwrap();
        assert_eq!(sent.api_key, "sk-static");
        assert!(sent.extra_headers.is_empty());
        assert_eq!(sent.url, "https://api.openai.com/v1/responses");
    }

    #[tokio::test]
    async fn oauth_codex_credential_path_sends_backend_headers() {
        let transport = Arc::new(FakeTransport {
            response: success_responses(),
            request: Mutex::new(None),
        });
        let provider = OpenAiCompatibleProvider::with_transport(
            crate::codex::oauth_config(
                Arc::new(aifluxon_auth::StaticBearerCredential::new("oauth-token")),
                "account-123",
            ),
            transport.clone(),
        );
        provider
            .next_turn(request(), Arc::new(aifluxon_core::NoopModelEventSink))
            .await
            .unwrap();
        let sent = transport.request.lock().unwrap().clone().unwrap();
        assert_eq!(sent.api_key, "oauth-token");
        assert_eq!(sent.url, "https://chatgpt.com/backend-api/codex/responses");
        assert!(sent
            .extra_headers
            .iter()
            .any(|(name, value)| name == "ChatGPT-Account-ID" && value == "account-123"));
        assert!(sent
            .extra_headers
            .iter()
            .any(|(name, value)| name == "originator" && value == crate::codex::ORIGINATOR));
    }
}
