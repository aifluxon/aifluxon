use aifluxon_api::{
    envelope_to_json, operation_snapshot_to_json, register_provider_from_json,
    unlock_encrypted_store, user_content_request_with_system, Aifluxon, AifluxonAuthError,
    AifluxonError, AifluxonErrorKind, AllowAllToolPolicy, CodexAuth, CodexLoginAttempt,
    CodexProviderHandle, ContentPart, EncryptedFileSecretStore, ImageContent,
    JsonFileProviderStateStore, JsonFileSessionStore, MemorySecretStore, NoopRunEventSink,
    OperationDecision, OperationId, OperationMode, PendingOperationDraft, ProviderBinding,
    ProviderFeatureRequest, ProviderRegistry, RunHandle, RunId, RunLimits, RunSnapshot, SessionId,
    SystemKeyringStore, ToolDecision, ToolDescriptor, ToolEffect, ToolExecutionContext,
    ToolExecutionError, ToolExecutor, ToolInvocation, ToolPolicy, ToolPolicyInput, ToolRegistry,
    ToolResult, DEFAULT_SERVICE_NAME,
};
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::PyAny;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use tokio::sync::Mutex as AsyncMutex;

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
    runs: Mutex<HashMap<String, Arc<AsyncMutex<RunHandle>>>>,
}

#[pymethods]
impl NativeAgent {
    #[new]
    #[pyo3(signature = (provider_spec, store_path=None, policy_callback=None, max_model_rounds=32, max_tool_invocations=64, oauth_provider=None))]
    fn new(
        provider_spec: &str,
        store_path: Option<&str>,
        policy_callback: Option<Py<PyAny>>,
        max_model_rounds: u32,
        max_tool_invocations: u32,
        oauth_provider: Option<&NativeCodexProvider>,
    ) -> PyResult<Self> {
        let registry = ProviderRegistry::new();
        let binding = if let Some(oauth) = oauth_provider {
            oauth.inner.register(&registry).map_err(map_error)?
        } else {
            let spec: Value = serde_json::from_str(provider_spec).map_err(invalid_config)?;
            register_provider_from_json(&registry, &spec).map_err(map_error)?
        };
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

    #[pyo3(signature = (prompt, session_id=None, features_json=None, system_prompt=None))]
    fn start(
        &self,
        py: Python<'_>,
        prompt: &str,
        session_id: Option<&str>,
        features_json: Option<&str>,
        system_prompt: Option<&str>,
    ) -> PyResult<String> {
        let prompt = prompt.to_string();
        self.start_content(
            py,
            vec![ContentPart::Text(prompt)],
            session_id,
            features_json,
            system_prompt,
        )
    }

    #[pyo3(signature = (content_json, session_id=None, features_json=None, system_prompt=None))]
    fn start_with_content(
        &self,
        py: Python<'_>,
        content_json: &str,
        session_id: Option<&str>,
        features_json: Option<&str>,
        system_prompt: Option<&str>,
    ) -> PyResult<String> {
        self.start_content(
            py,
            parse_content_parts(content_json)?,
            session_id,
            features_json,
            system_prompt,
        )
    }

    fn next_event(&self, py: Python<'_>, run_id: &str) -> PyResult<Option<String>> {
        let handle = self.run_handle(run_id)?;
        let event = py.detach(|| {
            RUNTIME.block_on(async {
                handle
                    .lock()
                    .await
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
    fn start_content(
        &self,
        py: Python<'_>,
        content: Vec<ContentPart>,
        session_id: Option<&str>,
        features_json: Option<&str>,
        system_prompt: Option<&str>,
    ) -> PyResult<String> {
        let inner = self.inner.clone();
        let provider = self.binding.provider_id.clone();
        let model = self.binding.model.clone();
        let limits = self.limits;
        let session = session_id
            .map(SessionId::parse_or_stable_key)
            .transpose()
            .map_err(invalid_request)?;
        let features = parse_features(features_json)?;
        let system_prompt = system_prompt
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let handle = py
            .detach(|| {
                RUNTIME.block_on(inner.start(user_content_request_with_system(
                    provider,
                    model,
                    content,
                    session,
                    limits,
                    features,
                    system_prompt,
                )))
            })
            .map_err(map_error)?;
        let run_id = handle.id().hyphenated();
        self.runs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(run_id.clone(), Arc::new(AsyncMutex::new(handle)));
        Ok(run_id)
    }

    fn run_handle(&self, run_id: &str) -> PyResult<Arc<AsyncMutex<RunHandle>>> {
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
        parse_python_tool_result(value)
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

fn parse_features(raw: Option<&str>) -> PyResult<ProviderFeatureRequest> {
    let Some(raw) = raw
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "{}")
    else {
        return Ok(ProviderFeatureRequest::default());
    };
    let value: Value = serde_json::from_str(raw).map_err(invalid_request)?;
    Ok(ProviderFeatureRequest {
        web_search: false,
        image_generation: false,
        reasoning_effort: optional_feature_str(&value, "reasoning_effort")?,
        thinking_mode: optional_feature_str(&value, "thinking_mode")?,
        thinking_budget: optional_feature_str(&value, "thinking_budget")?,
        prompt_cache_key: None,
        explicit_cache: false,
    })
}

fn parse_content_parts(raw: &str) -> PyResult<Vec<ContentPart>> {
    let value: Value = serde_json::from_str(raw).map_err(invalid_request)?;
    let parts = value
        .as_array()
        .filter(|parts| !parts.is_empty())
        .ok_or_else(|| invalid_request("Multimodal content must be a non-empty JSON array."))?;
    parts
        .iter()
        .enumerate()
        .map(|(index, part)| {
            let kind = part
                .get("type")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid_request(format!("Content part {index} has no `type`.")))?;
            match kind {
                "text" => part
                    .get("text")
                    .and_then(Value::as_str)
                    .map(|text| ContentPart::Text(text.to_string()))
                    .ok_or_else(|| {
                        invalid_request(format!("Text content part {index} requires `text`."))
                    }),
                "image" => {
                    let reference = content_part_string(part, index, "reference")?;
                    let mime_type =
                        content_part_string(part, index, "mime_type")?.to_ascii_lowercase();
                    if !mime_type.starts_with("image/") {
                        return Err(invalid_request(format!(
                            "Image content part {index} requires an image MIME type."
                        )));
                    }
                    Ok(ContentPart::Image(ImageContent::new(reference, mime_type)))
                }
                other => Err(invalid_request(format!(
                    "Unsupported content part type `{other}` at index {index}."
                ))),
            }
        })
        .collect()
}

fn parse_python_tool_result(value: Value) -> Result<ToolResult, ToolExecutionError> {
    let Some(envelope) = value.get("$aifluxon_tool_result") else {
        return Ok(ToolResult::from_value(value));
    };
    let content_value = envelope.get("content").cloned().unwrap_or(Value::Null);
    let content = parse_content_parts(&content_value.to_string())
        .map_err(|error| ToolExecutionError::Failed(error.to_string()))?;
    let event_value = envelope
        .get("value")
        .cloned()
        .unwrap_or_else(|| json!({ "content": content_value }));
    Ok(ToolResult::with_content(event_value, content))
}

fn content_part_string(value: &Value, index: usize, field: &str) -> PyResult<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            invalid_request(format!(
                "Content part {index} requires a non-empty `{field}`."
            ))
        })
}

fn optional_feature_str(value: &Value, field: &str) -> PyResult<Option<String>> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => {
            let text = text.trim();
            if text.is_empty() {
                Ok(None)
            } else {
                Ok(Some(text.to_string()))
            }
        }
        Some(Value::Number(number)) if field == "thinking_budget" => Ok(number
            .as_u64()
            .filter(|value| *value > 0)
            .map(|value| value.to_string())),
        _ => Err(invalid_request(format!(
            "Feature `{field}` must be a string."
        ))),
    }
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

fn map_auth_error(error: AifluxonAuthError) -> PyErr {
    PyRuntimeError::new_err(
        json!({
            "kind": error.kind().as_str(),
            "message": error.message(),
        })
        .to_string(),
    )
}

fn account_json(account: &aifluxon_api::CodexAccount) -> Value {
    json!({
        "id": account.id,
        "email": account.email,
        "expires_at": account.expires_at,
    })
}

fn status_json(status: &aifluxon_api::CodexAuthStatus) -> Value {
    json!({
        "account": account_json(&status.account),
        "state": format!("{:?}", status.state),
    })
}

#[pyclass]
struct NativeSystemKeyringStore {
    service_name: String,
}

#[pymethods]
impl NativeSystemKeyringStore {
    #[new]
    #[pyo3(signature = (service_name=None))]
    fn new(service_name: Option<String>) -> Self {
        Self {
            service_name: service_name.unwrap_or_else(|| DEFAULT_SERVICE_NAME.to_string()),
        }
    }

    #[getter]
    fn service_name(&self) -> String {
        self.service_name.clone()
    }
}

#[pyclass]
struct NativeMemorySecretStore {
    inner: Arc<MemorySecretStore>,
}

#[pymethods]
impl NativeMemorySecretStore {
    #[new]
    fn new() -> Self {
        Self {
            inner: Arc::new(MemorySecretStore::new()),
        }
    }
}

#[pyclass]
struct NativeEncryptedFileSecretStore {
    inner: Arc<EncryptedFileSecretStore>,
}

#[pymethods]
impl NativeEncryptedFileSecretStore {
    #[new]
    fn new(path: &str) -> Self {
        Self {
            inner: Arc::new(EncryptedFileSecretStore::new(path)),
        }
    }

    #[getter]
    fn path(&self) -> String {
        self.inner.path().display().to_string()
    }

    #[getter]
    fn is_unlocked(&self) -> bool {
        self.inner.is_unlocked()
    }

    fn unlock(&self, password: &str) -> PyResult<()> {
        unlock_encrypted_store(&self.inner, password).map_err(map_auth_error)
    }

    fn lock(&self) {
        self.inner.lock();
    }
}

enum NativeSecretStoreKind {
    System(String),
    Memory(Arc<MemorySecretStore>),
    Encrypted(Arc<EncryptedFileSecretStore>),
}

#[pyclass]
struct NativeCodexAuth {
    inner: CodexAuth,
}

impl NativeCodexAuth {
    fn from_store(kind: NativeSecretStoreKind) -> PyResult<Self> {
        let auth = match kind {
            NativeSecretStoreKind::System(service) => CodexAuth::builder()
                .secret_store(SystemKeyringStore::new(service))
                .build(),
            NativeSecretStoreKind::Memory(store) => CodexAuth::builder()
                .secret_store_shared(store as Arc<dyn aifluxon_api::SecretStore>)
                .build(),
            NativeSecretStoreKind::Encrypted(store) => CodexAuth::builder()
                .secret_store_shared(store as Arc<dyn aifluxon_api::SecretStore>)
                .build(),
        }
        .map_err(map_auth_error)?;
        Ok(Self { inner: auth })
    }
}

#[pymethods]
impl NativeCodexAuth {
    #[new]
    #[pyo3(signature = (secret_store=None))]
    fn new(secret_store: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        let Some(store) = secret_store else {
            return Self::from_store(NativeSecretStoreKind::System(
                DEFAULT_SERVICE_NAME.to_string(),
            ));
        };
        if let Ok(system) = store.extract::<PyRef<NativeSystemKeyringStore>>() {
            return Self::from_store(NativeSecretStoreKind::System(system.service_name.clone()));
        }
        if let Ok(memory) = store.extract::<PyRef<NativeMemorySecretStore>>() {
            return Self::from_store(NativeSecretStoreKind::Memory(memory.inner.clone()));
        }
        if let Ok(encrypted) = store.extract::<PyRef<NativeEncryptedFileSecretStore>>() {
            return Self::from_store(NativeSecretStoreKind::Encrypted(encrypted.inner.clone()));
        }
        Err(invalid_config(
            "CodexAuth secret_store must be SystemKeyringStore, MemorySecretStore, or EncryptedFileSecretStore.",
        ))
    }

    fn login(&self, py: Python<'_>) -> PyResult<NativeCodexLoginAttempt> {
        let auth = self.inner.clone();
        let attempt = py
            .detach(|| RUNTIME.block_on(auth.begin_login()))
            .map_err(map_auth_error)?;
        Ok(NativeCodexLoginAttempt {
            authorization_url: attempt.authorization_url().to_string(),
            inner: Mutex::new(Some(attempt)),
        })
    }

    fn accounts(&self, py: Python<'_>) -> PyResult<String> {
        let auth = self.inner.clone();
        let accounts = py
            .detach(|| RUNTIME.block_on(auth.accounts()))
            .map_err(map_auth_error)?;
        Ok(json!(accounts.iter().map(account_json).collect::<Vec<_>>()).to_string())
    }

    fn status(&self, py: Python<'_>, account_id: &str) -> PyResult<String> {
        let auth = self.inner.clone();
        let account_id = account_id.to_string();
        let status = py
            .detach(|| RUNTIME.block_on(auth.status(&account_id)))
            .map_err(map_auth_error)?;
        Ok(status_json(&status).to_string())
    }

    fn refresh(&self, py: Python<'_>, account_id: &str) -> PyResult<String> {
        let auth = self.inner.clone();
        let account_id = account_id.to_string();
        let status = py
            .detach(|| RUNTIME.block_on(auth.refresh(&account_id)))
            .map_err(map_auth_error)?;
        Ok(status_json(&status).to_string())
    }

    fn logout(&self, py: Python<'_>, account_id: &str) -> PyResult<()> {
        let auth = self.inner.clone();
        let account_id = account_id.to_string();
        py.detach(|| RUNTIME.block_on(auth.logout(&account_id)))
            .map_err(map_auth_error)
    }

    fn provider(&self, model: &str, account_id: Option<&str>) -> PyResult<NativeCodexProvider> {
        let handle = self
            .inner
            .provider(model, account_id.map(ToOwned::to_owned))
            .map_err(map_auth_error)?;
        Ok(NativeCodexProvider { inner: handle })
    }

    fn seed_account(
        &self,
        account_id: &str,
        access_token: &str,
        refresh_token: &str,
        id_token: &str,
    ) -> PyResult<String> {
        let account = self
            .inner
            .seed_account_for_tests(account_id, access_token, refresh_token, id_token)
            .map_err(map_auth_error)?;
        Ok(account_json(&account).to_string())
    }
}

#[pyclass]
struct NativeCodexLoginAttempt {
    authorization_url: String,
    inner: Mutex<Option<CodexLoginAttempt>>,
}

#[pymethods]
impl NativeCodexLoginAttempt {
    #[getter]
    fn authorization_url(&self) -> String {
        self.authorization_url.clone()
    }

    fn wait(&self, py: Python<'_>) -> PyResult<String> {
        let attempt = self.take_attempt()?;
        py.detach(|| RUNTIME.block_on(attempt.wait()))
            .map(|account| account_json(&account).to_string())
            .map_err(map_auth_error)
    }

    fn cancel(&self, py: Python<'_>) -> PyResult<()> {
        if let Some(attempt) = self.take_attempt_optional() {
            py.detach(|| RUNTIME.block_on(attempt.cancel()));
        }
        Ok(())
    }
}

impl NativeCodexLoginAttempt {
    fn take_attempt(&self) -> PyResult<CodexLoginAttempt> {
        self.take_attempt_optional().ok_or_else(|| {
            PyRuntimeError::new_err(
                json!({
                    "kind": "Cancelled",
                    "message": "Codex login attempt is no longer active.",
                })
                .to_string(),
            )
        })
    }

    fn take_attempt_optional(&self) -> Option<CodexLoginAttempt> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }
}

#[pyclass]
struct NativeCodexProvider {
    inner: CodexProviderHandle,
}

#[pymethods]
impl NativeCodexProvider {
    #[getter]
    fn provider_id(&self) -> String {
        self.inner.provider_id().as_str().to_string()
    }

    #[getter]
    fn model(&self) -> String {
        self.inner.model().to_string()
    }

    #[getter]
    fn account_id(&self) -> String {
        self.inner.account_id().to_string()
    }

    fn spec_json(&self) -> String {
        json!({
            "kind": "codex_oauth",
            "model": self.inner.model(),
            "account_id": self.inner.account_id(),
        })
        .to_string()
    }
}

#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<NativeAgent>()?;
    m.add_class::<NativeCodexAuth>()?;
    m.add_class::<NativeCodexLoginAttempt>()?;
    m.add_class::<NativeCodexProvider>()?;
    m.add_class::<NativeSystemKeyringStore>()?;
    m.add_class::<NativeMemorySecretStore>()?;
    m.add_class::<NativeEncryptedFileSecretStore>()?;
    Ok(())
}
