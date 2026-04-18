use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::diagnosis::{
    Diagnosis, DiagnosisThresholds, Diagnostician, ExecutionTrace, ToolCallTrace,
};
use crate::sandbox_runner::{SandboxOutcome, SandboxRunner};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IterationConfig {
    #[serde(default = "default_max_rounds")]
    pub max_rounds: u32,
    #[serde(default)]
    pub thresholds: Option<DiagnosisThresholds>,
    #[serde(default)]
    pub test_cases: Vec<TestCase>,
    #[serde(default = "default_pass_threshold")]
    pub pass_threshold: f64,
}

fn default_max_rounds() -> u32 {
    3
}
fn default_pass_threshold() -> f64 {
    0.8
}

impl Default for IterationConfig {
    fn default() -> Self {
        Self {
            max_rounds: default_max_rounds(),
            thresholds: None,
            test_cases: Vec::new(),
            pass_threshold: default_pass_threshold(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestCase {
    pub name: String,
    pub messages: Vec<serde_json::Value>,
    pub expected_contains: Option<Vec<String>>,
    pub expected_not_contains: Option<Vec<String>>,
    pub max_latency_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IterationResult {
    pub status: IterationStatus,
    pub rounds_executed: u32,
    pub diagnoses: Vec<Diagnosis>,
    pub test_results: Vec<TestCaseResult>,
    pub pass_rate: f64,
    pub final_prompt: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IterationStatus {
    /// All tests pass, no critical diagnoses.
    Passed,
    /// Some improvements made but not all tests pass.
    PartialImprovement,
    /// Max rounds reached without meeting the pass threshold.
    MaxRoundsReached,
    /// No sandbox runner available; diagnoses only.
    DiagnosisOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestCaseResult {
    pub name: String,
    pub passed: bool,
    pub output: String,
    pub latency_ms: u64,
    pub failure_reason: Option<String>,
}

pub struct SelfIterEngine {
    diagnostician: Diagnostician,
    sandbox: Option<Arc<dyn SandboxRunner>>,
    config: IterationConfig,
}

impl SelfIterEngine {
    pub fn new(config: IterationConfig) -> Self {
        let thresholds = config.thresholds.clone().unwrap_or_default();
        Self {
            diagnostician: Diagnostician::new(thresholds),
            sandbox: None,
            config,
        }
    }

    /// Engine with default iteration config and no sandbox (diagnosis-only).
    pub fn diagnosis_only() -> Self {
        Self::new(IterationConfig::default())
    }

    pub fn with_sandbox(mut self, sandbox: Arc<dyn SandboxRunner>) -> Self {
        self.sandbox = Some(sandbox);
        self
    }

    /// Run the diagnosis-only path (no sandbox execution).
    pub fn diagnose_trace(&self, trace: &ExecutionTrace) -> Vec<Diagnosis> {
        self.diagnostician.diagnose(trace)
    }

    /// Convenience: diagnose from the current tool-failure streak in the agent loop.
    pub fn diagnose_tool_failure_streak(
        &self,
        agent_id: &str,
        session_id: &str,
        loop_iteration: u32,
        failures: &[ToolCallTrace],
    ) -> Vec<Diagnosis> {
        let trace = ExecutionTrace {
            agent_id: agent_id.to_string(),
            session_id: session_id.to_string(),
            iterations: loop_iteration,
            tool_calls: failures.to_vec(),
            output: None,
            latency_ms: 0,
            estimated_cost: 0.0,
        };
        self.diagnose_trace(&trace)
    }

    /// Turn diagnoses into LLM-facing remediation text (suggested_fix lines only).
    pub fn format_recovery_guidance(diagnoses: &[Diagnosis]) -> Option<String> {
        let lines: Vec<String> = diagnoses
            .iter()
            .filter_map(|d| d.suggested_fix.as_ref().map(|fix| format!("- {}", fix)))
            .collect();
        if lines.is_empty() {
            None
        } else {
            Some(lines.join("\n"))
        }
    }

    /// Full self-iteration loop:
    /// 1. Diagnose the execution trace
    /// 2. Generate fix suggestions
    /// 3. Run test cases in sandbox
    /// 4. Repeat until pass threshold or max rounds
    pub async fn iterate(
        &self,
        agent_id: &str,
        current_prompt: &str,
        trace: &ExecutionTrace,
    ) -> IterationResult {
        let diagnoses = self.diagnostician.diagnose(trace);

        let sandbox = match &self.sandbox {
            Some(s) => s,
            None => {
                return IterationResult {
                    status: IterationStatus::DiagnosisOnly,
                    rounds_executed: 0,
                    diagnoses,
                    test_results: Vec::new(),
                    pass_rate: 0.0,
                    final_prompt: None,
                };
            }
        };

        if self.config.test_cases.is_empty() {
            return IterationResult {
                status: IterationStatus::DiagnosisOnly,
                rounds_executed: 0,
                diagnoses,
                test_results: Vec::new(),
                pass_rate: 0.0,
                final_prompt: None,
            };
        }

        let mut best_prompt = current_prompt.to_string();
        let mut best_pass_rate = 0.0;
        let all_diagnoses = diagnoses;
        let mut last_test_results = Vec::new();

        const HARD_MAX_ROUNDS: u32 = 20;
        const MAX_PROMPT_CHARS: usize = 100_000;
        let effective_max = self.config.max_rounds.min(HARD_MAX_ROUNDS);

        for round in 0..effective_max {
            let test_results = run_test_suite(
                sandbox.as_ref(),
                agent_id,
                &best_prompt,
                &self.config.test_cases,
            )
            .await;

            let passed = test_results.iter().filter(|r| r.passed).count();
            let total = test_results.len().max(1);
            let pass_rate = passed as f64 / total as f64;

            last_test_results = test_results;

            if pass_rate >= self.config.pass_threshold {
                return IterationResult {
                    status: IterationStatus::Passed,
                    rounds_executed: round + 1,
                    diagnoses: all_diagnoses,
                    test_results: last_test_results,
                    pass_rate,
                    final_prompt: Some(best_prompt),
                };
            }

            if pass_rate > best_pass_rate {
                best_pass_rate = pass_rate;
            }

            // Apply simple fix heuristics based on failures
            let failures: Vec<_> = last_test_results.iter().filter(|r| !r.passed).collect();

            let fix_hint = generate_fix_hint(&failures, &all_diagnoses);
            let candidate = format!("{best_prompt}\n\n{fix_hint}");
            if candidate.len() > MAX_PROMPT_CHARS {
                tracing::warn!(
                    agent = agent_id,
                    round,
                    prompt_len = candidate.len(),
                    "prompt exceeded {MAX_PROMPT_CHARS} chars, stopping self-iteration"
                );
                break;
            }
            best_prompt = candidate;

            tracing::info!(
                agent = agent_id,
                round = round + 1,
                pass_rate,
                failures = failures.len(),
                "self-iter: round completed"
            );
        }

        let status = if best_pass_rate > 0.0 {
            IterationStatus::PartialImprovement
        } else {
            IterationStatus::MaxRoundsReached
        };

        IterationResult {
            status,
            rounds_executed: self.config.max_rounds,
            diagnoses: all_diagnoses,
            test_results: last_test_results,
            pass_rate: best_pass_rate,
            final_prompt: Some(best_prompt),
        }
    }
}

async fn run_test_suite(
    sandbox: &dyn SandboxRunner,
    agent_id: &str,
    system_prompt: &str,
    test_cases: &[TestCase],
) -> Vec<TestCaseResult> {
    let mut results = Vec::new();

    for tc in test_cases {
        match sandbox
            .run_sandboxed(agent_id, system_prompt, &tc.messages)
            .await
        {
            Ok(sandbox_result) => {
                let mut passed = sandbox_result.outcome == SandboxOutcome::Success;
                let mut failure_reason = None;

                if let Some(expected) = &tc.expected_contains {
                    for exp in expected {
                        if !sandbox_result.output.contains(exp) {
                            passed = false;
                            failure_reason = Some(format!("output missing expected: '{exp}'"));
                            break;
                        }
                    }
                }

                if let Some(not_expected) = &tc.expected_not_contains {
                    for ne in not_expected {
                        if sandbox_result.output.contains(ne) {
                            passed = false;
                            failure_reason = Some(format!("output contains forbidden: '{ne}'"));
                            break;
                        }
                    }
                }

                if let Some(max_ms) = tc.max_latency_ms {
                    if sandbox_result.latency_ms > max_ms {
                        passed = false;
                        failure_reason = Some(format!(
                            "latency {}ms exceeds max {}ms",
                            sandbox_result.latency_ms, max_ms
                        ));
                    }
                }

                results.push(TestCaseResult {
                    name: tc.name.clone(),
                    passed,
                    output: sandbox_result.output,
                    latency_ms: sandbox_result.latency_ms,
                    failure_reason,
                });
            }
            Err(e) => {
                results.push(TestCaseResult {
                    name: tc.name.clone(),
                    passed: false,
                    output: String::new(),
                    latency_ms: 0,
                    failure_reason: Some(format!("sandbox error: {e}")),
                });
            }
        }
    }

    results
}

fn generate_fix_hint(failures: &[&TestCaseResult], diagnoses: &[Diagnosis]) -> String {
    let mut hints = Vec::new();

    for diag in diagnoses {
        if let Some(fix) = &diag.suggested_fix {
            hints.push(format!("- {fix}"));
        }
    }

    for fail in failures {
        if let Some(reason) = &fail.failure_reason {
            hints.push(format!("- Fix test '{}': {reason}", fail.name));
        }
    }

    if hints.is_empty() {
        "Please improve the response quality and accuracy.".into()
    } else {
        format!("IMPORTANT improvements needed:\n{}", hints.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnosis::{Diagnosis, DiagnosisKind, ExecutionTrace, Severity, ToolCallTrace};
    use crate::sandbox_runner::{
        DirectSandboxRunner, SandboxBackend, SandboxOutcome, SandboxResult, SandboxRunner,
    };

    struct MockSandboxBackend;

    #[async_trait::async_trait]
    impl SandboxBackend for MockSandboxBackend {
        async fn execute_prompt(
            &self,
            _agent_id: &str,
            _system_prompt: &str,
            _test_messages: &[serde_json::Value],
        ) -> anyhow::Result<SandboxResult> {
            Ok(SandboxResult {
                outcome: SandboxOutcome::Success,
                output: "SANDBOX_OK".into(),
                latency_ms: 0,
                token_usage: None,
            })
        }
    }

    /// Returns sandbox output that never satisfies `expected_contains` in failing-suite tests.
    struct MockWrongOutputSandboxBackend;

    #[async_trait::async_trait]
    impl SandboxBackend for MockWrongOutputSandboxBackend {
        async fn execute_prompt(
            &self,
            _agent_id: &str,
            _system_prompt: &str,
            _test_messages: &[serde_json::Value],
        ) -> anyhow::Result<SandboxResult> {
            Ok(SandboxResult {
                outcome: SandboxOutcome::Success,
                output: "WRONG_OUTPUT_FROM_SANDBOX".into(),
                latency_ms: 0,
                token_usage: None,
            })
        }
    }

    #[tokio::test]
    async fn iterate_appends_trace_diagnosis_suggested_fixes_to_prompt() {
        let config = IterationConfig {
            max_rounds: 2,
            pass_threshold: 1.0,
            thresholds: Some(DiagnosisThresholds {
                max_iterations: 100,
                max_tool_failures: 10,
                ..Default::default()
            }),
            test_cases: vec![TestCase {
                name: "must_contain_expected".into(),
                messages: vec![serde_json::json!({"role": "user", "content": "ping"})],
                expected_contains: Some(vec!["MAGIC_EXPECTED_SUBSTRING".into()]),
                expected_not_contains: None,
                max_latency_ms: None,
            }],
            ..Default::default()
        };

        let runner: std::sync::Arc<dyn SandboxRunner> = std::sync::Arc::new(
            DirectSandboxRunner::new(std::sync::Arc::new(MockWrongOutputSandboxBackend)),
        );

        let engine = SelfIterEngine::new(config).with_sandbox(runner);

        let trace = ExecutionTrace {
            agent_id: "agent-test".into(),
            session_id: "sess-1".into(),
            iterations: 2,
            tool_calls: vec![
                ToolCallTrace {
                    tool_name: "http_fetch".into(),
                    success: false,
                    latency_ms: 5000,
                    error: Some("timeout".into()),
                },
                ToolCallTrace {
                    tool_name: "http_fetch".into(),
                    success: false,
                    latency_ms: 2,
                    error: Some("connection refused".into()),
                },
            ],
            output: Some("adequate output here".into()),
            latency_ms: 1000,
            estimated_cost: 0.01,
        };

        let result = engine
            .iterate("agent-test", "base prompt", &trace)
            .await;

        assert_ne!(result.status, IterationStatus::DiagnosisOnly);
        assert!(
            result
                .diagnoses
                .iter()
                .any(|d| d.kind == DiagnosisKind::ToolCallFailure),
            "expected ToolCallFailure from consecutive tool errors"
        );

        let timeout_fix_snippet = "narrow the request";
        let conn_fix_snippet = "firewalls allow";
        let final_prompt = result
            .final_prompt
            .as_ref()
            .expect("iterate with sandbox and test cases should set final_prompt");

        assert!(
            final_prompt.contains(timeout_fix_snippet),
            "final_prompt should embed timeout diagnosis suggested fix; got:\n{final_prompt}"
        );
        assert!(
            final_prompt.contains(conn_fix_snippet),
            "final_prompt should embed connection-refused diagnosis suggested fix; got:\n{final_prompt}"
        );
    }

    #[test]
    fn format_recovery_guidance_matches_diagnosis_suggested_fixes() {
        let fix_alpha = "CUSTOM_FIX_ALPHA: retry with smaller batches";
        let fix_beta = "CUSTOM_FIX_BETA: verify TLS and SNI";
        let diagnoses = vec![
            Diagnosis {
                kind: DiagnosisKind::ToolCallFailure,
                description: "tool 'alpha' failed".into(),
                severity: Severity::Warning,
                suggested_fix: Some(fix_alpha.into()),
                context: serde_json::json!({}),
            },
            Diagnosis {
                kind: DiagnosisKind::ToolCallFailure,
                description: "tool 'beta' failed".into(),
                severity: Severity::Warning,
                suggested_fix: Some(fix_beta.into()),
                context: serde_json::json!({}),
            },
        ];

        let guidance = SelfIterEngine::format_recovery_guidance(&diagnoses)
            .expect("format_recovery_guidance should list suggested_fix lines");

        assert!(guidance.contains(fix_alpha), "guidance:\n{guidance}");
        assert!(guidance.contains(fix_beta), "guidance:\n{guidance}");
    }

    #[tokio::test]
    async fn iterate_with_runner_executes_tests_not_diagnosis_only() {
        let config = IterationConfig {
            max_rounds: 2,
            pass_threshold: 0.8,
            test_cases: vec![TestCase {
                name: "smoke".into(),
                messages: vec![serde_json::json!({"role": "user", "content": "ping"})],
                expected_contains: Some(vec!["SANDBOX_OK".into()]),
                expected_not_contains: None,
                max_latency_ms: None,
            }],
            ..Default::default()
        };

        let runner: std::sync::Arc<dyn SandboxRunner> = std::sync::Arc::new(
            DirectSandboxRunner::new(std::sync::Arc::new(MockSandboxBackend)),
        );

        let engine = SelfIterEngine::new(config).with_sandbox(runner);

        let trace = ExecutionTrace {
            agent_id: "test".into(),
            session_id: "s1".into(),
            iterations: 3,
            tool_calls: vec![],
            output: Some("adequate output here".into()),
            latency_ms: 1000,
            estimated_cost: 0.01,
        };

        let result = engine.iterate("test", "you are helpful", &trace).await;

        assert_ne!(result.status, IterationStatus::DiagnosisOnly);
        assert_eq!(result.status, IterationStatus::Passed);
        assert_eq!(result.rounds_executed, 1);
        assert_eq!(result.test_results.len(), 1);
        assert!(result.test_results[0].passed);
    }

    #[test]
    fn diagnosis_only_without_sandbox() {
        let config = IterationConfig::default();
        let engine = SelfIterEngine::new(config);

        let trace = ExecutionTrace {
            agent_id: "test".into(),
            session_id: "s1".into(),
            iterations: 15,
            tool_calls: vec![],
            output: Some("ok".into()),
            latency_ms: 1000,
            estimated_cost: 0.01,
        };

        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(engine.iterate("test", "you are helpful", &trace));

        assert_eq!(result.status, IterationStatus::DiagnosisOnly);
        assert!(result
            .diagnoses
            .iter()
            .any(|d| d.kind == crate::diagnosis::DiagnosisKind::LoopDetected));
    }

    #[test]
    fn generate_fix_hint_output() {
        let tr = TestCaseResult {
            name: "greeting".into(),
            passed: false,
            output: "hi".into(),
            latency_ms: 100,
            failure_reason: Some("output missing expected: 'hello'".into()),
        };
        let hint = generate_fix_hint(&[&tr], &[]);
        assert!(hint.contains("greeting"));
    }
}
