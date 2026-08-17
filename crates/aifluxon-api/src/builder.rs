use crate::backend::BackendConfiguration;
use crate::{
    Aifluxon, AifluxonError, AifluxonErrorKind, EmptyWorkspace, ProviderRegistry, RunEventSink,
    ToolPolicy, ToolRegistry, Workspace,
};
use std::sync::Arc;

#[derive(Default)]
pub struct AifluxonBuilder {
    provider_registry: Option<Arc<ProviderRegistry>>,
    tool_registry: Option<Arc<ToolRegistry>>,
    tool_policy: Option<Arc<dyn ToolPolicy>>,
    event_sink: Option<Arc<dyn RunEventSink>>,
    workspace: Option<Arc<dyn Workspace>>,
    session_store: Option<Arc<dyn aifluxon_runtime::SessionStore>>,
    provider_state_store: Option<Arc<dyn aifluxon_runtime::ProviderStateStore>>,
    checkpoint_store: Option<Arc<dyn aifluxon_runtime::RunCheckpointStore>>,
    run_table: Option<aifluxon_runtime::RunTable>,
}

impl AifluxonBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn provider_registry(mut self, registry: impl Into<Arc<ProviderRegistry>>) -> Self {
        self.provider_registry = Some(registry.into());
        self
    }

    pub fn tool_registry(mut self, registry: impl Into<Arc<ToolRegistry>>) -> Self {
        self.tool_registry = Some(registry.into());
        self
    }

    pub fn tool_policy(mut self, policy: Arc<dyn ToolPolicy>) -> Self {
        self.tool_policy = Some(policy);
        self
    }

    pub fn event_sink(mut self, sink: Arc<dyn RunEventSink>) -> Self {
        self.event_sink = Some(sink);
        self
    }

    pub fn workspace(mut self, workspace: Arc<dyn Workspace>) -> Self {
        self.workspace = Some(workspace);
        self
    }

    pub fn session_store(mut self, store: Arc<dyn aifluxon_runtime::SessionStore>) -> Self {
        self.session_store = Some(store);
        self
    }

    pub fn provider_state_store(
        mut self,
        store: Arc<dyn aifluxon_runtime::ProviderStateStore>,
    ) -> Self {
        self.provider_state_store = Some(store);
        self
    }

    pub fn checkpoint_store(
        mut self,
        store: Arc<dyn aifluxon_runtime::RunCheckpointStore>,
    ) -> Self {
        self.checkpoint_store = Some(store);
        self
    }

    pub fn run_table(mut self, run_table: aifluxon_runtime::RunTable) -> Self {
        self.run_table = Some(run_table);
        self
    }

    pub fn build(self) -> Result<Aifluxon, AifluxonError> {
        let provider_registry = required(self.provider_registry, "provider_registry")?;
        let tool_registry = required(self.tool_registry, "tool_registry")?;
        let tool_policy = required(self.tool_policy, "tool_policy")?;
        let event_sink = required(self.event_sink, "event_sink")?;
        let workspace = self.workspace.unwrap_or_else(|| Arc::new(EmptyWorkspace));
        let session_store = self
            .session_store
            .unwrap_or_else(|| Arc::new(aifluxon_runtime::InMemorySessionStore::default()));
        let provider_state_store = self
            .provider_state_store
            .unwrap_or_else(|| Arc::new(aifluxon_runtime::InMemoryProviderStateStore::default()));
        let checkpoint_store = self
            .checkpoint_store
            .unwrap_or_else(|| Arc::new(aifluxon_runtime::InMemoryRunCheckpointStore::default()));

        Ok(Aifluxon::from_configuration(BackendConfiguration {
            provider_registry,
            tool_registry,
            tool_policy,
            event_sink,
            workspace,
            run_table: self
                .run_table
                .unwrap_or_else(aifluxon_runtime::RunTable::new),
            session_store,
            provider_state_store,
            checkpoint_store,
        }))
    }
}

fn required<T>(value: Option<T>, name: &str) -> Result<T, AifluxonError> {
    value.ok_or_else(|| {
        AifluxonError::new(
            AifluxonErrorKind::InvalidConfiguration,
            format!("AIFLUXON builder requires `{name}`."),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NoopRunEventSink;

    struct TestPolicy;
    impl ToolPolicy for TestPolicy {
        fn evaluate(
            &self,
            _input: &aifluxon_runtime::ToolPolicyInput<'_>,
        ) -> aifluxon_runtime::ToolDecision {
            aifluxon_runtime::ToolDecision::Allow
        }
    }

    #[test]
    fn builder_requires_each_stable_extension_boundary() {
        let error = match Aifluxon::builder().build() {
            Ok(_) => panic!("incomplete builder must fail"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), AifluxonErrorKind::InvalidConfiguration);
        assert!(error.message().contains("provider_registry"));
    }

    #[test]
    fn builder_constructs_transport_neutral_facade() {
        Aifluxon::builder()
            .provider_registry(ProviderRegistry::new())
            .tool_registry(ToolRegistry::new())
            .tool_policy(Arc::new(TestPolicy))
            .event_sink(Arc::new(NoopRunEventSink))
            .build()
            .unwrap();
    }
}
