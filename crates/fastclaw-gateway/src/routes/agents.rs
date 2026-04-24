//! REST API for managing on-disk agent JSON configs and hot-reload.

use std::path::PathBuf;

use axum::extract::{Path, State};
use axum::Json;
use fastclaw_core::agent_config::AgentConfig;
use serde::Deserialize;
use serde_json::json;

use crate::extract::AppJson;
use crate::state::AppState;

use super::common::memory_scoped_tool_visible_for_agent;
use super::error::AppError;

fn agents_dir(state: &AppState) -> PathBuf {
    fastclaw_core::paths::resolve_agents_dir_from(Some(&state.config.paths))
}

fn agent_json_path(state: &AppState, agent_id: &str) -> PathBuf {
    agents_dir(state).join(format!("{agent_id}.json"))
}

fn validate_agent_id_param(id: &str) -> Result<(), AppError> {
    if id.is_empty() || id.len() > 128 {
        return Err(AppError::BadRequest("invalid agent_id".into()));
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(AppError::BadRequest(
            "agent_id may only contain letters, digits, underscore, and hyphen".into(),
        ));
    }
    Ok(())
}

pub fn tool_effective_enabled(agent: &AgentConfig, tool_name: &str) -> bool {
    if !memory_scoped_tool_visible_for_agent(tool_name, &agent.agent_id) {
        return false;
    }
    if !agent.behavior.tools_deny.is_empty()
        && agent.behavior.tools_deny.iter().any(|d| d == tool_name)
    {
        return false;
    }
    if !agent.behavior.tools_allow.is_empty()
        && !agent
            .behavior
            .tools_allow
            .iter()
            .any(|a| a == tool_name)
    {
        return false;
    }
    true
}

pub fn rebuild_behavior_tool_lists(
    agent: &mut AgentConfig,
    registry_tool_names: &[String],
    toggles: &[(String, bool)],
) {
    let toggle_map: std::collections::HashMap<&str, bool> =
        toggles.iter().map(|(k, v)| (k.as_str(), *v)).collect();

    let mut resolved: Vec<(String, bool)> = Vec::new();
    for name in registry_tool_names {
        if !memory_scoped_tool_visible_for_agent(name, &agent.agent_id) {
            continue;
        }
        let enabled = toggle_map
            .get(name.as_str())
            .copied()
            .unwrap_or_else(|| tool_effective_enabled(agent, name));
        resolved.push((name.clone(), enabled));
    }

    let total = resolved.len();
    let enabled_count = resolved.iter().filter(|(_, e)| *e).count();

    if total == 0 {
        agent.behavior.tools_allow.clear();
        agent.behavior.tools_deny.clear();
        return;
    }

    if enabled_count == total {
        agent.behavior.tools_allow.clear();
        agent.behavior.tools_deny.clear();
    } else if enabled_count <= total.saturating_sub(enabled_count) {
        agent.behavior.tools_allow = resolved
            .iter()
            .filter(|(_, e)| *e)
            .map(|(n, _)| n.clone())
            .collect();
        agent.behavior.tools_deny.clear();
    } else {
        agent.behavior.tools_allow.clear();
        agent.behavior.tools_deny = resolved
            .iter()
            .filter(|(_, e)| !*e)
            .map(|(n, _)| n.clone())
            .collect();
    }
}

async fn write_agent_config_file(state: &AppState, config: &AgentConfig) -> Result<(), AppError> {
    let dir = agents_dir(state);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("create agents dir: {e}")))?;
    let path = agent_json_path(state, config.agent_id.as_str());
    let bytes = serde_json::to_vec_pretty(config)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("serialize agent config: {e}")))?;
    tokio::fs::write(&path, bytes)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("write {}: {e}", path.display())))
}

pub(super) async fn get_agent(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    validate_agent_id_param(&agent_id)?;
    let cfg = {
        let router = state.router.read().await;
        router
            .agent_by_id(&agent_id)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("agent not found: {agent_id}")))?
    };
    Ok(Json(cfg))
}

pub(super) async fn put_agent(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    AppJson(config): AppJson<AgentConfig>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    validate_agent_id_param(&agent_id)?;
    if config.agent_id != agent_id {
        return Err(AppError::BadRequest(format!(
            "agentId in body (`{}`) must match URL path (`{agent_id}`)",
            config.agent_id
        )));
    }
    write_agent_config_file(&state, &config).await?;
    let count = state.reload_agents().await?;
    Ok(Json(json!({
        "ok": true,
        "agentId": config.agent_id,
        "reloaded": count,
    })))
}

pub(super) async fn post_agent(
    State(state): State<AppState>,
    AppJson(config): AppJson<AgentConfig>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    validate_agent_id_param(config.agent_id.as_str())?;
    let path = agent_json_path(&state, config.agent_id.as_str());
    if path.exists() {
        return Err(AppError::BadRequest(format!(
            "agent `{}` already exists",
            config.agent_id
        )));
    }
    write_agent_config_file(&state, &config).await?;
    let count = state.reload_agents().await?;
    Ok(Json(json!({
        "ok": true,
        "agentId": config.agent_id,
        "reloaded": count,
    })))
}

pub(super) async fn delete_agent(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    validate_agent_id_param(&agent_id)?;
    let count_before = {
        let router = state.router.read().await;
        router.agent_count()
    };
    if count_before <= 1 {
        return Err(AppError::BadRequest(
            "refusing to delete the last remaining agent".into(),
        ));
    }
    let path = agent_json_path(&state, &agent_id);
    if !path.exists() {
        return Err(AppError::NotFound(format!(
            "agent config file not found for `{agent_id}`"
        )));
    }
    tokio::fs::remove_file(&path)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("remove {}: {e}", path.display())))?;
    let count = state.reload_agents().await?;
    Ok(Json(json!({ "ok": true, "deleted": agent_id, "reloaded": count })))
}

pub(super) async fn list_agent_tools(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    validate_agent_id_param(&agent_id)?;
    let agent = {
        let router = state.router.read().await;
        router
            .agent_by_id(&agent_id)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("agent not found: {agent_id}")))?
    };
    let tools: Vec<_> = state
        .tool_registry
        .definitions()
        .iter()
        .map(|td| {
            let name = &td.function.name;
            let enabled = tool_effective_enabled(&agent, name);
            json!({
                "id": name,
                "enabled": enabled,
                "description": td.function.description,
            })
        })
        .collect();
    Ok(Json(json!({ "agentId": agent_id, "tools": tools })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AgentToolsPutBody {
    tools: Vec<AgentToolToggle>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AgentToolToggle {
    id: String,
    enabled: bool,
}

pub(super) async fn put_agent_tools(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    AppJson(body): AppJson<AgentToolsPutBody>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    validate_agent_id_param(&agent_id)?;
    let mut agent = {
        let router = state.router.read().await;
        router
            .agent_by_id(&agent_id)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("agent not found: {agent_id}")))?
    };

    let registry_names: Vec<String> = state
        .tool_registry
        .definitions()
        .iter()
        .map(|td| td.function.name.clone())
        .collect();

    let toggles: Vec<(String, bool)> = body
        .tools
        .into_iter()
        .map(|t| (t.id, t.enabled))
        .collect();

    rebuild_behavior_tool_lists(&mut agent, &registry_names, &toggles);
    write_agent_config_file(&state, &agent).await?;
    let count = state.reload_agents().await?;
    Ok(Json(json!({ "ok": true, "agentId": agent_id, "reloaded": count })))
}
