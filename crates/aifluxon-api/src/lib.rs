//! Stable, transport-neutral application facade for AIFLUXON.

mod agent;
mod auth;
mod backend;
mod builder;
mod controlled;
mod events_json;
mod prompt;
mod providers;
mod result;
mod run;
mod stream;
mod workspace;

#[cfg(test)]
mod engine_tests;

pub use agent::{ProviderRegistry, ProviderRegistryError, ToolPolicy};
pub use auth::{
    unlock_encrypted_store, AifluxonAuthError, AifluxonAuthErrorKind, CodexAccount, CodexAuth,
    CodexAuthBuilder, CodexAuthState, CodexAuthStatus, CodexLoginAttempt, CodexProviderHandle,
    EncryptedFileSecretStore, MemorySecretStore, SecretStore, SecretString, SystemKeyringStore,
    CODEX_OAUTH_BASE_URL, DEFAULT_SERVICE_NAME,
};
pub use backend::{Aifluxon, AifluxonError, AifluxonErrorKind};
pub use builder::AifluxonBuilder;
pub use controlled::ControlledProvider;
pub use events_json::{envelope_to_json, event_to_json, operation_snapshot_to_json};
pub use prompt::{
    user_content_request_with_system, user_prompt_request, user_prompt_request_with_features,
    user_prompt_request_with_system,
};
pub use providers::{
    register_provider_from_json, ProviderBinding, CODEX_DEFAULT_BASE_URL,
    DEEPSEEK_DEFAULT_BASE_URL, GEMINI_DEFAULT_BASE_URL, KIMI_DEFAULT_BASE_URL,
    OPENAI_DEFAULT_BASE_URL, QWEN_DEFAULT_BASE_URL,
};
pub use result::RunResult;
pub use run::{RunHandle, RunSnapshot};
pub use stream::{NoopRunEventSink, RunEvent, RunEventEnvelope, RunEventSink, RunEventStream};
pub use workspace::{EmptyWorkspace, Workspace};

pub use aifluxon_core::{
    ArtifactRef, CapabilityId, ContentPart, ContinuationReason, ExecutionAuthority, ImageContent,
    Message, MessageRole, ModelEventSink, ModelProvider, ModelRef, ModelTurn, ModelTurnRequest,
    NoopModelEventSink, OperationDecision, OperationId, OperationMode, PendingOperation,
    PendingOperationDraft, PermissionMode, ProviderCapabilities, ProviderError,
    ProviderFeatureRequest, ProviderId, ProviderSessionKey, ProviderTerminal, RunContext, RunId,
    RunLimits, RunRequest, RunState, SessionId, ToolCall, ToolDescriptor, ToolEffect,
    ToolInvocationId,
};
pub use aifluxon_runtime::{
    AllowAllToolPolicy, InMemoryProviderStateStore, InMemoryRunCheckpointStore,
    InMemorySessionStore, JsonFileProviderStateStore, JsonFileSessionStore, ProviderStateRecord,
    ProviderStateStore, RegisteredTool, RunCheckpoint, RunCheckpointStore, SessionRecord,
    SessionStore, SessionSummary, StoreError, ToolDecision, ToolExecutionContext,
    ToolExecutionError, ToolExecutor, ToolInvocation, ToolPolicyInput, ToolRegistry,
    ToolRegistryError, ToolResult,
};

#[cfg(test)]
mod crate_boundary {
    #[test]
    fn api_manifest_forbids_host_binding_and_server_dependencies() {
        let manifest = include_str!("../Cargo.toml");
        let dependencies = manifest
            .split_once("[dependencies]")
            .expect("api manifest must have dependencies")
            .1
            .split("\n[")
            .next()
            .expect("dependency section must be readable");
        let allowed = [
            "async-trait.workspace",
            "aifluxon-core",
            "aifluxon-runtime",
            "aifluxon-providers",
            "aifluxon-auth",
            "serde_json.workspace",
            "thiserror.workspace",
            "tokio.workspace",
        ];
        for dependency in dependencies
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .filter_map(|line| line.split_once('=').map(|(name, _)| name.trim()))
        {
            assert!(
                allowed.contains(&dependency),
                "aifluxon-api dependency `{dependency}` is outside its boundary"
            );
        }
    }
}
