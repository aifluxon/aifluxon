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

enum ToolEntry {
    InFlight,
    Completed(ToolResult),
    Failed(ToolExecutionError),
}

#[derive(Default)]
pub struct ToolLedger {
    entries: Mutex<HashMap<ToolInvocationId, ToolEntry>>,
}

pub enum ToolClaimOutcome<'a> {
    Cached(ToolResult),
    Reserved(ToolClaim<'a>),
}

/// An unstarted reservation is released on drop. Once execution starts, an interrupted
/// or failed invocation is retained: its side effects cannot safely be retried.
pub struct ToolClaim<'a> {
    ledger: &'a ToolLedger,
    id: ToolInvocationId,
    started: bool,
    settled: bool,
}

impl ToolClaim<'_> {
    pub fn start(&mut self) {
        self.started = true;
    }

    pub fn complete(mut self, result: ToolResult) {
        self.ledger
            .entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(self.id, ToolEntry::Completed(result));
        self.settled = true;
    }

    pub fn fail(mut self, error: ToolExecutionError) {
        self.ledger
            .entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(self.id, ToolEntry::Failed(error));
        self.settled = true;
    }
}

impl Drop for ToolClaim<'_> {
    fn drop(&mut self) {
        if self.settled {
            return;
        }
        let mut entries = self
            .ledger
            .entries
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if self.started {
            entries.insert(
                self.id,
                ToolEntry::Failed(ToolExecutionError::Failed(
                    "Tool execution was interrupted; its side effect will not be repeated."
                        .to_string(),
                )),
            );
        } else {
            entries.remove(&self.id);
        }
    }
}

impl ToolLedger {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, invocation_id: &ToolInvocationId) -> Option<ToolResult> {
        match self
            .entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(invocation_id)
        {
            Some(ToolEntry::Completed(result)) => Some(result.clone()),
            _ => None,
        }
    }

    /// Imports a completed result only when no invocation already owns the ID.
    pub fn record(&self, invocation_id: ToolInvocationId, result: ToolResult) {
        self.entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(invocation_id)
            .or_insert(ToolEntry::Completed(result));
    }

    /// Concurrent duplicates are rejected; completed duplicates reuse the exact result.
    pub fn claim(&self, id: ToolInvocationId) -> Result<ToolClaimOutcome<'_>, ToolExecutionError> {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        match entries.get(&id) {
            Some(ToolEntry::Completed(result)) => {
                return Ok(ToolClaimOutcome::Cached(result.clone()))
            }
            Some(ToolEntry::InFlight) => return Err(ToolExecutionError::RejectedDuplicate),
            Some(ToolEntry::Failed(error)) => return Err(error.clone()),
            None => {}
        }
        entries.insert(id, ToolEntry::InFlight);
        Ok(ToolClaimOutcome::Reserved(ToolClaim {
            ledger: self,
            id,
            started: false,
            settled: false,
        }))
    }

    pub fn execute_once<F>(
        &self,
        id: ToolInvocationId,
        execute: F,
    ) -> Result<ToolResult, ToolExecutionError>
    where
        F: FnOnce() -> Result<ToolResult, ToolExecutionError>,
    {
        let mut claim = match self.claim(id)? {
            ToolClaimOutcome::Cached(result) => return Ok(result),
            ToolClaimOutcome::Reserved(claim) => claim,
        };
        claim.start();
        match execute() {
            Ok(result) => {
                claim.complete(result.clone());
                Ok(result)
            }
            Err(error) => {
                claim.fail(error.clone());
                Err(error)
            }
        }
    }

    pub async fn execute_once_async<F, Fut>(
        &self,
        id: ToolInvocationId,
        execute: F,
    ) -> Result<ToolResult, ToolExecutionError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<ToolResult, ToolExecutionError>>,
    {
        let mut claim = match self.claim(id)? {
            ToolClaimOutcome::Cached(result) => return Ok(result),
            ToolClaimOutcome::Reserved(claim) => claim,
        };
        claim.start();
        match execute().await {
            Ok(result) => {
                claim.complete(result.clone());
                Ok(result)
            }
            Err(error) => {
                claim.fail(error.clone());
                Err(error)
            }
        }
    }

    pub fn execution_count(&self) -> usize {
        self.entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .filter(|entry| matches!(entry, ToolEntry::Completed(_)))
            .count()
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
        "errorCode": "TOOL_ARGUMENT_VALIDATION_FAILED",
        "validation": {
            "kind": error.kind(),
            "path": error.path(),
            "pathFormat": "json_pointer",
            "retryable": true,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[tokio::test]
    async fn concurrent_async_duplicate_is_rejected_and_completed_result_is_reused() {
        let ledger = Arc::new(ToolLedger::new());
        let id = ToolInvocationId::new();
        let executions = Arc::new(AtomicUsize::new(0));
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (finish_tx, finish_rx) = tokio::sync::oneshot::channel();
        let worker = {
            let ledger = ledger.clone();
            let executions = executions.clone();
            tokio::spawn(async move {
                ledger
                    .execute_once_async(id, || async {
                        executions.fetch_add(1, Ordering::SeqCst);
                        started_tx.send(()).unwrap();
                        finish_rx.await.unwrap();
                        Ok(ToolResult::with_content(
                            json!("done"),
                            vec![ContentPart::Text("result".into())],
                        ))
                    })
                    .await
                    .unwrap()
            })
        };
        started_rx.await.unwrap();
        assert!(matches!(
            ledger
                .execute_once_async(id, || async { panic!("duplicate executed") })
                .await,
            Err(ToolExecutionError::RejectedDuplicate)
        ));
        // Synchronous callers and imported records use the same reservation.
        ledger.record(
            id,
            ToolResult::from_value(json!("must not overwrite claim")),
        );
        assert!(matches!(
            ledger.execute_once(id, || panic!("duplicate executed")),
            Err(ToolExecutionError::RejectedDuplicate)
        ));
        finish_tx.send(()).unwrap();
        let first = worker.await.unwrap();
        let cached = ledger
            .execute_once_async(id, || async { panic!("cached invocation executed") })
            .await
            .unwrap();
        assert_eq!(first.content, cached.content);
        assert_eq!(cached.value, json!("done"));
        assert_eq!(executions.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn aborted_or_failed_async_execution_cannot_repeat_side_effects() {
        let ledger = Arc::new(ToolLedger::new());
        let id = ToolInvocationId::new();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let worker = {
            let ledger = ledger.clone();
            tokio::spawn(async move {
                ledger
                    .execute_once_async(id, || async {
                        started_tx.send(()).unwrap();
                        std::future::pending().await
                    })
                    .await
            })
        };
        started_rx.await.unwrap();
        worker.abort();
        assert!(worker.await.unwrap_err().is_cancelled());
        assert!(ledger
            .execute_once_async(id, || async { panic!("interrupted effect repeated") })
            .await
            .is_err());
        let failed = ToolInvocationId::new();
        let error = ToolExecutionError::Failed("effect may already have happened".into());
        assert_eq!(
            ledger
                .execute_once_async(failed, || async { Err(error.clone()) })
                .await
                .unwrap_err(),
            error
        );
        assert_eq!(
            ledger
                .execute_once(failed, || panic!("failed effect repeated"))
                .unwrap_err(),
            error
        );
        let unstarted = ToolInvocationId::new();
        drop(ledger.claim(unstarted).unwrap());
        assert!(ledger
            .execute_once(unstarted, || Ok(ToolResult::from_value(Value::Null)))
            .is_ok());
    }

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

    #[test]
    fn validation_result_is_structured_and_retryable() {
        let error = ToolValidationError::SchemaViolation {
            kind: "type".to_string(),
            path: "/edit/summary".to_string(),
            message: "null is not of type string".to_string(),
        };

        assert_eq!(
            validation_error_value(&error),
            json!({
                "ok": false,
                "error": "null is not of type string",
                "errorCode": "TOOL_ARGUMENT_VALIDATION_FAILED",
                "validation": {
                    "kind": "type",
                    "path": "/edit/summary",
                    "pathFormat": "json_pointer",
                    "retryable": true,
                }
            })
        );
    }
}
