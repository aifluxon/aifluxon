use crate::result::{terminal_result, RunResult};
use crate::{AifluxonError, RunEvent, RunEventStream};
use aifluxon_core::{OperationSnapshot, RunContext, RunId, RunState};

#[derive(Debug)]
pub struct RunHandle {
    context: RunContext,
    events: RunEventStream,
}

impl RunHandle {
    pub(crate) fn new(context: RunContext, events: RunEventStream) -> Self {
        Self { context, events }
    }
    pub fn id(&self) -> RunId {
        self.context.run_id
    }

    pub fn context(&self) -> RunContext {
        self.context
    }

    pub fn events(&mut self) -> &mut RunEventStream {
        &mut self.events
    }

    pub fn into_events(self) -> RunEventStream {
        self.events
    }

    /// Waits for the canonical Runtime terminal and returns its result.
    ///
    /// This consumes remaining events on the handle. Hosts that also iterate
    /// `events()` should share one consumer rather than calling both.
    pub async fn result(&mut self) -> Result<RunResult, AifluxonError> {
        let mut output = Vec::new();
        let mut usage = None;
        let mut failure = None;
        let mut state = None;
        while let Some(envelope) = self.events.next().await {
            match envelope.event {
                RunEvent::UsageUpdated { usage: value } => usage = Some(value),
                RunEvent::Completed { output: messages } => {
                    output = messages;
                    state = Some(RunState::Completed);
                }
                RunEvent::Failed { message } => {
                    failure = Some(message);
                    state = Some(RunState::Failed);
                }
                RunEvent::Cancelled => state = Some(RunState::Cancelled),
                _ => {}
            }
        }
        terminal_result(
            self.context.run_id,
            self.context.session_id,
            state.unwrap_or(RunState::Failed),
            output,
            usage,
            failure,
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RunSnapshot {
    pub context: RunContext,
    pub state: RunState,
    pub last_event_sequence: u64,
    pub pending_operations: Vec<OperationSnapshot>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_exposes_only_generic_run_identity() {
        let context = RunContext {
            run_id: RunId::new(),
            session_id: None,
            parent_run_id: Some(RunId::new()),
        };
        let handle = RunHandle {
            context,
            events: RunEventStream::closed(),
        };
        assert_eq!(handle.id(), context.run_id);
        assert_eq!(handle.context(), context);
    }
}
