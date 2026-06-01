use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Pool, Sqlite, Transaction};
use std::collections::HashMap;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

use fastclaw_core::types::ChatMessage;

use crate::models::{
    Session, SessionCreateOutcome, SessionMessage, SessionSummary, SubAgentRunRow,
};

const MSG_CACHE_MAX_SESSIONS: usize = 32;
/// Per-session message count cap for the in-memory cache.
/// Older messages are evicted from the cache (not from SQLite).
const MSG_CACHE_MAX_MESSAGES_PER_SESSION: usize = 200;

pub struct SessionStore {
    pool: Pool<Sqlite>,
    /// In-memory cache of ChatMessage lists keyed by session_id.
    /// Uses `Arc` so readers get a cheap reference-counted clone instead of
    /// deep-copying the entire message list on every cache hit.
    msg_cache: Arc<RwLock<HashMap<String, Arc<Vec<ChatMessage>>>>>,
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

    /// Create a SessionStore backed by an existing pool (tables are created if missing).
    pub async fn from_pool(pool: Pool<Sqlite>) -> anyhow::Result<Self> {
        let store = Self {
            pool,
            msg_cache: Arc::new(RwLock::new(HashMap::new())),
        };
        store.run_migrations().await?;
        tracing::info!("session store opened (shared pool)");
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
            "SELECT COUNT(*) FROM pragma_table_info('sessions') WHERE name = 'work_dir'",
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
            "SELECT COUNT(*) FROM pragma_table_info('sessions') WHERE name = 'total_prompt_tokens'",
        )
        .fetch_one(&self.pool)
        .await
        .map(|c| c > 0)
        .unwrap_or(false);
        if !has_usage {
            sqlx::query(
                "ALTER TABLE sessions ADD COLUMN total_prompt_tokens INTEGER NOT NULL DEFAULT 0",
            )
            .execute(&self.pool)
            .await?;
            sqlx::query("ALTER TABLE sessions ADD COLUMN total_completion_tokens INTEGER NOT NULL DEFAULT 0")
                .execute(&self.pool)
                .await?;
            sqlx::query(
                "ALTER TABLE sessions ADD COLUMN total_elapsed_ms INTEGER NOT NULL DEFAULT 0",
            )
            .execute(&self.pool)
            .await?;
            tracing::info!("migrated sessions table: added usage tracking columns");
        }

        // Migration: add per-message usage columns if missing
        let has_msg_usage: bool = sqlx::query_scalar::<_, i32>(
            "SELECT COUNT(*) FROM pragma_table_info('messages') WHERE name = 'prompt_tokens'",
        )
        .fetch_one(&self.pool)
        .await
        .map(|c| c > 0)
        .unwrap_or(false);
        if !has_msg_usage {
            sqlx::query("ALTER TABLE messages ADD COLUMN prompt_tokens INTEGER NOT NULL DEFAULT 0")
                .execute(&self.pool)
                .await?;
            sqlx::query(
                "ALTER TABLE messages ADD COLUMN completion_tokens INTEGER NOT NULL DEFAULT 0",
            )
            .execute(&self.pool)
            .await?;
            sqlx::query("ALTER TABLE messages ADD COLUMN total_tokens INTEGER NOT NULL DEFAULT 0")
                .execute(&self.pool)
                .await?;
            sqlx::query("ALTER TABLE messages ADD COLUMN elapsed_ms INTEGER NOT NULL DEFAULT 0")
                .execute(&self.pool)
                .await?;
            tracing::info!("migrated messages table: added per-message usage columns");
        }

        // conversation_traces table for harness / eval replay
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS conversation_traces (
                trace_id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                agent_id TEXT NOT NULL,
                model TEXT NOT NULL,
                context_window INTEGER,
                started_at TEXT NOT NULL,
                finished_at TEXT,
                turns_json TEXT NOT NULL DEFAULT '[]',
                metadata_json TEXT NOT NULL DEFAULT '{}',
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_traces_session ON conversation_traces(session_id)",
        )
        .execute(&self.pool)
        .await?;

        // Migration: add subagent_runs table if missing
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS subagent_runs (
                run_id TEXT PRIMARY KEY,
                parent_session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                parent_message_id TEXT NOT NULL DEFAULT '',
                agent_id TEXT NOT NULL,
                subagent_type TEXT NOT NULL DEFAULT 'general',
                task TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                result TEXT,
                tool_calls_made INTEGER NOT NULL DEFAULT 0,
                iterations INTEGER NOT NULL DEFAULT 0,
                token_usage_json TEXT,
                depth INTEGER NOT NULL DEFAULT 1,
                elapsed_ms INTEGER,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                completed_at TEXT
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_subagent_runs_session ON subagent_runs(parent_session_id, created_at DESC)"
        )
        .execute(&self.pool)
        .await?;

        // content_replacement_records: persist per-message budget decisions for session resume
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS content_replacement_records (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                tool_use_id TEXT NOT NULL,
                replacement TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_crr_session ON content_replacement_records(session_id)",
        )
        .execute(&self.pool)
        .await?;

        // collapse_state: persists CollapseStore JSON for session resume
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS collapse_state (
                session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
                state_json TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            )",
        )
        .execute(&self.pool)
        .await?;

        // session_memory: persists SessionMemory JSON across compaction cycles
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS session_memory (
                session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
                memory_json TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            )",
        )
        .execute(&self.pool)
        .await?;

        // Migration: add reasoning_content and compact_metadata_json to messages
        let has_reasoning: bool = sqlx::query_scalar::<_, i32>(
            "SELECT COUNT(*) FROM pragma_table_info('messages') WHERE name = 'reasoning_content'",
        )
        .fetch_one(&self.pool)
        .await
        .map(|c| c > 0)
        .unwrap_or(false);
        if !has_reasoning {
            sqlx::query("ALTER TABLE messages ADD COLUMN reasoning_content TEXT")
                .execute(&self.pool)
                .await?;
            sqlx::query("ALTER TABLE messages ADD COLUMN compact_metadata_json TEXT")
                .execute(&self.pool)
                .await?;
            tracing::info!("migrated messages table: added reasoning_content and compact_metadata_json columns");
        }

        // Migration: add source column to track session origin (client/feishu/api/cron)
        let has_source: bool = sqlx::query_scalar::<_, i32>(
            "SELECT COUNT(*) FROM pragma_table_info('sessions') WHERE name = 'source'",
        )
        .fetch_one(&self.pool)
        .await
        .map(|c| c > 0)
        .unwrap_or(false);
        if !has_source {
            sqlx::query("ALTER TABLE sessions ADD COLUMN source TEXT NOT NULL DEFAULT 'client'")
                .execute(&self.pool)
                .await?;
            // Backfill: infer source from session ID patterns
            sqlx::query(
                "UPDATE sessions SET source = 'feishu' WHERE id LIKE 'agent:%:feishu:%' OR id LIKE 'agent:%:group:feishu:%'"
            )
            .execute(&self.pool)
            .await?;
            sqlx::query("UPDATE sessions SET source = 'cron' WHERE title LIKE '[定时]%'")
                .execute(&self.pool)
                .await?;
            tracing::info!("migrated sessions table: added source column with backfill");
        }

        // Migration: add transcript_json column to subagent_runs for sidechain transcript
        let has_transcript: bool = sqlx::query_scalar::<_, i32>(
            "SELECT COUNT(*) FROM pragma_table_info('subagent_runs') WHERE name = 'transcript_json'",
        )
        .fetch_one(&self.pool)
        .await
        .map(|c| c > 0)
        .unwrap_or(false);
        if !has_transcript {
            sqlx::query("ALTER TABLE subagent_runs ADD COLUMN transcript_json TEXT")
                .execute(&self.pool)
                .await?;
            tracing::info!("migrated subagent_runs table: added transcript_json column");
        }

        // Migration: normalize all sessions.agent_id to "main" (single-agent refactoring)
        let non_main_count: i32 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sessions WHERE agent_id != 'main'",
        )
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0);
        if non_main_count > 0 {
            sqlx::query("UPDATE sessions SET agent_id = 'main' WHERE agent_id != 'main'")
                .execute(&self.pool)
                .await?;
            tracing::info!(
                count = non_main_count,
                "migrated sessions: normalized agent_id to 'main'"
            );
        }

        // history_items: canonical HistoryItem persistence (Sprint 7)
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS history_items (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                turn_id TEXT NOT NULL,
                item_type TEXT NOT NULL,
                item_json TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_hi_session ON history_items(session_id, id)",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_hi_turn ON history_items(session_id, turn_id)",
        )
        .execute(&self.pool)
        .await?;

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
        self.create_session_full(session_id, agent_id, title, None, None)
            .await
    }

    pub async fn create_session_with_work_dir(
        &self,
        session_id: &str,
        agent_id: &str,
        title: Option<&str>,
        work_dir: Option<&str>,
    ) -> anyhow::Result<SessionCreateOutcome> {
        self.create_session_full(session_id, agent_id, title, work_dir, None)
            .await
    }

    pub async fn create_session_full(
        &self,
        session_id: &str,
        agent_id: &str,
        title: Option<&str>,
        work_dir: Option<&str>,
        source: Option<&str>,
    ) -> anyhow::Result<SessionCreateOutcome> {
        let effective_agent_id = if agent_id != "main" {
            tracing::debug!(
                original = %agent_id,
                "normalizing session agent_id to 'main'"
            );
            "main"
        } else {
            agent_id
        };

        let mut tx = self.pool.begin().await?;

        let existed: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE id = ?")
            .bind(session_id)
            .fetch_one(&mut *tx)
            .await?;

        sqlx::query(
            "INSERT INTO sessions (id, agent_id, title, work_dir, source) VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET updated_at = datetime('now')",
        )
        .bind(session_id)
        .bind(effective_agent_id)
        .bind(title)
        .bind(work_dir)
        .bind(source.unwrap_or("client"))
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
            "SELECT id, agent_id, title, work_dir, source, created_at, updated_at, message_count,
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
            "SELECT id, agent_id, title, work_dir, source, created_at, updated_at, message_count,
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
                source: s.source,
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
            let vec = Arc::make_mut(cached);
            vec.push(msg.clone());
            if vec.len() > MSG_CACHE_MAX_MESSAGES_PER_SESSION {
                let excess = vec.len() - MSG_CACHE_MAX_MESSAGES_PER_SESSION;
                vec.drain(..excess);
            }
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
            .map(serde_json::to_string)
            .transpose()?;

        let content_json: Option<String> = match &msg.content {
            None => None,
            Some(v) => Some(serde_json::to_string(v)?),
        };

        let compact_metadata_json = msg
            .compact_metadata
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;

        sqlx::query(
            "INSERT INTO messages (session_id, role, content, name, tool_calls_json, tool_call_id, reasoning_content, compact_metadata_json)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(session_id)
        .bind(role)
        .bind(&content_json)
        .bind(&msg.name)
        .bind(&tool_calls_json)
        .bind(&msg.tool_call_id)
        .bind(&msg.reasoning_content)
        .bind(&compact_metadata_json)
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
            let vec = Arc::make_mut(cached);
            vec.extend_from_slice(messages);
            if vec.len() > MSG_CACHE_MAX_MESSAGES_PER_SESSION {
                let excess = vec.len() - MSG_CACHE_MAX_MESSAGES_PER_SESSION;
                vec.drain(..excess);
            }
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
                ..Default::default()
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
            "SELECT id, session_id, role, content, name, tool_calls_json, tool_call_id, created_at,
                    prompt_tokens, completion_tokens, total_tokens, elapsed_ms,
                    reasoning_content, compact_metadata_json
             FROM messages WHERE session_id = ? ORDER BY id ASC",
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(messages)
    }

    /// Convert stored messages back into ChatMessage format for the LLM.
    /// Uses an in-memory cache to avoid re-reading from SQLite on every turn.
    /// Returns `Arc<Vec<ChatMessage>>` so callers share the same allocation.
    pub async fn load_chat_messages(&self, session_id: &str) -> anyhow::Result<Arc<Vec<ChatMessage>>> {
        {
            let cache = self.msg_cache.read().await;
            if let Some(cached) = cache.get(session_id) {
                return Ok(Arc::clone(cached));
            }
        }

        let messages = self.load_chat_messages_from_db(session_id).await?;

        let arc = {
            let mut cache = self.msg_cache.write().await;
            if cache.len() >= MSG_CACHE_MAX_SESSIONS && !cache.contains_key(session_id) {
                if let Some(oldest) = cache.keys().next().cloned() {
                    cache.remove(&oldest);
                }
            }
            let cached = if messages.len() > MSG_CACHE_MAX_MESSAGES_PER_SESSION {
                Arc::new(messages[messages.len() - MSG_CACHE_MAX_MESSAGES_PER_SESSION..].to_vec())
            } else {
                Arc::new(messages)
            };
            cache.insert(session_id.to_string(), Arc::clone(&cached));
            cached
        };

        Ok(arc)
    }

    fn parse_chat_messages_from_rows(
        rows: Vec<SessionMessage>,
    ) -> anyhow::Result<Vec<ChatMessage>> {
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

            let compact_metadata = row
                .compact_metadata_json
                .as_deref()
                .map(serde_json::from_str)
                .transpose()?;

            messages.push(ChatMessage {
                role,
                content,
                reasoning_content: row.reasoning_content,
                name: row.name,
                tool_calls,
                tool_call_id: row.tool_call_id,
                compact_metadata,
            });
        }

        Ok(messages)
    }

    async fn load_chat_messages_from_db(
        &self,
        session_id: &str,
    ) -> anyhow::Result<Vec<ChatMessage>> {
        let rows = self.load_messages(session_id).await?;
        Self::parse_chat_messages_from_rows(rows)
    }

    /// Replace all messages in a session atomically (used by compaction).
    pub async fn replace_messages(
        &self,
        session_id: &str,
        messages: &[ChatMessage],
    ) -> anyhow::Result<()> {
        let mut tx = self.pool.begin().await?;

        sqlx::query("DELETE FROM messages WHERE session_id = ?")
            .bind(session_id)
            .execute(&mut *tx)
            .await?;

        // Reset message_count before re-inserting (each append increments by 1).
        sqlx::query(
            "UPDATE sessions SET message_count = 0, updated_at = datetime('now') WHERE id = ?",
        )
        .bind(session_id)
        .execute(&mut *tx)
        .await?;

        for msg in messages {
            Self::append_message_in_transaction(&mut tx, session_id, msg).await?;
        }

        tx.commit().await?;

        let mut cache = self.msg_cache.write().await;
        cache.insert(session_id.to_string(), Arc::new(messages.to_vec()));

        Ok(())
    }

    /// Invalidate the message cache for a session (e.g. after external edits).
    pub async fn invalidate_msg_cache(&self, session_id: &str) {
        let mut cache = self.msg_cache.write().await;
        cache.remove(session_id);
    }

    /// Update the title of an existing session.
    pub async fn update_title(&self, session_id: &str, title: &str) -> anyhow::Result<bool> {
        let result =
            sqlx::query("UPDATE sessions SET title = ?, updated_at = datetime('now') WHERE id = ?")
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

    /// Stamp the most recent assistant message in a session with per-message usage.
    pub async fn stamp_last_assistant_usage(
        &self,
        session_id: &str,
        prompt_tokens: u32,
        completion_tokens: u32,
        total_tokens: u32,
        elapsed_ms: u64,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE messages SET
                prompt_tokens = ?, completion_tokens = ?, total_tokens = ?, elapsed_ms = ?
             WHERE id = (SELECT MAX(id) FROM messages WHERE session_id = ? AND role = 'assistant')",
        )
        .bind(prompt_tokens as i64)
        .bind(completion_tokens as i64)
        .bind(total_tokens as i64)
        .bind(elapsed_ms as i64)
        .bind(session_id)
        .execute(&self.pool)
        .await?;
        Ok(())
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

    // -----------------------------------------------------------------------
    // Conversation trace CRUD
    // -----------------------------------------------------------------------

    /// Insert or replace a conversation trace.
    pub async fn upsert_trace(
        &self,
        trace: &fastclaw_core::types::ConversationTrace,
    ) -> anyhow::Result<()> {
        let turns_json = serde_json::to_string(&trace.turns)?;
        let metadata_json = serde_json::to_string(&trace.metadata)?;
        let cw = trace.context_window.map(|v| v as i64);
        sqlx::query(
            "INSERT OR REPLACE INTO conversation_traces
                (trace_id, session_id, agent_id, model, context_window, started_at, finished_at, turns_json, metadata_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )
        .bind(&trace.trace_id)
        .bind(&trace.session_id)
        .bind(&trace.agent_id)
        .bind(&trace.model)
        .bind(cw)
        .bind(&trace.started_at)
        .bind(&trace.finished_at)
        .bind(&turns_json)
        .bind(&metadata_json)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Retrieve a trace by its ID.
    pub async fn get_trace(
        &self,
        trace_id: &str,
    ) -> anyhow::Result<Option<fastclaw_core::types::ConversationTrace>> {
        let row = sqlx::query_as::<_, (String, String, String, String, Option<i64>, String, Option<String>, String, String)>(
            "SELECT trace_id, session_id, agent_id, model, context_window, started_at, finished_at, turns_json, metadata_json
             FROM conversation_traces WHERE trace_id = ?1",
        )
        .bind(trace_id)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(r) => Ok(Some(row_to_trace(r)?)),
            None => Ok(None),
        }
    }

    /// List traces, most recent first.
    pub async fn list_traces(
        &self,
        limit: u32,
        offset: u32,
    ) -> anyhow::Result<Vec<fastclaw_core::types::ConversationTrace>> {
        let rows = sqlx::query_as::<_, (String, String, String, String, Option<i64>, String, Option<String>, String, String)>(
            "SELECT trace_id, session_id, agent_id, model, context_window, started_at, finished_at, turns_json, metadata_json
             FROM conversation_traces ORDER BY created_at DESC LIMIT ?1 OFFSET ?2",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(row_to_trace).collect()
    }

    /// Delete a trace by ID.
    pub async fn delete_trace(&self, trace_id: &str) -> anyhow::Result<bool> {
        let result = sqlx::query("DELETE FROM conversation_traces WHERE trace_id = ?1")
            .bind(trace_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    // ── Sub-agent run persistence ────────────────────────────────────

    /// Save a sub-agent run snapshot (insert or update).
    pub async fn save_subagent_run(&self, run: &SubAgentRunRow) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO subagent_runs (
                run_id, parent_session_id, parent_message_id, agent_id,
                subagent_type, task, status, result,
                tool_calls_made, iterations, token_usage_json,
                depth, elapsed_ms, created_at, completed_at, transcript_json
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(run_id) DO UPDATE SET
                status = excluded.status,
                result = excluded.result,
                tool_calls_made = excluded.tool_calls_made,
                iterations = excluded.iterations,
                token_usage_json = excluded.token_usage_json,
                elapsed_ms = excluded.elapsed_ms,
                completed_at = excluded.completed_at,
                transcript_json = excluded.transcript_json",
        )
        .bind(&run.run_id)
        .bind(&run.parent_session_id)
        .bind(&run.parent_message_id)
        .bind(&run.agent_id)
        .bind(&run.subagent_type)
        .bind(&run.task)
        .bind(&run.status)
        .bind(&run.result)
        .bind(run.tool_calls_made)
        .bind(run.iterations)
        .bind(&run.token_usage_json)
        .bind(run.depth)
        .bind(run.elapsed_ms)
        .bind(&run.created_at)
        .bind(&run.completed_at)
        .bind(&run.transcript_json)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Get a single sub-agent run by its run_id.
    pub async fn get_subagent_run(&self, run_id: &str) -> anyhow::Result<Option<SubAgentRunRow>> {
        let row =
            sqlx::query_as::<_, SubAgentRunRow>("SELECT * FROM subagent_runs WHERE run_id = ?")
                .bind(run_id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row)
    }

    /// List sub-agent runs for a parent session, ordered by creation time (newest first).
    pub async fn list_subagent_runs(
        &self,
        parent_session_id: &str,
    ) -> anyhow::Result<Vec<SubAgentRunRow>> {
        let rows = sqlx::query_as::<_, SubAgentRunRow>(
            "SELECT * FROM subagent_runs WHERE parent_session_id = ? ORDER BY created_at DESC",
        )
        .bind(parent_session_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Delete sub-agent runs that are terminal and older than `max_age_hours`.
    pub async fn cleanup_subagent_runs(&self, max_age_hours: u64) -> anyhow::Result<u64> {
        let result = sqlx::query(
            "DELETE FROM subagent_runs WHERE status IN ('completed', 'failed', 'cancelled') AND created_at < datetime('now', ?)"
        )
        .bind(format!("-{max_age_hours} hours"))
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    // ── Content Replacement Records ─────────────────────────────────────

    /// Persist content replacement records for a session.
    /// These records allow byte-identical reconstruction of `ContentReplacementState`
    /// on session resume, preserving prompt cache prefix stability.
    pub async fn save_replacement_records(
        &self,
        session_id: &str,
        records: &[crate::models::ContentReplacementRow],
    ) -> anyhow::Result<()> {
        if records.is_empty() {
            return Ok(());
        }

        let mut tx = self.pool.begin().await?;
        for record in records {
            sqlx::query(
                "INSERT INTO content_replacement_records (session_id, tool_use_id, replacement)
                 VALUES (?, ?, ?)",
            )
            .bind(session_id)
            .bind(&record.tool_use_id)
            .bind(&record.replacement)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;

        tracing::debug!(
            session_id,
            count = records.len(),
            "saved content replacement records"
        );
        Ok(())
    }

    /// Load all content replacement records for a session, ordered by insertion.
    pub async fn load_replacement_records(
        &self,
        session_id: &str,
    ) -> anyhow::Result<Vec<crate::models::ContentReplacementRow>> {
        let rows = sqlx::query_as::<_, crate::models::ContentReplacementRow>(
            "SELECT tool_use_id, replacement FROM content_replacement_records
             WHERE session_id = ? ORDER BY id ASC",
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// Delete all content replacement records for a session (e.g. on session reset).
    pub async fn delete_replacement_records(&self, session_id: &str) -> anyhow::Result<u64> {
        let result = sqlx::query("DELETE FROM content_replacement_records WHERE session_id = ?")
            .bind(session_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }

    // ── Collapse state persistence ──────────────────────────────────

    /// Save the collapse state JSON for a session (upsert).
    ///
    /// The caller is responsible for serializing the `CollapseStore` to JSON.
    /// This decouples `fastclaw-session` from `fastclaw-context`.
    pub async fn save_collapse_state(
        &self,
        session_id: &str,
        state_json: &str,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO collapse_state (session_id, state_json, updated_at)
             VALUES (?, ?, datetime('now'))
             ON CONFLICT(session_id) DO UPDATE SET
                state_json = excluded.state_json,
                updated_at = excluded.updated_at",
        )
        .bind(session_id)
        .bind(state_json)
        .execute(&self.pool)
        .await?;

        tracing::debug!(session_id, "saved collapse state");
        Ok(())
    }

    /// Load the collapse state JSON for a session.
    ///
    /// Returns `None` if no collapse state has been saved for this session.
    /// The caller deserializes the JSON back into a `CollapseStore`.
    pub async fn load_collapse_state(&self, session_id: &str) -> anyhow::Result<Option<String>> {
        let row = sqlx::query_scalar::<_, String>(
            "SELECT state_json FROM collapse_state WHERE session_id = ?",
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Persist session memory JSON for a session.
    pub async fn save_session_memory(
        &self,
        session_id: &str,
        memory_json: &str,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO session_memory (session_id, memory_json, updated_at)
             VALUES (?, ?, datetime('now'))
             ON CONFLICT(session_id) DO UPDATE SET
                memory_json = excluded.memory_json,
                updated_at = excluded.updated_at",
        )
        .bind(session_id)
        .bind(memory_json)
        .execute(&self.pool)
        .await?;

        tracing::debug!(session_id, "saved session memory");
        Ok(())
    }

    /// Load session memory JSON for a session.
    pub async fn load_session_memory(&self, session_id: &str) -> anyhow::Result<Option<String>> {
        let row = sqlx::query_scalar::<_, String>(
            "SELECT memory_json FROM session_memory WHERE session_id = ?",
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Append a single HistoryItem.
    pub async fn append_history_item(
        &self,
        session_id: &str,
        item: &fastclaw_protocol::HistoryItem,
    ) -> anyhow::Result<()> {
        let turn_id = item.turn_id().as_str();
        let item_type = extract_history_item_type(item);
        let item_json = serde_json::to_string(item)?;

        sqlx::query(
            "INSERT INTO history_items (session_id, turn_id, item_type, item_json) VALUES (?, ?, ?, ?)",
        )
        .bind(session_id)
        .bind(turn_id)
        .bind(item_type)
        .bind(&item_json)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Append multiple HistoryItems in a single transaction.
    pub async fn append_history_items(
        &self,
        session_id: &str,
        items: &[fastclaw_protocol::HistoryItem],
    ) -> anyhow::Result<()> {
        if items.is_empty() {
            return Ok(());
        }
        let mut tx = self.pool.begin().await?;
        for item in items {
            let turn_id = item.turn_id().as_str();
            let item_type = extract_history_item_type(item);
            let item_json = serde_json::to_string(item)?;

            sqlx::query(
                "INSERT INTO history_items (session_id, turn_id, item_type, item_json) VALUES (?, ?, ?, ?)",
            )
            .bind(session_id)
            .bind(turn_id)
            .bind(item_type)
            .bind(&item_json)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Load all HistoryItems for a session, ordered by insertion.
    pub async fn load_history(
        &self,
        session_id: &str,
    ) -> anyhow::Result<Vec<fastclaw_protocol::HistoryItem>> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT item_json FROM history_items WHERE session_id = ? ORDER BY id",
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;

        rows.iter()
            .map(|(json,)| serde_json::from_str(json).map_err(Into::into))
            .collect()
    }

    /// Load HistoryItems for a specific turn.
    pub async fn load_history_for_turn(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> anyhow::Result<Vec<fastclaw_protocol::HistoryItem>> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT item_json FROM history_items WHERE session_id = ? AND turn_id = ? ORDER BY id",
        )
        .bind(session_id)
        .bind(turn_id)
        .fetch_all(&self.pool)
        .await?;

        rows.iter()
            .map(|(json,)| serde_json::from_str(json).map_err(Into::into))
            .collect()
    }
}

fn extract_history_item_type(item: &fastclaw_protocol::HistoryItem) -> &'static str {
    match item {
        fastclaw_protocol::HistoryItem::Message { .. } => "message",
        fastclaw_protocol::HistoryItem::ToolUse { .. } => "tool_use",
        fastclaw_protocol::HistoryItem::CompactBoundary { .. } => "compact_boundary",
        fastclaw_protocol::HistoryItem::TurnUsage { .. } => "turn_usage",
        _ => "unknown",
    }
}

fn row_to_trace(
    r: (
        String,
        String,
        String,
        String,
        Option<i64>,
        String,
        Option<String>,
        String,
        String,
    ),
) -> anyhow::Result<fastclaw_core::types::ConversationTrace> {
    Ok(fastclaw_core::types::ConversationTrace {
        trace_id: r.0,
        session_id: r.1,
        agent_id: r.2,
        model: r.3,
        context_window: r.4.map(|v| v as u32),
        started_at: r.5,
        finished_at: r.6,
        turns: serde_json::from_str(&r.7)?,
        metadata: serde_json::from_str(&r.8)?,
    })
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
                reasoning_content: None,
                name: None,
                tool_calls: None,
                tool_call_id: None,
            compact_metadata: None,
            },
            ChatMessage {
                role: Role::Assistant,
                content: Some("b".into()),
                reasoning_content: None,
                name: None,
                tool_calls: None,
                tool_call_id: None,
            compact_metadata: None,
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
                reasoning_content: None,
                name: None,
                tool_calls: None,
                tool_call_id: None,
            compact_metadata: None,
            },
            ChatMessage {
                role: Role::User,
                content: Some("second".into()),
                reasoning_content: None,
                name: None,
                tool_calls: None,
                tool_call_id: None,
            compact_metadata: None,
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
            reasoning_content: None,
            name: None,
            tool_calls: None,
            tool_call_id: None,
            compact_metadata: None,
        }];
        assert!(store.append_messages("missing", &msgs).await.is_err());
    }

    #[tokio::test]
    async fn create_session_duplicate_id_refreshes_timestamp_keeps_metadata() {
        let store = SessionStore::open_memory().await.unwrap();
        assert_eq!(
            store
                .create_session("dup", "main", Some("first"))
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
                .create_session("dup", "main", Some("second"))
                .await
                .unwrap(),
            SessionCreateOutcome::AlreadyExisted
        );

        let s = store.get_session("dup").await.unwrap().unwrap();
        assert_eq!(s.agent_id, "main");
        assert_eq!(s.title.as_deref(), Some("first"));
        assert_ne!(
            s.updated_at, stale_updated_at,
            "ON CONFLICT should bump updated_at; stale was {stale_updated_at}"
        );
    }

    fn sample_trace(id: &str) -> fastclaw_core::types::ConversationTrace {
        fastclaw_core::types::ConversationTrace {
            trace_id: id.to_string(),
            session_id: "s1".into(),
            agent_id: "a1".into(),
            model: "gpt-4o".into(),
            context_window: Some(128000),
            started_at: "2026-01-01T00:00:00Z".into(),
            finished_at: Some("2026-01-01T00:00:01Z".into()),
            turns: vec![],
            metadata: serde_json::Map::new(),
        }
    }

    #[tokio::test]
    async fn upsert_and_get_trace() {
        let store = SessionStore::open_memory().await.unwrap();
        let t = sample_trace("tr-1");
        store.upsert_trace(&t).await.unwrap();
        let got = store.get_trace("tr-1").await.unwrap().unwrap();
        assert_eq!(got.trace_id, "tr-1");
        assert_eq!(got.agent_id, "a1");
        assert_eq!(got.model, "gpt-4o");
        assert_eq!(got.context_window, Some(128000));
    }

    #[tokio::test]
    async fn list_traces_pagination() {
        let store = SessionStore::open_memory().await.unwrap();
        for i in 0..5 {
            store
                .upsert_trace(&sample_trace(&format!("tr-{i}")))
                .await
                .unwrap();
        }
        let page1 = store.list_traces(2, 0).await.unwrap();
        assert_eq!(page1.len(), 2);
        let page2 = store.list_traces(2, 2).await.unwrap();
        assert_eq!(page2.len(), 2);
        let page3 = store.list_traces(2, 4).await.unwrap();
        assert_eq!(page3.len(), 1);
    }

    #[tokio::test]
    async fn delete_trace_removes() {
        let store = SessionStore::open_memory().await.unwrap();
        store.upsert_trace(&sample_trace("tr-del")).await.unwrap();
        assert!(store.get_trace("tr-del").await.unwrap().is_some());
        let deleted = store.delete_trace("tr-del").await.unwrap();
        assert!(deleted);
        assert!(store.get_trace("tr-del").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn upsert_trace_overwrites() {
        let store = SessionStore::open_memory().await.unwrap();
        let mut t = sample_trace("tr-ow");
        t.model = "old-model".into();
        store.upsert_trace(&t).await.unwrap();

        t.model = "new-model".into();
        store.upsert_trace(&t).await.unwrap();

        let got = store.get_trace("tr-ow").await.unwrap().unwrap();
        assert_eq!(got.model, "new-model");
    }

    #[tokio::test]
    async fn get_nonexistent_trace() {
        let store = SessionStore::open_memory().await.unwrap();
        assert!(store.get_trace("nope").await.unwrap().is_none());
    }

    fn make_subagent_row(run_id: &str, session_id: &str, status: &str) -> SubAgentRunRow {
        SubAgentRunRow {
            run_id: run_id.into(),
            parent_session_id: session_id.into(),
            parent_message_id: "msg1".into(),
            agent_id: "agent1".into(),
            subagent_type: "general".into(),
            task: "test task".into(),
            status: status.into(),
            result: Some("done".into()),
            tool_calls_made: 3,
            iterations: 2,
            token_usage_json: None,
            depth: 0,
            elapsed_ms: Some(1500),
            created_at: "2025-01-01T00:00:00Z".into(),
            completed_at: Some("2025-01-01T00:00:01Z".into()),
            transcript_json: None,
        }
    }

    async fn setup_store_with_sessions(session_ids: &[&str]) -> SessionStore {
        let store = SessionStore::open_memory().await.unwrap();
        for &sid in session_ids {
            store.create_session(sid, "agent", None).await.unwrap();
        }
        store
    }

    #[tokio::test]
    async fn save_and_get_subagent_run() {
        let store = setup_store_with_sessions(&["s1"]).await;
        let row = make_subagent_row("run1", "s1", "completed");

        store.save_subagent_run(&row).await.unwrap();
        let loaded = store.get_subagent_run("run1").await.unwrap().unwrap();
        assert_eq!(loaded.run_id, "run1");
        assert_eq!(loaded.status, "completed");
        assert_eq!(loaded.tool_calls_made, 3);
        assert_eq!(loaded.task, "test task");
    }

    #[tokio::test]
    async fn save_subagent_run_upserts_on_conflict() {
        let store = setup_store_with_sessions(&["s1"]).await;
        let mut row = make_subagent_row("run1", "s1", "running");
        row.result = None;
        store.save_subagent_run(&row).await.unwrap();

        row.status = "completed".into();
        row.result = Some("final result".into());
        row.tool_calls_made = 5;
        store.save_subagent_run(&row).await.unwrap();

        let loaded = store.get_subagent_run("run1").await.unwrap().unwrap();
        assert_eq!(loaded.status, "completed");
        assert_eq!(loaded.result.as_deref(), Some("final result"));
        assert_eq!(loaded.tool_calls_made, 5);
    }

    #[tokio::test]
    async fn list_subagent_runs_filters_by_session() {
        let store = setup_store_with_sessions(&["s1", "s2"]).await;
        store
            .save_subagent_run(&make_subagent_row("r1", "s1", "completed"))
            .await
            .unwrap();
        store
            .save_subagent_run(&make_subagent_row("r2", "s1", "failed"))
            .await
            .unwrap();
        store
            .save_subagent_run(&make_subagent_row("r3", "s2", "completed"))
            .await
            .unwrap();

        let s1_runs = store.list_subagent_runs("s1").await.unwrap();
        assert_eq!(s1_runs.len(), 2);
        assert!(s1_runs.iter().all(|r| r.parent_session_id == "s1"));

        let s2_runs = store.list_subagent_runs("s2").await.unwrap();
        assert_eq!(s2_runs.len(), 1);
    }

    #[tokio::test]
    async fn get_subagent_run_returns_none_for_unknown() {
        let store = SessionStore::open_memory().await.unwrap();
        assert!(store
            .get_subagent_run("nonexistent")
            .await
            .unwrap()
            .is_none());
    }

    // ── Content Replacement Record tests ────────────────────────────────

    use crate::models::ContentReplacementRow;

    #[tokio::test]
    async fn save_and_load_replacement_records() {
        let store = SessionStore::open_memory().await.unwrap();
        store.create_session("s1", "agent", None).await.unwrap();

        let records = vec![
            ContentReplacementRow {
                tool_use_id: "tu_1".into(),
                replacement: "<persisted-output>\npreview 1\n</persisted-output>".into(),
            },
            ContentReplacementRow {
                tool_use_id: "tu_2".into(),
                replacement: "<persisted-output>\npreview 2\n</persisted-output>".into(),
            },
        ];
        store
            .save_replacement_records("s1", &records)
            .await
            .unwrap();

        let loaded = store.load_replacement_records("s1").await.unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].tool_use_id, "tu_1");
        assert_eq!(
            loaded[0].replacement,
            "<persisted-output>\npreview 1\n</persisted-output>"
        );
        assert_eq!(loaded[1].tool_use_id, "tu_2");
        assert_eq!(
            loaded[1].replacement,
            "<persisted-output>\npreview 2\n</persisted-output>"
        );
    }

    #[tokio::test]
    async fn load_replacement_records_empty_when_none_saved() {
        let store = SessionStore::open_memory().await.unwrap();
        store.create_session("s1", "agent", None).await.unwrap();

        let loaded = store.load_replacement_records("s1").await.unwrap();
        assert!(loaded.is_empty());
    }

    #[tokio::test]
    async fn save_replacement_records_empty_vec_is_noop() {
        let store = SessionStore::open_memory().await.unwrap();
        store.create_session("s1", "agent", None).await.unwrap();

        store.save_replacement_records("s1", &[]).await.unwrap();
        let loaded = store.load_replacement_records("s1").await.unwrap();
        assert!(loaded.is_empty());
    }

    #[tokio::test]
    async fn save_replacement_records_appends_to_existing() {
        let store = SessionStore::open_memory().await.unwrap();
        store.create_session("s1", "agent", None).await.unwrap();

        let batch1 = vec![ContentReplacementRow {
            tool_use_id: "tu_1".into(),
            replacement: "[r1]".into(),
        }];
        store.save_replacement_records("s1", &batch1).await.unwrap();

        let batch2 = vec![ContentReplacementRow {
            tool_use_id: "tu_2".into(),
            replacement: "[r2]".into(),
        }];
        store.save_replacement_records("s1", &batch2).await.unwrap();

        let loaded = store.load_replacement_records("s1").await.unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].tool_use_id, "tu_1");
        assert_eq!(loaded[1].tool_use_id, "tu_2");
    }

    #[tokio::test]
    async fn delete_replacement_records_removes_all() {
        let store = SessionStore::open_memory().await.unwrap();
        store.create_session("s1", "agent", None).await.unwrap();

        let records = vec![
            ContentReplacementRow {
                tool_use_id: "tu_1".into(),
                replacement: "[r1]".into(),
            },
            ContentReplacementRow {
                tool_use_id: "tu_2".into(),
                replacement: "[r2]".into(),
            },
        ];
        store
            .save_replacement_records("s1", &records)
            .await
            .unwrap();

        let deleted = store.delete_replacement_records("s1").await.unwrap();
        assert_eq!(deleted, 2);

        let loaded = store.load_replacement_records("s1").await.unwrap();
        assert!(loaded.is_empty());
    }

    #[tokio::test]
    async fn replacement_records_cascade_on_session_delete() {
        let store = SessionStore::open_memory().await.unwrap();
        store.create_session("s1", "agent", None).await.unwrap();

        let records = vec![ContentReplacementRow {
            tool_use_id: "tu_1".into(),
            replacement: "[r]".into(),
        }];
        store
            .save_replacement_records("s1", &records)
            .await
            .unwrap();

        store.delete_session("s1").await.unwrap();
        let loaded = store.load_replacement_records("s1").await.unwrap();
        assert!(loaded.is_empty());
    }

    #[tokio::test]
    async fn replacement_records_isolated_between_sessions() {
        let store = SessionStore::open_memory().await.unwrap();
        store.create_session("s1", "agent", None).await.unwrap();
        store.create_session("s2", "agent", None).await.unwrap();

        store
            .save_replacement_records(
                "s1",
                &[ContentReplacementRow {
                    tool_use_id: "tu_a".into(),
                    replacement: "[ra]".into(),
                }],
            )
            .await
            .unwrap();
        store
            .save_replacement_records(
                "s2",
                &[ContentReplacementRow {
                    tool_use_id: "tu_b".into(),
                    replacement: "[rb]".into(),
                }],
            )
            .await
            .unwrap();

        let s1 = store.load_replacement_records("s1").await.unwrap();
        assert_eq!(s1.len(), 1);
        assert_eq!(s1[0].tool_use_id, "tu_a");

        let s2 = store.load_replacement_records("s2").await.unwrap();
        assert_eq!(s2.len(), 1);
        assert_eq!(s2[0].tool_use_id, "tu_b");
    }

    #[tokio::test]
    async fn save_and_load_collapse_state() {
        let store = SessionStore::open_memory().await.unwrap();
        store.create_session("s1", "agent", None).await.unwrap();

        let state = r#"{"spans":{"0":{"start_round":0,"end_round":2,"summary":"round 0-2 summary","summary_tokens":10,"original_tokens":500,"created_at":1700000000000}}}"#;
        store.save_collapse_state("s1", state).await.unwrap();

        let loaded = store.load_collapse_state("s1").await.unwrap();
        assert_eq!(loaded.as_deref(), Some(state));
    }

    #[tokio::test]
    async fn load_collapse_state_returns_none_when_empty() {
        let store = SessionStore::open_memory().await.unwrap();
        store.create_session("s1", "agent", None).await.unwrap();

        let loaded = store.load_collapse_state("s1").await.unwrap();
        assert!(loaded.is_none(), "should return None for unsaved state");
    }

    #[tokio::test]
    async fn save_collapse_state_upserts() {
        let store = SessionStore::open_memory().await.unwrap();
        store.create_session("s1", "agent", None).await.unwrap();

        store
            .save_collapse_state("s1", r#"{"spans":{}}"#)
            .await
            .unwrap();
        store.save_collapse_state("s1", r#"{"spans":{"0":{"start_round":0,"end_round":1,"summary":"v2","summary_tokens":5,"original_tokens":100,"created_at":0}}}"#).await.unwrap();

        let loaded = store.load_collapse_state("s1").await.unwrap().unwrap();
        assert!(loaded.contains("v2"), "should have the updated state");
        assert!(
            !loaded.contains(r#""spans":{}"#),
            "should not have old empty state"
        );
    }

    #[tokio::test]
    async fn collapse_state_isolated_per_session() {
        let store = SessionStore::open_memory().await.unwrap();
        store.create_session("s1", "agent", None).await.unwrap();
        store.create_session("s2", "agent", None).await.unwrap();

        store.save_collapse_state("s1", r#"{"spans":{"0":{"start_round":0,"end_round":0,"summary":"s1 data","summary_tokens":5,"original_tokens":50,"created_at":0}}}"#).await.unwrap();

        let s1 = store.load_collapse_state("s1").await.unwrap();
        assert!(s1.is_some());
        assert!(s1.unwrap().contains("s1 data"));

        let s2 = store.load_collapse_state("s2").await.unwrap();
        assert!(s2.is_none(), "s2 should have no state");
    }

    #[tokio::test]
    async fn history_items_roundtrip() {
        use fastclaw_protocol::{ContentPart, HistoryItem, TurnId};

        let store = SessionStore::open_memory().await.unwrap();
        store
            .create_session("s1", "agent-1", Some("test"))
            .await
            .unwrap();

        let turn_id = TurnId::new("t1");
        let items = vec![
            HistoryItem::Message {
                turn_id: turn_id.clone(),
                role: Role::User,
                content: vec![ContentPart::Text {
                    text: "hello".into(),
                }],
                phase: None,
                reasoning_content: None,
            },
            HistoryItem::ToolUse {
                turn_id: turn_id.clone(),
                call_id: "tc-1".into(),
                tool_name: "read_file".into(),
                arguments: r#"{"path":"a.txt"}"#.into(),
                output: "file contents".into(),
                success: true,
                duration_ms: Some(42),
            },
        ];

        store.append_history_items("s1", &items).await.unwrap();

        let loaded = store.load_history("s1").await.unwrap();
        assert_eq!(loaded.len(), 2);

        let turn_items = store.load_history_for_turn("s1", "t1").await.unwrap();
        assert_eq!(turn_items.len(), 2);
    }
}
