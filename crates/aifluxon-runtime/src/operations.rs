use aifluxon_core::{
    OperationDecision, OperationId, OperationMode, OperationSnapshot, OperationState,
    PendingOperation, RunId,
};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::SystemTime;
use tokio::sync::Notify;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OperationError {
    IdentifierCollision,
    UnknownOperation,
    WrongRun,
    AlreadyResolved,
    BlockingWaitRequired,
    DeferredCommitRequired,
    InvalidState,
    RunTerminal,
    Cancelled,
    Expired,
    CommitFailed(String),
    StoreUnavailable,
}

impl OperationError {
    pub fn message(&self) -> String {
        match self {
            Self::IdentifierCollision => "Operation identifier collision.".to_string(),
            Self::UnknownOperation => "Operation was not found.".to_string(),
            Self::WrongRun => "Operation does not belong to the requested run.".to_string(),
            Self::AlreadyResolved => "Operation was already resolved.".to_string(),
            Self::BlockingWaitRequired => {
                "Only blocking approvals can be awaited for a decision.".to_string()
            }
            Self::DeferredCommitRequired => {
                "Deferred operations must use the prepared-effect commit path.".to_string()
            }
            Self::InvalidState => "Operation is not in a valid state for this action.".to_string(),
            Self::RunTerminal => {
                "The run is terminal and cannot accept operation changes.".to_string()
            }
            Self::Cancelled => "Operation was cancelled with its run.".to_string(),
            Self::Expired => "Operation expired before it was resolved.".to_string(),
            Self::CommitFailed(message) => format!("Prepared operation commit failed: {message}"),
            Self::StoreUnavailable => "Operation store is unavailable.".to_string(),
        }
    }
}

struct OperationRecord {
    operation: PendingOperation,
    state: OperationState,
    decision: Option<OperationDecision>,
    notify: Arc<Notify>,
}

#[derive(Default)]
struct OperationStoreInner {
    operations: HashMap<OperationId, OperationRecord>,
    terminal_runs: HashSet<RunId>,
}

#[derive(Clone, Default)]
pub struct InMemoryOperationStore {
    inner: Arc<Mutex<OperationStoreInner>>,
}

impl InMemoryOperationStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> Result<MutexGuard<'_, OperationStoreInner>, OperationError> {
        self.inner
            .lock()
            .map_err(|_| OperationError::StoreUnavailable)
    }

    fn record_for_run<'a>(
        inner: &'a OperationStoreInner,
        run_id: &RunId,
        operation_id: &OperationId,
    ) -> Result<&'a OperationRecord, OperationError> {
        let record = inner
            .operations
            .get(operation_id)
            .ok_or(OperationError::UnknownOperation)?;
        if &record.operation.run_id != run_id {
            return Err(OperationError::WrongRun);
        }
        Ok(record)
    }

    fn record_for_run_mut<'a>(
        inner: &'a mut OperationStoreInner,
        run_id: &RunId,
        operation_id: &OperationId,
    ) -> Result<&'a mut OperationRecord, OperationError> {
        let record = inner
            .operations
            .get_mut(operation_id)
            .ok_or(OperationError::UnknownOperation)?;
        if &record.operation.run_id != run_id {
            return Err(OperationError::WrongRun);
        }
        Ok(record)
    }

    pub fn register(&self, operation: PendingOperation) -> Result<(), OperationError> {
        let mut inner = self.lock()?;
        if inner.terminal_runs.contains(&operation.run_id) {
            return Err(OperationError::RunTerminal);
        }
        if inner.operations.contains_key(&operation.id) {
            return Err(OperationError::IdentifierCollision);
        }
        if operation
            .deadline
            .is_some_and(|deadline| deadline <= SystemTime::now())
        {
            return Err(OperationError::Expired);
        }
        inner.operations.insert(
            operation.id,
            OperationRecord {
                operation,
                state: OperationState::Pending,
                decision: None,
                notify: Arc::new(Notify::new()),
            },
        );
        Ok(())
    }

    pub fn snapshot(&self, run_id: &RunId) -> Result<Vec<OperationSnapshot>, OperationError> {
        let inner = self.lock()?;
        let mut snapshots = inner
            .operations
            .values()
            .filter(|record| &record.operation.run_id == run_id)
            .map(|record| OperationSnapshot {
                operation: record.operation.clone(),
                state: record.state.clone(),
            })
            .collect::<Vec<_>>();
        snapshots.sort_by_key(|snapshot| snapshot.operation.id.hyphenated());
        Ok(snapshots)
    }

    pub fn snapshot_operation(
        &self,
        run_id: &RunId,
        operation_id: &OperationId,
    ) -> Result<OperationSnapshot, OperationError> {
        let inner = self.lock()?;
        let record = Self::record_for_run(&inner, run_id, operation_id)?;
        Ok(OperationSnapshot {
            operation: record.operation.clone(),
            state: record.state.clone(),
        })
    }

    pub fn resolve(
        &self,
        run_id: &RunId,
        operation_id: &OperationId,
        decision: OperationDecision,
    ) -> Result<(), OperationError> {
        let notify = {
            let mut inner = self.lock()?;
            if inner.terminal_runs.contains(run_id) {
                return Err(OperationError::RunTerminal);
            }
            let record = Self::record_for_run_mut(&mut inner, run_id, operation_id)?;
            if record.state != OperationState::Pending {
                return Err(OperationError::AlreadyResolved);
            }
            if record.operation.mode == OperationMode::DeferredCommit
                && matches!(decision, OperationDecision::Approve { .. })
            {
                return Err(OperationError::DeferredCommitRequired);
            }
            record.state = match &decision {
                OperationDecision::Approve { .. } => OperationState::Approved,
                OperationDecision::Reject { .. } => OperationState::Rejected,
            };
            record.decision = Some(decision);
            record.notify.clone()
        };
        notify.notify_waiters();
        Ok(())
    }

    pub async fn wait(
        &self,
        run_id: &RunId,
        operation_id: &OperationId,
    ) -> Result<OperationDecision, OperationError> {
        loop {
            let (notified, state, decision, mode) = {
                let inner = self.lock()?;
                let record = Self::record_for_run(&inner, run_id, operation_id)?;
                (
                    record.notify.clone().notified_owned(),
                    record.state.clone(),
                    record.decision.clone(),
                    record.operation.mode,
                )
            };
            if mode != OperationMode::BlockingApproval {
                return Err(OperationError::BlockingWaitRequired);
            }
            match state {
                OperationState::Pending => notified.await,
                OperationState::Approved | OperationState::Rejected => {
                    return decision.ok_or(OperationError::InvalidState)
                }
                OperationState::Cancelled => return Err(OperationError::Cancelled),
                OperationState::Expired => return Err(OperationError::Expired),
                OperationState::Failed { message } => {
                    return Err(OperationError::CommitFailed(message))
                }
                OperationState::Committing | OperationState::Committed => {
                    return Err(OperationError::InvalidState)
                }
            }
        }
    }

    pub async fn wait_for_commit_or_reject(
        &self,
        run_id: &RunId,
        operation_id: &OperationId,
    ) -> Result<OperationDecision, OperationError> {
        loop {
            let (notified, state, decision, mode) = {
                let inner = self.lock()?;
                let record = Self::record_for_run(&inner, run_id, operation_id)?;
                (
                    record.notify.clone().notified_owned(),
                    record.state.clone(),
                    record.decision.clone(),
                    record.operation.mode,
                )
            };
            if mode != OperationMode::DeferredCommit {
                return Err(OperationError::DeferredCommitRequired);
            }
            match state {
                OperationState::Pending => notified.await,
                OperationState::Committing => return Ok(OperationDecision::Approve { data: None }),
                OperationState::Rejected => return decision.ok_or(OperationError::InvalidState),
                OperationState::Cancelled => return Err(OperationError::Cancelled),
                OperationState::Expired => return Err(OperationError::Expired),
                OperationState::Failed { message } => {
                    return Err(OperationError::CommitFailed(message))
                }
                OperationState::Approved | OperationState::Committed => {
                    return Err(OperationError::InvalidState)
                }
            }
        }
    }

    pub fn begin_commit(
        &self,
        run_id: &RunId,
        operation_id: &OperationId,
    ) -> Result<(), OperationError> {
        let notify = {
            let mut inner = self.lock()?;
            if inner.terminal_runs.contains(run_id) {
                return Err(OperationError::RunTerminal);
            }
            let record = Self::record_for_run_mut(&mut inner, run_id, operation_id)?;
            if record.operation.mode != OperationMode::DeferredCommit {
                return Err(OperationError::DeferredCommitRequired);
            }
            if record.state != OperationState::Pending {
                return Err(OperationError::AlreadyResolved);
            }
            record.state = OperationState::Committing;
            record.notify.clone()
        };
        notify.notify_waiters();
        Ok(())
    }

    pub fn finish_blocking_approval(
        &self,
        run_id: &RunId,
        operation_id: &OperationId,
        result: Result<(), String>,
    ) -> Result<(), OperationError> {
        let mut inner = self.lock()?;
        let record = Self::record_for_run_mut(&mut inner, run_id, operation_id)?;
        if record.operation.mode != OperationMode::BlockingApproval
            || record.state != OperationState::Approved
        {
            return Err(OperationError::InvalidState);
        }
        record.state = match result {
            Ok(()) => OperationState::Committed,
            Err(message) => OperationState::Failed { message },
        };
        Ok(())
    }

    pub fn finish_commit(
        &self,
        run_id: &RunId,
        operation_id: &OperationId,
        result: Result<(), String>,
    ) -> Result<(), OperationError> {
        let mut inner = self.lock()?;
        let record = Self::record_for_run_mut(&mut inner, run_id, operation_id)?;
        if record.state != OperationState::Committing {
            return Err(OperationError::InvalidState);
        }
        record.state = match result {
            Ok(()) => OperationState::Committed,
            Err(message) => OperationState::Failed { message },
        };
        Ok(())
    }

    pub fn cancel_run(&self, run_id: &RunId) -> Result<usize, OperationError> {
        let notifies = {
            let mut inner = self.lock()?;
            inner.terminal_runs.insert(*run_id);
            inner
                .operations
                .values_mut()
                .filter(|record| {
                    record.operation.run_id == *run_id
                        && matches!(
                            record.state,
                            OperationState::Pending | OperationState::Approved
                        )
                })
                .map(|record| {
                    record.state = OperationState::Cancelled;
                    record.notify.clone()
                })
                .collect::<Vec<_>>()
        };
        let count = notifies.len();
        for notify in notifies {
            notify.notify_waiters();
        }
        Ok(count)
    }

    pub fn expire(&self, now: SystemTime) -> Result<usize, OperationError> {
        let notifies = {
            let mut inner = self.lock()?;
            inner
                .operations
                .values_mut()
                .filter(|record| {
                    record.state == OperationState::Pending
                        && record
                            .operation
                            .deadline
                            .is_some_and(|deadline| deadline <= now)
                })
                .map(|record| {
                    record.state = OperationState::Expired;
                    record.notify.clone()
                })
                .collect::<Vec<_>>()
        };
        let count = notifies.len();
        for notify in notifies {
            notify.notify_waiters();
        }
        Ok(count)
    }

    pub fn mark_run_terminal(&self, run_id: &RunId) -> Result<usize, OperationError> {
        self.cancel_run(run_id)
    }

    pub fn remove(&self, run_id: &RunId, operation_id: &OperationId) -> Result<(), OperationError> {
        let notify = {
            let mut inner = self.lock()?;
            let record = Self::record_for_run(&inner, run_id, operation_id)?;
            let notify = record.notify.clone();
            inner.operations.remove(operation_id);
            notify
        };
        notify.notify_waiters();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aifluxon_core::{PendingOperationDraft, ToolEffect, ToolInvocationId};
    use serde_json::{json, Value};
    use std::time::Duration;

    fn operation(mode: OperationMode) -> PendingOperation {
        PendingOperationDraft {
            invocation_id: Some(ToolInvocationId::new()),
            effect: ToolEffect::ExternalSideEffect,
            mode,
            summary: "Confirm the prepared effect".to_string(),
            payload: json!({ "scope": "once" }),
            deadline: None,
        }
        .bind(OperationId::new(), RunId::new())
    }

    #[tokio::test]
    async fn blocking_approve_and_reject_wake_waiters_once() {
        for decision in [
            OperationDecision::Approve { data: None },
            OperationDecision::Reject {
                reason: Some("no".to_string()),
            },
        ] {
            let store = InMemoryOperationStore::new();
            let operation = operation(OperationMode::BlockingApproval);
            store.register(operation.clone()).unwrap();
            let waiter = {
                let store = store.clone();
                tokio::spawn(async move { store.wait(&operation.run_id, &operation.id).await })
            };
            tokio::task::yield_now().await;
            let snapshots = store.snapshot(&operation.run_id).unwrap();
            let operation_id = snapshots[0].operation.id;
            store
                .resolve(
                    &snapshots[0].operation.run_id,
                    &operation_id,
                    decision.clone(),
                )
                .unwrap();
            assert_eq!(waiter.await.unwrap().unwrap(), decision);
            assert_eq!(
                store
                    .resolve(
                        &snapshots[0].operation.run_id,
                        &operation_id,
                        OperationDecision::Reject { reason: None }
                    )
                    .unwrap_err(),
                OperationError::AlreadyResolved
            );
        }
    }

    #[tokio::test]
    async fn blocking_cancel_and_timeout_wake_waiters() {
        let cancelled_store = InMemoryOperationStore::new();
        let cancelled = operation(OperationMode::BlockingApproval);
        cancelled_store.register(cancelled.clone()).unwrap();
        let waiter = {
            let store = cancelled_store.clone();
            tokio::spawn(async move { store.wait(&cancelled.run_id, &cancelled.id).await })
        };
        tokio::task::yield_now().await;
        let run_id = cancelled_store.snapshot(&cancelled.run_id).unwrap()[0]
            .operation
            .run_id;
        cancelled_store.cancel_run(&run_id).unwrap();
        assert_eq!(
            waiter.await.unwrap().unwrap_err(),
            OperationError::Cancelled
        );

        let expired_store = InMemoryOperationStore::new();
        let mut expired = operation(OperationMode::BlockingApproval);
        expired.deadline = Some(SystemTime::now() + Duration::from_millis(20));
        expired_store.register(expired.clone()).unwrap();
        let waiter = {
            let store = expired_store.clone();
            tokio::spawn(async move { store.wait(&expired.run_id, &expired.id).await })
        };
        tokio::time::sleep(Duration::from_millis(25)).await;
        expired_store.expire(SystemTime::now()).unwrap();
        assert_eq!(waiter.await.unwrap().unwrap_err(), OperationError::Expired);
    }

    #[tokio::test]
    async fn listener_drop_does_not_resolve_or_remove_operation() {
        let store = InMemoryOperationStore::new();
        let operation = operation(OperationMode::BlockingApproval);
        store.register(operation.clone()).unwrap();
        let waiter = {
            let store = store.clone();
            let operation = operation.clone();
            tokio::spawn(async move { store.wait(&operation.run_id, &operation.id).await })
        };
        tokio::task::yield_now().await;
        waiter.abort();
        assert_eq!(
            store.snapshot(&operation.run_id).unwrap()[0].state,
            OperationState::Pending
        );
    }

    #[test]
    fn unknown_wrong_run_collision_and_terminal_run_are_rejected() {
        let store = InMemoryOperationStore::new();
        let operation = operation(OperationMode::BlockingApproval);
        store.register(operation.clone()).unwrap();
        assert_eq!(
            store.register(operation.clone()).unwrap_err(),
            OperationError::IdentifierCollision
        );
        assert_eq!(
            store
                .resolve(
                    &operation.run_id,
                    &OperationId::new(),
                    OperationDecision::Reject { reason: None }
                )
                .unwrap_err(),
            OperationError::UnknownOperation
        );
        assert_eq!(
            store
                .resolve(
                    &RunId::new(),
                    &operation.id,
                    OperationDecision::Reject { reason: None }
                )
                .unwrap_err(),
            OperationError::WrongRun
        );
        store.mark_run_terminal(&operation.run_id).unwrap();
        assert_eq!(
            store
                .resolve(
                    &operation.run_id,
                    &operation.id,
                    OperationDecision::Reject { reason: None }
                )
                .unwrap_err(),
            OperationError::RunTerminal
        );
        let new_operation = PendingOperationDraft {
            invocation_id: None,
            effect: ToolEffect::Unknown,
            mode: OperationMode::BlockingApproval,
            summary: "late".to_string(),
            payload: Value::Null,
            deadline: None,
        }
        .bind(OperationId::new(), operation.run_id);
        assert_eq!(
            store.register(new_operation).unwrap_err(),
            OperationError::RunTerminal
        );
    }

    #[tokio::test]
    async fn deferred_operation_never_uses_blocking_wait_or_plain_approval() {
        let store = InMemoryOperationStore::new();
        let operation = operation(OperationMode::DeferredCommit);
        store.register(operation.clone()).unwrap();
        assert_eq!(
            store
                .wait(&operation.run_id, &operation.id)
                .await
                .unwrap_err(),
            OperationError::BlockingWaitRequired
        );
        assert_eq!(
            store
                .resolve(
                    &operation.run_id,
                    &operation.id,
                    OperationDecision::Approve { data: None }
                )
                .unwrap_err(),
            OperationError::DeferredCommitRequired
        );
    }

    #[test]
    fn deferred_commit_is_at_most_once_and_reports_real_completion() {
        let store = InMemoryOperationStore::new();
        let operation = operation(OperationMode::DeferredCommit);
        store.register(operation.clone()).unwrap();
        store
            .begin_commit(&operation.run_id, &operation.id)
            .unwrap();
        assert_eq!(
            store
                .begin_commit(&operation.run_id, &operation.id)
                .unwrap_err(),
            OperationError::AlreadyResolved
        );
        store
            .finish_commit(&operation.run_id, &operation.id, Ok(()))
            .unwrap();
        assert_eq!(
            store.snapshot(&operation.run_id).unwrap()[0].state,
            OperationState::Committed
        );
    }

    #[test]
    fn commit_failure_is_not_committed_and_cancel_before_commit_prevents_it() {
        let failed_store = InMemoryOperationStore::new();
        let failed = operation(OperationMode::DeferredCommit);
        failed_store.register(failed.clone()).unwrap();
        failed_store
            .begin_commit(&failed.run_id, &failed.id)
            .unwrap();
        failed_store
            .finish_commit(&failed.run_id, &failed.id, Err("disk changed".to_string()))
            .unwrap();
        assert_eq!(
            failed_store.snapshot(&failed.run_id).unwrap()[0].state,
            OperationState::Failed {
                message: "disk changed".to_string()
            }
        );

        let cancelled_store = InMemoryOperationStore::new();
        let cancelled = operation(OperationMode::DeferredCommit);
        cancelled_store.register(cancelled.clone()).unwrap();
        cancelled_store.cancel_run(&cancelled.run_id).unwrap();
        assert_eq!(
            cancelled_store
                .begin_commit(&cancelled.run_id, &cancelled.id)
                .unwrap_err(),
            OperationError::RunTerminal
        );
    }
}
