use std::sync::Arc;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::dreaming::{DreamCycleReport, DreamingPipeline};
use crate::embedding::EmbeddingProvider;
use crate::episodic::{Episode, EpisodicMemory, ForgetPolicy};
use crate::importance::ImportanceScorer;
use crate::semantic::{Fact, FactCategory, SemanticMemory};
use crate::working::WorkingMemory;

/// Four typed memory categories aligned with qwen-code's memory architecture.
///
/// - `User`: personal preferences, stated facts about the user
/// - `Project`: workspace/codebase knowledge, file structures, conventions
/// - `Feedback`: corrections, error patterns, "do this not that"
/// - `Reference`: external docs, API patterns, third-party knowledge
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryType {
    User,
    Project,
    Feedback,
    Reference,
}

impl MemoryType {
    pub fn as_tag(&self) -> &str {
        match self {
            Self::User => "user",
            Self::Project => "project",
            Self::Feedback => "feedback",
            Self::Reference => "reference",
        }
    }

    pub fn to_fact_category(&self) -> FactCategory {
        match self {
            Self::User => FactCategory::UserPreference,
            Self::Project => FactCategory::DomainKnowledge,
            Self::Feedback => FactCategory::Correction,
            Self::Reference => FactCategory::Custom("reference".to_string()),
        }
    }

    pub fn from_tag(tag: &str) -> Option<Self> {
        match tag {
            "user" => Some(Self::User),
            "project" => Some(Self::Project),
            "feedback" => Some(Self::Feedback),
            "reference" => Some(Self::Reference),
            _ => None,
        }
    }
}

/// Unified facade over episodic, semantic, working memory and the dreaming pipeline.
///
/// Provides high-level `extract`, `recall`, `forget`, and `dream` operations
/// with typed memory categories.
pub struct MemoryManager {
    pub episodic: Arc<EpisodicMemory>,
    pub semantic: Arc<SemanticMemory>,
    pub working: WorkingMemory,
    pub embedder: Option<Arc<dyn EmbeddingProvider>>,
    pub scorer: Option<ImportanceScorer>,
    agent_id: String,
}

/// A recalled memory item from any subsystem.
#[derive(Debug, Clone, Serialize)]
pub struct RecalledMemory {
    pub id: String,
    pub memory_type: Option<MemoryType>,
    pub source: MemorySource,
    pub content: String,
    pub relevance: f32,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemorySource {
    Episodic,
    Semantic,
}

impl MemoryManager {
    pub fn new(
        episodic: Arc<EpisodicMemory>,
        semantic: Arc<SemanticMemory>,
        embedder: Option<Arc<dyn EmbeddingProvider>>,
        scorer: Option<ImportanceScorer>,
        agent_id: impl Into<String>,
        working_memory_size: usize,
    ) -> Self {
        Self {
            episodic,
            semantic,
            working: WorkingMemory::new(working_memory_size),
            embedder,
            scorer,
            agent_id: agent_id.into(),
        }
    }

    /// Extract and store a memory from a conversation fragment.
    ///
    /// Stores as both an episode (timeline) and optionally a semantic fact.
    pub async fn extract(
        &self,
        session_id: &str,
        summary: &str,
        memory_type: MemoryType,
        importance: f32,
    ) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let tag = memory_type.as_tag();

        let episode = Episode {
            id: id.clone(),
            session_id: session_id.to_string(),
            agent_id: self.agent_id.clone(),
            summary: summary.to_string(),
            importance: importance.clamp(0.0, 1.0),
            tags: tag.to_string(),
            created_at: now.clone(),
            dreamed_at: None,
        };

        self.episodic
            .record_auto(&episode, self.embedder.as_deref())
            .await?;

        if matches!(memory_type, MemoryType::User | MemoryType::Project) {
            let fact = Fact {
                id: format!("auto_{}", &id[..8]),
                category: memory_type.to_fact_category().as_str().to_string(),
                subject: self.agent_id.clone(),
                predicate: "knows".to_string(),
                object: summary.to_string(),
                confidence: importance,
                source_session: Some(session_id.to_string()),
                created_at: now.clone(),
                updated_at: now,
            };
            if let Err(e) = self
                .semantic
                .upsert_auto(&fact, self.embedder.as_deref())
                .await
            {
                tracing::warn!(error = %e, "failed to upsert semantic memory");
            }
        }

        Ok(id)
    }

    /// Recall memories relevant to a query, searching across all subsystems.
    ///
    /// Returns results from both episodic and semantic memory, merged by relevance.
    pub async fn recall(
        &self,
        query: &str,
        memory_type: Option<MemoryType>,
        limit: usize,
    ) -> Result<Vec<RecalledMemory>> {
        let mut results = Vec::new();

        let query_vec = if let Some(ref emb) = self.embedder {
            emb.embed(query).await.ok()
        } else {
            None
        };

        let episodes = match self
            .episodic
            .hybrid_search(query, query_vec.as_ref(), 0.5, limit * 2)
            .await
        {
            Ok(eps) => eps,
            Err(e) => {
                tracing::warn!(error = %e, "memory recall: episodic hybrid_search failed");
                Vec::new()
            }
        };

        for (ep, score) in episodes {
            let ep_type = MemoryType::from_tag(&ep.tags);
            if let Some(filter) = memory_type {
                if ep_type != Some(filter) {
                    continue;
                }
            }
            results.push(RecalledMemory {
                id: ep.id,
                memory_type: ep_type,
                source: MemorySource::Episodic,
                content: ep.summary,
                relevance: score,
                created_at: ep.created_at,
            });
        }

        let facts = if let Some(ref qv) = query_vec {
            match self.semantic.search_by_vector(qv, limit).await {
                Ok(facts) => facts,
                Err(e) => {
                    tracing::warn!(error = %e, "memory recall: semantic vector search failed");
                    Vec::new()
                }
            }
        } else {
            match self.semantic.search(query, limit as i64).await {
                Ok(facts) => facts.into_iter().map(|f| (f, 0.5)).collect(),
                Err(e) => {
                    tracing::warn!(error = %e, "memory recall: semantic text search failed");
                    Vec::new()
                }
            }
        };

        for (fact, score) in facts {
            let fact_type = match FactCategory::from_str(&fact.category) {
                FactCategory::UserPreference | FactCategory::UserFact => Some(MemoryType::User),
                FactCategory::DomainKnowledge => Some(MemoryType::Project),
                FactCategory::Correction => Some(MemoryType::Feedback),
                FactCategory::Custom(ref s) if s == "reference" => Some(MemoryType::Reference),
                _ => None,
            };
            if let Some(filter) = memory_type {
                if fact_type != Some(filter) {
                    continue;
                }
            }
            let content = format!("{} {} {}", fact.subject, fact.predicate, fact.object);
            results.push(RecalledMemory {
                id: fact.id,
                memory_type: fact_type,
                source: MemorySource::Semantic,
                content,
                relevance: score,
                created_at: fact.created_at,
            });
        }

        results.sort_by(|a, b| {
            b.relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(limit);
        Ok(results)
    }

    /// Apply the forgetting policy to prune old/low-importance memories.
    pub async fn forget(&self, policy: &ForgetPolicy) -> Result<usize> {
        self.episodic.forget(policy).await
    }

    /// Run the dreaming pipeline to consolidate episodic memories into semantic knowledge.
    pub async fn dream(&self, limit: i64) -> Result<DreamCycleReport> {
        let pipeline = DreamingPipeline {
            episodic: &self.episodic,
            semantic: &self.semantic,
            embedder: self.embedder.clone(),
            scorer: self.scorer.clone(),
        };
        pipeline.run_dream_cycle(limit).await
    }

    /// Build a context-aware recap for the system prompt.
    pub async fn build_context_recap(&self, limit: i64) -> Result<String> {
        let mut recap = String::new();

        let episodic_recap = self
            .episodic
            .build_recap(Some(&self.agent_id), limit)
            .await?;
        if !episodic_recap.is_empty() {
            recap.push_str(&episodic_recap);
        }

        let semantic_recap = self.semantic.build_user_context(limit).await?;
        if !semantic_recap.is_empty() {
            if !recap.is_empty() {
                recap.push('\n');
            }
            recap.push_str(&semantic_recap);
        }

        Ok(recap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::SqlitePool;

    async fn setup() -> MemoryManager {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let episodic = Arc::new(EpisodicMemory::open(pool.clone()).await.unwrap());
        let semantic = Arc::new(SemanticMemory::open(pool).await.unwrap());
        MemoryManager::new(episodic, semantic, None, None, "test_agent", 50)
    }

    #[tokio::test]
    async fn extract_and_recall_user_memory() {
        let mm = setup().await;
        mm.extract("s1", "user prefers dark mode", MemoryType::User, 0.8)
            .await
            .unwrap();

        let results = mm
            .recall("dark mode", Some(MemoryType::User), 10)
            .await
            .unwrap();
        assert!(!results.is_empty());
        assert!(results[0].content.contains("dark mode"));
    }

    #[tokio::test]
    async fn extract_project_memory() {
        let mm = setup().await;
        mm.extract(
            "s1",
            "prod database runs on port 5433",
            MemoryType::Project,
            0.9,
        )
        .await
        .unwrap();

        let results = mm.recall("database", None, 10).await.unwrap();
        assert!(!results.is_empty());
    }

    #[tokio::test]
    async fn memory_type_filtering() {
        let mm = setup().await;
        mm.extract("s1", "user likes Rust", MemoryType::User, 0.7)
            .await
            .unwrap();
        mm.extract("s1", "project uses PostgreSQL", MemoryType::Project, 0.8)
            .await
            .unwrap();

        let user_only = mm
            .recall("likes", Some(MemoryType::User), 10)
            .await
            .unwrap();
        for m in &user_only {
            assert_eq!(m.memory_type, Some(MemoryType::User));
        }
    }

    #[tokio::test]
    async fn memory_type_tags() {
        assert_eq!(MemoryType::User.as_tag(), "user");
        assert_eq!(MemoryType::Project.as_tag(), "project");
        assert_eq!(MemoryType::Feedback.as_tag(), "feedback");
        assert_eq!(MemoryType::Reference.as_tag(), "reference");
        assert_eq!(MemoryType::from_tag("user"), Some(MemoryType::User));
        assert_eq!(MemoryType::from_tag("unknown"), None);
    }
}
