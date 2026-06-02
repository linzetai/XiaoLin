//! Audit logging for sensitive operations.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};

/// Audit event kinds tracked by the system.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditAction {
    AgentCreated,
    AgentUpdated,
    AgentDeleted,
    SessionDeleted,
    ToolInvoked,
    PluginInvoked,
    ConfigChanged,
    AuthFailure,
    RateLimited,
    TraceDeleted,
    Custom(String),
}

impl std::fmt::Display for AuditAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuditAction::Custom(s) => write!(f, "custom:{s}"),
            other => write!(
                f,
                "{}",
                serde_json::to_string(other)
                    .unwrap_or_default()
                    .trim_matches('"')
            ),
        }
    }
}

/// A single audit log entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEvent {
    pub id: String,
    pub action: String,
    pub actor: String,
    pub target: Option<String>,
    pub detail: Option<String>,
    pub timestamp: String,
}

/// Manages audit log storage in SQLite.
pub struct AuditLog {
    pool: Pool<Sqlite>,
}

impl AuditLog {
    pub async fn new(pool: Pool<Sqlite>) -> anyhow::Result<Self> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS audit_log (
                id TEXT PRIMARY KEY,
                action TEXT NOT NULL,
                actor TEXT NOT NULL,
                target TEXT,
                detail TEXT,
                timestamp TEXT NOT NULL DEFAULT (datetime('now'))
            )",
        )
        .execute(&pool)
        .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_audit_timestamp ON audit_log(timestamp DESC)")
            .execute(&pool)
            .await?;

        Ok(Self { pool })
    }

    /// Record an audit event.
    pub async fn record(
        &self,
        action: AuditAction,
        actor: &str,
        target: Option<&str>,
        detail: Option<&str>,
    ) -> anyhow::Result<()> {
        let id = format!("aud-{}", uuid::Uuid::new_v4());
        let action_str = action.to_string();
        let now = Utc::now().to_rfc3339();

        sqlx::query(
            "INSERT INTO audit_log (id, action, actor, target, detail, timestamp)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind(&id)
        .bind(&action_str)
        .bind(actor)
        .bind(target)
        .bind(detail)
        .bind(&now)
        .execute(&self.pool)
        .await?;

        tracing::debug!(action = %action_str, actor, target, "audit event recorded");
        Ok(())
    }

    /// List recent audit events.
    pub async fn list(&self, limit: u32, offset: u32) -> anyhow::Result<Vec<AuditEvent>> {
        let rows = sqlx::query_as::<
            _,
            (
                String,
                String,
                String,
                Option<String>,
                Option<String>,
                String,
            ),
        >(
            "SELECT id, action, actor, target, detail, timestamp
             FROM audit_log ORDER BY timestamp DESC LIMIT ?1 OFFSET ?2",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| AuditEvent {
                id: r.0,
                action: r.1,
                actor: r.2,
                target: r.3,
                detail: r.4,
                timestamp: r.5,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn mem_pool() -> Pool<Sqlite> {
        sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn record_and_list() {
        let pool = mem_pool().await;
        let log = AuditLog::new(pool).await.unwrap();
        log.record(
            AuditAction::AgentCreated,
            "user-1",
            Some("agent-a"),
            Some("created"),
        )
        .await
        .unwrap();
        log.record(AuditAction::AgentDeleted, "user-1", Some("agent-b"), None)
            .await
            .unwrap();
        log.record(AuditAction::AuthFailure, "unknown", None, Some("bad key"))
            .await
            .unwrap();

        let events = log.list(10, 0).await.unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].action, "auth_failure");
    }

    #[tokio::test]
    async fn list_pagination() {
        let pool = mem_pool().await;
        let log = AuditLog::new(pool).await.unwrap();
        for i in 0..5 {
            log.record(AuditAction::ToolInvoked, &format!("u{i}"), None, None)
                .await
                .unwrap();
        }
        let page1 = log.list(2, 0).await.unwrap();
        assert_eq!(page1.len(), 2);
        let page2 = log.list(2, 2).await.unwrap();
        assert_eq!(page2.len(), 2);
    }

    #[test]
    fn action_display() {
        assert_eq!(AuditAction::AgentCreated.to_string(), "agent_created");
        assert_eq!(AuditAction::Custom("foo".into()).to_string(), "custom:foo");
    }

    #[tokio::test]
    async fn optional_fields() {
        let pool = mem_pool().await;
        let log = AuditLog::new(pool).await.unwrap();
        log.record(AuditAction::ConfigChanged, "admin", None, None)
            .await
            .unwrap();
        let events = log.list(1, 0).await.unwrap();
        assert!(events[0].target.is_none());
        assert!(events[0].detail.is_none());
    }
}
