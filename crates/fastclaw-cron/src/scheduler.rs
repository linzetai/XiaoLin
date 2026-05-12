use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use cron::Schedule;
use tokio::sync::Notify;

use crate::store::{CronJob, CronJobStore, JobAction, NotifyChannel};

/// Callback trait for executing job actions.
#[async_trait::async_trait]
pub trait JobTrigger: Send + Sync + 'static {
    /// Execute an agent chat. When `notify_channels` is non-empty, the
    /// implementation should run the agent within the channel's conversation
    /// session (so context is shared) and send the reply through the channel.
    /// Returns the agent's reply text and a bool indicating whether the reply
    /// was already sent to channels (so on_job_completed can skip duplicate push).
    async fn trigger_agent_chat(
        &self,
        agent_id: &str,
        message: &str,
        session_id: Option<&str>,
        notify_channels: &[NotifyChannel],
    ) -> anyhow::Result<(String, bool)>;

    async fn trigger_webhook(
        &self,
        url: &str,
        method: Option<&str>,
        body: Option<&serde_json::Value>,
    ) -> anyhow::Result<()>;

    /// Called after a job finishes successfully. Override to send in-app notifications.
    /// `sent_via_channel` is true when the agent reply was already delivered through
    /// the notify channels during `trigger_agent_chat`, so a duplicate push can be skipped.
    async fn on_job_completed(
        &self,
        _job_id: &str,
        _job_name: &str,
        _output: Option<&str>,
        _notify_channels: &[NotifyChannel],
        _sent_via_channel: bool,
    ) {
    }

    /// Called after a job fails. Override to send in-app notifications.
    async fn on_job_failed(
        &self,
        _job_id: &str,
        _job_name: &str,
        _error: &str,
        _notify_channels: &[NotifyChannel],
    ) {
    }
}

pub struct CronScheduler {
    store: Arc<CronJobStore>,
    trigger: Arc<dyn JobTrigger>,
    tick_interval: Duration,
    wake: Arc<Notify>,
}

impl CronScheduler {
    pub fn new(store: Arc<CronJobStore>, trigger: Arc<dyn JobTrigger>) -> Self {
        Self {
            store,
            trigger,
            tick_interval: Duration::from_secs(1),
            wake: Arc::new(Notify::new()),
        }
    }

    /// Create a scheduler sharing an external `Notify` so callers outside the
    /// scheduler (e.g. IPC commands, agent tools) can wake it up.
    pub fn with_wake(
        store: Arc<CronJobStore>,
        trigger: Arc<dyn JobTrigger>,
        wake: Arc<Notify>,
    ) -> Self {
        Self {
            store,
            trigger,
            tick_interval: Duration::from_secs(1),
            wake,
        }
    }

    /// Notify the scheduler to check for due jobs immediately (e.g., after a new job is added).
    pub fn wake(&self) {
        self.wake.notify_one();
    }

    /// Run the scheduler loop. Call this in a `tokio::spawn`.
    pub async fn run(&self) -> anyhow::Result<()> {
        let recovered = self.store.recover_stale().await?;
        if recovered > 0 {
            tracing::warn!(
                recovered,
                "cron: recovered stale running jobs after restart"
            );
        }

        self.initialize_next_runs().await;

        loop {
            let now = Utc::now();
            let due = self.store.due_jobs(&now).await?;

            for job in due {
                let store = self.store.clone();
                let trigger = self.trigger.clone();
                tokio::spawn(async move {
                    execute_job(&store, &*trigger, job).await;
                });
            }

            tokio::select! {
                _ = tokio::time::sleep(self.tick_interval) => {}
                _ = self.wake.notified() => {}
            }
        }
    }

    async fn initialize_next_runs(&self) {
        if let Ok(jobs) = self.store.list().await {
            for mut job in jobs {
                if job.next_run.is_none() && job.enabled {
                    if let Some(next) = compute_next_run(&job.schedule) {
                        job.next_run = Some(next);
                        let _ = self.store.upsert(&job).await;
                    }
                }
            }
        }
    }
}

async fn execute_job(store: &CronJobStore, trigger: &dyn JobTrigger, job: CronJob) {
    let job_id = job.id.clone();
    let schedule = job.schedule.clone();

    match store.mark_running(&job_id).await {
        Ok(true) => {}
        Ok(false) => {
            tracing::debug!(job = %job_id, "cron: job already claimed by another tick, skipping");
            return;
        }
        Err(e) => {
            tracing::error!(job = %job_id, error = %e, "cron: failed to mark job running");
            return;
        }
    }

    tracing::info!(job = %job_id, name = %job.name, "cron: executing job");

    let run_id = store.insert_run(&job_id).await.unwrap_or(-1);

    let result: Result<(Option<String>, bool), anyhow::Error> = match &job.action {
        JobAction::AgentChat {
            agent_id,
            message,
            session_id,
        } => trigger
            .trigger_agent_chat(
                agent_id,
                message,
                session_id.as_deref(),
                &job.notify_channels,
            )
            .await
            .map(|(reply, sent)| (Some(reply), sent)),
        JobAction::Webhook { url, method, body } => trigger
            .trigger_webhook(url, method.as_deref(), body.as_ref())
            .await
            .map(|_| (None, false)),
    };

    let next = compute_next_run(&schedule);
    let next_ref = next.as_deref();

    match result {
        Ok((output, sent_via_channel)) => {
            tracing::info!(job = %job_id, next = ?next_ref, "cron: job completed");
            let _ = store.mark_completed(&job_id, next_ref).await;
            if run_id >= 0 {
                let _ = store.complete_run(run_id, output.as_deref()).await;
            }
            trigger
                .on_job_completed(
                    &job_id,
                    &job.name,
                    output.as_deref(),
                    &job.notify_channels,
                    sent_via_channel,
                )
                .await;
        }
        Err(e) => {
            let err_msg = e.to_string();
            let safe_msg: String = err_msg.chars().take(500).collect();
            tracing::error!(job = %job_id, error = %safe_msg, "cron: job failed");
            let _ = store.mark_failed(&job_id, &safe_msg, next_ref).await;
            if run_id >= 0 {
                let _ = store.fail_run(run_id, &safe_msg).await;
            }
            trigger
                .on_job_failed(&job_id, &job.name, &safe_msg, &job.notify_channels)
                .await;
        }
    }

    if run_id >= 0 {
        let _ = store.prune_runs(&job_id, 50).await;
    }
}

/// Compute the next scheduled run time after `now`.
///
/// Cron expressions are interpreted in the **system's local timezone** so that
/// `"0 9 * * *"` means "9 AM on the machine running FastClaw", regardless of the
/// UTC offset of the server.  The returned RFC3339 string is **normalized to UTC**
/// (offset `+00:00`) so all `next_run` values in the database are directly comparable
/// with `Utc::now()` via SQLite text collation.
fn compute_next_run(schedule_str: &str) -> Option<String> {
    let schedule = Schedule::from_str(schedule_str).ok()?;
    let next_local = schedule.upcoming(chrono::Local).next()?;
    // Normalize to UTC so SQLite text comparison with Utc::now().to_rfc3339() is correct.
    Some(next_local.with_timezone(&Utc).to_rfc3339())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cron_schedule() {
        let next = compute_next_run("0 */5 * * * *");
        assert!(next.is_some(), "should parse a 6-field cron expression");
    }

    #[test]
    fn invalid_schedule_returns_none() {
        assert!(compute_next_run("not a cron").is_none());
    }
}
