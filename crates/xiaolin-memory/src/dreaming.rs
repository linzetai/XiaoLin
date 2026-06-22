use anyhow::Result;
use regex::Regex;
use std::sync::{Arc, OnceLock};

use crate::embedding::EmbeddingProvider;
use crate::episodic::EpisodicMemory;
use crate::importance::ImportanceScorer;
use crate::semantic::{Fact, FactCategory, SemanticMemory};

#[derive(Debug, Clone, Default)]
pub struct DreamCycleReport {
    pub episodes_considered: usize,
    pub episodes_marked: usize,
    pub relationships_added: usize,
    pub facts_extracted: usize,
    pub embeddings_backfilled: usize,
    pub importance_rescored: usize,
    /// High-importance episodes identified as potential skill candidates.
    pub skill_candidates_found: usize,
}

pub struct DreamingPipeline<'a> {
    pub episodic: &'a EpisodicMemory,
    pub semantic: &'a SemanticMemory,
    pub embedder: Option<Arc<dyn EmbeddingProvider>>,
    pub scorer: Option<ImportanceScorer>,
}

impl DreamingPipeline<'_> {
    pub async fn run_dream_cycle(&self, limit: i64) -> Result<DreamCycleReport> {
        let episodes = self.episodic.recent_unprocessed(limit).await?;
        let mut report = DreamCycleReport {
            episodes_considered: episodes.len(),
            ..Default::default()
        };
        if episodes.is_empty() {
            return Ok(report);
        }

        let mut ids = Vec::with_capacity(episodes.len());
        for ep in &episodes {
            let pairs = extract_entity_relations(&ep.summary);
            for (src, rel, tgt) in &pairs {
                self.semantic
                    .add_relationship(src, rel, tgt, ep.importance.clamp(0.0, 1.0))
                    .await?;
                report.relationships_added += 1;
            }

            let facts = extract_facts(&ep.summary);
            for (subj, pred, obj) in facts {
                let fact_id = format!(
                    "dream_{}_{}",
                    normalize_entity(&subj).replace(' ', "_"),
                    &uuid::Uuid::new_v4().to_string()[..8]
                );
                let now = chrono::Utc::now().to_rfc3339();
                let fact = Fact {
                    id: fact_id,
                    subject: subj,
                    predicate: pred.clone(),
                    object: obj,
                    category: classify_fact_predicate(&pred).as_str().to_string(),
                    confidence: ep.importance.clamp(0.0, 1.0),
                    source_session: Some(ep.session_id.clone()),
                    created_at: now.clone(),
                    updated_at: now,
                };
                if self
                    .semantic
                    .upsert_auto(&fact, self.embedder.as_deref())
                    .await
                    .is_ok()
                {
                    report.facts_extracted += 1;
                }
            }

            ids.push(ep.id.clone());
        }

        let marked = self.episodic.mark_episodes_dreamed(&ids).await?;
        report.episodes_marked = marked;

        self.backfill_embeddings(&mut report, limit).await;
        self.rescore_importance(&mut report, limit).await;

        report.skill_candidates_found = self
            .episodic
            .high_importance(0.8, 20)
            .await
            .map(|eps| {
                eps.iter()
                    .filter(|e| is_procedural_episode(&e.summary))
                    .count()
            })
            .unwrap_or(0);

        Ok(report)
    }

    async fn backfill_embeddings(&self, report: &mut DreamCycleReport, limit: i64) {
        let Some(ref embedder) = self.embedder else {
            return;
        };

        if let Ok(episodes) = self.episodic.unembedded_episodes(limit).await {
            let texts: Vec<&str> = episodes.iter().map(|e| e.summary.as_str()).collect();
            if !texts.is_empty() {
                if let Ok(vecs) = embedder.embed_batch(&texts).await {
                    for (ep, vec) in episodes.iter().zip(vecs.iter()) {
                        if self.episodic.update_embedding(&ep.id, vec).await.is_ok() {
                            report.embeddings_backfilled += 1;
                        }
                    }
                }
            }
        }

        if let Ok(facts) = self.semantic.unembedded_facts(limit).await {
            let texts: Vec<String> = facts
                .iter()
                .map(|f| format!("{} {} {}", f.subject, f.predicate, f.object))
                .collect();
            let refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
            if !refs.is_empty() {
                if let Ok(vecs) = embedder.embed_batch(&refs).await {
                    for (fact, vec) in facts.iter().zip(vecs.iter()) {
                        if self.semantic.update_embedding(&fact.id, vec).await.is_ok() {
                            report.embeddings_backfilled += 1;
                        }
                    }
                }
            }
        }
    }

    async fn rescore_importance(&self, report: &mut DreamCycleReport, limit: i64) {
        if self.scorer.is_none() {
            return;
        }

        if let Ok(episodes) = self.episodic.recent(None, limit).await {
            for ep in &episodes {
                if (ep.importance - 0.5).abs() < 0.01 {
                    let new_score = ImportanceScorer::score_single(&ep.summary);
                    if (new_score - ep.importance).abs() > 0.05
                        && self
                            .episodic
                            .update_importance(&ep.id, new_score)
                            .await
                            .is_ok()
                    {
                        report.importance_rescored += 1;
                    }
                }
            }
        }
    }
}

fn extract_entity_relations(text: &str) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let t = text.trim();
    if t.is_empty() {
        return out;
    }

    static IS_RE: OnceLock<Regex> = OnceLock::new();
    static USES_RE: OnceLock<Regex> = OnceLock::new();
    static DEPENDS_RE: OnceLock<Regex> = OnceLock::new();

    let is_re = IS_RE.get_or_init(|| {
        Regex::new(
            r"(?i)([A-Za-z0-9][A-Za-z0-9 _.\-]{0,48}?)\s+is\s+([A-Za-z0-9][A-Za-z0-9 _.\-]{0,48})",
        )
        .expect("regex")
    });
    let uses_re = USES_RE.get_or_init(|| {
        Regex::new(r"(?i)([A-Za-z0-9][A-Za-z0-9 _.\-]{0,48}?)\s+uses\s+([A-Za-z0-9][A-Za-z0-9 _.\-]{0,48})")
            .expect("regex")
    });
    let depends_re = DEPENDS_RE.get_or_init(|| {
        Regex::new(
            r"(?i)([A-Za-z0-9][A-Za-z0-9 _.\-]{0,48}?)\s+depends\s+on\s+([A-Za-z0-9][A-Za-z0-9 _.\-]{0,48})",
        )
        .expect("regex")
    });

    for cap in is_re.captures_iter(t) {
        let a = normalize_entity(cap.get(1).map(|m| m.as_str()).unwrap_or(""));
        let b = normalize_entity(cap.get(2).map(|m| m.as_str()).unwrap_or(""));
        if !a.is_empty() && !b.is_empty() {
            out.push((a, "is".into(), b));
        }
    }
    for cap in uses_re.captures_iter(t) {
        let a = normalize_entity(cap.get(1).map(|m| m.as_str()).unwrap_or(""));
        let b = normalize_entity(cap.get(2).map(|m| m.as_str()).unwrap_or(""));
        if !a.is_empty() && !b.is_empty() {
            out.push((a, "uses".into(), b));
        }
    }
    for cap in depends_re.captures_iter(t) {
        let a = normalize_entity(cap.get(1).map(|m| m.as_str()).unwrap_or(""));
        let b = normalize_entity(cap.get(2).map(|m| m.as_str()).unwrap_or(""));
        if !a.is_empty() && !b.is_empty() {
            out.push((a, "depends_on".into(), b));
        }
    }

    out
}

/// Extract SPO facts from summaries for storage as semantic facts.
/// Captures patterns like "X prefers Y", "X chose Y", "X selected Y".
fn extract_facts(text: &str) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let t = text.trim();
    if t.is_empty() {
        return out;
    }

    static PREFERS_RE: OnceLock<Regex> = OnceLock::new();
    static LIKES_RE: OnceLock<Regex> = OnceLock::new();
    static WANTS_RE: OnceLock<Regex> = OnceLock::new();
    static CHOSE_RE: OnceLock<Regex> = OnceLock::new();
    static SELECTED_RE: OnceLock<Regex> = OnceLock::new();

    let ent = r"[A-Za-z0-9\u{4e00}-\u{9fff}][A-Za-z0-9 _.\-\u{4e00}-\u{9fff}]{0,48}?";

    let prefers_re = PREFERS_RE
        .get_or_init(|| Regex::new(&format!(r"(?i)({ent})\s+prefers?\s+({ent})")).expect("regex"));
    let likes_re =
        LIKES_RE.get_or_init(|| Regex::new(&format!(r"(?i)({ent})\s+likes?\s+({ent})")).expect("regex"));
    let wants_re =
        WANTS_RE.get_or_init(|| Regex::new(&format!(r"(?i)({ent})\s+wants?\s+({ent})")).expect("regex"));
    let chose_re = CHOSE_RE
        .get_or_init(|| Regex::new(&format!(r"(?i)({ent})\s+chose\s+({ent})")).expect("regex"));
    let selected_re = SELECTED_RE
        .get_or_init(|| Regex::new(&format!(r"(?i)({ent})\s+selected\s+({ent})")).expect("regex"));

    for (re, pred) in [
        (prefers_re, "prefers"),
        (likes_re, "likes"),
        (wants_re, "wants"),
        (chose_re, "chose"),
        (selected_re, "selected"),
    ] {
        for cap in re.captures_iter(t) {
            let a = normalize_entity(cap.get(1).map(|m| m.as_str()).unwrap_or(""));
            let b = normalize_entity(cap.get(2).map(|m| m.as_str()).unwrap_or(""));
            if !a.is_empty() && !b.is_empty() {
                out.push((a, pred.to_string(), b));
            }
        }
    }
    out
}

/// Heuristic: an episode summary is "procedural" (good skill candidate)
/// if it describes multi-step actions, error fixes, or setup workflows.
fn is_procedural_episode(summary: &str) -> bool {
    let lower = summary.to_lowercase();
    let step_indicators = [
        "then", "next", "after", "finally", "step", "first", "然后", "接着", "最后", "步骤", "首先",
    ];
    let action_indicators = [
        "fix",
        "setup",
        "install",
        "configure",
        "migrate",
        "deploy",
        "build",
        "修复",
        "安装",
        "配置",
        "迁移",
        "部署",
    ];

    let has_steps = step_indicators.iter().any(|w| lower.contains(w));
    let has_actions = action_indicators.iter().any(|w| lower.contains(w));
    let word_count = summary.split_whitespace().count();

    (word_count > 30 || has_steps) && has_actions
}

fn classify_fact_predicate(predicate: &str) -> FactCategory {
    match predicate.to_lowercase().as_str() {
        "prefers" | "likes" | "wants" | "chose" | "selected" => FactCategory::UserPreference,
        _ => FactCategory::DomainKnowledge,
    }
}

fn normalize_entity(s: &str) -> String {
    s.trim()
        .trim_matches(|c: char| matches!(c, '.' | ',' | ';' | ':' | '"' | '\'' | ')' | '('))
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::episodic::{Episode, EpisodicMemory};
    use sqlx::SqlitePool;

    #[tokio::test]
    async fn dream_cycle_extracts_and_marks() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let episodic = EpisodicMemory::open(pool.clone()).await.unwrap();
        let semantic = SemanticMemory::open(pool).await.unwrap();

        episodic
            .record(&Episode {
                id: "e1".into(),
                session_id: "s".into(),
                agent_id: "main".into(),
                summary: "The API service depends on Redis and the worker uses Postgres.".into(),
                importance: 0.9,
                tags: String::new(),
                created_at: "2026-04-18T10:00:00Z".into(),
                dreamed_at: None,
            })
            .await
            .unwrap();

        let pipe = DreamingPipeline {
            episodic: &episodic,
            semantic: &semantic,
            embedder: None,
            scorer: None,
        };
        let r = pipe.run_dream_cycle(10).await.unwrap();
        assert_eq!(r.episodes_considered, 1);
        assert_eq!(r.episodes_marked, 1);
        assert!(r.relationships_added >= 2);

        let rels = semantic.get_relationships("The API service").await.unwrap();
        assert!(rels.iter().any(|x| x.relation == "depends_on"));

        let pending = episodic.recent_unprocessed(10).await.unwrap();
        assert!(pending.is_empty());
    }

    #[test]
    fn extract_patterns() {
        let s = "User auth is OAuth2. The app uses SQLite. The build depends on cargo.";
        let p = extract_entity_relations(s);
        assert!(p.iter().any(|(_, rel, _)| rel == "is"));
        assert!(p.iter().any(|(_, rel, _)| rel == "uses"));
        assert!(p.iter().any(|(_, rel, _)| rel == "depends_on"));
    }

    #[tokio::test]
    async fn dream_cycle_no_unprocessed_returns_empty_report() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let episodic = EpisodicMemory::open(pool.clone()).await.unwrap();
        let semantic = SemanticMemory::open(pool).await.unwrap();

        let pipe = DreamingPipeline {
            episodic: &episodic,
            semantic: &semantic,
            embedder: None,
            scorer: None,
        };
        let r = pipe.run_dream_cycle(10).await.unwrap();
        assert_eq!(r.episodes_considered, 0);
        assert_eq!(r.episodes_marked, 0);
        assert_eq!(r.relationships_added, 0);
    }

    #[tokio::test]
    async fn dream_cycle_processes_multiple_episodes_in_order() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let episodic = EpisodicMemory::open(pool.clone()).await.unwrap();
        let semantic = SemanticMemory::open(pool).await.unwrap();

        episodic
            .record(&Episode {
                id: "e_first".into(),
                session_id: "s".into(),
                agent_id: "main".into(),
                summary: "The cache service is Redis.".into(),
                importance: 0.7,
                tags: String::new(),
                created_at: "2026-04-18T09:00:00Z".into(),
                dreamed_at: None,
            })
            .await
            .unwrap();
        episodic
            .record(&Episode {
                id: "e_second".into(),
                session_id: "s".into(),
                agent_id: "main".into(),
                summary: "The worker uses Postgres.".into(),
                importance: 0.8,
                tags: String::new(),
                created_at: "2026-04-18T10:00:00Z".into(),
                dreamed_at: None,
            })
            .await
            .unwrap();

        let pipe = DreamingPipeline {
            episodic: &episodic,
            semantic: &semantic,
            embedder: None,
            scorer: None,
        };
        let r = pipe.run_dream_cycle(10).await.unwrap();
        assert_eq!(r.episodes_considered, 2);
        assert_eq!(r.episodes_marked, 2);
        assert_eq!(r.relationships_added, 2);

        let rels_cache = semantic
            .get_relationships("The cache service")
            .await
            .unwrap();
        assert!(rels_cache.iter().any(|x| x.relation == "is"));

        let rels_worker = semantic.get_relationships("The worker").await.unwrap();
        assert!(rels_worker.iter().any(|x| x.relation == "uses"));

        let pending = episodic.recent_unprocessed(10).await.unwrap();
        assert!(pending.is_empty());
    }

    #[tokio::test]
    async fn dream_cycle_idempotent_second_run() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let episodic = EpisodicMemory::open(pool.clone()).await.unwrap();
        let semantic = SemanticMemory::open(pool).await.unwrap();

        episodic
            .record(&Episode {
                id: "e1".into(),
                session_id: "s".into(),
                agent_id: "main".into(),
                summary: "The API depends on Redis.".into(),
                importance: 0.5,
                tags: String::new(),
                created_at: "2026-04-18T11:00:00Z".into(),
                dreamed_at: None,
            })
            .await
            .unwrap();

        let pipe = DreamingPipeline {
            episodic: &episodic,
            semantic: &semantic,
            embedder: None,
            scorer: None,
        };
        let first = pipe.run_dream_cycle(10).await.unwrap();
        assert_eq!(first.episodes_considered, 1);
        assert_eq!(first.episodes_marked, 1);
        assert!(first.relationships_added >= 1);

        let second = pipe.run_dream_cycle(10).await.unwrap();
        assert_eq!(second.episodes_considered, 0);
        assert_eq!(second.episodes_marked, 0);
        assert_eq!(second.relationships_added, 0);
    }
}
