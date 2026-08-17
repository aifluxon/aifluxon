use crate::budget::AgentLoopBudget;
use crate::operations::{InMemoryOperationStore, OperationError};
use aifluxon_core::{OperationSnapshot, RunId};

pub fn snapshot_operations(
    store: &InMemoryOperationStore,
    run_id: &RunId,
) -> Result<Vec<OperationSnapshot>, OperationError> {
    store.snapshot(run_id)
}

#[allow(dead_code)]
pub fn default_runtime_budget() -> AgentLoopBudget {
    crate::budget::default_agent_loop_budget()
}
