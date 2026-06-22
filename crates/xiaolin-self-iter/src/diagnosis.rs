use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnosis {
    pub kind: DiagnosisKind,
    pub description: String,
    pub severity: Severity,
    pub suggested_fix: Option<String>,
    pub context: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosisKind {
    ToolCallFailure,
    LoopDetected,
    OutputQualityLow,
    ContextOverflow,
    LatencySpike,
    CostOverrun,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Error,
    Critical,
}

/// Analyzes agent execution traces to produce diagnoses.
pub struct Diagnostician {
    thresholds: DiagnosisThresholds,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosisThresholds {
    pub max_iterations: u32,
    pub max_tool_failures: u32,
    pub max_latency_ms: u64,
    pub max_cost_per_request: f64,
    pub min_output_length: usize,
}

impl Default for DiagnosisThresholds {
    fn default() -> Self {
        Self {
            max_iterations: 10,
            max_tool_failures: 3,
            max_latency_ms: 30_000,
            max_cost_per_request: 1.0,
            min_output_length: 10,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionTrace {
    pub agent_id: String,
    pub session_id: String,
    pub iterations: u32,
    pub tool_calls: Vec<ToolCallTrace>,
    pub output: Option<String>,
    pub latency_ms: u64,
    pub estimated_cost: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallTrace {
    pub tool_name: String,
    pub success: bool,
    pub latency_ms: u64,
    pub error: Option<String>,
}

/// Heuristic remediation text derived from common tool error substrings.
pub fn suggest_fix_from_error_message(error: &str) -> String {
    let e = error.to_lowercase();
    if e.contains("timeout") || e.contains("timed out") {
        return "The tool timed out: narrow the request, retry once, or verify the backing service and network are reachable.".into();
    }
    if e.contains("connection refused")
        || e.contains("econnrefused")
        || e.contains("could not connect")
    {
        return "Connection was refused: confirm the target host/port is up, firewalls allow traffic, and any required daemon is running.".into();
    }
    if e.contains("dns") || e.contains("nxdomain") || e.contains("name or service not known") {
        return "Name resolution failed: check the hostname spelling, DNS configuration, and VPN/proxy settings.".into();
    }
    if e.contains("404") || e.contains("not found") {
        return "A resource was not found: verify paths, IDs, URLs, and repository or file names before retrying.".into();
    }
    if e.contains("401")
        || e.contains("403")
        || e.contains("unauthorized")
        || e.contains("forbidden")
    {
        return "Authentication or authorization failed: refresh credentials, API keys, or OAuth tokens and confirm required scopes.".into();
    }
    if e.contains("json")
        || e.contains("parse")
        || e.contains("serde")
        || e.contains("unexpected token")
    {
        return "Arguments or response JSON may be invalid: validate the schema, escape strings properly, and ensure required fields are present.".into();
    }
    if e.contains("rate limit") || e.contains("429") || e.contains("too many requests") {
        return "Rate limited: back off, reduce parallelism, or wait before calling the same tool again.".into();
    }
    if e.contains("tool not found") || e.contains("unknown tool") {
        return "The tool name is not registered: use an available tool from the tools list and match the name exactly.".into();
    }
    format!(
        "Review tool arguments, environment, and dependencies. Raw error (truncated): {}",
        error.chars().take(280).collect::<String>()
    )
}

impl Diagnostician {
    pub fn new(thresholds: DiagnosisThresholds) -> Self {
        Self { thresholds }
    }

    pub fn diagnose(&self, trace: &ExecutionTrace) -> Vec<Diagnosis> {
        let mut diagnoses = Vec::new();

        // Per-failed-call hints (supports short streaks in the agent tool loop).
        for tc in trace.tool_calls.iter().filter(|t| !t.success) {
            let suggested_fix = tc
                .error
                .as_ref()
                .map(|msg| suggest_fix_from_error_message(msg))
                .unwrap_or_else(|| {
                    "Verify tool arguments, permissions, and that required services are reachable."
                        .into()
                });
            let description = match &tc.error {
                Some(err) => format!("Tool '{}' failed: {}", tc.tool_name, err),
                None => format!(
                    "Tool '{}' reported failure with no error message",
                    tc.tool_name
                ),
            };
            diagnoses.push(Diagnosis {
                kind: DiagnosisKind::ToolCallFailure,
                description,
                severity: Severity::Warning,
                suggested_fix: Some(suggested_fix),
                context: serde_json::json!({
                    "tool": tc.tool_name,
                    "error": tc.error,
                    "latency_ms": tc.latency_ms,
                }),
            });
        }

        let tool_failures: Vec<_> = trace.tool_calls.iter().filter(|t| !t.success).collect();

        if tool_failures.len() as u32 > self.thresholds.max_tool_failures {
            let failed_tools: Vec<_> = tool_failures.iter().map(|t| t.tool_name.clone()).collect();
            diagnoses.push(Diagnosis {
                kind: DiagnosisKind::ToolCallFailure,
                description: format!(
                    "{} tool calls failed (threshold: {})",
                    tool_failures.len(),
                    self.thresholds.max_tool_failures
                ),
                severity: Severity::Error,
                suggested_fix: Some("Review tool configurations and input validation".into()),
                context: serde_json::json!({ "failed_tools": failed_tools }),
            });
        }

        if trace.iterations >= self.thresholds.max_iterations {
            diagnoses.push(Diagnosis {
                kind: DiagnosisKind::LoopDetected,
                description: format!(
                    "Agent reached {} iterations (max: {}), possible reasoning loop",
                    trace.iterations, self.thresholds.max_iterations
                ),
                severity: Severity::Warning,
                suggested_fix: Some(
                    "Add loop-breaking instructions to system prompt or reduce max_iterations"
                        .into(),
                ),
                context: serde_json::json!({ "iterations": trace.iterations }),
            });
        }

        if let Some(output) = &trace.output {
            let output_len = output.chars().count();
            if output_len < self.thresholds.min_output_length {
                diagnoses.push(Diagnosis {
                    kind: DiagnosisKind::OutputQualityLow,
                    description: format!(
                        "Output too short ({} chars, minimum: {})",
                        output_len,
                        self.thresholds.min_output_length
                    ),
                    severity: Severity::Warning,
                    suggested_fix: Some(
                        "Improve system prompt to encourage more detailed responses".into(),
                    ),
                    context: serde_json::json!({ "output_length": output_len }),
                });
            }
        }

        if trace.latency_ms > self.thresholds.max_latency_ms {
            diagnoses.push(Diagnosis {
                kind: DiagnosisKind::LatencySpike,
                description: format!(
                    "Request took {}ms (threshold: {}ms)",
                    trace.latency_ms, self.thresholds.max_latency_ms
                ),
                severity: Severity::Warning,
                suggested_fix: Some(
                    "Consider using a faster model or reducing context size".into(),
                ),
                context: serde_json::json!({ "latency_ms": trace.latency_ms }),
            });
        }

        if trace.estimated_cost > self.thresholds.max_cost_per_request {
            diagnoses.push(Diagnosis {
                kind: DiagnosisKind::CostOverrun,
                description: format!(
                    "Request cost ${:.4} (threshold: ${:.4})",
                    trace.estimated_cost, self.thresholds.max_cost_per_request
                ),
                severity: Severity::Error,
                suggested_fix: Some("Use a cheaper model or reduce context/output size".into()),
                context: serde_json::json!({ "cost": trace.estimated_cost }),
            });
        }

        // Detect tool call patterns that suggest loops
        if trace.tool_calls.len() >= 4 {
            let names: Vec<_> = trace.tool_calls.iter().map(|t| &t.tool_name).collect();
            if names.len() >= 4 {
                let last_two = &names[names.len() - 2..];
                let prev_two = &names[names.len() - 4..names.len() - 2];
                if last_two == prev_two {
                    diagnoses.push(Diagnosis {
                        kind: DiagnosisKind::LoopDetected,
                        description: "Repeated tool call pattern detected".into(),
                        severity: Severity::Warning,
                        suggested_fix: Some(
                            "The agent is calling the same tools repeatedly; consider adding break conditions".into(),
                        ),
                        context: serde_json::json!({
                            "pattern": last_two.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                        }),
                    });
                }
            }
        }

        diagnoses
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_tool_failures() {
        let diag = Diagnostician::new(DiagnosisThresholds {
            max_tool_failures: 1,
            ..Default::default()
        });

        let trace = ExecutionTrace {
            agent_id: "test".into(),
            session_id: "s1".into(),
            iterations: 2,
            tool_calls: vec![
                ToolCallTrace {
                    tool_name: "web_search".into(),
                    success: false,
                    latency_ms: 100,
                    error: Some("timeout".into()),
                },
                ToolCallTrace {
                    tool_name: "web_search".into(),
                    success: false,
                    latency_ms: 100,
                    error: Some("timeout".into()),
                },
            ],
            output: Some("Unable to complete search.".into()),
            latency_ms: 5000,
            estimated_cost: 0.01,
        };

        let results = diag.diagnose(&trace);
        assert!(results
            .iter()
            .any(|d| d.kind == DiagnosisKind::ToolCallFailure));
    }

    #[test]
    fn detect_loop() {
        let diag = Diagnostician::new(Default::default());

        let trace = ExecutionTrace {
            agent_id: "test".into(),
            session_id: "s1".into(),
            iterations: 10,
            tool_calls: vec![
                ToolCallTrace {
                    tool_name: "a".into(),
                    success: true,
                    latency_ms: 10,
                    error: None,
                },
                ToolCallTrace {
                    tool_name: "b".into(),
                    success: true,
                    latency_ms: 10,
                    error: None,
                },
                ToolCallTrace {
                    tool_name: "a".into(),
                    success: true,
                    latency_ms: 10,
                    error: None,
                },
                ToolCallTrace {
                    tool_name: "b".into(),
                    success: true,
                    latency_ms: 10,
                    error: None,
                },
            ],
            output: Some("looped output".into()),
            latency_ms: 5000,
            estimated_cost: 0.01,
        };

        let results = diag.diagnose(&trace);
        assert!(results
            .iter()
            .any(|d| d.kind == DiagnosisKind::LoopDetected));
    }

    #[test]
    fn suggest_fix_timeout() {
        let s = super::suggest_fix_from_error_message("request timeout after 30s");
        assert!(s.to_lowercase().contains("timed out") || s.to_lowercase().contains("timeout"));
    }

    #[test]
    fn detect_cost_overrun() {
        let diag = Diagnostician::new(DiagnosisThresholds {
            max_cost_per_request: 0.5,
            ..Default::default()
        });

        let trace = ExecutionTrace {
            agent_id: "test".into(),
            session_id: "s1".into(),
            iterations: 1,
            tool_calls: vec![],
            output: Some("expensive response".into()),
            latency_ms: 1000,
            estimated_cost: 1.5,
        };

        let results = diag.diagnose(&trace);
        assert!(results.iter().any(|d| d.kind == DiagnosisKind::CostOverrun));
    }
}
