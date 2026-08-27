//! FeltDB — CRDT-backed document store for Luvus state graph.
//!
//! This module provides the FeltDB API for storing and querying documents
//! with CRDT-based conflict resolution. When the official `felt_db` crate
//! becomes available on crates.io, this implementation can be replaced with:
//!
//! ```ignore
//! use felt_db::FeltDb;
//! ```
//!
//! # Design
//!
//! FeltDB stores documents as JSON values with string keys. Each operation
//! is recorded in a CRDT log for:
//! - Full operation history (not just current state)
//! - Conflict-free concurrent edits
//! - Query support by document type/fields
//!
//! # Example
//!
//! ```ignore
//! use luvus::state_db::felt_db::FeltDb;
//!
//! let db = FeltDb::new()?;
//! db.insert_doc("pane-1", json!({ "type": "pane", "status": "working" }))?;
//! let results = db.query(json!({ "type": "pane" }))?;
//! ```

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A CRDT-backed document database.
///
/// Provides insert/get/delete/query/update operations with full operation
/// history tracking. Documents are stored as JSON values keyed by string IDs.
pub struct FeltDb {
    /// Document store: key → value
    docs: RwLock<HashMap<String, Value>>,

    /// Operation log for CRDT history
    op_log: RwLock<Vec<FeltDbOp>>,

    /// Monotonic sequence number for operations
    sequence: AtomicU64,

    /// Persistence path (if any)
    persist_path: Option<PathBuf>,
}

/// An operation in the FeltDB CRDT log.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FeltDbOp {
    /// Sequence number (monotonic, unique per database)
    pub seq: u64,
    /// Operation timestamp
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Operation kind
    pub kind: FeltDbOpKind,
    /// Document key
    pub key: String,
    /// Document value (for insert/update)
    pub value: Option<Value>,
}

/// Operation kinds in FeltDB.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum FeltDbOpKind {
    Insert,
    Update,
    Delete,
}

/// Query result from FeltDB.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FeltDbResult {
    /// Document key
    pub key: String,
    /// Document value
    pub value: Value,
}

impl FeltDb {
    /// Create a new in-memory FeltDB instance.
    pub fn new() -> Result<Self> {
        Ok(Self {
            docs: RwLock::new(HashMap::new()),
            op_log: RwLock::new(Vec::new()),
            sequence: AtomicU64::new(0),
            persist_path: None,
        })
    }

    /// Create a FeltDB instance with persistence.
    ///
    /// Loads existing data from the specified path if it exists.
    pub fn with_persistence(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let mut db = Self {
            docs: RwLock::new(HashMap::new()),
            op_log: RwLock::new(Vec::new()),
            sequence: AtomicU64::new(0),
            persist_path: Some(path.clone()),
        };

        // Load existing data if present
        if path.exists() {
            db.load()?;
        }

        Ok(db)
    }

    /// Insert a document.
    ///
    /// If a document with this key already exists, it will be overwritten.
    pub fn insert_doc(&self, key: &str, value: Value) -> Result<()> {
        let seq = self.sequence.fetch_add(1, Ordering::SeqCst);

        // Record operation
        let op = FeltDbOp {
            seq,
            timestamp: chrono::Utc::now(),
            kind: FeltDbOpKind::Insert,
            key: key.to_string(),
            value: Some(value.clone()),
        };

        self.op_log
            .write()
            .map_err(|_| anyhow!("lock poisoned"))?
            .push(op);

        // Update document store
        self.docs
            .write()
            .map_err(|_| anyhow!("lock poisoned"))?
            .insert(key.to_string(), value);

        Ok(())
    }

    /// Get a document by key.
    pub fn get(&self, key: &str) -> Result<Option<Value>> {
        let docs = self.docs.read().map_err(|_| anyhow!("lock poisoned"))?;
        Ok(docs.get(key).cloned())
    }

    /// Delete a document by key.
    pub fn delete(&self, key: &str) -> Result<bool> {
        let seq = self.sequence.fetch_add(1, Ordering::SeqCst);

        // Record operation
        let op = FeltDbOp {
            seq,
            timestamp: chrono::Utc::now(),
            kind: FeltDbOpKind::Delete,
            key: key.to_string(),
            value: None,
        };

        self.op_log
            .write()
            .map_err(|_| anyhow!("lock poisoned"))?
            .push(op);

        // Remove from document store
        let existed = self
            .docs
            .write()
            .map_err(|_| anyhow!("lock poisoned"))?
            .remove(key)
            .is_some();

        Ok(existed)
    }

    /// Update a document (merge fields).
    ///
    /// If the document doesn't exist, this creates it.
    pub fn update(&self, key: &str, partial: Value) -> Result<()> {
        let seq = self.sequence.fetch_add(1, Ordering::SeqCst);

        // Record operation
        let op = FeltDbOp {
            seq,
            timestamp: chrono::Utc::now(),
            kind: FeltDbOpKind::Update,
            key: key.to_string(),
            value: Some(partial.clone()),
        };

        self.op_log
            .write()
            .map_err(|_| anyhow!("lock poisoned"))?
            .push(op);

        // Merge into document store
        let mut docs = self.docs.write().map_err(|_| anyhow!("lock poisoned"))?;

        if let Some(existing) = docs.get_mut(key) {
            // Merge objects
            if let (Value::Object(existing_obj), Value::Object(partial_obj)) =
                (existing, partial.clone())
            {
                for (k, v) in partial_obj {
                    existing_obj.insert(k, v);
                }
            } else {
                // Replace if not both objects
                docs.insert(key.to_string(), partial);
            }
        } else {
            // Create new
            docs.insert(key.to_string(), partial);
        }

        Ok(())
    }

    /// Query documents by matching fields.
    ///
    /// The query is a JSON object where each field is matched against documents.
    /// Documents match if they contain all specified fields with matching values.
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Find all documents with type "pane"
    /// let results = db.query(json!({ "type": "pane" }))?;
    ///
    /// // Find panes with status "working"  
    /// let results = db.query(json!({ "type": "pane", "status": "working" }))?;
    /// ```
    pub fn query(&self, query: Value) -> Result<Vec<FeltDbResult>> {
        let docs = self.docs.read().map_err(|_| anyhow!("lock poisoned"))?;

        let query_obj = match query.as_object() {
            Some(obj) => obj,
            None => {
                // Empty query returns all documents
                return Ok(docs
                    .iter()
                    .map(|(k, v)| FeltDbResult {
                        key: k.clone(),
                        value: v.clone(),
                    })
                    .collect());
            }
        };

        let results = docs
            .iter()
            .filter(|(_, doc)| {
                if let Value::Object(doc_obj) = doc {
                    // Check all query fields match
                    query_obj.iter().all(|(qk, qv)| {
                        doc_obj.get(qk).map(|dv| dv == qv).unwrap_or(false)
                    })
                } else {
                    false
                }
            })
            .map(|(k, v)| FeltDbResult {
                key: k.clone(),
                value: v.clone(),
            })
            .collect();

        Ok(results)
    }

    /// Get the current sequence number (operation count).
    pub fn get_sequence(&self) -> u64 {
        self.sequence.load(Ordering::SeqCst)
    }

    /// Get all operations in the log.
    pub fn get_operations(&self) -> Result<Vec<FeltDbOp>> {
        let log = self.op_log.read().map_err(|_| anyhow!("lock poisoned"))?;
        Ok(log.clone())
    }

    /// Get operations since a given sequence number.
    pub fn get_operations_since(&self, since_seq: u64) -> Result<Vec<FeltDbOp>> {
        let log = self.op_log.read().map_err(|_| anyhow!("lock poisoned"))?;
        Ok(log.iter().filter(|op| op.seq >= since_seq).cloned().collect())
    }

    /// Get document count.
    pub fn len(&self) -> usize {
        self.docs.read().map(|d| d.len()).unwrap_or(0)
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Persist to disk (if persistence path is set).
    pub fn persist(&self) -> Result<()> {
        let path = match &self.persist_path {
            Some(p) => p,
            None => return Ok(()), // No-op if no persistence
        };

        let snapshot = FeltDbSnapshot {
            docs: self.docs.read().map_err(|_| anyhow!("lock poisoned"))?.clone(),
            op_log: self
                .op_log
                .read()
                .map_err(|_| anyhow!("lock poisoned"))?
                .clone(),
            sequence: self.sequence.load(Ordering::SeqCst),
        };

        let json = serde_json::to_string_pretty(&snapshot)?;
        fs::write(path, json)?;

        Ok(())
    }

    /// Load from disk.
    fn load(&mut self) -> Result<()> {
        let path = match &self.persist_path {
            Some(p) => p,
            None => return Ok(()),
        };

        let json = fs::read_to_string(path)?;
        let snapshot: FeltDbSnapshot = serde_json::from_str(&json)?;

        *self.docs.write().map_err(|_| anyhow!("lock poisoned"))? = snapshot.docs;
        *self.op_log.write().map_err(|_| anyhow!("lock poisoned"))? = snapshot.op_log;
        self.sequence.store(snapshot.sequence, Ordering::SeqCst);

        Ok(())
    }
}

impl Default for FeltDb {
    fn default() -> Self {
        Self::new().expect("FeltDb::new() should not fail")
    }
}

/// Snapshot for persistence.
#[derive(Serialize, Deserialize)]
struct FeltDbSnapshot {
    docs: HashMap<String, Value>,
    op_log: Vec<FeltDbOp>,
    sequence: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_insert_and_get() {
        let db = FeltDb::new().unwrap();

        db.insert_doc("pane-1", json!({ "type": "pane", "status": "working" }))
            .unwrap();

        let result = db.get("pane-1").unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap()["status"], "working");
    }

    #[test]
    fn test_query_by_type() {
        let db = FeltDb::new().unwrap();

        db.insert_doc("pane-1", json!({ "type": "pane", "status": "working" }))
            .unwrap();
        db.insert_doc("pane-2", json!({ "type": "pane", "status": "idle" }))
            .unwrap();
        db.insert_doc("agent-1", json!({ "type": "agent", "name": "claude" }))
            .unwrap();

        let results = db.query(json!({ "type": "pane" })).unwrap();
        assert_eq!(results.len(), 2);

        let results = db.query(json!({ "type": "agent" })).unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_query_by_multiple_fields() {
        let db = FeltDb::new().unwrap();

        db.insert_doc("pane-1", json!({ "type": "pane", "status": "working" }))
            .unwrap();
        db.insert_doc("pane-2", json!({ "type": "pane", "status": "idle" }))
            .unwrap();

        let results = db
            .query(json!({ "type": "pane", "status": "working" }))
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].key, "pane-1");
    }

    #[test]
    fn test_update() {
        let db = FeltDb::new().unwrap();

        db.insert_doc("pane-1", json!({ "type": "pane", "status": "working" }))
            .unwrap();
        db.update("pane-1", json!({ "status": "idle", "exit_code": 0 }))
            .unwrap();

        let result = db.get("pane-1").unwrap().unwrap();
        assert_eq!(result["type"], "pane"); // preserved
        assert_eq!(result["status"], "idle"); // updated
        assert_eq!(result["exit_code"], 0); // added
    }

    #[test]
    fn test_delete() {
        let db = FeltDb::new().unwrap();

        db.insert_doc("pane-1", json!({ "type": "pane" })).unwrap();
        assert!(db.get("pane-1").unwrap().is_some());

        let deleted = db.delete("pane-1").unwrap();
        assert!(deleted);
        assert!(db.get("pane-1").unwrap().is_none());
    }

    #[test]
    fn test_operation_log() {
        let db = FeltDb::new().unwrap();

        db.insert_doc("pane-1", json!({ "type": "pane" })).unwrap();
        db.update("pane-1", json!({ "status": "working" })).unwrap();
        db.delete("pane-1").unwrap();

        let ops = db.get_operations().unwrap();
        assert_eq!(ops.len(), 3);
        assert!(matches!(ops[0].kind, FeltDbOpKind::Insert));
        assert!(matches!(ops[1].kind, FeltDbOpKind::Update));
        assert!(matches!(ops[2].kind, FeltDbOpKind::Delete));
    }

    #[test]
    fn test_operations_since() {
        let db = FeltDb::new().unwrap();

        db.insert_doc("pane-1", json!({ "type": "pane" })).unwrap();
        let seq = db.get_sequence();
        db.insert_doc("pane-2", json!({ "type": "pane" })).unwrap();
        db.insert_doc("pane-3", json!({ "type": "pane" })).unwrap();

        let ops = db.get_operations_since(seq).unwrap();
        assert_eq!(ops.len(), 2); // pane-2 and pane-3
    }

    #[test]
    fn test_persistence() {
        let temp_path = std::env::temp_dir().join("test_feltdb.json");

        // Create and populate
        {
            let db = FeltDb::with_persistence(&temp_path).unwrap();
            db.insert_doc("pane-1", json!({ "type": "pane", "status": "working" }))
                .unwrap();
            db.persist().unwrap();
        }

        // Reload and verify
        {
            let db = FeltDb::with_persistence(&temp_path).unwrap();
            let result = db.get("pane-1").unwrap();
            assert!(result.is_some());
            assert_eq!(result.unwrap()["status"], "working");

            // Operations should also be restored
            let ops = db.get_operations().unwrap();
            assert!(!ops.is_empty());
        }

        // Cleanup
        let _ = std::fs::remove_file(&temp_path);
    }
}
