use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use tokio::sync::RwLock;

/// Persisted snapshot of DAG execution progress.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DagCheckpoint {
    pub dag_id: String,
    pub node_states: HashMap<String, NodeState>,
    pub node_outputs: HashMap<String, serde_json::Value>,
    pub created_at: DateTime<Utc>,
}

/// Lifecycle state of a single node (for checkpointing / resume).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum NodeState {
    Pending,
    Running,
    Completed,
    Failed(String),
    Skipped,
}

#[async_trait]
pub trait CheckpointStore: Send + Sync {
    async fn save_node_state(
        &self,
        dag_id: &str,
        node_id: &str,
        state: &NodeState,
    ) -> anyhow::Result<()>;

    async fn save_node_output(
        &self,
        dag_id: &str,
        node_id: &str,
        output: &serde_json::Value,
    ) -> anyhow::Result<()>;

    async fn load_checkpoint(&self, dag_id: &str) -> anyhow::Result<Option<DagCheckpoint>>;

    async fn clear_checkpoint(&self, dag_id: &str) -> anyhow::Result<()>;
}

/// In-memory checkpoint store (correct trait semantics; no cross-process persistence).
#[derive(Debug, Default)]
pub struct InMemoryCheckpointStore {
    inner: RwLock<Option<DagCheckpoint>>,
}

impl InMemoryCheckpointStore {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(None),
        }
    }
}

#[async_trait]
impl CheckpointStore for InMemoryCheckpointStore {
    async fn save_node_state(
        &self,
        dag_id: &str,
        node_id: &str,
        state: &NodeState,
    ) -> anyhow::Result<()> {
        let mut guard = self.inner.write().await;
        let cp = guard.get_or_insert_with(|| DagCheckpoint {
            dag_id: dag_id.to_string(),
            node_states: HashMap::new(),
            node_outputs: HashMap::new(),
            created_at: Utc::now(),
        });
        if cp.dag_id != dag_id {
            anyhow::bail!(
                "checkpoint dag_id mismatch: store has {}, got {}",
                cp.dag_id,
                dag_id
            );
        }
        cp.node_states.insert(node_id.to_string(), state.clone());
        Ok(())
    }

    async fn save_node_output(
        &self,
        dag_id: &str,
        node_id: &str,
        output: &serde_json::Value,
    ) -> anyhow::Result<()> {
        let mut guard = self.inner.write().await;
        let cp = guard.get_or_insert_with(|| DagCheckpoint {
            dag_id: dag_id.to_string(),
            node_states: HashMap::new(),
            node_outputs: HashMap::new(),
            created_at: Utc::now(),
        });
        if cp.dag_id != dag_id {
            anyhow::bail!(
                "checkpoint dag_id mismatch: store has {}, got {}",
                cp.dag_id,
                dag_id
            );
        }
        cp.node_outputs.insert(node_id.to_string(), output.clone());
        Ok(())
    }

    async fn load_checkpoint(&self, dag_id: &str) -> anyhow::Result<Option<DagCheckpoint>> {
        let guard = self.inner.read().await;
        Ok(guard.as_ref().filter(|c| c.dag_id == dag_id).cloned())
    }

    async fn clear_checkpoint(&self, dag_id: &str) -> anyhow::Result<()> {
        let mut guard = self.inner.write().await;
        if guard.as_ref().is_some_and(|c| c.dag_id == dag_id) {
            *guard = None;
        }
        Ok(())
    }
}

const DAG_CHECKPOINT_MIGRATION: &str = r#"
CREATE TABLE IF NOT EXISTS dag_checkpoints (
    dag_id TEXT NOT NULL,
    node_id TEXT NOT NULL,
    state_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (dag_id, node_id)
);
CREATE TABLE IF NOT EXISTS dag_outputs (
    dag_id TEXT NOT NULL,
    node_id TEXT NOT NULL,
    output_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (dag_id, node_id)
);
CREATE INDEX IF NOT EXISTS idx_dag_checkpoints_dag_id ON dag_checkpoints(dag_id);
CREATE INDEX IF NOT EXISTS idx_dag_outputs_dag_id ON dag_outputs(dag_id);
"#;

#[derive(Clone)]
pub struct SqliteCheckpointStore {
    pool: sqlx::SqlitePool,
}

impl SqliteCheckpointStore {
    pub async fn open(pool: sqlx::SqlitePool) -> anyhow::Result<Self> {
        let s = Self { pool };
        s.migrate().await?;
        Ok(s)
    }

    async fn migrate(&self) -> anyhow::Result<()> {
        sqlx::raw_sql(DAG_CHECKPOINT_MIGRATION)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[async_trait]
impl CheckpointStore for SqliteCheckpointStore {
    async fn save_node_state(
        &self,
        dag_id: &str,
        node_id: &str,
        state: &NodeState,
    ) -> anyhow::Result<()> {
        let state_json = serde_json::to_string(state)?;
        let created_at = Utc::now().to_rfc3339();
        sqlx::query(
            r#"INSERT INTO dag_checkpoints (dag_id, node_id, state_json, created_at)
               VALUES (?, ?, ?, ?)
               ON CONFLICT(dag_id, node_id) DO UPDATE SET state_json = excluded.state_json"#,
        )
        .bind(dag_id)
        .bind(node_id)
        .bind(state_json)
        .bind(created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn save_node_output(
        &self,
        dag_id: &str,
        node_id: &str,
        output: &serde_json::Value,
    ) -> anyhow::Result<()> {
        let output_json = serde_json::to_string(output)?;
        let created_at = Utc::now().to_rfc3339();
        sqlx::query(
            r#"INSERT INTO dag_outputs (dag_id, node_id, output_json, created_at)
               VALUES (?, ?, ?, ?)
               ON CONFLICT(dag_id, node_id) DO UPDATE SET output_json = excluded.output_json"#,
        )
        .bind(dag_id)
        .bind(node_id)
        .bind(output_json)
        .bind(created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn load_checkpoint(&self, dag_id: &str) -> anyhow::Result<Option<DagCheckpoint>> {
        let mut node_states = HashMap::new();
        let mut rows =
            sqlx::query("SELECT node_id, state_json FROM dag_checkpoints WHERE dag_id = ?")
                .bind(dag_id)
                .fetch_all(&self.pool)
                .await?;
        for row in rows.drain(..) {
            let node_id: String = row.try_get("node_id")?;
            let state_json: String = row.try_get("state_json")?;
            let state: NodeState = serde_json::from_str(&state_json)?;
            node_states.insert(node_id, state);
        }

        let mut node_outputs = HashMap::new();
        let mut out_rows =
            sqlx::query("SELECT node_id, output_json FROM dag_outputs WHERE dag_id = ?")
                .bind(dag_id)
                .fetch_all(&self.pool)
                .await?;
        for row in out_rows.drain(..) {
            let node_id: String = row.try_get("node_id")?;
            let output_json: String = row.try_get("output_json")?;
            let output: serde_json::Value = serde_json::from_str(&output_json)?;
            node_outputs.insert(node_id, output);
        }

        if node_states.is_empty() && node_outputs.is_empty() {
            return Ok(None);
        }

        let min_created: Option<String> = sqlx::query_scalar(
            r#"SELECT MIN(ts) FROM (
                SELECT created_at AS ts FROM dag_checkpoints WHERE dag_id = ?
                UNION ALL
                SELECT created_at AS ts FROM dag_outputs WHERE dag_id = ?
            )"#,
        )
        .bind(dag_id)
        .bind(dag_id)
        .fetch_one(&self.pool)
        .await?;

        let created_at = min_created
            .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(Utc::now);

        Ok(Some(DagCheckpoint {
            dag_id: dag_id.to_string(),
            node_states,
            node_outputs,
            created_at,
        }))
    }

    async fn clear_checkpoint(&self, dag_id: &str) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM dag_checkpoints WHERE dag_id = ?")
            .bind(dag_id)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM dag_outputs WHERE dag_id = ?")
            .bind(dag_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqliteConnectOptions;
    use sqlx::SqlitePool;
    use std::str::FromStr;

    async fn test_pool() -> sqlx::SqlitePool {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:?cache=shared")
            .unwrap()
            .create_if_missing(true);
        SqlitePool::connect_with(opts).await.unwrap()
    }

    #[tokio::test]
    async fn sqlite_store_save_load_roundtrip() {
        let pool = test_pool().await;
        let store = SqliteCheckpointStore::open(pool).await.unwrap();
        let dag_id = "dag-1";

        store
            .save_node_state(dag_id, "n1", &NodeState::Completed)
            .await
            .unwrap();
        store
            .save_node_state(dag_id, "n2", &NodeState::Running)
            .await
            .unwrap();
        store
            .save_node_output(dag_id, "n1", &serde_json::json!({ "x": 1 }))
            .await
            .unwrap();

        let cp = store.load_checkpoint(dag_id).await.unwrap().unwrap();
        assert_eq!(cp.dag_id, dag_id);
        assert_eq!(cp.node_states.get("n1"), Some(&NodeState::Completed));
        assert_eq!(cp.node_states.get("n2"), Some(&NodeState::Running));
        assert_eq!(
            cp.node_outputs.get("n1"),
            Some(&serde_json::json!({ "x": 1 }))
        );
    }

    #[tokio::test]
    async fn sqlite_store_clear() {
        let pool = test_pool().await;
        let store = SqliteCheckpointStore::open(pool).await.unwrap();
        let dag_id = "dag-clear";

        store
            .save_node_state(dag_id, "a", &NodeState::Completed)
            .await
            .unwrap();
        store
            .save_node_output(dag_id, "a", &serde_json::json!(null))
            .await
            .unwrap();

        assert!(store.load_checkpoint(dag_id).await.unwrap().is_some());

        store.clear_checkpoint(dag_id).await.unwrap();
        assert!(store.load_checkpoint(dag_id).await.unwrap().is_none());
    }
}
