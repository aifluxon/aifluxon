#![allow(dead_code)]

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum AgentTerminalOutcome {
    Completed = 1,
    Failed = 2,
    Cancelled = 3,
    BudgetExceeded = 4,
}

#[derive(Clone, Default)]
pub struct AgentTerminalGuard {
    inner: Arc<AtomicU8>,
}

impl AgentTerminalGuard {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn try_enter(&self, outcome: AgentTerminalOutcome) -> bool {
        self.inner
            .compare_exchange(0, outcome as u8, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    pub fn outcome(&self) -> Option<AgentTerminalOutcome> {
        match self.inner.load(Ordering::SeqCst) {
            1 => Some(AgentTerminalOutcome::Completed),
            2 => Some(AgentTerminalOutcome::Failed),
            3 => Some(AgentTerminalOutcome::Cancelled),
            4 => Some(AgentTerminalOutcome::BudgetExceeded),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bug_agent_006_terminal_signal_is_emitted_once() {
        let cases = [
            (
                AgentTerminalOutcome::Completed,
                AgentTerminalOutcome::Cancelled,
            ),
            (
                AgentTerminalOutcome::Failed,
                AgentTerminalOutcome::Cancelled,
            ),
            (
                AgentTerminalOutcome::Cancelled,
                AgentTerminalOutcome::Completed,
            ),
            (
                AgentTerminalOutcome::BudgetExceeded,
                AgentTerminalOutcome::Completed,
            ),
        ];
        for (first, second) in cases {
            let guard = AgentTerminalGuard::new();
            assert!(guard.try_enter(first));
            assert!(!guard.try_enter(second));
            assert!(!guard.try_enter(first));
            assert_eq!(guard.outcome(), Some(first));
        }
    }
}
