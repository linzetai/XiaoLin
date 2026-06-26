use std::path::PathBuf;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use xiaolin_core::agent_config::{self, AgentConfig, AgentModelConfig};
use xiaolin_core::config::XiaoLinConfig;

pub(crate) fn merge_model_base_urls_into_credentials(
    credentials: &xiaolin_core::config::CredentialsConfig,
    models: &std::collections::HashMap<String, xiaolin_core::config::ModelProviderConfig>,
) -> xiaolin_core::config::CredentialsConfig {
    let mut merged = credentials.clone();

    for (key, model_cfg) in models {
        let base_url = model_cfg
            .base_url
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let Some(base_url) = base_url else {
            continue;
        };

        let entry = merged.providers.entry(key.clone()).or_default();
        let has_base = entry
            .base_url
            .as_deref()
            .map(str::trim)
            .map(|s| !s.is_empty())
            .unwrap_or(false);
        if !has_base {
            entry.base_url = Some(base_url.to_string());
        }
    }

    merged
}

pub(crate) async fn open_unified_pool(
    db_dir: &std::path::Path,
) -> anyhow::Result<sqlx::SqlitePool> {
    let unified_db = db_dir.with_file_name("xiaolin.db");
    if let Some(parent) = unified_db.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let options = SqliteConnectOptions::new()
        .filename(&unified_db)
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .foreign_keys(true)
        .busy_timeout(std::time::Duration::from_secs(5));
    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(options)
        .await?;
    tracing::info!(path = %unified_db.display(), "unified database pool opened");
    Ok(pool)
}

pub(crate) async fn open_memory_pool_at(
    db_path: &std::path::Path,
) -> anyhow::Result<sqlx::SqlitePool> {
    let options = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .foreign_keys(true)
        .busy_timeout(std::time::Duration::from_secs(5));
    let pool = SqlitePoolOptions::new()
        .max_connections(3)
        .connect_with(options)
        .await?;
    Ok(pool)
}

/// Migrate legacy per-database files (sessions.db, evolution.db, cron.db)
/// into the unified xiaolin.db, then rename the old files to `.bak`.
///
/// Each legacy DB is ATTACHed, its tables' data is `INSERT OR IGNORE`-ed
/// into the main DB, then DETACHed. This is idempotent: running twice
/// on already-migrated files is harmless because the `.bak` rename prevents
/// re-processing.
///
/// IMPORTANT: ATTACH DATABASE is per-connection in SQLite, so we acquire
/// a single connection from the pool and run the entire ATTACH→copy→DETACH
/// sequence on it.
pub(crate) async fn migrate_legacy_databases(
    pool: &sqlx::SqlitePool,
    db_dir: &std::path::Path,
) -> anyhow::Result<()> {
    let legacy_dbs: &[(&str, &[&str])] = &[
        (
            "sessions.db",
            &[
                "sessions",
                "messages",
                "conversation_traces",
                "subagent_runs",
                "content_replacement_records",
                "collapse_state",
                "session_memory",
                "history_items",
                "event_log",
            ],
        ),
        (
            "evolution.db",
            &[
                "feedback",
                "trajectories",
                "trajectory_steps",
                "extracted_skills",
                "skill_parameters",
                "skill_usages",
                "evolution_session_skills",
                "prompt_candidates",
            ],
        ),
        ("cron.db", &["cron_jobs", "cron_job_runs", "notifications"]),
    ];

    let mut conn = pool.acquire().await?;

    for (filename, tables) in legacy_dbs {
        let legacy_path = db_dir.join(filename);
        if !legacy_path.exists() {
            continue;
        }

        tracing::info!(file = %legacy_path.display(), "migrating legacy database");

        let path_str = legacy_path.to_string_lossy().to_string();
        let attach = format!("ATTACH DATABASE '{}' AS legacy", path_str);
        sqlx::query(&attach).execute(&mut *conn).await?;

        for table in *tables {
            let sql = format!(
                "INSERT OR IGNORE INTO main.{t} SELECT * FROM legacy.{t}",
                t = table
            );
            match sqlx::query(&sql).execute(&mut *conn).await {
                Ok(result) => {
                    let rows = result.rows_affected();
                    if rows > 0 {
                        tracing::info!(table, rows, "migrated rows from {}", filename);
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        table,
                        error = %e,
                        "skipped table migration from {} (table may not exist in legacy DB)",
                        filename
                    );
                }
            }
        }

        sqlx::query("DETACH DATABASE legacy")
            .execute(&mut *conn)
            .await?;

        let bak_path = legacy_path.with_extension("db.bak");
        if let Err(e) = tokio::fs::rename(&legacy_path, &bak_path).await {
            tracing::warn!(
                from = %legacy_path.display(),
                to = %bak_path.display(),
                error = %e,
                "failed to rename legacy db to .bak"
            );
        } else {
            tracing::info!(
                from = %legacy_path.display(),
                to = %bak_path.display(),
                "legacy database renamed to .bak"
            );
        }

        for ext in &["db-wal", "db-shm"] {
            let wal_path = legacy_path.with_extension(ext);
            if wal_path.exists() {
                let bak_wal = bak_path.with_extension(ext);
                let _ = tokio::fs::rename(&wal_path, &bak_wal).await;
            }
        }
    }

    Ok(())
}

pub(crate) fn resolve_db_path(
    paths_cfg: &xiaolin_core::config::PathsConfig,
) -> anyhow::Result<PathBuf> {
    Ok(xiaolin_core::paths::resolve_db_path_from(Some(paths_cfg)))
}

pub(crate) fn load_agents(config: &XiaoLinConfig) -> anyhow::Result<Vec<AgentConfig>> {
    let config_dir = resolve_agents_dir(&config.paths);
    if config_dir.exists() {
        let agents = agent_config::load_agent_configs(&config_dir)?;
        if agents.is_empty() {
            tracing::warn!(
                dir = %config_dir.display(),
                "agents config directory is empty, using built-in default"
            );
            Ok(vec![builtin_default_agent(config)])
        } else {
            Ok(agents)
        }
    } else {
        tracing::warn!(dir = %config_dir.display(), "agents config directory not found, using built-in default");
        Ok(vec![builtin_default_agent(config)])
    }
}

fn resolve_agents_dir(paths_cfg: &xiaolin_core::config::PathsConfig) -> PathBuf {
    xiaolin_core::paths::resolve_agents_dir_from(Some(paths_cfg))
}

pub(crate) fn builtin_default_agent(config: &XiaoLinConfig) -> AgentConfig {
    AgentConfig {
        agent_id: "main".into(),
        name: Some("Main Agent".to_string()),
        description: Some("Built-in default assistant".to_string()),
        model: builtin_default_model(config),
        system_prompt: None,
        tools: Vec::new(),
        behavior: Default::default(),
        mcp_servers: Vec::new(),
        min_tier: None,
        max_tier: None,
        avatar: None,
        channels: std::collections::HashMap::new(),
    }
}

fn builtin_default_model(config: &XiaoLinConfig) -> AgentModelConfig {
    let mut model = AgentModelConfig::default();
    let default_ref = config
        .agents
        .defaults
        .model
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    if let Some(model_ref) = default_ref {
        if let Some((provider, model_name)) = model_ref.split_once('/') {
            let provider = provider.trim();
            let model_name = model_name.trim();
            if !provider.is_empty() && !model_name.is_empty() {
                model.provider = provider.to_string();
                model.model = model_name.to_string();
                return model;
            }
        }
        model.model = model_ref.to_string();
        if let Some((provider_key, _)) = config
            .models
            .iter()
            .find(|(_, cfg)| cfg.model == model.model)
        {
            model.provider = provider_key.clone();
        }
        return model;
    }

    if let Some((key, cfg)) = config.models.iter().next() {
        if !cfg.model.is_empty() {
            model.provider = key.clone();
            model.model = cfg.model.clone();
        }
    }
    model
}

pub(crate) fn resolve_skills_dir(paths_cfg: &xiaolin_core::config::PathsConfig) -> PathBuf {
    xiaolin_core::paths::resolve_skills_dir_from(Some(paths_cfg))
}

pub(crate) fn resolve_state_dir(paths_cfg: &xiaolin_core::config::PathsConfig) -> PathBuf {
    xiaolin_core::paths::resolve_state_dir_from(Some(paths_cfg))
}

pub(crate) fn persist_skills_deny_cleanup(cleaned_deny: &[String]) -> anyhow::Result<()> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("cannot resolve home directory"))?;
    let cfg_path = home.join(".xiaolin/config/default.json");
    let mut cfg_value: serde_json::Value = if cfg_path.exists() {
        let text = std::fs::read_to_string(&cfg_path)?;
        json5::from_str(&text).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    let skills_obj = cfg_value.as_object_mut().and_then(|root| {
        root.entry("skills")
            .or_insert_with(|| serde_json::json!({}))
            .as_object_mut()
    });

    if let Some(skills) = skills_obj {
        skills.insert("deny".to_string(), serde_json::to_value(cleaned_deny)?);
    }

    if let Some(parent) = cfg_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&cfg_path, serde_json::to_string_pretty(&cfg_value)?)?;
    tracing::info!(path = %cfg_path.display(), "persisted cleaned skills.deny list");
    Ok(())
}
