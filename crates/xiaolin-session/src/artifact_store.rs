use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

/// A persisted file artifact record.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileArtifactRecord {
    pub path: String,
    pub operation: String,
    pub timestamp: String,
    pub tool_call_id: String,
    pub bytes: u64,
}

#[async_trait]
pub trait ArtifactStore: Send + Sync {
    async fn record_artifact(
        &self,
        session_id: &str,
        path: &str,
        operation: &str,
        tool_call_id: &str,
        bytes: u64,
        timestamp_ms: u64,
    ) -> anyhow::Result<()>;

    async fn get_session_artifacts(
        &self,
        session_id: &str,
    ) -> anyhow::Result<Vec<FileArtifactRecord>>;

    async fn delete_session_artifacts(&self, session_id: &str) -> anyhow::Result<()>;
}

const MAX_ARTIFACTS_PER_SESSION: i64 = 500;

pub struct SqliteArtifactStore {
    pool: SqlitePool,
}

impl SqliteArtifactStore {
    pub async fn open(pool: SqlitePool) -> anyhow::Result<Self> {
        let store = Self { pool };
        store.ensure_tables().await?;
        Ok(store)
    }

    async fn ensure_tables(&self) -> anyhow::Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS file_artifacts (
                id            INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id    TEXT NOT NULL,
                path          TEXT NOT NULL,
                operation     TEXT NOT NULL,
                timestamp     TEXT NOT NULL,
                tool_call_id  TEXT NOT NULL,
                bytes         INTEGER NOT NULL DEFAULT 0
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_session_id ON file_artifacts (session_id, timestamp DESC)",
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }
}

#[async_trait]
impl ArtifactStore for SqliteArtifactStore {
    async fn record_artifact(
        &self,
        session_id: &str,
        path: &str,
        operation: &str,
        tool_call_id: &str,
        bytes: u64,
        timestamp_ms: u64,
    ) -> anyhow::Result<()> {
        let timestamp = DateTime::<Utc>::from_timestamp_millis(timestamp_ms as i64)
            .unwrap_or_else(Utc::now)
            .to_rfc3339();

        sqlx::query(
            "INSERT INTO file_artifacts (session_id, path, operation, timestamp, tool_call_id, bytes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind(session_id)
        .bind(path)
        .bind(operation)
        .bind(timestamp)
        .bind(tool_call_id)
        .bind(bytes as i64)
        .execute(&self.pool)
        .await?;

        if let Err(e) = sqlx::query(
            "DELETE FROM file_artifacts WHERE session_id = ?1 AND id NOT IN (
                SELECT id FROM file_artifacts WHERE session_id = ?1 ORDER BY id DESC LIMIT ?2
            )",
        )
        .bind(session_id)
        .bind(MAX_ARTIFACTS_PER_SESSION)
        .execute(&self.pool)
        .await
        {
            tracing::warn!(session_id, error = %e, "failed to prune old artifacts");
        }

        Ok(())
    }

    async fn get_session_artifacts(
        &self,
        session_id: &str,
    ) -> anyhow::Result<Vec<FileArtifactRecord>> {
        let rows = sqlx::query_as::<_, ArtifactRow>(
            "SELECT path, operation, timestamp, tool_call_id, bytes
             FROM file_artifacts
             WHERE session_id = ?1
             ORDER BY timestamp DESC",
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| FileArtifactRecord {
                path: r.path,
                operation: r.operation,
                timestamp: r.timestamp,
                tool_call_id: r.tool_call_id,
                bytes: r.bytes.max(0) as u64,
            })
            .collect())
    }

    async fn delete_session_artifacts(&self, session_id: &str) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM file_artifacts WHERE session_id = ?1")
            .bind(session_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[derive(sqlx::FromRow)]
struct ArtifactRow {
    path: String,
    operation: String,
    timestamp: String,
    tool_call_id: String,
    bytes: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn open_memory_store() -> SqliteArtifactStore {
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .create_if_missing(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        SqliteArtifactStore::open(pool).await.unwrap()
    }

    #[tokio::test]
    async fn record_and_list_artifacts() {
        let store = open_memory_store().await;
        store
            .record_artifact("s1", "src/main.rs", "modified", "call-1", 1024, 1_700_000_000_000)
            .await
            .unwrap();
        store
            .record_artifact("s1", "src/lib.rs", "created", "call-2", 512, 1_700_000_000_001)
            .await
            .unwrap();

        let artifacts = store.get_session_artifacts("s1").await.unwrap();
        assert_eq!(artifacts.len(), 2);
        assert_eq!(artifacts[0].path, "src/lib.rs");
        assert_eq!(artifacts[1].path, "src/main.rs");
    }

    #[tokio::test]
    async fn delete_session_artifacts() {
        let store = open_memory_store().await;
        store
            .record_artifact("s1", "a.rs", "created", "c1", 100, 1)
            .await
            .unwrap();
        store.delete_session_artifacts("s1").await.unwrap();
        assert!(store.get_session_artifacts("s1").await.unwrap().is_empty());
    }
}
