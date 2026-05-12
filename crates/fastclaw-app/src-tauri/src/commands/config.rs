use std::sync::Arc;

use super::helpers::{collect_available_models, get_state};
use crate::embedded::GatewayInfo;
use crate::AppData;
use fastclaw_core::config_access::{
    filter_config_for_read, navigate_config, persist_config_key, set_nested_key,
    CONFIG_READABLE_KEYS, CONFIG_WRITABLE_KEYS,
};
use serde_json::json;

// ─── Model connection test ───

#[tauri::command]
pub async fn test_model_connection(
    base_url: String,
    api_key: String,
    model: Option<String>,
) -> Result<serde_json::Value, String> {
    let base = base_url.trim_end_matches('/');
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    // Try /models first; fall back to a lightweight chat completion probe.
    let models_url = format!("{base}/models");
    let models_resp = client
        .get(&models_url)
        .header("Authorization", format!("Bearer {api_key}"))
        .send()
        .await;

    if let Ok(resp) = models_resp {
        if resp.status().is_success() {
            return Ok(json!({ "ok": true, "method": "models" }));
        }
        // 401/403 means auth failure — report immediately.
        let status = resp.status().as_u16();
        if status == 401 || status == 403 {
            let body = resp.text().await.unwrap_or_default();
            let snippet = if body.len() > 150 {
                &body[..150]
            } else {
                &body
            };
            return Err(format!("认证失败 (HTTP {status}): {snippet}"));
        }
    }

    // Fallback: lightweight chat completion with max_tokens=1 and a tiny prompt.
    let chat_url = format!("{base}/chat/completions");
    let model_name = model.unwrap_or_else(|| "gpt-3.5-turbo".to_string());
    let payload = json!({
        "model": model_name,
        "messages": [{"role": "user", "content": "hi"}],
        "max_tokens": 1,
        "stream": false,
    });
    let chat_resp = client
        .post(&chat_url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("连接失败: {e}"))?;

    let status = chat_resp.status().as_u16();
    if chat_resp.status().is_success() {
        Ok(json!({ "ok": true, "method": "chat" }))
    } else if status == 401 || status == 403 {
        let body = chat_resp.text().await.unwrap_or_default();
        let snippet = if body.len() > 150 {
            &body[..150]
        } else {
            &body
        };
        Err(format!("认证失败 (HTTP {status}): {snippet}"))
    } else {
        let body = chat_resp.text().await.unwrap_or_default();
        let snippet = if body.len() > 150 {
            &body[..150]
        } else {
            &body
        };
        Err(format!("HTTP {status}: {snippet}"))
    }
}

// ─── Gateway info & health ───

#[tauri::command]
pub async fn get_gateway_info(state: tauri::State<'_, AppData>) -> Result<GatewayInfo, String> {
    let gw = state.gateway.lock().await;
    match gw.as_ref() {
        Some(g) => Ok(g.info().clone()),
        None => Err("gateway not started".into()),
    }
}

#[tauri::command]
pub async fn health_check(state: tauri::State<'_, AppData>) -> Result<bool, String> {
    let gw = state.gateway.lock().await;
    Ok(gw.as_ref().is_some())
}

// ─── Models ───

#[tauri::command]
pub async fn list_models(state: tauri::State<'_, AppData>) -> Result<serde_json::Value, String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;
    let models = collect_available_models(app);
    Ok(json!({"models": models}))
}

// ─── Config ───

#[tauri::command]
pub async fn get_config(
    state: tauri::State<'_, AppData>,
    key: Option<String>,
) -> Result<serde_json::Value, String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;
    let key = key.as_deref().unwrap_or("");

    let live: serde_json::Value = (**app.cfg.config_live.load()).clone();

    if key.is_empty() {
        let filtered = filter_config_for_read(&live);
        return Ok(filtered);
    }

    let top_key = key.split('.').next().unwrap_or(key);
    if !CONFIG_READABLE_KEYS.contains(&top_key) {
        return Err(format!("access denied: key '{}' is not readable", top_key));
    }

    let value = navigate_config(&live, key);
    Ok(json!({"key": key, "value": value}))
}

#[tauri::command]
pub async fn set_config(
    state: tauri::State<'_, AppData>,
    key: String,
    value: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let gw = state.gateway.lock().await;
    let app = get_state(&gw)?;

    if key.is_empty() {
        return Err("key parameter required".into());
    }

    let top_key = key.split('.').next().unwrap_or(&key);
    if !CONFIG_WRITABLE_KEYS.contains(&top_key) {
        return Err(format!("access denied: key '{}' is read-only", top_key));
    }

    let mut cfg_value: serde_json::Value = (**app.cfg.config_live.load()).clone();
    if set_nested_key(&mut cfg_value, &key, value.clone()).is_err() {
        return Err("failed to set nested key".into());
    }

    serde_json::from_value::<fastclaw_core::config::FastClawConfig>(cfg_value.clone())
        .map_err(|e| format!("validation failed: {e}"))?;

    let persisted = persist_config_key(&key, &value);
    let applied = persisted.is_ok();
    if let Err(ref e) = persisted {
        tracing::warn!(key = key.as_str(), error = %e, "config.set: validated but failed to persist");
    }

    if applied {
        app.cfg.config_live.store(Arc::new(cfg_value));
        tracing::info!(
            key = key.as_str(),
            "config.set: persisted and updated in-memory"
        );
        if top_key == "credentials" || top_key == "models" {
            if let Err(e) = app.reload_agents().await {
                tracing::warn!(
                    key = key.as_str(),
                    error = %e,
                    "config.set: updated config but failed to refresh runtime providers"
                );
            }
        }
        if top_key == "webSearch" || top_key == "credentials" {
            if let Err(e) = app.reload_web_search() {
                tracing::warn!(
                    key = key.as_str(),
                    error = %e,
                    "config.set: failed to hot-reload web search"
                );
            }
        }
        if top_key == "security" {
            if let Ok(parsed) = serde_json::from_value::<fastclaw_core::config::FastClawConfig>(
                (**app.cfg.config_live.load()).clone(),
            ) {
                fastclaw_security::ssrf::set_ssrf_allowed_hosts(
                    parsed.security.ssrf_allowed_hosts.clone(),
                );
                fastclaw_security::dangerous_ops::set_dangerous_ops_config(
                    parsed.security.dangerous_ops_policy,
                    &parsed.security.dangerous_patterns,
                );
                tracing::info!(
                    hosts = ?parsed.security.ssrf_allowed_hosts,
                    dangerous_ops_policy = ?parsed.security.dangerous_ops_policy,
                    "config.set: hot-reloaded security settings"
                );
            }
        }
    }

    Ok(json!({
        "key": key,
        "persisted": applied,
        "pendingRestart": false,
    }))
}
