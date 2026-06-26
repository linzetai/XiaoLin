use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use serde_json::json;

use crate::state::AppState;
use xiaolin_core::config_access::{
    filter_config_for_read, navigate_config, persist_config_key, set_nested_key,
    CONFIG_READABLE_KEYS, CONFIG_WRITABLE_KEYS,
};

use super::send_resp;
use super::types::WsResponse;

pub async fn handle_models_list(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
) {
    let mut models: Vec<serde_json::Value> = Vec::new();
    let mut seen = std::collections::HashSet::<String>::new();
    let live: serde_json::Value = (**state.cfg.config_live.load()).clone();
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
            let provider = key.clone();
            let dedupe_key = format!("{provider}::{model}");
            if !seen.insert(dedupe_key) {
                continue;
            }
            models.push(json!({
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
    // Append models from LLM provider plugins.
    if let Ok(registry) = state.ext.llm_plugin_registry.try_read() {
        for plugin in registry.list() {
            if !plugin.enabled {
                continue;
            }
            let provider_id = format!("plugin:{}", plugin.id);
            for m in &plugin.models {
                let dedupe_key = format!("{provider_id}::{}", m.id);
                if !seen.insert(dedupe_key) {
                    continue;
                }
                models.push(json!({
                    "agentId": format!("plugin:{}", plugin.id),
                    "model": m.id,
                    "provider": provider_id,
                    "contextWindow": m.context_window,
                    "costPer1kInput": serde_json::Value::Null,
                    "costPer1kOutput": serde_json::Value::Null,
                    "supportsReasoning": serde_json::Value::Null,
                    "pluginName": plugin.name,
                }));
            }
        }
    }

    send_resp(
        sender,
        &WsResponse {
            id: req_id,
            msg_type: "models.list".into(),
            data: Some(json!({"models": models})),
            error: None,
        },
    )
    .await;
}

// ---------- Config API: config.get / config.set ----------

pub async fn handle_config_get(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    params: serde_json::Value,
) {
    let key = params.get("key").and_then(|v| v.as_str()).unwrap_or("");
    let live: serde_json::Value = (**state.cfg.config_live.load()).clone();
    if key.is_empty() {
        let filtered = filter_config_for_read(&live);
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "config.get".into(),
                data: Some(filtered),
                error: None,
            },
        )
        .await;
        return;
    }

    let top_key = key.split('.').next().unwrap_or(key);
    if !CONFIG_READABLE_KEYS.contains(&top_key) {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "error".into(),
                data: None,
                error: Some(json!({
                    "code": 403,
                    "message": format!("access denied: key '{}' is not readable", top_key)
                })),
            },
        )
        .await;
        return;
    }

    let value = navigate_config(&live, key);
    send_resp(
        sender,
        &WsResponse {
            id: req_id,
            msg_type: "config.get".into(),
            data: Some(json!({ "key": key, "value": value })),
            error: None,
        },
    )
    .await;
}

pub async fn handle_config_set(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    params: serde_json::Value,
) {
    let key = match params.get("key").and_then(|v| v.as_str()) {
        Some(k) if !k.is_empty() => k,
        _ => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "error".into(),
                    data: None,
                    error: Some(json!({"code": -32602, "message": "key parameter required"})),
                },
            )
            .await;
            return;
        }
    };

    let value = match params.get("value") {
        Some(v) => v.clone(),
        None => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "error".into(),
                    data: None,
                    error: Some(json!({"code": -32602, "message": "value parameter required"})),
                },
            )
            .await;
            return;
        }
    };

    let top_key = key.split('.').next().unwrap_or(key);
    if !CONFIG_WRITABLE_KEYS.contains(&top_key) {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "error".into(),
                data: None,
                error: Some(json!({
                    "code": 403,
                    "message": format!("access denied: key '{}' is read-only via WS", top_key)
                })),
            },
        )
        .await;
        return;
    }

    let mut cfg_value: serde_json::Value = (**state.cfg.config_live.load()).clone();
    if set_nested_key(&mut cfg_value, key, value.clone()).is_err() {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "error".into(),
                data: None,
                error: Some(json!({"code": -32602, "message": "failed to set nested key"})),
            },
        )
        .await;
        return;
    }

    match serde_json::from_value::<xiaolin_core::config::XiaoLinConfig>(cfg_value.clone()) {
        Ok(new_config) => {
            let persisted = persist_config_key(key, &value);
            let applied = persisted.is_ok();
            if let Err(ref e) = persisted {
                tracing::warn!(key, error = %e, "config.set: validated but failed to persist");
            } else {
                tracing::info!(key, "config.set persisted to user config");
                state.cfg.config_live.store(Arc::new(cfg_value));
                if top_key == "security" {
                    xiaolin_security::ssrf::set_ssrf_allowed_hosts(
                        new_config.security.ssrf_allowed_hosts.clone(),
                    );
                    if let Err(e) = xiaolin_security::dangerous_ops::set_dangerous_ops_config(
                        new_config.security.dangerous_ops_policy,
                        &new_config.security.dangerous_patterns,
                    ) {
                        tracing::error!(error = %e, "config.set: failed to update dangerous-ops policy");
                    }
                    state.cfg.auth.reload(&xiaolin_security::AuthConfig {
                        enabled: !new_config.security.api_keys.is_empty(),
                        api_keys: new_config.security.api_keys.clone(),
                    });
                    tracing::info!(
                        hosts = ?new_config.security.ssrf_allowed_hosts,
                        dangerous_ops_policy = ?new_config.security.dangerous_ops_policy,
                        api_key_count = new_config.security.api_keys.len(),
                        "config.set: hot-reloaded security settings"
                    );
                }
                if top_key == "webSearch" || top_key == "credentials" {
                    if let Err(e) = state.reload_web_search() {
                        tracing::warn!(key, error = %e, "config.set: failed to hot-reload web search");
                    }
                }
                if top_key == "credentials" || top_key == "models" {
                    let agents = state.cfg.last_good_agents.read().await.clone();
                    state.refresh_runtime_agent_providers(&agents);
                    tracing::info!(key, "config.set: hot-reloaded LLM providers");
                }
                if top_key == "skills" {
                    state
                        .rt
                        .runtime
                        .set_skills_deny(new_config.skills.deny.clone());
                    state.rt.runtime.set_skills_context_budget_percent(
                        new_config.skills.context_budget_percent,
                    );
                    if let Err(e) = state.reload_skills() {
                        tracing::warn!(key, error = %e, "config.set: failed to hot-reload skills");
                    }
                }
            }
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "config.set".into(),
                    data: Some(json!({
                        "key": key,
                        "value": value,
                        "status": "validated",
                        "persisted": applied,
                        "pendingRestart": false,
                    })),
                    error: None,
                },
            )
            .await;
        }
        Err(e) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "error".into(),
                    data: None,
                    error: Some(json!({
                        "code": -32602,
                        "message": format!("validation failed: {e}")
                    })),
                },
            )
            .await;
        }
    }
}
