use anyhow::Result;
use sqlx::SqlitePool;
use std::collections::HashMap;

/// Persistent cache of skill embedding vectors, keyed by `(skill_id, content_hash)`.
///
/// When a skill's content changes (different hash), the old embedding is stale and
/// must be recomputed. The store itself does not depend on any embedding provider;
/// the gateway layer computes vectors and passes them in.
pub struct SkillEmbeddingStore {
    pool: SqlitePool,
}

impl SkillEmbeddingStore {
    pub async fn open(pool: SqlitePool) -> Result<Self> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS skill_embeddings (
                skill_id       TEXT PRIMARY KEY,
                content_hash   TEXT NOT NULL,
                embedding      BLOB NOT NULL,
                embedding_norm REAL NOT NULL,
                updated_at     TEXT NOT NULL DEFAULT (datetime('now'))
            );
            "#,
        )
        .execute(&pool)
        .await?;
        Ok(Self { pool })
    }

    /// Upsert an embedding for a skill. Replaces any existing row for that `skill_id`.
    pub async fn upsert(
        &self,
        skill_id: &str,
        content_hash: &str,
        embedding: &[f32],
    ) -> Result<()> {
        let blob = embedding_to_blob(embedding);
        let norm = l2_norm(embedding) as f64;
        sqlx::query(
            r#"
            INSERT INTO skill_embeddings (skill_id, content_hash, embedding, embedding_norm, updated_at)
            VALUES (?, ?, ?, ?, datetime('now'))
            ON CONFLICT(skill_id) DO UPDATE SET
                content_hash = excluded.content_hash,
                embedding    = excluded.embedding,
                embedding_norm = excluded.embedding_norm,
                updated_at   = excluded.updated_at
            "#,
        )
        .bind(skill_id)
        .bind(content_hash)
        .bind(&blob)
        .bind(norm)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Look up cached content hashes for a set of skill IDs.
    /// Returns a map of `skill_id → content_hash` for skills that have cached embeddings.
    pub async fn cached_hashes(&self, skill_ids: &[&str]) -> Result<HashMap<String, String>> {
        if skill_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let placeholders = skill_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT skill_id, content_hash FROM skill_embeddings WHERE skill_id IN ({placeholders})"
        );
        let mut query = sqlx::query_as::<_, (String, String)>(&sql);
        for id in skill_ids {
            query = query.bind(*id);
        }
        let rows = query.fetch_all(&self.pool).await?;
        Ok(rows.into_iter().collect())
    }

    /// Load all embeddings. Returns `(skill_id, embedding_vec)` pairs.
    pub async fn all_embeddings(&self) -> Result<Vec<(String, Vec<f32>)>> {
        let rows: Vec<(String, Vec<u8>)> = sqlx::query_as(
            "SELECT skill_id, embedding FROM skill_embeddings WHERE embedding IS NOT NULL",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .filter_map(|(id, blob)| {
                blob_to_embedding(&blob).map(|emb| (id, emb))
            })
            .collect())
    }

    /// Search by cosine similarity against all stored embeddings.
    /// Returns `(skill_id, similarity_score)` sorted by descending score.
    pub async fn search_by_vector(
        &self,
        query_vec: &[f32],
        limit: usize,
    ) -> Result<Vec<(String, f32)>> {
        let all = self.all_embeddings().await?;
        let mut scored: Vec<(String, f32)> = all
            .into_iter()
            .map(|(id, emb)| {
                let sim = cosine_similarity(query_vec, &emb);
                (id, sim)
            })
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);
        Ok(scored)
    }

    /// Remove embeddings for skills no longer in the registry.
    pub async fn prune(&self, active_skill_ids: &[&str]) -> Result<u64> {
        if active_skill_ids.is_empty() {
            let r = sqlx::query("DELETE FROM skill_embeddings")
                .execute(&self.pool)
                .await?;
            return Ok(r.rows_affected());
        }
        let placeholders = active_skill_ids
            .iter()
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "DELETE FROM skill_embeddings WHERE skill_id NOT IN ({placeholders})"
        );
        let mut query = sqlx::query(&sql);
        for id in active_skill_ids {
            query = query.bind(*id);
        }
        let r = query.execute(&self.pool).await?;
        Ok(r.rows_affected())
    }
}

/// Compute a stable hash of skill content for cache invalidation.
pub fn content_hash(content: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    content.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn embedding_to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn blob_to_embedding(blob: &[u8]) -> Option<Vec<f32>> {
    if !blob.len().is_multiple_of(4) {
        return None;
    }
    Some(
        blob.chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
    )
}

fn l2_norm(v: &[f32]) -> f32 {
    v.iter().map(|x| x * x).sum::<f32>().sqrt()
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom < 1e-10 {
        0.0
    } else {
        dot / denom
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
    async fn upsert_and_search() {
        let pool = test_pool().await;
        let store = SkillEmbeddingStore::open(pool).await.unwrap();

        store
            .upsert("skill-a", "hash1", &[1.0, 0.0, 0.0])
            .await
            .unwrap();
        store
            .upsert("skill-b", "hash2", &[0.0, 1.0, 0.0])
            .await
            .unwrap();

        let results = store
            .search_by_vector(&[0.9, 0.1, 0.0], 10)
            .await
            .unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, "skill-a");
        assert!(results[0].1 > results[1].1);
    }

    #[tokio::test]
    async fn cached_hashes_returns_stored() {
        let pool = test_pool().await;
        let store = SkillEmbeddingStore::open(pool).await.unwrap();
        store
            .upsert("s1", "abc", &[1.0, 0.0])
            .await
            .unwrap();

        let hashes = store.cached_hashes(&["s1", "s2"]).await.unwrap();
        assert_eq!(hashes.get("s1").map(|s| s.as_str()), Some("abc"));
        assert!(hashes.get("s2").is_none());
    }

    #[tokio::test]
    async fn upsert_updates_hash() {
        let pool = test_pool().await;
        let store = SkillEmbeddingStore::open(pool).await.unwrap();
        store.upsert("s1", "old", &[1.0]).await.unwrap();
        store.upsert("s1", "new", &[0.5]).await.unwrap();

        let hashes = store.cached_hashes(&["s1"]).await.unwrap();
        assert_eq!(hashes["s1"], "new");
    }

    #[tokio::test]
    async fn prune_removes_stale() {
        let pool = test_pool().await;
        let store = SkillEmbeddingStore::open(pool).await.unwrap();
        store.upsert("keep", "h1", &[1.0]).await.unwrap();
        store.upsert("remove", "h2", &[0.5]).await.unwrap();

        let removed = store.prune(&["keep"]).await.unwrap();
        assert_eq!(removed, 1);

        let all = store.all_embeddings().await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].0, "keep");
    }

    #[test]
    fn content_hash_deterministic() {
        let h1 = content_hash("hello world");
        let h2 = content_hash("hello world");
        let h3 = content_hash("hello world!");
        assert_eq!(h1, h2);
        assert_ne!(h1, h3);
    }

    #[test]
    fn cosine_similarity_basic() {
        assert!((cosine_similarity(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);
        assert!(cosine_similarity(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
        assert_eq!(cosine_similarity(&[], &[]), 0.0);
    }

    #[test]
    fn embedding_blob_roundtrip() {
        let v = vec![1.0f32, -2.5, 3.14];
        let blob = embedding_to_blob(&v);
        let back = blob_to_embedding(&blob).unwrap();
        assert_eq!(v, back);
    }
}
