use std::path::PathBuf;

use fastclaw_core::agent_config::{self, AgentConfig, AgentModelConfig};
use fastclaw_core::config::FastClawConfig;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

pub(crate) fn merge_model_base_urls_into_credentials(
    credentials: &fastclaw_core::config::CredentialsConfig,
    models: &std::collections::HashMap<String, fastclaw_core::config::ModelProviderConfig>,
) -> fastclaw_core::config::CredentialsConfig {
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

pub(crate) async fn open_memory_pool_named(
    db_path: &std::path::Path,
    name: &str,
) -> anyhow::Result<sqlx::SqlitePool> {
    let target_db = db_path.with_file_name(name);
    if let Some(parent) = target_db.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let options = SqliteConnectOptions::new()
        .filename(&target_db)
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .foreign_keys(true)
        .busy_timeout(std::time::Duration::from_secs(5));
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await?;
    Ok(pool)
}

pub(crate) fn resolve_db_path(
    paths_cfg: &fastclaw_core::config::PathsConfig,
) -> anyhow::Result<PathBuf> {
    Ok(fastclaw_core::paths::resolve_db_path_from(Some(paths_cfg)))
}

pub(crate) fn load_agents(config: &FastClawConfig) -> anyhow::Result<Vec<AgentConfig>> {
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

fn resolve_agents_dir(paths_cfg: &fastclaw_core::config::PathsConfig) -> PathBuf {
    fastclaw_core::paths::resolve_agents_dir_from(Some(paths_cfg))
}

pub(crate) fn builtin_default_agent(config: &FastClawConfig) -> AgentConfig {
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

fn builtin_default_model(config: &FastClawConfig) -> AgentModelConfig {
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
    }
    model
}

pub(crate) fn resolve_skills_dir(paths_cfg: &fastclaw_core::config::PathsConfig) -> PathBuf {
    fastclaw_core::paths::resolve_skills_dir_from(Some(paths_cfg))
}

pub(crate) fn resolve_state_dir(paths_cfg: &fastclaw_core::config::PathsConfig) -> PathBuf {
    fastclaw_core::paths::resolve_state_dir_from(Some(paths_cfg))
}

pub(crate) fn persist_skills_deny_cleanup(cleaned_deny: &[String]) -> anyhow::Result<()> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("cannot resolve home directory"))?;
    let cfg_path = home.join(".fastclaw/config/default.json");
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
