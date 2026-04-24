use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Pool, Sqlite, Transaction};
use std::collections::HashMap;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

use fastclaw_core::types::ChatMessage;

use crate::models::{Session, SessionCreateOutcome, SessionMessage, SessionSummary};

const MSG_CACHE_MAX_SESSIONS: usize = 32;

pub struct SessionStore {
    pool: Pool<Sqlite>,
    /// In-memory cache of ChatMessage lists keyed by session_id.
    /// Avoids re-reading the full history from SQLite on every turn.
    msg_cache: Arc<RwLock<HashMap<String, Vec<ChatMessage>>>>,
}

impl SessionStore {
    /// Open (or create) a SQLite database at the given path with WAL mode enabled.
    pub async fn open(db_path: &Path) -> anyhow::Result<Self> {
        if let Some(parent) = db_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let options = SqliteConnectOptions::from_str(&format!("sqlite:{}", db_path.display()))?
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5));

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await?;

        let store = Self {
            pool,
            msg_cache: Arc::new(RwLock::new(HashMap::new())),
        };
        store.run_migrations().await?;

        tracing::info!(path = %db_path.display(), "session store opened");
        Ok(store)
    }

    pub fn pool(&self) -> Pool<Sqlite> {
        self.pool.clone()
    }

    /// Open an in-memory database (for testing).
    pub async fn open_memory() -> anyhow::Result<Self> {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")?
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5));

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;

        let store = Self {
            pool,
            msg_cache: Arc::new(RwLock::new(HashMap::new())),
        };
        store.run_migrations().await?;
        Ok(store)
    }

    async fn run_migrations(&self) -> anyhow::Result<()> {
        // SQLite defaults foreign_keys to OFF; sqlx also applies this per connection,
        // but we set it explicitly here so migration-time DDL runs with FK checks on.
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&self.pool)
            .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                title TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                message_count INTEGER NOT NULL DEFAULT 0
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                role TEXT NOT NULL,
                content TEXT,
                name TEXT,
                tool_calls_json TEXT,
                tool_call_id TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id, id)")
            .execute(&self.pool)
            .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_sessions_updated ON sessions(updated_at DESC)")
            .execute(&self.pool)
            .await?;

        // Migration: add work_dir column if missing
        let has_work_dir: bool = sqlx::query_scalar::<_, i32>(
            "SELECT COUNT(*) FROM pragma_table_info('sessions') WHERE name = 'work_dir'"
        )
        .fetch_one(&self.pool)
        .await
        .map(|c| c > 0)
        .unwrap_or(false);
        if !has_work_dir {
            sqlx::query("ALTER TABLE sessions ADD COLUMN work_dir TEXT")
                .execute(&self.pool)
                .await?;
            tracing::info!("migrated sessions table: added work_dir column");
        }

        // Migration: add usage tracking columns if missing
        let has_usage: bool = sqlx::query_scalar::<_, i32>(
            "SELECT COUNT(*) FROM pragma_table_info('sessions') WHERE name = 'total_prompt_tokens'"
        )
        .fetch_one(&self.pool)
        .await
        .map(|c| c > 0)
        .unwrap_or(false);
        if !has_usage {
            sqlx::query("ALTER TABLE sessions ADD COLUMN total_prompt_tokens INTEGER NOT NULL DEFAULT 0")
                .execute(&self.pool)
                .await?;
            sqlx::query("ALTER TABLE sessions ADD COLUMN total_completion_tokens INTEGER NOT NULL DEFAULT 0")
                .execute(&self.pool)
                .await?;
            sqlx::query("ALTER TABLE sessions ADD COLUMN total_elapsed_ms INTEGER NOT NULL DEFAULT 0")
                .execute(&self.pool)
                .await?;
            tracing::info!("migrated sessions table: added usage tracking columns");
        }

        Ok(())
    }

    /// Create a new session.
    ///
    /// If `session_id` already exists, `agent_id` and `title` are left unchanged and
    /// `updated_at` is set to the current time (SQLite `ON CONFLICT`).
    pub async fn create_session(
        &self,
        session_id: &str,
        agent_id: &str,
        title: Option<&str>,
    ) -> anyhow::Result<SessionCreateOutcome> {
        self.create_session_with_work_dir(session_id, agent_id, title, None).await
    }

    pub async fn create_session_with_work_dir(
        &self,
        session_id: &str,
        agent_id: &str,
        title: Option<&str>,
        work_dir: Option<&str>,
    ) -> anyhow::Result<SessionCreateOutcome> {
        let mut tx = self.pool.begin().await?;

        let existed: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE id = ?")
            .bind(session_id)
            .fetch_one(&mut *tx)
            .await?;

        sqlx::query(
            "INSERT INTO sessions (id, agent_id, title, work_dir) VALUES (?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET updated_at = datetime('now')",
        )
        .bind(session_id)
        .bind(agent_id)
        .bind(title)
        .bind(work_dir)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(if existed > 0 {
            SessionCreateOutcome::AlreadyExisted
        } else {
            SessionCreateOutcome::Created
        })
    }

    pub async fn update_work_dir(
        &self,
        session_id: &str,
        work_dir: Option<&str>,
    ) -> anyhow::Result<()> {
        sqlx::query("UPDATE sessions SET work_dir = ?, updated_at = datetime('now') WHERE id = ?")
            .bind(work_dir)
            .bind(session_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Get a session by ID, or None if it doesn't exist.
    pub async fn get_session(&self, session_id: &str) -> anyhow::Result<Option<Session>> {
        let session = sqlx::query_as::<_, Session>(
            "SELECT id, agent_id, title, work_dir, created_at, updated_at, message_count,
                    total_prompt_tokens, total_completion_tokens, total_elapsed_ms
             FROM sessions WHERE id = ?",
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(session)
    }

    /// List sessions ordered by most recently updated.
    pub async fn list_sessions(
        &self,
        limit: i64,
        offset: i64,
    ) -> anyhow::Result<Vec<SessionSummary>> {
        let rows = sqlx::query_as::<_, Session>(
            "SELECT id, agent_id, title, work_dir, created_at, updated_at, message_count,
                    total_prompt_tokens, total_completion_tokens, total_elapsed_ms
             FROM sessions ORDER BY updated_at DESC LIMIT ? OFFSET ?",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|s| SessionSummary {
                id: s.id,
                agent_id: s.agent_id,
                title: s.title,
                work_dir: s.work_dir,
                message_count: s.message_count,
                created_at: s.created_at,
                updated_at: s.updated_at,
                total_prompt_tokens: s.total_prompt_tokens,
                total_completion_tokens: s.total_completion_tokens,
                total_elapsed_ms: s.total_elapsed_ms,
            })
            .collect())
    }

    /// Append a chat message to a session.
    pub async fn append_message(&self, session_id: &str, msg: &ChatMessage) -> anyhow::Result<()> {
        let mut tx = self.pool.begin().await?;
        Self::append_message_in_transaction(&mut tx, session_id, msg).await?;
        tx.commit().await?;

        let mut cache = self.msg_cache.write().await;
        if let Some(cached) = cache.get_mut(session_id) {
            cached.push(msg.clone());
        }

        Ok(())
    }

    async fn append_message_in_transaction(
        tx: &mut Transaction<'_, Sqlite>,
        session_id: &str,
        msg: &ChatMessage,
    ) -> anyhow::Result<()> {
        let role = serde_json::to_string(&msg.role)?;
        let role = role.trim_matches('"');

        let tool_calls_json = msg
            .tool_calls
            .as_ref()
            .map(|tc| serde_json::to_string(tc))
            .transpose()?;

        let content_json: Option<String> = match &msg.content {
            None => None,
            Some(v) => Some(serde_json::to_string(v)?),
        };

        sqlx::query(
            "INSERT INTO messages (session_id, role, content, name, tool_calls_json, tool_call_id)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(session_id)
        .bind(role)
        .bind(&content_json)
        .bind(&msg.name)
        .bind(&tool_calls_json)
        .bind(&msg.tool_call_id)
        .execute(&mut **tx)
        .await?;

        sqlx::query(
            "UPDATE sessions SET message_count = message_count + 1, updated_at = datetime('now') WHERE id = ?",
        )
        .bind(session_id)
        .execute(&mut **tx)
        .await?;

        Ok(())
    }

    /// Append multiple messages in a single transaction.
    pub async fn append_messages(
        &self,
        session_id: &str,
        messages: &[ChatMessage],
    ) -> anyhow::Result<()> {
        if messages.is_empty() {
            return Ok(());
        }

        let mut tx = self.pool.begin().await?;
        for msg in messages {
            Self::append_message_in_transaction(&mut tx, session_id, msg).await?;
        }
        tx.commit().await?;

        let mut cache = self.msg_cache.write().await;
        if let Some(cached) = cache.get_mut(session_id) {
            cached.extend_from_slice(messages);
        }

        Ok(())
    }

    /// Upsert a partial assistant message for crash recovery during streaming.
    /// Only updates an assistant row if it is the very last message in the session
    /// (i.e. a partial row we previously inserted). If the last message is not an
    /// assistant row (e.g. the user message that started this turn), inserts a new one.
    pub async fn save_partial_assistant(
        &self,
        session_id: &str,
        content: &str,
    ) -> anyhow::Result<()> {
        let content_json = serde_json::to_string(&serde_json::Value::String(content.to_string()))?;
        let updated = sqlx::query(
            "UPDATE messages SET content = ?
             WHERE id = (SELECT MAX(id) FROM messages WHERE session_id = ?)
               AND role = 'assistant'",
        )
        .bind(&content_json)
        .bind(session_id)
        .execute(&self.pool)
        .await?
        .rows_affected();

        if updated == 0 {
            let msg = fastclaw_core::types::ChatMessage {
                role: fastclaw_core::types::Role::Assistant,
                content: Some(serde_json::Value::String(content.to_string())),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            };
            self.append_message(session_id, &msg).await?;
        } else {
            self.invalidate_msg_cache(session_id).await;
        }
        Ok(())
    }

    /// Remove the partial assistant message (called before inserting the final one).
    /// Only deletes if the last message in the session is an assistant row, protecting
    /// finalized assistant messages from prior turns.
    pub async fn remove_partial_assistant(&self, session_id: &str) -> anyhow::Result<()> {
        let deleted = sqlx::query(
            "DELETE FROM messages
             WHERE id = (SELECT MAX(id) FROM messages WHERE session_id = ?)
               AND role = 'assistant'",
        )
        .bind(session_id)
        .execute(&self.pool)
        .await?
        .rows_affected();

        if deleted > 0 {
            sqlx::query(
                "UPDATE sessions SET message_count = message_count - 1, updated_at = datetime('now') WHERE id = ?",
            )
            .bind(session_id)
            .execute(&self.pool)
            .await?;
            self.invalidate_msg_cache(session_id).await;
        }
        Ok(())
    }

    /// Load all messages for a session, ordered by insertion order.
    pub async fn load_messages(&self, session_id: &str) -> anyhow::Result<Vec<SessionMessage>> {
        let messages = sqlx::query_as::<_, SessionMessage>(
            "SELECT id, session_id, role, content, name, tool_calls_json, tool_call_id, created_at
             FROM messages WHERE session_id = ? ORDER BY id ASC",
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(messages)
    }

    /// Convert stored messages back into ChatMessage format for the LLM.
    /// Uses an in-memory cache to avoid re-reading from SQLite on every turn.
    pub async fn load_chat_messages(&self, session_id: &str) -> anyhow::Result<Vec<ChatMessage>> {
        {
            let cache = self.msg_cache.read().await;
            if let Some(cached) = cache.get(session_id) {
                return Ok(cached.clone());
            }
        }

        let messages = self.load_chat_messages_from_db(session_id).await?;

        {
            let mut cache = self.msg_cache.write().await;
            if cache.len() >= MSG_CACHE_MAX_SESSIONS && !cache.contains_key(session_id) {
                if let Some(oldest) = cache.keys().next().cloned() {
                    cache.remove(&oldest);
                }
            }
            cache.insert(session_id.to_string(), messages.clone());
        }

        Ok(messages)
    }

    fn parse_chat_messages_from_rows(rows: Vec<SessionMessage>) -> anyhow::Result<Vec<ChatMessage>> {
        let mut messages = Vec::with_capacity(rows.len());

        for row in rows {
            let role = match row.role.as_str() {
                "system" => fastclaw_core::types::Role::System,
                "user" => fastclaw_core::types::Role::User,
                "assistant" => fastclaw_core::types::Role::Assistant,
                "tool" => fastclaw_core::types::Role::Tool,
                other => {
                    tracing::warn!(role = other, "unknown message role, skipping");
                    continue;
                }
            };

            let tool_calls = row
                .tool_calls_json
                .as_deref()
                .map(serde_json::from_str)
                .transpose()?;

            let content: Option<serde_json::Value> = match row.content.as_deref() {
                None | Some("") => None,
                Some(s) => match serde_json::from_str(s) {
                    Ok(v) => Some(v),
                    Err(_) => Some(serde_json::Value::String(s.to_string())),
                },
            };

            messages.push(ChatMessage {
                role,
                content,
                name: row.name,
                tool_calls,
                tool_call_id: row.tool_call_id,
            });
        }

        Ok(messages)
    }

    async fn load_chat_messages_from_db(&self, session_id: &str) -> anyhow::Result<Vec<ChatMessage>> {
        let rows = self.load_messages(session_id).await?;
        Self::parse_chat_messages_from_rows(rows)
    }

    /// Invalidate the message cache for a session (e.g. after external edits).
    pub async fn invalidate_msg_cache(&self, session_id: &str) {
        let mut cache = self.msg_cache.write().await;
        cache.remove(session_id);
    }

    /// Update the title of an existing session.
    pub async fn update_title(&self, session_id: &str, title: &str) -> anyhow::Result<bool> {
        let result = sqlx::query(
            "UPDATE sessions SET title = ?, updated_at = datetime('now') WHERE id = ?",
        )
        .bind(title)
        .bind(session_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Delete a session and all its messages.
    pub async fn delete_session(&self, session_id: &str) -> anyhow::Result<bool> {
        let result = sqlx::query("DELETE FROM sessions WHERE id = ?")
            .bind(session_id)
            .execute(&self.pool)
            .await?;

        if result.rows_affected() > 0 {
            self.invalidate_msg_cache(session_id).await;
        }

        Ok(result.rows_affected() > 0)
    }

    /// Accumulate token usage and elapsed time for a session (additive).
    pub async fn accumulate_usage(
        &self,
        session_id: &str,
        prompt_tokens: u32,
        completion_tokens: u32,
        elapsed_ms: u64,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE sessions SET
                total_prompt_tokens = total_prompt_tokens + ?,
                total_completion_tokens = total_completion_tokens + ?,
                total_elapsed_ms = total_elapsed_ms + ?,
                updated_at = datetime('now')
             WHERE id = ?",
        )
        .bind(prompt_tokens as i64)
        .bind(completion_tokens as i64)
        .bind(elapsed_ms as i64)
        .bind(session_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Delete sessions that haven't been updated within the given TTL (in hours).
    pub async fn cleanup_expired(&self, ttl_hours: u64) -> anyhow::Result<u64> {
        let result = sqlx::query("DELETE FROM sessions WHERE updated_at < datetime('now', ?)")
            .bind(format!("-{ttl_hours} hours"))
            .execute(&self.pool)
            .await?;

        let count = result.rows_affected();
        if count > 0 {
            tracing::info!(deleted = count, ttl_hours, "cleaned up expired sessions");
        }
        Ok(count)
    }
}

#[cfg(test)]
impl SessionStore {
    async fn exec_raw(&self, sql: &str) -> anyhow::Result<()> {
        sqlx::query(sql).execute(&self.pool).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fastclaw_core::types::{ChatMessage, Role};

    #[tokio::test]
    async fn append_messages_commits_all_when_valid() {
        let store = SessionStore::open_memory().await.unwrap();
        store.create_session("s1", "agent", None).await.unwrap();

        let msgs = vec![
            ChatMessage {
                role: Role::User,
                content: Some("a".into()),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            },
            ChatMessage {
                role: Role::Assistant,
                content: Some("b".into()),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            },
        ];
        store.append_messages("s1", &msgs).await.unwrap();

        let loaded = store.load_messages("s1").await.unwrap();
        assert_eq!(loaded.len(), 2);
        let session = store.get_session("s1").await.unwrap().unwrap();
        assert_eq!(session.message_count, 2);
    }

    /// When the second insert in a batch fails, the first must not persist (transaction rollback).
    #[tokio::test]
    async fn append_messages_rolls_back_entire_batch_on_mid_failure() {
        let store = SessionStore::open_memory().await.unwrap();
        store.create_session("s1", "agent", None).await.unwrap();

        store
            .exec_raw(
                "CREATE TRIGGER trg_fail_second_message \
                 AFTER INSERT ON messages \
                 FOR EACH ROW \
                 WHEN (SELECT COUNT(*) FROM messages WHERE session_id = NEW.session_id) > 1 \
                 BEGIN SELECT RAISE(ABORT, 'simulated second-row failure'); END",
            )
            .await
            .unwrap();

        let msgs = vec![
            ChatMessage {
                role: Role::User,
                content: Some("first".into()),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            },
            ChatMessage {
                role: Role::User,
                content: Some("second".into()),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            },
        ];

        assert!(store.append_messages("s1", &msgs).await.is_err());

        let loaded = store.load_messages("s1").await.unwrap();
        assert!(
            loaded.is_empty(),
            "expected no messages after rollback, got {}",
            loaded.len()
        );
        let session = store.get_session("s1").await.unwrap().unwrap();
        assert_eq!(session.message_count, 0);
    }

    #[tokio::test]
    async fn append_messages_rejects_unknown_session_foreign_key() {
        let store = SessionStore::open_memory().await.unwrap();
        let msgs = vec![ChatMessage {
            role: Role::User,
            content: Some("x".into()),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }];
        assert!(store.append_messages("missing", &msgs).await.is_err());
    }

    #[tokio::test]
    async fn create_session_duplicate_id_refreshes_timestamp_keeps_metadata() {
        let store = SessionStore::open_memory().await.unwrap();
        assert_eq!(
            store
                .create_session("dup", "a1", Some("first"))
                .await
                .unwrap(),
            SessionCreateOutcome::Created
        );

        store
            .exec_raw(
                "UPDATE sessions SET updated_at = datetime('now', '-2 hours') WHERE id = 'dup'",
            )
            .await
            .unwrap();

        let stale_updated_at = store.get_session("dup").await.unwrap().unwrap().updated_at;

        assert_eq!(
            store
                .create_session("dup", "a2", Some("second"))
                .await
                .unwrap(),
            SessionCreateOutcome::AlreadyExisted
        );

        let s = store.get_session("dup").await.unwrap().unwrap();
        assert_eq!(s.agent_id, "a1");
        assert_eq!(s.title.as_deref(), Some("first"));
        assert_ne!(
            s.updated_at, stale_updated_at,
            "ON CONFLICT should bump updated_at; stale was {stale_updated_at}"
        );
    }
}
