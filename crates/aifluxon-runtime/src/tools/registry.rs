use super::executor::{prepare_invocation, ToolExecutionError, ToolExecutor, ToolInvocation};
use aifluxon_core::{ToolDescriptor, ToolInvocationId, ToolValidationError};
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct RegisteredTool {
    descriptor: ToolDescriptor,
    executor: Arc<dyn ToolExecutor>,
}

impl RegisteredTool {
    pub fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    pub fn executor(&self) -> Arc<dyn ToolExecutor> {
        Arc::clone(&self.executor)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ToolRegistryError {
    DuplicateToolName { name: String },
}

impl fmt::Display for ToolRegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateToolName { name } => {
                write!(formatter, "Tool `{name}` is already registered.")
            }
        }
    }
}

impl std::error::Error for ToolRegistryError {}

#[derive(Default)]
pub struct ToolRegistry {
    tools: Mutex<HashMap<String, RegisteredTool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &self,
        descriptor: ToolDescriptor,
        executor: Arc<dyn ToolExecutor>,
    ) -> Result<(), ToolRegistryError> {
        let name = descriptor.name.clone();
        let mut tools = self
            .tools
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if tools.contains_key(&name) {
            return Err(ToolRegistryError::DuplicateToolName { name });
        }
        tools.insert(
            name,
            RegisteredTool {
                descriptor,
                executor,
            },
        );
        Ok(())
    }

    pub fn resolve(&self, name: &str) -> Option<RegisteredTool> {
        self.tools
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(name)
            .cloned()
    }

    pub fn contains(&self, name: &str) -> bool {
        self.tools
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains_key(name)
    }

    pub fn descriptors(&self) -> Vec<ToolDescriptor> {
        let mut descriptors = self
            .tools
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values()
            .map(|registered| registered.descriptor.clone())
            .collect::<Vec<_>>();
        descriptors.sort_by(|left, right| left.name.cmp(&right.name));
        descriptors
    }

    pub fn prepare_invocation(
        &self,
        invocation_id: ToolInvocationId,
        name: &str,
        raw_arguments: &str,
    ) -> Result<(ToolInvocation, Arc<dyn ToolExecutor>), ToolExecutionError> {
        let registered = self.resolve(name).ok_or(ToolExecutionError::Validation(
            ToolValidationError::UnknownTool,
        ))?;
        let invocation = prepare_invocation(invocation_id, registered.descriptor(), raw_arguments)?;
        Ok((invocation, registered.executor()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::executor::{ToolExecutionContext, ToolResult};
    use aifluxon_core::ToolEffect;
    use async_trait::async_trait;
    use serde_json::json;

    struct NoopExecutor;

    #[async_trait]
    impl ToolExecutor for NoopExecutor {
        async fn execute(
            &self,
            _invocation: ToolInvocation,
            _ctx: ToolExecutionContext,
        ) -> Result<ToolResult, ToolExecutionError> {
            Ok(ToolResult {
                value: json!({ "ok": true }),
                content: None,
            })
        }
    }

    fn descriptor(name: &str, effect: ToolEffect) -> ToolDescriptor {
        ToolDescriptor {
            name: name.to_string(),
            description: format!("Execute {name}"),
            input_schema: json!({
                "type": "object",
                "properties": { "query": { "type": "string" } },
                "required": ["query"]
            }),
            effect,
            required_capabilities: Vec::new(),
            parallel_safe: true,
        }
    }

    #[test]
    fn arbitrary_custom_tool_resolves_with_its_descriptor_and_executor() {
        let registry = ToolRegistry::new();
        registry
            .register(
                descriptor("query_inventory", ToolEffect::Network),
                Arc::new(NoopExecutor),
            )
            .unwrap();

        let registered = registry.resolve("query_inventory").unwrap();
        assert_eq!(registered.descriptor().name, "query_inventory");
        assert_eq!(registered.descriptor().effect, ToolEffect::Network);
        assert!(registry.resolve("missing_tool").is_none());
    }

    #[test]
    fn duplicate_registration_is_rejected_explicitly() {
        let registry = ToolRegistry::new();
        registry
            .register(
                descriptor("query_inventory", ToolEffect::Network),
                Arc::new(NoopExecutor),
            )
            .unwrap();
        assert_eq!(
            registry
                .register(
                    descriptor("query_inventory", ToolEffect::PureRead),
                    Arc::new(NoopExecutor),
                )
                .unwrap_err(),
            ToolRegistryError::DuplicateToolName {
                name: "query_inventory".to_string()
            }
        );
    }

    #[test]
    fn unknown_tool_fails_before_argument_preparation() {
        let registry = ToolRegistry::new();
        let result = registry.prepare_invocation(
            ToolInvocationId::new(),
            "missing_tool",
            r#"{"query":"value"}"#,
        );
        assert!(matches!(
            result,
            Err(ToolExecutionError::Validation(
                ToolValidationError::UnknownTool
            ))
        ));
    }
}
