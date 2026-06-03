use axum::extract::{Path, State};
use axum::Json;
use serde_json::json;

use crate::extract::AppJson;
use crate::state::AppState;

use super::error::AppError;

/// GET /api/v1/llm-plugins — list all loaded LLM provider plugins.
pub(super) async fn list_plugins(
    State(state): State<AppState>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let guard = state.ext.llm_plugin_registry.read().await;
    let plugins: Vec<_> = guard
        .list()
        .iter()
        .map(|p| {
            json!({
                "id": p.id,
                "name": p.name,
                "version": p.version,
                "description": p.description,
                "type": p.plugin_type,
                "enabled": p.enabled,
                "models": p.models,
            })
        })
        .collect();
    Ok(Json(json!({ "plugins": plugins, "count": plugins.len() })))
}

/// GET /api/v1/llm-plugins/:id — get a single plugin config.
pub(super) async fn get_plugin(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let guard = state.ext.llm_plugin_registry.read().await;
    let plugin = guard
        .get(&plugin_id)
        .ok_or_else(|| AppError::NotFound(format!("LLM plugin not found: {plugin_id}")))?;
    Ok(Json(json!(plugin)))
}

/// POST /api/v1/llm-plugins — create a new LLM provider plugin.
/// Writes the config JSON file to the plugins directory and registers it.
pub(super) async fn create_plugin(
    State(state): State<AppState>,
    AppJson(config): AppJson<xiaolin_core::llm_plugin::LlmPluginConfig>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    if config.id.is_empty() {
        return Err(AppError::BadRequest("plugin id must not be empty".into()));
    }

    // Validate config
    match config.plugin_type {
        xiaolin_core::llm_plugin::LlmPluginType::Middleware => {
            if config.middleware.is_none() {
                return Err(AppError::BadRequest(
                    "middleware config required for type=middleware".into(),
                ));
            }
        }
        xiaolin_core::llm_plugin::LlmPluginType::Process => {
            if config.process.is_none() {
                return Err(AppError::BadRequest(
                    "process config required for type=process".into(),
                ));
            }
        }
    }

    // Check for duplicates
    {
        let guard = state.ext.llm_plugin_registry.read().await;
        if guard.get(&config.id).is_some() {
            return Err(AppError::BadRequest(format!(
                "LLM plugin '{}' already exists",
                config.id
            )));
        }
    }

    // Persist to disk
    let plugins_dir = xiaolin_core::llm_plugin::resolve_plugins_dir(
        &state.cfg.config.llm_plugins,
        &state.cfg.config.paths,
    );
    if let Err(e) = std::fs::create_dir_all(&plugins_dir) {
        return Err(AppError::Internal(anyhow::anyhow!(
            "failed to create plugins directory: {e}"
        )));
    }
    let file_path = plugins_dir.join(format!("{}.json", config.id));
    let json_str = serde_json::to_string_pretty(&config)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("failed to serialize plugin: {e}")))?;
    std::fs::write(&file_path, &json_str).map_err(|e| {
        AppError::Internal(anyhow::anyhow!(
            "failed to write plugin file {}: {e}",
            file_path.display()
        ))
    })?;

    // Register in memory
    let id = config.id.clone();
    {
        let mut guard = state.ext.llm_plugin_registry.write().await;
        guard.register(config);
    }

    tracing::info!(plugin_id = %id, "LLM plugin created and registered");
    Ok(Json(json!({ "id": id, "ok": true })))
}

/// PUT /api/v1/llm-plugins/:id — update an existing plugin.
pub(super) async fn update_plugin(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
    AppJson(mut config): AppJson<xiaolin_core::llm_plugin::LlmPluginConfig>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    config.id = plugin_id.clone();

    // Persist to disk
    let plugins_dir = xiaolin_core::llm_plugin::resolve_plugins_dir(
        &state.cfg.config.llm_plugins,
        &state.cfg.config.paths,
    );
    let file_path = plugins_dir.join(format!("{}.json", plugin_id));
    let json_str = serde_json::to_string_pretty(&config)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("failed to serialize plugin: {e}")))?;
    std::fs::write(&file_path, &json_str).map_err(|e| {
        AppError::Internal(anyhow::anyhow!(
            "failed to write plugin file {}: {e}",
            file_path.display()
        ))
    })?;

    // Update registry
    {
        let mut guard = state.ext.llm_plugin_registry.write().await;
        guard.register(config);
    }

    tracing::info!(plugin_id = %plugin_id, "LLM plugin updated");
    Ok(Json(json!({ "id": plugin_id, "ok": true })))
}

/// DELETE /api/v1/llm-plugins/:id — remove a plugin.
pub(super) async fn delete_plugin(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    // Remove from disk
    let plugins_dir = xiaolin_core::llm_plugin::resolve_plugins_dir(
        &state.cfg.config.llm_plugins,
        &state.cfg.config.paths,
    );
    let file_path = plugins_dir.join(format!("{}.json", plugin_id));
    if file_path.exists() {
        std::fs::remove_file(&file_path).map_err(|e| {
            AppError::Internal(anyhow::anyhow!("failed to delete plugin file: {e}"))
        })?;
    }

    // Remove from registry
    let removed = {
        let mut guard = state.ext.llm_plugin_registry.write().await;
        guard.unregister(&plugin_id).is_some()
    };

    tracing::info!(plugin_id = %plugin_id, removed, "LLM plugin deleted");
    Ok(Json(json!({ "id": plugin_id, "deleted": removed })))
}

/// POST /api/v1/llm-plugins/:id/test — test plugin connectivity.
pub(super) async fn test_plugin(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let guard = state.ext.llm_plugin_registry.read().await;
    let provider = guard
        .create_provider(&plugin_id)
        .map_err(|e| AppError::BadRequest(format!("failed to create provider from plugin: {e}")))?;

    let model = guard
        .get(&plugin_id)
        .and_then(|p| p.models.first())
        .map(|m| m.id.clone())
        .unwrap_or_else(|| "gpt-4o-mini".to_string());

    drop(guard);

    let params = xiaolin_agent::CompletionParams {
        model: &model,
        messages: &[xiaolin_core::types::ChatMessage {
            role: xiaolin_core::types::Role::User,
            content: Some(serde_json::Value::String(
                "Say hello in one word.".to_string(),
            )),
        ..Default::default()
        }],
        temperature: 0.0,
        max_tokens: Some(16),
        tools: None,
    };

    match tokio::time::timeout(
        std::time::Duration::from_secs(30),
        provider.chat_completion(&params),
    )
    .await
    {
        Ok(Ok(resp)) => {
            let reply = resp
                .choices
                .first()
                .and_then(|c| c.message.text_content())
                .unwrap_or_default();
            Ok(Json(json!({
                "ok": true,
                "model": resp.model,
                "reply": reply,
            })))
        }
        Ok(Err(e)) => Ok(Json(json!({
            "ok": false,
            "error": e.to_string(),
        }))),
        Err(_) => Ok(Json(json!({
            "ok": false,
            "error": "test timed out after 30s",
        }))),
    }
}
