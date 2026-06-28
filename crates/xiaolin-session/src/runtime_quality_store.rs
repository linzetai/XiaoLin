use sqlx::SqlitePool;
use xiaolin_protocol::{TurnQualityDiagnosisCode, TurnQualitySeverity, TurnQualitySummary};

/// Persists one runtime-quality summary per completed turn.
pub struct RuntimeQualityStore {
    pool: SqlitePool,
}

impl RuntimeQualityStore {
    pub async fn open(pool: SqlitePool) -> anyhow::Result<Self> {
        let store = Self { pool };
        store.ensure_tables().await?;
        Ok(store)
    }

    async fn ensure_tables(&self) -> anyhow::Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS turn_quality_summary (
                session_id                      TEXT NOT NULL,
                turn_id                         TEXT NOT NULL,
                agent_id                        TEXT NOT NULL,
                model_provider                  TEXT NOT NULL,
                model                           TEXT NOT NULL,
                started_at                      TEXT NOT NULL,
                ended_at                        TEXT NOT NULL,
                elapsed_ms                      INTEGER NOT NULL,
                first_delta_ms                  INTEGER,
                first_content_ms                INTEGER,
                first_reasoning_ms              INTEGER,
                first_tool_ms                   INTEGER,
                iterations                      INTEGER NOT NULL,
                tool_calls_total                INTEGER NOT NULL,
                tool_failures_total             INTEGER NOT NULL,
                tool_time_ms_total              INTEGER NOT NULL,
                slowest_tool_name               TEXT,
                slowest_tool_ms                 INTEGER,
                repeated_tool_warn_count        INTEGER NOT NULL,
                repeated_tool_force_stop_count  INTEGER NOT NULL,
                input_tokens                    INTEGER NOT NULL,
                output_tokens                   INTEGER NOT NULL,
                cache_read_tokens               INTEGER NOT NULL,
                cache_creation_tokens           INTEGER NOT NULL,
                cache_hit_pct                   REAL,
                estimated_cost_usd              REAL,
                context_tokens                  INTEGER,
                context_window                  INTEGER,
                context_usage_pct               REAL,
                compressed                      INTEGER NOT NULL,
                tokens_saved                    INTEGER,
                compact_count                   INTEGER NOT NULL,
                diagnosis_code                  TEXT NOT NULL,
                severity                        TEXT NOT NULL,
                evidence_json                   TEXT NOT NULL,
                asset_count                     INTEGER NOT NULL DEFAULT 0,
                raw_output_token_estimate       INTEGER NOT NULL DEFAULT 0,
                projected_output_tokens         INTEGER NOT NULL DEFAULT 0,
                recall_count                    INTEGER NOT NULL DEFAULT 0,
                repeated_tool_call_indicators   INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (session_id, turn_id)
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_turn_quality_started_at
             ON turn_quality_summary(started_at DESC)",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_turn_quality_session
             ON turn_quality_summary(session_id, started_at DESC)",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_turn_quality_diagnosis
             ON turn_quality_summary(diagnosis_code, severity)",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_turn_quality_slowest_tool
             ON turn_quality_summary(slowest_tool_ms DESC)",
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn upsert_summary(&self, summary: &TurnQualitySummary) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO turn_quality_summary (
                session_id, turn_id, agent_id, model_provider, model, started_at, ended_at,
                elapsed_ms, first_delta_ms, first_content_ms, first_reasoning_ms, first_tool_ms,
                iterations, tool_calls_total, tool_failures_total, tool_time_ms_total,
                slowest_tool_name, slowest_tool_ms, repeated_tool_warn_count,
                repeated_tool_force_stop_count, input_tokens, output_tokens, cache_read_tokens,
                cache_creation_tokens, cache_hit_pct, estimated_cost_usd, context_tokens,
                context_window, context_usage_pct, compressed, tokens_saved, compact_count,
                diagnosis_code, severity, evidence_json,
                asset_count, raw_output_token_estimate, projected_output_tokens,
                recall_count, repeated_tool_call_indicators
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7,
                ?8, ?9, ?10, ?11, ?12,
                ?13, ?14, ?15, ?16,
                ?17, ?18, ?19,
                ?20, ?21, ?22, ?23,
                ?24, ?25, ?26, ?27,
                ?28, ?29, ?30, ?31, ?32,
                ?33, ?34, ?35,
                ?36, ?37, ?38,
                ?39, ?40
            )
            ON CONFLICT(session_id, turn_id) DO UPDATE SET
                agent_id = excluded.agent_id,
                model_provider = excluded.model_provider,
                model = excluded.model,
                started_at = excluded.started_at,
                ended_at = excluded.ended_at,
                elapsed_ms = excluded.elapsed_ms,
                first_delta_ms = excluded.first_delta_ms,
                first_content_ms = excluded.first_content_ms,
                first_reasoning_ms = excluded.first_reasoning_ms,
                first_tool_ms = excluded.first_tool_ms,
                iterations = excluded.iterations,
                tool_calls_total = excluded.tool_calls_total,
                tool_failures_total = excluded.tool_failures_total,
                tool_time_ms_total = excluded.tool_time_ms_total,
                slowest_tool_name = excluded.slowest_tool_name,
                slowest_tool_ms = excluded.slowest_tool_ms,
                repeated_tool_warn_count = excluded.repeated_tool_warn_count,
                repeated_tool_force_stop_count = excluded.repeated_tool_force_stop_count,
                input_tokens = excluded.input_tokens,
                output_tokens = excluded.output_tokens,
                cache_read_tokens = excluded.cache_read_tokens,
                cache_creation_tokens = excluded.cache_creation_tokens,
                cache_hit_pct = excluded.cache_hit_pct,
                estimated_cost_usd = excluded.estimated_cost_usd,
                context_tokens = excluded.context_tokens,
                context_window = excluded.context_window,
                context_usage_pct = excluded.context_usage_pct,
                compressed = excluded.compressed,
                tokens_saved = excluded.tokens_saved,
                compact_count = excluded.compact_count,
                diagnosis_code = excluded.diagnosis_code,
                severity = excluded.severity,
                evidence_json = excluded.evidence_json,
                asset_count = excluded.asset_count,
                raw_output_token_estimate = excluded.raw_output_token_estimate,
                projected_output_tokens = excluded.projected_output_tokens,
                recall_count = excluded.recall_count,
                repeated_tool_call_indicators = excluded.repeated_tool_call_indicators",
        )
        .bind(&summary.session_id)
        .bind(&summary.turn_id)
        .bind(&summary.agent_id)
        .bind(&summary.model_provider)
        .bind(&summary.model)
        .bind(&summary.started_at)
        .bind(&summary.ended_at)
        .bind(summary.elapsed_ms as i64)
        .bind(summary.first_delta_ms.map(|v| v as i64))
        .bind(summary.first_content_ms.map(|v| v as i64))
        .bind(summary.first_reasoning_ms.map(|v| v as i64))
        .bind(summary.first_tool_ms.map(|v| v as i64))
        .bind(summary.iterations as i64)
        .bind(summary.tool_calls_total as i64)
        .bind(summary.tool_failures_total as i64)
        .bind(summary.tool_time_ms_total as i64)
        .bind(&summary.slowest_tool_name)
        .bind(summary.slowest_tool_ms.map(|v| v as i64))
        .bind(summary.repeated_tool_warn_count as i64)
        .bind(summary.repeated_tool_force_stop_count as i64)
        .bind(summary.input_tokens as i64)
        .bind(summary.output_tokens as i64)
        .bind(summary.cache_read_tokens as i64)
        .bind(summary.cache_creation_tokens as i64)
        .bind(summary.cache_hit_pct)
        .bind(summary.estimated_cost_usd)
        .bind(summary.context_tokens.map(i64::from))
        .bind(summary.context_window.map(i64::from))
        .bind(summary.context_usage_pct)
        .bind(if summary.compressed { 1_i64 } else { 0_i64 })
        .bind(summary.tokens_saved)
        .bind(summary.compact_count as i64)
        .bind(summary.diagnosis_code.as_str())
        .bind(summary.severity.as_str())
        .bind(serde_json::to_string(&summary.evidence_json)?)
        .bind(summary.asset_count as i64)
        .bind(summary.raw_output_token_estimate as i64)
        .bind(summary.projected_output_tokens as i64)
        .bind(summary.recall_count as i64)
        .bind(summary.repeated_tool_call_indicators as i64)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn query_recent(&self, limit: i64) -> anyhow::Result<Vec<TurnQualitySummary>> {
        self.query_with_sql(
            "SELECT * FROM turn_quality_summary ORDER BY started_at DESC LIMIT ?1",
            vec![QueryArg::I64(limit.clamp(1, 1000))],
        )
        .await
    }

    pub async fn query_session(
        &self,
        session_id: &str,
        limit: i64,
    ) -> anyhow::Result<Vec<TurnQualitySummary>> {
        self.query_with_sql(
            "SELECT * FROM turn_quality_summary
             WHERE session_id = ?1
             ORDER BY started_at DESC
             LIMIT ?2",
            vec![
                QueryArg::String(session_id.to_string()),
                QueryArg::I64(limit.clamp(1, 1000)),
            ],
        )
        .await
    }

    pub async fn get_turn(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> anyhow::Result<Option<TurnQualitySummary>> {
        let rows = self
            .query_with_sql(
                "SELECT * FROM turn_quality_summary
                 WHERE session_id = ?1 AND turn_id = ?2
                 LIMIT 1",
                vec![
                    QueryArg::String(session_id.to_string()),
                    QueryArg::String(turn_id.to_string()),
                ],
            )
            .await?;
        Ok(rows.into_iter().next())
    }

    async fn query_with_sql(
        &self,
        sql: &str,
        args: Vec<QueryArg>,
    ) -> anyhow::Result<Vec<TurnQualitySummary>> {
        let mut query = sqlx::query(sql);
        for arg in args {
            query = match arg {
                QueryArg::I64(v) => query.bind(v),
                QueryArg::String(v) => query.bind(v),
            };
        }
        let rows = query.fetch_all(&self.pool).await?;
        rows.into_iter().map(row_to_summary).collect()
    }
}

enum QueryArg {
    I64(i64),
    String(String),
}

fn diagnosis_from_str(value: &str) -> TurnQualityDiagnosisCode {
    match value {
        "normal" => TurnQualityDiagnosisCode::Normal,
        "slow_first_token" => TurnQualityDiagnosisCode::SlowFirstToken,
        "slow_tools" => TurnQualityDiagnosisCode::SlowTools,
        "tool_failures" => TurnQualityDiagnosisCode::ToolFailures,
        "tool_loop" => TurnQualityDiagnosisCode::ToolLoop,
        "high_context" => TurnQualityDiagnosisCode::HighContext,
        "high_cost" => TurnQualityDiagnosisCode::HighCost,
        "cache_miss" => TurnQualityDiagnosisCode::CacheMiss,
        "many_iterations" => TurnQualityDiagnosisCode::ManyIterations,
        "aborted" => TurnQualityDiagnosisCode::Aborted,
        "error" => TurnQualityDiagnosisCode::Error,
        _ => TurnQualityDiagnosisCode::Unknown,
    }
}

fn severity_from_str(value: &str) -> TurnQualitySeverity {
    match value {
        "warn" => TurnQualitySeverity::Warn,
        "error" => TurnQualitySeverity::Error,
        _ => TurnQualitySeverity::Info,
    }
}

fn row_to_summary(row: sqlx::sqlite::SqliteRow) -> anyhow::Result<TurnQualitySummary> {
    use sqlx::Row;

    let evidence_raw: String = row.try_get("evidence_json")?;
    Ok(TurnQualitySummary {
        session_id: row.try_get("session_id")?,
        turn_id: row.try_get("turn_id")?,
        agent_id: row.try_get("agent_id")?,
        model_provider: row.try_get("model_provider")?,
        model: row.try_get("model")?,
        started_at: row.try_get("started_at")?,
        ended_at: row.try_get("ended_at")?,
        elapsed_ms: row.try_get::<i64, _>("elapsed_ms")? as u64,
        first_delta_ms: row
            .try_get::<Option<i64>, _>("first_delta_ms")?
            .map(|v| v as u64),
        first_content_ms: row
            .try_get::<Option<i64>, _>("first_content_ms")?
            .map(|v| v as u64),
        first_reasoning_ms: row
            .try_get::<Option<i64>, _>("first_reasoning_ms")?
            .map(|v| v as u64),
        first_tool_ms: row
            .try_get::<Option<i64>, _>("first_tool_ms")?
            .map(|v| v as u64),
        iterations: row.try_get::<i64, _>("iterations")? as u32,
        tool_calls_total: row.try_get::<i64, _>("tool_calls_total")? as u32,
        tool_failures_total: row.try_get::<i64, _>("tool_failures_total")? as u32,
        tool_time_ms_total: row.try_get::<i64, _>("tool_time_ms_total")? as u64,
        slowest_tool_name: row.try_get("slowest_tool_name")?,
        slowest_tool_ms: row
            .try_get::<Option<i64>, _>("slowest_tool_ms")?
            .map(|v| v as u64),
        repeated_tool_warn_count: row.try_get::<i64, _>("repeated_tool_warn_count")? as u32,
        repeated_tool_force_stop_count: row.try_get::<i64, _>("repeated_tool_force_stop_count")?
            as u32,
        input_tokens: row.try_get::<i64, _>("input_tokens")? as u32,
        output_tokens: row.try_get::<i64, _>("output_tokens")? as u32,
        cache_read_tokens: row.try_get::<i64, _>("cache_read_tokens")? as u32,
        cache_creation_tokens: row.try_get::<i64, _>("cache_creation_tokens")? as u32,
        cache_hit_pct: row.try_get("cache_hit_pct")?,
        estimated_cost_usd: row.try_get("estimated_cost_usd")?,
        context_tokens: row
            .try_get::<Option<i64>, _>("context_tokens")?
            .map(|v| v as u32),
        context_window: row
            .try_get::<Option<i64>, _>("context_window")?
            .map(|v| v as u32),
        context_usage_pct: row.try_get("context_usage_pct")?,
        compressed: row.try_get::<i64, _>("compressed")? != 0,
        tokens_saved: row.try_get("tokens_saved")?,
        compact_count: row.try_get::<i64, _>("compact_count")? as u32,
        diagnosis_code: diagnosis_from_str(row.try_get::<String, _>("diagnosis_code")?.as_str()),
        severity: severity_from_str(row.try_get::<String, _>("severity")?.as_str()),
        evidence_json: serde_json::from_str(&evidence_raw)?,
        asset_count: row.try_get::<i64, _>("asset_count")? as u32,
        raw_output_token_estimate: row.try_get::<i64, _>("raw_output_token_estimate")? as u64,
        projected_output_tokens: row.try_get::<i64, _>("projected_output_tokens")? as u64,
        recall_count: row.try_get::<i64, _>("recall_count")? as u32,
        repeated_tool_call_indicators: row.try_get::<i64, _>("repeated_tool_call_indicators")? as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    fn sample_summary(session_id: &str, turn_id: &str) -> TurnQualitySummary {
        TurnQualitySummary {
            session_id: session_id.to_string(),
            turn_id: turn_id.to_string(),
            agent_id: "default".to_string(),
            model_provider: "openai".to_string(),
            model: "gpt-test".to_string(),
            started_at: "2026-06-25T00:00:00Z".to_string(),
            ended_at: "2026-06-25T00:00:02Z".to_string(),
            elapsed_ms: 2000,
            first_delta_ms: Some(200),
            first_content_ms: Some(250),
            first_reasoning_ms: None,
            first_tool_ms: Some(800),
            iterations: 2,
            tool_calls_total: 1,
            tool_failures_total: 0,
            tool_time_ms_total: 120,
            slowest_tool_name: Some("read_file".to_string()),
            slowest_tool_ms: Some(120),
            repeated_tool_warn_count: 0,
            repeated_tool_force_stop_count: 0,
            input_tokens: 100,
            output_tokens: 40,
            cache_read_tokens: 20,
            cache_creation_tokens: 0,
            cache_hit_pct: Some(20.0),
            estimated_cost_usd: Some(0.001),
            context_tokens: Some(300),
            context_window: Some(1000),
            context_usage_pct: Some(30.0),
            compressed: false,
            tokens_saved: None,
            compact_count: 0,
            diagnosis_code: TurnQualityDiagnosisCode::Normal,
            severity: TurnQualitySeverity::Info,
            evidence_json: serde_json::json!({"elapsedMs": 2000, "toolCallsTotal": 1}),
            asset_count: 0,
            raw_output_token_estimate: 0,
            projected_output_tokens: 0,
            recall_count: 0,
            repeated_tool_call_indicators: 0,
        }
    }

    #[tokio::test]
    async fn upsert_and_query_summary() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let store = RuntimeQualityStore::open(pool).await.unwrap();
        let mut summary = sample_summary("s1", "t1");
        store.upsert_summary(&summary).await.unwrap();

        summary.diagnosis_code = TurnQualityDiagnosisCode::SlowTools;
        summary.severity = TurnQualitySeverity::Warn;
        summary.slowest_tool_ms = Some(6_000);
        store.upsert_summary(&summary).await.unwrap();

        let rows = store.query_session("s1", 10).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].diagnosis_code, TurnQualityDiagnosisCode::SlowTools);
        assert_eq!(rows[0].slowest_tool_ms, Some(6_000));

        let turn = store.get_turn("s1", "t1").await.unwrap().unwrap();
        assert_eq!(turn.evidence_json["elapsedMs"], 2000);
    }
}
