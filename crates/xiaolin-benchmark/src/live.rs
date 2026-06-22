use std::path::PathBuf;
use std::sync::Arc;

use xiaolin_agent::runtime::orchestrator::ToolOrchestrator;
use xiaolin_agent::AgentRuntime;
use xiaolin_core::agent_config::AgentConfig;
use xiaolin_core::tool::ToolRegistry;
use xiaolin_core::tool_runtime::ApprovalStrategy;
use xiaolin_core::types::{ChatMessage, ChatRequest, Role};
use xiaolin_protocol::event::AgentEvent;

use crate::metrics::MetricsCollector;
use crate::runner::{BenchmarkExecutor, FileSnapshot, TaskExecution};
use crate::task::{BenchmarkTask, GraderConfig};

/// Executes benchmark tasks using a real LLM provider and full agent runtime.
pub struct LiveExecutor {
    runtime: Arc<AgentRuntime>,
    tool_registry: Arc<ToolRegistry>,
    agent_config: AgentConfig,
    orchestrator: Arc<ToolOrchestrator>,
    fixtures_dir: PathBuf,
}

impl LiveExecutor {
    pub fn new(
        runtime: Arc<AgentRuntime>,
        tool_registry: Arc<ToolRegistry>,
        agent_config: AgentConfig,
        fixtures_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            runtime,
            tool_registry,
            agent_config,
            orchestrator: Arc::new(ToolOrchestrator::default()),
            fixtures_dir: fixtures_dir.into(),
        }
    }

    fn setup_workspace(&self, task: &BenchmarkTask) -> anyhow::Result<tempfile::TempDir> {
        let tmp = tempfile::tempdir()?;

        if let Some(fixture) = &task.environment.workspace_fixture {
            let fixture_path = self.fixtures_dir.join(fixture);
            if fixture_path.exists() {
                copy_dir_recursive(&fixture_path, tmp.path())?;
            } else {
                tracing::warn!(
                    fixture = %fixture_path.display(),
                    "Fixture directory not found, using empty workspace"
                );
            }
        }

        Ok(tmp)
    }
}

#[async_trait::async_trait]
impl BenchmarkExecutor for LiveExecutor {
    async fn execute(&self, task: &BenchmarkTask) -> anyhow::Result<TaskExecution> {
        let workspace = self.setup_workspace(task)?;
        let pre_run_files = snapshot_unchanged_files(workspace.path(), &task.graders);

        let timeout_ms = task.environment.timeout_ms.unwrap_or(120_000);

        let request = ChatRequest {
            model: None,
            messages: vec![ChatMessage {
                role: Role::User,
                content: Some(serde_json::Value::String(task.prompt.clone())),
                reasoning_content: None,
                name: None,
                tool_calls: None,
                tool_call_id: None,
                compact_metadata: None,
                enriched_tool_calls_json: None,
                segment_order: None,
            }],
            agent_id: None,
            session_id: None,
            stream: false,
            temperature: None,
            max_tokens: None,
            tools: None,
            slash_intent: None,
            work_dir: Some(workspace.path().to_string_lossy().into_owned()),
            response_language: None,
            goal_mode: None,
        };

        let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentEvent>(256);

        let runtime = self.runtime.clone();
        let config = self.agent_config.clone();
        let registry = self.tool_registry.clone();
        let orchestrator = self.orchestrator.clone();

        let exec_handle = tokio::spawn(async move {
            runtime
                .execute_unified(
                    &config,
                    &request,
                    &registry,
                    tx,
                    ApprovalStrategy::AutoApprove,
                    None,
                    orchestrator,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                )
                .await
        });

        let collect_handle = tokio::spawn(async move {
            let mut collector = MetricsCollector::new();
            while let Some(event) = rx.recv().await {
                collector.process_event(&event);
            }
            collector.finalize()
        });

        let timeout = tokio::time::Duration::from_millis(timeout_ms);
        let result = tokio::time::timeout(timeout, async {
            let exec_result = exec_handle.await;
            if let Err(ref e) = exec_result {
                tracing::error!(error = %e, "Agent execution task failed");
            }
            if let Ok(Err(ref e)) = exec_result {
                tracing::error!(error = %e, "Agent execution returned error");
            }
            collect_handle.await
        })
        .await;

        match result {
            Ok(Ok(collected)) => Ok(TaskExecution {
                collected,
                workspace: Some(workspace),
                pre_run_files,
            }),
            Ok(Err(e)) => anyhow::bail!("Collection task panicked: {e}"),
            Err(_) => anyhow::bail!("Task timed out after {timeout_ms}ms"),
        }
    }
}

fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dest = dst.join(entry.file_name());
        if ty.is_dir() {
            std::fs::create_dir_all(&dest)?;
            copy_dir_recursive(&entry.path(), &dest)?;
        } else {
            std::fs::copy(entry.path(), &dest)?;
        }
    }
    Ok(())
}

fn unchanged_paths(graders: &[GraderConfig]) -> Vec<String> {
    graders
        .iter()
        .filter_map(|g| match g {
            GraderConfig::FilesystemCheck { unchanged, .. } if !unchanged.is_empty() => {
                Some(unchanged.clone())
            }
            _ => None,
        })
        .flatten()
        .collect()
}

fn snapshot_unchanged_files(
    workspace_dir: &std::path::Path,
    graders: &[GraderConfig],
) -> std::collections::HashMap<String, FileSnapshot> {
    use std::collections::HashMap;
    use std::time::UNIX_EPOCH;

    let mut out = HashMap::new();
    for rel in unchanged_paths(graders) {
        let path = workspace_dir.join(&rel);
        if !path.exists() {
            continue;
        }
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        let modified_secs = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        out.insert(
            rel,
            FileSnapshot {
                size: meta.len(),
                modified_secs,
            },
        );
    }
    out
}
