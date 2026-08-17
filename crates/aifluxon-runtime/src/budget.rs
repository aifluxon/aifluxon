#![allow(dead_code)]

use aifluxon_core::RunLimits;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentBudgetExceeded {
    pub kind: AgentBudgetKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentBudgetKind {
    ModelRounds,
    ToolInvocations,
    WallTime,
    InputTokens,
    OutputTokens,
    ReasoningTokens,
}

#[derive(Clone, Debug)]
pub struct AgentLoopBudget {
    pub max_model_rounds: u32,
    pub max_tool_invocations: u32,
    pub max_wall_time: Duration,
    pub max_input_tokens: u64,
    pub max_output_tokens: u64,
    pub max_reasoning_tokens: u64,
    model_rounds: u32,
    tool_invocations: u32,
    input_tokens: u64,
    output_tokens: u64,
    reasoning_tokens: u64,
    started_at: Instant,
}

pub fn default_agent_loop_budget() -> AgentLoopBudget {
    AgentLoopBudget::with_tool_round_limit(64)
}

impl AgentLoopBudget {
    pub fn from_run_limits(limits: RunLimits) -> Self {
        Self {
            max_model_rounds: limits.max_model_rounds,
            max_tool_invocations: limits.max_tool_invocations,
            max_wall_time: Duration::from_secs(30 * 60),
            max_input_tokens: u64::MAX,
            max_output_tokens: u64::MAX,
            max_reasoning_tokens: u64::MAX,
            model_rounds: 0,
            tool_invocations: 0,
            input_tokens: 0,
            output_tokens: 0,
            reasoning_tokens: 0,
            started_at: Instant::now(),
        }
    }

    pub fn with_tool_round_limit(max_tool_invocations: u32) -> Self {
        Self::from_run_limits(RunLimits {
            max_model_rounds: max_tool_invocations,
            max_tool_invocations,
        })
    }

    pub fn consume_model_round(&mut self) -> Result<(), AgentBudgetExceeded> {
        if self.model_rounds >= self.max_model_rounds {
            return Err(AgentBudgetExceeded {
                kind: AgentBudgetKind::ModelRounds,
            });
        }
        self.model_rounds += 1;
        self.check_wall_time()
    }

    pub fn consume_tool_invocation(&mut self) -> Result<(), AgentBudgetExceeded> {
        if self.tool_invocations >= self.max_tool_invocations {
            return Err(AgentBudgetExceeded {
                kind: AgentBudgetKind::ToolInvocations,
            });
        }
        self.tool_invocations += 1;
        self.check_wall_time()
    }

    pub fn consume_tokens(
        &mut self,
        input: u64,
        output: u64,
        reasoning: u64,
    ) -> Result<(), AgentBudgetExceeded> {
        self.input_tokens = self.input_tokens.saturating_add(input);
        self.output_tokens = self.output_tokens.saturating_add(output);
        self.reasoning_tokens = self.reasoning_tokens.saturating_add(reasoning);
        if self.input_tokens > self.max_input_tokens {
            return Err(AgentBudgetExceeded {
                kind: AgentBudgetKind::InputTokens,
            });
        }
        if self.output_tokens > self.max_output_tokens {
            return Err(AgentBudgetExceeded {
                kind: AgentBudgetKind::OutputTokens,
            });
        }
        if self.reasoning_tokens > self.max_reasoning_tokens {
            return Err(AgentBudgetExceeded {
                kind: AgentBudgetKind::ReasoningTokens,
            });
        }
        self.check_wall_time()
    }

    pub fn tool_invocations(&self) -> u32 {
        self.tool_invocations
    }

    pub fn model_rounds(&self) -> u32 {
        self.model_rounds
    }

    fn check_wall_time(&self) -> Result<(), AgentBudgetExceeded> {
        if self.started_at.elapsed() > self.max_wall_time {
            return Err(AgentBudgetExceeded {
                kind: AgentBudgetKind::WallTime,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bug_agent_012_budget_is_monotonic_across_retries() {
        let mut budget = AgentLoopBudget::with_tool_round_limit(64);
        for _ in 0..64 {
            budget.consume_tool_invocation().unwrap();
        }
        assert_eq!(budget.tool_invocations(), 64);
        assert_eq!(
            budget.consume_tool_invocation().unwrap_err().kind,
            AgentBudgetKind::ToolInvocations
        );
        assert_eq!(
            budget.consume_tool_invocation().unwrap_err().kind,
            AgentBudgetKind::ToolInvocations
        );
        assert_eq!(budget.tool_invocations(), 64);
    }
}
