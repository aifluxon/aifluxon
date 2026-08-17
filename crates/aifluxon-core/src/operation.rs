use crate::ids::{OperationId, RunId, ToolInvocationId};
use crate::tool::ToolEffect;
use serde_json::Value;
use std::time::SystemTime;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationMode {
    BlockingApproval,
    DeferredCommit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingOperation {
    pub id: OperationId,
    pub run_id: RunId,
    pub invocation_id: Option<ToolInvocationId>,
    pub effect: ToolEffect,
    pub mode: OperationMode,
    pub summary: String,
    pub payload: Value,
    pub deadline: Option<SystemTime>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingOperationDraft {
    pub invocation_id: Option<ToolInvocationId>,
    pub effect: ToolEffect,
    pub mode: OperationMode,
    pub summary: String,
    pub payload: Value,
    pub deadline: Option<SystemTime>,
}

impl PendingOperationDraft {
    pub fn bind(self, id: OperationId, run_id: RunId) -> PendingOperation {
        PendingOperation {
            id,
            run_id,
            invocation_id: self.invocation_id,
            effect: self.effect,
            mode: self.mode,
            summary: self.summary,
            payload: self.payload,
            deadline: self.deadline,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OperationDecision {
    Approve { data: Option<Value> },
    Reject { reason: Option<String> },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OperationState {
    Pending,
    Approved,
    Rejected,
    Committing,
    Committed,
    Failed { message: String },
    Cancelled,
    Expired,
}

impl OperationState {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Rejected
                | Self::Committed
                | Self::Failed { .. }
                | Self::Cancelled
                | Self::Expired
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationSnapshot {
    pub operation: PendingOperation,
    pub state: OperationState,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn generic_operation_can_be_constructed_without_product_approval_types() {
        let run_id = RunId::new();
        let invocation_id = ToolInvocationId::new();
        let operation = PendingOperation {
            id: OperationId::new(),
            run_id,
            invocation_id: Some(invocation_id),
            effect: ToolEffect::ExternalSideEffect,
            mode: OperationMode::BlockingApproval,
            summary: "Allow the external effect".to_string(),
            payload: json!({ "scope": "account" }),
            deadline: None,
        };
        let decision = OperationDecision::Approve {
            data: Some(json!({ "scope": "once" })),
        };

        assert_eq!(operation.run_id, run_id);
        assert_eq!(operation.invocation_id, Some(invocation_id));
        assert!(matches!(
            decision,
            OperationDecision::Approve { data: Some(_) }
        ));
    }

    #[test]
    fn blocking_and_deferred_modes_are_product_neutral_and_distinct() {
        let draft = PendingOperationDraft {
            invocation_id: None,
            effect: ToolEffect::FsWrite,
            mode: OperationMode::DeferredCommit,
            summary: "Commit the prepared change".to_string(),
            payload: Value::Null,
            deadline: None,
        };
        let operation = draft.bind(OperationId::new(), RunId::new());

        assert_eq!(operation.mode, OperationMode::DeferredCommit);
        assert_ne!(operation.mode, OperationMode::BlockingApproval);
        assert!(!OperationState::Committing.is_terminal());
        assert!(OperationState::Committed.is_terminal());
    }
}
