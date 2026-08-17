use crate::{
    ArtifactRef, Message, PendingOperation, RunContext, RunId, RunState, ToolInvocationId,
};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub enum RunEvent {
    RunStarted {
        context: RunContext,
    },
    StateChanged {
        state: RunState,
    },
    ModelDelta {
        delta: String,
    },
    ReasoningDelta {
        delta: String,
    },
    ToolStarted {
        invocation_id: ToolInvocationId,
        name: String,
        arguments: Value,
    },
    ToolFinished {
        invocation_id: ToolInvocationId,
        name: String,
        result: Value,
    },
    OperationRequested {
        operation: PendingOperation,
    },
    UsageUpdated {
        usage: Value,
    },
    ArtifactProduced {
        artifact: ArtifactRef,
    },
    Completed {
        output: Vec<Message>,
    },
    Failed {
        message: String,
    },
    Cancelled,
}

impl RunEvent {
    pub fn terminal_state(&self) -> Option<RunState> {
        match self {
            Self::Completed { .. } => Some(RunState::Completed),
            Self::Failed { .. } => Some(RunState::Failed),
            Self::Cancelled => Some(RunState::Cancelled),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RunEventEnvelope {
    pub sequence: u64,
    pub run_id: RunId,
    pub event: RunEvent,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_terminal_events_map_to_terminal_states() {
        assert_eq!(
            RunEvent::Cancelled.terminal_state(),
            Some(RunState::Cancelled)
        );
        assert_eq!(
            RunEvent::Completed { output: Vec::new() }.terminal_state(),
            Some(RunState::Completed)
        );
        assert_eq!(
            RunEvent::ModelDelta {
                delta: "x".to_string(),
            }
            .terminal_state(),
            None
        );
    }
}
