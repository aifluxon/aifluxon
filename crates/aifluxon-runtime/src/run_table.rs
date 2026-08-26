use crate::{
    AgentBudgetExceeded, AgentLoopBudget, InMemoryOperationStore, OperationError, ToolLedger,
};
use aifluxon_core::{
    Message, OperationSnapshot, RunContext, RunEvent, RunEventEnvelope, RunId, RunLimits, RunState,
    SessionId, ToolInvocationId,
};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::sync::Notify;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RunTableError {
    IdentifierCollision,
    UnknownRun,
    RunTerminal,
    InvalidTransition { from: RunState, to: RunState },
    PendingOperations,
    StoreUnavailable,
    Operation(OperationError),
}

impl RunTableError {
    pub fn message(&self) -> String {
        match self {
            Self::IdentifierCollision => "Run identifier collision.".to_string(),
            Self::UnknownRun => "Run was not found.".to_string(),
            Self::RunTerminal => "Run is already terminal.".to_string(),
            Self::InvalidTransition { from, to } => {
                format!("Invalid run state transition from {from:?} to {to:?}.")
            }
            Self::PendingOperations => {
                "Run cannot finish while operations are still pending.".to_string()
            }
            Self::StoreUnavailable => "Run table is unavailable.".to_string(),
            Self::Operation(error) => error.message(),
        }
    }
}

impl From<OperationError> for RunTableError {
    fn from(value: OperationError) -> Self {
        Self::Operation(value)
    }
}

#[derive(Clone, Default)]
pub struct RunCancellationToken {
    cancelled: Arc<AtomicBool>,
    notify: Arc<Notify>,
}

impl RunCancellationToken {
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub async fn cancelled(&self) {
        loop {
            let notified = self.notify.notified();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }

    fn cancel(&self) -> bool {
        if self.cancelled.swap(true, Ordering::SeqCst) {
            return false;
        }
        self.notify.notify_waiters();
        true
    }
}

pub trait RunCancellationHook: Send + Sync {
    fn on_run_cancelled(&self, run_id: &RunId) -> Result<(), String>;
}

#[derive(Clone, Debug, PartialEq)]
pub struct CanonicalRunSnapshot {
    pub context: RunContext,
    pub state: RunState,
    pub last_event_sequence: u64,
    pub pending_operations: Vec<OperationSnapshot>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CancellationReport {
    pub snapshot: CanonicalRunSnapshot,
    pub hook_errors: Vec<String>,
}

struct RunRecord {
    context: RunContext,
    state: RunState,
    cancellation: RunCancellationToken,
    budget: Arc<Mutex<AgentLoopBudget>>,
    ledger: Arc<ToolLedger>,
    hooks: Vec<Arc<dyn RunCancellationHook>>,
    events: Vec<RunEventEnvelope>,
}

#[derive(Default)]
struct RunTableInner {
    runs: HashMap<RunId, RunRecord>,
}

#[derive(Clone, Default)]
pub struct RunTable {
    inner: Arc<Mutex<RunTableInner>>,
    operations: InMemoryOperationStore,
    event_notify: Arc<Notify>,
}

impl RunTable {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> Result<MutexGuard<'_, RunTableInner>, RunTableError> {
        self.inner
            .lock()
            .map_err(|_| RunTableError::StoreUnavailable)
    }

    pub fn start(
        &self,
        session_id: Option<SessionId>,
        parent_run_id: Option<RunId>,
        limits: RunLimits,
    ) -> Result<RunContext, RunTableError> {
        self.register(
            RunContext {
                run_id: RunId::new(),
                session_id,
                parent_run_id,
            },
            limits,
        )
    }

    pub fn register(
        &self,
        context: RunContext,
        limits: RunLimits,
    ) -> Result<RunContext, RunTableError> {
        let started = RunEventEnvelope {
            sequence: 1,
            run_id: context.run_id,
            event: RunEvent::RunStarted { context },
        };
        let mut inner = self.lock()?;
        if inner.runs.contains_key(&context.run_id) {
            return Err(RunTableError::IdentifierCollision);
        }
        inner.runs.insert(
            context.run_id,
            RunRecord {
                context,
                state: RunState::Running,
                cancellation: RunCancellationToken::default(),
                budget: Arc::new(Mutex::new(AgentLoopBudget::from_run_limits(limits))),
                ledger: Arc::new(ToolLedger::new()),
                hooks: Vec::new(),
                events: vec![started],
            },
        );
        drop(inner);
        self.wake_event_waiters();
        Ok(context)
    }

    pub fn event_notify(&self) -> Arc<Notify> {
        self.event_notify.clone()
    }

    fn wake_event_waiters(&self) {
        self.event_notify.notify_waiters();
    }

    pub fn contains(&self, run_id: &RunId) -> Result<bool, RunTableError> {
        Ok(self.lock()?.runs.contains_key(run_id))
    }

    pub fn context(&self, run_id: &RunId) -> Result<RunContext, RunTableError> {
        self.lock()?
            .runs
            .get(run_id)
            .map(|record| record.context)
            .ok_or(RunTableError::UnknownRun)
    }

    pub fn cancellation_token(
        &self,
        run_id: &RunId,
    ) -> Result<RunCancellationToken, RunTableError> {
        self.lock()?
            .runs
            .get(run_id)
            .map(|record| record.cancellation.clone())
            .ok_or(RunTableError::UnknownRun)
    }

    pub fn register_cancellation_hook(
        &self,
        run_id: &RunId,
        hook: Arc<dyn RunCancellationHook>,
    ) -> Result<(), RunTableError> {
        let mut inner = self.lock()?;
        let record = inner
            .runs
            .get_mut(run_id)
            .ok_or(RunTableError::UnknownRun)?;
        if is_terminal(record.state) {
            return Err(RunTableError::RunTerminal);
        }
        record.hooks.push(hook);
        Ok(())
    }

    pub fn operations(&self) -> &InMemoryOperationStore {
        &self.operations
    }

    pub fn set_state(&self, run_id: &RunId, state: RunState) -> Result<u64, RunTableError> {
        if is_terminal(state) {
            return Err(RunTableError::InvalidTransition {
                from: self.snapshot(run_id)?.state,
                to: state,
            });
        }
        let mut inner = self.lock()?;
        let record = inner
            .runs
            .get_mut(run_id)
            .ok_or(RunTableError::UnknownRun)?;
        if is_terminal(record.state) {
            return Err(RunTableError::RunTerminal);
        }
        if !matches!(
            (record.state, state),
            (RunState::Running, RunState::Running)
                | (RunState::Running, RunState::AwaitingOperation)
                | (RunState::AwaitingOperation, RunState::AwaitingOperation)
                | (RunState::AwaitingOperation, RunState::Running)
        ) {
            return Err(RunTableError::InvalidTransition {
                from: record.state,
                to: state,
            });
        }
        record.state = state;
        let sequence = append_event(record, RunEvent::StateChanged { state })?;
        drop(inner);
        self.wake_event_waiters();
        Ok(sequence)
    }

    pub fn emit(&self, run_id: &RunId, event: RunEvent) -> Result<u64, RunTableError> {
        if event.terminal_state().is_some() || matches!(event, RunEvent::RunStarted { .. }) {
            return Err(RunTableError::InvalidTransition {
                from: self.snapshot(run_id)?.state,
                to: event.terminal_state().unwrap_or(RunState::Running),
            });
        }
        let mut inner = self.lock()?;
        let record = inner
            .runs
            .get_mut(run_id)
            .ok_or(RunTableError::UnknownRun)?;
        if is_terminal(record.state) {
            return Err(RunTableError::RunTerminal);
        }
        let sequence = append_event(record, event)?;
        drop(inner);
        self.wake_event_waiters();
        Ok(sequence)
    }

    pub fn finish_completed(
        &self,
        run_id: &RunId,
        output: Vec<Message>,
    ) -> Result<u64, RunTableError> {
        self.finish(run_id, RunEvent::Completed { output })
    }

    pub fn finish_failed(
        &self,
        run_id: &RunId,
        message: impl Into<String>,
    ) -> Result<u64, RunTableError> {
        self.finish(
            run_id,
            RunEvent::Failed {
                message: message.into(),
            },
        )
    }

    fn finish(&self, run_id: &RunId, event: RunEvent) -> Result<u64, RunTableError> {
        if self
            .operations
            .snapshot(run_id)?
            .iter()
            .any(|snapshot| !snapshot.state.is_terminal())
        {
            return Err(RunTableError::PendingOperations);
        }
        let state = event
            .terminal_state()
            .expect("finish accepts only terminal events");
        let sequence = {
            let mut inner = self.lock()?;
            let record = inner
                .runs
                .get_mut(run_id)
                .ok_or(RunTableError::UnknownRun)?;
            if is_terminal(record.state) {
                return Err(RunTableError::RunTerminal);
            }
            record.state = state;
            append_event(record, event)?
        };
        self.wake_event_waiters();
        self.operations.mark_run_terminal(run_id)?;
        Ok(sequence)
    }

    pub fn cancel(&self, run_id: &RunId) -> Result<CancellationReport, RunTableError> {
        let hooks = {
            let mut inner = self.lock()?;
            let record = inner
                .runs
                .get_mut(run_id)
                .ok_or(RunTableError::UnknownRun)?;
            if is_terminal(record.state) {
                return Err(RunTableError::RunTerminal);
            }
            record.state = RunState::Cancelled;
            record.cancellation.cancel();
            append_event(record, RunEvent::Cancelled)?;
            std::mem::take(&mut record.hooks)
        };
        self.wake_event_waiters();
        self.operations.cancel_run(run_id)?;
        let mut hook_errors = Vec::new();
        for hook in hooks {
            if let Err(error) = hook.on_run_cancelled(run_id) {
                hook_errors.push(error);
            }
        }
        Ok(CancellationReport {
            snapshot: self.snapshot(run_id)?,
            hook_errors,
        })
    }

    pub fn cancel_tree(
        &self,
        root_run_id: &RunId,
    ) -> Result<Vec<CancellationReport>, RunTableError> {
        if !self.contains(root_run_id)? {
            return Err(RunTableError::UnknownRun);
        }
        let targets = {
            let inner = self.lock()?;
            let mut pending = vec![*root_run_id];
            let mut targets = Vec::new();
            while let Some(parent) = pending.pop() {
                if targets.contains(&parent) {
                    continue;
                }
                targets.push(parent);
                pending.extend(inner.runs.values().filter_map(|record| {
                    (record.context.parent_run_id == Some(parent)).then_some(record.context.run_id)
                }));
            }
            targets
        };
        let mut reports = Vec::new();
        for target in targets {
            match self.cancel(&target) {
                Ok(report) => reports.push(report),
                Err(RunTableError::RunTerminal) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(reports)
    }

    pub fn consume_model_round(&self, run_id: &RunId) -> Result<(), AgentBudgetExceeded> {
        let budget = self.budget(run_id).map_err(|_| AgentBudgetExceeded {
            kind: crate::AgentBudgetKind::ModelRounds,
        })?;
        let result = budget
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .consume_model_round();
        result
    }

    pub fn consume_tool_invocation(&self, run_id: &RunId) -> Result<(), AgentBudgetExceeded> {
        let budget = self.budget(run_id).map_err(|_| AgentBudgetExceeded {
            kind: crate::AgentBudgetKind::ToolInvocations,
        })?;
        let result = budget
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .consume_tool_invocation();
        result
    }

    fn budget(&self, run_id: &RunId) -> Result<Arc<Mutex<AgentLoopBudget>>, RunTableError> {
        self.lock()?
            .runs
            .get(run_id)
            .map(|record| record.budget.clone())
            .ok_or(RunTableError::UnknownRun)
    }

    pub fn execute_tool_once(
        &self,
        run_id: &RunId,
        invocation_id: ToolInvocationId,
        execute: impl FnOnce() -> Value,
    ) -> Result<Value, RunTableError> {
        let ledger = self
            .lock()?
            .runs
            .get(run_id)
            .map(|record| record.ledger.clone())
            .ok_or(RunTableError::UnknownRun)?;
        ledger
            .execute_once(invocation_id, || {
                Ok(crate::ToolResult::from_value(execute()))
            })
            .map(|result| result.value)
            .map_err(|_| RunTableError::StoreUnavailable)
    }

    pub fn cached_tool_result(
        &self,
        run_id: &RunId,
        invocation_id: &ToolInvocationId,
    ) -> Result<Option<Value>, RunTableError> {
        let ledger = self
            .lock()?
            .runs
            .get(run_id)
            .map(|record| record.ledger.clone())
            .ok_or(RunTableError::UnknownRun)?;
        Ok(ledger.get(invocation_id).map(|result| result.value))
    }

    pub(crate) fn cached_tool_execution(
        &self,
        run_id: &RunId,
        invocation_id: &ToolInvocationId,
    ) -> Result<Option<crate::ToolResult>, RunTableError> {
        let ledger = self
            .lock()?
            .runs
            .get(run_id)
            .map(|record| record.ledger.clone())
            .ok_or(RunTableError::UnknownRun)?;
        Ok(ledger.get(invocation_id))
    }

    pub fn record_tool_result(
        &self,
        run_id: &RunId,
        invocation_id: ToolInvocationId,
        value: Value,
    ) -> Result<(), RunTableError> {
        let ledger = self
            .lock()?
            .runs
            .get(run_id)
            .map(|record| record.ledger.clone())
            .ok_or(RunTableError::UnknownRun)?;
        ledger.record(invocation_id, crate::ToolResult::from_value(value));
        Ok(())
    }

    pub(crate) fn record_tool_execution(
        &self,
        run_id: &RunId,
        invocation_id: ToolInvocationId,
        result: crate::ToolResult,
    ) -> Result<(), RunTableError> {
        let ledger = self
            .lock()?
            .runs
            .get(run_id)
            .map(|record| record.ledger.clone())
            .ok_or(RunTableError::UnknownRun)?;
        ledger.record(invocation_id, result);
        Ok(())
    }

    pub fn tool_execution_count(&self, run_id: &RunId) -> Result<usize, RunTableError> {
        let ledger = self
            .lock()?
            .runs
            .get(run_id)
            .map(|record| record.ledger.clone())
            .ok_or(RunTableError::UnknownRun)?;
        Ok(ledger.execution_count())
    }

    pub fn model_rounds(&self, run_id: &RunId) -> Result<u32, RunTableError> {
        let budget = self.budget(run_id)?;
        let rounds = budget
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .model_rounds();
        Ok(rounds)
    }

    pub fn snapshot(&self, run_id: &RunId) -> Result<CanonicalRunSnapshot, RunTableError> {
        let (context, state, last_event_sequence) = {
            let inner = self.lock()?;
            let record = inner.runs.get(run_id).ok_or(RunTableError::UnknownRun)?;
            (
                record.context,
                record.state,
                record
                    .events
                    .last()
                    .map(|event| event.sequence)
                    .unwrap_or(0),
            )
        };
        Ok(CanonicalRunSnapshot {
            context,
            state,
            last_event_sequence,
            pending_operations: self.operations.snapshot(run_id)?,
        })
    }

    pub fn events_since(
        &self,
        run_id: &RunId,
        sequence: u64,
    ) -> Result<Vec<RunEventEnvelope>, RunTableError> {
        let inner = self.lock()?;
        let record = inner.runs.get(run_id).ok_or(RunTableError::UnknownRun)?;
        Ok(record
            .events
            .iter()
            .filter(|event| event.sequence > sequence)
            .cloned()
            .collect())
    }
}

fn append_event(record: &mut RunRecord, event: RunEvent) -> Result<u64, RunTableError> {
    if record
        .events
        .last()
        .is_some_and(|last| last.event.terminal_state().is_some())
    {
        return Err(RunTableError::RunTerminal);
    }
    let sequence = record
        .events
        .last()
        .map(|event| event.sequence.saturating_add(1))
        .unwrap_or(1);
    record.events.push(RunEventEnvelope {
        sequence,
        run_id: record.context.run_id,
        event,
    });
    Ok(sequence)
}

fn is_terminal(state: RunState) -> bool {
    matches!(
        state,
        RunState::Completed | RunState::Failed | RunState::Cancelled
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use aifluxon_core::{OperationId, OperationMode, PendingOperationDraft, ToolEffect};
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;

    fn limits() -> RunLimits {
        RunLimits {
            max_model_rounds: 2,
            max_tool_invocations: 2,
        }
    }

    #[test]
    fn session_and_run_identity_are_distinct_and_session_is_stable_across_runs() {
        let table = RunTable::new();
        let session = SessionId::new();
        let first = table.start(Some(session), None, limits()).unwrap();
        let second = table.start(Some(session), None, limits()).unwrap();
        let temporary = table.start(None, None, limits()).unwrap();

        assert_ne!(first.run_id, second.run_id);
        assert_eq!(first.session_id, second.session_id);
        assert_eq!(temporary.session_id, None);
    }

    #[test]
    fn exact_lookup_does_not_accept_prefixes() {
        let table = RunTable::new();
        let context = table.start(None, None, limits()).unwrap();
        assert!(table.context(&context.run_id).is_ok());
        assert_eq!(table.context(&RunId::new()), Err(RunTableError::UnknownRun));
    }

    #[test]
    fn run_owned_budget_and_typed_ledger_are_monotonic_and_at_most_once() {
        let table = RunTable::new();
        let context = table.start(None, None, limits()).unwrap();
        table.consume_model_round(&context.run_id).unwrap();
        table.consume_model_round(&context.run_id).unwrap();
        assert!(table.consume_model_round(&context.run_id).is_err());

        let calls = AtomicUsize::new(0);
        let invocation = ToolInvocationId::from_stable_key("provider-call-1");
        let first = table
            .execute_tool_once(&context.run_id, invocation, || {
                calls.fetch_add(1, Ordering::SeqCst);
                serde_json::json!({ "ok": true })
            })
            .unwrap();
        let replay = table
            .execute_tool_once(&context.run_id, invocation, || {
                calls.fetch_add(1, Ordering::SeqCst);
                serde_json::json!({ "ok": false })
            })
            .unwrap();
        assert_eq!(first, replay);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    struct CountingHook {
        calls: Arc<AtomicUsize>,
        fail: bool,
    }

    impl RunCancellationHook for CountingHook {
        fn on_run_cancelled(&self, _run_id: &RunId) -> Result<(), String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                Err("cleanup failed".to_string())
            } else {
                Ok(())
            }
        }
    }

    #[tokio::test]
    async fn cancel_wakes_provider_and_operation_waiters_and_calls_hook_once() {
        let table = RunTable::new();
        let context = table.start(None, None, limits()).unwrap();
        let token = table.cancellation_token(&context.run_id).unwrap();
        let operation = PendingOperationDraft {
            invocation_id: None,
            effect: ToolEffect::ExternalSideEffect,
            mode: OperationMode::BlockingApproval,
            summary: "confirm".to_string(),
            payload: Value::Null,
            deadline: None,
        }
        .bind(OperationId::new(), context.run_id);
        table.operations().register(operation.clone()).unwrap();
        let operation_wait = {
            let store = table.operations().clone();
            tokio::spawn(async move { store.wait(&operation.run_id, &operation.id).await })
        };
        let provider_wait = tokio::spawn(async move { token.cancelled().await });
        let calls = Arc::new(AtomicUsize::new(0));
        table
            .register_cancellation_hook(
                &context.run_id,
                Arc::new(CountingHook {
                    calls: calls.clone(),
                    fail: true,
                }),
            )
            .unwrap();

        let report = table.cancel(&context.run_id).unwrap();
        tokio::time::timeout(Duration::from_secs(1), provider_wait)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            operation_wait.await.unwrap(),
            Err(OperationError::Cancelled)
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(report.hook_errors, vec!["cleanup failed"]);
        assert_eq!(report.snapshot.state, RunState::Cancelled);
        assert_eq!(
            table.cancel(&context.run_id),
            Err(RunTableError::RunTerminal)
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn child_cancellation_is_generic_and_recursive() {
        let table = RunTable::new();
        let parent = table.start(None, None, limits()).unwrap();
        let child = table.start(None, Some(parent.run_id), limits()).unwrap();
        let grandchild = table.start(None, Some(child.run_id), limits()).unwrap();
        let other = table.start(None, None, limits()).unwrap();

        let reports = table.cancel_tree(&parent.run_id).unwrap();
        assert_eq!(reports.len(), 3);
        assert_eq!(
            table.snapshot(&grandchild.run_id).unwrap().state,
            RunState::Cancelled
        );
        assert_eq!(
            table.snapshot(&other.run_id).unwrap().state,
            RunState::Running
        );
    }

    #[test]
    fn event_sequence_is_monotonic_terminal_is_final_and_pending_blocks_completion() {
        let table = RunTable::new();
        let context = table.start(None, None, limits()).unwrap();
        assert_eq!(
            table
                .emit(
                    &context.run_id,
                    RunEvent::ModelDelta {
                        delta: "a".to_string(),
                    },
                )
                .unwrap(),
            2
        );
        let operation = PendingOperationDraft {
            invocation_id: None,
            effect: ToolEffect::FsWrite,
            mode: OperationMode::DeferredCommit,
            summary: "prepared".to_string(),
            payload: Value::Null,
            deadline: None,
        }
        .bind(OperationId::new(), context.run_id);
        table.operations().register(operation.clone()).unwrap();
        assert_eq!(
            table.finish_completed(&context.run_id, Vec::new()),
            Err(RunTableError::PendingOperations)
        );
        table
            .operations()
            .begin_commit(&context.run_id, &operation.id)
            .unwrap();
        table
            .operations()
            .finish_commit(&context.run_id, &operation.id, Ok(()))
            .unwrap();
        assert_eq!(
            table.finish_completed(&context.run_id, Vec::new()).unwrap(),
            3
        );
        assert_eq!(
            table.emit(
                &context.run_id,
                RunEvent::ModelDelta {
                    delta: "late".to_string(),
                }
            ),
            Err(RunTableError::RunTerminal)
        );
        let events = table.events_since(&context.run_id, 0).unwrap();
        assert_eq!(
            events
                .iter()
                .map(|event| event.sequence)
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert!(events.last().unwrap().event.terminal_state().is_some());
        assert_eq!(
            table.snapshot(&context.run_id).unwrap().last_event_sequence,
            3
        );
    }
}
