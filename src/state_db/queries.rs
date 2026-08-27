//! Query builder for the state graph.
//!
//! Provides a type-safe way to construct queries against the state database.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::operations::EntityType;

/// A query against the state database.
///
/// Queries can filter by entity, type, time range, and operation type.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StateQuery {
    /// Filter to a specific entity by id.
    pub entity_id: Option<String>,
    /// Filter to entities of a specific type.
    pub entity_type: Option<EntityType>,
    /// Filter to operations after this timestamp.
    pub since: Option<DateTime<Utc>>,
    /// Filter to operations before this timestamp.
    pub until: Option<DateTime<Utc>>,
    /// Maximum number of results to return.
    pub limit: Option<usize>,
    /// Skip this many results (for pagination).
    pub offset: Option<usize>,
    /// Filter to specific operation types by name.
    pub op_types: Option<Vec<String>>,
}

impl StateQuery {
    /// Create an empty query (matches everything).
    pub fn new() -> Self {
        Self::default()
    }

    /// Query operations for a specific workspace.
    pub fn workspace(id: &str) -> Self {
        Self {
            entity_id: Some(id.to_string()),
            entity_type: Some(EntityType::Workspace),
            ..Default::default()
        }
    }

    /// Query operations for a specific pane.
    pub fn pane(id: &str) -> Self {
        Self {
            entity_id: Some(id.to_string()),
            entity_type: Some(EntityType::Pane),
            ..Default::default()
        }
    }

    /// Query operations for a specific entity.
    pub fn entity(id: &str) -> Self {
        Self {
            entity_id: Some(id.to_string()),
            ..Default::default()
        }
    }

    /// Query all operations of a specific type.
    pub fn by_type(entity_type: EntityType) -> Self {
        Self {
            entity_type: Some(entity_type),
            ..Default::default()
        }
    }

    /// Filter to operations since a given time.
    pub fn since(mut self, time: DateTime<Utc>) -> Self {
        self.since = Some(time);
        self
    }

    /// Filter to operations until a given time.
    pub fn until(mut self, time: DateTime<Utc>) -> Self {
        self.until = Some(time);
        self
    }

    /// Limit the number of results.
    pub fn limit(mut self, n: usize) -> Self {
        self.limit = Some(n);
        self
    }

    /// Skip results for pagination.
    pub fn offset(mut self, n: usize) -> Self {
        self.offset = Some(n);
        self
    }

    /// Filter to specific operation types.
    pub fn op_types(mut self, types: Vec<String>) -> Self {
        self.op_types = Some(types);
        self
    }

    /// Query operations from the last N hours.
    pub fn last_hours(hours: i64) -> Self {
        Self {
            since: Some(Utc::now() - chrono::Duration::hours(hours)),
            ..Default::default()
        }
    }

    /// Query operations from the last N minutes.
    pub fn last_minutes(minutes: i64) -> Self {
        Self {
            since: Some(Utc::now() - chrono::Duration::minutes(minutes)),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workspace_query() {
        let query = StateQuery::workspace("ws-1");
        assert_eq!(query.entity_id, Some("ws-1".to_string()));
        assert_eq!(query.entity_type, Some(EntityType::Workspace));
    }

    #[test]
    fn test_query_builder() {
        let query = StateQuery::new()
            .limit(10)
            .offset(5)
            .since(Utc::now() - chrono::Duration::hours(1));

        assert_eq!(query.limit, Some(10));
        assert_eq!(query.offset, Some(5));
        assert!(query.since.is_some());
    }
}
