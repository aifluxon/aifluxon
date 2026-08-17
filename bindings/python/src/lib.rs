use aifluxon_api::{
    envelope_to_json, operation_snapshot_to_json, register_provider_from_json, user_prompt_request,
    Aifluxon, AifluxonError, AifluxonErrorKind, AllowAllToolPolicy, JsonFileProviderStateStore,
    JsonFileSessionStore, NoopRunEventSink, OperationDecision, OperationId, OperationMode,
    PendingOperationDraft, ProviderBinding, ProviderRegistry, RunHandle, RunId, RunLimits,
    RunSnapshot, SessionId, ToolDecision, ToolDescriptor, ToolEffect, ToolExecutionContext,
    ToolExecutionError, ToolExecutor, ToolInvocation, ToolPolicy, ToolPolicyInput, ToolRegistry,
    ToolResult,
};
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::PyAny;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

static RUNTIME: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("AIFLUXON Python runtime")
});

#[pyclass]
struct NativeAgent {
    inner: Aifluxon,
    binding: ProviderBinding,
    limits: RunLimits,
    tools: Arc<ToolRegistry>,
    runs: Mutex<HashMap<String, Arc<Mutex<RunHandle>>>>,
}

#[pymethods]
impl NativeAgent {
    #[new]
    #[pyo3(signature = (provider_spec, store_path=None, policy_callback=None, max_model_rounds=32, max_tool_invocations=64))]
    fn new(
        provider_spec: &str,
        store_path: Option<&str>,
        policy_callback: Option<Py<PyAny>>,
        max_model_rounds: u32,
        max_tool_invocations: u32,
    ) -> PyResult<Self> {
        let spec: Value = serde_json::from_str(provider_spec).map_err(invalid_config)?;
        let registry = ProviderRegistry::new();
        let binding = register_provider_from_json(&registry, &spec).map_err(map_error)?;
        let tools = Arc::new(ToolRegistry::new());
        let policy: Arc<dyn ToolPolicy> = match policy_callback {
            Some(callback) => Arc::new(PythonToolPolicy { callback }),
            None => Arc::new(AllowAllToolPolicy),
        };
        let mut builder = Aifluxon::builder()
            .provider_registry(registry)
            .tool_registry(tools.clone())
            .tool_policy(policy)
            .event_sink(Arc::new(NoopRunEventSink));
        if let Some(root) = store_path {
            builder = builder
                .session_store(Arc::new(
                    JsonFileSessionStore::new(root).map_err(store_error)?,
                ))
                .provider_state_store(Arc::new(
                    JsonFileProviderStateStore::new(root).map_err(store_error)?,
                ));
        }
        Ok(Self {
            inner: builder.build().map_err(map_error)?,
            binding,
            limits: RunLimits {
                max_model_rounds,
                max_tool_invocations,
            },
            tools,
            runs: Mutex::new(HashMap::new()),
        })
    }

    fn provider_id(&self) -> String {
        self.binding.provider_id.as_str().to_string()
    }

    fn model(&self) -> String {
        self.binding.model.clone()
    }

    fn register_tool(&self, descriptor_json: &str, callback: Py<PyAny>) -> PyResult<()> {
        let descriptor = parse_descriptor(descriptor_json)?;
        self.tools
            .register(descriptor, Arc::new(PythonToolExecutor { callback }))
            .map_err(|error| invalid_config(error.to_string()))
    }

    fn start(&self, py: Python<'_>, prompt: &str, session_id: Option<&str>) -> PyResult<String> {
        let inner = self.inner.clone();
        let provider = self.binding.provider_id.clone();
        let model = self.binding.model.clone();
        let limits = self.limits;
        let prompt = prompt.to_string();
        let session = session_id
            .map(SessionId::parse_or_stable_key)
            .transpose()
            .map_err(invalid_request)?;
        let handle = py
            .detach(|| {
                RUNTIME.block_on(inner.start(user_prompt_request(
                    provider, model, prompt, session, limits,
                )))
            })
            .map_err(map_error)?;
        let run_id = handle.id().hyphenated();
        self.runs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(run_id.clone(), Arc::new(Mutex::new(handle)));
        Ok(run_id)
    }

    fn next_event(&self, py: Python<'_>, run_id: &str) -> PyResult<Option<String>> {
        let handle = self.run_handle(run_id)?;
        let event = py.detach(|| {
            RUNTIME.block_on(async {
                handle
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .events()
                    .next()
                    .await
            })
        });
        Ok(event.map(|envelope| envelope_to_json(&envelope).to_string()))
    }

    fn cancel(&self, py: Python<'_>, run_id: &str) -> PyResult<()> {
        let inner = self.inner.clone();
        let parsed = RunId::parse(run_id).map_err(invalid_request)?;
        py.detach(|| RUNTIME.block_on(inner.cancel(parsed)))
            .map_err(map_error)
    }

    fn snapshot(&self, py: Python<'_>, run_id: &str) -> PyResult<String> {
        let inner = self.inner.clone();
        let parsed = RunId::parse(run_id).map_err(invalid_request)?;
        let snapshot = py
            .detach(|| RUNTIME.block_on(inner.snapshot(parsed)))
            .map_err(map_error)?;
        Ok(snapshot_to_json(&snapshot).to_string())
    }

    fn resolve_operation(
        &self,
        py: Python<'_>,
        run_id: &str,
        operation_id: &str,
        decision_json: &str,
    ) -> PyResult<()> {
        let inner = self.inner.clone();
        let run = RunId::parse(run_id).map_err(invalid_request)?;
        let operation = OperationId::parse(operation_id).map_err(invalid_request)?;
        let decision = parse_decision(decision_json)?;
        py.detach(|| RUNTIME.block_on(inner.resolve_operation(run, operation, decision)))
            .map_err(map_error)
    }

    fn commit_operation(&self, py: Python<'_>, run_id: &str, operation_id: &str) -> PyResult<()> {
        let inner = self.inner.clone();
        let run = RunId::parse(run_id).map_err(invalid_request)?;
        let operation = OperationId::parse(operation_id).map_err(invalid_request)?;
        py.detach(|| RUNTIME.block_on(inner.commit_prepared_operation(run, operation)))
            .map_err(map_error)
    }

    fn create_session(&self, py: Python<'_>) -> PyResult<String> {
        let inner = self.inner.clone();
        let id = py
            .detach(|| RUNTIME.block_on(inner.create_session()))
            .map_err(map_error)?;
        Ok(id.hyphenated())
    }

    fn open_session(&self, py: Python<'_>, session_id: &str) -> PyResult<String> {
        let inner = self.inner.clone();
        let id = SessionId::parse_or_stable_key(session_id).map_err(invalid_request)?;
        let record = py
            .detach(|| RUNTIME.block_on(inner.open_session(id)))
            .map_err(map_error)?;
        Ok(session_record_json(&record).to_string())
    }

    fn open_or_create_session(&self, py: Python<'_>, session_id: &str) -> PyResult<String> {
        let inner = self.inner.clone();
        let id = SessionId::parse_or_stable_key(session_id).map_err(invalid_request)?;
        let record = py
            .detach(|| RUNTIME.block_on(inner.open_or_create_session(id)))
            .map_err(map_error)?;
        Ok(session_record_json(&record).to_string())
    }

    fn list_sessions(&self, py: Python<'_>) -> PyResult<String> {
        let inner = self.inner.clone();
        let sessions = py
            .detach(|| RUNTIME.block_on(inner.list_sessions()))
            .map_err(map_error)?;
        Ok(json!(sessions
            .iter()
            .map(|summary| json!({
                "id": summary.id.hyphenated(),
                "revision": summary.revision,
                "created_at": summary.created_at,
                "updated_at": summary.updated_at,
                "message_count": summary.message_count,
            }))
            .collect::<Vec<_>>())
        .to_string())
    }

    fn delete_session(&self, py: Python<'_>, session_id: &str) -> PyResult<()> {
        let inner = self.inner.clone();
        let id = SessionId::parse_or_stable_key(session_id).map_err(invalid_request)?;
        py.detach(|| RUNTIME.block_on(inner.delete_session(id)))
            .map_err(map_error)
    }
}

impl NativeAgent {
    fn run_handle(&self, run_id: &str) -> PyResult<Arc<Mutex<RunHandle>>> {
        self.runs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(run_id)
            .cloned()
            .ok_or_else(|| invalid_request(format!("Unknown run `{run_id}`.")))
    }
}

struct PythonToolExecutor {
    callback: Py<PyAny>,
}

#[async_trait::async_trait]
impl ToolExecutor for PythonToolExecutor {
    async fn execute(
        &self,
        invocation: ToolInvocation,
        _ctx: ToolExecutionContext,
    ) -> Result<ToolResult, ToolExecutionError> {
        let callback = Python::attach(|py| self.callback.clone_ref(py));
        let payload = invocation.arguments.to_string();
        let joined = tokio::task::spawn_blocking(move || {
            Python::attach(|py| {
                callback
                    .bind(py)
                    .call1((payload,))
                    .and_then(|value| value.extract::<String>())
                    .map_err(|error| error.to_string())
            })
        })
        .await
        .map_err(|error| ToolExecutionError::Failed(error.to_string()))?;
        let text = joined.map_err(ToolExecutionError::Failed)?;
        let value = serde_json::from_str(&text).unwrap_or_else(|_| json!({ "result": text }));
        Ok(ToolResult { value })
    }
}

struct PythonToolPolicy {
    callback: Py<PyAny>,
}

impl ToolPolicy for PythonToolPolicy {
    fn evaluate(&self, input: &ToolPolicyInput<'_>) -> ToolDecision {
        let result = Python::attach(|py| {
            self.callback
                .bind(py)
                .call1((
                    input.descriptor.name.as_str(),
                    input.invocation.arguments.to_string(),
                    effect_name(input.descriptor.effect),
                ))
                .and_then(|value| value.extract::<String>())
                .map_err(|error| error.to_string())
        });
        match result.and_then(parse_policy_decision) {
            Ok(decision) => decision,
            Err(message) => ToolDecision::Deny { reason: message },
        }
    }
}

fn parse_policy_decision(raw: String) -> Result<ToolDecision, String> {
    let value: Value = serde_json::from_str(&raw).map_err(|error| error.to_string())?;
    match value
        .get("decision")
        .and_then(Value::as_str)
        .unwrap_or("deny")
    {
        "allow" => Ok(ToolDecision::Allow),
        "deny" => Ok(ToolDecision::Deny {
            reason: value
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("The tool policy denied this invocation.")
                .to_string(),
        }),
        "require_approval" => Ok(ToolDecision::RequireApproval {
            operation: PendingOperationDraft {
                invocation_id: None,
                effect: ToolEffect::Unknown,
                mode: match value.get("mode").and_then(Value::as_str) {
                    Some("deferred_commit") => OperationMode::DeferredCommit,
                    _ => OperationMode::BlockingApproval,
                },
                summary: value
                    .get("summary")
                    .and_then(Value::as_str)
                    .unwrap_or("Tool invocation requires host approval.")
                    .to_string(),
                payload: value.get("payload").cloned().unwrap_or(Value::Null),
                deadline: None,
            },
        }),
        other => Err(format!("Unknown tool policy decision `{other}`.")),
    }
}

fn parse_descriptor(raw: &str) -> PyResult<ToolDescriptor> {
    let value: Value = serde_json::from_str(raw).map_err(invalid_config)?;
    Ok(ToolDescriptor {
        name: required_str(&value, "name")?,
        description: value
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        input_schema: value
            .get("input_schema")
            .cloned()
            .unwrap_or_else(|| json!({ "type": "object" })),
        effect: parse_effect(value.get("effect").and_then(Value::as_str))?,
        required_capabilities: value
            .get("required_capabilities")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(aifluxon_api::CapabilityId::new)
                    .collect()
            })
            .unwrap_or_default(),
        parallel_safe: value
            .get("parallel_safe")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

fn parse_effect(raw: Option<&str>) -> PyResult<ToolEffect> {
    Ok(match raw.unwrap_or("unknown") {
        "pure_read" => ToolEffect::PureRead,
        "fs_read" => ToolEffect::FsRead,
        "fs_write" => ToolEffect::FsWrite,
        "process_spawn" => ToolEffect::ProcessSpawn,
        "process_control" => ToolEffect::ProcessControl,
        "network" => ToolEffect::Network,
        "settings_write" => ToolEffect::SettingsWrite,
        "external_side_effect" => ToolEffect::ExternalSideEffect,
        "unknown" => ToolEffect::Unknown,
        other => {
            return Err(invalid_config(format!(
                "Unsupported tool effect `{other}`."
            )))
        }
    })
}

fn effect_name(effect: ToolEffect) -> &'static str {
    match effect {
        ToolEffect::PureRead => "pure_read",
        ToolEffect::FsRead => "fs_read",
        ToolEffect::FsWrite => "fs_write",
        ToolEffect::ProcessSpawn => "process_spawn",
        ToolEffect::ProcessControl => "process_control",
        ToolEffect::Network => "network",
        ToolEffect::SettingsWrite => "settings_write",
        ToolEffect::ExternalSideEffect => "external_side_effect",
        ToolEffect::Unknown => "unknown",
    }
}

fn parse_decision(raw: &str) -> PyResult<OperationDecision> {
    let value: Value = serde_json::from_str(raw).map_err(invalid_request)?;
    match value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("approve")
    {
        "approve" => Ok(OperationDecision::Approve {
            data: value.get("data").cloned(),
        }),
        "reject" => Ok(OperationDecision::Reject {
            reason: value
                .get("reason")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
        }),
        other => Err(invalid_request(format!(
            "Unsupported operation decision `{other}`."
        ))),
    }
}

fn snapshot_to_json(snapshot: &RunSnapshot) -> Value {
    json!({
        "run_id": snapshot.context.run_id.hyphenated(),
        "session_id": snapshot.context.session_id.map(|id| id.hyphenated()),
        "state": format!("{:?}", snapshot.state).to_ascii_lowercase(),
        "last_event_sequence": snapshot.last_event_sequence,
        "pending_operations": snapshot
            .pending_operations
            .iter()
            .map(operation_snapshot_to_json)
            .collect::<Vec<_>>(),
    })
}

fn session_record_json(record: &aifluxon_api::SessionRecord) -> Value {
    json!({
        "id": record.id.hyphenated(),
        "revision": record.revision,
        "created_at": record.created_at,
        "updated_at": record.updated_at,
        "message_count": record.messages.len(),
    })
}

fn required_str(value: &Value, field: &str) -> PyResult<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| invalid_config(format!("Missing `{field}`.")))
}

fn map_error(error: AifluxonError) -> PyErr {
    native_error(error.kind(), error.message())
}

fn store_error(error: aifluxon_api::StoreError) -> PyErr {
    let kind = match error {
        aifluxon_api::StoreError::Conflict => AifluxonErrorKind::StateConflict,
        aifluxon_api::StoreError::InvalidId => AifluxonErrorKind::InvalidRequest,
        _ => AifluxonErrorKind::Internal,
    };
    native_error(kind, error.to_string())
}

fn native_error(kind: AifluxonErrorKind, message: impl Into<String>) -> PyErr {
    PyRuntimeError::new_err(
        json!({
            "kind": format!("{kind:?}"),
            "message": message.into(),
        })
        .to_string(),
    )
}

fn invalid_config(error: impl ToString) -> PyErr {
    native_error(AifluxonErrorKind::InvalidConfiguration, error.to_string())
}

fn invalid_request(error: impl ToString) -> PyErr {
    native_error(AifluxonErrorKind::InvalidRequest, error.to_string())
}

#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<NativeAgent>()?;
    Ok(())
}
