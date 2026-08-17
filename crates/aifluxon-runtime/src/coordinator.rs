#![allow(dead_code)]

use crate::budget::AgentBudgetExceeded;
use crate::continuation::{apply_continuation, ContinuationCounts};
use crate::{
    can_run_tool_calls_in_parallel, ParallelToolCall, RunTable, ToolDecision, ToolExecutionContext,
    ToolPolicy, ToolPolicyInput, ToolRegistry,
};
use aifluxon_core::{
    ContentPart, ExecutionAuthority, Message, MessageRole, ModelEventSink, ModelProvider,
    ModelTurn, ModelTurnRequest, NoopModelEventSink, OperationDecision, OperationId, OperationMode,
    ProviderError, ProviderTerminal, RunEvent, ToolCall, ToolInvocationId,
};
use futures_util::future::join_all;
use serde_json::Value;
use std::sync::Arc;

#[derive(Clone)]
pub struct AgentCoordinator {
    run_table: RunTable,
    run_id: aifluxon_core::RunId,
}

#[derive(Clone, Debug)]
pub struct RunExecutionResult {
    pub turn: ModelTurn,
    pub messages: Vec<Message>,
}

impl Default for AgentCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentCoordinator {
    pub fn new() -> Self {
        let run_table = RunTable::new();
        let context = run_table
            .start(
                None,
                None,
                aifluxon_core::RunLimits {
                    max_model_rounds: 64,
                    max_tool_invocations: 64,
                },
            )
            .expect("fresh local run must register");
        Self::for_run(run_table, context.run_id)
    }

    pub fn for_run(run_table: RunTable, run_id: aifluxon_core::RunId) -> Self {
        Self { run_table, run_id }
    }

    pub fn consume_model_round(&self) -> Result<(), AgentBudgetExceeded> {
        self.run_table.consume_model_round(&self.run_id)
    }

    pub fn consume_tool_invocation(&self) -> Result<(), AgentBudgetExceeded> {
        self.run_table.consume_tool_invocation(&self.run_id)
    }

    pub fn execution_count(&self) -> usize {
        self.run_table
            .tool_execution_count(&self.run_id)
            .unwrap_or(0)
    }

    pub fn model_rounds(&self) -> u32 {
        self.run_table.model_rounds(&self.run_id).unwrap_or(0)
    }

    pub fn cached_tool_result(&self, invocation_id: &str) -> Option<Value> {
        self.run_table
            .cached_tool_result(
                &self.run_id,
                &ToolInvocationId::from_stable_key(invocation_id),
            )
            .ok()
            .flatten()
    }

    pub fn record_tool_result(&self, invocation_id: &str, value: Value) {
        let _ = self.run_table.record_tool_result(
            &self.run_id,
            ToolInvocationId::from_stable_key(invocation_id),
            value,
        );
    }

    pub fn execute_once_recorded(
        &self,
        invocation_id: &str,
        execute: impl FnOnce() -> Value,
    ) -> Value {
        self.run_table
            .execute_tool_once(
                &self.run_id,
                ToolInvocationId::from_stable_key(invocation_id),
                execute,
            )
            .unwrap_or_else(|error| {
                serde_json::json!({
                    "ok": false,
                    "error": error.message(),
                })
            })
    }

    pub async fn run_model_tool_loop<P: ModelProvider>(
        &self,
        provider: &P,
        mut request: ModelTurnRequest,
        execute: impl Fn(&ToolCall) -> Value + Send + Sync,
    ) -> Result<ModelTurn, ProviderError> {
        let sink: Arc<dyn ModelEventSink> = Arc::new(NoopModelEventSink);
        let mut continuations = ContinuationCounts::default();
        loop {
            self.consume_model_round().map_err(|error| {
                ProviderError::message(format!("Agent loop budget exceeded: {:?}", error.kind))
            })?;
            let turn = provider.next_turn(request.clone(), sink.clone()).await?;
            request.opaque_state = Some(turn.opaque.clone());
            if let Some(reason) = turn.terminal.continuation_reason() {
                if continuations.try_begin(reason) {
                    apply_continuation(&mut request, &turn, reason);
                    continue;
                }
                return Ok(turn);
            }
            if turn.tool_calls.is_empty() || turn.terminal != ProviderTerminal::ToolCalls {
                return Ok(turn);
            }
            continuations.reset_incomplete();
            request.messages.push(assistant_message(&turn));
            for call in &turn.tool_calls {
                self.consume_tool_invocation().map_err(|error| {
                    ProviderError::message(format!("Agent loop budget exceeded: {:?}", error.kind))
                })?;
                let invocation_id = call.id.hyphenated();
                let result = self.execute_once_recorded(&invocation_id, || execute(call));
                request.messages.push(Message {
                    role: MessageRole::Tool,
                    content: vec![ContentPart::Text(result.to_string())],
                    tool_calls: Vec::new(),
                    tool_call_id: Some(call.id),
                    provider_state: None,
                });
            }
        }
    }

    pub async fn run_registered_model_tool_loop(
        &self,
        provider: Arc<dyn ModelProvider>,
        tools: Arc<ToolRegistry>,
        policy: Arc<dyn ToolPolicy>,
        authority: ExecutionAuthority,
        mut request: ModelTurnRequest,
        sink: Arc<dyn ModelEventSink>,
    ) -> Result<RunExecutionResult, ProviderError> {
        let cancellation = self
            .run_table
            .cancellation_token(&self.run_id)
            .map_err(|error| ProviderError::message(error.message()))?;
        let mut continuations = ContinuationCounts::default();
        loop {
            if cancellation.is_cancelled() {
                return Err(ProviderError::message("Run was cancelled."));
            }
            self.consume_model_round().map_err(|error| {
                ProviderError::message(format!("Agent loop budget exceeded: {:?}", error.kind))
            })?;
            let turn = tokio::select! {
                _ = cancellation.cancelled() => return Err(ProviderError::message("Run was cancelled.")),
                result = provider.next_turn(request.clone(), sink.clone()) => result?,
            };
            request.opaque_state = Some(turn.opaque.clone());
            if let Some(reason) = turn.terminal.continuation_reason() {
                if continuations.try_begin(reason) {
                    apply_continuation(&mut request, &turn, reason);
                    continue;
                }
                return Ok(complete_without_tools(request.messages, turn));
            }
            if turn.tool_calls.is_empty() || turn.terminal != ProviderTerminal::ToolCalls {
                return Ok(complete_without_tools(request.messages, turn));
            }

            continuations.reset_incomplete();
            request.messages.push(assistant_message(&turn));
            let resolved = turn
                .tool_calls
                .iter()
                .map(|call| {
                    tools
                        .resolve(&call.name)
                        .map(|registered| (call, registered))
                        .ok_or_else(|| {
                            ProviderError::message(format!(
                                "Provider requested unregistered tool `{}`.",
                                call.name
                            ))
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            let parallel = can_run_tool_calls_in_parallel(
                &resolved
                    .iter()
                    .map(|(_, registered)| ParallelToolCall {
                        descriptor: registered.descriptor(),
                    })
                    .collect::<Vec<_>>(),
            );

            let results = if parallel {
                join_all(turn.tool_calls.iter().cloned().map(|call| {
                    let coordinator = self.clone();
                    let tools = tools.clone();
                    let policy = policy.clone();
                    let authority = authority.clone();
                    let cancellation = cancellation.clone();
                    async move {
                        coordinator
                            .execute_registered_tool(call, tools, policy, authority, cancellation)
                            .await
                    }
                }))
                .await
                .into_iter()
                .collect::<Result<Vec<_>, _>>()?
            } else {
                let mut results = Vec::with_capacity(turn.tool_calls.len());
                for call in turn.tool_calls.iter().cloned() {
                    results.push(
                        self.execute_registered_tool(
                            call,
                            tools.clone(),
                            policy.clone(),
                            authority.clone(),
                            cancellation.clone(),
                        )
                        .await?,
                    );
                }
                results
            };
            request.messages.extend(results);
        }
    }

    async fn execute_registered_tool(
        &self,
        call: ToolCall,
        tools: Arc<ToolRegistry>,
        policy: Arc<dyn ToolPolicy>,
        authority: ExecutionAuthority,
        cancellation: crate::RunCancellationToken,
    ) -> Result<Message, ProviderError> {
        self.consume_tool_invocation().map_err(|error| {
            ProviderError::message(format!("Agent loop budget exceeded: {:?}", error.kind))
        })?;
        if let Some(result) = self
            .run_table
            .cached_tool_result(&self.run_id, &call.id)
            .map_err(|error| ProviderError::message(error.message()))?
        {
            return Ok(tool_message(&call, result));
        }
        let raw_arguments = serde_json::to_string(&call.arguments).map_err(|error| {
            ProviderError::message(format!("Tool arguments are invalid: {error}"))
        })?;
        let (invocation, executor) = tools
            .prepare_invocation(call.id, &call.name, &raw_arguments)
            .map_err(|error| ProviderError::message(error.message()))?;
        let registered = tools
            .resolve(&call.name)
            .ok_or_else(|| ProviderError::message("Tool disappeared during execution."))?;
        let facts = policy
            .resolve_facts(registered.descriptor(), &invocation, &authority)
            .map_err(ProviderError::message)?;
        let mut approved_operation: Option<(OperationId, OperationMode)> = None;
        match policy.evaluate(&ToolPolicyInput {
            descriptor: registered.descriptor(),
            invocation: &invocation,
            facts: &facts,
            authority: &authority,
        }) {
            ToolDecision::Allow => {}
            ToolDecision::Deny { reason } => {
                return Ok(tool_message(
                    &call,
                    serde_json::json!({ "ok": false, "error": reason }),
                ))
            }
            ToolDecision::RequireApproval { operation } => {
                let mode = operation.mode;
                let operation = operation.bind(OperationId::new(), self.run_id);
                self.run_table
                    .operations()
                    .register(operation.clone())
                    .map_err(|error| ProviderError::message(error.message()))?;
                self.run_table
                    .set_state(&self.run_id, aifluxon_core::RunState::AwaitingOperation)
                    .map_err(|error| ProviderError::message(error.message()))?;
                self.run_table
                    .emit(
                        &self.run_id,
                        RunEvent::OperationRequested {
                            operation: operation.clone(),
                        },
                    )
                    .map_err(|error| ProviderError::message(error.message()))?;
                let host_decision = match mode {
                    OperationMode::BlockingApproval => self
                        .run_table
                        .operations()
                        .wait(&self.run_id, &operation.id)
                        .await
                        .map_err(|error| ProviderError::message(error.message()))?,
                    OperationMode::DeferredCommit => self
                        .run_table
                        .operations()
                        .wait_for_commit_or_reject(&self.run_id, &operation.id)
                        .await
                        .map_err(|error| ProviderError::message(error.message()))?,
                };
                match host_decision {
                    OperationDecision::Approve { .. } => self
                        .run_table
                        .set_state(&self.run_id, aifluxon_core::RunState::Running)
                        .map(|_| approved_operation = Some((operation.id, mode)))
                        .map_err(|error| ProviderError::message(error.message()))?,
                    OperationDecision::Reject { reason } => {
                        self.run_table
                            .set_state(&self.run_id, aifluxon_core::RunState::Running)
                            .map_err(|error| ProviderError::message(error.message()))?;
                        return Ok(tool_message(
                            &call,
                            serde_json::json!({
                                "ok": false,
                                "error": reason.unwrap_or_else(|| "Operation rejected.".to_string())
                            }),
                        ));
                    }
                };
            }
        }
        self.run_table
            .emit(
                &self.run_id,
                RunEvent::ToolStarted {
                    invocation_id: call.id,
                    name: call.name.clone(),
                    arguments: call.arguments.clone(),
                },
            )
            .map_err(|error| ProviderError::message(error.message()))?;
        let execution = tokio::select! {
            _ = cancellation.cancelled() => Err(ProviderError::message("Run was cancelled.")),
            result = executor.execute(invocation, ToolExecutionContext) => result
                .map_err(|error| ProviderError::message(error.message())),
        };
        let result = match execution {
            Ok(result) => result,
            Err(error) => {
                if let Some((operation_id, mode)) = approved_operation {
                    let _ = finish_host_operation(
                        &self.run_table,
                        &self.run_id,
                        operation_id,
                        mode,
                        Err(error.to_string()),
                    );
                }
                return Err(error);
            }
        };
        if let Some((operation_id, mode)) = approved_operation {
            finish_host_operation(&self.run_table, &self.run_id, operation_id, mode, Ok(()))
                .map_err(|error| ProviderError::message(error.message()))?;
        }
        self.run_table
            .record_tool_result(&self.run_id, call.id, result.value.clone())
            .map_err(|error| ProviderError::message(error.message()))?;
        self.run_table
            .emit(
                &self.run_id,
                RunEvent::ToolFinished {
                    invocation_id: call.id,
                    name: call.name.clone(),
                    result: result.value.clone(),
                },
            )
            .map_err(|error| ProviderError::message(error.message()))?;
        Ok(tool_message(&call, result.value))
    }
}

fn finish_host_operation(
    table: &RunTable,
    run_id: &aifluxon_core::RunId,
    operation_id: OperationId,
    mode: OperationMode,
    result: Result<(), String>,
) -> Result<(), crate::operations::OperationError> {
    match mode {
        OperationMode::BlockingApproval => {
            table
                .operations()
                .finish_blocking_approval(run_id, &operation_id, result)
        }
        OperationMode::DeferredCommit => {
            table
                .operations()
                .finish_commit(run_id, &operation_id, result)
        }
    }
}

fn complete_without_tools(mut messages: Vec<Message>, turn: ModelTurn) -> RunExecutionResult {
    if !turn.text.is_empty() || !turn.reasoning.is_empty() || !turn.opaque.is_null() {
        messages.push(assistant_message(&turn));
    }
    RunExecutionResult { turn, messages }
}

fn assistant_message(turn: &ModelTurn) -> Message {
    Message {
        role: MessageRole::Assistant,
        content: (!turn.text.is_empty())
            .then(|| ContentPart::Text(turn.text.clone()))
            .into_iter()
            .collect(),
        tool_calls: turn.tool_calls.clone(),
        tool_call_id: None,
        provider_state: (!turn.opaque.is_null()).then(|| turn.opaque.clone()),
    }
}

fn tool_message(call: &ToolCall, result: Value) -> Message {
    Message {
        role: MessageRole::Tool,
        content: vec![ContentPart::Text(result.to_string())],
        tool_calls: Vec::new(),
        tool_call_id: Some(call.id),
        provider_state: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::AgentBudgetKind;
    use aifluxon_core::{ProviderCapabilities, ProviderSessionKey, RunId, ToolInvocationId};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    struct CountingProvider {
        remaining_tool_turns: Mutex<u32>,
    }

    #[async_trait::async_trait]
    impl ModelProvider for CountingProvider {
        fn id(&self) -> &aifluxon_core::ProviderId {
            static ID: std::sync::OnceLock<aifluxon_core::ProviderId> = std::sync::OnceLock::new();
            ID.get_or_init(|| aifluxon_core::ProviderId::new("counting"))
        }

        fn capabilities(&self) -> ProviderCapabilities {
            ProviderCapabilities::openai_compatible()
        }

        async fn next_turn(
            &self,
            _request: ModelTurnRequest,
            _sink: Arc<dyn ModelEventSink>,
        ) -> Result<ModelTurn, ProviderError> {
            let mut remaining = self
                .remaining_tool_turns
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if *remaining == 0 {
                return Ok(ModelTurn {
                    text: "done".to_string(),
                    reasoning: String::new(),
                    tool_calls: Vec::new(),
                    usage: None,
                    terminal: aifluxon_core::ProviderTerminal::Stop,
                    opaque: serde_json::json!({ "thought_signature": "kept" }),
                });
            }
            *remaining -= 1;
            Ok(ModelTurn {
                text: String::new(),
                reasoning: String::new(),
                tool_calls: vec![ToolCall {
                    id: ToolInvocationId::from_stable_key("call-1"),
                    name: "read_file".to_string(),
                    arguments: serde_json::json!({}),
                }],
                usage: None,
                terminal: aifluxon_core::ProviderTerminal::ToolCalls,
                opaque: serde_json::json!({}),
            })
        }
    }

    #[test]
    fn bug_agent_014_budget_is_not_reset_after_provider_retry() {
        let coordinator = AgentCoordinator::new();
        coordinator.consume_model_round().unwrap();
        coordinator.consume_tool_invocation().unwrap();
        coordinator.consume_model_round().unwrap();
        assert!(coordinator.consume_model_round().is_ok());
        for _ in 0..61 {
            coordinator.consume_model_round().unwrap();
        }
        assert_eq!(
            coordinator.consume_model_round().unwrap_err().kind,
            AgentBudgetKind::ModelRounds
        );
        assert_eq!(
            coordinator.consume_model_round().unwrap_err().kind,
            AgentBudgetKind::ModelRounds
        );
    }

    #[tokio::test]
    async fn bug_agent_032_web_protocol_ledger_does_not_reexecute_runtime_tools() {
        let coordinator = AgentCoordinator::new();
        let invocations = AtomicUsize::new(0);
        let execute = |_call: &ToolCall| {
            invocations.fetch_add(1, Ordering::SeqCst);
            serde_json::json!({ "ok": true })
        };
        let provider = CountingProvider {
            remaining_tool_turns: Mutex::new(2),
        };
        let request = ModelTurnRequest {
            model: "test-model".to_string(),
            messages: Vec::new(),
            tools: Vec::new(),
            session_key: ProviderSessionKey::from_cache_session("session-a"),
            run_id: RunId::new(),
            opaque_state: None,
            features: Default::default(),
        };
        let turn = coordinator
            .run_model_tool_loop(&provider, request, execute)
            .await
            .unwrap();
        assert_eq!(turn.text, "done");
        assert_eq!(turn.opaque["thought_signature"], "kept");
        assert_eq!(invocations.load(Ordering::SeqCst), 1);
        assert_eq!(coordinator.execution_count(), 1);
    }

    struct ContinuationProvider {
        remaining: Mutex<u32>,
        session_keys: Mutex<Vec<String>>,
        run_ids: Mutex<Vec<RunId>>,
    }

    #[async_trait::async_trait]
    impl ModelProvider for ContinuationProvider {
        fn id(&self) -> &aifluxon_core::ProviderId {
            static ID: std::sync::OnceLock<aifluxon_core::ProviderId> = std::sync::OnceLock::new();
            ID.get_or_init(|| aifluxon_core::ProviderId::new("continuation"))
        }

        fn capabilities(&self) -> ProviderCapabilities {
            ProviderCapabilities::openai_compatible()
        }

        async fn next_turn(
            &self,
            request: ModelTurnRequest,
            _sink: Arc<dyn ModelEventSink>,
        ) -> Result<ModelTurn, ProviderError> {
            self.session_keys
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(request.session_key.as_str().to_string());
            self.run_ids
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(request.run_id);
            let mut remaining = self
                .remaining
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if *remaining == 0 {
                return Ok(ModelTurn {
                    text: "done".to_string(),
                    reasoning: String::new(),
                    tool_calls: Vec::new(),
                    usage: None,
                    terminal: ProviderTerminal::Stop,
                    opaque: serde_json::json!({ "cursor": "kept" }),
                });
            }
            *remaining -= 1;
            Ok(ModelTurn {
                text: "partial".to_string(),
                reasoning: String::new(),
                tool_calls: Vec::new(),
                usage: None,
                terminal: ProviderTerminal::Continue(
                    aifluxon_core::ContinuationReason::ProviderRequested,
                ),
                opaque: serde_json::json!({ "cursor": "kept" }),
            })
        }
    }

    fn continuation_request() -> ModelTurnRequest {
        ModelTurnRequest {
            model: "test-model".to_string(),
            messages: Vec::new(),
            tools: Vec::new(),
            session_key: ProviderSessionKey::from_cache_session("session-a"),
            run_id: RunId::new(),
            opaque_state: None,
            features: Default::default(),
        }
    }

    #[tokio::test]
    async fn continuation_runs_another_model_turn() {
        let coordinator = AgentCoordinator::new();
        let provider = ContinuationProvider {
            remaining: Mutex::new(1),
            session_keys: Mutex::new(Vec::new()),
            run_ids: Mutex::new(Vec::new()),
        };
        let turn = coordinator
            .run_model_tool_loop(
                &provider,
                continuation_request(),
                |_| serde_json::json!({ "ok": true }),
            )
            .await
            .unwrap();
        assert_eq!(turn.text, "done");
        assert_eq!(coordinator.model_rounds(), 2);
        assert_eq!(provider.session_keys.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn continuation_does_not_reset_model_budget() {
        let coordinator = AgentCoordinator::new();
        let provider = ContinuationProvider {
            remaining: Mutex::new(1),
            session_keys: Mutex::new(Vec::new()),
            run_ids: Mutex::new(Vec::new()),
        };
        coordinator
            .run_model_tool_loop(
                &provider,
                continuation_request(),
                |_| serde_json::json!({ "ok": true }),
            )
            .await
            .unwrap();
        assert_eq!(coordinator.model_rounds(), 2);
        for _ in 0..62 {
            coordinator.consume_model_round().unwrap();
        }
        assert_eq!(
            coordinator.consume_model_round().unwrap_err().kind,
            AgentBudgetKind::ModelRounds
        );
    }

    #[tokio::test]
    async fn continuation_preserves_run_identity_and_session_key() {
        let coordinator = AgentCoordinator::new();
        let provider = ContinuationProvider {
            remaining: Mutex::new(1),
            session_keys: Mutex::new(Vec::new()),
            run_ids: Mutex::new(Vec::new()),
        };
        coordinator
            .run_model_tool_loop(
                &provider,
                continuation_request(),
                |_| serde_json::json!({ "ok": true }),
            )
            .await
            .unwrap();
        let keys = provider.session_keys.lock().unwrap().clone();
        let runs = provider.run_ids.lock().unwrap().clone();
        assert_eq!(keys, vec!["session-a".to_string(), "session-a".to_string()]);
        assert_eq!(runs[0], runs[1]);
    }
}
