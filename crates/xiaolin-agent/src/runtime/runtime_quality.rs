use xiaolin_core::agent_config::AgentConfig;
use xiaolin_core::types::Usage;
use xiaolin_protocol::{TurnId, TurnQualityDiagnosisCode, TurnQualitySeverity, TurnQualitySummary};
use xiaolin_session::RuntimeQualityStore;

use super::query_state::QueryLoopState;

#[derive(Debug, Clone)]
pub(crate) struct ToolQualitySample {
    pub name: String,
    pub success: bool,
    pub duration_ms: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct RuntimeQualityCollector {
    started_at: chrono::DateTime<chrono::Utc>,
    first_delta_ms: Option<u64>,
    first_content_ms: Option<u64>,
    first_reasoning_ms: Option<u64>,
    first_tool_ms: Option<u64>,
    cache_read_tokens: u32,
    cache_creation_tokens: u32,
    estimated_cost_usd: Option<f64>,
    compact_count: u32,
    tokens_saved: Option<i64>,
    tool_samples: Vec<ToolQualitySample>,
}

impl RuntimeQualityCollector {
    pub fn new() -> Self {
        Self {
            started_at: chrono::Utc::now(),
            first_delta_ms: None,
            first_content_ms: None,
            first_reasoning_ms: None,
            first_tool_ms: None,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            estimated_cost_usd: None,
            compact_count: 0,
            tokens_saved: None,
            tool_samples: Vec::new(),
        }
    }

    pub fn mark_delta(&mut self, elapsed_ms: u64) {
        self.first_delta_ms.get_or_insert(elapsed_ms);
    }

    pub fn mark_content(&mut self, elapsed_ms: u64) {
        self.first_content_ms.get_or_insert(elapsed_ms);
    }

    pub fn mark_reasoning(&mut self, elapsed_ms: u64) {
        self.first_reasoning_ms.get_or_insert(elapsed_ms);
    }

    pub fn mark_tool(&mut self, elapsed_ms: u64) {
        self.first_tool_ms.get_or_insert(elapsed_ms);
    }

    pub fn record_usage(&mut self, usage: &Usage, estimated_cost_usd: Option<f64>) {
        self.cache_read_tokens = self
            .cache_read_tokens
            .saturating_add(usage.effective_cache_read_tokens());
        self.cache_creation_tokens = self
            .cache_creation_tokens
            .saturating_add(usage.effective_cache_creation_tokens());
        if let Some(cost) = estimated_cost_usd {
            let total = self.estimated_cost_usd.unwrap_or(0.0) + cost;
            self.estimated_cost_usd = Some(total);
        }
    }

    pub fn record_tool(&mut self, name: &str, success: bool, duration_ms: u64) {
        self.tool_samples.push(ToolQualitySample {
            name: name.to_string(),
            success,
            duration_ms,
        });
    }

    #[allow(dead_code)]
    pub fn record_compact(&mut self, pre_tokens: usize, post_tokens: usize) {
        self.compact_count = self.compact_count.saturating_add(1);
        let saved = pre_tokens as i64 - post_tokens as i64;
        self.tokens_saved = Some(self.tokens_saved.unwrap_or(0) + saved);
    }

    #[allow(dead_code)]
    pub fn build_summary(
        &self,
        session_id: &str,
        turn_id: &TurnId,
        config: &AgentConfig,
        model: &str,
        stream_start: std::time::Instant,
        context_window: u32,
        state: &QueryLoopState,
    ) -> TurnQualitySummary {
        self.build_summary_with_diagnosis_override(
            session_id,
            turn_id,
            config,
            model,
            stream_start,
            context_window,
            state,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn build_summary_with_diagnosis_override(
        &self,
        session_id: &str,
        turn_id: &TurnId,
        config: &AgentConfig,
        model: &str,
        stream_start: std::time::Instant,
        context_window: u32,
        state: &QueryLoopState,
        diagnosis_override: Option<(TurnQualityDiagnosisCode, TurnQualitySeverity)>,
    ) -> TurnQualitySummary {
        let ended_at = chrono::Utc::now();
        let elapsed_ms = stream_start.elapsed().as_millis() as u64;
        let usage = state.build_usage();
        let input_tokens = usage.as_ref().map_or(0, |u| u.prompt_tokens);
        let output_tokens = usage.as_ref().map_or(0, |u| u.completion_tokens);
        let context_tokens = Some(state.last_estimated_tokens as u32);
        let context_window = Some(context_window);
        let context_usage_pct = context_tokens
            .zip(context_window)
            .and_then(|(tokens, window)| {
                (window > 0).then_some((tokens as f64 / window as f64) * 100.0)
            });
        let cache_hit_pct = (input_tokens > 0)
            .then_some((self.cache_read_tokens as f64 / input_tokens as f64) * 100.0);
        let (repeated_tool_warn_count, repeated_tool_force_stop_count) = state.repetition_stats();
        let tool_failures_total = self.tool_samples.iter().filter(|s| !s.success).count() as u32;
        let tool_time_ms_total = self.tool_samples.iter().map(|s| s.duration_ms).sum::<u64>();
        let slowest = self.tool_samples.iter().max_by_key(|s| s.duration_ms);
        let compact_count = self.compact_count;

        let diagnosis_input = DiagnosisInput {
            first_delta_ms: self.first_delta_ms,
            tool_failures_total,
            tool_calls_total: state.total_tool_calls,
            slowest_tool_ms: slowest.map(|s| s.duration_ms),
            repeated_tool_warn_count,
            repeated_tool_force_stop_count,
            context_usage_pct,
            estimated_cost_usd: self.estimated_cost_usd,
            cache_hit_pct,
            iterations: state.iteration,
        };
        let (diagnosis_code, severity) =
            diagnosis_override.unwrap_or_else(|| diagnose(&diagnosis_input));
        let evidence_json = serde_json::json!({
            "elapsedMs": elapsed_ms,
            "firstDeltaMs": self.first_delta_ms,
            "toolCallsTotal": state.total_tool_calls,
            "toolFailuresTotal": tool_failures_total,
            "slowestToolMs": slowest.map(|s| s.duration_ms),
            "repeatedToolWarnCount": repeated_tool_warn_count,
            "repeatedToolForceStopCount": repeated_tool_force_stop_count,
            "contextUsagePct": context_usage_pct,
            "estimatedCostUsd": self.estimated_cost_usd,
            "cacheHitPct": cache_hit_pct,
            "iterations": state.iteration,
            "compactCount": compact_count,
            "flags": {
                "hasToolFailures": tool_failures_total > 0,
                "hasToolLoopWarning": repeated_tool_warn_count > 0,
                "hasToolLoopForceStop": repeated_tool_force_stop_count > 0,
                "highContext": context_usage_pct.is_some_and(|pct| pct >= HIGH_CONTEXT_USAGE_PCT),
                "lowCacheHit": cache_hit_pct.is_some_and(|pct| pct <= LOW_CACHE_HIT_PCT),
                "manyIterations": state.iteration >= MANY_ITERATIONS_THRESHOLD,
            },
        });

        TurnQualitySummary {
            session_id: session_id.to_string(),
            turn_id: turn_id.to_string(),
            agent_id: config.agent_id.to_string(),
            model_provider: config.model.provider.clone(),
            model: model.to_string(),
            started_at: self.started_at.to_rfc3339(),
            ended_at: ended_at.to_rfc3339(),
            elapsed_ms,
            first_delta_ms: self.first_delta_ms,
            first_content_ms: self.first_content_ms,
            first_reasoning_ms: self.first_reasoning_ms,
            first_tool_ms: self.first_tool_ms,
            iterations: state.iteration,
            tool_calls_total: state.total_tool_calls,
            tool_failures_total,
            tool_time_ms_total,
            slowest_tool_name: slowest.map(|s| s.name.clone()),
            slowest_tool_ms: slowest.map(|s| s.duration_ms),
            repeated_tool_warn_count,
            repeated_tool_force_stop_count,
            input_tokens,
            output_tokens,
            cache_read_tokens: self.cache_read_tokens,
            cache_creation_tokens: self.cache_creation_tokens,
            cache_hit_pct,
            estimated_cost_usd: self.estimated_cost_usd,
            context_tokens,
            context_window,
            context_usage_pct,
            compressed: compact_count > 0,
            tokens_saved: self.tokens_saved,
            compact_count,
            diagnosis_code,
            severity,
            evidence_json,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn persist_runtime_quality_summary(
    store: Option<&RuntimeQualityStore>,
    session_id: Option<&str>,
    collector: &RuntimeQualityCollector,
    turn_id: &TurnId,
    config: &AgentConfig,
    model: &str,
    stream_start: std::time::Instant,
    context_window: u32,
    state: &QueryLoopState,
    diagnosis_override: Option<(TurnQualityDiagnosisCode, TurnQualitySeverity)>,
) -> anyhow::Result<Option<TurnQualitySummary>> {
    let (Some(store), Some(session_id)) = (store, session_id) else {
        return Ok(None);
    };
    let summary = collector.build_summary_with_diagnosis_override(
        session_id,
        turn_id,
        config,
        model,
        stream_start,
        context_window,
        state,
        diagnosis_override,
    );
    store.upsert_summary(&summary).await?;
    Ok(Some(summary))
}

#[derive(Debug, Clone, Copy)]
struct DiagnosisInput {
    first_delta_ms: Option<u64>,
    tool_failures_total: u32,
    tool_calls_total: u32,
    slowest_tool_ms: Option<u64>,
    repeated_tool_warn_count: u32,
    repeated_tool_force_stop_count: u32,
    context_usage_pct: Option<f64>,
    estimated_cost_usd: Option<f64>,
    cache_hit_pct: Option<f64>,
    iterations: u32,
}

const SLOW_FIRST_DELTA_MS: u64 = 15_000;
const SLOW_TOOL_MS: u64 = 10_000;
const HIGH_CONTEXT_USAGE_PCT: f64 = 90.0;
const HIGH_COST_USD: f64 = 1.0;
const LOW_CACHE_HIT_PCT: f64 = 5.0;
const MANY_ITERATIONS_THRESHOLD: u32 = 20;

fn diagnose(input: &DiagnosisInput) -> (TurnQualityDiagnosisCode, TurnQualitySeverity) {
    if input.repeated_tool_force_stop_count > 0 {
        return (
            TurnQualityDiagnosisCode::ToolLoop,
            TurnQualitySeverity::Error,
        );
    }
    if input.repeated_tool_warn_count > 0 {
        return (
            TurnQualityDiagnosisCode::ToolLoop,
            TurnQualitySeverity::Warn,
        );
    }
    if input.tool_calls_total > 0 && input.tool_failures_total == input.tool_calls_total {
        return (
            TurnQualityDiagnosisCode::ToolFailures,
            TurnQualitySeverity::Error,
        );
    }
    if input.tool_failures_total > 0 {
        return (
            TurnQualityDiagnosisCode::ToolFailures,
            TurnQualitySeverity::Warn,
        );
    }
    if input.slowest_tool_ms.is_some_and(|ms| ms >= SLOW_TOOL_MS) {
        return (
            TurnQualityDiagnosisCode::SlowTools,
            TurnQualitySeverity::Warn,
        );
    }
    if input
        .first_delta_ms
        .is_some_and(|ms| ms >= SLOW_FIRST_DELTA_MS)
    {
        return (
            TurnQualityDiagnosisCode::SlowFirstToken,
            TurnQualitySeverity::Warn,
        );
    }
    if input
        .context_usage_pct
        .is_some_and(|pct| pct >= HIGH_CONTEXT_USAGE_PCT)
    {
        return (
            TurnQualityDiagnosisCode::HighContext,
            TurnQualitySeverity::Warn,
        );
    }
    if input
        .estimated_cost_usd
        .is_some_and(|cost| cost >= HIGH_COST_USD)
    {
        return (
            TurnQualityDiagnosisCode::HighCost,
            TurnQualitySeverity::Warn,
        );
    }
    if input
        .cache_hit_pct
        .is_some_and(|pct| pct <= LOW_CACHE_HIT_PCT)
    {
        return (
            TurnQualityDiagnosisCode::CacheMiss,
            TurnQualitySeverity::Warn,
        );
    }
    if input.iterations >= MANY_ITERATIONS_THRESHOLD {
        return (
            TurnQualityDiagnosisCode::ManyIterations,
            TurnQualitySeverity::Warn,
        );
    }
    (TurnQualityDiagnosisCode::Normal, TurnQualitySeverity::Info)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    #[test]
    fn diagnosis_prioritizes_tool_loop_force_stop() {
        let input = DiagnosisInput {
            first_delta_ms: Some(20_000),
            tool_failures_total: 0,
            tool_calls_total: 5,
            slowest_tool_ms: Some(20_000),
            repeated_tool_warn_count: 1,
            repeated_tool_force_stop_count: 1,
            context_usage_pct: Some(99.0),
            estimated_cost_usd: Some(2.0),
            cache_hit_pct: Some(0.0),
            iterations: 30,
        };

        let (code, severity) = diagnose(&input);
        assert_eq!(code, TurnQualityDiagnosisCode::ToolLoop);
        assert_eq!(severity, TurnQualitySeverity::Error);
    }

    #[test]
    fn diagnosis_marks_all_tool_failures_as_error() {
        let input = DiagnosisInput {
            first_delta_ms: Some(100),
            tool_failures_total: 3,
            tool_calls_total: 3,
            slowest_tool_ms: Some(100),
            repeated_tool_warn_count: 0,
            repeated_tool_force_stop_count: 0,
            context_usage_pct: Some(10.0),
            estimated_cost_usd: Some(0.01),
            cache_hit_pct: Some(100.0),
            iterations: 1,
        };

        let (code, severity) = diagnose(&input);
        assert_eq!(code, TurnQualityDiagnosisCode::ToolFailures);
        assert_eq!(severity, TurnQualitySeverity::Error);
    }

    #[test]
    fn diagnosis_marks_cache_miss_without_llm() {
        let input = DiagnosisInput {
            first_delta_ms: Some(100),
            tool_failures_total: 0,
            tool_calls_total: 0,
            slowest_tool_ms: None,
            repeated_tool_warn_count: 0,
            repeated_tool_force_stop_count: 0,
            context_usage_pct: Some(10.0),
            estimated_cost_usd: Some(0.01),
            cache_hit_pct: Some(0.0),
            iterations: 1,
        };

        let (code, severity) = diagnose(&input);
        assert_eq!(code, TurnQualityDiagnosisCode::CacheMiss);
        assert_eq!(severity, TurnQualitySeverity::Warn);
    }

    #[test]
    fn collector_aggregates_tools_without_prompt_or_output_bodies() {
        let mut collector = RuntimeQualityCollector::new();
        collector.mark_delta(100);
        collector.mark_content(150);
        collector.record_tool("read_file", false, SLOW_TOOL_MS + 1);
        collector.record_tool("shell_exec", true, 50);

        let mut state = QueryLoopState::new(10);
        state.iteration = 2;
        state.total_tool_calls = 2;
        state.acc_prompt_tokens = 100;
        state.acc_completion_tokens = 25;
        let config: AgentConfig = serde_json::from_value(serde_json::json!({
            "agentId": "default"
        }))
        .unwrap();
        let summary = collector.build_summary(
            "session-1",
            &TurnId::new("turn-1"),
            &config,
            "model-test",
            std::time::Instant::now(),
            1000,
            &state,
        );

        assert_eq!(summary.tool_calls_total, 2);
        assert_eq!(summary.tool_failures_total, 1);
        assert_eq!(summary.slowest_tool_name.as_deref(), Some("read_file"));
        assert_eq!(
            summary.diagnosis_code,
            TurnQualityDiagnosisCode::ToolFailures
        );
        let evidence = summary.evidence_json.to_string();
        assert!(!evidence.contains("user prompt"));
        assert!(!evidence.contains("tool output"));
        assert!(!evidence.contains("arguments"));
    }

    #[tokio::test]
    async fn synthetic_completed_turn_persists_one_runtime_quality_row() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let store = RuntimeQualityStore::open(pool).await.unwrap();

        let mut collector = RuntimeQualityCollector::new();
        collector.mark_delta(25);
        collector.mark_content(30);
        collector.mark_tool(75);
        collector.record_tool("read_file", true, 40);
        collector.record_usage(
            &Usage {
                prompt_tokens: 120,
                completion_tokens: 35,
                total_tokens: 155,
                cache_read_tokens: 120,
                ..Default::default()
            },
            None,
        );

        let mut state = QueryLoopState::new(10);
        state.iteration = 1;
        state.total_tool_calls = 1;
        state.acc_prompt_tokens = 120;
        state.acc_completion_tokens = 35;
        state.last_estimated_tokens = 250;

        let config: AgentConfig = serde_json::from_value(serde_json::json!({
            "agentId": "main",
            "model": {
                "provider": "openai"
            }
        }))
        .unwrap();
        let turn_id = TurnId::new("turn-runtime-quality-synthetic");

        let persisted = persist_runtime_quality_summary(
            Some(&store),
            Some("session-runtime-quality-synthetic"),
            &collector,
            &turn_id,
            &config,
            "model-test",
            std::time::Instant::now(),
            1_000,
            &state,
            None,
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(persisted.session_id, "session-runtime-quality-synthetic");
        assert_eq!(persisted.turn_id, "turn-runtime-quality-synthetic");
        assert_eq!(persisted.diagnosis_code, TurnQualityDiagnosisCode::Normal);

        let rows = store
            .query_session("session-runtime-quality-synthetic", 10)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        let row = store
            .get_turn(
                "session-runtime-quality-synthetic",
                "turn-runtime-quality-synthetic",
            )
            .await
            .unwrap()
            .unwrap();

        assert_eq!(row.iterations, 1);
        assert_eq!(row.tool_calls_total, 1);
        assert_eq!(row.tool_failures_total, 0);
        assert_eq!(row.slowest_tool_name.as_deref(), Some("read_file"));
        assert_eq!(row.first_delta_ms, Some(25));
        assert_eq!(row.first_content_ms, Some(30));
        assert_eq!(row.first_tool_ms, Some(75));
        assert_eq!(row.input_tokens, 120);
        assert_eq!(row.output_tokens, 35);
        assert_eq!(row.context_tokens, Some(250));
        assert_eq!(row.context_window, Some(1_000));
    }
}
