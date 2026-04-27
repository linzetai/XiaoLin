use anyhow::Result;
use petgraph::graph::{DiGraph, EdgeIndex, NodeIndex};
use petgraph::visit::EdgeRef;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;

use crate::embedding::{cosine_similarity, l2_norm, EmbeddingProvider, EmbeddingVec};

fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Same idea as episodic search: cap BLOB decodes before exact cosine.
const VECTOR_SEARCH_CANDIDATE_MULT: usize = 48;
const VECTOR_SEARCH_CANDIDATE_MIN: usize = 256;
const VECTOR_SEARCH_CANDIDATE_MAX: usize = 4096;

pub const DEFAULT_FIND_PATH_MAX_DEPTH: usize = 5;

/// Category of a semantic fact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FactCategory {
    UserPreference,
    UserFact,
    DomainKnowledge,
    Correction,
    Custom(String),
}

impl FactCategory {
    pub fn as_str(&self) -> &str {
        match self {
            Self::UserPreference => "user_preference",
            Self::UserFact => "user_fact",
            Self::DomainKnowledge => "domain_knowledge",
            Self::Correction => "correction",
            Self::Custom(s) => s.as_str(),
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "user_preference" => Self::UserPreference,
            "user_fact" => Self::UserFact,
            "domain_knowledge" => Self::DomainKnowledge,
            "correction" => Self::Correction,
            other => Self::Custom(other.to_string()),
        }
    }
}

/// A persistent fact / piece of knowledge.
///
/// Facts are the agent's long-term declarative memory — user preferences
/// ("prefers dark mode"), corrections ("their name is Lin, not Ling"),
/// domain knowledge ("the prod database is on port 5433"), etc.
///
/// Facts form a flat key-value store with categories, searchable by
/// keyword or category. Graph edges between named entities live in the
/// `relationships` table (see [`SemanticMemory::add_relationship`]).
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Fact {
    pub id: String,
    pub category: String,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub confidence: f32,
    pub source_session: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// A directed typed edge between two entity names (minimal graph MVP).
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, PartialEq)]
pub struct Relationship {
    pub id: String,
    pub source_entity: String,
    pub relation: String,
    pub target_entity: String,
    pub confidence: f32,
    pub metadata: Option<String>,
    pub created_at: String,
}

/// In-memory directed multi-edge index over `relationships`, kept in sync with SQLite.
#[derive(Debug, Default)]
struct RelationshipGraph {
    graph: DiGraph<String, Relationship>,
    entity_to_node: HashMap<String, NodeIndex>,
}

impl RelationshipGraph {
    fn node_or_insert(&mut self, name: &str) -> NodeIndex {
        if let Some(&ix) = self.entity_to_node.get(name) {
            ix
        } else {
            let ix = self.graph.add_node(name.to_string());
            self.entity_to_node.insert(name.to_string(), ix);
            ix
        }
    }

    /// Remove directed edges matching the full triple (parallel edges unlikely; handle all).
    fn remove_triple_edges(&mut self, source: &str, relation: &str, target: &str) {
        let (Some(&s), Some(&t)) = (
            self.entity_to_node.get(source),
            self.entity_to_node.get(target),
        ) else {
            return;
        };
        let mut to_remove: Vec<EdgeIndex> = self
            .graph
            .edges_directed(s, petgraph::Direction::Outgoing)
            .filter(|e| e.target() == t && e.weight().relation == relation)
            .map(|e| e.id())
            .collect();
        // Remove higher indices first so swaps in `remove_edge` do not invalidate remaining ids.
        to_remove.sort_by_key(|ix| std::cmp::Reverse(ix.index()));
        for eid in to_remove {
            let _ = self.graph.remove_edge(eid);
        }
    }

    fn upsert_relationship(&mut self, r: Relationship) {
        self.remove_triple_edges(&r.source_entity, &r.relation, &r.target_entity);
        let s = self.node_or_insert(&r.source_entity);
        let t = self.node_or_insert(&r.target_entity);
        self.graph.add_edge(s, t, r);
    }

    fn neighbors_undirected(&self, node: NodeIndex) -> Vec<(NodeIndex, Relationship)> {
        let mut out = Vec::new();
        for e in self
            .graph
            .edges_directed(node, petgraph::Direction::Outgoing)
        {
            out.push((e.target(), e.weight().clone()));
        }
        for e in self
            .graph
            .edges_directed(node, petgraph::Direction::Incoming)
        {
            out.push((e.source(), e.weight().clone()));
        }
        out
    }

    /// Undirected BFS by hop count; returns oriented edges along the path (same as former SQL BFS).
    fn find_path(
        &self,
        from_entity: &str,
        to_entity: &str,
        max_depth: usize,
    ) -> Option<Vec<Relationship>> {
        if from_entity == to_entity {
            return Some(Vec::new());
        }
        let (&start, &goal) = (
            self.entity_to_node.get(from_entity)?,
            self.entity_to_node.get(to_entity)?,
        );

        let mut q = VecDeque::new();
        let mut visited: HashSet<NodeIndex> = HashSet::new();
        let mut parent: HashMap<NodeIndex, (NodeIndex, Relationship)> = HashMap::new();

        q.push_back((start, 0usize));
        visited.insert(start);

        while let Some((node, plen)) = q.pop_front() {
            if plen >= max_depth {
                continue;
            }
            for (nb, rel) in self.neighbors_undirected(node) {
                if visited.contains(&nb) {
                    continue;
                }
                visited.insert(nb);
                parent.insert(nb, (node, rel.clone()));
                if nb == goal {
                    let mut edges_rev = Vec::new();
                    let mut cur = nb;
                    while cur != start {
                        let (p, ed) = parent.get(&cur)?;
                        edges_rev.push(ed.clone());
                        cur = *p;
                    }
                    edges_rev.reverse();
                    return Some(edges_rev);
                }
                q.push_back((nb, plen + 1));
            }
        }
        None
    }

    fn get_related_entities_limited(
        &self,
        entity: &str,
        depth: usize,
        limit: Option<usize>,
    ) -> Vec<String> {
        if depth == 0 {
            return Vec::new();
        }
        let Some(&start) = self.entity_to_node.get(entity) else {
            return Vec::new();
        };

        if depth == 1 {
            let mut out = Vec::new();
            let mut seen: HashSet<String> = HashSet::new();
            seen.insert(entity.to_string());
            for (nb, _) in self.neighbors_undirected(start) {
                let name = self.graph[nb].clone();
                if seen.insert(name.clone()) {
                    out.push(name);
                }
                if limit.is_some_and(|lim| out.len() >= lim) {
                    break;
                }
            }
            return out;
        }

        let mut out: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        seen.insert(entity.to_string());
        let mut frontier = vec![start];

        for _ in 0..depth {
            if frontier.is_empty() {
                break;
            }
            let mut next_frontier: Vec<NodeIndex> = Vec::new();

            for &u in &frontier {
                for (nb, _) in self.neighbors_undirected(u) {
                    let nb_name = self.graph[nb].clone();
                    if seen.insert(nb_name.clone()) {
                        out.push(nb_name.clone());
                        next_frontier.push(nb);
                        if limit.is_some_and(|lim| out.len() >= lim) {
                            return out;
                        }
                    }
                }
            }
            frontier = next_frontier;
        }
        out
    }
}

pub struct SemanticMemory {
    pool: SqlitePool,
    /// Incremental relationship index; rebuilt from SQL in [`SemanticMemory::open`].
    relationship_graph: Mutex<RelationshipGraph>,
}

impl SemanticMemory {
    pub async fn open(pool: SqlitePool) -> Result<Self> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS facts (
                id              TEXT PRIMARY KEY,
                category        TEXT NOT NULL DEFAULT 'domain_knowledge',
                subject         TEXT NOT NULL,
                predicate       TEXT NOT NULL,
                object          TEXT NOT NULL,
                confidence      REAL NOT NULL DEFAULT 1.0,
                source_session  TEXT,
                embedding       BLOB,
                embedding_norm  REAL,
                created_at      TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at      TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX IF NOT EXISTS idx_fact_cat     ON facts(category);
            CREATE INDEX IF NOT EXISTS idx_fact_subject ON facts(subject);
            CREATE INDEX IF NOT EXISTS idx_fact_conf_upd ON facts(confidence, updated_at);
            CREATE TABLE IF NOT EXISTS relationships (
                id              TEXT PRIMARY KEY,
                source_entity   TEXT NOT NULL,
                relation        TEXT NOT NULL,
                target_entity   TEXT NOT NULL,
                confidence      REAL NOT NULL DEFAULT 1.0,
                metadata        TEXT,
                created_at      TEXT NOT NULL DEFAULT (datetime('now')),
                UNIQUE(source_entity, relation, target_entity)
            );
            CREATE INDEX IF NOT EXISTS idx_rel_src ON relationships(source_entity);
            CREATE INDEX IF NOT EXISTS idx_rel_tgt ON relationships(target_entity);
            "#,
        )
        .execute(&pool)
        .await?;

        // Migration: add embedding column if table existed without it
        let _ = sqlx::query("ALTER TABLE facts ADD COLUMN embedding BLOB")
            .execute(&pool)
            .await;
        let _ = sqlx::query("ALTER TABLE facts ADD COLUMN embedding_norm REAL")
            .execute(&pool)
            .await;
        let _ = sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_fact_conf_upd ON facts(confidence, updated_at)",
        )
        .execute(&pool)
        .await;

        let relationship_graph = Mutex::new(Self::load_relationship_graph_from_db(&pool).await?);
        Ok(Self {
            pool,
            relationship_graph,
        })
    }

    async fn load_relationship_graph_from_db(pool: &SqlitePool) -> Result<RelationshipGraph> {
        let rows: Vec<Relationship> = sqlx::query_as::<_, Relationship>(
            "SELECT id, source_entity, relation, target_entity, confidence, metadata, created_at \
             FROM relationships",
        )
        .fetch_all(pool)
        .await?;
        let mut g = RelationshipGraph::default();
        for r in rows {
            g.upsert_relationship(r);
        }
        Ok(g)
    }

    /// Upsert a fact. If a fact with the same id already exists, update it.
    pub async fn upsert(&self, fact: &Fact) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO facts (id, category, subject, predicate, object, confidence, source_session, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                category = excluded.category,
                subject = excluded.subject,
                predicate = excluded.predicate,
                object = excluded.object,
                confidence = excluded.confidence,
                source_session = excluded.source_session,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(&fact.id)
        .bind(&fact.category)
        .bind(&fact.subject)
        .bind(&fact.predicate)
        .bind(&fact.object)
        .bind(fact.confidence)
        .bind(&fact.source_session)
        .bind(&fact.created_at)
        .bind(&fact.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Upsert a fact together with its embedding vector.
    pub async fn upsert_with_embedding(&self, fact: &Fact, embedding: &EmbeddingVec) -> Result<()> {
        let blob = embedding_to_blob(embedding);
        let norm = l2_norm(embedding) as f64;
        sqlx::query(
            r#"
            INSERT INTO facts (id, category, subject, predicate, object, confidence, source_session, embedding, embedding_norm, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                category = excluded.category,
                subject = excluded.subject,
                predicate = excluded.predicate,
                object = excluded.object,
                confidence = excluded.confidence,
                source_session = excluded.source_session,
                embedding = excluded.embedding,
                embedding_norm = excluded.embedding_norm,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(&fact.id)
        .bind(&fact.category)
        .bind(&fact.subject)
        .bind(&fact.predicate)
        .bind(&fact.object)
        .bind(fact.confidence)
        .bind(&fact.source_session)
        .bind(&blob)
        .bind(norm)
        .bind(&fact.created_at)
        .bind(&fact.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Upsert a fact, automatically computing its embedding if a provider is given.
    pub async fn upsert_auto(
        &self,
        fact: &Fact,
        embedder: Option<&dyn EmbeddingProvider>,
    ) -> Result<()> {
        match embedder {
            Some(ep) => {
                let text = fact_text(fact);
                let vec = ep.embed(&text).await?;
                self.upsert_with_embedding(fact, &vec).await
            }
            None => self.upsert(fact).await,
        }
    }

    /// Retrieve all facts of a given category.
    pub async fn by_category(&self, category: &FactCategory, limit: i64) -> Result<Vec<Fact>> {
        let rows = sqlx::query_as::<_, Fact>(
            "SELECT id, category, subject, predicate, object, confidence, source_session, created_at, updated_at \
             FROM facts WHERE category = ? ORDER BY confidence DESC, updated_at DESC LIMIT ?",
        )
        .bind(category.as_str())
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Find facts about a specific subject.
    pub async fn about(&self, subject: &str, limit: i64) -> Result<Vec<Fact>> {
        let escaped = escape_like(subject);
        let pattern = format!("%{escaped}%");
        let rows = sqlx::query_as::<_, Fact>(
            "SELECT id, category, subject, predicate, object, confidence, source_session, created_at, updated_at \
             FROM facts WHERE subject LIKE ? ESCAPE '\\' ORDER BY confidence DESC LIMIT ?",
        )
        .bind(&pattern)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Full-text search across subject, predicate, and object.
    pub async fn search(&self, keyword: &str, limit: i64) -> Result<Vec<Fact>> {
        let escaped = escape_like(keyword);
        let pattern = format!("%{escaped}%");
        let rows = sqlx::query_as::<_, Fact>(
            r#"SELECT id, category, subject, predicate, object, confidence, source_session, created_at, updated_at
               FROM facts
               WHERE subject LIKE ?1 ESCAPE '\' OR predicate LIKE ?1 ESCAPE '\' OR object LIKE ?1 ESCAPE '\'
               ORDER BY confidence DESC, updated_at DESC
               LIMIT ?2"#,
        )
        .bind(&pattern)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Delete a fact by id.
    pub async fn delete(&self, id: &str) -> Result<bool> {
        let r = sqlx::query("DELETE FROM facts WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }

    /// Build a text block of user preferences for prompt injection.
    pub async fn build_user_context(&self, limit: i64) -> Result<String> {
        let prefs = self
            .by_category(&FactCategory::UserPreference, limit)
            .await?;
        let corrections = self.by_category(&FactCategory::Correction, limit).await?;

        if prefs.is_empty() && corrections.is_empty() {
            return Ok(String::new());
        }

        let mut out = String::from("[semantic memory]\n");
        if !prefs.is_empty() {
            out.push_str("Preferences:\n");
            for f in &prefs {
                out.push_str(&format!("- {} {} {}\n", f.subject, f.predicate, f.object));
            }
        }
        if !corrections.is_empty() {
            out.push_str("Corrections:\n");
            for f in &corrections {
                out.push_str(&format!("- {} {} {}\n", f.subject, f.predicate, f.object));
            }
        }
        Ok(out)
    }

    /// Get all facts (paginated).
    pub async fn list(&self, offset: i64, limit: i64) -> Result<Vec<Fact>> {
        let rows = sqlx::query_as::<_, Fact>(
            "SELECT id, category, subject, predicate, object, confidence, source_session, created_at, updated_at \
             FROM facts ORDER BY updated_at DESC LIMIT ? OFFSET ?",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Search by vector similarity. Returns (fact, score) sorted by descending
    /// similarity.
    ///
    /// **Performance:** Like [`EpisodicMemory::search_by_vector`], only a
    /// candidate shard ordered by `confidence DESC`, `updated_at DESC` is
    /// loaded and decoded (`idx_fact_conf_upd`). Global recall is traded for
    /// throughput on large tables.
    pub async fn search_by_vector(
        &self,
        query_vec: &EmbeddingVec,
        limit: usize,
    ) -> Result<Vec<(Fact, f32)>> {
        let qn = l2_norm(query_vec) as f64;
        let candidate_cap = (limit.saturating_mul(VECTOR_SEARCH_CANDIDATE_MULT))
            .clamp(VECTOR_SEARCH_CANDIDATE_MIN, VECTOR_SEARCH_CANDIDATE_MAX)
            as i64;

        let rows: Vec<FactWithBlob> = sqlx::query_as(
            "SELECT id, category, subject, predicate, object, confidence, source_session, embedding, embedding_norm, created_at, updated_at \
             FROM facts WHERE embedding IS NOT NULL \
             ORDER BY confidence DESC, updated_at DESC \
             LIMIT ?",
        )
        .bind(candidate_cap)
        .fetch_all(&self.pool)
        .await?;

        let mut scored: Vec<(Fact, f32)> = rows
            .into_iter()
            .filter_map(|mut r| {
                let blob = r.embedding.take()?;
                let emb = blob_to_embedding(&blob)?;
                let row_norm = r.embedding_norm.unwrap_or_else(|| l2_norm(&emb) as f64);
                if row_norm < 1e-9 || qn < 1e-9 {
                    return None;
                }
                let recomputed = l2_norm(&emb) as f64;
                if (recomputed - row_norm).abs() > (row_norm * 0.5 + 1e-6) {
                    return None;
                }
                let sim = cosine_similarity(query_vec, &emb);
                Some((r.into_fact(), sim))
            })
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);
        Ok(scored)
    }

    /// Insert or refresh a typed edge between entities (`ON CONFLICT` upserts
    /// confidence and metadata).
    pub async fn add_relationship(
        &self,
        source: &str,
        relation: &str,
        target: &str,
        confidence: f32,
    ) -> Result<()> {
        self.add_relationship_with_meta(source, relation, target, confidence, None)
            .await
    }

    /// Same as [`SemanticMemory::add_relationship`] with optional JSON/text metadata.
    pub async fn add_relationship_with_meta(
        &self,
        source: &str,
        relation: &str,
        target: &str,
        confidence: f32,
        metadata: Option<&str>,
    ) -> Result<()> {
        let id = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            r#"
            INSERT INTO relationships (id, source_entity, relation, target_entity, confidence, metadata, created_at)
            VALUES (?, ?, ?, ?, ?, ?, datetime('now'))
            ON CONFLICT(source_entity, relation, target_entity) DO UPDATE SET
                confidence = excluded.confidence,
                metadata = excluded.metadata
            "#,
        )
        .bind(&id)
        .bind(source)
        .bind(relation)
        .bind(target)
        .bind(confidence)
        .bind(metadata)
        .execute(&self.pool)
        .await?;

        let row = sqlx::query_as::<_, Relationship>(
            "SELECT id, source_entity, relation, target_entity, confidence, metadata, created_at \
             FROM relationships WHERE source_entity = ? AND relation = ? AND target_entity = ?",
        )
        .bind(source)
        .bind(relation)
        .bind(target)
        .fetch_one(&self.pool)
        .await?;

        let mut g = self
            .relationship_graph
            .lock()
            .map_err(|e| anyhow::anyhow!("relationship graph lock poisoned: {e}"))?;
        g.upsert_relationship(row);
        Ok(())
    }

    /// All relationships where `entity` appears as source or target.
    pub async fn get_relationships(&self, entity: &str) -> Result<Vec<Relationship>> {
        let rows = sqlx::query_as::<_, Relationship>(
            "SELECT id, source_entity, relation, target_entity, confidence, metadata, created_at \
             FROM relationships WHERE source_entity = ? OR target_entity = ? \
             ORDER BY confidence DESC, created_at DESC",
        )
        .bind(entity)
        .bind(entity)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Breadth-first search on an **undirected** view of the graph (edges may be
    /// traversed source→target or target→source) for a path of at most
    /// `max_depth` hops. Returns the list of edges on the first path found.
    pub async fn find_path(
        &self,
        from_entity: &str,
        to_entity: &str,
        max_depth: usize,
    ) -> Result<Option<Vec<Relationship>>> {
        let g = self
            .relationship_graph
            .lock()
            .map_err(|e| anyhow::anyhow!("relationship graph lock poisoned: {e}"))?;
        Ok(g.find_path(from_entity, to_entity, max_depth))
    }

    /// Entities reachable in 1..=`depth` undirected hops from `entity`
    /// (excluding `entity` itself).
    pub async fn get_related_entities(&self, entity: &str, depth: usize) -> Result<Vec<String>> {
        self.get_related_entities_limited(entity, depth, None).await
    }

    /// Like [`SemanticMemory::get_related_entities`], but stops after collecting
    /// `limit` distinct entities.
    pub async fn get_related_entities_limited(
        &self,
        entity: &str,
        depth: usize,
        limit: Option<usize>,
    ) -> Result<Vec<String>> {
        let g = self
            .relationship_graph
            .lock()
            .map_err(|e| anyhow::anyhow!("relationship graph lock poisoned: {e}"))?;
        Ok(g.get_related_entities_limited(entity, depth, limit))
    }

    /// Hybrid search: combines keyword LIKE search with vector similarity.
    ///
    /// Results are ranked by a weighted sum: `alpha * vector_score + (1 - alpha) * keyword_score`.
    /// `alpha` in \[0, 1\]: 1.0 = pure vector, 0.0 = pure keyword.
    pub async fn hybrid_search(
        &self,
        keyword: &str,
        query_vec: Option<&EmbeddingVec>,
        alpha: f32,
        limit: usize,
    ) -> Result<Vec<(Fact, f32)>> {
        use std::collections::HashMap;

        let alpha = alpha.clamp(0.0, 1.0);
        let mut scores: HashMap<String, (Fact, f32)> = HashMap::new();

        // Keyword component
        if !keyword.is_empty() {
            let kw_results = self.search(keyword, limit as i64 * 2).await?;
            let n = kw_results.len().max(1) as f32;
            for (rank, fact) in kw_results.into_iter().enumerate() {
                let kw_score = 1.0 - (rank as f32 / n);
                let weighted = (1.0 - alpha) * kw_score;
                scores
                    .entry(fact.id.clone())
                    .and_modify(|(_, s)| *s += weighted)
                    .or_insert((fact, weighted));
            }
        }

        // Vector component
        if let Some(qv) = query_vec {
            let vec_results = self.search_by_vector(qv, limit * 2).await?;
            for (fact, sim) in vec_results {
                let weighted = alpha * sim;
                scores
                    .entry(fact.id.clone())
                    .and_modify(|(_, s)| *s += weighted)
                    .or_insert((fact, weighted));
            }
        }

        let mut ranked: Vec<(Fact, f32)> = scores.into_values().collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        ranked.truncate(limit);
        Ok(ranked)
    }

    /// Return facts that have no embedding vector stored.
    pub async fn unembedded_facts(&self, limit: i64) -> Result<Vec<Fact>> {
        let rows = sqlx::query_as::<_, Fact>(
            "SELECT id, category, subject, predicate, object, confidence, source_session, created_at, updated_at \
             FROM facts WHERE embedding IS NULL ORDER BY updated_at DESC LIMIT ?",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Backfill embedding + norm for an existing fact row.
    pub async fn update_embedding(&self, id: &str, embedding: &EmbeddingVec) -> Result<()> {
        let blob = embedding_to_blob(embedding);
        let norm = l2_norm(embedding) as f64;
        sqlx::query("UPDATE facts SET embedding = ?, embedding_norm = ? WHERE id = ?")
            .bind(&blob)
            .bind(norm)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

// ---- helpers ----

fn fact_text(f: &Fact) -> String {
    format!("{} {} {}", f.subject, f.predicate, f.object)
}

fn embedding_to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn blob_to_embedding(blob: &[u8]) -> Option<EmbeddingVec> {
    if !blob.len().is_multiple_of(4) {
        return None;
    }
    Some(
        blob.chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
    )
}

#[derive(sqlx::FromRow)]
struct FactWithBlob {
    id: String,
    category: String,
    subject: String,
    predicate: String,
    object: String,
    confidence: f32,
    source_session: Option<String>,
    embedding: Option<Vec<u8>>,
    embedding_norm: Option<f64>,
    created_at: String,
    updated_at: String,
}

impl FactWithBlob {
    fn into_fact(self) -> Fact {
        Fact {
            id: self.id,
            category: self.category,
            subject: self.subject,
            predicate: self.predicate,
            object: self.object,
            confidence: self.confidence,
            source_session: self.source_session,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn mem() -> SemanticMemory {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        SemanticMemory::open(pool).await.unwrap()
    }

    fn fact(id: &str, cat: &str, subj: &str, pred: &str, obj: &str) -> Fact {
        Fact {
            id: id.into(),
            category: cat.into(),
            subject: subj.into(),
            predicate: pred.into(),
            object: obj.into(),
            confidence: 1.0,
            source_session: None,
            created_at: "2026-04-16T10:00:00Z".into(),
            updated_at: "2026-04-16T10:00:00Z".into(),
        }
    }

    #[tokio::test]
    async fn upsert_and_retrieve() {
        let m = mem().await;
        m.upsert(&fact("f1", "user_preference", "user", "prefers", "Rust"))
            .await
            .unwrap();

        let prefs = m
            .by_category(&FactCategory::UserPreference, 10)
            .await
            .unwrap();
        assert_eq!(prefs.len(), 1);
        assert_eq!(prefs[0].object, "Rust");

        m.upsert(&fact(
            "f1",
            "user_preference",
            "user",
            "prefers",
            "Rust + Python",
        ))
        .await
        .unwrap();
        let prefs = m
            .by_category(&FactCategory::UserPreference, 10)
            .await
            .unwrap();
        assert_eq!(prefs.len(), 1);
        assert_eq!(prefs[0].object, "Rust + Python");
    }

    #[tokio::test]
    async fn search_across_fields() {
        let m = mem().await;
        m.upsert(&fact(
            "f1",
            "domain_knowledge",
            "prod_db",
            "runs_on",
            "port 5433",
        ))
        .await
        .unwrap();
        m.upsert(&fact("f2", "user_fact", "user_name", "is", "Lin"))
            .await
            .unwrap();

        let hits = m.search("prod", 10).await.unwrap();
        assert_eq!(hits.len(), 1);

        let hits = m.search("Lin", 10).await.unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[tokio::test]
    async fn build_user_context_output() {
        let m = mem().await;
        m.upsert(&fact(
            "f1",
            "user_preference",
            "user",
            "prefers",
            "concise replies",
        ))
        .await
        .unwrap();
        m.upsert(&fact(
            "f2",
            "correction",
            "user_name",
            "is_not",
            "Ling, it is Lin",
        ))
        .await
        .unwrap();

        let ctx = m.build_user_context(10).await.unwrap();
        assert!(ctx.contains("[semantic memory]"));
        assert!(ctx.contains("Preferences:"));
        assert!(ctx.contains("Corrections:"));
    }

    #[tokio::test]
    async fn delete_fact() {
        let m = mem().await;
        m.upsert(&fact("f1", "user_fact", "a", "b", "c"))
            .await
            .unwrap();
        assert!(m.delete("f1").await.unwrap());
        assert!(!m.delete("f1").await.unwrap());
    }

    #[tokio::test]
    async fn upsert_with_embedding_and_vector_search() {
        let m = mem().await;
        let f = fact("f1", "domain_knowledge", "prod", "runs_on", "port 5433");
        let emb = vec![1.0, 0.0, 0.0];
        m.upsert_with_embedding(&f, &emb).await.unwrap();

        let f2 = fact("f2", "user_fact", "user", "likes", "Rust");
        let emb2 = vec![0.0, 1.0, 0.0];
        m.upsert_with_embedding(&f2, &emb2).await.unwrap();

        let query = vec![0.9, 0.1, 0.0];
        let results = m.search_by_vector(&query, 10).await.unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0.id, "f1");
        assert!(results[0].1 > results[1].1);
    }

    #[tokio::test]
    async fn hybrid_search_combines_keyword_and_vector() {
        let m = mem().await;
        let f1 = fact("f1", "domain_knowledge", "database", "port", "5433");
        let emb1 = vec![1.0, 0.0, 0.0];
        m.upsert_with_embedding(&f1, &emb1).await.unwrap();

        let f2 = fact("f2", "user_fact", "database", "type", "postgres");
        let emb2 = vec![0.0, 1.0, 0.0];
        m.upsert_with_embedding(&f2, &emb2).await.unwrap();

        let f3 = fact("f3", "user_fact", "editor", "is", "vim");
        let emb3 = vec![0.0, 0.0, 1.0];
        m.upsert_with_embedding(&f3, &emb3).await.unwrap();

        let query_vec = vec![0.9, 0.1, 0.0];
        let results = m
            .hybrid_search("database", Some(&query_vec), 0.5, 10)
            .await
            .unwrap();
        assert!(results.len() >= 2);
        // f1 should rank higher: matches keyword "database" AND is closest vector
        assert_eq!(results[0].0.id, "f1");
    }

    #[tokio::test]
    async fn relationships_path_and_neighbours() {
        let m = mem().await;
        m.add_relationship("Alice", "works_at", "Acme", 1.0)
            .await
            .unwrap();
        m.add_relationship("Acme", "city", "NYC", 0.8)
            .await
            .unwrap();
        m.add_relationship("Bob", "works_at", "Other", 1.0)
            .await
            .unwrap();

        let alice_rels = m.get_relationships("Alice").await.unwrap();
        assert_eq!(alice_rels.len(), 1);

        let path = m.find_path("Alice", "NYC", 5).await.unwrap();
        assert!(path.is_some());
        let p = path.unwrap();
        assert_eq!(p.len(), 2);

        let related = m.get_related_entities("Alice", 2).await.unwrap();
        assert!(related.contains(&"Acme".to_string()));
        assert!(related.contains(&"NYC".to_string()));
    }

    #[tokio::test]
    async fn add_relationship_upserts_unique_triple() {
        let m = mem().await;
        m.add_relationship("A", "rel", "B", 0.5).await.unwrap();
        m.add_relationship("A", "rel", "B", 0.9).await.unwrap();
        let r = m.get_relationships("A").await.unwrap();
        assert_eq!(r.len(), 1);
        assert!((r[0].confidence - 0.9).abs() < 1e-5);
    }

    /// Exercises the petgraph index: incremental edge adds (no reopen) then path + neighbours.
    #[tokio::test]
    async fn petgraph_incremental_index_find_path_and_related() {
        let m = mem().await;
        m.add_relationship("n0", "next", "n1", 1.0).await.unwrap();
        m.add_relationship("n1", "next", "n2", 1.0).await.unwrap();
        m.add_relationship("n2", "next", "n3", 1.0).await.unwrap();

        let path = m.find_path("n0", "n3", 10).await.unwrap();
        let p = path.expect("path along chain");
        assert_eq!(p.len(), 3);
        assert_eq!(p[0].source_entity, "n0");
        assert_eq!(p[0].target_entity, "n1");
        assert_eq!(p[2].source_entity, "n2");
        assert_eq!(p[2].target_entity, "n3");

        let related = m.get_related_entities("n1", 2).await.unwrap();
        assert!(related.contains(&"n0".to_string()));
        assert!(related.contains(&"n2".to_string()));
        assert!(related.contains(&"n3".to_string()));

        m.add_relationship("n0", "next", "n1", 0.3).await.unwrap();
        let path2 = m.find_path("n0", "n3", 10).await.unwrap();
        assert!(path2.is_some());
        assert_eq!(path2.unwrap().len(), 3);
    }
}
