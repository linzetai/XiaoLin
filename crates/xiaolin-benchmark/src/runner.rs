use crate::grader;
use crate::metrics::{CollectedResult, MetricsCollector, RunMetrics};
use crate::report::{RunReport, TaskReport};
use crate::task::BenchmarkTask;
use std::collections::HashMap;
use std::path::Path;
use xiaolin_protocol::event::AgentEvent;

/// Metadata captured before agent execution for filesystem unchanged checks.
#[derive(Debug, Clone, Default)]
pub struct FileSnapshot {
    pub size: u64,
    pub modified_secs: u64,
}

/// Execution result from running a single benchmark task.
pub struct TaskExecution {
    pub collected: CollectedResult,
    /// The temp workspace used for execution. Kept alive so graders can inspect files.
    pub workspace: Option<tempfile::TempDir>,
    /// Pre-run file metadata for `FilesystemCheck.unchanged` graders.
    pub pre_run_files: HashMap<String, FileSnapshot>,
}

/// Trait for executing benchmark tasks. Implementations provide the actual
/// agent runtime (scripted mock or live LLM).
#[async_trait::async_trait]
pub trait BenchmarkExecutor: Send + Sync {
    /// Run a single benchmark task and return the collected events.
    /// The executor should:
    /// 1. Set up a workspace from the task's fixture (if any)
    /// 2. Send the task prompt to the agent
    /// 3. Collect all AgentEvents
    /// 4. Return the collected result
    async fn execute(&self, task: &BenchmarkTask) -> anyhow::Result<TaskExecution>;
}

/// Runs a suite of benchmark tasks using the provided executor.
pub struct BenchmarkRunner {
    run_id: String,
}

impl BenchmarkRunner {
    pub fn new(run_id: impl Into<String>) -> Self {
        Self {
            run_id: run_id.into(),
        }
    }

    pub fn generate() -> Self {
        let id = format!(
            "bench-{}",
            chrono::Utc::now().format("%Y%m%d-%H%M%S")
        );
        Self { run_id: id }
    }

    /// Run all tasks and produce a report.
    pub async fn run(
        &self,
        tasks: &[BenchmarkTask],
        executor: &dyn BenchmarkExecutor,
        workspace_dir: &Path,
    ) -> RunReport {
        let mut report = RunReport::new(self.run_id.clone());

        for (i, task) in tasks.iter().enumerate() {
            tracing::info!(
                task_id = %task.id,
                index = i + 1,
                total = tasks.len(),
                "Running benchmark task"
            );

            let task_report = self.run_single(task, executor, workspace_dir).await;
            report.add_task(task_report);
        }

        report
    }

    async fn run_single(
        &self,
        task: &BenchmarkTask,
        executor: &dyn BenchmarkExecutor,
        workspace_dir: &Path,
    ) -> TaskReport {
        match executor.execute(task).await {
            Ok(execution) => {
                let grading_dir = execution
                    .workspace
                    .as_ref()
                    .map_or_else(|| workspace_dir.to_path_buf(), |w| w.path().to_path_buf());

                let grades = grader::evaluate_graders(
                    &task.graders,
                    &execution.collected,
                    &grading_dir,
                    &execution.pre_run_files,
                );
                let pass = grader::all_passed(&grades);

                TaskReport {
                    run_id: self.run_id.clone(),
                    task_id: task.id.clone(),
                    suite: task.suite.clone(),
                    pass,
                    graders: grades,
                    metrics: execution.collected.metrics,
                }
            }
            Err(e) => {
                tracing::error!(task_id = %task.id, error = %e, "Task execution failed");
                TaskReport {
                    run_id: self.run_id.clone(),
                    task_id: task.id.clone(),
                    suite: task.suite.clone(),
                    pass: false,
                    graders: vec![grader::GradeResult {
                        grader_type: "execution_error".into(),
                        pass: false,
                        reason: format!("Execution failed: {e}"),
                    }],
                    metrics: RunMetrics::default(),
                }
            }
        }
    }
}

/// A simple executor that replays pre-recorded AgentEvents from a JSON file.
/// Useful for deterministic regression tests without any agent dependency.
pub struct ReplayExecutor {
    fixtures_dir: std::path::PathBuf,
}

impl ReplayExecutor {
    pub fn new(fixtures_dir: impl Into<std::path::PathBuf>) -> Self {
        Self {
            fixtures_dir: fixtures_dir.into(),
        }
    }
}

#[async_trait::async_trait]
impl BenchmarkExecutor for ReplayExecutor {
    async fn execute(&self, task: &BenchmarkTask) -> anyhow::Result<TaskExecution> {
        let events_path = self.fixtures_dir.join(&task.id).join("events.json");
        if !events_path.exists() {
            anyhow::bail!("fixture file not found: {}", events_path.display());
        }
        let content = tokio::fs::read_to_string(&events_path).await?;
        let events: Vec<AgentEvent> = serde_json::from_str(&content)?;

        let mut collector = MetricsCollector::new();
        for event in &events {
            collector.process_event(event);
        }

        Ok(TaskExecution {
            collected: collector.finalize(),
            workspace: None,
            pre_run_files: HashMap::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::GraderConfig;

    struct MockExecutor {
        result: CollectedResult,
    }

    #[async_trait::async_trait]
    impl BenchmarkExecutor for MockExecutor {
        async fn execute(&self, _task: &BenchmarkTask) -> anyhow::Result<TaskExecution> {
            Ok(TaskExecution {
                collected: self.result.clone(),
                workspace: None,
                pre_run_files: HashMap::new(),
            })
        }
    }

    #[tokio::test]
    async fn runner_executes_tasks() {
        let executor = MockExecutor {
            result: CollectedResult {
                metrics: RunMetrics::default(),
                assistant_text: "port is 8080".into(),
                tool_names_used: vec!["read_file".into()],
            },
        };

        let tasks = vec![BenchmarkTask {
            id: "test-001".into(),
            version: 1,
            suite: "test".into(),
            tier: crate::task::Tier::L1,
            tags: vec![],
            prompt: "read the file".into(),
            graders: vec![
                GraderConfig::OutputContains {
                    patterns: vec!["8080".into()],
                },
                GraderConfig::ToolTrace {
                    must_include: vec!["read_file".into()],
                    must_not_include: vec!["shell_exec".into()],
                    allowed_shell_patterns: vec![],
                },
            ],
            metrics: Default::default(),
            environment: Default::default(),
        }];

        let runner = BenchmarkRunner::new("test-run");
        let report = runner
            .run(&tasks, &executor, Path::new("/tmp"))
            .await;

        assert_eq!(report.total(), 1);
        assert_eq!(report.passed(), 1);
        assert!(report.tasks[0].pass);
    }

    #[tokio::test]
    async fn runner_handles_execution_failure() {
        struct FailingExecutor;

        #[async_trait::async_trait]
        impl BenchmarkExecutor for FailingExecutor {
            async fn execute(&self, _task: &BenchmarkTask) -> anyhow::Result<TaskExecution> {
                anyhow::bail!("connection refused")
            }
        }

        let tasks = vec![BenchmarkTask {
            id: "fail-001".into(),
            version: 1,
            suite: "test".into(),
            tier: crate::task::Tier::L1,
            tags: vec![],
            prompt: "do something".into(),
            graders: vec![],
            metrics: Default::default(),
            environment: Default::default(),
        }];

        let runner = BenchmarkRunner::new("test-run");
        let report = runner
            .run(&tasks, &FailingExecutor, Path::new("/tmp"))
            .await;

        assert_eq!(report.total(), 1);
        assert_eq!(report.failed(), 1);
        assert!(!report.tasks[0].pass);
        assert!(report.tasks[0].graders[0].reason.contains("connection refused"));
    }
}
