use crate::{
    AifluxonBuilder, OperationDecision, OperationId, ProviderRegistry, RunEventSink, RunHandle,
    RunId, RunRequest, RunSnapshot, SessionSummary, ToolPolicy, ToolRegistry, Workspace,
};
use aifluxon_core::{
    AgentFilesystemScope, ExecutionAuthority, ModelEventSink, ModelTurnRequest, PermissionMode,
    ProviderSessionKey, RunEvent, RunState, SessionId,
};
use aifluxon_runtime::{
    now_millis, ProviderStateRecord, ProviderStateStore, RunCheckpoint, RunCheckpointStore,
    SessionRecord, SessionStore,
};
use aifluxon_runtime::{AgentCoordinator, RunTable, RunTableError};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AifluxonErrorKind {
    InvalidConfiguration,
    InvalidRequest,
    Provider,
    Tool,
    PolicyDenied,
    OperationPending,
    OperationRejected,
    Cancelled,
    BudgetExceeded,
    StateConflict,
    RuntimeUnavailable,
    Failed,
    Internal,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("{message}")]
pub struct AifluxonError {
    kind: AifluxonErrorKind,
    message: String,
}

impl AifluxonError {
    pub fn new(kind: AifluxonErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> AifluxonErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

pub(crate) struct BackendConfiguration {
    pub provider_registry: Arc<ProviderRegistry>,
    pub tool_registry: Arc<ToolRegistry>,
    pub tool_policy: Arc<dyn ToolPolicy>,
    pub event_sink: Arc<dyn RunEventSink>,
    pub workspace: Arc<dyn Workspace>,
    pub run_table: RunTable,
    pub session_store: Arc<dyn SessionStore>,
    pub provider_state_store: Arc<dyn ProviderStateStore>,
    pub checkpoint_store: Arc<dyn RunCheckpointStore>,
}

#[derive(Clone)]
pub struct Aifluxon {
    configuration: Arc<BackendConfiguration>,
}

impl Aifluxon {
    pub fn builder() -> AifluxonBuilder {
        AifluxonBuilder::new()
    }

    pub(crate) fn from_configuration(configuration: BackendConfiguration) -> Self {
        Self {
            configuration: Arc::new(configuration),
        }
    }

    pub async fn start(&self, request: RunRequest) -> Result<RunHandle, AifluxonError> {
        self.launch(None, request).await
    }

    /// Attaches a canonical run that the Host already registered on a shared `RunTable`.
    pub async fn start_on(
        &self,
        run_id: RunId,
        request: RunRequest,
    ) -> Result<RunHandle, AifluxonError> {
        self.launch(Some(run_id), request).await
    }

    async fn launch(
        &self,
        existing_run: Option<RunId>,
        request: RunRequest,
    ) -> Result<RunHandle, AifluxonError> {
        self.validate_request(&request)?;
        let provider = self
            .configuration
            .provider_registry
            .resolve_required(&request.model.provider)
            .map_err(|error| AifluxonError::new(AifluxonErrorKind::Provider, error.to_string()))?;
        let session_id = request.session_id;
        let mut session_record = match session_id {
            Some(session_id) => self
                .configuration
                .session_store
                .load(&session_id)
                .await
                .map_err(map_store_error)?
                .unwrap_or_else(|| SessionRecord::new(session_id)),
            None => SessionRecord::new(SessionId::new()),
        };
        let context = match existing_run {
            Some(run_id) => self
                .configuration
                .run_table
                .context(&run_id)
                .map_err(map_runtime_error)?,
            None => self
                .configuration
                .run_table
                .start(session_id, None, request.limits)
                .map_err(map_runtime_error)?,
        };
        let (event_sender, event_receiver) = tokio::sync::mpsc::channel(128);
        spawn_event_forwarder(
            self.configuration.run_table.clone(),
            context.run_id,
            event_sender,
            self.configuration.event_sink.clone(),
        );

        let tools = selected_tools(
            &self.configuration.tool_registry,
            request.allowed_tools.as_ref(),
        );
        let mut capabilities = HashSet::new();
        for descriptor in &tools {
            for capability in &descriptor.required_capabilities {
                if self.configuration.workspace.supports(capability) {
                    capabilities.insert(capability.clone());
                }
            }
        }
        let mut authority = request.authority.unwrap_or(ExecutionAuthority {
            permission_mode: PermissionMode::Default,
            filesystem_scope: AgentFilesystemScope::Isolated,
            capabilities: HashSet::new(),
        });
        authority.capabilities.extend(capabilities);
        let session_key = request.session_key.unwrap_or_else(|| {
            ProviderSessionKey::from_session_id(&session_id.unwrap_or(session_record.id))
        });
        let canonical_messages = crate::prompt::merge_session_and_request_messages(
            if session_id.is_some() {
                session_record.messages.clone()
            } else {
                Vec::new()
            },
            request.messages,
        );
        let opaque_state = match session_id {
            Some(session_id) => self
                .configuration
                .provider_state_store
                .load(&session_id, &request.model.provider)
                .await
                .map_err(map_store_error)?
                .map(|record| record.state),
            None => None,
        };
        let model_request = ModelTurnRequest {
            model: request.model.model,
            messages: canonical_messages.clone(),
            tools,
            session_key,
            run_id: context.run_id,
            opaque_state,
            features: request.features.clone(),
        };
        let table = self.configuration.run_table.clone();
        let registry = self.configuration.tool_registry.clone();
        let policy = self.configuration.tool_policy.clone();
        let session_store = self.configuration.session_store.clone();
        let provider_state_store = self.configuration.provider_state_store.clone();
        let checkpoint_store = self.configuration.checkpoint_store.clone();
        let provider_id = request.model.provider;
        checkpoint_store
            .save(RunCheckpoint {
                run_id: context.run_id,
                session_id,
                state: RunState::Running,
                messages: canonical_messages,
                provider_state: model_request.opaque_state.clone(),
                updated_at: now_millis(),
            })
            .await
            .map_err(map_store_error)?;
        let failure_messages = model_request.messages.clone();
        let failure_provider_state = model_request.opaque_state.clone();
        tokio::spawn(async move {
            let coordinator = AgentCoordinator::for_run(table.clone(), context.run_id);
            let sink: Arc<dyn ModelEventSink> = Arc::new(RuntimeModelSink {
                table: table.clone(),
                run_id: context.run_id,
            });
            match coordinator
                .run_registered_model_tool_loop(
                    provider,
                    registry,
                    policy,
                    authority,
                    model_request,
                    sink,
                )
                .await
            {
                Ok(result) => {
                    let persistence = async {
                        if let Some(session_id) = session_id {
                            session_record.messages = result.messages.clone();
                            session_store.save(session_record).await?;
                            if !result.turn.opaque.is_null() {
                                provider_state_store
                                    .save(ProviderStateRecord {
                                        session_id,
                                        provider_id,
                                        state: result.turn.opaque.clone(),
                                    })
                                    .await?;
                            }
                        }
                        checkpoint_store
                            .save(RunCheckpoint {
                                run_id: context.run_id,
                                session_id,
                                state: RunState::Completed,
                                messages: result.messages.clone(),
                                provider_state: (!result.turn.opaque.is_null())
                                    .then(|| result.turn.opaque.clone()),
                                updated_at: now_millis(),
                            })
                            .await?;
                        Ok::<(), aifluxon_runtime::StoreError>(())
                    }
                    .await;
                    match persistence {
                        Ok(()) => {
                            let _ = table.finish_completed(&context.run_id, result.messages);
                        }
                        Err(error) => {
                            let _ = table.finish_failed(&context.run_id, error.to_string());
                        }
                    }
                }
                Err(error) => {
                    let state = table
                        .snapshot(&context.run_id)
                        .map(|snapshot| snapshot.state)
                        .unwrap_or(RunState::Failed);
                    let checkpoint_state = if state == RunState::Cancelled {
                        RunState::Cancelled
                    } else {
                        RunState::Failed
                    };
                    let checkpoint_result = checkpoint_store
                        .save(RunCheckpoint {
                            run_id: context.run_id,
                            session_id,
                            state: checkpoint_state,
                            messages: failure_messages,
                            provider_state: failure_provider_state,
                            updated_at: now_millis(),
                        })
                        .await;
                    if state != RunState::Cancelled {
                        let failure = match checkpoint_result {
                            Ok(()) => error.to_string(),
                            Err(store_error) => format!(
                                "{error}; additionally failed to persist the run checkpoint: {store_error}"
                            ),
                        };
                        let _ = table.finish_failed(&context.run_id, failure);
                    }
                }
            }
        });
        Ok(RunHandle::new(
            context,
            crate::RunEventStream::from_receiver(event_receiver),
        ))
    }

    pub async fn cancel(&self, run_id: RunId) -> Result<(), AifluxonError> {
        self.configuration
            .run_table
            .cancel(&run_id)
            .map(|_| ())
            .map_err(map_runtime_error)
    }

    pub async fn snapshot(&self, run_id: RunId) -> Result<RunSnapshot, AifluxonError> {
        let snapshot = self
            .configuration
            .run_table
            .snapshot(&run_id)
            .map_err(map_runtime_error)?;
        Ok(RunSnapshot {
            context: snapshot.context,
            state: snapshot.state,
            last_event_sequence: snapshot.last_event_sequence,
            pending_operations: snapshot.pending_operations,
        })
    }

    pub async fn resolve_operation(
        &self,
        run_id: RunId,
        operation_id: OperationId,
        decision: OperationDecision,
    ) -> Result<(), AifluxonError> {
        self.configuration
            .run_table
            .operations()
            .resolve(&run_id, &operation_id, decision)
            .map_err(|error| AifluxonError::new(AifluxonErrorKind::StateConflict, error.message()))
    }

    pub async fn commit_prepared_operation(
        &self,
        run_id: RunId,
        operation_id: OperationId,
    ) -> Result<(), AifluxonError> {
        self.configuration
            .run_table
            .operations()
            .begin_commit(&run_id, &operation_id)
            .map_err(|error| AifluxonError::new(AifluxonErrorKind::StateConflict, error.message()))
    }

    pub async fn create_session(&self) -> Result<SessionId, AifluxonError> {
        let saved = self
            .configuration
            .session_store
            .save(SessionRecord::new(SessionId::new()))
            .await
            .map_err(map_store_error)?;
        Ok(saved.id)
    }

    pub async fn open_session(
        &self,
        session_id: SessionId,
    ) -> Result<aifluxon_runtime::SessionRecord, AifluxonError> {
        self.configuration
            .session_store
            .load(&session_id)
            .await
            .map_err(map_store_error)?
            .ok_or_else(|| {
                AifluxonError::new(
                    AifluxonErrorKind::InvalidRequest,
                    format!("Session `{}` does not exist.", session_id.hyphenated()),
                )
            })
    }

    pub async fn open_or_create_session(
        &self,
        session_id: SessionId,
    ) -> Result<aifluxon_runtime::SessionRecord, AifluxonError> {
        match self
            .configuration
            .session_store
            .load(&session_id)
            .await
            .map_err(map_store_error)?
        {
            Some(record) => Ok(record),
            None => self
                .configuration
                .session_store
                .save(SessionRecord::new(session_id))
                .await
                .map_err(map_store_error),
        }
    }

    pub async fn list_sessions(&self) -> Result<Vec<SessionSummary>, AifluxonError> {
        self.configuration
            .session_store
            .list()
            .await
            .map_err(map_store_error)
    }

    pub async fn delete_session(&self, session_id: SessionId) -> Result<(), AifluxonError> {
        self.configuration
            .session_store
            .delete(&session_id)
            .await
            .map_err(map_store_error)
    }

    fn validate_request(&self, request: &RunRequest) -> Result<(), AifluxonError> {
        if request.model.provider.as_str().trim().is_empty()
            || request.model.model.trim().is_empty()
        {
            return Err(AifluxonError::new(
                AifluxonErrorKind::InvalidRequest,
                "Run requests require a provider identifier and model name.",
            ));
        }
        if !self
            .configuration
            .provider_registry
            .contains(&request.model.provider)
        {
            return Err(AifluxonError::new(
                AifluxonErrorKind::Provider,
                format!(
                    "Provider `{}` is not registered.",
                    request.model.provider.as_str()
                ),
            ));
        }
        if let Some(allowed_tools) = &request.allowed_tools {
            if let Some(unknown) = allowed_tools
                .iter()
                .find(|name| !self.configuration.tool_registry.contains(name))
            {
                return Err(AifluxonError::new(
                    AifluxonErrorKind::Tool,
                    format!("Tool `{unknown}` is not registered."),
                ));
            }
        }
        Ok(())
    }
}

struct RuntimeModelSink {
    table: RunTable,
    run_id: RunId,
}

impl ModelEventSink for RuntimeModelSink {
    fn on_text_delta(&self, delta: &str) {
        let _ = self.table.emit(
            &self.run_id,
            RunEvent::ModelDelta {
                delta: delta.to_string(),
            },
        );
    }

    fn on_reasoning_delta(&self, delta: &str) {
        let _ = self.table.emit(
            &self.run_id,
            RunEvent::ReasoningDelta {
                delta: delta.to_string(),
            },
        );
    }

    fn on_usage(&self, usage: &serde_json::Value) {
        let _ = self.table.emit(
            &self.run_id,
            RunEvent::UsageUpdated {
                usage: usage.clone(),
            },
        );
    }
}

fn selected_tools(
    registry: &ToolRegistry,
    allowed: Option<&Vec<String>>,
) -> Vec<aifluxon_core::ToolDescriptor> {
    match allowed {
        Some(allowed) => allowed
            .iter()
            .filter_map(|name| registry.resolve(name).map(|tool| tool.descriptor().clone()))
            .collect(),
        None => registry.descriptors(),
    }
}

fn spawn_event_forwarder(
    table: RunTable,
    run_id: RunId,
    sender: tokio::sync::mpsc::Sender<aifluxon_core::RunEventEnvelope>,
    sink: Arc<dyn RunEventSink>,
) {
    tokio::spawn(async move {
        let mut sequence = 0;
        loop {
            let notify = table.event_notify();
            let notified = notify.notified();
            let events = match table.events_since(&run_id, sequence) {
                Ok(events) => events,
                Err(_) => return,
            };
            if events.is_empty() {
                tokio::select! {
                    _ = notified => {}
                    _ = tokio::time::sleep(Duration::from_millis(5)) => {}
                }
                continue;
            }
            for event in events {
                sequence = event.sequence;
                sink.emit(event.clone());
                let terminal = event.event.terminal_state().is_some();
                if sender.send(event).await.is_err() || terminal {
                    return;
                }
                tokio::task::yield_now().await;
            }
        }
    });
}

fn map_runtime_error(error: RunTableError) -> AifluxonError {
    let kind = match error {
        RunTableError::UnknownRun => AifluxonErrorKind::InvalidRequest,
        RunTableError::RunTerminal => AifluxonErrorKind::StateConflict,
        _ => AifluxonErrorKind::Internal,
    };
    AifluxonError::new(kind, error.message())
}

fn map_store_error(error: aifluxon_runtime::StoreError) -> AifluxonError {
    let kind = match error {
        aifluxon_runtime::StoreError::Conflict => AifluxonErrorKind::StateConflict,
        aifluxon_runtime::StoreError::InvalidId => AifluxonErrorKind::InvalidRequest,
        _ => AifluxonErrorKind::Internal,
    };
    AifluxonError::new(kind, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NoopRunEventSink;
    use aifluxon_core::{
        ContentPart, Message, MessageRole, ModelProvider, ModelRef, ModelTurn, ModelTurnRequest,
        ProviderCapabilities, ProviderError, ProviderId, ProviderTerminal, RunLimits,
    };
    use aifluxon_providers::OpenAiCompatibleProvider;
    use aifluxon_runtime::{InMemoryProviderStateStore, JsonFileSessionStore};
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestPolicy;
    impl ToolPolicy for TestPolicy {
        fn evaluate(
            &self,
            _input: &aifluxon_runtime::ToolPolicyInput<'_>,
        ) -> aifluxon_runtime::ToolDecision {
            aifluxon_runtime::ToolDecision::Allow
        }
    }

    fn request(allowed_tools: Option<Vec<String>>) -> RunRequest {
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
                provider: ProviderId::new("controlled_provider"),
                model: "controlled-model".to_string(),
            },
            session_key: None,
            allowed_tools,
            limits: RunLimits {
                max_model_rounds: 4,
                max_tool_invocations: 4,
            },
            features: Default::default(),
            authority: None,
        }
    }

    fn facade(registry: ProviderRegistry) -> Aifluxon {
        Aifluxon::builder()
            .provider_registry(registry)
            .tool_registry(ToolRegistry::new())
            .tool_policy(Arc::new(TestPolicy))
            .event_sink(Arc::new(NoopRunEventSink))
            .build()
            .unwrap()
    }

    #[derive(Clone)]
    struct RecordingProvider {
        id: ProviderId,
        requests: Arc<Mutex<Vec<ModelTurnRequest>>>,
    }

    #[async_trait::async_trait]
    impl ModelProvider for RecordingProvider {
        fn id(&self) -> &ProviderId {
            &self.id
        }

        fn capabilities(&self) -> ProviderCapabilities {
            ProviderCapabilities::openai_compatible()
        }

        async fn next_turn(
            &self,
            request: ModelTurnRequest,
            _sink: Arc<dyn ModelEventSink>,
        ) -> Result<ModelTurn, ProviderError> {
            let next_cursor = request
                .opaque_state
                .as_ref()
                .and_then(|state| state.get("cursor"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0)
                + 1;
            self.requests
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(request);
            Ok(ModelTurn {
                text: format!("response-{next_cursor}"),
                reasoning: String::new(),
                tool_calls: Vec::new(),
                usage: None,
                terminal: ProviderTerminal::Stop,
                opaque: serde_json::json!({ "cursor": next_cursor }),
            })
        }
    }

    fn temp_root(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "aifluxon-api-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    async fn wait_for_terminal(handle: &mut RunHandle) {
        while let Some(event) = handle.events().next().await {
            if matches!(
                event.event,
                RunEvent::Completed { .. } | RunEvent::Failed { .. } | RunEvent::Cancelled { .. }
            ) {
                return;
            }
        }
        panic!("run event stream closed before a terminal event");
    }

    #[tokio::test]
    async fn start_rejects_unregistered_provider_before_runtime_handoff() {
        let error = facade(ProviderRegistry::new())
            .start(request(None))
            .await
            .unwrap_err();
        assert_eq!(error.kind(), AifluxonErrorKind::Provider);
    }

    #[tokio::test]
    async fn start_attaches_the_single_runtime_owner_and_streams_terminal_failure() {
        let registry = ProviderRegistry::new();
        registry
            .register(
                ProviderId::new("controlled_provider"),
                OpenAiCompatibleProvider::new("controlled_provider"),
            )
            .unwrap();
        let mut handle = facade(registry).start(request(None)).await.unwrap();
        let started = handle.events().next().await.unwrap();
        assert!(matches!(started.event, RunEvent::RunStarted { .. }));
        let terminal = handle.events().next().await.unwrap();
        assert!(matches!(terminal.event, RunEvent::Failed { .. }));
    }

    #[tokio::test]
    async fn session_messages_restore_across_facades_and_provider_state_follows_session() {
        let root = temp_root("persistent-session");
        let session_id = SessionId::new();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let provider_state_store = Arc::new(InMemoryProviderStateStore::default());

        for turn in 0..2 {
            let registry = ProviderRegistry::new();
            registry
                .register(
                    ProviderId::new("controlled_provider"),
                    RecordingProvider {
                        id: ProviderId::new("controlled_provider"),
                        requests: requests.clone(),
                    },
                )
                .unwrap();
            let session_store = Arc::new(JsonFileSessionStore::new(&root).unwrap());
            let backend = Aifluxon::builder()
                .provider_registry(registry)
                .tool_registry(ToolRegistry::new())
                .tool_policy(Arc::new(TestPolicy))
                .event_sink(Arc::new(NoopRunEventSink))
                .session_store(session_store)
                .provider_state_store(provider_state_store.clone())
                .build()
                .unwrap();
            let mut run_request = request(None);
            run_request.session_id = Some(session_id);
            run_request.messages[0].content = vec![ContentPart::Text(format!("turn-{turn}"))];
            let mut handle = backend.start(run_request).await.unwrap();
            wait_for_terminal(&mut handle).await;
        }

        let recorded = requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(recorded.len(), 2);
        assert_eq!(recorded[0].messages.len(), 1);
        assert_eq!(recorded[1].messages.len(), 3);
        assert_eq!(
            recorded[1].opaque_state,
            Some(serde_json::json!({ "cursor": 1 }))
        );
        assert_ne!(recorded[0].run_id, recorded[1].run_id);
        assert_eq!(
            recorded[0].session_key.as_str(),
            recorded[1].session_key.as_str()
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn result_returns_canonical_completed_output_without_summing_deltas() {
        let registry = ProviderRegistry::new();
        registry
            .register(
                ProviderId::new("controlled"),
                crate::ControlledProvider::from_text_responses("controlled", ["canonical-answer"]),
            )
            .unwrap();
        let backend = facade(registry);
        let mut handle = backend
            .start(crate::user_prompt_request(
                ProviderId::new("controlled"),
                "controlled-model",
                "hello",
                None,
                RunLimits::default(),
            ))
            .await
            .unwrap();
        let result = handle.result().await.unwrap();
        assert_eq!(result.state, RunState::Completed);
        assert_eq!(result.text, "canonical-answer");
        assert_eq!(result.run_id, handle.id());
    }

    #[tokio::test]
    async fn session_crud_creates_distinct_runs_under_one_session() {
        let registry = ProviderRegistry::new();
        registry
            .register(
                ProviderId::new("controlled"),
                crate::ControlledProvider::from_text_responses("controlled", ["one", "two"]),
            )
            .unwrap();
        let backend = facade(registry);
        let session_id = backend.create_session().await.unwrap();
        let opened = backend.open_session(session_id).await.unwrap();
        assert_eq!(opened.id, session_id);
        let listed = backend.list_sessions().await.unwrap();
        assert_eq!(listed.len(), 1);

        let mut first = backend
            .start(crate::user_prompt_request(
                ProviderId::new("controlled"),
                "controlled-model",
                "first",
                Some(session_id),
                RunLimits::default(),
            ))
            .await
            .unwrap();
        first.result().await.unwrap();
        let mut second = backend
            .start(crate::user_prompt_request(
                ProviderId::new("controlled"),
                "controlled-model",
                "second",
                Some(session_id),
                RunLimits::default(),
            ))
            .await
            .unwrap();
        second.result().await.unwrap();
        assert_ne!(first.id(), second.id());
        assert_eq!(first.context().session_id, Some(session_id));
        assert_eq!(second.context().session_id, Some(session_id));
        backend.delete_session(session_id).await.unwrap();
        assert!(backend.list_sessions().await.unwrap().is_empty());
    }
}
