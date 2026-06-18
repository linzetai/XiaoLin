use anyhow::Result;
use sqlx::SqlitePool;
use std::collections::HashMap;

/// Tracks skill usage events (reads and prompt injections) for analytics
/// and ordering. Isolated from `xiaolin-evolution`'s `skill_usages` table.
pub struct SkillUsageStore {
    pool: SqlitePool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageEventType {
    /// Skill was read via the `skill` tool's `read` action.
    Read,
    /// Skill content was injected into the prompt context.
    Injection,
}

impl UsageEventType {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Injection => "injection",
        }
    }
}

impl SkillUsageStore {
    pub async fn open(pool: SqlitePool) -> Result<Self> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS skill_usage (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                skill_id   TEXT NOT NULL,
                event_type TEXT NOT NULL,
                session_id TEXT,
                timestamp  TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX IF NOT EXISTS idx_skill_usage_skill_id ON skill_usage(skill_id);
            CREATE INDEX IF NOT EXISTS idx_skill_usage_timestamp ON skill_usage(timestamp);
            "#,
        )
        .execute(&pool)
        .await?;
        Ok(Self { pool })
    }

    /// Record a usage event.
    pub async fn record(
        &self,
        skill_id: &str,
        event_type: UsageEventType,
        session_id: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO skill_usage (skill_id, event_type, session_id) VALUES (?, ?, ?)",
        )
        .bind(skill_id)
        .bind(event_type.as_str())
        .bind(session_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Record multiple injection events in a single transaction.
    pub async fn record_injections(
        &self,
        skill_ids: &[&str],
        session_id: Option<&str>,
    ) -> Result<()> {
        if skill_ids.is_empty() {
            return Ok(());
        }
        let mut tx = self.pool.begin().await?;
        for skill_id in skill_ids {
            sqlx::query(
                "INSERT INTO skill_usage (skill_id, event_type, session_id) VALUES (?, 'injection', ?)",
            )
            .bind(*skill_id)
            .bind(session_id)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Get usage counts for the last N days, keyed by skill_id.
    pub async fn usage_counts(&self, days: u32) -> Result<HashMap<String, u64>> {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT skill_id, COUNT(*) as cnt FROM skill_usage \
             WHERE timestamp >= datetime('now', ? || ' days') \
             GROUP BY skill_id",
        )
        .bind(-(days as i64))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(id, cnt)| (id, cnt as u64))
            .collect())
    }

    /// Delete events older than N days.
    pub async fn purge_old(&self, retention_days: u32) -> Result<u64> {
        let result = sqlx::query(
            "DELETE FROM skill_usage WHERE timestamp < datetime('now', ? || ' days')",
        )
        .bind(-(retention_days as i64))
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqliteConnectOptions;
    use std::str::FromStr;
    use std::time::Duration;

    async fn test_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .busy_timeout(Duration::from_secs(5));
        SqlitePool::connect_with(opts).await.unwrap()
    }

    #[tokio::test]
    async fn record_and_count() {
        let pool = test_pool().await;
        let store = SkillUsageStore::open(pool).await.unwrap();

        store
            .record("skill-a", UsageEventType::Read, Some("sess1"))
            .await
            .unwrap();
        store
            .record("skill-a", UsageEventType::Injection, Some("sess1"))
            .await
            .unwrap();
        store
            .record("skill-b", UsageEventType::Read, None)
            .await
            .unwrap();

        let counts = store.usage_counts(30).await.unwrap();
        assert_eq!(counts["skill-a"], 2);
        assert_eq!(counts["skill-b"], 1);
    }

    #[tokio::test]
    async fn record_injections_batch() {
        let pool = test_pool().await;
        let store = SkillUsageStore::open(pool).await.unwrap();

        store
            .record_injections(&["s1", "s2", "s3"], Some("sess"))
            .await
            .unwrap();

        let counts = store.usage_counts(30).await.unwrap();
        assert_eq!(counts.len(), 3);
        assert_eq!(counts["s1"], 1);
    }

    #[tokio::test]
    async fn purge_keeps_recent() {
        let pool = test_pool().await;
        let store = SkillUsageStore::open(pool.clone()).await.unwrap();

        store
            .record("a", UsageEventType::Read, None)
            .await
            .unwrap();

        // Insert a backdated row directly
        sqlx::query(
            "INSERT INTO skill_usage (skill_id, event_type, timestamp) \
             VALUES ('old', 'read', datetime('now', '-100 days'))",
        )
        .execute(&pool)
        .await
        .unwrap();

        let purged = store.purge_old(90).await.unwrap();
        assert_eq!(purged, 1);

        let counts = store.usage_counts(365).await.unwrap();
        assert_eq!(counts.len(), 1);
        assert!(counts.contains_key("a"));
    }

    #[tokio::test]
    async fn empty_injections_is_noop() {
        let pool = test_pool().await;
        let store = SkillUsageStore::open(pool).await.unwrap();
        store.record_injections(&[], None).await.unwrap();
        let counts = store.usage_counts(30).await.unwrap();
        assert!(counts.is_empty());
    }
}
