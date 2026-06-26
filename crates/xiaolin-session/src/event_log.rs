use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use sqlx::sqlite::SqlitePool;
use tokio::sync::mpsc;
use xiaolin_protocol::AgentEvent;

use crate::search_index::{try_index_event, SearchIndex};

const BATCH_CAPACITY: usize = 2048;
const BATCH_SIZE: usize = 64;
const FLUSH_INTERVAL_MS: u64 = 50;
const ENQUEUE_TIMEOUT: Duration = Duration::from_secs(5);

struct EventEntry {
    session_id: String,
    turn_id: String,
    event_type: String,
    event_json: String,
}

/// Append-only event log for session replay and debugging.
///
/// Events are buffered in an internal channel and flushed to SQLite in
/// batches (up to 64 events per transaction, or every 50ms), avoiding
/// per-event INSERT overhead on the streaming hot path.
pub struct EventLog {
    pool: SqlitePool,
    tx: mpsc::Sender<EventEntry>,
    writer_handle: Option<tokio::task::JoinHandle<()>>,
}

impl EventLog {
    pub fn new(pool: SqlitePool) -> Self {
        Self::with_search_index(pool, None)
    }

    pub fn with_search_index(pool: SqlitePool, search_index: Option<Arc<SearchIndex>>) -> Self {
        let (tx, rx) = mpsc::channel(BATCH_CAPACITY);
        let writer_pool = pool.clone();
        let writer_handle = tokio::spawn(batch_writer(writer_pool, rx, search_index));
        Self {
            pool,
            tx,
            writer_handle: Some(writer_handle),
        }
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

    /// Submit an event to the batch writer. Uses a non-blocking send first;
    /// if the buffer is full, waits up to [`ENQUEUE_TIMEOUT`] instead of dropping.
    pub fn append(&self, session_id: &str, event: &AgentEvent) {
        let turn_id = event.turn_id().to_string();
        let event_json = match serde_json::to_string(event) {
            Ok(j) => j,
            Err(e) => {
                tracing::warn!(error = %e, "event_log: failed to serialize event");
                return;
            }
        };
        let event_type = extract_event_type(&event_json);

        let entry = EventEntry {
            session_id: session_id.to_string(),
            turn_id,
            event_type,
            event_json,
        };

        match self.tx.try_send(entry) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(entry)) => {
                let tx = self.tx.clone();
                if let Ok(handle) = tokio::runtime::Handle::try_current() {
                    handle.spawn(async move {
                        match tokio::time::timeout(ENQUEUE_TIMEOUT, tx.send(entry)).await {
                            Ok(Ok(())) => {}
                            Ok(Err(_)) => {
                                tracing::warn!(
                                    "event_log: channel closed while waiting to enqueue"
                                );
                            }
                            Err(_) => {
                                tracing::warn!("event_log: timed out waiting to enqueue event");
                            }
                        }
                    });
                } else {
                    tracing::warn!(
                        "event_log: no tokio runtime available, falling back to blocking send"
                    );
                    if tx.blocking_send(entry).is_err() {
                        tracing::warn!("event_log: channel closed, dropping event");
                    }
                }
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                tracing::warn!("event_log: channel closed, dropping event");
            }
        }
    }

    /// Flush remaining events and wait for the writer task to finish.
    pub async fn shutdown(&mut self) {
        // Drop the sender so the writer drains and exits
        let (dead_tx, _) = mpsc::channel(1);
        let _ = std::mem::replace(&mut self.tx, dead_tx);

        if let Some(handle) = self.writer_handle.take() {
            let _ = handle.await;
        }
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

    pub async fn events_for_session(&self, session_id: &str) -> anyhow::Result<Vec<AgentEvent>> {
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT event_json FROM event_log WHERE session_id = ? ORDER BY id")
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
        ctx: &xiaolin_protocol::TurnContextItem,
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

    pub async fn last_turn_context(
        &self,
        session_id: &str,
    ) -> anyhow::Result<Option<xiaolin_protocol::TurnContextItem>> {
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

async fn batch_writer(
    pool: SqlitePool,
    mut rx: mpsc::Receiver<EventEntry>,
    search_index: Option<Arc<SearchIndex>>,
) {
    let mut buffer: Vec<EventEntry> = Vec::with_capacity(BATCH_SIZE);
    let mut delta_accum: HashMap<(String, String), String> = HashMap::new();
    let flush_interval = tokio::time::Duration::from_millis(FLUSH_INTERVAL_MS);

    loop {
        // Wait for first event or channel close
        tokio::select! {
            entry = rx.recv() => {
                match entry {
                    Some(e) => buffer.push(e),
                    None => {
                        // Channel closed — flush remaining and exit
                        if !buffer.is_empty() {
                            flush_batch(&pool, &mut buffer, search_index.as_ref(), &mut delta_accum)
                                .await;
                        }
                        return;
                    }
                }
            }
        }

        // Drain up to BATCH_SIZE or wait for flush interval
        let deadline = tokio::time::Instant::now() + flush_interval;
        while buffer.len() < BATCH_SIZE {
            tokio::select! {
                entry = rx.recv() => {
                    match entry {
                        Some(e) => buffer.push(e),
                        None => {
                            flush_batch(&pool, &mut buffer, search_index.as_ref(), &mut delta_accum)
                                .await;
                            return;
                        }
                    }
                }
                _ = tokio::time::sleep_until(deadline) => break,
            }
        }

        flush_batch(&pool, &mut buffer, search_index.as_ref(), &mut delta_accum).await;
    }
}

async fn flush_batch(
    pool: &SqlitePool,
    buffer: &mut Vec<EventEntry>,
    search_index: Option<&Arc<SearchIndex>>,
    delta_accum: &mut HashMap<(String, String), String>,
) {
    if buffer.is_empty() {
        return;
    }

    let result: Result<(), sqlx::Error> = async {
        let mut tx = pool.begin().await?;
        for entry in buffer.iter() {
            sqlx::query(
                "INSERT INTO event_log (session_id, turn_id, event_type, event_json) VALUES (?, ?, ?, ?)",
            )
            .bind(&entry.session_id)
            .bind(&entry.turn_id)
            .bind(&entry.event_type)
            .bind(&entry.event_json)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }
    .await;

    match result {
        Ok(()) => {
            if let Some(index) = search_index {
                for entry in buffer.iter() {
                    if let Some((role, content)) = try_index_event(
                        &entry.event_type,
                        &entry.event_json,
                        delta_accum,
                        &entry.session_id,
                        &entry.turn_id,
                    ) {
                        if let Err(e) = index
                            .index_row(&entry.session_id, &entry.turn_id, &role, &content, None)
                            .await
                        {
                            tracing::warn!(
                                session_id = %entry.session_id,
                                turn_id = %entry.turn_id,
                                error = %e,
                                "search_index: event_log hook failed"
                            );
                        }
                    }
                }
            }
        }
        Err(e) => {
            tracing::warn!(count = buffer.len(), error = %e, "event_log: batch flush failed");
        }
    }

    buffer.clear();
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
    use xiaolin_protocol::TurnId;

    async fn make_pool() -> SqlitePool {
        SqlitePool::connect("sqlite::memory:").await.unwrap()
    }

    #[tokio::test]
    async fn append_and_query_events() {
        let pool = make_pool().await;
        let mut log = EventLog::new(pool);
        log.ensure_table().await.unwrap();

        let turn_id = TurnId::new("t1");
        let evt = AgentEvent::Error {
            turn_id: turn_id.clone(),
            message: "test error".into(),
            error_code: None,
        };

        log.append("s1", &evt);
        // Allow batch writer to flush
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let events = log.events_for_turn("s1", "t1").await.unwrap();
        assert_eq!(events.len(), 1);
        if let AgentEvent::Error { message, .. } = &events[0] {
            assert_eq!(message, "test error");
        } else {
            panic!("wrong event type");
        }

        log.shutdown().await;
    }

    #[tokio::test]
    async fn events_for_session_returns_all() {
        let pool = make_pool().await;
        let mut log = EventLog::new(pool);
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
        );
        log.append(
            "s1",
            &AgentEvent::Error {
                turn_id: t2,
                message: "err2".into(),
                error_code: None,
            },
        );

        // Allow batch writer to flush
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let events = log.events_for_session("s1").await.unwrap();
        assert_eq!(events.len(), 2);

        log.shutdown().await;
    }
}
