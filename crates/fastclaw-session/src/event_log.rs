use fastclaw_protocol::AgentEvent;
use sqlx::sqlite::SqlitePool;

/// Append-only event log for session replay and debugging.
///
/// Each event is serialized to JSON and stored with the turn_id extracted
/// for efficient per-turn queries. This provides a lossless audit trail
/// of everything that happened during agent execution.
pub struct EventLog {
    pool: SqlitePool,
}

impl EventLog {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn ensure_table(&self) -> anyhow::Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS event_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                turn_id TEXT NOT NULL,
                event_type TEXT NOT NULL,
                event_json TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_event_log_session_turn ON event_log(session_id, turn_id)",
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn append(
        &self,
        session_id: &str,
        event: &AgentEvent,
    ) -> anyhow::Result<()> {
        let turn_id = event.turn_id().as_str();
        let event_json = serde_json::to_string(event)?;
        let event_type = extract_event_type(&event_json);

        sqlx::query(
            "INSERT INTO event_log (session_id, turn_id, event_type, event_json) VALUES (?, ?, ?, ?)",
        )
        .bind(session_id)
        .bind(turn_id)
        .bind(&event_type)
        .bind(&event_json)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn events_for_turn(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> anyhow::Result<Vec<AgentEvent>> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT event_json FROM event_log WHERE session_id = ? AND turn_id = ? ORDER BY id",
        )
        .bind(session_id)
        .bind(turn_id)
        .fetch_all(&self.pool)
        .await?;

        rows.iter()
            .map(|(json,)| serde_json::from_str(json).map_err(Into::into))
            .collect()
    }

    pub async fn events_for_session(
        &self,
        session_id: &str,
    ) -> anyhow::Result<Vec<AgentEvent>> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT event_json FROM event_log WHERE session_id = ? ORDER BY id",
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;

        rows.iter()
            .map(|(json,)| serde_json::from_str(json).map_err(Into::into))
            .collect()
    }

    pub async fn append_turn_context(
        &self,
        session_id: &str,
        ctx: &fastclaw_protocol::TurnContextItem,
    ) -> anyhow::Result<()> {
        let turn_id = ctx.turn_id.as_str();
        let ctx_json = serde_json::to_string(ctx)?;

        sqlx::query(
            "INSERT INTO event_log (session_id, turn_id, event_type, event_json) VALUES (?, ?, 'turn_context', ?)",
        )
        .bind(session_id)
        .bind(turn_id)
        .bind(&ctx_json)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Load the most recent TurnContextItem for a session (for resume).
    pub async fn last_turn_context(
        &self,
        session_id: &str,
    ) -> anyhow::Result<Option<fastclaw_protocol::TurnContextItem>> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT event_json FROM event_log WHERE session_id = ? AND event_type = 'turn_context' ORDER BY id DESC LIMIT 1",
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some((json,)) => Ok(Some(serde_json::from_str(&json)?)),
            None => Ok(None),
        }
    }
}

fn extract_event_type(json: &str) -> String {
    serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|v| v.get("type").and_then(|t| t.as_str()).map(String::from))
        .unwrap_or_else(|| "unknown".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use fastclaw_protocol::TurnId;

    async fn make_pool() -> SqlitePool {
        SqlitePool::connect("sqlite::memory:").await.unwrap()
    }

    #[tokio::test]
    async fn append_and_query_events() {
        let pool = make_pool().await;
        let log = EventLog::new(pool);
        log.ensure_table().await.unwrap();

        let turn_id = TurnId::new("t1");
        let evt = AgentEvent::Error {
            turn_id: turn_id.clone(),
            message: "test error".into(),
            error_code: None,
        };

        log.append("s1", &evt).await.unwrap();

        let events = log.events_for_turn("s1", "t1").await.unwrap();
        assert_eq!(events.len(), 1);
        if let AgentEvent::Error { message, .. } = &events[0] {
            assert_eq!(message, "test error");
        } else {
            panic!("wrong event type");
        }
    }

    #[tokio::test]
    async fn events_for_session_returns_all() {
        let pool = make_pool().await;
        let log = EventLog::new(pool);
        log.ensure_table().await.unwrap();

        let t1 = TurnId::new("t1");
        let t2 = TurnId::new("t2");

        log.append(
            "s1",
            &AgentEvent::Error {
                turn_id: t1,
                message: "err1".into(),
                error_code: None,
            },
        )
        .await
        .unwrap();
        log.append(
            "s1",
            &AgentEvent::Error {
                turn_id: t2,
                message: "err2".into(),
                error_code: None,
            },
        )
        .await
        .unwrap();

        let events = log.events_for_session("s1").await.unwrap();
        assert_eq!(events.len(), 2);
    }
}
