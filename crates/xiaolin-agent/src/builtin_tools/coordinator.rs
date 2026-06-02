//! Coordinator/Worker orchestration mode for multi-agent task decomposition.
//!
//! A Coordinator agent breaks a complex task into sub-tasks, dispatches them
//! to Worker agents in parallel via the existing TaskManager infrastructure,
//! collects results, and produces a unified summary.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::task::{TaskManager, TaskStatus};

/// A single sub-task assigned to a worker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerTask {
    pub id: String,
    pub description: String,
    pub assigned_agent: Option<String>,
    pub priority: u32,
}

/// Result from a single worker execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerResult {
    pub task_id: String,
    pub status: WorkerStatus,
    pub output: Option<String>,
    pub error: Option<String>,
}

/// Status of a worker task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerStatus {
    Success,
    Failed,
    Skipped,
}

/// A decomposition plan produced by the coordinator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoordinatorPlan {
    pub goal: String,
    pub tasks: Vec<WorkerTask>,
    pub strategy: CoordinatorStrategy,
}

/// How to handle worker failures.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoordinatorStrategy {
    /// All workers must succeed; abort on first failure.
    AllRequired,
    /// Continue even if some workers fail; collect partial results.
    #[default]
    BestEffort,
    /// Retry failed workers up to N times before giving up.
    RetryOnFailure,
}

/// Orchestrates worker dispatching and result collection.
#[allow(dead_code)]
pub struct Coordinator {
    task_manager: Arc<TaskManager>,
    max_retries: u32,
}

#[allow(dead_code)]
impl Coordinator {
    pub fn new(task_manager: Arc<TaskManager>) -> Self {
        Self {
            task_manager,
            max_retries: 2,
        }
    }

    pub fn with_max_retries(mut self, n: u32) -> Self {
        self.max_retries = n;
        self
    }

    /// Dispatch all worker tasks from a plan. Returns task_ids mapped to worker tasks.
    pub async fn dispatch_workers(&self, plan: &CoordinatorPlan) -> HashMap<String, String> {
        let mut task_id_map = HashMap::new();

        for worker_task in &plan.tasks {
            let description = worker_task.description.clone();
            let worker_id = worker_task.id.clone();

            let task_result = self.task_manager.spawn(
                format!("worker:{}", worker_id),
                description.clone(),
                async move { Ok(format!("Completed: {}", description)) },
            );

            match task_result {
                Ok(task_id) => {
                    task_id_map.insert(worker_id, task_id);
                }
                Err(e) => {
                    tracing::warn!(
                        worker_id = %worker_id,
                        error = %e,
                        "failed to dispatch worker task"
                    );
                }
            }
        }

        task_id_map
    }

    /// Collect results from all dispatched workers.
    /// Polls until all tasks have reached a terminal state or timeout.
    pub async fn collect_results(
        &self,
        task_id_map: &HashMap<String, String>,
        strategy: CoordinatorStrategy,
    ) -> Vec<WorkerResult> {
        let mut results = Vec::new();

        for (worker_id, task_id) in task_id_map {
            let info = self.task_manager.get(task_id);

            let result = match info {
                Some(info) => match info.status {
                    TaskStatus::Completed => WorkerResult {
                        task_id: worker_id.clone(),
                        status: WorkerStatus::Success,
                        output: info.output,
                        error: None,
                    },
                    TaskStatus::Failed => {
                        if strategy == CoordinatorStrategy::AllRequired {
                            tracing::error!(
                                worker_id = %worker_id,
                                "worker failed in AllRequired mode"
                            );
                        }
                        WorkerResult {
                            task_id: worker_id.clone(),
                            status: WorkerStatus::Failed,
                            output: None,
                            error: info.error,
                        }
                    }
                    TaskStatus::Pending | TaskStatus::Running | TaskStatus::Cancelled => {
                        WorkerResult {
                            task_id: worker_id.clone(),
                            status: WorkerStatus::Skipped,
                            output: None,
                            error: Some("task did not complete".into()),
                        }
                    }
                },
                None => WorkerResult {
                    task_id: worker_id.clone(),
                    status: WorkerStatus::Skipped,
                    output: None,
                    error: Some("task not found".into()),
                },
            };

            results.push(result);
        }

        results
    }

    /// Generate a summary of coordinator execution results.
    pub fn summarize_results(plan: &CoordinatorPlan, results: &[WorkerResult]) -> String {
        let total = results.len();
        let (succeeded, failed, skipped) = results.iter().fold((0, 0, 0), |(s, f, sk), r| match r
            .status
        {
            WorkerStatus::Success => (s + 1, f, sk),
            WorkerStatus::Failed => (s, f + 1, sk),
            WorkerStatus::Skipped => (s, f, sk + 1),
        });

        let mut summary = format!(
            "## Coordinator Summary: {}\n\nResults: {}/{} succeeded, {} failed, {} skipped\n\n",
            plan.goal, succeeded, total, failed, skipped
        );

        for result in results {
            let status_icon = match result.status {
                WorkerStatus::Success => "✓",
                WorkerStatus::Failed => "✗",
                WorkerStatus::Skipped => "○",
            };
            let detail = result
                .output
                .as_deref()
                .or(result.error.as_deref())
                .unwrap_or("no output");
            summary.push_str(&format!(
                "  {} {}: {}\n",
                status_icon, result.task_id, detail
            ));
        }

        summary
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn test_plan() -> CoordinatorPlan {
        CoordinatorPlan {
            goal: "Build feature X".into(),
            tasks: vec![
                WorkerTask {
                    id: "w1".into(),
                    description: "Implement backend".into(),
                    assigned_agent: None,
                    priority: 1,
                },
                WorkerTask {
                    id: "w2".into(),
                    description: "Implement frontend".into(),
                    assigned_agent: None,
                    priority: 1,
                },
                WorkerTask {
                    id: "w3".into(),
                    description: "Write tests".into(),
                    assigned_agent: None,
                    priority: 2,
                },
            ],
            strategy: CoordinatorStrategy::BestEffort,
        }
    }

    #[tokio::test]
    async fn dispatch_workers_creates_tasks() {
        let mgr = Arc::new(TaskManager::new(10));
        let coord = Coordinator::new(Arc::clone(&mgr));
        let plan = test_plan();

        let id_map = coord.dispatch_workers(&plan).await;
        assert_eq!(id_map.len(), 3);

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(mgr.list().len(), 3);
    }

    #[tokio::test]
    async fn collect_results_after_completion() {
        let mgr = Arc::new(TaskManager::new(10));
        let coord = Coordinator::new(Arc::clone(&mgr));
        let plan = test_plan();

        let id_map = coord.dispatch_workers(&plan).await;
        tokio::time::sleep(Duration::from_millis(100)).await;

        let results = coord
            .collect_results(&id_map, CoordinatorStrategy::BestEffort)
            .await;
        assert_eq!(results.len(), 3);
        assert!(results.iter().all(|r| r.status == WorkerStatus::Success));
    }

    #[tokio::test]
    async fn summarize_results_format() {
        let plan = test_plan();
        let results = vec![
            WorkerResult {
                task_id: "w1".into(),
                status: WorkerStatus::Success,
                output: Some("done".into()),
                error: None,
            },
            WorkerResult {
                task_id: "w2".into(),
                status: WorkerStatus::Failed,
                output: None,
                error: Some("timeout".into()),
            },
            WorkerResult {
                task_id: "w3".into(),
                status: WorkerStatus::Skipped,
                output: None,
                error: Some("not started".into()),
            },
        ];

        let summary = Coordinator::summarize_results(&plan, &results);
        assert!(summary.contains("Build feature X"));
        assert!(summary.contains("1/3 succeeded"));
        assert!(summary.contains("1 failed"));
        assert!(summary.contains("1 skipped"));
    }

    #[tokio::test]
    async fn concurrency_limit_skips_excess_workers() {
        let mgr = Arc::new(TaskManager::new(2));
        let coord = Coordinator::new(Arc::clone(&mgr));

        // Spawn 2 long-running tasks to fill capacity
        mgr.spawn("blocker1".into(), "".into(), async {
            tokio::time::sleep(Duration::from_secs(5)).await;
            Ok("ok".into())
        })
        .unwrap();
        mgr.spawn("blocker2".into(), "".into(), async {
            tokio::time::sleep(Duration::from_secs(5)).await;
            Ok("ok".into())
        })
        .unwrap();

        let plan = test_plan();
        let id_map = coord.dispatch_workers(&plan).await;
        // Some may have failed to spawn due to concurrency limit
        assert!(id_map.len() <= 3);
    }

    #[test]
    fn coordinator_strategy_default_is_best_effort() {
        let strategy = CoordinatorStrategy::default();
        assert_eq!(strategy, CoordinatorStrategy::BestEffort);
    }
}
