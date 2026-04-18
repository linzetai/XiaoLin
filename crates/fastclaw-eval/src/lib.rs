//! Mini evaluation harness for FastClaw agent behavior.
//!
//! Drive cases with [`EvalAgentDriver`] — use [`MockEvalAgent`] for deterministic runs
//! without a live LLM, or plug in a real runtime-backed driver later.

use std::fmt;
use std::path::Path;
use std::time::Instant;

use anyhow::Context;
use async_trait::async_trait;
use chrono::Utc;
use serde::Deserialize;

// Re-export workspace crates for downstream eval binaries/tests.
pub use fastclaw_agent;
pub use fastclaw_core;

/// A single evaluation case.
pub struct EvalCase {
    pub id: String,
    pub category: String,
    pub description: String,
    pub user_messages: Vec<String>,
    pub expected_behaviors: Vec<ExpectedBehavior>,
    pub max_turns: u32,
    pub timeout_secs: u64,
}

/// What we expect the agent to do.
pub enum ExpectedBehavior {
    /// Agent should use a specific tool at some point.
    UsesTool(String),
    /// Agent's final response should contain this text (case-insensitive).
    ResponseContains(String),
    /// Agent should NOT use this tool.
    DoesNotUseTool(String),
    /// Agent completes within N tool calls (inclusive).
    CompletesWithinToolCalls(u32),
    /// Custom validator function.
    Custom(Box<dyn Fn(&EvalResult) -> bool + Send + Sync>),
}

impl fmt::Debug for ExpectedBehavior {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExpectedBehavior::UsesTool(t) => f.debug_tuple("UsesTool").field(t).finish(),
            ExpectedBehavior::ResponseContains(t) => {
                f.debug_tuple("ResponseContains").field(t).finish()
            }
            ExpectedBehavior::DoesNotUseTool(t) => {
                f.debug_tuple("DoesNotUseTool").field(t).finish()
            }
            ExpectedBehavior::CompletesWithinToolCalls(n) => {
                f.debug_tuple("CompletesWithinToolCalls").field(n).finish()
            }
            ExpectedBehavior::Custom(_) => f.write_str("Custom(<fn>)"),
        }
    }
}

/// Raw outcome from an agent driver (before behavior checks).
#[derive(Debug, Clone, Default)]
pub struct EvalRunArtifacts {
    pub tool_calls_made: Vec<String>,
    pub total_turns: u32,
    pub final_response: Option<String>,
}

/// Result of running one eval case.
#[derive(Debug, Clone)]
pub struct EvalResult {
    pub case_id: String,
    pub passed: bool,
    pub tool_calls_made: Vec<String>,
    pub total_turns: u32,
    pub final_response: Option<String>,
    pub duration_ms: u64,
    pub failure_reasons: Vec<String>,
}

/// Result of running a full eval suite.
#[derive(Debug, Clone)]
pub struct EvalSuiteResult {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub results: Vec<EvalResult>,
    pub pass_rate: f64,
    pub run_at: String,
}

/// Runs one case with a wall-clock timeout.
pub async fn run_eval_case(
    case: &EvalCase,
    agent: &dyn EvalAgentDriver,
) -> anyhow::Result<EvalResult> {
    let start = Instant::now();
    let timeout = tokio::time::Duration::from_secs(case.timeout_secs);

    let run = tokio::time::timeout(timeout, agent.run_case(case))
        .await
        .with_context(|| format!("eval case '{}' timed out after {}s", case.id, case.timeout_secs))??;

    let duration_ms = start.elapsed().as_millis() as u64;
    let mut failure_reasons = Vec::new();

    if run.total_turns > case.max_turns {
        failure_reasons.push(format!(
            "exceeded max_turns: {} > {}",
            run.total_turns, case.max_turns
        ));
    }

    for behavior in &case.expected_behaviors {
        match behavior {
            ExpectedBehavior::UsesTool(name) => {
                if !run.tool_calls_made.iter().any(|t| t == name) {
                    failure_reasons.push(format!("expected UsesTool({name}), got {:?}", run.tool_calls_made));
                }
            }
            ExpectedBehavior::ResponseContains(needle) => {
                let hay = run
                    .final_response
                    .as_deref()
                    .unwrap_or("")
                    .to_lowercase();
                if !hay.contains(&needle.to_lowercase()) {
                    failure_reasons.push(format!(
                        "expected ResponseContains({needle:?}), final_response={:?}",
                        run.final_response
                    ));
                }
            }
            ExpectedBehavior::DoesNotUseTool(name) => {
                if run.tool_calls_made.iter().any(|t| t == name) {
                    failure_reasons.push(format!(
                        "expected DoesNotUseTool({name}), but it was called (trace: {:?})",
                        run.tool_calls_made
                    ));
                }
            }
            ExpectedBehavior::CompletesWithinToolCalls(max) => {
                let n = run.tool_calls_made.len() as u32;
                if n > *max {
                    failure_reasons.push(format!(
                        "expected <= {max} tool calls, got {n}: {:?}",
                        run.tool_calls_made
                    ));
                }
            }
            ExpectedBehavior::Custom(_) => {}
        }
    }

    let mut result = EvalResult {
        case_id: case.id.clone(),
        passed: failure_reasons.is_empty(),
        tool_calls_made: run.tool_calls_made.clone(),
        total_turns: run.total_turns,
        final_response: run.final_response.clone(),
        duration_ms,
        failure_reasons: failure_reasons.clone(),
    };

    for behavior in &case.expected_behaviors {
        if let ExpectedBehavior::Custom(f) = behavior {
            if !f(&result) {
                failure_reasons.push("Custom validator returned false".to_string());
            }
        }
    }

    result.passed = failure_reasons.is_empty();
    result.failure_reasons = failure_reasons;
    Ok(result)
}

/// Runs every case in order and aggregates pass rate.
pub async fn run_eval_suite(
    cases: &[EvalCase],
    agent: &dyn EvalAgentDriver,
) -> anyhow::Result<EvalSuiteResult> {
    let mut results = Vec::with_capacity(cases.len());
    for c in cases {
        results.push(run_eval_case(c, agent).await?);
    }
    let passed = results.iter().filter(|r| r.passed).count();
    let failed = results.len() - passed;
    let total = results.len();
    let pass_rate = if total == 0 {
        0.0
    } else {
        passed as f64 / total as f64
    };
    Ok(EvalSuiteResult {
        total,
        passed,
        failed,
        results,
        pass_rate,
        run_at: Utc::now().to_rfc3339(),
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EvalCaseFile {
    id: String,
    category: String,
    description: String,
    user_messages: Vec<String>,
    expected_behaviors: Vec<ExpectedBehaviorFile>,
    max_turns: u32,
    timeout_secs: u64,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum ExpectedBehaviorFile {
    UsesTool { tool: String },
    ResponseContains { text: String },
    DoesNotUseTool { tool: String },
    CompletesWithinToolCalls { max: u32 },
}

impl TryFrom<EvalCaseFile> for EvalCase {
    type Error = anyhow::Error;

    fn try_from(f: EvalCaseFile) -> Result<Self, Self::Error> {
        let mut expected_behaviors = Vec::new();
        for b in f.expected_behaviors {
            expected_behaviors.push(match b {
                ExpectedBehaviorFile::UsesTool { tool } => ExpectedBehavior::UsesTool(tool),
                ExpectedBehaviorFile::ResponseContains { text } => {
                    ExpectedBehavior::ResponseContains(text)
                }
                ExpectedBehaviorFile::DoesNotUseTool { tool } => {
                    ExpectedBehavior::DoesNotUseTool(tool)
                }
                ExpectedBehaviorFile::CompletesWithinToolCalls { max } => {
                    ExpectedBehavior::CompletesWithinToolCalls(max)
                }
            });
        }
        Ok(EvalCase {
            id: f.id,
            category: f.category,
            description: f.description,
            user_messages: f.user_messages,
            expected_behaviors,
            max_turns: f.max_turns,
            timeout_secs: f.timeout_secs,
        })
    }
}

/// Load `*.json` eval cases from a directory (non-recursive).
pub fn load_eval_cases_from_dir(dir: impl AsRef<Path>) -> anyhow::Result<Vec<EvalCase>> {
    let dir = dir.as_ref();
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .with_context(|| format!("read_dir {}", dir.display()))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("json"))
        .collect();
    paths.sort();
    let mut cases = Vec::new();
    for p in paths {
        let text = std::fs::read_to_string(&p)
            .with_context(|| format!("read {}", p.display()))?;
        let file: EvalCaseFile = serde_json::from_str(&text)
            .with_context(|| format!("parse eval JSON {}", p.display()))?;
        cases.push(EvalCase::try_from(file)?);
    }
    Ok(cases)
}

/// Produces [`EvalRunArtifacts`] for a case (real agent, replay harness, etc.).
#[async_trait]
pub trait EvalAgentDriver: Send + Sync {
    async fn run_case(&self, case: &EvalCase) -> anyhow::Result<EvalRunArtifacts>;
}

/// Lookup-table mock: returns scripted tool traces and final text per case id.
pub struct MockEvalAgent {
    /// Map case id → scripted outcome.
    pub responses: std::collections::HashMap<String, EvalRunArtifacts>,
}

impl MockEvalAgent {
    pub fn new(responses: std::collections::HashMap<String, EvalRunArtifacts>) -> Self {
        Self { responses }
    }

    /// Built-in outcomes aligned with `eval/cases/*.json` for smoke testing without an LLM.
    pub fn builtin_suite_defaults() -> Self {
        let mut m = std::collections::HashMap::new();
        m.insert(
            "basic-001".into(),
            EvalRunArtifacts {
                tool_calls_made: vec!["calculator".into()],
                total_turns: 2,
                final_response: Some("The product is 391.".into()),
            },
        );
        m.insert(
            "basic-002".into(),
            EvalRunArtifacts {
                tool_calls_made: vec!["get_current_time".into()],
                total_turns: 2,
                final_response: Some(r#"{"utc": "2026-04-20T12:00:00Z"} — current UTC per gateway."#.into()),
            },
        );
        m.insert(
            "basic-003".into(),
            EvalRunArtifacts {
                tool_calls_made: vec!["read_file".into()],
                total_turns: 2,
                final_response: Some("Top of Cargo.toml shows `[workspace]`.".into()),
            },
        );
        m.insert(
            "tool-001".into(),
            EvalRunArtifacts {
                tool_calls_made: vec!["web_search".into()],
                total_turns: 2,
                final_response: Some("web_search snippets mention Rust eval patterns.".into()),
            },
        );
        m.insert(
            "tool-002".into(),
            EvalRunArtifacts {
                tool_calls_made: vec!["memory_store".into(), "memory_search".into()],
                total_turns: 3,
                final_response: Some("Stored preference color=blue; memory_search recalled it.".into()),
            },
        );
        m.insert(
            "reason-001".into(),
            EvalRunArtifacts {
                tool_calls_made: vec![],
                total_turns: 1,
                final_response: Some("After giving away 2 of 5 apples, Alice has 3 left.".into()),
            },
        );
        m.insert(
            "reason-002".into(),
            EvalRunArtifacts {
                tool_calls_made: vec!["read_file".into()],
                total_turns: 2,
                final_response: Some("The file declares `name = \"fastclaw-eval\"` as the package.".into()),
            },
        );
        m.insert(
            "error-001".into(),
            EvalRunArtifacts {
                tool_calls_made: vec!["read_file".into()],
                total_turns: 2,
                final_response: Some(
                    "read_file failed: ENOENT (not found) for that path; I will not retry blindly."
                        .into(),
                ),
            },
        );
        m.insert(
            "error-002".into(),
            EvalRunArtifacts {
                tool_calls_made: vec!["calculator".into(), "calculator".into()],
                total_turns: 3,
                final_response: Some(
                    "First expression was invalid; retried with 17*23 and got 391.".into(),
                ),
            },
        );
        m.insert(
            "meta-001".into(),
            EvalRunArtifacts {
                tool_calls_made: vec![],
                total_turns: 1,
                final_response: Some(
                    "I am a FastClaw agent with tools such as read_file, calculator, and web_search."
                        .into(),
                ),
            },
        );
        Self::new(m)
    }
}

#[async_trait]
impl EvalAgentDriver for MockEvalAgent {
    async fn run_case(&self, case: &EvalCase) -> anyhow::Result<EvalRunArtifacts> {
        self.responses
            .get(&case.id)
            .cloned()
            .with_context(|| format!("MockEvalAgent: no scripted outcome for case id {:?}", case.id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_suite_passes_builtin_json_cases() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../eval/cases");
        let cases = load_eval_cases_from_dir(&dir).expect("load cases");
        let agent = MockEvalAgent::builtin_suite_defaults();
        let suite = run_eval_suite(&cases, &agent).await.expect("suite");
        assert_eq!(suite.total, 10);
        assert_eq!(suite.failed, 0, "{:?}", suite.results);
    }

    #[tokio::test]
    async fn uses_tool_failure_detected() {
        let case = EvalCase {
            id: "x".into(),
            category: "t".into(),
            description: "d".into(),
            user_messages: vec!["hi".into()],
            expected_behaviors: vec![ExpectedBehavior::UsesTool("calculator".into())],
            max_turns: 5,
            timeout_secs: 5,
        };
        let mut map = std::collections::HashMap::new();
        map.insert(
            "x".into(),
            EvalRunArtifacts {
                tool_calls_made: vec![],
                total_turns: 1,
                final_response: Some("no tools".into()),
            },
        );
        let agent = MockEvalAgent::new(map);
        let r = run_eval_case(&case, &agent).await.unwrap();
        assert!(!r.passed);
    }

    #[test]
    fn completes_within_tool_calls_passes() {
        let case = EvalCase {
            id: "c".into(),
            category: "t".into(),
            description: "d".into(),
            user_messages: vec![],
            expected_behaviors: vec![ExpectedBehavior::CompletesWithinToolCalls(2)],
            max_turns: 5,
            timeout_secs: 5,
        };
        let mut map = std::collections::HashMap::new();
        map.insert(
            "c".into(),
            EvalRunArtifacts {
                tool_calls_made: vec!["a".into(), "b".into()],
                total_turns: 1,
                final_response: None,
            },
        );
        let rt = tokio::runtime::Runtime::new().unwrap();
        let r = rt.block_on(run_eval_case(&case, &MockEvalAgent::new(map))).unwrap();
        assert!(r.passed);
    }

    #[test]
    fn custom_validator_runs() {
        let case = EvalCase {
            id: "c".into(),
            category: "t".into(),
            description: "d".into(),
            user_messages: vec![],
            expected_behaviors: vec![ExpectedBehavior::Custom(Box::new(|r: &EvalResult| {
                r.total_turns == 7
            }))],
            max_turns: 99,
            timeout_secs: 5,
        };
        let mut map = std::collections::HashMap::new();
        map.insert(
            "c".into(),
            EvalRunArtifacts {
                tool_calls_made: vec![],
                total_turns: 7,
                final_response: None,
            },
        );
        let rt = tokio::runtime::Runtime::new().unwrap();
        let r = rt
            .block_on(run_eval_case(&case, &MockEvalAgent::new(map.clone())))
            .unwrap();
        assert!(r.passed);

        let case_bad = EvalCase {
            id: "c".into(),
            category: "t".into(),
            description: "d".into(),
            user_messages: vec![],
            expected_behaviors: vec![ExpectedBehavior::Custom(Box::new(|r: &EvalResult| {
                r.total_turns == 99
            }))],
            max_turns: 99,
            timeout_secs: 5,
        };
        let r2 = rt
            .block_on(run_eval_case(&case_bad, &MockEvalAgent::new(map.clone())))
            .unwrap();
        assert!(!r2.passed);
    }
}
