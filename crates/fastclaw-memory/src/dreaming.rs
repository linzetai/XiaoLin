use anyhow::Result;
use regex::Regex;
use std::sync::OnceLock;

use crate::episodic::EpisodicMemory;
use crate::semantic::SemanticMemory;

#[derive(Debug, Clone, Default)]
pub struct DreamCycleReport {
    pub episodes_considered: usize,
    pub episodes_marked: usize,
    pub relationships_added: usize,
}

pub struct DreamingPipeline<'a> {
    pub episodic: &'a EpisodicMemory,
    pub semantic: &'a SemanticMemory,
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
            for (src, rel, tgt) in pairs {
                self.semantic
                    .add_relationship(&src, &rel, &tgt, ep.importance.clamp(0.0, 1.0))
                    .await?;
                report.relationships_added += 1;
            }
            ids.push(ep.id.clone());
        }

        let marked = self.episodic.mark_episodes_dreamed(&ids).await?;
        report.episodes_marked = marked;
        Ok(report)
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
        Regex::new(r"(?i)([A-Za-z0-9][A-Za-z0-9 _.\-]{0,48}?)\s+is\s+([A-Za-z0-9][A-Za-z0-9 _.\-]{0,48})")
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
                summary: "The API service depends on Redis and the worker uses Postgres."
                    .into(),
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
        };
        let r = pipe.run_dream_cycle(10).await.unwrap();
        assert_eq!(r.episodes_considered, 2);
        assert_eq!(r.episodes_marked, 2);
        assert_eq!(r.relationships_added, 2);

        let rels_cache = semantic.get_relationships("The cache service").await.unwrap();
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
