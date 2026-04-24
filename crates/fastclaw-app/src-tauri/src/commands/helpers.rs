use std::sync::Arc;

use fastclaw_gateway::AppState;
use serde_json::json;

pub fn collect_available_models(app: &AppState) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::<String>::new();

    let live: serde_json::Value = (**app.cfg.config_live.load()).clone();

    if let Some(models_obj) = live.get("models").and_then(|v| v.as_object()) {
        for (key, cfg) in models_obj {
            let model = cfg
                .get("model")
                .or_else(|| cfg.get("defaultModel"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            if model.is_empty() {
                continue;
            }
            // Use the config key (e.g. "dashscope") as provider — it matches
            // credential lookup keys and is recognized by create_provider_chain.
            let provider = key.clone();
            let dedupe_key = format!("{provider}::{model}");
            if !seen.insert(dedupe_key) {
                continue;
            }
            out.push(json!({
                "agentId": key,
                "model": model,
                "provider": provider,
                "contextWindow": cfg.get("contextWindow").cloned().unwrap_or(serde_json::Value::Null),
                "costPer1kInput": cfg.get("costPer1kInput").cloned().unwrap_or(serde_json::Value::Null),
                "costPer1kOutput": cfg.get("costPer1kOutput").cloned().unwrap_or(serde_json::Value::Null),
                "supportsReasoning": cfg.get("supportsReasoning").cloned().unwrap_or(serde_json::Value::Null),
            }));
        }
    }

    out
}

pub fn get_state(gw: &Option<crate::embedded::EmbeddedGateway>) -> Result<&AppState, String> {
    gw.as_ref()
        .map(|g| g.app_state())
        .ok_or_else(|| "gateway not started".to_string())
}

pub fn ensure_agent_workspace_bootstrap(app: &AppState, agent_id: &str) -> Result<(), String> {
    let state_dir = fastclaw_core::paths::resolve_state_dir_from(Some(&app.cfg.config.paths));
    let ws_root = fastclaw_core::workspace::resolve_workspace_root(&state_dir, agent_id, None);
    let ws = fastclaw_core::workspace::AgentWorkspace::new(ws_root, agent_id.to_string());
    ws.ensure_bootstrap()
        .map_err(|e| format!("ensure workspace bootstrap failed: {e}"))
}

pub fn validate_agent_id(agent_id: &str) -> Result<(), String> {
    if agent_id.is_empty() {
        return Err("agent_id cannot be empty".to_string());
    }
    if agent_id.len() > 64 {
        return Err("agent_id too long (max 64 characters)".to_string());
    }
    if !agent_id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        return Err(
            "agent_id contains invalid characters; only [a-zA-Z0-9_-] are allowed".to_string(),
        );
    }
    Ok(())
}

pub fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// Sync an agent's per-agent channels into the global `config_live` so the
/// running gateway can route inbound messages.  Also ensures matching bindings
/// exist.
pub fn sync_agent_channels_to_live(
    app: &AppState,
    agent_id: &str,
    channels: &std::collections::HashMap<String, fastclaw_core::config::ChannelConfig>,
) {
    let mut live_val: serde_json::Value = (**app.cfg.config_live.load()).clone();

    // Merge channels into global config_live.channels
    if let Some(obj) = live_val
        .get_mut("channels")
        .and_then(|v: &mut serde_json::Value| v.as_object_mut())
    {
        for (ch_id, ch_cfg) in channels {
            if let Ok(val) = serde_json::to_value(ch_cfg) {
                obj.insert(ch_id.clone(), val);
            }
        }
    } else if !channels.is_empty() {
        let mut obj = serde_json::Map::new();
        for (ch_id, ch_cfg) in channels {
            if let Ok(val) = serde_json::to_value(ch_cfg) {
                obj.insert(ch_id.clone(), val);
            }
        }
        live_val["channels"] = serde_json::Value::Object(obj);
    }

    // Ensure bindings exist for each channel
    if live_val.get("bindings").is_none() {
        live_val["bindings"] = json!([]);
    }
    if let Some(arr) = live_val
        .get_mut("bindings")
        .and_then(|v: &mut serde_json::Value| v.as_array_mut())
    {
        // Remove old bindings for this agent
        arr.retain(|b: &serde_json::Value| {
            b.get("agentId")
                .and_then(|a: &serde_json::Value| a.as_str())
                != Some(agent_id)
        });
        // Re-add for current channels
        for ch_id in channels.keys() {
            arr.push(json!({
                "agentId": agent_id,
                "match": { "channel": ch_id }
            }));
        }
    }

    app.cfg.config_live.store(Arc::new(live_val));

    tracing::info!(
        agent_id,
        channel_count = channels.len(),
        "synced per-agent channels to config_live"
    );
}

/// Remove all channels and bindings belonging to a deleted agent from `config_live`.
pub fn cleanup_agent_channels_from_live(
    app: &AppState,
    agent_id: &str,
) {
    let mut live: serde_json::Value = (**app.cfg.config_live.load()).clone();
    if let Some(arr) = live
        .get_mut("bindings")
        .and_then(|v: &mut serde_json::Value| v.as_array_mut())
    {
        arr.retain(|b: &serde_json::Value| {
            b.get("agentId")
                .and_then(|a: &serde_json::Value| a.as_str())
                != Some(agent_id)
        });
    }
    app.cfg.config_live.store(Arc::new(live));
    tracing::info!(agent_id, "cleaned up channel bindings for deleted agent");
}
