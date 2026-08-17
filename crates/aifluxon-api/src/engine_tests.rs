use crate::{
    Aifluxon, AifluxonErrorKind, AllowAllToolPolicy, ContinuationReason, ModelEventSink,
    ModelProvider, ModelRef, ModelTurn, ModelTurnRequest, NoopRunEventSink, OperationDecision,
    ProviderCapabilities, ProviderError, ProviderId, ProviderRegistry, ProviderTerminal, RunEvent,
    RunLimits, RunRequest, RunState, ToolCall, ToolDescriptor, ToolEffect, ToolExecutionContext,
    ToolExecutionError, ToolExecutor, ToolInvocation, ToolInvocationId, ToolPolicy, ToolRegistry,
    ToolResult,
};
use aifluxon_core::{
    ContentPart, Message, MessageRole, OperationMode, PendingOperationDraft, ProviderSessionKey,
};
use aifluxon_runtime::{ToolDecision, ToolPolicyInput};
use serde_json::json;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

struct ScriptedProvider {
    id: ProviderId,
    turns: MutexTurns,
    requests: Arc<std::sync::Mutex<Vec<ModelTurnRequest>>>,
}

struct MutexTurns(std::sync::Mutex<Vec<ModelTurn>>);

#[async_trait::async_trait]
impl ModelProvider for ScriptedProvider {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::openai_compatible()
    }

    async fn next_turn(
        &self,
        request: ModelTurnRequest,
        sink: Arc<dyn ModelEventSink>,
    ) -> Result<ModelTurn, ProviderError> {
        let turn =
            self.turns.0.lock().unwrap().pop().ok_or_else(|| {
                ProviderError::message("Scripted provider has no remaining turns.")
            })?;
        self.requests.lock().unwrap().push(request);
        if !turn.text.is_empty() {
            sink.on_text_delta(&turn.text);
        }
        Ok(turn)
    }
}

struct ParkingProvider {
    id: ProviderId,
    started: Arc<std::sync::atomic::AtomicBool>,
}

#[async_trait::async_trait]
impl ModelProvider for ParkingProvider {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::openai_compatible()
    }

    async fn next_turn(
        &self,
        _request: ModelTurnRequest,
        _sink: Arc<dyn ModelEventSink>,
    ) -> Result<ModelTurn, ProviderError> {
        self.started.store(true, Ordering::SeqCst);
        std::future::pending().await
    }
}

struct EchoTool {
    executions: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl ToolExecutor for EchoTool {
    async fn execute(
        &self,
        invocation: ToolInvocation,
        _ctx: ToolExecutionContext,
    ) -> Result<ToolResult, ToolExecutionError> {
        self.executions.fetch_add(1, Ordering::SeqCst);
        Ok(ToolResult {
            value: json!({ "ok": true, "name": invocation.name }),
        })
    }
}

struct ApprovalPolicy {
    mode: OperationMode,
}

impl ToolPolicy for ApprovalPolicy {
    fn evaluate(&self, input: &ToolPolicyInput<'_>) -> ToolDecision {
        ToolDecision::RequireApproval {
            operation: PendingOperationDraft {
                invocation_id: Some(input.invocation.id),
                effect: input.descriptor.effect,
                mode: self.mode,
                summary: "host approval".to_string(),
                payload: json!({ "name": input.descriptor.name }),
                deadline: None,
            },
        }
    }
}

fn user_request(provider: &str) -> RunRequest {
    RunRequest {
        session_id: None,
        messages: vec![Message {
            role: MessageRole::User,
            content: vec![ContentPart::Text("hello".to_string())],
            tool_calls: Vec::new(),
            tool_call_id: None,
            provider_state: None,
        }],
        model: ModelRef {
            provider: ProviderId::new(provider),
            model: "scripted".to_string(),
        },
        session_key: Some(ProviderSessionKey::from_cache_session("session-a")),
        allowed_tools: None,
        limits: RunLimits {
            max_model_rounds: 8,
            max_tool_invocations: 8,
        },
        features: Default::default(),
        authority: None,
    }
}

fn scripted(turns: Vec<ModelTurn>) -> ScriptedProvider {
    ScriptedProvider {
        id: ProviderId::new("scripted"),
        turns: MutexTurns(std::sync::Mutex::new(turns)),
        requests: Arc::new(std::sync::Mutex::new(Vec::new())),
    }
}

fn continue_turn(reason: ContinuationReason, text: &str, opaque: serde_json::Value) -> ModelTurn {
    ModelTurn {
        text: text.to_string(),
        reasoning: String::new(),
        tool_calls: Vec::new(),
        usage: None,
        terminal: ProviderTerminal::Continue(reason),
        opaque,
    }
}

fn stop_turn(text: &str) -> ModelTurn {
    ModelTurn {
        text: text.to_string(),
        reasoning: String::new(),
        tool_calls: Vec::new(),
        usage: None,
        terminal: ProviderTerminal::Stop,
        opaque: json!({}),
    }
}

fn tool_turn(name: &str, id: &str) -> ModelTurn {
    ModelTurn {
        text: String::new(),
        reasoning: String::new(),
        tool_calls: vec![ToolCall {
            id: ToolInvocationId::from_stable_key(id),
            name: name.to_string(),
            arguments: json!({}),
            provider_call_id: Some(id.to_string()),
        }],
        usage: None,
        terminal: ProviderTerminal::ToolCalls,
        opaque: json!({}),
    }
}

fn read_descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: "read_file".to_string(),
        description: "read".to_string(),
        input_schema: json!({ "type": "object" }),
        effect: ToolEffect::PureRead,
        required_capabilities: Vec::new(),
        parallel_safe: true,
    }
}

async fn wait_for_terminal(handle: &mut crate::RunHandle) -> RunEvent {
    let mut last = None;
    while let Some(event) = handle.events().next().await {
        if event.event.terminal_state().is_some() {
            return event.event;
        }
        last = Some(event.event);
    }
    panic!("missing terminal event, last={last:?}");
}

fn backend(
    registry: ProviderRegistry,
    tools: ToolRegistry,
    policy: Arc<dyn ToolPolicy>,
) -> Aifluxon {
    Aifluxon::builder()
        .provider_registry(registry)
        .tool_registry(tools)
        .tool_policy(policy)
        .event_sink(Arc::new(NoopRunEventSink))
        .build()
        .unwrap()
}

#[tokio::test]
async fn start_returns_a_live_handle_before_the_model_finishes() {
    let started = Arc::new(AtomicBool::new(false));
    let registry = ProviderRegistry::new();
    registry
        .register(
            ProviderId::new("scripted"),
            ParkingProvider {
                id: ProviderId::new("scripted"),
                started: started.clone(),
            },
        )
        .unwrap();
    let backend = backend(registry, ToolRegistry::new(), Arc::new(AllowAllToolPolicy));
    let handle = backend.start(user_request("scripted")).await.unwrap();
    let snapshot = backend.snapshot(handle.id()).await.unwrap();
    assert_eq!(snapshot.state, RunState::Running);
    assert!(snapshot.last_event_sequence >= 1);
    while !started.load(Ordering::SeqCst) {
        tokio::task::yield_now().await;
    }
    backend.cancel(handle.id()).await.unwrap();
}

#[tokio::test]
async fn provider_final_response_completes_exactly_once() {
    let registry = ProviderRegistry::new();
    registry
        .register(
            ProviderId::new("scripted"),
            scripted(vec![stop_turn("done")]),
        )
        .unwrap();
    let backend = backend(registry, ToolRegistry::new(), Arc::new(AllowAllToolPolicy));
    let mut handle = backend.start(user_request("scripted")).await.unwrap();
    let mut terminals = 0u32;
    let mut last_seq = 0;
    while let Some(event) = handle.events().next().await {
        assert!(event.sequence > last_seq);
        last_seq = event.sequence;
        if event.event.terminal_state().is_some() {
            terminals += 1;
            assert!(matches!(event.event, RunEvent::Completed { .. }));
        }
    }
    assert_eq!(terminals, 1);
}

#[tokio::test]
async fn tool_result_continues_the_model_turn() {
    let executions = Arc::new(AtomicUsize::new(0));
    let registry = ProviderRegistry::new();
    registry
        .register(
            ProviderId::new("scripted"),
            scripted(vec![
                stop_turn("after-tool"),
                tool_turn("read_file", "call-1"),
            ]),
        )
        .unwrap();
    let tools = ToolRegistry::new();
    tools
        .register(
            read_descriptor(),
            Arc::new(EchoTool {
                executions: executions.clone(),
            }),
        )
        .unwrap();
    let backend = backend(registry, tools, Arc::new(AllowAllToolPolicy));
    let mut handle = backend.start(user_request("scripted")).await.unwrap();
    assert!(matches!(
        wait_for_terminal(&mut handle).await,
        RunEvent::Completed { .. }
    ));
    assert_eq!(executions.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn duplicate_tool_id_is_executed_at_most_once() {
    let executions = Arc::new(AtomicUsize::new(0));
    let registry = ProviderRegistry::new();
    registry
        .register(
            ProviderId::new("scripted"),
            scripted(vec![
                stop_turn("after-replay"),
                tool_turn("read_file", "same-call"),
                tool_turn("read_file", "same-call"),
            ]),
        )
        .unwrap();
    let tools = ToolRegistry::new();
    tools
        .register(
            read_descriptor(),
            Arc::new(EchoTool {
                executions: executions.clone(),
            }),
        )
        .unwrap();
    let backend = backend(registry, tools, Arc::new(AllowAllToolPolicy));
    let mut handle = backend.start(user_request("scripted")).await.unwrap();
    wait_for_terminal(&mut handle).await;
    assert_eq!(executions.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn cancel_unblocks_a_waiting_provider() {
    let started = Arc::new(AtomicBool::new(false));
    let registry = ProviderRegistry::new();
    registry
        .register(
            ProviderId::new("scripted"),
            ParkingProvider {
                id: ProviderId::new("scripted"),
                started: started.clone(),
            },
        )
        .unwrap();
    let backend = backend(registry, ToolRegistry::new(), Arc::new(AllowAllToolPolicy));
    let mut handle = backend.start(user_request("scripted")).await.unwrap();
    while !started.load(Ordering::SeqCst) {
        tokio::task::yield_now().await;
    }
    backend.cancel(handle.id()).await.unwrap();
    assert!(matches!(
        wait_for_terminal(&mut handle).await,
        RunEvent::Cancelled | RunEvent::Failed { .. }
    ));
}

#[tokio::test]
async fn blocking_approval_pauses_and_resumes() {
    let executions = Arc::new(AtomicUsize::new(0));
    let registry = ProviderRegistry::new();
    registry
        .register(
            ProviderId::new("scripted"),
            scripted(vec![
                stop_turn("approved"),
                tool_turn("read_file", "call-1"),
            ]),
        )
        .unwrap();
    let tools = ToolRegistry::new();
    tools
        .register(
            read_descriptor(),
            Arc::new(EchoTool {
                executions: executions.clone(),
            }),
        )
        .unwrap();
    let backend = backend(
        registry,
        tools,
        Arc::new(ApprovalPolicy {
            mode: OperationMode::BlockingApproval,
        }),
    );
    let mut handle = backend.start(user_request("scripted")).await.unwrap();
    let mut operation_id = None;
    while let Some(event) = handle.events().next().await {
        if let RunEvent::OperationRequested { operation } = event.event {
            operation_id = Some(operation.id);
            break;
        }
    }
    let operation_id = operation_id.expect("approval requested");
    backend
        .resolve_operation(
            handle.id(),
            operation_id,
            OperationDecision::Approve { data: None },
        )
        .await
        .unwrap();
    assert!(matches!(
        wait_for_terminal(&mut handle).await,
        RunEvent::Completed { .. }
    ));
    assert_eq!(executions.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn deferred_commit_is_not_flattened_to_plain_approval() {
    let registry = ProviderRegistry::new();
    registry
        .register(
            ProviderId::new("scripted"),
            scripted(vec![
                stop_turn("after-commit"),
                tool_turn("read_file", "call-1"),
            ]),
        )
        .unwrap();
    let executions = Arc::new(AtomicUsize::new(0));
    let tools = ToolRegistry::new();
    tools
        .register(
            read_descriptor(),
            Arc::new(EchoTool {
                executions: executions.clone(),
            }),
        )
        .unwrap();
    let backend = backend(
        registry,
        tools,
        Arc::new(ApprovalPolicy {
            mode: OperationMode::DeferredCommit,
        }),
    );
    let mut handle = backend.start(user_request("scripted")).await.unwrap();
    let operation_id = loop {
        let Some(envelope) = handle.events().next().await else {
            panic!("missing operation request");
        };
        if let RunEvent::OperationRequested { operation } = envelope.event {
            assert_eq!(operation.mode, OperationMode::DeferredCommit);
            break operation.id;
        }
    };
    let approve_error = backend
        .resolve_operation(
            handle.id(),
            operation_id,
            OperationDecision::Approve { data: None },
        )
        .await
        .unwrap_err();
    assert_eq!(approve_error.kind(), AifluxonErrorKind::StateConflict);
    backend
        .commit_prepared_operation(handle.id(), operation_id)
        .await
        .unwrap();
    assert!(matches!(
        wait_for_terminal(&mut handle).await,
        RunEvent::Completed { .. }
    ));
    assert_eq!(executions.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn budget_stops_additional_model_rounds() {
    let registry = ProviderRegistry::new();
    registry
        .register(
            ProviderId::new("scripted"),
            scripted(vec![
                stop_turn("should-not-run"),
                tool_turn("read_file", "call-1"),
            ]),
        )
        .unwrap();
    let tools = ToolRegistry::new();
    tools
        .register(
            read_descriptor(),
            Arc::new(EchoTool {
                executions: Arc::new(AtomicUsize::new(0)),
            }),
        )
        .unwrap();
    let backend = backend(registry, tools, Arc::new(AllowAllToolPolicy));
    let mut request = user_request("scripted");
    request.limits.max_model_rounds = 1;
    let mut handle = backend.start(request).await.unwrap();
    assert!(matches!(
        wait_for_terminal(&mut handle).await,
        RunEvent::Failed { .. }
    ));
}

#[tokio::test]
async fn unknown_tool_fails_closed() {
    let registry = ProviderRegistry::new();
    registry
        .register(
            ProviderId::new("scripted"),
            scripted(vec![tool_turn("missing_tool", "call-1")]),
        )
        .unwrap();
    let backend = backend(registry, ToolRegistry::new(), Arc::new(AllowAllToolPolicy));
    let mut handle = backend.start(user_request("scripted")).await.unwrap();
    assert!(matches!(
        wait_for_terminal(&mut handle).await,
        RunEvent::Failed { .. }
    ));
}

#[tokio::test]
async fn unknown_allowed_tool_is_rejected_before_runtime_handoff() {
    let registry = ProviderRegistry::new();
    registry
        .register(
            ProviderId::new("scripted"),
            scripted(vec![stop_turn("unused")]),
        )
        .unwrap();
    let backend = backend(registry, ToolRegistry::new(), Arc::new(AllowAllToolPolicy));
    let mut request = user_request("scripted");
    request.allowed_tools = Some(vec!["missing_tool".to_string()]);
    let error = backend.start(request).await.unwrap_err();
    assert_eq!(error.kind(), AifluxonErrorKind::Tool);
}

#[tokio::test]
async fn start_on_reuses_a_host_registered_run() {
    let table = aifluxon_runtime::RunTable::new();
    let context = table
        .start(
            None,
            None,
            RunLimits {
                max_model_rounds: 4,
                max_tool_invocations: 4,
            },
        )
        .unwrap();
    let registry = ProviderRegistry::new();
    registry
        .register(
            ProviderId::new("scripted"),
            scripted(vec![stop_turn("shared-table")]),
        )
        .unwrap();
    let backend = Aifluxon::builder()
        .provider_registry(registry)
        .tool_registry(ToolRegistry::new())
        .tool_policy(Arc::new(AllowAllToolPolicy))
        .event_sink(Arc::new(NoopRunEventSink))
        .run_table(table)
        .build()
        .unwrap();
    let mut handle = backend
        .start_on(context.run_id, user_request("scripted"))
        .await
        .unwrap();
    assert_eq!(handle.id(), context.run_id);
    wait_for_terminal(&mut handle).await;
}

struct ContinueThenPark {
    first_started: Arc<AtomicBool>,
    second_started: Arc<AtomicBool>,
}

#[async_trait::async_trait]
impl ModelProvider for ContinueThenPark {
    fn id(&self) -> &ProviderId {
        static ID: std::sync::OnceLock<ProviderId> = std::sync::OnceLock::new();
        ID.get_or_init(|| ProviderId::new("scripted"))
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::openai_compatible()
    }

    async fn next_turn(
        &self,
        _request: ModelTurnRequest,
        _sink: Arc<dyn ModelEventSink>,
    ) -> Result<ModelTurn, ProviderError> {
        if !self.first_started.swap(true, Ordering::SeqCst) {
            return Ok(continue_turn(
                ContinuationReason::ProviderRequested,
                "partial",
                json!({}),
            ));
        }
        self.second_started.store(true, Ordering::SeqCst);
        std::future::pending().await
    }
}

struct AlwaysContinue;

#[async_trait::async_trait]
impl ModelProvider for AlwaysContinue {
    fn id(&self) -> &ProviderId {
        static ID: std::sync::OnceLock<ProviderId> = std::sync::OnceLock::new();
        ID.get_or_init(|| ProviderId::new("scripted"))
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::openai_compatible()
    }

    async fn next_turn(
        &self,
        _request: ModelTurnRequest,
        _sink: Arc<dyn ModelEventSink>,
    ) -> Result<ModelTurn, ProviderError> {
        Ok(continue_turn(
            ContinuationReason::ProviderRequested,
            "again",
            json!({}),
        ))
    }
}

async fn start_scripted(turns: Vec<ModelTurn>) -> (crate::Aifluxon, crate::RunHandle) {
    let registry = ProviderRegistry::new();
    registry
        .register(ProviderId::new("scripted"), scripted(turns))
        .unwrap();
    let backend = backend(registry, ToolRegistry::new(), Arc::new(AllowAllToolPolicy));
    let handle = backend.start(user_request("scripted")).await.unwrap();
    (backend, handle)
}

#[tokio::test]
async fn continuation_runs_another_model_turn() {
    let provider = scripted(vec![
        stop_turn("done"),
        continue_turn(ContinuationReason::ProviderRequested, "partial", json!({})),
    ]);
    let requests = provider.requests.clone();
    let registry = ProviderRegistry::new();
    registry
        .register(ProviderId::new("scripted"), provider)
        .unwrap();
    let backend = backend(registry, ToolRegistry::new(), Arc::new(AllowAllToolPolicy));
    let mut handle = backend.start(user_request("scripted")).await.unwrap();
    assert!(matches!(
        wait_for_terminal(&mut handle).await,
        RunEvent::Completed { .. }
    ));
    assert_eq!(requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn continuation_preserves_run_identity_and_provider_session_key() {
    let provider = scripted(vec![
        stop_turn("done"),
        continue_turn(ContinuationReason::ProviderRequested, "partial", json!({})),
    ]);
    let requests = provider.requests.clone();
    let registry = ProviderRegistry::new();
    registry
        .register(ProviderId::new("scripted"), provider)
        .unwrap();
    let backend = backend(registry, ToolRegistry::new(), Arc::new(AllowAllToolPolicy));
    let mut handle = backend.start(user_request("scripted")).await.unwrap();
    let run_id = handle.id();
    wait_for_terminal(&mut handle).await;
    let recorded = requests.lock().unwrap();
    assert_eq!(recorded.len(), 2);
    assert_eq!(recorded[0].run_id, run_id);
    assert_eq!(recorded[1].run_id, run_id);
    assert_eq!(
        recorded[0].session_key.as_str(),
        recorded[1].session_key.as_str()
    );
    assert_eq!(recorded[0].session_key.as_str(), "session-a");
}

#[tokio::test]
async fn continuation_keeps_tool_ledger_and_does_not_reexecute_side_effect() {
    let executions = Arc::new(AtomicUsize::new(0));
    let registry = ProviderRegistry::new();
    registry
        .register(
            ProviderId::new("scripted"),
            scripted(vec![
                stop_turn("after-continue"),
                continue_turn(ContinuationReason::Incomplete, "need tools", json!({})),
                tool_turn("read_file", "call-1"),
            ]),
        )
        .unwrap();
    let tools = ToolRegistry::new();
    tools
        .register(
            read_descriptor(),
            Arc::new(EchoTool {
                executions: executions.clone(),
            }),
        )
        .unwrap();
    let backend = backend(registry, tools, Arc::new(AllowAllToolPolicy));
    let mut handle = backend.start(user_request("scripted")).await.unwrap();
    wait_for_terminal(&mut handle).await;
    assert_eq!(executions.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn continuation_does_not_reexecute_recorded_tool_call() {
    let executions = Arc::new(AtomicUsize::new(0));
    let registry = ProviderRegistry::new();
    registry
        .register(
            ProviderId::new("scripted"),
            scripted(vec![
                stop_turn("after-replay"),
                tool_turn("read_file", "same-call"),
                continue_turn(ContinuationReason::ProviderRequested, "continue", json!({})),
                tool_turn("read_file", "same-call"),
            ]),
        )
        .unwrap();
    let tools = ToolRegistry::new();
    tools
        .register(
            read_descriptor(),
            Arc::new(EchoTool {
                executions: executions.clone(),
            }),
        )
        .unwrap();
    let backend = backend(registry, tools, Arc::new(AllowAllToolPolicy));
    let mut handle = backend.start(user_request("scripted")).await.unwrap();
    wait_for_terminal(&mut handle).await;
    assert_eq!(executions.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn continuation_respects_cancellation() {
    let first_started = Arc::new(AtomicBool::new(false));
    let second_started = Arc::new(AtomicBool::new(false));
    let registry = ProviderRegistry::new();
    registry
        .register(
            ProviderId::new("scripted"),
            ContinueThenPark {
                first_started: first_started.clone(),
                second_started: second_started.clone(),
            },
        )
        .unwrap();
    let backend = backend(registry, ToolRegistry::new(), Arc::new(AllowAllToolPolicy));
    let mut handle = backend.start(user_request("scripted")).await.unwrap();
    while !second_started.load(Ordering::SeqCst) {
        tokio::task::yield_now().await;
    }
    backend.cancel(handle.id()).await.unwrap();
    assert!(matches!(
        wait_for_terminal(&mut handle).await,
        RunEvent::Cancelled | RunEvent::Failed { .. }
    ));
}

#[tokio::test]
async fn continuation_stops_at_budget() {
    let registry = ProviderRegistry::new();
    registry
        .register(ProviderId::new("scripted"), AlwaysContinue)
        .unwrap();
    let backend = backend(registry, ToolRegistry::new(), Arc::new(AllowAllToolPolicy));
    let mut request = user_request("scripted");
    request.limits.max_model_rounds = 2;
    let mut handle = backend.start(request).await.unwrap();
    assert!(matches!(
        wait_for_terminal(&mut handle).await,
        RunEvent::Failed { .. }
    ));
}

#[tokio::test]
async fn continuation_terminal_exactly_once_and_event_sequence_is_monotonic() {
    let (_backend, mut handle) = start_scripted(vec![
        stop_turn("done"),
        continue_turn(
            ContinuationReason::SummaryOnly,
            "",
            json!({
                "hidden_context": "internal recap"
            }),
        ),
    ])
    .await;
    let mut terminals = 0u32;
    let mut last_seq = 0;
    while let Some(event) = handle.events().next().await {
        assert!(event.sequence > last_seq);
        last_seq = event.sequence;
        if event.event.terminal_state().is_some() {
            terminals += 1;
            assert!(matches!(event.event, RunEvent::Completed { .. }));
        }
    }
    assert_eq!(terminals, 1);
}

#[tokio::test]
async fn normal_answer_without_tools_does_not_continue() {
    let provider = scripted(vec![stop_turn("The answer is 42.")]);
    let requests = provider.requests.clone();
    let registry = ProviderRegistry::new();
    registry
        .register(ProviderId::new("scripted"), provider)
        .unwrap();
    let backend = backend(registry, ToolRegistry::new(), Arc::new(AllowAllToolPolicy));
    let mut handle = backend.start(user_request("scripted")).await.unwrap();
    wait_for_terminal(&mut handle).await;
    assert_eq!(requests.lock().unwrap().len(), 1);
}
