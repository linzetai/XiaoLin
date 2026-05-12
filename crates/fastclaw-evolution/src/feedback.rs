use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

/// Explicit user feedback (thumbs up/down, rating, correction text).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackKind {
    ThumbsUp,
    ThumbsDown,
    Rating(f32),
    Correction(String),
}

/// An implicit interaction signal (tool success/failure, retry, abort, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractionSignal {
    ToolSuccess { tool_name: String },
    ToolFailure { tool_name: String, error: String },
    UserRetry,
    UserAbort,
    ConversationCompleted,
    LongResponse { tokens: u32 },
}

/// A feedback record persisted for analysis.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Feedback {
    pub id: String,
    pub session_id: String,
    pub agent_id: String,
    pub message_id: Option<String>,
    pub kind: String,
    pub payload: String,
    pub created_at: String,
}

pub struct FeedbackStore {
    pool: SqlitePool,
}

impl FeedbackStore {
    pub async fn open(pool: SqlitePool) -> Result<Self> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS feedback (
                id          TEXT PRIMARY KEY,
                session_id  TEXT NOT NULL,
                agent_id    TEXT NOT NULL DEFAULT 'main',
                message_id  TEXT,
                kind        TEXT NOT NULL,
                payload     TEXT NOT NULL DEFAULT '{}',
                created_at  TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX IF NOT EXISTS idx_fb_session ON feedback(session_id);
            CREATE INDEX IF NOT EXISTS idx_fb_agent   ON feedback(agent_id);
            CREATE INDEX IF NOT EXISTS idx_fb_kind    ON feedback(kind);
            "#,
        )
        .execute(&pool)
        .await?;

        Ok(Self { pool })
    }

    /// Record explicit user feedback.
    pub async fn record_feedback(
        &self,
        session_id: &str,
        agent_id: &str,
        message_id: Option<&str>,
        kind: &FeedbackKind,
    ) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let (kind_str, payload) = match kind {
            FeedbackKind::ThumbsUp => ("thumbs_up".to_string(), "{}".to_string()),
            FeedbackKind::ThumbsDown => ("thumbs_down".to_string(), "{}".to_string()),
            FeedbackKind::Rating(r) => (
                "rating".to_string(),
                serde_json::json!({"value": r}).to_string(),
            ),
            FeedbackKind::Correction(c) => (
                "correction".to_string(),
                serde_json::json!({"text": c}).to_string(),
            ),
        };

        sqlx::query(
            "INSERT INTO feedback (id, session_id, agent_id, message_id, kind, payload, created_at) VALUES (?,?,?,?,?,?,?)",
        )
        .bind(&id)
        .bind(session_id)
        .bind(agent_id)
        .bind(message_id)
        .bind(&kind_str)
        .bind(&payload)
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(id)
    }

    /// Record an implicit interaction signal.
    pub async fn record_signal(
        &self,
        session_id: &str,
        agent_id: &str,
        signal: &InteractionSignal,
    ) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let kind_str = match signal {
            InteractionSignal::ToolSuccess { .. } => "tool_success",
            InteractionSignal::ToolFailure { .. } => "tool_failure",
            InteractionSignal::UserRetry => "user_retry",
            InteractionSignal::UserAbort => "user_abort",
            InteractionSignal::ConversationCompleted => "conversation_completed",
            InteractionSignal::LongResponse { .. } => "long_response",
        };
        let payload = serde_json::to_string(signal)?;

        sqlx::query(
            "INSERT INTO feedback (id, session_id, agent_id, kind, payload, created_at) VALUES (?,?,?,?,?,?)",
        )
        .bind(&id)
        .bind(session_id)
        .bind(agent_id)
        .bind(kind_str)
        .bind(&payload)
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(id)
    }

    /// Get recent feedback for an agent.
    pub async fn recent(&self, agent_id: &str, limit: i64) -> Result<Vec<Feedback>> {
        let rows = sqlx::query_as::<_, Feedback>(
            "SELECT * FROM feedback WHERE agent_id = ? ORDER BY created_at DESC LIMIT ?",
        )
        .bind(agent_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Count total feedback entries for an agent (uncapped).
    pub async fn count(&self, agent_id: &str) -> Result<i64> {
        let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM feedback WHERE agent_id = ?")
            .bind(agent_id)
            .fetch_one(&self.pool)
            .await?;
        Ok(count)
    }

    /// Get feedback counts by kind for an agent.
    pub async fn counts_by_kind(&self, agent_id: &str) -> Result<Vec<(String, i64)>> {
        let rows: Vec<KindCount> = sqlx::query_as(
            "SELECT kind, COUNT(*) as count FROM feedback WHERE agent_id = ? GROUP BY kind ORDER BY count DESC",
        )
        .bind(agent_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|r| (r.kind, r.count)).collect())
    }

    /// Get all feedback for a session.
    pub async fn by_session(&self, session_id: &str) -> Result<Vec<Feedback>> {
        let rows = sqlx::query_as::<_, Feedback>(
            "SELECT * FROM feedback WHERE session_id = ? ORDER BY created_at",
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }
}

#[derive(sqlx::FromRow)]
struct KindCount {
    kind: String,
    count: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn store() -> FeedbackStore {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        FeedbackStore::open(pool).await.unwrap()
    }

    #[tokio::test]
    async fn record_and_retrieve_feedback() {
        let s = store().await;
        s.record_feedback("s1", "main", None, &FeedbackKind::ThumbsUp)
            .await
            .unwrap();
        s.record_feedback("s1", "main", None, &FeedbackKind::ThumbsDown)
            .await
            .unwrap();
        s.record_feedback("s1", "main", None, &FeedbackKind::Rating(4.5))
            .await
            .unwrap();

        let all = s.recent("main", 10).await.unwrap();
        assert_eq!(all.len(), 3);
    }

    #[tokio::test]
    async fn record_signal() {
        let s = store().await;
        s.record_signal(
            "s1",
            "main",
            &InteractionSignal::ToolSuccess {
                tool_name: "calculator".into(),
            },
        )
        .await
        .unwrap();
        s.record_signal("s1", "main", &InteractionSignal::UserRetry)
            .await
            .unwrap();

        let counts = s.counts_by_kind("main").await.unwrap();
        assert_eq!(counts.len(), 2);
    }

    #[tokio::test]
    async fn counts_by_kind() {
        let s = store().await;
        for _ in 0..3 {
            s.record_feedback("s1", "main", None, &FeedbackKind::ThumbsUp)
                .await
                .unwrap();
        }
        s.record_feedback("s1", "main", None, &FeedbackKind::ThumbsDown)
            .await
            .unwrap();

        let counts = s.counts_by_kind("main").await.unwrap();
        let up = counts.iter().find(|(k, _)| k == "thumbs_up").unwrap();
        assert_eq!(up.1, 3);
    }
}
