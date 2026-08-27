//! AgentContext: what agents see when they query workspace state + history.
//!
//! This module provides the structured context that agents can request via the
//! `workspace.context` UHP method to understand the current state and recent
//! history of a workspace.

use serde::{Deserialize, Serialize};

use super::operations::{OpType, StateOp};

/// Context about a workspace that agents can query.
///
/// Includes both the current state snapshot and recent operation history,
/// allowing agents to reason about what has happened and what is happening.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentContext {
    /// The workspace id this context is for.
    pub workspace_id: String,
    /// Current state snapshot (panes, agents, files, etc.).
    pub current_state: Vec<serde_json::Value>,
    /// Recent operations (full history is queryable separately).
    pub recent_changes: Vec<StateOp>,
    /// Summary statistics about this context.
    #[serde(default)]
    pub stats: ContextStats,
}

/// Summary statistics for agent context.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ContextStats {
    /// Number of active panes in the workspace.
    pub active_panes: usize,
    /// Number of active agents detected.
    pub active_agents: usize,
    /// Number of operations in the recent window.
    pub recent_op_count: usize,
    /// Total tokens used across all agents (if tracked).
    pub total_tokens: Option<u64>,
    /// Total cost across all agents (if tracked).
    pub total_cost: Option<f64>,
}

impl AgentContext {
    /// Create a new agent context for a workspace.
    pub fn new(workspace_id: String) -> Self {
        Self {
            workspace_id,
            current_state: Vec::new(),
            recent_changes: Vec::new(),
            stats: ContextStats::default(),
        }
    }

    /// Get the list of active agents from the current state.
    pub fn active_agents(&self) -> Vec<String> {
        self.current_state
            .iter()
            .filter_map(|v| v.get("agent").and_then(|a| a.as_str()).map(String::from))
            .collect()
    }

    /// Get operations where a task completed.
    pub fn completed_tasks(&self) -> Vec<&StateOp> {
        self.recent_changes
            .iter()
            .filter(|op| matches!(&op.op_type, OpType::TaskCompleted { .. }))
            .collect()
    }

    /// Get operations where an agent's status changed.
    pub fn agent_status_changes(&self) -> Vec<&StateOp> {
        self.recent_changes
            .iter()
            .filter(|op| matches!(&op.op_type, OpType::AgentStatusChanged { .. }))
            .collect()
    }

    /// Get operations where a pane was created.
    pub fn pane_creations(&self) -> Vec<&StateOp> {
        self.recent_changes
            .iter()
            .filter(|op| matches!(&op.op_type, OpType::PaneCreated { .. }))
            .collect()
    }

    /// Get the most recent operation of any type.
    pub fn latest_operation(&self) -> Option<&StateOp> {
        self.recent_changes.last()
    }

    /// Human-readable summary of the context.
    pub fn summary(&self) -> String {
        format!(
            "{} state items, {} recent operations, {} active agents",
            self.current_state.len(),
            self.recent_changes.len(),
            self.stats.active_agents
        )
    }

    /// Check if there are any blocked agents.
    pub fn has_blocked_agents(&self) -> bool {
        self.recent_changes.iter().any(|op| {
            matches!(
                &op.op_type,
                OpType::AgentStatusChanged { new, .. } if new == "blocked"
            )
        })
    }

    /// Get the total token usage from recent operations.
    pub fn token_usage(&self) -> u64 {
        self.recent_changes
            .iter()
            .filter_map(|op| {
                if let OpType::AgentTokensUsed { tokens, .. } = &op.op_type {
                    Some(*tokens)
                } else {
                    None
                }
            })
            .sum()
    }

    /// Get the total cost from recent operations.
    pub fn cost(&self) -> f64 {
        self.recent_changes
            .iter()
            .filter_map(|op| {
                if let OpType::AgentTokensUsed { cost, .. } = &op.op_type {
                    Some(*cost)
                } else {
                    None
                }
            })
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state_db::operations::EntityType;

    #[test]
    fn test_agent_context_creation() {
        let ctx = AgentContext::new("ws-1".to_string());
        assert_eq!(ctx.workspace_id, "ws-1");
        assert!(ctx.current_state.is_empty());
        assert!(ctx.recent_changes.is_empty());
    }

    #[test]
    fn test_completed_tasks() {
        let mut ctx = AgentContext::new("ws-1".to_string());
        ctx.recent_changes.push(StateOp::new(
            "task-1".to_string(),
            EntityType::Task,
            OpType::TaskCompleted {
                task_id: "task-1".to_string(),
            },
        ));
        ctx.recent_changes.push(StateOp::new(
            "pane-1".to_string(),
            EntityType::Pane,
            OpType::PaneCreated {
                agent: None,
                cwd: "/tmp".to_string(),
            },
        ));

        let completed = ctx.completed_tasks();
        assert_eq!(completed.len(), 1);
    }

    #[test]
    fn test_token_usage() {
        let mut ctx = AgentContext::new("ws-1".to_string());
        ctx.recent_changes.push(StateOp::new(
            "agent-1".to_string(),
            EntityType::Agent,
            OpType::AgentTokensUsed {
                tokens: 1000,
                cost: 0.01,
            },
        ));
        ctx.recent_changes.push(StateOp::new(
            "agent-1".to_string(),
            EntityType::Agent,
            OpType::AgentTokensUsed {
                tokens: 500,
                cost: 0.005,
            },
        ));

        assert_eq!(ctx.token_usage(), 1500);
        assert!((ctx.cost() - 0.015).abs() < f64::EPSILON);
    }
}
