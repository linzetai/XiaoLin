//! Conversation trajectory recording for skill auto-formation (Hermes-style).

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TrajectoryOutcome {
    Success {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        user_rating: Option<f64>,
    },
    Failure {
        reason: String,
    },
    Abandoned,
    Unknown,
}

impl TrajectoryOutcome {
    fn outcome_kind(&self) -> &'static str {
        match self {
            TrajectoryOutcome::Success { .. } => "success",
            TrajectoryOutcome::Failure { .. } => "failure",
            TrajectoryOutcome::Abandoned => "abandoned",
            TrajectoryOutcome::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TrajectoryStep {
    pub role: String,
    pub action_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Trajectory {
    pub id: String,
    pub agent_id: String,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_type: Option<String>,
    pub steps: Vec<TrajectoryStep>,
    pub outcome: TrajectoryOutcome,
    pub created_at: String,
}

/// Rule-based task type from tool names used in the trajectory.
pub fn infer_task_type(steps: &[TrajectoryStep]) -> Option<String> {
    let mut saw_research = false;
    let mut saw_code_edit = false;
    let mut saw_terminal = false;
    let mut saw_read = false;

    for s in steps {
        let name = s
            .tool_name
            .as_deref()
            .map(str::to_lowercase)
            .unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        if name.contains("web_search")
            || name.contains("search_web")
            || name.contains("brave_search")
        {
            saw_research = true;
        }
        if name.contains("code_edit")
            || name.contains("search_replace")
            || name.contains("apply_patch")
            || (name.contains("write") && name.contains("file"))
        {
            saw_code_edit = true;
        }
        if name.contains("run_terminal")
            || name.contains("shell")
            || name.contains("execute_command")
            || name == "bash"
        {
            saw_terminal = true;
        }
        if name.contains("read_file")
            || name.contains("grep")
            || name.contains("glob_file")
            || name.contains("list_dir")
        {
            saw_read = true;
        }
    }

    // Priority: more specific workflows first
    if saw_code_edit {
        return Some("code_modification".to_string());
    }
    if saw_read && !saw_code_edit {
        return Some("code_reading".to_string());
    }
    if saw_terminal {
        return Some("terminal_execution".to_string());
    }
    if saw_research {
        return Some("research".to_string());
    }
    None
}

pub struct TrajectoryStore {
    pool: SqlitePool,
}

impl TrajectoryStore {
    pub async fn open(pool: SqlitePool) -> Result<Self> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS trajectories (
                id            TEXT PRIMARY KEY,
                agent_id      TEXT NOT NULL,
                session_id    TEXT NOT NULL,
                task_type     TEXT,
                outcome_kind  TEXT NOT NULL,
                outcome_json  TEXT NOT NULL,
                created_at    TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_traj_agent ON trajectories(agent_id);
            CREATE INDEX IF NOT EXISTS idx_traj_session ON trajectories(session_id);
            CREATE INDEX IF NOT EXISTS idx_traj_task_type ON trajectories(task_type);
            CREATE INDEX IF NOT EXISTS idx_traj_outcome ON trajectories(outcome_kind);
            CREATE INDEX IF NOT EXISTS idx_traj_created ON trajectories(created_at DESC);

            CREATE TABLE IF NOT EXISTS trajectory_steps (
                id             INTEGER PRIMARY KEY AUTOINCREMENT,
                trajectory_id  TEXT NOT NULL REFERENCES trajectories(id) ON DELETE CASCADE,
                step_index     INTEGER NOT NULL,
                role           TEXT NOT NULL,
                action_type    TEXT NOT NULL,
                tool_name      TEXT,
                summary        TEXT NOT NULL,
                success        INTEGER,
                UNIQUE(trajectory_id, step_index)
            );
            CREATE INDEX IF NOT EXISTS idx_traj_steps_traj ON trajectory_steps(trajectory_id);
            "#,
        )
        .execute(&pool)
        .await?;

        Ok(Self { pool })
    }

    pub fn pool(&self) -> SqlitePool {
        self.pool.clone()
    }

    pub async fn record_trajectory(&self, trajectory: &Trajectory) -> Result<()> {
        let outcome_json = serde_json::to_string(&trajectory.outcome)?;
        let outcome_kind = trajectory.outcome.outcome_kind();
        let mut tx = self.pool.begin().await?;

        sqlx::query(
            "INSERT INTO trajectories (id, agent_id, session_id, task_type, outcome_kind, outcome_json, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&trajectory.id)
        .bind(&trajectory.agent_id)
        .bind(&trajectory.session_id)
        .bind(&trajectory.task_type)
        .bind(outcome_kind)
        .bind(&outcome_json)
        .bind(&trajectory.created_at)
        .execute(&mut *tx)
        .await?;

        for (idx, step) in trajectory.steps.iter().enumerate() {
            let success_int = step.success.map(|b| if b { 1i32 } else { 0 });
            sqlx::query(
                "INSERT INTO trajectory_steps
                 (trajectory_id, step_index, role, action_type, tool_name, summary, success)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&trajectory.id)
            .bind(idx as i64)
            .bind(&step.role)
            .bind(&step.action_type)
            .bind(&step.tool_name)
            .bind(&step.summary)
            .bind(success_int)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    pub async fn get_trajectories_by_task_type(
        &self,
        task_type: &str,
        limit: i64,
    ) -> Result<Vec<Trajectory>> {
        let rows: Vec<(String, String, String, String, Option<String>, String, String)> = sqlx::query_as(
            "SELECT id, agent_id, session_id, outcome_kind, task_type, outcome_json, created_at
             FROM trajectories WHERE task_type = ? ORDER BY created_at DESC LIMIT ?",
        )
        .bind(task_type)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        self.hydrate_trajectories(rows).await
    }

    pub async fn get_recent_successful(
        &self,
        agent_id: &str,
        limit: i64,
    ) -> Result<Vec<Trajectory>> {
        let rows: Vec<(String, String, String, String, Option<String>, String, String)> = sqlx::query_as(
            "SELECT id, agent_id, session_id, outcome_kind, task_type, outcome_json, created_at
             FROM trajectories
             WHERE agent_id = ? AND outcome_kind = 'success'
             ORDER BY created_at DESC LIMIT ?",
        )
        .bind(agent_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        self.hydrate_trajectories(rows).await
    }

    /// Recent successful trajectories across all agents (newest first).
    pub async fn get_recent_successful_global(&self, limit: i64) -> Result<Vec<Trajectory>> {
        let rows: Vec<(String, String, String, String, Option<String>, String, String)> = sqlx::query_as(
            "SELECT id, agent_id, session_id, outcome_kind, task_type, outcome_json, created_at
             FROM trajectories
             WHERE outcome_kind = 'success'
             ORDER BY created_at DESC LIMIT ?",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        self.hydrate_trajectories(rows).await
    }

    async fn hydrate_trajectories(
        &self,
        rows: Vec<(String, String, String, String, Option<String>, String, String)>,
    ) -> Result<Vec<Trajectory>> {
        let mut out = Vec::with_capacity(rows.len());
        for (id, agent_id, session_id, _outcome_kind, task_type, outcome_json, created_at) in rows {
            let outcome: TrajectoryOutcome = serde_json::from_str(&outcome_json)?;
            let steps: Vec<(i64, String, String, Option<String>, String, Option<i32>)> = sqlx::query_as(
                "SELECT step_index, role, action_type, tool_name, summary, success
                 FROM trajectory_steps WHERE trajectory_id = ? ORDER BY step_index ASC",
            )
            .bind(&id)
            .fetch_all(&self.pool)
            .await?;

            let steps: Vec<TrajectoryStep> = steps
                .into_iter()
                .map(
                    |(_idx, role, action_type, tool_name, summary, success)| TrajectoryStep {
                        role,
                        action_type,
                        tool_name,
                        summary,
                        success: success.map(|v| v != 0),
                    },
                )
                .collect();

            out.push(Trajectory {
                id,
                agent_id,
                session_id,
                task_type,
                steps,
                outcome,
                created_at,
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        TrajectoryStore::open(pool.clone()).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn trajectory_record_and_retrieve() {
        let pool = test_pool().await;
        let store = TrajectoryStore::open(pool).await.unwrap();

        let tr = Trajectory {
            id: "t1".into(),
            agent_id: "agent-a".into(),
            session_id: "sess-1".into(),
            task_type: Some("research".into()),
            steps: vec![TrajectoryStep {
                role: "tool".into(),
                action_type: "tool_call".into(),
                tool_name: Some("web_search".into()),
                summary: "search docs".into(),
                success: Some(true),
            }],
            outcome: TrajectoryOutcome::Success { user_rating: None },
            created_at: chrono::Utc::now().to_rfc3339(),
        };

        store.record_trajectory(&tr).await.unwrap();
        let got = store
            .get_trajectories_by_task_type("research", 10)
            .await
            .unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].id, "t1");
        assert_eq!(got[0].steps.len(), 1);
    }

    #[test]
    fn infer_task_type_from_tool_names() {
        let steps = vec![TrajectoryStep {
            role: "assistant".into(),
            action_type: "tool_call".into(),
            tool_name: Some("web_search".into()),
            summary: "q".into(),
            success: None,
        }];
        assert_eq!(infer_task_type(&steps), Some("research".to_string()));

        let steps = vec![TrajectoryStep {
            role: "tool".into(),
            action_type: "tool_result".into(),
            tool_name: Some("apply_patch".into()),
            summary: "ok".into(),
            success: Some(true),
        }];
        assert_eq!(infer_task_type(&steps), Some("code_modification".to_string()));
    }
}
