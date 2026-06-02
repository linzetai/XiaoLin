//! Persistent store for extracted skills, parameters, and usage telemetry.
#![allow(clippy::type_complexity)]

use anyhow::{Context, Result};
use sqlx::SqlitePool;

use crate::feedback::FeedbackKind;
use crate::skill_extractor::{ExtractedSkill, SkillParam, SkillStatus};

/// Format active / candidate skills as system-prompt guidance for the LLM.
pub fn format_skills_for_prompt(skills: &[ExtractedSkill]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let mut blocks = vec![
        "\n\n---\n## Learned task skills (Hermes-style)\n".to_string(),
        "Apply the following strategies when they match the user's task. ".to_string(),
        "Treat placeholders as parameters you infer from the request.\n\n".to_string(),
    ];
    for sk in skills {
        blocks.push(format!("### {}\n", sk.name));
        blocks.push(format!("- **Task pattern**: {}\n", sk.task_pattern));
        blocks.push(format!("- **Strategy**:\n{}\n", sk.strategy_template));
        if !sk.parameters.is_empty() {
            blocks.push("- **Parameters**:\n".to_string());
            for p in &sk.parameters {
                let dv = p
                    .default_value
                    .as_ref()
                    .map(|d| format!(" (default: `{d}`)"))
                    .unwrap_or_default();
                blocks.push(format!(
                    "  - `{}` ({}) — {}{}\n",
                    p.name, p.param_type, p.description, dv
                ));
            }
        }
        blocks.push(format!(
            "- **Historical success rate**: {:.0}%\n\n",
            sk.success_rate * 100.0
        ));
    }
    blocks.push("---\n".to_string());
    blocks.concat()
}

/// Formats up to a few **candidate** skills for low-priority prompt injection.
///
/// Each heading is prefixed with `[experimental]` so the model can treat them as unverified.
pub fn format_candidate_skills_for_prompt(skills: &[ExtractedSkill]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let mut blocks = vec![
        "\n\n---\n## Experimental candidate skills (gathering usage data)\n".to_string(),
        "Lower priority than active skills; use only when clearly relevant.\n\n".to_string(),
    ];
    for sk in skills {
        blocks.push(format!("### [experimental] {}\n", sk.name));
        blocks.push(format!("- **Task pattern**: {}\n", sk.task_pattern));
        blocks.push(format!("- **Strategy**:\n{}\n", sk.strategy_template));
        if !sk.parameters.is_empty() {
            blocks.push("- **Parameters**:\n".to_string());
            for p in &sk.parameters {
                let dv = p
                    .default_value
                    .as_ref()
                    .map(|d| format!(" (default: `{d}`)"))
                    .unwrap_or_default();
                blocks.push(format!(
                    "  - `{}` ({}) — {}{}\n",
                    p.name, p.param_type, p.description, dv
                ));
            }
        }
        blocks.push(format!(
            "- **Historical success rate**: {:.0}%\n\n",
            sk.success_rate * 100.0
        ));
    }
    blocks.push("---\n".to_string());
    blocks.concat()
}

/// Summary counts returned by [`SkillStore::maintenance`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct MaintenanceReport {
    /// Candidate skills promoted to active.
    pub promoted: u32,
    /// Active skills retired for persistent underperformance.
    pub retired_active: u32,
}

pub struct SkillStore {
    pool: SqlitePool,
}

impl SkillStore {
    pub async fn open(pool: SqlitePool) -> Result<Self> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS extracted_skills (
                id                    TEXT PRIMARY KEY,
                name                  TEXT NOT NULL,
                task_pattern          TEXT NOT NULL,
                strategy_template     TEXT NOT NULL,
                source_trajectory_ids TEXT NOT NULL,
                success_rate          REAL NOT NULL DEFAULT 0,
                usage_count           INTEGER NOT NULL DEFAULT 0,
                success_count         INTEGER NOT NULL DEFAULT 0,
                status                TEXT NOT NULL,
                created_at            TEXT NOT NULL,
                version               INTEGER NOT NULL DEFAULT 1,
                parent_id             TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_es_task ON extracted_skills(task_pattern);
            CREATE INDEX IF NOT EXISTS idx_es_status ON extracted_skills(status);

            CREATE TABLE IF NOT EXISTS skill_parameters (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                skill_id     TEXT NOT NULL REFERENCES extracted_skills(id) ON DELETE CASCADE,
                name         TEXT NOT NULL,
                param_type   TEXT NOT NULL,
                description  TEXT NOT NULL,
                default_value TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_sp_skill ON skill_parameters(skill_id);

            CREATE TABLE IF NOT EXISTS skill_usages (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                skill_id   TEXT NOT NULL REFERENCES extracted_skills(id) ON DELETE CASCADE,
                success    INTEGER NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX IF NOT EXISTS idx_su_skill ON skill_usages(skill_id);

            CREATE TABLE IF NOT EXISTS evolution_session_skills (
                session_id TEXT NOT NULL,
                skill_id   TEXT NOT NULL,
                PRIMARY KEY (session_id, skill_id)
            );
            CREATE INDEX IF NOT EXISTS idx_ess_session ON evolution_session_skills(session_id);
            "#,
        )
        .execute(&pool)
        .await?;

        Self::migrate_legacy_schema(&pool).await?;

        Ok(Self { pool })
    }

    async fn migrate_legacy_schema(pool: &SqlitePool) -> Result<()> {
        let has_version: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM pragma_table_info('extracted_skills') WHERE name = 'version'",
        )
        .fetch_one(pool)
        .await?;

        if has_version == 0 {
            sqlx::query(
                "ALTER TABLE extracted_skills ADD COLUMN version INTEGER NOT NULL DEFAULT 1",
            )
            .execute(pool)
            .await?;
        }

        let has_parent: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM pragma_table_info('extracted_skills') WHERE name = 'parent_id'",
        )
        .fetch_one(pool)
        .await?;

        if has_parent == 0 {
            sqlx::query("ALTER TABLE extracted_skills ADD COLUMN parent_id TEXT")
                .execute(pool)
                .await?;
        }

        Ok(())
    }

    pub fn pool(&self) -> SqlitePool {
        self.pool.clone()
    }

    pub async fn save_skill(&self, skill: &ExtractedSkill) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let sources = serde_json::to_string(&skill.source_trajectory_ids)?;
        let status_str = skill_status_str(&skill.status);
        let success_count_i = if skill.usage_count > 0 {
            ((skill.success_rate * skill.usage_count as f64).round() as i64).max(0)
        } else {
            0i64
        };

        sqlx::query(
            "INSERT OR REPLACE INTO extracted_skills
             (id, name, task_pattern, strategy_template, source_trajectory_ids, success_rate, usage_count, success_count, status, created_at, version, parent_id)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&skill.id)
        .bind(&skill.name)
        .bind(&skill.task_pattern)
        .bind(&skill.strategy_template)
        .bind(&sources)
        .bind(skill.success_rate)
        .bind(skill.usage_count)
        .bind(success_count_i)
        .bind(status_str)
        .bind(&skill.created_at)
        .bind(skill.version as i64)
        .bind(&skill.parent_id)
        .execute(&mut *tx)
        .await?;

        sqlx::query("DELETE FROM skill_parameters WHERE skill_id = ?")
            .bind(&skill.id)
            .execute(&mut *tx)
            .await?;

        for p in &skill.parameters {
            sqlx::query(
                "INSERT INTO skill_parameters (skill_id, name, param_type, description, default_value)
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(&skill.id)
            .bind(&p.name)
            .bind(&p.param_type)
            .bind(&p.description)
            .bind(&p.default_value)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    pub async fn find_by_task_type(
        &self,
        task_type: &str,
        limit: usize,
    ) -> Result<Vec<ExtractedSkill>> {
        let rows: Vec<(
            String,
            String,
            String,
            String,
            String,
            f64,
            i64,
            i64,
            String,
            String,
            i64,
            Option<String>,
        )> = sqlx::query_as(
            "SELECT id, name, task_pattern, strategy_template, source_trajectory_ids,
                    success_rate, usage_count, success_count, status, created_at, version, parent_id
             FROM extracted_skills
             WHERE task_pattern = ?
             ORDER BY success_rate DESC, usage_count DESC
             LIMIT ?",
        )
        .bind(task_type)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        self.hydrate_skills(rows).await
    }

    /// Keyword overlap against `task_pattern`, `name`, and `strategy_template` (active + candidate).
    pub async fn find_similar(
        &self,
        task_description: &str,
        limit: usize,
    ) -> Result<Vec<ExtractedSkill>> {
        let tokens = tokenize(task_description);
        if tokens.is_empty() {
            return Ok(Vec::new());
        }

        let rows: Vec<(
            String,
            String,
            String,
            String,
            String,
            f64,
            i64,
            i64,
            String,
            String,
            i64,
            Option<String>,
        )> = sqlx::query_as(
            "SELECT id, name, task_pattern, strategy_template, source_trajectory_ids,
                    success_rate, usage_count, success_count, status, created_at, version, parent_id
             FROM extracted_skills
             WHERE status IN ('active', 'candidate')",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut scored: Vec<(i32, ExtractedSkill)> = Vec::new();
        for row in rows {
            let skill = self.row_to_skill(row).await?;
            let mut score = 0i32;
            for t in &tokens {
                if t.is_empty() {
                    continue;
                }
                if skill.task_pattern.to_lowercase().contains(t) {
                    score += 3;
                }
                if skill.name.to_lowercase().contains(t) {
                    score += 2;
                }
                if skill.strategy_template.to_lowercase().contains(t) {
                    score += 1;
                }
            }
            if score > 0 {
                scored.push((score, skill));
            }
        }
        scored.sort_by_key(|b| std::cmp::Reverse(b.0));
        Ok(scored.into_iter().take(limit).map(|(_, s)| s).collect())
    }

    pub async fn record_usage(&self, skill_id: &str, success: bool) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("INSERT INTO skill_usages (skill_id, success) VALUES (?, ?)")
            .bind(skill_id)
            .bind(if success { 1i32 } else { 0 })
            .execute(&mut *tx)
            .await?;

        let row: Option<(i64, i64)> =
            sqlx::query_as("SELECT usage_count, success_count FROM extracted_skills WHERE id = ?")
                .bind(skill_id)
                .fetch_optional(&mut *tx)
                .await?;

        let (usage, succ) = match row {
            Some((u, s)) => {
                let nu = u + 1;
                let ns = s + if success { 1 } else { 0 };
                (nu, ns)
            }
            None => {
                tx.commit().await?;
                return Ok(());
            }
        };

        let rate = succ as f64 / usage as f64;
        sqlx::query(
            "UPDATE extracted_skills SET usage_count = ?, success_count = ?, success_rate = ? WHERE id = ?",
        )
        .bind(usage)
        .bind(succ)
        .bind(rate)
        .bind(skill_id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }

    pub async fn update_status(&self, skill_id: &str, status: SkillStatus) -> Result<()> {
        let s = skill_status_str(&status);
        sqlx::query("UPDATE extracted_skills SET status = ? WHERE id = ?")
            .bind(s)
            .bind(skill_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_active_skills(&self, limit: usize) -> Result<Vec<ExtractedSkill>> {
        let rows: Vec<(
            String,
            String,
            String,
            String,
            String,
            f64,
            i64,
            i64,
            String,
            String,
            i64,
            Option<String>,
        )> = sqlx::query_as(
            "SELECT id, name, task_pattern, strategy_template, source_trajectory_ids,
                    success_rate, usage_count, success_count, status, created_at, version, parent_id
             FROM extracted_skills
             WHERE status = 'active'
             ORDER BY success_rate DESC, usage_count DESC
             LIMIT ?",
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        self.hydrate_skills(rows).await
    }

    /// Marks **active** skills as retired when usage is high enough and success rate is at or below the ceiling.
    pub async fn retire_underperforming(
        &self,
        min_usage: u32,
        max_success_rate: f64,
    ) -> Result<u32> {
        let r = sqlx::query(
            "UPDATE extracted_skills SET status = 'retired'
             WHERE status = 'active'
               AND usage_count >= ?
               AND success_rate <= ?",
        )
        .bind(min_usage as i64)
        .bind(max_success_rate)
        .execute(&self.pool)
        .await?;

        Ok(r.rows_affected() as u32)
    }

    /// Promote **candidate** skills that have enough usage and a solid success rate to **active**.
    ///
    /// Returns the number of rows updated.
    pub async fn promote_candidates(&self, min_usage: u32, min_success_rate: f64) -> Result<u32> {
        let r = sqlx::query(
            "UPDATE extracted_skills SET status = 'active'
             WHERE status = 'candidate'
               AND usage_count >= ?
               AND success_rate >= ?",
        )
        .bind(min_usage as i64)
        .bind(min_success_rate)
        .execute(&self.pool)
        .await?;

        Ok(r.rows_affected() as u32)
    }

    /// Register which evolution skills were surfaced to the model for a chat session.
    pub async fn register_session_skills(
        &self,
        session_id: &str,
        skill_ids: &[String],
    ) -> Result<()> {
        if session_id.trim().is_empty() || skill_ids.is_empty() {
            return Ok(());
        }
        let mut tx = self.pool.begin().await?;
        for sid in skill_ids {
            sqlx::query(
                "INSERT OR IGNORE INTO evolution_session_skills (session_id, skill_id) VALUES (?, ?)",
            )
            .bind(session_id)
            .bind(sid)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    async fn session_skill_ids(&self, session_id: &str) -> Result<Vec<String>> {
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT skill_id FROM evolution_session_skills WHERE session_id = ?")
                .bind(session_id)
                .fetch_all(&self.pool)
                .await?;
        Ok(rows.into_iter().map(|(s,)| s).collect())
    }

    /// Adjust skill telemetry from explicit session feedback (thumbs / ratings).
    ///
    /// Positive feedback increments success; negative decrements success and may retire weak skills.
    pub async fn apply_feedback(&self, session_id: &str, kind: &FeedbackKind) -> Result<()> {
        let (positive, negative) = match kind {
            FeedbackKind::ThumbsUp => (true, false),
            FeedbackKind::ThumbsDown => (false, true),
            FeedbackKind::Rating(r) => {
                if *r >= 4.0 {
                    (true, false)
                } else if *r < 3.0 {
                    (false, true)
                } else {
                    (false, false)
                }
            }
            FeedbackKind::Correction(_) => (false, false),
        };

        if !positive && !negative {
            return Ok(());
        }

        let ids = self.session_skill_ids(session_id).await?;
        if ids.is_empty() {
            return Ok(());
        }

        for skill_id in ids {
            let mut tx = self.pool.begin().await?;
            let row: Option<(i64, i64, String)> = sqlx::query_as(
                "SELECT usage_count, success_count, status FROM extracted_skills WHERE id = ?",
            )
            .bind(&skill_id)
            .fetch_optional(&mut *tx)
            .await?;

            let Some((usage, success_count, status)) = row else {
                tx.commit().await?;
                continue;
            };

            if matches!(parse_skill_status(&status), SkillStatus::Retired) {
                tx.commit().await?;
                continue;
            }

            let (new_usage, new_success) = if positive {
                (usage + 1, success_count + 1)
            } else {
                (usage, (success_count - 1).max(0))
            };

            let rate = new_success as f64 / new_usage.max(1) as f64;
            sqlx::query(
                "UPDATE extracted_skills SET usage_count = ?, success_count = ?, success_rate = ? WHERE id = ?",
            )
            .bind(new_usage)
            .bind(new_success)
            .bind(rate)
            .bind(&skill_id)
            .execute(&mut *tx)
            .await?;

            if negative && new_usage >= 5 && rate < 0.3 {
                sqlx::query("UPDATE extracted_skills SET status = 'retired' WHERE id = ?")
                    .bind(&skill_id)
                    .execute(&mut *tx)
                    .await?;
            }

            tx.commit().await?;
        }

        Ok(())
    }

    /// Runs [`Self::promote_candidates`] (defaults: usage ≥ 3, rate ≥ 0.7) then [`Self::retire_underperforming`]
    /// (defaults: active usage ≥ 10, rate ≤ 0.3).
    pub async fn maintenance(&self) -> Result<MaintenanceReport> {
        let promoted = self.promote_candidates(3, 0.7).await?;
        let retired_active = self.retire_underperforming(10, 0.3).await?;
        Ok(MaintenanceReport {
            promoted,
            retired_active,
        })
    }

    /// Load a single extracted skill by id (any status).
    pub async fn get_skill(&self, skill_id: &str) -> Result<Option<ExtractedSkill>> {
        let row: Option<(
            String,
            String,
            String,
            String,
            String,
            f64,
            i64,
            i64,
            String,
            String,
            i64,
            Option<String>,
        )> = sqlx::query_as(
            "SELECT id, name, task_pattern, strategy_template, source_trajectory_ids,
                    success_rate, usage_count, success_count, status, created_at, version, parent_id
             FROM extracted_skills WHERE id = ?",
        )
        .bind(skill_id)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(r) => Ok(Some(self.row_to_skill(r).await?)),
            None => Ok(None),
        }
    }

    /// Creates a successor row with `version + 1`, links [`ExtractedSkill::parent_id`], and retires the predecessor.
    pub async fn create_new_version(
        &self,
        old_id: &str,
        revised: ExtractedSkill,
    ) -> Result<String> {
        let old = self
            .get_skill(old_id)
            .await?
            .with_context(|| format!("skill not found: {old_id}"))?;

        let mut next = revised;
        next.id = uuid::Uuid::new_v4().to_string();
        next.version = old.version.saturating_add(1);
        next.parent_id = Some(old.id.clone());
        next.created_at = chrono::Utc::now().to_rfc3339();

        self.save_skill(&next).await?;
        self.update_status(old_id, SkillStatus::Retired).await?;
        Ok(next.id)
    }

    /// Walks the `parent_id` chain from `start_id` (newest → older versions).
    pub async fn get_version_history(&self, start_id: &str) -> Result<Vec<ExtractedSkill>> {
        let mut out = Vec::new();
        let mut cur_id: Option<String> = Some(start_id.to_string());

        while let Some(id) = cur_id.take() {
            let Some(skill) = self.get_skill(&id).await? else {
                break;
            };
            cur_id = skill.parent_id.clone();
            out.push(skill);
        }

        Ok(out)
    }

    async fn hydrate_skills(
        &self,
        rows: Vec<(
            String,
            String,
            String,
            String,
            String,
            f64,
            i64,
            i64,
            String,
            String,
            i64,
            Option<String>,
        )>,
    ) -> Result<Vec<ExtractedSkill>> {
        let mut skills = Vec::new();
        for row in rows {
            skills.push(self.row_to_skill(row).await?);
        }
        Ok(skills)
    }

    async fn row_to_skill(
        &self,
        row: (
            String,
            String,
            String,
            String,
            String,
            f64,
            i64,
            i64,
            String,
            String,
            i64,
            Option<String>,
        ),
    ) -> Result<ExtractedSkill> {
        let (
            id,
            name,
            task_pattern,
            strategy_template,
            source_trajectory_ids,
            success_rate,
            usage_count,
            _success_count,
            status,
            created_at,
            version,
            parent_id,
        ) = row;

        let source_trajectory_ids: Vec<String> = serde_json::from_str(&source_trajectory_ids)?;
        let parameters: Vec<(String, String, String, Option<String>)> = sqlx::query_as(
            "SELECT name, param_type, description, default_value FROM skill_parameters WHERE skill_id = ?",
        )
        .bind(&id)
        .fetch_all(&self.pool)
        .await?;

        let parameters: Vec<SkillParam> = parameters
            .into_iter()
            .map(
                |(name, param_type, description, default_value)| SkillParam {
                    name,
                    param_type,
                    description,
                    default_value,
                },
            )
            .collect();

        Ok(ExtractedSkill {
            id,
            name,
            task_pattern,
            strategy_template,
            parameters,
            source_trajectory_ids,
            success_rate,
            usage_count,
            status: parse_skill_status(&status),
            created_at,
            version: version.max(1) as u32,
            parent_id,
        })
    }
}

fn skill_status_str(s: &SkillStatus) -> &'static str {
    match s {
        SkillStatus::Candidate => "candidate",
        SkillStatus::Active => "active",
        SkillStatus::Retired => "retired",
    }
}

fn parse_skill_status(s: &str) -> SkillStatus {
    match s {
        "active" => SkillStatus::Active,
        "retired" => SkillStatus::Retired,
        _ => SkillStatus::Candidate,
    }
}

fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter_map(|w| {
            let w = w.trim();
            if w.len() >= 2 {
                Some(w.to_string())
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skill_extractor::SkillStatus;
    use sqlx::sqlite::SqliteConnectOptions;
    use sqlx::SqlitePool;
    use std::str::FromStr;
    use std::time::Duration;

    async fn test_pool() -> SqlitePool {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePool::connect_with(options).await.unwrap();
        SkillStore::open(pool.clone()).await.unwrap();
        pool
    }

    fn sample_skill(id: &str, task: &str) -> ExtractedSkill {
        ExtractedSkill {
            id: id.to_string(),
            name: "n".into(),
            task_pattern: task.to_string(),
            strategy_template: "1. Use `grep` then 2. Use `read_file`.".into(),
            parameters: vec![],
            source_trajectory_ids: vec!["t1".into()],
            success_rate: 0.9,
            usage_count: 0,
            status: SkillStatus::Active,
            created_at: chrono::Utc::now().to_rfc3339(),
            version: 1,
            parent_id: None,
        }
    }

    #[tokio::test]
    async fn skill_store_save_and_find() {
        let pool = test_pool().await;
        let store = SkillStore::open(pool).await.unwrap();
        let sk = sample_skill("sk1", "research");
        store.save_skill(&sk).await.unwrap();
        let found = store.find_by_task_type("research", 10).await.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "sk1");
    }

    #[tokio::test]
    async fn skill_store_record_usage_updates_stats() {
        let pool = test_pool().await;
        let store = SkillStore::open(pool).await.unwrap();
        let mut sk = sample_skill("sk2", "code");
        sk.usage_count = 0;
        sk.success_rate = 0.0;
        store.save_skill(&sk).await.unwrap();

        store.record_usage("sk2", true).await.unwrap();
        store.record_usage("sk2", false).await.unwrap();
        store.record_usage("sk2", true).await.unwrap();

        let found = store.find_by_task_type("code", 5).await.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].usage_count, 3);
        assert!((found[0].success_rate - 2.0 / 3.0).abs() < 1e-6);
    }

    #[tokio::test]
    async fn skill_store_retire_underperforming() {
        let pool = test_pool().await;
        let store = SkillStore::open(pool).await.unwrap();
        let mut sk = sample_skill("sk3", "x");
        sk.success_rate = 0.2;
        sk.usage_count = 20;
        store.save_skill(&sk).await.unwrap();
        sqlx::query(
            "UPDATE extracted_skills SET usage_count = 20, success_count = 4, success_rate = 0.2 WHERE id = 'sk3'",
        )
        .execute(&store.pool)
        .await
        .unwrap();

        let n = store.retire_underperforming(10, 0.3).await.unwrap();
        assert_eq!(n, 1);
        let st = store.find_by_task_type("x", 5).await.unwrap();
        assert_eq!(st[0].status, SkillStatus::Retired);
    }

    #[test]
    fn format_skills_for_prompt_includes_strategy() {
        let sk = sample_skill("id", "research");
        let txt = format_skills_for_prompt(std::slice::from_ref(&sk));
        assert!(txt.contains("Learned task skills"));
        assert!(txt.contains("grep"));
        assert!(txt.contains("read_file"));
        assert!(txt.contains("research"));
    }

    #[tokio::test]
    async fn promote_candidates_respects_thresholds() {
        let pool = test_pool().await;
        let store = SkillStore::open(pool).await.unwrap();
        let mut c = sample_skill("cand", "t");
        c.status = SkillStatus::Candidate;
        c.usage_count = 4;
        c.success_rate = 0.8;
        store.save_skill(&c).await.unwrap();
        sqlx::query(
            "UPDATE extracted_skills SET usage_count = 4, success_count = 3, success_rate = 0.75 WHERE id = 'cand'",
        )
        .execute(&store.pool)
        .await
        .unwrap();

        let n = store.promote_candidates(3, 0.7).await.unwrap();
        assert_eq!(n, 1);
        let g = store.get_skill("cand").await.unwrap().unwrap();
        assert_eq!(g.status, SkillStatus::Active);
    }

    #[tokio::test]
    async fn apply_feedback_positive_updates_counters() {
        let pool = test_pool().await;
        let store = SkillStore::open(pool).await.unwrap();
        let sk = sample_skill("sfb", "t");
        store.save_skill(&sk).await.unwrap();
        store
            .register_session_skills("sess1", &["sfb".into()])
            .await
            .unwrap();

        store
            .apply_feedback("sess1", &FeedbackKind::ThumbsUp)
            .await
            .unwrap();
        let g = store.get_skill("sfb").await.unwrap().unwrap();
        assert_eq!(g.usage_count, 1);
        assert!((g.success_rate - 1.0).abs() < 1e-6);
    }

    #[tokio::test]
    async fn apply_feedback_negative_retires_when_weak_and_used() {
        let pool = test_pool().await;
        let store = SkillStore::open(pool).await.unwrap();
        let mut sk = sample_skill("neg", "t");
        sk.usage_count = 5;
        sk.success_rate = 0.4;
        store.save_skill(&sk).await.unwrap();
        sqlx::query(
            "UPDATE extracted_skills SET usage_count = 5, success_count = 2, success_rate = 0.4 WHERE id = 'neg'",
        )
        .execute(&store.pool)
        .await
        .unwrap();
        store
            .register_session_skills("sess-neg", &["neg".into()])
            .await
            .unwrap();

        store
            .apply_feedback("sess-neg", &FeedbackKind::ThumbsDown)
            .await
            .unwrap();
        let g = store.get_skill("neg").await.unwrap().unwrap();
        assert_eq!(g.status, SkillStatus::Retired);
        assert!(g.success_rate < 0.3);
    }

    #[tokio::test]
    async fn versioning_create_and_history() {
        let pool = test_pool().await;
        let store = SkillStore::open(pool).await.unwrap();
        let v1 = sample_skill("v1", "task");
        store.save_skill(&v1).await.unwrap();

        let mut revised = v1.clone();
        revised.name = "v2 name".into();
        revised.strategy_template = "updated".into();
        let new_id = store.create_new_version("v1", revised).await.unwrap();

        let old = store.get_skill("v1").await.unwrap().unwrap();
        assert_eq!(old.status, SkillStatus::Retired);

        let new = store.get_skill(&new_id).await.unwrap().unwrap();
        assert_eq!(new.version, 2);
        assert_eq!(new.parent_id.as_deref(), Some("v1"));

        let hist = store.get_version_history(&new_id).await.unwrap();
        assert_eq!(hist.len(), 2);
        assert_eq!(hist[0].id, new_id);
        assert_eq!(hist[1].id, "v1");
    }
}
