use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkTask {
    pub id: String,
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub suite: String,
    #[serde(default)]
    pub tier: Tier,
    #[serde(default)]
    pub tags: Vec<String>,
    pub prompt: String,
    #[serde(default)]
    pub graders: Vec<GraderConfig>,
    #[serde(default)]
    pub metrics: MetricsConfig,
    #[serde(default)]
    pub environment: EnvironmentConfig,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
pub enum Tier {
    #[default]
    L1,
    L0,
    L2,
    L3,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GraderConfig {
    OutputContains {
        patterns: Vec<String>,
    },
    OutputNotContains {
        patterns: Vec<String>,
    },
    ToolTrace {
        #[serde(default)]
        must_include: Vec<String>,
        #[serde(default)]
        must_not_include: Vec<String>,
        #[serde(default)]
        allowed_shell_patterns: Vec<String>,
    },
    TokenBudget {
        max_total_tokens: u64,
    },
    TurnLimit {
        max_turns: u32,
    },
    FilesystemCheck {
        #[serde(default)]
        must_exist: Vec<String>,
        #[serde(default)]
        must_not_exist: Vec<String>,
        #[serde(default)]
        unchanged: Vec<String>,
        #[serde(default)]
        files: Vec<FileCheck>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileCheck {
    pub path: String,
    #[serde(default)]
    pub contains: Vec<String>,
    #[serde(default)]
    pub not_contains: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MetricsConfig {
    #[serde(default)]
    pub thresholds: MetricsThresholds,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MetricsThresholds {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_total_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_cost_usd: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EnvironmentConfig {
    #[serde(default)]
    pub workspace_fixture: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub max_turns: Option<u32>,
}

impl BenchmarkTask {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let task: Self = serde_yaml::from_str(&content)?;
        if task.id.is_empty() {
            anyhow::bail!("Task missing required field: id");
        }
        if task.prompt.is_empty() {
            anyhow::bail!("Task missing required field: prompt");
        }
        Ok(task)
    }

    pub fn load_dir(dir: &Path) -> anyhow::Result<Vec<Self>> {
        let mut tasks = Vec::new();
        if !dir.is_dir() {
            return Ok(tasks);
        }
        for entry in walkdir(dir)? {
            if entry.extension().is_some_and(|e| e == "yaml" || e == "yml") {
                match Self::load(&entry) {
                    Ok(task) => tasks.push(task),
                    Err(e) => {
                        tracing::warn!(path = %entry.display(), error = %e, "skipping invalid task");
                    }
                }
            }
        }
        tasks.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(tasks)
    }
}

fn walkdir(dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut result = Vec::new();
    fn visit(dir: &Path, out: &mut Vec<PathBuf>) -> anyhow::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                visit(&path, out)?;
            } else {
                out.push(path);
            }
        }
        Ok(())
    }
    visit(dir, &mut result)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_task() {
        let yaml = r#"
id: test-001
prompt: "Read the file and tell me the port number"
"#;
        let task: BenchmarkTask = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(task.id, "test-001");
        assert_eq!(task.tier, Tier::L1);
        assert!(task.graders.is_empty());
    }

    #[test]
    fn parse_full_task() {
        let yaml = r#"
id: tool-routing-001
version: 1
suite: tool-routing
tier: L1
tags: [shell, routing]
prompt: "Read config.toml and tell me server.port"
graders:
  - type: output_contains
    patterns: ["8080"]
  - type: tool_trace
    must_include: [read_file]
    must_not_include: [shell_exec]
  - type: token_budget
    max_total_tokens: 50000
  - type: turn_limit
    max_turns: 3
metrics:
  thresholds:
    max_turns: 3
    max_total_tokens: 50000
environment:
  workspace_fixture: "fixtures/tool-routing/config-read"
  timeout_ms: 30000
"#;
        let task: BenchmarkTask = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(task.id, "tool-routing-001");
        assert_eq!(task.suite, "tool-routing");
        assert_eq!(task.graders.len(), 4);
    }

    #[test]
    fn reject_missing_id() {
        let yaml = r#"
prompt: "do something"
"#;
        let result: Result<BenchmarkTask, _> = serde_yaml::from_str(yaml);
        assert!(result.is_err());
    }

    #[test]
    fn load_nonexistent_file() {
        let result = BenchmarkTask::load(Path::new("/nonexistent/task.yaml"));
        assert!(result.is_err());
    }
}
