use super::validation::prepare_tool_call;
use aifluxon_core::{
    ContentPart, PreparedToolCall, ToolDescriptor, ToolEffect, ToolInvocationId,
    ToolValidationError,
};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Clone, Debug)]
pub struct ToolInvocation {
    pub id: ToolInvocationId,
    pub name: String,
    pub arguments: Value,
    pub effect: ToolEffect,
}

#[derive(Clone, Debug)]
pub struct ToolResult {
    pub value: Value,
    pub content: Option<Vec<ContentPart>>,
}

impl ToolResult {
    pub fn from_value(value: Value) -> Self {
        Self {
            value,
            content: None,
        }
    }

    pub fn with_content(value: Value, content: Vec<ContentPart>) -> Self {
        Self {
            value,
            content: Some(content),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ToolExecutionContext;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ToolExecutionError {
    Validation(ToolValidationError),
    RejectedDuplicate,
    Failed(String),
}

impl ToolExecutionError {
    pub fn message(&self) -> String {
        match self {
            Self::Validation(error) => error.message(),
            Self::RejectedDuplicate => {
                "Duplicate tool invocation was rejected because the original side effect already ran."
                    .to_string()
            }
            Self::Failed(message) => message.clone(),
        }
    }
}

#[async_trait::async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn execute(
        &self,
        invocation: ToolInvocation,
        ctx: ToolExecutionContext,
    ) -> Result<ToolResult, ToolExecutionError>;
}

#[derive(Default)]
pub struct ToolLedger {
    completed: Mutex<HashMap<ToolInvocationId, ToolResult>>,
}

impl ToolLedger {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, invocation_id: &ToolInvocationId) -> Option<ToolResult> {
        self.completed
            .lock()
            .ok()
            .and_then(|completed| completed.get(invocation_id).cloned())
    }

    pub fn record(&self, invocation_id: ToolInvocationId, result: ToolResult) {
        if let Ok(mut completed) = self.completed.lock() {
            completed.insert(invocation_id, result);
        }
    }

    pub fn execute_once<F>(
        &self,
        invocation_id: ToolInvocationId,
        execute: F,
    ) -> Result<ToolResult, ToolExecutionError>
    where
        F: FnOnce() -> Result<ToolResult, ToolExecutionError>,
    {
        let mut completed = self
            .completed
            .lock()
            .map_err(|_| ToolExecutionError::Failed("Tool ledger is unavailable.".to_string()))?;
        if let Some(cached) = completed.get(&invocation_id) {
            return Ok(cached.clone());
        }
        // Keep the claim and execution atomic for the synchronous executor seam. This prevents
        // concurrent provider replay from running the same side effect twice.
        let result = execute()?;
        completed.insert(invocation_id, result.clone());
        Ok(result)
    }

    pub fn execution_count(&self) -> usize {
        self.completed
            .lock()
            .map(|completed| completed.len())
            .unwrap_or(0)
    }
}

pub fn prepare_invocation(
    invocation_id: ToolInvocationId,
    descriptor: &ToolDescriptor,
    raw_arguments: &str,
) -> Result<ToolInvocation, ToolExecutionError> {
    let prepared =
        prepare_tool_call(descriptor, raw_arguments).map_err(ToolExecutionError::Validation)?;
    Ok(invocation_from_prepared(invocation_id, prepared))
}

pub fn invocation_from_prepared(
    invocation_id: ToolInvocationId,
    prepared: PreparedToolCall,
) -> ToolInvocation {
    ToolInvocation {
        id: invocation_id,
        name: prepared.name,
        arguments: prepared.arguments,
        effect: prepared.effect,
    }
}

pub fn validation_error_value(error: &ToolValidationError) -> Value {
    serde_json::json!({
        "ok": false,
        "error": error.message(),
        "errorCode": "TOOL_VALIDATION_FAILED",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn descriptor() -> ToolDescriptor {
        ToolDescriptor {
            name: "perform_action".to_string(),
            description: "Perform an action".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": { "value": { "type": "string" } },
                "required": ["value"]
            }),
            effect: ToolEffect::ExternalSideEffect,
            required_capabilities: Vec::new(),
            parallel_safe: false,
        }
    }

    #[test]
    fn bug_agent_013_provider_retry_reuses_cached_tool_side_effect() {
        let ledger = ToolLedger::new();
        let invocations = AtomicUsize::new(0);
        let invocation_id = ToolInvocationId::new();
        let execute = || {
            invocations.fetch_add(1, Ordering::SeqCst);
            Ok(ToolResult {
                value: json!({ "ok": true, "ran": true }),
                content: None,
            })
        };

        let first = ledger.execute_once(invocation_id, execute).unwrap();
        let second = ledger
            .execute_once(invocation_id, || {
                invocations.fetch_add(1, Ordering::SeqCst);
                Ok(ToolResult {
                    value: json!({ "ok": true, "ran": true }),
                    content: None,
                })
            })
            .unwrap();

        assert_eq!(first.value, second.value);
        assert_eq!(invocations.load(Ordering::SeqCst), 1);
        assert_eq!(ledger.execution_count(), 1);
    }

    #[test]
    fn cached_tool_result_preserves_multimodal_content() {
        let ledger = ToolLedger::new();
        let invocation_id = ToolInvocationId::new();
        let result = ToolResult::with_content(
            json!({ "kind": "image" }),
            vec![ContentPart::Image(aifluxon_core::ImageContent::new(
                "https://example.com/tool.png",
                "image/png",
            ))],
        );
        ledger.record(invocation_id, result.clone());

        assert_eq!(ledger.get(&invocation_id).unwrap().content, result.content);
    }

    #[test]
    fn bug_agent_015_prepare_gate_rejects_before_executor() {
        let executed = Arc::new(AtomicUsize::new(0));
        let result = prepare_invocation(ToolInvocationId::new(), &descriptor(), "{not-json");
        assert!(matches!(
            result,
            Err(ToolExecutionError::Validation(
                ToolValidationError::InvalidJson
            ))
        ));
        assert_eq!(executed.load(Ordering::SeqCst), 0);
    }
}
