use sqlx::sqlite::SqlitePool;
use sqlx::{Row, Transaction};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

use xiaolin_protocol::{SearchFilters, SearchIndexStatusResponse, SearchResult};

const META_LAST_EVENT_LOG_ID: &str = "last_event_log_id";
const META_LAST_MESSAGE_ID: &str = "last_message_id";
const BULK_COMMIT_INTERVAL: usize = 500;

/// Progress snapshot for background indexing.
pub type IndexStatus = SearchIndexStatusResponse;

/// FTS5-backed full-text search index over message content.
pub struct SearchIndex {
    pool: SqlitePool,
    is_indexing: AtomicBool,
}

impl SearchIndex {
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            pool,
            is_indexing: AtomicBool::new(false),
        }
    }

    pub fn pool(&self) -> SqlitePool {
        self.pool.clone()
    }

    pub fn is_indexing(&self) -> bool {
        self.is_indexing.load(Ordering::Relaxed)
    }

    /// Create FTS5 virtual table and metadata table if missing.
    pub async fn ensure_schema(&self) -> anyhow::Result<()> {
        sqlx::query(
            "CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
                content,
                session_id UNINDEXED,
                turn_id UNINDEXED,
                role UNINDEXED,
                message_id UNINDEXED,
                tokenize = 'unicode61'
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS search_index_meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Index a single row, delegating to upsert for deduplication.
    pub async fn index_row(
        &self,
        session_id: &str,
        turn_id: &str,
        role: &str,
        content: &str,
        message_id: Option<&str>,
    ) -> anyhow::Result<()> {
        self.upsert_row(session_id, turn_id, role, content, message_id)
            .await
    }

    /// Delete any existing row for the dedup key, then insert fresh content.
    pub async fn upsert_row(
        &self,
        session_id: &str,
        turn_id: &str,
        role: &str,
        content: &str,
        message_id: Option<&str>,
    ) -> anyhow::Result<()> {
        if content.trim().is_empty() {
            return Ok(());
        }

        let mut tx = self.pool.begin().await?;
        Self::upsert_row_in_tx(&mut tx, session_id, turn_id, role, content, message_id).await?;
        tx.commit().await?;
        Ok(())
    }

    /// True when event_log or messages tables have rows beyond the stored cursors.
    pub async fn needs_backfill(&self) -> anyhow::Result<bool> {
        let (event_cursor, message_cursor) = self.get_cursor().await?;

        let max_event_log_id: i64 =
            sqlx::query_scalar("SELECT COALESCE(MAX(id), 0) FROM event_log")
                .fetch_one(&self.pool)
                .await?;

        let max_message_id: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(id), 0) FROM messages")
            .fetch_one(&self.pool)
            .await?;

        Ok(event_cursor < max_event_log_id as u64 || message_cursor < max_message_id as u64)
    }

    /// Read `(last_event_log_id, last_message_id)` cursors from meta.
    pub async fn get_cursor(&self) -> anyhow::Result<(u64, u64)> {
        let event_log_id = self.get_meta_u64(META_LAST_EVENT_LOG_ID).await?;
        let message_id = self.get_meta_u64(META_LAST_MESSAGE_ID).await?;
        Ok((event_log_id, message_id))
    }

    /// Persist both indexing cursors.
    pub async fn set_cursor(&self, event_log_id: u64, message_id: u64) -> anyhow::Result<()> {
        self.set_meta_u64(META_LAST_EVENT_LOG_ID, event_log_id)
            .await?;
        self.set_meta_u64(META_LAST_MESSAGE_ID, message_id).await?;
        Ok(())
    }

    /// FTS search with BM25 ranking, snippet extraction, and session joins.
    /// For CJK queries, falls back to LIKE-based search since unicode61 tokenizer
    /// does not handle multi-character CJK terms well.
    pub async fn search(
        &self,
        query: &str,
        filters: &SearchFilters,
        limit: i64,
        offset: i64,
    ) -> anyhow::Result<Vec<SearchResult>> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }

        if contains_cjk(trimmed) {
            self.search_like(trimmed, filters, limit, offset).await
        } else {
            self.search_fts(trimmed, filters, limit, offset).await
        }
    }

    async fn search_fts(
        &self,
        query: &str,
        filters: &SearchFilters,
        limit: i64,
        offset: i64,
    ) -> anyhow::Result<Vec<SearchResult>> {
        let fts_query = prepare_fts_query(query);
        if fts_query.is_empty() {
            return Ok(Vec::new());
        }

        let mut sql = String::from(
            "SELECT f.session_id, f.turn_id, f.role, f.message_id,
                    COALESCE(s.title, '') AS session_title,
                    s.work_dir,
                    snippet(messages_fts, 0, '<b>', '</b>', '…', 32) AS snippet,
                    s.updated_at AS timestamp,
                    bm25(messages_fts) AS rank
             FROM messages_fts f
             JOIN sessions s ON s.id = f.session_id
             WHERE messages_fts MATCH ?",
        );

        if filters.work_dir.is_some() {
            sql.push_str(" AND s.work_dir = ?");
        }
        if filters.date_from.is_some() {
            sql.push_str(" AND s.updated_at >= ?");
        }
        if filters.date_to.is_some() {
            sql.push_str(" AND s.updated_at <= ?");
        }
        sql.push_str(" ORDER BY rank LIMIT ? OFFSET ?");

        let mut q = sqlx::query(&sql).bind(&fts_query);
        if let Some(work_dir) = &filters.work_dir {
            q = q.bind(work_dir);
        }
        if let Some(date_from) = &filters.date_from {
            q = q.bind(date_from);
        }
        if let Some(date_to) = &filters.date_to {
            q = q.bind(date_to);
        }
        q = q.bind(limit).bind(offset);

        let rows = q.fetch_all(&self.pool).await?;
        let mut results = Vec::with_capacity(rows.len());
        for row in rows {
            results.push(SearchResult {
                session_id: row.get("session_id"),
                turn_id: row.get("turn_id"),
                role: row.get("role"),
                message_id: row.get::<Option<String>, _>("message_id"),
                session_title: row.get("session_title"),
                work_dir: row.get("work_dir"),
                snippet: row.get("snippet"),
                timestamp: row.get("timestamp"),
                rank: row.get("rank"),
            });
        }

        Ok(results)
    }

    /// LIKE-based fallback for CJK queries where FTS5 unicode61 tokenizer is ineffective.
    async fn search_like(
        &self,
        query: &str,
        filters: &SearchFilters,
        limit: i64,
        offset: i64,
    ) -> anyhow::Result<Vec<SearchResult>> {
        let like_pattern = format!("%{}%", query.replace('%', "\\%").replace('_', "\\_"));

        let mut sql = String::from(
            "SELECT f.session_id, f.turn_id, f.role, f.message_id, f.content,
                    COALESCE(s.title, '') AS session_title,
                    s.work_dir,
                    s.updated_at AS timestamp
             FROM messages_fts f
             JOIN sessions s ON s.id = f.session_id
             WHERE f.content LIKE ? ESCAPE '\\'",
        );

        if filters.work_dir.is_some() {
            sql.push_str(" AND s.work_dir = ?");
        }
        if filters.date_from.is_some() {
            sql.push_str(" AND s.updated_at >= ?");
        }
        if filters.date_to.is_some() {
            sql.push_str(" AND s.updated_at <= ?");
        }
        sql.push_str(" ORDER BY s.updated_at DESC LIMIT ? OFFSET ?");

        let mut q = sqlx::query(&sql).bind(&like_pattern);
        if let Some(work_dir) = &filters.work_dir {
            q = q.bind(work_dir);
        }
        if let Some(date_from) = &filters.date_from {
            q = q.bind(date_from);
        }
        if let Some(date_to) = &filters.date_to {
            q = q.bind(date_to);
        }
        q = q.bind(limit).bind(offset);

        let rows = q.fetch_all(&self.pool).await?;
        let mut results = Vec::with_capacity(rows.len());
        for row in rows {
            let content: String = row.get("content");
            let snippet = generate_snippet(&content, query, 30);
            results.push(SearchResult {
                session_id: row.get("session_id"),
                turn_id: row.get("turn_id"),
                role: row.get("role"),
                message_id: row.get::<Option<String>, _>("message_id"),
                session_title: row.get("session_title"),
                work_dir: row.get("work_dir"),
                snippet,
                timestamp: row.get("timestamp"),
                rank: 0.0,
            });
        }

        Ok(results)
    }

    /// Remove all indexed rows for a deleted session.
    pub async fn delete_session(&self, session_id: &str) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM messages_fts WHERE session_id = ?")
            .bind(session_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Clear the FTS table and reset indexing cursors.
    pub async fn rebuild(&self) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM messages_fts")
            .execute(&self.pool)
            .await?;

        sqlx::query("DELETE FROM search_index_meta WHERE key IN (?, ?)")
            .bind(META_LAST_EVENT_LOG_ID)
            .bind(META_LAST_MESSAGE_ID)
            .execute(&self.pool)
            .await?;

        let _ = sqlx::query("INSERT INTO messages_fts(messages_fts) VALUES('optimize')")
            .execute(&self.pool)
            .await;

        Ok(())
    }

    /// Scan historical event_log and messages rows, committing every 500 upserts.
    pub async fn bulk_index_history(
        &self,
        progress_tx: Option<tokio::sync::watch::Sender<(u64, u64)>>,
    ) -> anyhow::Result<()> {
        self.is_indexing.store(true, Ordering::Relaxed);

        let result = self.bulk_index_history_inner(progress_tx).await;

        self.is_indexing.store(false, Ordering::Relaxed);
        result
    }

    /// Return indexed row count, estimated total, and whether bulk indexing is active.
    pub async fn index_status(&self) -> anyhow::Result<IndexStatus> {
        let indexed_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages_fts")
            .fetch_one(&self.pool)
            .await?;

        let total_count = self.count_searchable_sources().await?;

        Ok(IndexStatus {
            indexed_count: indexed_count.max(0) as u64,
            total_count,
            is_indexing: self.is_indexing(),
        })
    }

    async fn bulk_index_history_inner(
        &self,
        progress_tx: Option<tokio::sync::watch::Sender<(u64, u64)>>,
    ) -> anyhow::Result<()> {
        let total_count = self.count_searchable_sources().await?;
        let (mut event_cursor, mut message_cursor) = self.get_cursor().await?;
        let mut processed: u64 = 0;
        let mut pending_commits = 0usize;
        let mut delta_accum: HashMap<(String, String), String> = HashMap::new();

        let event_rows = sqlx::query(
            "SELECT id, session_id, turn_id, event_type, event_json
             FROM event_log
             WHERE id > ?
             ORDER BY id",
        )
        .bind(event_cursor as i64)
        .fetch_all(&self.pool)
        .await?;

        let message_rows = sqlx::query(
            "SELECT id, session_id, role, content
             FROM messages
             WHERE id > ? AND role IN ('user', 'assistant')
             ORDER BY id",
        )
        .bind(message_cursor as i64)
        .fetch_all(&self.pool)
        .await?;

        let mut tx = self.pool.begin().await?;

        for row in event_rows {
            let id: i64 = row.get("id");
            let session_id: String = row.get("session_id");
            let turn_id: String = row.get("turn_id");
            let event_type: String = row.get("event_type");
            let event_json: String = row.get("event_json");

            if let Some((role, content)) = extract_searchable_from_event(
                &event_type,
                &event_json,
                &mut delta_accum,
                &session_id,
                &turn_id,
            ) {
                Self::upsert_row_in_tx(&mut tx, &session_id, &turn_id, &role, &content, None)
                    .await?;
                pending_commits += 1;
            }

            event_cursor = id as u64;
            processed += 1;

            if pending_commits >= BULK_COMMIT_INTERVAL {
                self.set_cursor_in_tx(&mut tx, event_cursor, message_cursor)
                    .await?;
                tx.commit().await?;
                tx = self.pool.begin().await?;
                pending_commits = 0;
                if let Some(tx_progress) = &progress_tx {
                    let _ = tx_progress.send((processed, total_count));
                }
            }
        }

        for row in message_rows {
            let id: i64 = row.get("id");
            let session_id: String = row.get("session_id");
            let role: String = row.get("role");
            let content: Option<String> = row.get("content");

            if let Some(text) = extract_plain_text(content.as_deref()) {
                let turn_id = id.to_string();
                let message_id = id.to_string();
                Self::upsert_row_in_tx(
                    &mut tx,
                    &session_id,
                    &turn_id,
                    &role,
                    &text,
                    Some(&message_id),
                )
                .await?;
                pending_commits += 1;
            }

            message_cursor = id as u64;
            processed += 1;

            if pending_commits >= BULK_COMMIT_INTERVAL {
                self.set_cursor_in_tx(&mut tx, event_cursor, message_cursor)
                    .await?;
                tx.commit().await?;
                tx = self.pool.begin().await?;
                pending_commits = 0;
                if let Some(tx_progress) = &progress_tx {
                    let _ = tx_progress.send((processed, total_count));
                }
            }
        }

        self.set_cursor_in_tx(&mut tx, event_cursor, message_cursor)
            .await?;
        tx.commit().await?;

        if let Some(tx_progress) = &progress_tx {
            let _ = tx_progress.send((processed, total_count));
        }

        Ok(())
    }

    async fn count_searchable_sources(&self) -> anyhow::Result<u64> {
        let event_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM event_log WHERE event_type IN ('content_delta', 'brief_message')",
        )
        .fetch_one(&self.pool)
        .await?;

        let message_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM messages
             WHERE role IN ('user', 'assistant')
               AND content IS NOT NULL
               AND TRIM(content) != ''
               AND TRIM(content) != 'null'",
        )
        .fetch_one(&self.pool)
        .await?;

        Ok((event_count.max(0) + message_count.max(0)) as u64)
    }

    async fn get_meta_u64(&self, key: &str) -> anyhow::Result<u64> {
        let value: Option<String> =
            sqlx::query_scalar("SELECT value FROM search_index_meta WHERE key = ?")
                .bind(key)
                .fetch_optional(&self.pool)
                .await?;

        Ok(value.and_then(|v| v.parse().ok()).unwrap_or(0))
    }

    async fn set_meta_u64(&self, key: &str, value: u64) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO search_index_meta (key, value) VALUES (?, ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(key)
        .bind(value.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn set_cursor_in_tx(
        &self,
        tx: &mut Transaction<'_, sqlx::Sqlite>,
        event_log_id: u64,
        message_id: u64,
    ) -> anyhow::Result<()> {
        Self::set_meta_u64_in_tx(tx, META_LAST_EVENT_LOG_ID, event_log_id).await?;
        Self::set_meta_u64_in_tx(tx, META_LAST_MESSAGE_ID, message_id).await?;
        Ok(())
    }

    async fn set_meta_u64_in_tx(
        tx: &mut Transaction<'_, sqlx::Sqlite>,
        key: &str,
        value: u64,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO search_index_meta (key, value) VALUES (?, ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(key)
        .bind(value.to_string())
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    async fn upsert_row_in_tx(
        tx: &mut Transaction<'_, sqlx::Sqlite>,
        session_id: &str,
        turn_id: &str,
        role: &str,
        content: &str,
        message_id: Option<&str>,
    ) -> anyhow::Result<()> {
        if content.trim().is_empty() {
            return Ok(());
        }

        sqlx::query("DELETE FROM messages_fts WHERE session_id = ? AND turn_id = ? AND role = ?")
            .bind(session_id)
            .bind(turn_id)
            .bind(role)
            .execute(&mut **tx)
            .await?;

        sqlx::query(
            "INSERT INTO messages_fts (content, session_id, turn_id, role, message_id)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(content)
        .bind(session_id)
        .bind(turn_id)
        .bind(role)
        .bind(message_id)
        .execute(&mut **tx)
        .await?;

        Ok(())
    }
}

fn contains_cjk(s: &str) -> bool {
    s.chars().any(|c| {
        matches!(c,
            '\u{4E00}'..='\u{9FFF}' |   // CJK Unified Ideographs
            '\u{3400}'..='\u{4DBF}' |   // CJK Extension A
            '\u{F900}'..='\u{FAFF}' |   // CJK Compatibility Ideographs
            '\u{3000}'..='\u{303F}' |   // CJK Symbols and Punctuation
            '\u{3040}'..='\u{309F}' |   // Hiragana
            '\u{30A0}'..='\u{30FF}' |   // Katakana
            '\u{AC00}'..='\u{D7AF}'     // Hangul Syllables
        )
    })
}

fn generate_snippet(content: &str, query: &str, context_chars: usize) -> String {
    let lower_content = content.to_lowercase();
    let lower_query = query.to_lowercase();

    if let Some(pos) = lower_content.find(&lower_query) {
        let start = content[..pos]
            .char_indices()
            .rev()
            .nth(context_chars)
            .map(|(i, _)| i)
            .unwrap_or(0);
        let end_of_match = pos + query.len();
        let end = content[end_of_match..]
            .char_indices()
            .nth(context_chars)
            .map(|(i, _)| end_of_match + i)
            .unwrap_or(content.len());

        let prefix = if start > 0 { "…" } else { "" };
        let suffix = if end < content.len() { "…" } else { "" };
        let before = &content[start..pos];
        let matched = &content[pos..end_of_match];
        let after = &content[end_of_match..end];

        format!("{prefix}{before}<b>{matched}</b>{after}{suffix}")
    } else {
        let truncated: String = content.chars().take(80).collect();
        if truncated.len() < content.len() {
            format!("{truncated}…")
        } else {
            truncated
        }
    }
}

fn prepare_fts_query(query: &str) -> String {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    if trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2 {
        return trimmed.to_string();
    }

    trimmed
        .split_whitespace()
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" ")
}

fn extract_plain_text(content: Option<&str>) -> Option<String> {
    let raw = content?;
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "null" {
        return None;
    }

    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        return extract_text_from_json_value(&value);
    }

    Some(trimmed.to_string())
}

fn extract_text_from_json_value(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(text) if !text.trim().is_empty() => Some(text.clone()),
        serde_json::Value::Array(parts) => {
            let texts: Vec<String> = parts
                .iter()
                .filter_map(|part| {
                    part.get("text")
                        .and_then(|t| t.as_str())
                        .or_else(|| part.get("content").and_then(|t| t.as_str()))
                        .map(str::to_string)
                })
                .collect();
            if texts.is_empty() {
                None
            } else {
                Some(texts.join("\n"))
            }
        }
        _ => None,
    }
}

/// Returns true when an event_log row may contain searchable message text.
pub fn is_searchable_event_type(event_type: &str) -> bool {
    event_type.contains("message") || event_type.contains("content")
}

/// Parse searchable text from an event_log row for FTS indexing.
pub fn try_index_event(
    event_type: &str,
    event_json: &str,
    delta_accum: &mut HashMap<(String, String), String>,
    session_id: &str,
    turn_id: &str,
) -> Option<(String, String)> {
    if !is_searchable_event_type(event_type) {
        return None;
    }
    extract_searchable_from_event(event_type, event_json, delta_accum, session_id, turn_id)
}

/// Extract plain text from a messages.content JSON column value.
pub fn extract_message_content(content: Option<&str>) -> Option<String> {
    extract_plain_text(content)
}

fn extract_searchable_from_event(
    event_type: &str,
    event_json: &str,
    delta_accum: &mut HashMap<(String, String), String>,
    session_id: &str,
    turn_id: &str,
) -> Option<(String, String)> {
    let value = serde_json::from_str::<serde_json::Value>(event_json).ok()?;

    match event_type {
        "content_delta" => {
            let delta = value.get("delta")?;
            let chunk = delta.get("content").and_then(|c| c.as_str())?;
            if chunk.is_empty() {
                return None;
            }
            let key = (session_id.to_string(), turn_id.to_string());
            let entry = delta_accum.entry(key).or_default();
            entry.push_str(chunk);
            Some(("assistant".to_string(), entry.clone()))
        }
        "brief_message" => {
            let content = value.get("content")?.as_str()?;
            if content.trim().is_empty() {
                return None;
            }
            Some(("assistant".to_string(), content.to_string()))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::str::FromStr;

    async fn test_pool() -> SqlitePool {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .foreign_keys(true);
        SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap()
    }

    async fn seed_schema(pool: &SqlitePool) {
        sqlx::query(
            "CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                title TEXT,
                work_dir TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                message_count INTEGER NOT NULL DEFAULT 0
            )",
        )
        .execute(pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                role TEXT NOT NULL,
                content TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            )",
        )
        .execute(pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE event_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                turn_id TEXT NOT NULL,
                event_type TEXT NOT NULL,
                event_json TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            )",
        )
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn upsert_deduplicates_by_session_turn_role() {
        let pool = test_pool().await;
        seed_schema(&pool).await;
        let index = SearchIndex::new(pool);
        index.ensure_schema().await.unwrap();

        index
            .upsert_row("s1", "t1", "assistant", "hello", None)
            .await
            .unwrap();
        index
            .upsert_row("s1", "t1", "assistant", "hello world", None)
            .await
            .unwrap();

        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM messages_fts WHERE session_id = 's1'")
                .fetch_one(&index.pool)
                .await
                .unwrap();
        assert_eq!(count, 1);

        let content: String =
            sqlx::query_scalar("SELECT content FROM messages_fts WHERE session_id = 's1'")
                .fetch_one(&index.pool)
                .await
                .unwrap();
        assert_eq!(content, "hello world");
    }

    #[tokio::test]
    async fn search_returns_snippet_and_filters() {
        let pool = test_pool().await;
        seed_schema(&pool).await;
        let index = SearchIndex::new(pool.clone());
        index.ensure_schema().await.unwrap();

        sqlx::query(
            "INSERT INTO sessions (id, agent_id, title, work_dir, updated_at)
             VALUES ('s1', 'agent', 'Rust chat', '/proj/rust', '2026-06-01T12:00:00Z')",
        )
        .execute(&pool)
        .await
        .unwrap();

        index
            .upsert_row("s1", "t1", "assistant", "Rust ownership is unique", None)
            .await
            .unwrap();

        let filters = SearchFilters {
            work_dir: Some("/proj/rust".to_string()),
            date_from: Some("2026-06-01T00:00:00Z".to_string()),
            date_to: Some("2026-06-02T00:00:00Z".to_string()),
        };

        let results = index.search("ownership", &filters, 10, 0).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].session_title, "Rust chat");
        assert!(results[0].snippet.contains("ownership"));
    }

    #[tokio::test]
    async fn bulk_index_history_processes_event_log_and_messages() {
        let pool = test_pool().await;
        seed_schema(&pool).await;
        let index = SearchIndex::new(pool.clone());
        index.ensure_schema().await.unwrap();

        sqlx::query("INSERT INTO sessions (id, agent_id, title) VALUES ('s1', 'agent', 'chat')")
            .execute(&pool)
            .await
            .unwrap();

        sqlx::query(
            "INSERT INTO event_log (session_id, turn_id, event_type, event_json)
             VALUES ('s1', 't1', 'content_delta', ?)",
        )
        .bind(r#"{"type":"content_delta","delta":{"content":"hello "}}"#)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO event_log (session_id, turn_id, event_type, event_json)
             VALUES ('s1', 't1', 'content_delta', ?)",
        )
        .bind(r#"{"type":"content_delta","delta":{"content":"world"}}"#)
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query("INSERT INTO messages (session_id, role, content) VALUES ('s1', 'user', ?)")
            .bind(serde_json::to_string(&serde_json::json!("find me")).unwrap())
            .execute(&pool)
            .await
            .unwrap();

        index.bulk_index_history(None).await.unwrap();

        let (event_cursor, message_cursor) = index.get_cursor().await.unwrap();
        assert_eq!(event_cursor, 2);
        assert_eq!(message_cursor, 1);

        let assistant_hits = index
            .search("world", &SearchFilters::default(), 10, 0)
            .await
            .unwrap();
        assert_eq!(assistant_hits.len(), 1);

        let user_hits = index
            .search("find", &SearchFilters::default(), 10, 0)
            .await
            .unwrap();
        assert_eq!(user_hits.len(), 1);
    }

    #[tokio::test]
    async fn search_cjk_uses_like_fallback() {
        let pool = test_pool().await;
        seed_schema(&pool).await;
        let index = SearchIndex::new(pool.clone());
        index.ensure_schema().await.unwrap();

        sqlx::query(
            "INSERT INTO sessions (id, agent_id, title, work_dir, updated_at)
             VALUES ('s1', 'agent', '成本统计会话', '/proj', '2026-06-10T10:00:00Z')",
        )
        .execute(&pool)
        .await
        .unwrap();

        index
            .upsert_row("s1", "t1", "user", "帮我查看成本统计", None)
            .await
            .unwrap();
        index
            .upsert_row("s1", "t2", "assistant", "当前会话的总成本为 0.5 美元", None)
            .await
            .unwrap();

        let results = index
            .search("成本", &SearchFilters::default(), 10, 0)
            .await
            .unwrap();
        assert!(
            results.len() >= 1,
            "should find CJK results via LIKE fallback"
        );
        assert!(results[0].snippet.contains("<b>成本</b>"));

        let results2 = index
            .search("统计", &SearchFilters::default(), 10, 0)
            .await
            .unwrap();
        assert!(results2.len() >= 1);

        let no_results = index
            .search("不存在的内容", &SearchFilters::default(), 10, 0)
            .await
            .unwrap();
        assert_eq!(no_results.len(), 0);
    }
}
