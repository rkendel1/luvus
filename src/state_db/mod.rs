//! State graph: queryable operation history for agents.
//!
//! Every app mutation flows through here: pane creation, agent status changes,
//! workspace modifications. The state graph provides agents with full context
//! about what has happened, not just the current snapshot.
//!
//! # Architecture
//!
//! The StateDb is a single-writer operation log with:
//! - In-memory index for fast queries (DashMap by entity)
//! - Global sequence-numbered operation log
//! - JSON checkpoint persistence
//!
//! This is **not** a CRDT or distributed database. It is a local state store
//! that records operations for agent context and debugging.
//!
//! # Usage
//!
//! ```ignore
//! use luvus::state_db::{StateDb, StateOp, EntityType, OpType};
//!
//! // Initialize the state database
//! let db = StateDb::new(&home_dir)?;
//!
//! // Record an operation
//! db.record_op(StateOp::new(
//!     "pane-1".to_string(),
//!     EntityType::Pane,
//!     OpType::PaneCreated { agent: Some("claude".into()), cwd: "/home".into() },
//! ));
//!
//! // Query agent context
//! let ctx = db.agent_context("ws-1");
//! ```

pub mod agent_context;
pub mod operations;
pub mod queries;

pub use agent_context::AgentContext;
pub use operations::{EntityType, OpError, OpType, StateOp};
pub use queries::StateQuery;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde_json::json;

/// Maximum operations to keep in the recent index per entity.
const MAX_OPS_PER_ENTITY: usize = 1000;

/// Maximum total operations to keep in memory before pruning.
const MAX_TOTAL_OPS: usize = 100_000;

/// Default time window for "recent" operations in agent context (1 hour).
const DEFAULT_RECENT_WINDOW_HOURS: i64 = 1;

/// Local state graph: all app mutations recorded as queryable operations.
///
/// Thread-safe and lock-free for reads; writes use fine-grained locking per
/// entity. Uses DashMap for fast in-memory indexing with JSON checkpoint
/// persistence.
///
/// This is a single-writer operation log, not a CRDT or distributed database.
#[derive(Clone)]
pub struct StateDb {
    /// In-memory index of operations by entity id (for fast queries).
    ops_by_entity: Arc<DashMap<String, Vec<StateOp>>>,

    /// Global operation log (all operations, newest last).
    global_log: Arc<DashMap<u64, StateOp>>,

    /// Monotonic sequence number for the global log.
    sequence: Arc<std::sync::atomic::AtomicU64>,

    /// Checkpoint timestamp for incremental persistence.
    checkpoint_at: Arc<std::sync::RwLock<DateTime<Utc>>>,

    /// Home directory for persistence.
    home_dir: PathBuf,
}

impl StateDb {
    /// Initialize the state database.
    pub fn new(home_dir: &Path) -> anyhow::Result<Self> {
        let db = Self {
            ops_by_entity: Arc::new(DashMap::new()),
            global_log: Arc::new(DashMap::new()),
            sequence: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            checkpoint_at: Arc::new(std::sync::RwLock::new(Utc::now())),
            home_dir: home_dir.to_path_buf(),
        };

        // Attempt to load existing checkpoint (for in-memory index recovery)
        if let Err(e) = db.load_checkpoint() {
            // Log but don't fail — fresh start is fine
            eprintln!("state_db: no checkpoint loaded: {e}");
        }

        Ok(db)
    }

    /// Record a state operation.
    ///
    /// This is non-blocking and safe to call from any thread. Operations are
    /// indexed in memory for fast queries.
    pub fn record_op(&self, op: StateOp) {
        let entity_id = op.entity_id.clone();

        // Add to global log (in-memory)
        let seq = self
            .sequence
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.global_log.insert(seq, op.clone());

        // Index by entity (in-memory)
        self.ops_by_entity.entry(entity_id).or_default().push(op);

        // Prune if needed
        self.maybe_prune();
    }

    /// Query operations matching the given criteria.
    pub fn query(&self, query: &StateQuery) -> Vec<StateOp> {
        let mut results = Vec::new();

        if let Some(entity_id) = &query.entity_id {
            // Query specific entity
            if let Some(ops) = self.ops_by_entity.get(entity_id) {
                results.extend(self.filter_ops(ops.value(), query));
            }
        } else {
            // Query all entities
            for entry in self.ops_by_entity.iter() {
                results.extend(self.filter_ops(entry.value(), query));
            }
        }

        // Sort by timestamp
        results.sort_by_key(|a| a.timestamp);

        // Apply offset
        if let Some(offset) = query.offset {
            if offset < results.len() {
                results = results.into_iter().skip(offset).collect();
            } else {
                results.clear();
            }
        }

        // Apply limit
        if let Some(limit) = query.limit {
            results.truncate(limit);
        }

        results
    }

    /// Query operations for an entity since a given time.
    pub fn query_operations(&self, entity_id: &str, since: Option<DateTime<Utc>>) -> Vec<StateOp> {
        let query = StateQuery {
            entity_id: Some(entity_id.to_string()),
            since,
            ..Default::default()
        };
        self.query(&query)
    }

    /// Build agent context for a workspace.
    ///
    /// Returns the current state snapshot plus recent operation history.
    pub fn agent_context(&self, workspace_id: &str) -> AgentContext {
        let since = Utc::now() - chrono::Duration::hours(DEFAULT_RECENT_WINDOW_HOURS);
        let recent_ops = self.query_operations(workspace_id, Some(since));

        // Build current state from related entities
        let mut current_state = Vec::new();

        // Add workspace info
        if let Some(ops) = self.ops_by_entity.get(workspace_id) {
            if let Some(last_op) = ops.last() {
                current_state.push(json!({
                    "type": "workspace",
                    "id": workspace_id,
                    "last_op": last_op.op_type,
                }));
            }
        }

        // Count active agents and panes from recent operations
        let mut active_agents = std::collections::HashSet::new();
        let mut active_panes = std::collections::HashSet::new();

        for op in &recent_ops {
            match &op.op_type {
                OpType::AgentDetected { name, .. } => {
                    active_agents.insert(name.clone());
                }
                OpType::PaneCreated { agent, .. } => {
                    active_panes.insert(op.entity_id.clone());
                    if let Some(agent_name) = agent {
                        active_agents.insert(agent_name.clone());
                    }
                }
                OpType::PaneClosed { .. } => {
                    active_panes.remove(&op.entity_id);
                }
                _ => {}
            }
        }

        let stats = agent_context::ContextStats {
            active_panes: active_panes.len(),
            active_agents: active_agents.len(),
            recent_op_count: recent_ops.len(),
            total_tokens: None,
            total_cost: None,
        };

        AgentContext {
            workspace_id: workspace_id.to_string(),
            current_state,
            recent_changes: recent_ops,
            stats,
        }
    }

    /// Get summary statistics about the state database.
    pub fn stats(&self) -> StateDbStats {
        let total_ops: usize = self.ops_by_entity.iter().map(|e| e.value().len()).sum();
        StateDbStats {
            total_operations: total_ops,
            entity_count: self.ops_by_entity.len(),
            checkpoint_at: *self.checkpoint_at.read().unwrap(),
            sequence: self.sequence.load(std::sync::atomic::Ordering::SeqCst),
        }
    }

    /// Checkpoint the state to disk.
    ///
    /// Called periodically (e.g., every 2 seconds like session saves).
    pub fn checkpoint(&self) -> anyhow::Result<()> {
        let checkpoint_path = self.home_dir.join("state_db.json");

        // Collect all operations
        let ops: Vec<StateOp> = self
            .ops_by_entity
            .iter()
            .flat_map(|e| e.value().clone())
            .collect();

        // Serialize and write
        let json = serde_json::to_string_pretty(&ops)?;
        std::fs::write(&checkpoint_path, json)?;

        // Update checkpoint timestamp
        *self.checkpoint_at.write().unwrap() = Utc::now();

        Ok(())
    }

    /// Load checkpoint from disk.
    fn load_checkpoint(&self) -> anyhow::Result<()> {
        let checkpoint_path = self.home_dir.join("state_db.json");

        if !checkpoint_path.exists() {
            return Err(anyhow::anyhow!("no checkpoint file"));
        }

        let json = std::fs::read_to_string(&checkpoint_path)?;
        let ops: Vec<StateOp> = serde_json::from_str(&json)?;

        for op in ops {
            self.record_op(op);
        }

        Ok(())
    }

    /// Filter operations by query criteria.
    fn filter_ops(&self, ops: &[StateOp], query: &StateQuery) -> Vec<StateOp> {
        ops.iter()
            .filter(|op| {
                // Filter by entity type
                if let Some(ref entity_type) = query.entity_type {
                    if &op.entity_type != entity_type {
                        return false;
                    }
                }

                // Filter by time range
                if let Some(since) = query.since {
                    if op.timestamp < since {
                        return false;
                    }
                }
                if let Some(until) = query.until {
                    if op.timestamp > until {
                        return false;
                    }
                }

                // Filter by operation type
                if let Some(ref op_types) = query.op_types {
                    let op_type_name = format!("{:?}", op.op_type);
                    if !op_types.iter().any(|t| op_type_name.contains(t)) {
                        return false;
                    }
                }

                true
            })
            .cloned()
            .collect()
    }

    /// Prune old operations if we exceed limits.
    fn maybe_prune(&self) {
        // Prune per-entity
        for mut entry in self.ops_by_entity.iter_mut() {
            if entry.value().len() > MAX_OPS_PER_ENTITY {
                let excess = entry.value().len() - MAX_OPS_PER_ENTITY;
                entry.value_mut().drain(0..excess);
            }
        }

        // Prune global log
        let total: usize = self.ops_by_entity.iter().map(|e| e.value().len()).sum();
        if total > MAX_TOTAL_OPS {
            // Remove oldest entries from global log
            let to_remove = total - MAX_TOTAL_OPS;
            let keys_to_remove: Vec<_> = self
                .global_log
                .iter()
                .take(to_remove)
                .map(|entry| *entry.key())
                .collect();

            for key in keys_to_remove {
                self.global_log.remove(&key);
            }
        }
    }
}

/// Statistics about the state database.
#[derive(Clone, Debug)]
pub struct StateDbStats {
    /// Total number of operations stored (in-memory index).
    pub total_operations: usize,
    /// Number of unique entities tracked.
    pub entity_count: usize,
    /// When the last checkpoint was written.
    pub checkpoint_at: DateTime<Utc>,
    /// Current sequence number (operation count).
    pub sequence: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_db() -> StateDb {
        StateDb::new(&PathBuf::from("/tmp")).unwrap()
    }

    #[test]
    fn test_record_and_query() {
        let db = temp_db();

        let op = StateOp::new(
            "pane-1".to_string(),
            EntityType::Pane,
            OpType::PaneCreated {
                agent: Some("claude".to_string()),
                cwd: "/home".to_string(),
            },
        );

        db.record_op(op);

        let results = db.query(&StateQuery::entity("pane-1"));
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entity_id, "pane-1");
    }

    #[test]
    fn test_agent_context() {
        let db = temp_db();

        db.record_op(StateOp::new(
            "ws-1".to_string(),
            EntityType::Workspace,
            OpType::WorkspaceCreated {
                name: "test".to_string(),
                cwd: "/home".to_string(),
            },
        ));

        db.record_op(StateOp::new(
            "ws-1".to_string(),
            EntityType::Pane,
            OpType::PaneCreated {
                agent: Some("claude".to_string()),
                cwd: "/home".to_string(),
            },
        ));

        let ctx = db.agent_context("ws-1");
        assert_eq!(ctx.workspace_id, "ws-1");
        assert!(!ctx.recent_changes.is_empty());
    }

    #[test]
    fn test_query_with_time_filter() {
        let db = temp_db();

        db.record_op(StateOp::new(
            "pane-1".to_string(),
            EntityType::Pane,
            OpType::PaneFocused,
        ));

        // Query operations from the last minute (should include our op)
        let results = db.query(&StateQuery::last_minutes(1));
        assert!(!results.is_empty());

        // Query operations from the future (should be empty)
        let future_query = StateQuery {
            since: Some(Utc::now() + chrono::Duration::hours(1)),
            ..Default::default()
        };
        let results = db.query(&future_query);
        assert!(results.is_empty());
    }

    #[test]
    fn test_stats() {
        let db = temp_db();

        db.record_op(StateOp::new(
            "pane-1".to_string(),
            EntityType::Pane,
            OpType::PaneFocused,
        ));

        let stats = db.stats();
        assert_eq!(stats.total_operations, 1);
        assert_eq!(stats.entity_count, 1);
    }
}
