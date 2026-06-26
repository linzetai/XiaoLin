use sqlx::SqlitePool;

/// Persists cost/token usage data aggregated by day.
pub struct CostStore {
    pool: SqlitePool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TokenUsageDaily {
    pub date: String,
    pub model: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
    pub cost_usd: f64,
    pub call_count: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolCallDaily {
    pub date: String,
    pub tool_name: String,
    pub success_count: i64,
    pub failure_count: i64,
    pub total_duration_ms: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionCostSummary {
    pub session_id: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub total_cost_usd: f64,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub turn_count: i64,
    pub model_breakdown: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CostSummary {
    pub total_cost_usd: f64,
    pub today_cost_usd: f64,
    pub budget_limit: Option<f64>,
    pub budget_used_pct: Option<f64>,
}

impl CostStore {
    pub async fn open(pool: SqlitePool) -> anyhow::Result<Self> {
        let store = Self { pool };
        store.ensure_tables().await?;
        Ok(store)
    }

    async fn ensure_tables(&self) -> anyhow::Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS token_usage_daily (
                date                  TEXT NOT NULL,
                model                 TEXT NOT NULL,
                input_tokens          INTEGER NOT NULL DEFAULT 0,
                output_tokens         INTEGER NOT NULL DEFAULT 0,
                cache_read_tokens     INTEGER NOT NULL DEFAULT 0,
                cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
                cost_usd              REAL NOT NULL DEFAULT 0.0,
                call_count            INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (date, model)
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS tool_call_daily (
                date              TEXT NOT NULL,
                tool_name         TEXT NOT NULL,
                success_count     INTEGER NOT NULL DEFAULT 0,
                failure_count     INTEGER NOT NULL DEFAULT 0,
                total_duration_ms INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (date, tool_name)
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS session_cost_summary (
                session_id          TEXT PRIMARY KEY,
                started_at          TEXT NOT NULL,
                ended_at            TEXT,
                total_cost_usd      REAL NOT NULL DEFAULT 0.0,
                total_input_tokens  INTEGER NOT NULL DEFAULT 0,
                total_output_tokens INTEGER NOT NULL DEFAULT 0,
                turn_count          INTEGER NOT NULL DEFAULT 0,
                model_breakdown     TEXT
            )",
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn record_token_usage(
        &self,
        date: &str,
        model: &str,
        input_tokens: i64,
        output_tokens: i64,
        cache_read_tokens: i64,
        cache_creation_tokens: i64,
        cost_usd: f64,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO token_usage_daily (date, model, input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens, cost_usd, call_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1)
             ON CONFLICT(date, model) DO UPDATE SET
                input_tokens = input_tokens + excluded.input_tokens,
                output_tokens = output_tokens + excluded.output_tokens,
                cache_read_tokens = cache_read_tokens + excluded.cache_read_tokens,
                cache_creation_tokens = cache_creation_tokens + excluded.cache_creation_tokens,
                cost_usd = cost_usd + excluded.cost_usd,
                call_count = call_count + 1",
        )
        .bind(date)
        .bind(model)
        .bind(input_tokens)
        .bind(output_tokens)
        .bind(cache_read_tokens)
        .bind(cache_creation_tokens)
        .bind(cost_usd)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn record_tool_call(
        &self,
        date: &str,
        tool_name: &str,
        success: bool,
        duration_ms: i64,
    ) -> anyhow::Result<()> {
        let (sc, fc) = if success { (1i64, 0i64) } else { (0, 1) };
        sqlx::query(
            "INSERT INTO tool_call_daily (date, tool_name, success_count, failure_count, total_duration_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(date, tool_name) DO UPDATE SET
                success_count = success_count + excluded.success_count,
                failure_count = failure_count + excluded.failure_count,
                total_duration_ms = total_duration_ms + excluded.total_duration_ms",
        )
        .bind(date)
        .bind(tool_name)
        .bind(sc)
        .bind(fc)
        .bind(duration_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn upsert_session_cost(
        &self,
        session_id: &str,
        cost_usd: f64,
        input_tokens: i64,
        output_tokens: i64,
        model_breakdown: Option<&str>,
    ) -> anyhow::Result<()> {
        let now = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
        sqlx::query(
            "INSERT INTO session_cost_summary (session_id, started_at, total_cost_usd, total_input_tokens, total_output_tokens, turn_count, model_breakdown)
             VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6)
             ON CONFLICT(session_id) DO UPDATE SET
                ended_at = ?2,
                total_cost_usd = total_cost_usd + excluded.total_cost_usd,
                total_input_tokens = total_input_tokens + excluded.total_input_tokens,
                total_output_tokens = total_output_tokens + excluded.total_output_tokens,
                turn_count = turn_count + 1,
                model_breakdown = COALESCE(excluded.model_breakdown, model_breakdown)",
        )
        .bind(session_id)
        .bind(&now)
        .bind(cost_usd)
        .bind(input_tokens)
        .bind(output_tokens)
        .bind(model_breakdown)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn query_daily_tokens(
        &self,
        from: &str,
        to: &str,
    ) -> anyhow::Result<Vec<TokenUsageDaily>> {
        let rows = sqlx::query_as::<_, TokenUsageDaily>(
            "SELECT date, model, input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens, cost_usd, call_count
             FROM token_usage_daily
             WHERE date >= ?1 AND date <= ?2
             ORDER BY date ASC, model ASC",
        )
        .bind(from)
        .bind(to)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn query_tool_stats(
        &self,
        from: &str,
        to: &str,
    ) -> anyhow::Result<Vec<ToolCallDaily>> {
        let rows = sqlx::query_as::<_, ToolCallDaily>(
            "SELECT date, tool_name,
                    SUM(success_count) as success_count,
                    SUM(failure_count) as failure_count,
                    SUM(total_duration_ms) as total_duration_ms
             FROM tool_call_daily
             WHERE date >= ?1 AND date <= ?2
             GROUP BY tool_name
             ORDER BY (SUM(success_count) + SUM(failure_count)) DESC",
        )
        .bind(from)
        .bind(to)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn query_sessions(&self, limit: i64) -> anyhow::Result<Vec<SessionCostSummary>> {
        let rows = sqlx::query_as::<_, SessionCostSummary>(
            "SELECT session_id, started_at, ended_at, total_cost_usd, total_input_tokens, total_output_tokens, turn_count, model_breakdown
             FROM session_cost_summary
             ORDER BY started_at DESC
             LIMIT ?1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn query_summary(&self, budget_limit: Option<f64>) -> anyhow::Result<CostSummary> {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();

        let total: (f64,) =
            sqlx::query_as("SELECT COALESCE(SUM(cost_usd), 0.0) FROM token_usage_daily")
                .fetch_one(&self.pool)
                .await?;

        let today_cost: (f64,) = sqlx::query_as(
            "SELECT COALESCE(SUM(cost_usd), 0.0) FROM token_usage_daily WHERE date = ?1",
        )
        .bind(&today)
        .fetch_one(&self.pool)
        .await?;

        let budget_used_pct = budget_limit.map(|limit| {
            if limit > 0.0 {
                (today_cost.0 / limit) * 100.0
            } else {
                0.0
            }
        });

        Ok(CostSummary {
            total_cost_usd: total.0,
            today_cost_usd: today_cost.0,
            budget_limit,
            budget_used_pct,
        })
    }
}

impl sqlx::FromRow<'_, sqlx::sqlite::SqliteRow> for TokenUsageDaily {
    fn from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;
        Ok(Self {
            date: row.try_get("date")?,
            model: row.try_get("model")?,
            input_tokens: row.try_get("input_tokens")?,
            output_tokens: row.try_get("output_tokens")?,
            cache_read_tokens: row.try_get("cache_read_tokens")?,
            cache_creation_tokens: row.try_get("cache_creation_tokens")?,
            cost_usd: row.try_get("cost_usd")?,
            call_count: row.try_get("call_count")?,
        })
    }
}

impl sqlx::FromRow<'_, sqlx::sqlite::SqliteRow> for ToolCallDaily {
    fn from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;
        Ok(Self {
            date: row.try_get("date")?,
            tool_name: row.try_get("tool_name")?,
            success_count: row.try_get("success_count")?,
            failure_count: row.try_get("failure_count")?,
            total_duration_ms: row.try_get("total_duration_ms")?,
        })
    }
}

impl sqlx::FromRow<'_, sqlx::sqlite::SqliteRow> for SessionCostSummary {
    fn from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;
        Ok(Self {
            session_id: row.try_get("session_id")?,
            started_at: row.try_get("started_at")?,
            ended_at: row.try_get("ended_at")?,
            total_cost_usd: row.try_get("total_cost_usd")?,
            total_input_tokens: row.try_get("total_input_tokens")?,
            total_output_tokens: row.try_get("total_output_tokens")?,
            turn_count: row.try_get("turn_count")?,
            model_breakdown: row.try_get("model_breakdown")?,
        })
    }
}
