//! StateOp: every mutation in Luvus as a queryable operation.
//!
//! State operations are the atomic units of change in the state graph. Each
//! operation records what changed, when, and to which entity.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A single state operation in the Luvus state graph.
///
/// Every mutation (pane creation, agent status change, workspace rename, etc.)
/// is recorded as a `StateOp` that agents can query for full context.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StateOp {
    /// Unique operation identifier.
    pub id: Uuid,
    /// The entity this operation targets (workspace/pane/agent id).
    pub entity_id: String,
    /// What kind of entity this is.
    pub entity_type: EntityType,
    /// What kind of operation this is.
    pub op_type: OpType,
    /// When this operation occurred.
    pub timestamp: DateTime<Utc>,
    /// Additional operation payload.
    pub data: serde_json::Value,
}

impl StateOp {
    /// Create a new state operation with auto-generated id and timestamp.
    pub fn new(entity_id: String, entity_type: EntityType, op_type: OpType) -> Self {
        Self {
            id: Uuid::new_v4(),
            entity_id,
            entity_type,
            op_type,
            timestamp: Utc::now(),
            data: serde_json::Value::Null,
        }
    }

    /// Create a new state operation with custom data payload.
    pub fn with_data(
        entity_id: String,
        entity_type: EntityType,
        op_type: OpType,
        data: serde_json::Value,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            entity_id,
            entity_type,
            op_type,
            timestamp: Utc::now(),
            data,
        }
    }
}

/// The type of entity a state operation targets.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum EntityType {
    Workspace,
    Pane,
    Tab,
    Agent,
    File,
    Task,
}

/// The specific operation type.
///
/// Each variant captures the semantic meaning of the mutation along with any
/// relevant metadata needed for agent context queries.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OpType {
    // ── Workspace operations ────────────────────────────────────────────────
    WorkspaceCreated {
        name: String,
        cwd: String,
    },
    WorkspaceRenamed {
        old: String,
        new: String,
    },
    WorkspacePinned,
    WorkspaceUnpinned,
    WorkspaceDeleted,
    WorkspaceFocused,

    // ── Pane operations ─────────────────────────────────────────────────────
    PaneCreated {
        agent: Option<String>,
        cwd: String,
    },
    PaneStatusChanged {
        old: String,
        new: String,
    },
    PaneNameSet {
        name: String,
    },
    PaneClosed {
        exit_code: Option<i32>,
    },
    PaneFocused,
    PaneSplit {
        direction: String,
    },

    // ── Agent operations ────────────────────────────────────────────────────
    AgentDetected {
        name: String,
        session_id: String,
    },
    AgentStatusChanged {
        old: String,
        new: String,
    },
    AgentTokensUsed {
        tokens: u64,
        cost: f64,
    },
    AgentSessionResumed {
        session_id: String,
    },
    AgentPromptSent {
        prompt_preview: String,
    },

    // ── Tab operations ──────────────────────────────────────────────────────
    TabCreated {
        name: Option<String>,
    },
    TabRenamed {
        old: Option<String>,
        new: Option<String>,
    },
    TabClosed,
    TabFocused,

    // ── Task operations (orchestration) ─────────────────────────────────────
    TaskCreated {
        task_id: String,
        title: String,
    },
    TaskUpdated {
        task_id: String,
        status: String,
    },
    TaskCompleted {
        task_id: String,
    },
    TaskFailed {
        task_id: String,
        reason: String,
    },

    // ── File operations ─────────────────────────────────────────────────────
    FileOpened {
        path: String,
    },
    FileEdited {
        path: String,
    },
    FileClosed {
        path: String,
    },

    // ── Generic operation (for extensibility) ───────────────────────────────
    Custom {
        name: String,
    },
}

/// Errors that can occur during state operations.
#[derive(Clone, Debug)]
pub enum OpError {
    /// Failed to serialize the operation.
    SerializeFailed(String),
    /// Failed to persist the operation.
    PersistFailed(String),
    /// The operation is invalid.
    InvalidOp(String),
    /// The entity was not found.
    EntityNotFound(String),
}

impl std::fmt::Display for OpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpError::SerializeFailed(msg) => write!(f, "serialization failed: {msg}"),
            OpError::PersistFailed(msg) => write!(f, "persistence failed: {msg}"),
            OpError::InvalidOp(msg) => write!(f, "invalid operation: {msg}"),
            OpError::EntityNotFound(id) => write!(f, "entity not found: {id}"),
        }
    }
}

impl std::error::Error for OpError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_op_creation() {
        let op = StateOp::new(
            "ws-1".to_string(),
            EntityType::Workspace,
            OpType::WorkspaceCreated {
                name: "test".to_string(),
                cwd: "/home/user".to_string(),
            },
        );

        assert_eq!(op.entity_id, "ws-1");
        assert_eq!(op.entity_type, EntityType::Workspace);
    }

    #[test]
    fn test_state_op_serialization() {
        let op = StateOp::new(
            "pane-1".to_string(),
            EntityType::Pane,
            OpType::PaneCreated {
                agent: Some("claude".to_string()),
                cwd: "/tmp".to_string(),
            },
        );

        let json = serde_json::to_string(&op).unwrap();
        let deserialized: StateOp = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.entity_id, op.entity_id);
    }
}
