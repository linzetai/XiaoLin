use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use xiaolin_protocol::event::AgentEvent;
use xiaolin_protocol::usage::TokenUsage;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RunMetrics {
    pub duration_ms: u64,
    pub iterations: u32,
    pub tool_calls_total: u32,
    pub tool_calls_success: u32,
    pub tool_calls_failed: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_usage: Option<TokenUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u32>,
    pub tool_calls_by_name: HashMap<String, ToolCallStats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_end_reason: Option<String>,
    /// Per-iteration breakdown for detailed analysis.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub iteration_details: Vec<IterationDetail>,
}

/// Per-iteration metrics snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IterationDetail {
    pub iteration: u32,
    pub tool_calls: Vec<ToolCallRecord>,
    pub cumulative_prompt_tokens: u32,
    pub cumulative_completion_tokens: u32,
    pub cumulative_total_tokens: u32,
    pub context_used_tokens: u32,
    pub context_limit_tokens: u32,
    pub context_compressed: bool,
}

/// Record of a single tool call within an iteration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRecord {
    pub tool_name: String,
    pub success: bool,
    pub call_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolCallStats {
    pub total: u32,
    pub success: u32,
    pub failed: u32,
}

impl RunMetrics {
    pub fn tool_success_rate(&self) -> f64 {
        if self.tool_calls_total == 0 {
            return 1.0;
        }
        f64::from(self.tool_calls_success) / f64::from(self.tool_calls_total)
    }

    pub fn tool_error_rate(&self) -> f64 {
        if self.tool_calls_total == 0 {
            return 0.0;
        }
        f64::from(self.tool_calls_failed) / f64::from(self.tool_calls_total)
    }

    pub fn avg_tokens_per_iteration(&self) -> f64 {
        if self.iterations == 0 {
            return 0.0;
        }
        let total = self.token_usage.as_ref().map_or(0, |u| u.total_tokens);
        f64::from(total) / f64::from(self.iterations)
    }
}

pub struct MetricsCollector {
    metrics: RunMetrics,
    assistant_text: String,
    tool_names_used: Vec<String>,
    current_iteration: u32,
    current_iter_tools: Vec<ToolCallRecord>,
    cumulative_prompt: u32,
    cumulative_completion: u32,
    cumulative_total: u32,
    last_context_used: u32,
    last_context_limit: u32,
    last_context_compressed: bool,
}

impl MetricsCollector {
    pub fn new() -> Self {
        Self {
            metrics: RunMetrics::default(),
            assistant_text: String::new(),
            tool_names_used: Vec::new(),
            current_iteration: 0,
            current_iter_tools: Vec::new(),
            cumulative_prompt: 0,
            cumulative_completion: 0,
            cumulative_total: 0,
            last_context_used: 0,
            last_context_limit: 0,
            last_context_compressed: false,
        }
    }

    pub fn process_event(&mut self, event: &AgentEvent) {
        match event {
            AgentEvent::IterationBoundary { iteration, .. } => {
                if self.current_iteration > 0 {
                    self.flush_iteration();
                }
                self.current_iteration = *iteration;
                self.current_iter_tools.clear();
            }
            AgentEvent::ToolExecuting { .. } => {}
            AgentEvent::ToolResult {
                tool_name,
                success,
                call_id,
                ..
            } => {
                self.metrics.tool_calls_total += 1;
                if *success {
                    self.metrics.tool_calls_success += 1;
                } else {
                    self.metrics.tool_calls_failed += 1;
                }
                let entry = self
                    .metrics
                    .tool_calls_by_name
                    .entry(tool_name.clone())
                    .or_default();
                entry.total += 1;
                if *success {
                    entry.success += 1;
                } else {
                    entry.failed += 1;
                }
                self.tool_names_used.push(tool_name.clone());
                self.current_iter_tools.push(ToolCallRecord {
                    tool_name: tool_name.clone(),
                    success: *success,
                    call_id: call_id.clone(),
                });
            }
            AgentEvent::ContentDelta { delta, .. } => {
                if let Some(text) = delta
                    .get("choices")
                    .and_then(|c| c.get(0))
                    .and_then(|c| c.get("delta"))
                    .and_then(|d| d.get("content"))
                    .and_then(|t| t.as_str())
                {
                    self.assistant_text.push_str(text);
                }
                if let Some(usage) = delta.get("usage") {
                    if let Some(pt) = usage.get("prompt_tokens").and_then(|v| v.as_u64()) {
                        self.cumulative_prompt = pt as u32;
                    }
                    if let Some(ct) = usage.get("completion_tokens").and_then(|v| v.as_u64()) {
                        self.cumulative_completion = ct as u32;
                    }
                    if let Some(tt) = usage.get("total_tokens").and_then(|v| v.as_u64()) {
                        self.cumulative_total = tt as u32;
                    }
                }
            }
            AgentEvent::ContextUsageUpdate {
                used_tokens,
                limit_tokens,
                compressed,
                ..
            } => {
                self.last_context_used = *used_tokens;
                self.last_context_limit = *limit_tokens;
                self.last_context_compressed = *compressed;
            }
            AgentEvent::TurnEnd {
                summary, reason, ..
            } => {
                if self.current_iteration > 0 {
                    self.flush_iteration();
                }
                self.metrics.duration_ms = summary.elapsed_ms;
                self.metrics.iterations = summary.iterations;
                self.metrics.tool_calls_total =
                    self.metrics.tool_calls_total.max(summary.tool_calls_made);
                self.metrics.token_usage.clone_from(&summary.usage);
                self.metrics.context_tokens = summary.context_tokens;
                self.metrics.context_window = summary.context_window;
                self.metrics.turn_end_reason.clone_from(reason);
            }
            _ => {}
        }
    }

    fn flush_iteration(&mut self) {
        let detail = IterationDetail {
            iteration: self.current_iteration,
            tool_calls: std::mem::take(&mut self.current_iter_tools),
            cumulative_prompt_tokens: self.cumulative_prompt,
            cumulative_completion_tokens: self.cumulative_completion,
            cumulative_total_tokens: self.cumulative_total,
            context_used_tokens: self.last_context_used,
            context_limit_tokens: self.last_context_limit,
            context_compressed: self.last_context_compressed,
        };
        self.metrics.iteration_details.push(detail);
    }

    pub fn finalize(self) -> CollectedResult {
        CollectedResult {
            metrics: self.metrics,
            assistant_text: self.assistant_text,
            tool_names_used: self.tool_names_used,
        }
    }
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct CollectedResult {
    pub metrics: RunMetrics,
    pub assistant_text: String,
    pub tool_names_used: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use xiaolin_protocol::event::TurnSummary;
    use xiaolin_protocol::TurnId;

    fn make_turn_id() -> TurnId {
        TurnId::new("test-turn")
    }

    #[test]
    fn collect_tool_results() {
        let mut collector = MetricsCollector::new();
        collector.process_event(&AgentEvent::ToolResult {
            turn_id: make_turn_id(),
            tool_name: "read_file".into(),
            call_id: "c1".into(),
            output: "content".into(),
            display_output: None,
            success: true,
            metadata: None,
        });
        collector.process_event(&AgentEvent::ToolResult {
            turn_id: make_turn_id(),
            tool_name: "edit_file".into(),
            call_id: "c2".into(),
            output: "error".into(),
            display_output: None,
            success: false,
            metadata: None,
        });
        let result = collector.finalize();
        assert_eq!(result.metrics.tool_calls_total, 2);
        assert_eq!(result.metrics.tool_calls_success, 1);
        assert_eq!(result.metrics.tool_calls_failed, 1);
        assert_eq!(result.metrics.tool_calls_by_name["read_file"].success, 1);
        assert_eq!(result.metrics.tool_calls_by_name["edit_file"].failed, 1);
        assert!((result.metrics.tool_success_rate() - 0.5).abs() < f64::EPSILON);
        assert!((result.metrics.tool_error_rate() - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn collect_turn_end() {
        let mut collector = MetricsCollector::new();
        collector.process_event(&AgentEvent::TurnEnd {
            turn_id: make_turn_id(),
            summary: TurnSummary {
                turn_id: make_turn_id(),
                tool_calls_made: 5,
                iterations: 3,
                usage: Some(TokenUsage {
                    prompt_tokens: 1000,
                    completion_tokens: 500,
                    total_tokens: 1500,
                    cached_input_tokens: 200,
                }),
                elapsed_ms: 5000,
                context_tokens: Some(4000),
                context_window: Some(128_000),
            },
            session_id: None,
            final_tool_calls: None,
            reason: Some("completed".into()),
        });
        let result = collector.finalize();
        assert_eq!(result.metrics.duration_ms, 5000);
        assert_eq!(result.metrics.iterations, 3);
        assert_eq!(
            result.metrics.token_usage.as_ref().unwrap().total_tokens,
            1500
        );
        assert_eq!(result.metrics.turn_end_reason.as_deref(), Some("completed"));
        assert!((result.metrics.avg_tokens_per_iteration() - 500.0).abs() < f64::EPSILON);
    }

    #[test]
    fn iteration_tracking() {
        let mut collector = MetricsCollector::new();

        collector.process_event(&AgentEvent::IterationBoundary {
            turn_id: make_turn_id(),
            iteration: 1,
        });
        collector.process_event(&AgentEvent::ToolResult {
            turn_id: make_turn_id(),
            tool_name: "read_file".into(),
            call_id: "c1".into(),
            output: "ok".into(),
            display_output: None,
            success: true,
            metadata: None,
        });

        collector.process_event(&AgentEvent::IterationBoundary {
            turn_id: make_turn_id(),
            iteration: 2,
        });
        collector.process_event(&AgentEvent::ToolResult {
            turn_id: make_turn_id(),
            tool_name: "edit_file".into(),
            call_id: "c2".into(),
            output: "ok".into(),
            display_output: None,
            success: true,
            metadata: None,
        });

        collector.process_event(&AgentEvent::TurnEnd {
            turn_id: make_turn_id(),
            summary: TurnSummary {
                turn_id: make_turn_id(),
                tool_calls_made: 2,
                iterations: 2,
                usage: None,
                elapsed_ms: 3000,
                context_tokens: None,
                context_window: None,
            },
            session_id: None,
            final_tool_calls: None,
            reason: None,
            diagnosis: None,
            plan_outcome: None,
        });

        let result = collector.finalize();
        assert_eq!(result.metrics.iteration_details.len(), 2);
        assert_eq!(result.metrics.iteration_details[0].iteration, 1);
        assert_eq!(result.metrics.iteration_details[0].tool_calls.len(), 1);
        assert_eq!(
            result.metrics.iteration_details[0].tool_calls[0].tool_name,
            "read_file"
        );
        assert_eq!(result.metrics.iteration_details[1].iteration, 2);
        assert_eq!(result.metrics.iteration_details[1].tool_calls.len(), 1);
    }
}
