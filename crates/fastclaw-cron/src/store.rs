use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJob {
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub schedule: String,
    pub action: JobAction,
    pub enabled: bool,
    pub last_run: Option<String>,
    pub next_run: Option<String>,
    #[serde(default)]
    pub status: JobStatus,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub run_count: i64,
    #[serde(default)]
    pub error_count: i64,
    pub last_error: Option<String>,
    /// Channels to notify when the job completes or fails.
    /// Each entry specifies a channel id and a target (chat/group) id.
    #[serde(default)]
    pub notify_channels: Vec<NotifyChannel>,
}

/// A channel + target to receive cron job completion/failure notifications.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotifyChannel {
    pub channel_id: String,
    pub target_id: String,
    /// "p2p" or "group". Defaults to "p2p".
    #[serde(default = "default_target_type")]
    pub target_type: String,
}

fn default_target_type() -> String {
    "p2p".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum JobAction {
    AgentChat {
        agent_id: String,
        message: String,
        #[serde(default)]
        session_id: Option<String>,
    },
    Webhook {
        url: String,
        #[serde(default)]
        method: Option<String>,
        #[serde(default)]
        body: Option<serde_json::Value>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    #[default]
    Idle,
    Running,
    Failed,
    Disabled,
}

impl std::fmt::Display for JobStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Idle => write!(f, "idle"),
            Self::Running => write!(f, "running"),
            Self::Failed => write!(f, "failed"),
            Self::Disabled => write!(f, "disabled"),
        }
    }
}

impl JobStatus {
    fn from_str(s: &str) -> Self {
        match s {
            "running" => Self::Running,
            "failed" => Self::Failed,
            "disabled" => Self::Disabled,
            _ => Self::Idle,
        }
    }
}

/// A single execution record for a cron job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJobRun {
    pub id: i64,
    pub job_id: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub status: String,
    pub output: Option<String>,
    pub error: Option<String>,
}

#[derive(sqlx::FromRow)]
struct CronJobRunRow {
    id: i64,
    job_id: String,
    started_at: String,
    ended_at: Option<String>,
    status: String,
    output: Option<String>,
    error: Option<String>,
}

impl From<CronJobRunRow> for CronJobRun {
    fn from(r: CronJobRunRow) -> Self {
        Self {
            id: r.id,
            job_id: r.job_id,
            started_at: r.started_at,
            ended_at: r.ended_at,
            status: r.status,
            output: r.output,
            error: r.error,
        }
    }
}

pub struct CronJobStore {
    pool: SqlitePool,
}

impl CronJobStore {
    pub async fn open(pool: SqlitePool) -> anyhow::Result<Self> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS cron_jobs (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                schedule TEXT NOT NULL,
                action TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1,
                last_run TEXT,
                next_run TEXT,
                status TEXT NOT NULL DEFAULT 'idle',
                created_at TEXT NOT NULL,
                run_count INTEGER NOT NULL DEFAULT 0,
                error_count INTEGER NOT NULL DEFAULT 0,
                last_error TEXT,
                notify_channels TEXT NOT NULL DEFAULT '[]'
            )",
        )
        .execute(&pool)
        .await?;

        // Migration: add notify_channels column to existing tables
        let _ = sqlx::query(
            "ALTER TABLE cron_jobs ADD COLUMN notify_channels TEXT NOT NULL DEFAULT '[]'",
        )
        .execute(&pool)
        .await;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS cron_job_runs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                job_id TEXT NOT NULL REFERENCES cron_jobs(id) ON DELETE CASCADE,
                started_at TEXT NOT NULL,
                ended_at TEXT,
                status TEXT NOT NULL DEFAULT 'running',
                output TEXT,
                error TEXT
            )",
        )
        .execute(&pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_cron_job_runs_job_id ON cron_job_runs(job_id, started_at DESC)",
        )
        .execute(&pool)
        .await?;

        Ok(Self { pool })
    }

    pub async fn upsert(&self, job: &CronJob) -> anyhow::Result<()> {
        let action_json = serde_json::to_string(&job.action)?;
        let notify_json = serde_json::to_string(&job.notify_channels)?;
        sqlx::query(
            "INSERT INTO cron_jobs (id, name, schedule, action, enabled, last_run, next_run, status, created_at, run_count, error_count, last_error, notify_channels)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                schedule = excluded.schedule,
                action = excluded.action,
                enabled = excluded.enabled,
                last_run = excluded.last_run,
                next_run = excluded.next_run,
                status = excluded.status,
                run_count = excluded.run_count,
                error_count = excluded.error_count,
                last_error = excluded.last_error,
                notify_channels = excluded.notify_channels",
        )
        .bind(&job.id)
        .bind(&job.name)
        .bind(&job.schedule)
        .bind(&action_json)
        .bind(job.enabled)
        .bind(&job.last_run)
        .bind(&job.next_run)
        .bind(job.status.to_string())
        .bind(&job.created_at)
        .bind(job.run_count)
        .bind(job.error_count)
        .bind(&job.last_error)
        .bind(&notify_json)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list(&self) -> anyhow::Result<Vec<CronJob>> {
        let rows = sqlx::query_as::<_, CronJobRow>("SELECT * FROM cron_jobs ORDER BY created_at")
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter().map(|r| r.into_job()).collect()
    }

    /// List jobs whose action contains a matching `agent_id` (AgentChat actions).
    pub async fn list_by_agent(&self, agent_id: &str) -> anyhow::Result<Vec<CronJob>> {
        let rows = sqlx::query_as::<_, CronJobRow>(
            "SELECT * FROM cron_jobs WHERE json_extract(action, '$.agent_id') = ? ORDER BY created_at",
        )
        .bind(agent_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(|r| r.into_job()).collect()
    }

    /// Delete all jobs belonging to a specific agent.
    pub async fn delete_by_agent(&self, agent_id: &str) -> anyhow::Result<u64> {
        let result =
            sqlx::query("DELETE FROM cron_jobs WHERE json_extract(action, '$.agent_id') = ?")
                .bind(agent_id)
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected())
    }

    pub async fn get(&self, id: &str) -> anyhow::Result<Option<CronJob>> {
        let row = sqlx::query_as::<_, CronJobRow>("SELECT * FROM cron_jobs WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        match row {
            Some(r) => Ok(Some(r.into_job()?)),
            None => Ok(None),
        }
    }

    pub async fn delete(&self, id: &str) -> anyhow::Result<bool> {
        let result = sqlx::query("DELETE FROM cron_jobs WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn due_jobs(&self, now: &DateTime<Utc>) -> anyhow::Result<Vec<CronJob>> {
        let now_str = now.to_rfc3339();
        let rows = sqlx::query_as::<_, CronJobRow>(
            "SELECT * FROM cron_jobs WHERE enabled = 1 AND status != 'running' AND (next_run IS NULL OR next_run <= ?)",
        )
        .bind(&now_str)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(|r| r.into_job()).collect()
    }

    /// Atomically claim a job for execution. Returns `true` if this caller won the race
    /// (i.e. the job was not already running). Returns `false` if another scheduler tick
    /// already claimed it.
    pub async fn mark_running(&self, id: &str) -> anyhow::Result<bool> {
        let result = sqlx::query(
            "UPDATE cron_jobs SET status = 'running' WHERE id = ? AND status != 'running'",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn mark_completed(&self, id: &str, next_run: Option<&str>) -> anyhow::Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "UPDATE cron_jobs SET status = 'idle', last_run = ?, next_run = ?, run_count = run_count + 1 WHERE id = ?",
        )
        .bind(&now)
        .bind(next_run)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn mark_failed(
        &self,
        id: &str,
        error: &str,
        next_run: Option<&str>,
    ) -> anyhow::Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "UPDATE cron_jobs SET status = 'failed', last_run = ?, next_run = ?, error_count = error_count + 1, last_error = ? WHERE id = ?",
        )
        .bind(&now)
        .bind(next_run)
        .bind(error)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // ─── Run log methods ───

    /// Insert a new run record when a job starts. Returns the run id.
    pub async fn insert_run(&self, job_id: &str) -> anyhow::Result<i64> {
        let now = Utc::now().to_rfc3339();
        let result = sqlx::query(
            "INSERT INTO cron_job_runs (job_id, started_at, status) VALUES (?, ?, 'running')",
        )
        .bind(job_id)
        .bind(&now)
        .execute(&self.pool)
        .await?;
        Ok(result.last_insert_rowid())
    }

    /// Mark a run as completed with optional output text.
    pub async fn complete_run(&self, run_id: i64, output: Option<&str>) -> anyhow::Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "UPDATE cron_job_runs SET status = 'ok', ended_at = ?, output = ? WHERE id = ?",
        )
        .bind(&now)
        .bind(output)
        .bind(run_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Mark a run as failed with error message.
    pub async fn fail_run(&self, run_id: i64, error: &str) -> anyhow::Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "UPDATE cron_job_runs SET status = 'error', ended_at = ?, error = ? WHERE id = ?",
        )
        .bind(&now)
        .bind(error)
        .bind(run_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// List recent runs for a given job, newest first.
    pub async fn list_runs(&self, job_id: &str, limit: i64) -> anyhow::Result<Vec<CronJobRun>> {
        let rows = sqlx::query_as::<_, CronJobRunRow>(
            "SELECT * FROM cron_job_runs WHERE job_id = ? ORDER BY started_at DESC LIMIT ?",
        )
        .bind(job_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(CronJobRun::from).collect())
    }

    /// Delete old runs beyond a retention limit per job.
    pub async fn prune_runs(&self, job_id: &str, keep: i64) -> anyhow::Result<u64> {
        let result = sqlx::query(
            "DELETE FROM cron_job_runs WHERE job_id = ? AND id NOT IN (
                SELECT id FROM cron_job_runs WHERE job_id = ? ORDER BY started_at DESC LIMIT ?
            )",
        )
        .bind(job_id)
        .bind(job_id)
        .bind(keep)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    /// Reset stale `running` jobs back to `idle` after restart.
    /// Increments `error_count` and records the reason, since the previous run
    /// was interrupted and may have had partial side effects.
    pub async fn recover_stale(&self) -> anyhow::Result<u64> {
        let result = sqlx::query(
            "UPDATE cron_jobs SET status = 'idle', error_count = error_count + 1, \
             last_error = 'recovered from stale running state after restart' \
             WHERE status = 'running'",
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }
}

#[derive(sqlx::FromRow)]
struct CronJobRow {
    id: String,
    name: String,
    schedule: String,
    action: String,
    enabled: bool,
    last_run: Option<String>,
    next_run: Option<String>,
    status: String,
    created_at: String,
    run_count: i64,
    error_count: i64,
    last_error: Option<String>,
    #[sqlx(default)]
    notify_channels: String,
}

impl CronJobRow {
    fn into_job(self) -> anyhow::Result<CronJob> {
        let action: JobAction = serde_json::from_str(&self.action)?;
        let notify_channels: Vec<NotifyChannel> = if self.notify_channels.is_empty() {
            Vec::new()
        } else {
            serde_json::from_str(&self.notify_channels).unwrap_or_default()
        };
        Ok(CronJob {
            id: self.id,
            name: self.name,
            schedule: self.schedule,
            action,
            enabled: self.enabled,
            last_run: self.last_run,
            next_run: self.next_run,
            status: JobStatus::from_str(&self.status),
            created_at: self.created_at,
            run_count: self.run_count,
            error_count: self.error_count,
            last_error: self.last_error,
            notify_channels,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

    async fn test_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::new()
            .filename(":memory:")
            .create_if_missing(true);
        SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn crud_lifecycle() {
        let pool = test_pool().await;
        let store = CronJobStore::open(pool).await.unwrap();

        let job = CronJob {
            id: "test-1".into(),
            name: "Test Job".into(),
            schedule: "0 * * * *".into(),
            action: JobAction::AgentChat {
                agent_id: "main".into(),
                message: "hello".into(),
                session_id: None,
            },
            enabled: true,
            last_run: None,
            next_run: None,
            status: JobStatus::Idle,
            created_at: Utc::now().to_rfc3339(),
            run_count: 0,
            error_count: 0,
            last_error: None,
            notify_channels: vec![],
        };

        store.upsert(&job).await.unwrap();

        let listed = store.list().await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "test-1");

        let fetched = store.get("test-1").await.unwrap().unwrap();
        assert_eq!(fetched.name, "Test Job");

        store.mark_running("test-1").await.unwrap();
        let running = store.get("test-1").await.unwrap().unwrap();
        assert_eq!(running.status, JobStatus::Running);

        store
            .mark_completed("test-1", Some("2026-05-01T00:00:00Z"))
            .await
            .unwrap();
        let done = store.get("test-1").await.unwrap().unwrap();
        assert_eq!(done.status, JobStatus::Idle);
        assert_eq!(done.run_count, 1);

        assert!(store.delete("test-1").await.unwrap());
        assert!(store.get("test-1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn recover_stale_jobs() {
        let pool = test_pool().await;
        let store = CronJobStore::open(pool).await.unwrap();

        let job = CronJob {
            id: "stale-1".into(),
            name: "Stale".into(),
            schedule: "* * * * *".into(),
            action: JobAction::Webhook {
                url: "http://example.com".into(),
                method: None,
                body: None,
            },
            enabled: true,
            last_run: None,
            next_run: None,
            status: JobStatus::Idle,
            created_at: Utc::now().to_rfc3339(),
            run_count: 0,
            error_count: 0,
            last_error: None,
            notify_channels: vec![],
        };
        store.upsert(&job).await.unwrap();
        store.mark_running("stale-1").await.unwrap();

        let recovered = store.recover_stale().await.unwrap();
        assert_eq!(recovered, 1);

        let j = store.get("stale-1").await.unwrap().unwrap();
        assert_eq!(j.status, JobStatus::Idle);
    }

    fn make_agent_chat_job(id: &str, agent_id: &str, name: &str) -> CronJob {
        CronJob {
            id: id.into(),
            name: name.into(),
            schedule: "0 * * * * *".into(),
            action: JobAction::AgentChat {
                agent_id: agent_id.into(),
                message: "hello".into(),
                session_id: None,
            },
            enabled: true,
            last_run: None,
            next_run: None,
            status: JobStatus::Idle,
            created_at: Utc::now().to_rfc3339(),
            run_count: 0,
            error_count: 0,
            last_error: None,
            notify_channels: vec![],
        }
    }

    #[tokio::test]
    async fn list_by_agent_filters_correctly() {
        let pool = test_pool().await;
        let store = CronJobStore::open(pool).await.unwrap();

        store
            .upsert(&make_agent_chat_job("j1", "agent-a", "Job A1"))
            .await
            .unwrap();
        store
            .upsert(&make_agent_chat_job("j2", "agent-a", "Job A2"))
            .await
            .unwrap();
        store
            .upsert(&make_agent_chat_job("j3", "agent-b", "Job B1"))
            .await
            .unwrap();

        let webhook_job = CronJob {
            id: "j4".into(),
            name: "Webhook".into(),
            schedule: "0 * * * * *".into(),
            action: JobAction::Webhook {
                url: "http://example.com".into(),
                method: None,
                body: None,
            },
            enabled: true,
            last_run: None,
            next_run: None,
            status: JobStatus::Idle,
            created_at: Utc::now().to_rfc3339(),
            run_count: 0,
            error_count: 0,
            last_error: None,
            notify_channels: vec![],
        };
        store.upsert(&webhook_job).await.unwrap();

        let all = store.list().await.unwrap();
        assert_eq!(all.len(), 4);

        let agent_a = store.list_by_agent("agent-a").await.unwrap();
        assert_eq!(agent_a.len(), 2);
        assert!(agent_a.iter().all(|j| j.id == "j1" || j.id == "j2"));

        let agent_b = store.list_by_agent("agent-b").await.unwrap();
        assert_eq!(agent_b.len(), 1);
        assert_eq!(agent_b[0].id, "j3");

        let agent_c = store.list_by_agent("agent-c").await.unwrap();
        assert_eq!(agent_c.len(), 0);
    }

    #[tokio::test]
    async fn delete_by_agent_removes_only_matching() {
        let pool = test_pool().await;
        let store = CronJobStore::open(pool).await.unwrap();

        store
            .upsert(&make_agent_chat_job("j1", "agent-a", "Job A1"))
            .await
            .unwrap();
        store
            .upsert(&make_agent_chat_job("j2", "agent-a", "Job A2"))
            .await
            .unwrap();
        store
            .upsert(&make_agent_chat_job("j3", "agent-b", "Job B1"))
            .await
            .unwrap();

        let deleted = store.delete_by_agent("agent-a").await.unwrap();
        assert_eq!(deleted, 2);

        let remaining = store.list().await.unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, "j3");
    }

    #[tokio::test]
    async fn delete_by_agent_no_match_returns_zero() {
        let pool = test_pool().await;
        let store = CronJobStore::open(pool).await.unwrap();

        store
            .upsert(&make_agent_chat_job("j1", "agent-a", "Job A1"))
            .await
            .unwrap();

        let deleted = store.delete_by_agent("nonexistent").await.unwrap();
        assert_eq!(deleted, 0);
        assert_eq!(store.list().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn notify_channels_round_trip() {
        let pool = test_pool().await;
        let store = CronJobStore::open(pool).await.unwrap();

        let job = CronJob {
            id: "nc-1".into(),
            name: "Notify Test".into(),
            schedule: "0 * * * * *".into(),
            action: JobAction::AgentChat {
                agent_id: "main".into(),
                message: "test".into(),
                session_id: None,
            },
            enabled: true,
            last_run: None,
            next_run: None,
            status: JobStatus::Idle,
            created_at: Utc::now().to_rfc3339(),
            run_count: 0,
            error_count: 0,
            last_error: None,
            notify_channels: vec![
                NotifyChannel {
                    channel_id: "feishu".into(),
                    target_id: "oc_abc123".into(),
                    target_type: "group".into(),
                },
                NotifyChannel {
                    channel_id: "slack".into(),
                    target_id: "C01234".into(),
                    target_type: "p2p".into(),
                },
            ],
        };

        store.upsert(&job).await.unwrap();

        let fetched = store.get("nc-1").await.unwrap().unwrap();
        assert_eq!(fetched.notify_channels.len(), 2);
        assert_eq!(fetched.notify_channels[0].channel_id, "feishu");
        assert_eq!(fetched.notify_channels[0].target_id, "oc_abc123");
        assert_eq!(fetched.notify_channels[0].target_type, "group");
        assert_eq!(fetched.notify_channels[1].channel_id, "slack");
        assert_eq!(fetched.notify_channels[1].target_type, "p2p");
    }
}
