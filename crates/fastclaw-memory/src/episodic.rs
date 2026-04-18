use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::embedding::{cosine_similarity, l2_norm, EmbeddingProvider, EmbeddingVec};

fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Tunable cap for how many embedding rows are loaded from SQLite before exact
/// cosine re-ranking. Importance/recency ordering approximates a shard so we do
/// not decode every BLOB on huge tables. See `search_by_vector` doc comment.
const VECTOR_SEARCH_CANDIDATE_MULT: usize = 48;
const VECTOR_SEARCH_CANDIDATE_MIN: usize = 256;
const VECTOR_SEARCH_CANDIDATE_MAX: usize = 4096;

/// Controls episodic forgetting: hard caps, exponential time decay, and a
/// recency "safety zone" where episodes are not subject to capacity eviction.
///
/// # Retention score (capacity phase)
///
/// After removing very low-importance stale rows, if the episode count still
/// exceeds [`ForgetPolicy::max_episodes`], we rank survivors by a **retention
/// score** and delete the lowest first (skipping the protected recent window):
///
/// `retention = importance * decay(age)`
///
/// where `age` is the wall-clock age of the episode in **days**, and with
/// half-life `H = decay_half_life_days` (in days),
///
/// `decay(age) = exp(-ln(2) * age / H)`  (so when `age = H`, decay = 1/2).
///
/// If `H <= 0`, decay is treated as **1** (no time penalty — only importance
/// and the capacity ordering matter among non-protected rows).
#[derive(Debug, Clone)]
pub struct ForgetPolicy {
    /// Maximum number of episodes to keep after `forget` completes (best effort
    /// if almost everything is inside the protected window).
    pub max_episodes: usize,
    /// Half-life in **days** for the exponential decay factor above.
    pub decay_half_life_days: f64,
    /// Episodes with `importance < min_importance` and older than
    /// `protect_recent_hours` are deleted in the first phase.
    pub min_importance: f64,
    /// Episodes created within the last `protect_recent_hours` are never
    /// removed in the capacity phase; they are also exempt from the first-phase
    /// low-importance purge **only when** we interpret protection as "never
    /// delete recent" — here the first phase deletes low importance **only if**
    /// the episode is **older** than this window (same hours threshold).
    pub protect_recent_hours: u64,
}

/// A memorable interaction or event extracted from conversations.
///
/// Episodes capture the *what happened* across sessions — tool calls that
/// succeeded, errors the user corrected, preferences expressed, etc.
/// They form a timeline the agent can search to recall prior context.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Episode {
    pub id: String,
    pub session_id: String,
    pub agent_id: String,
    pub summary: String,
    pub importance: f32,
    #[serde(default)]
    pub tags: String,
    pub created_at: String,
    #[serde(default)]
    pub dreamed_at: Option<String>,
}

pub struct EpisodicMemory {
    pool: SqlitePool,
}

impl EpisodicMemory {
    pub async fn open(pool: SqlitePool) -> Result<Self> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS episodes (
                id          TEXT PRIMARY KEY,
                session_id  TEXT NOT NULL,
                agent_id    TEXT NOT NULL DEFAULT 'main',
                summary     TEXT NOT NULL,
                importance  REAL NOT NULL DEFAULT 0.5,
                tags        TEXT NOT NULL DEFAULT '',
                embedding   BLOB,
                embedding_norm REAL,
                dreamed_at  TEXT,
                created_at  TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX IF NOT EXISTS idx_ep_session ON episodes(session_id);
            CREATE INDEX IF NOT EXISTS idx_ep_agent   ON episodes(agent_id);
            CREATE INDEX IF NOT EXISTS idx_ep_time    ON episodes(created_at);
            CREATE INDEX IF NOT EXISTS idx_ep_imp_time ON episodes(importance, created_at);
            "#,
        )
        .execute(&pool)
        .await?;

        let _ = sqlx::query("ALTER TABLE episodes ADD COLUMN embedding BLOB")
            .execute(&pool)
            .await;
        let _ = sqlx::query("ALTER TABLE episodes ADD COLUMN embedding_norm REAL")
            .execute(&pool)
            .await;
        let _ = sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_ep_imp_time ON episodes(importance, created_at)",
        )
        .execute(&pool)
        .await;
        let _ = sqlx::query("ALTER TABLE episodes ADD COLUMN dreamed_at TEXT")
            .execute(&pool)
            .await;

        Ok(Self { pool })
    }

    pub async fn recent_unprocessed(&self, limit: i64) -> Result<Vec<Episode>> {
        let rows = sqlx::query_as::<_, Episode>(
            "SELECT id, session_id, agent_id, summary, importance, tags, created_at, dreamed_at \
             FROM episodes WHERE dreamed_at IS NULL \
             ORDER BY created_at ASC LIMIT ?",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn mark_episodes_dreamed(&self, ids: &[String]) -> Result<usize> {
        let now = Utc::now().to_rfc3339();
        let mut n = 0usize;
        for id in ids {
            let r = sqlx::query("UPDATE episodes SET dreamed_at = ? WHERE id = ?")
                .bind(&now)
                .bind(id)
                .execute(&self.pool)
                .await?;
            n += r.rows_affected() as usize;
        }
        Ok(n)
    }

    pub async fn record(&self, episode: &Episode) -> Result<()> {
        sqlx::query(
            "INSERT INTO episodes (id, session_id, agent_id, summary, importance, tags, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&episode.id)
        .bind(&episode.session_id)
        .bind(&episode.agent_id)
        .bind(&episode.summary)
        .bind(episode.importance)
        .bind(&episode.tags)
        .bind(&episode.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Record an episode with its embedding vector.
    pub async fn record_with_embedding(
        &self,
        episode: &Episode,
        embedding: &EmbeddingVec,
    ) -> Result<()> {
        let blob = embedding_to_blob(embedding);
        let norm = l2_norm(embedding) as f64;
        sqlx::query(
            "INSERT INTO episodes (id, session_id, agent_id, summary, importance, tags, embedding, embedding_norm, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&episode.id)
        .bind(&episode.session_id)
        .bind(&episode.agent_id)
        .bind(&episode.summary)
        .bind(episode.importance)
        .bind(&episode.tags)
        .bind(&blob)
        .bind(norm)
        .bind(&episode.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Record an episode, automatically computing its embedding if a provider is given.
    pub async fn record_auto(
        &self,
        episode: &Episode,
        embedder: Option<&dyn EmbeddingProvider>,
    ) -> Result<()> {
        match embedder {
            Some(ep) => {
                let vec = ep.embed(&episode.summary).await?;
                self.record_with_embedding(episode, &vec).await
            }
            None => self.record(episode).await,
        }
    }

    /// Retrieve recent episodes, optionally filtered by agent.
    pub async fn recent(&self, agent_id: Option<&str>, limit: i64) -> Result<Vec<Episode>> {
        let rows = match agent_id {
            Some(aid) => {
                sqlx::query_as::<_, Episode>(
                    "SELECT id, session_id, agent_id, summary, importance, tags, created_at, dreamed_at \
                     FROM episodes WHERE agent_id = ? ORDER BY created_at DESC LIMIT ?",
                )
                .bind(aid)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
            }
            None => {
                sqlx::query_as::<_, Episode>(
                    "SELECT id, session_id, agent_id, summary, importance, tags, created_at, dreamed_at \
                     FROM episodes ORDER BY created_at DESC LIMIT ?",
                )
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
            }
        };
        Ok(rows)
    }

    /// Search episodes whose summary contains the keyword (case-insensitive).
    pub async fn search(&self, keyword: &str, limit: i64) -> Result<Vec<Episode>> {
        let escaped = escape_like(keyword);
        let pattern = format!("%{escaped}%");
        let rows = sqlx::query_as::<_, Episode>(
            "SELECT id, session_id, agent_id, summary, importance, tags, created_at, dreamed_at \
             FROM episodes WHERE summary LIKE ? ESCAPE '\\' ORDER BY importance DESC, created_at DESC LIMIT ?",
        )
        .bind(&pattern)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Search by tag. Tags are stored as comma-separated in the `tags` column.
    pub async fn search_by_tag(&self, tag: &str, limit: i64) -> Result<Vec<Episode>> {
        let escaped = escape_like(tag);
        let pattern = format!("%{escaped}%");
        let rows = sqlx::query_as::<_, Episode>(
            "SELECT id, session_id, agent_id, summary, importance, tags, created_at, dreamed_at \
             FROM episodes WHERE tags LIKE ? ESCAPE '\\' ORDER BY created_at DESC LIMIT ?",
        )
        .bind(&pattern)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Delete episodes older than `before`.
    pub async fn cleanup_before(&self, before: &str) -> Result<u64> {
        let r = sqlx::query("DELETE FROM episodes WHERE created_at < ?")
            .bind(before)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected())
    }

    /// Search by vector similarity across episode summaries.
    ///
    /// **Performance:** Instead of loading every row with a BLOB embedding,
    /// SQLite returns a **candidate shard** ordered by `importance DESC`,
    /// `created_at DESC` (see `idx_ep_imp_time`). Only those rows are decoded
    /// and cosine-scored. This is a pragmatic trade-off when ANN is unavailable:
    /// true global nearest neighbours might live outside the shard, but the
    /// most *agent-relevant* memories are usually among higher-importance /
    /// newer episodes.
    ///
    /// Rows also store `embedding_norm` (L2 at insert time) for diagnostics and
    /// cheap rejection of empty / corrupt vectors (`norm < 1e-9`).
    pub async fn search_by_vector(
        &self,
        query_vec: &EmbeddingVec,
        limit: usize,
    ) -> Result<Vec<(Episode, f32)>> {
        let qn = l2_norm(query_vec) as f64;
        let candidate_cap = (limit.saturating_mul(VECTOR_SEARCH_CANDIDATE_MULT))
            .max(VECTOR_SEARCH_CANDIDATE_MIN)
            .min(VECTOR_SEARCH_CANDIDATE_MAX) as i64;

        let half_cap = candidate_cap / 2;
        let rows: Vec<EpisodeWithBlob> = sqlx::query_as(
            "SELECT id, session_id, agent_id, summary, importance, tags, embedding, embedding_norm, created_at, dreamed_at \
             FROM episodes WHERE embedding IS NOT NULL \
             ORDER BY importance DESC, created_at DESC \
             LIMIT ?",
        )
        .bind(half_cap)
        .fetch_all(&self.pool)
        .await?;

        let recency_rows: Vec<EpisodeWithBlob> = sqlx::query_as(
            "SELECT id, session_id, agent_id, summary, importance, tags, embedding, embedding_norm, created_at, dreamed_at \
             FROM episodes WHERE embedding IS NOT NULL \
             ORDER BY created_at DESC \
             LIMIT ?",
        )
        .bind(candidate_cap - half_cap)
        .fetch_all(&self.pool)
        .await?;

        let mut seen_ids: std::collections::HashSet<String> = rows.iter().map(|r| r.id.clone()).collect();
        let mut rows = rows;
        for r in recency_rows {
            if seen_ids.insert(r.id.clone()) {
                rows.push(r);
            }
        }

        let mut scored: Vec<(Episode, f32)> = rows
            .into_iter()
            .filter_map(|mut r| {
                let blob = r.embedding.take()?;
                let emb = blob_to_embedding(&blob)?;
                let row_norm = r.embedding_norm.unwrap_or_else(|| l2_norm(&emb) as f64);
                if row_norm < 1e-9 || qn < 1e-9 as f64 {
                    return None;
                }
                // Optional Cauchy–Schwarz upper bound on |dot| for pruning:
                // |q·d| <= ||q|| ||d|| ⇒ cos <= 1 always — no safe lower prune.
                // We still skip when stored norm disagrees badly with recomputed
                // norm (corrupt BLOB) to avoid garbage scores.
                let recomputed = l2_norm(&emb) as f64;
                if (recomputed - row_norm).abs() > (row_norm * 0.5 + 1e-6) {
                    return None;
                }
                let sim = cosine_similarity(query_vec, &emb);
                Some((r.into_episode(), sim))
            })
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);
        Ok(scored)
    }

    /// Apply [`ForgetPolicy`] and return how many episodes were deleted.
    ///
    /// Phases:
    /// 1. **Low-importance stale purge:** delete rows with
    ///    `importance < min_importance` **and** age ≥ `protect_recent_hours`.
    /// 2. **Capacity eviction:** while count > `max_episodes`, among rows **not**
    ///    in the protected recent window, delete those with the lowest
    ///    `retention` score (see struct-level formula).
    pub async fn forget(&self, policy: &ForgetPolicy) -> Result<usize> {
        let now = Utc::now();
        let protect = chrono::Duration::hours(policy.protect_recent_hours as i64);
        let cutoff = (now - protect).to_rfc3339();

        let mut removed = 0usize;

        // Phase 1 — low importance, outside protected window (push filter to SQL)
        let phase1_ids: Vec<(String,)> = sqlx::query_as(
            "SELECT id FROM episodes WHERE importance < ? AND created_at < ? LIMIT 10000",
        )
        .bind(policy.min_importance as f32)
        .bind(&cutoff)
        .fetch_all(&self.pool)
        .await?;
        let phase1_ids: Vec<String> = phase1_ids.into_iter().map(|(id,)| id).collect();

        for chunk in phase1_ids.chunks(500) {
            if chunk.is_empty() {
                continue;
            }
            let mut qb = String::from("DELETE FROM episodes WHERE id IN (");
            for (i, _) in chunk.iter().enumerate() {
                if i > 0 {
                    qb.push(',');
                }
                qb.push('?');
            }
            qb.push(')');
            let mut q = sqlx::query(&qb);
            for id in chunk {
                q = q.bind(id);
            }
            let r = q.execute(&self.pool).await?;
            removed += r.rows_affected() as usize;
        }

        // Phase 2 — capacity by retention score
        loop {
            let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM episodes")
                .fetch_one(&self.pool)
                .await?;
            if count as usize <= policy.max_episodes {
                break;
            }

            let survivors: Vec<(String, f32, String)> =
                sqlx::query_as("SELECT id, importance, created_at FROM episodes")
                    .fetch_all(&self.pool)
                    .await?;

            let need = (count as usize) - policy.max_episodes;
            let mut scored: Vec<(String, f64)> = Vec::new();
            for (id, imp, created_at) in survivors {
                let Some(ts) = parse_sqlite_datetime(&created_at) else {
                    continue;
                };
                let age = now.signed_duration_since(ts);
                if age < protect {
                    continue;
                }
                let age_days = age.num_seconds().max(0) as f64 / 86_400.0;
                let decay = if policy.decay_half_life_days > 1e-9 {
                    (-std::f64::consts::LN_2 * age_days / policy.decay_half_life_days).exp()
                } else {
                    1.0
                };
                let retention = (imp as f64) * decay;
                scored.push((id, retention));
            }

            if scored.is_empty() {
                // Only protected-by-age rows remain; cannot shrink further.
                break;
            }

            scored.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
            let take = need.min(scored.len());
            let victims: Vec<String> = scored.into_iter().take(take).map(|(id, _)| id).collect();

            for chunk in victims.chunks(500) {
                if chunk.is_empty() {
                    continue;
                }
                let mut qb = String::from("DELETE FROM episodes WHERE id IN (");
                for (i, _) in chunk.iter().enumerate() {
                    if i > 0 {
                        qb.push(',');
                    }
                    qb.push('?');
                }
                qb.push(')');
                let mut q = sqlx::query(&qb);
                for id in chunk {
                    q = q.bind(id);
                }
                let r = q.execute(&self.pool).await?;
                removed += r.rows_affected() as usize;
            }
        }

        Ok(removed)
    }

    /// Hybrid search: keyword LIKE + vector similarity.
    pub async fn hybrid_search(
        &self,
        keyword: &str,
        query_vec: Option<&EmbeddingVec>,
        alpha: f32,
        limit: usize,
    ) -> Result<Vec<(Episode, f32)>> {
        use std::collections::HashMap;

        let alpha = alpha.clamp(0.0, 1.0);
        let mut scores: HashMap<String, (Episode, f32)> = HashMap::new();

        if !keyword.is_empty() {
            let kw_results = self.search(keyword, limit as i64 * 2).await?;
            let n = kw_results.len().max(1) as f32;
            for (rank, ep) in kw_results.into_iter().enumerate() {
                let kw_score = 1.0 - (rank as f32 / n);
                let weighted = (1.0 - alpha) * kw_score;
                scores
                    .entry(ep.id.clone())
                    .and_modify(|(_, s)| *s += weighted)
                    .or_insert((ep, weighted));
            }
        }

        if let Some(qv) = query_vec {
            let vec_results = self.search_by_vector(qv, limit * 2).await?;
            for (ep, sim) in vec_results {
                let weighted = alpha * sim;
                scores
                    .entry(ep.id.clone())
                    .and_modify(|(_, s)| *s += weighted)
                    .or_insert((ep, weighted));
            }
        }

        let mut ranked: Vec<(Episode, f32)> = scores.into_values().collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        ranked.truncate(limit);
        Ok(ranked)
    }

    /// Build a text recap of the N most important recent episodes,
    /// suitable for injecting into the system prompt.
    pub async fn build_recap(&self, agent_id: Option<&str>, limit: i64) -> Result<String> {
        let eps = self.recent(agent_id, limit).await?;
        if eps.is_empty() {
            return Ok(String::new());
        }
        let mut out = String::from("[episodic memory]\n");
        for ep in &eps {
            out.push_str(&format!(
                "- [{}] (importance={:.1}) {}\n",
                &ep.created_at[..10.min(ep.created_at.len())],
                ep.importance,
                ep.summary
            ));
        }
        Ok(out)
    }
}

// ---- helpers ----

/// Parse `created_at` written by SQLite `datetime('now')` or RFC3339 from tests.
fn parse_sqlite_datetime(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
                .ok()
                .map(|n| n.and_utc())
        })
}

fn embedding_to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn blob_to_embedding(blob: &[u8]) -> Option<EmbeddingVec> {
    if blob.len() % 4 != 0 {
        return None;
    }
    Some(
        blob.chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
    )
}

#[derive(sqlx::FromRow)]
struct EpisodeWithBlob {
    id: String,
    session_id: String,
    agent_id: String,
    summary: String,
    importance: f32,
    tags: String,
    embedding: Option<Vec<u8>>,
    embedding_norm: Option<f64>,
    created_at: String,
    dreamed_at: Option<String>,
}

impl EpisodeWithBlob {
    fn into_episode(self) -> Episode {
        Episode {
            id: self.id,
            session_id: self.session_id,
            agent_id: self.agent_id,
            summary: self.summary,
            importance: self.importance,
            tags: self.tags,
            created_at: self.created_at,
            dreamed_at: self.dreamed_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn mem() -> EpisodicMemory {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        EpisodicMemory::open(pool).await.unwrap()
    }

    fn ep(id: &str, summary: &str, importance: f32) -> Episode {
        Episode {
            id: id.to_string(),
            session_id: "s1".into(),
            agent_id: "main".into(),
            summary: summary.into(),
            importance,
            tags: "test".into(),
            created_at: "2026-04-16T10:00:00Z".into(),
            dreamed_at: None,
        }
    }

    #[tokio::test]
    async fn record_and_recent() {
        let m = mem().await;
        m.record(&ep("e1", "user prefers Rust", 0.9)).await.unwrap();
        m.record(&ep("e2", "fixed login bug", 0.6)).await.unwrap();

        let all = m.recent(None, 10).await.unwrap();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn keyword_search() {
        let m = mem().await;
        m.record(&ep("e1", "user prefers Rust over Python", 0.8))
            .await
            .unwrap();
        m.record(&ep("e2", "deployed to production", 0.5))
            .await
            .unwrap();

        let hits = m.search("Rust", 10).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].summary.contains("Rust"));
    }

    #[tokio::test]
    async fn build_recap_format() {
        let m = mem().await;
        m.record(&ep("e1", "user likes concise replies", 0.7))
            .await
            .unwrap();

        let recap = m.build_recap(Some("main"), 5).await.unwrap();
        assert!(recap.contains("[episodic memory]"));
        assert!(recap.contains("concise replies"));
    }

    #[tokio::test]
    async fn record_with_embedding_and_vector_search() {
        let m = mem().await;
        let e1 = ep("e1", "discussed Rust performance", 0.8);
        let emb1 = vec![1.0, 0.0, 0.0];
        m.record_with_embedding(&e1, &emb1).await.unwrap();

        let e2 = ep("e2", "fixed login bug", 0.5);
        let emb2 = vec![0.0, 1.0, 0.0];
        m.record_with_embedding(&e2, &emb2).await.unwrap();

        let query = vec![0.9, 0.1, 0.0];
        let results = m.search_by_vector(&query, 10).await.unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0.id, "e1");
        assert!(results[0].1 > results[1].1);
    }

    #[tokio::test]
    async fn hybrid_search_episodes() {
        let m = mem().await;
        let e1 = ep("e1", "deployed database migration", 0.7);
        let emb1 = vec![1.0, 0.0, 0.0];
        m.record_with_embedding(&e1, &emb1).await.unwrap();

        let e2 = ep("e2", "reviewed database schema", 0.6);
        let emb2 = vec![0.0, 1.0, 0.0];
        m.record_with_embedding(&e2, &emb2).await.unwrap();

        let e3 = ep("e3", "built login page", 0.4);
        let emb3 = vec![0.0, 0.0, 1.0];
        m.record_with_embedding(&e3, &emb3).await.unwrap();

        let query = vec![0.9, 0.1, 0.0];
        let results = m
            .hybrid_search("database", Some(&query), 0.5, 10)
            .await
            .unwrap();
        assert!(results.len() >= 2);
        assert_eq!(results[0].0.id, "e1");
    }

    #[tokio::test]
    async fn forget_purges_stale_low_importance() {
        let m = mem().await;
        let old = "2020-01-01T00:00:00Z";
        let mut low = ep("old_low", "stale low", 0.02);
        low.created_at = old.into();
        m.record(&low).await.unwrap();
        let mut high = ep("old_high", "keep", 0.9);
        high.created_at = old.into();
        m.record(&high).await.unwrap();

        let policy = ForgetPolicy {
            max_episodes: 100,
            decay_half_life_days: 365.0,
            min_importance: 0.05,
            protect_recent_hours: 1,
        };
        let n = m.forget(&policy).await.unwrap();
        assert_eq!(n, 1);
        let rest = m.recent(None, 10).await.unwrap();
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].id, "old_high");
    }

    #[tokio::test]
    async fn forget_capacity_eviction_by_retention() {
        let m = mem().await;
        let t = "2020-06-01T00:00:00Z";
        for (id, imp) in [("a", 0.9f32), ("b", 0.5f32), ("c", 0.2f32)] {
            let mut e = ep(id, "x", imp);
            e.created_at = t.into();
            m.record(&e).await.unwrap();
        }
        let policy = ForgetPolicy {
            max_episodes: 2,
            decay_half_life_days: 10_000.0,
            min_importance: 0.01,
            protect_recent_hours: 0,
        };
        let n = m.forget(&policy).await.unwrap();
        assert_eq!(n, 1);
        let rest = m.recent(None, 10).await.unwrap();
        assert_eq!(rest.len(), 2);
        assert!(rest.iter().all(|e| e.id != "c"));
    }

    #[tokio::test]
    async fn vector_search_importance_shard_excludes_low_rank_rows() {
        let m = mem().await;
        for i in 0..260u32 {
            let mut e = ep(&format!("f{i}"), "noise episode", 0.95);
            e.created_at = format!("2026-04-16T12:{:02}:00Z", i % 60);
            m.record_with_embedding(&e, &vec![0.0, 1.0, 0.0])
                .await
                .unwrap();
        }
        let mut gold = ep("golden", "true nearest neighbour", 0.05);
        gold.created_at = "2026-04-16T11:00:00Z".into();
        m.record_with_embedding(&gold, &vec![1.0, 0.0, 0.0])
            .await
            .unwrap();

        let q = vec![1.0f32, 0.0, 0.0];
        let hits = m.search_by_vector(&q, 1).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_ne!(hits[0].0.id, "golden");
        assert!(
            hits[0].1.abs() < 1e-5,
            "expected orthogonal noise, got {}",
            hits[0].1
        );
    }
}
