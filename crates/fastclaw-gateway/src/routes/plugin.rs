use axum::extract::{Path, State};
use axum::Json;
use serde_json::json;

use crate::extract::AppJson;
use crate::state::AppState;

use super::error::AppError;

pub(super) async fn list_plugins(
    State(state): State<AppState>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let registry = state.plugin_registry.read().await;
    let plugins: Vec<_> = registry
        .list_plugins()
        .into_iter()
        .map(|m| {
            json!({
                "id": m.id,
                "name": m.name,
                "version": m.version,
                "description": m.description,
                "capabilities": m.capabilities.iter().map(|c| json!({
                    "name": c.name,
                    "description": c.description,
                    "export_name": c.export_name,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();

    Ok(Json(json!({
        "plugins": plugins,
        "count": plugins.len(),
    })))
}

pub(super) async fn invoke_plugin(
    State(state): State<AppState>,
    Path((plugin_id, capability)): Path<(String, String)>,
    AppJson(input): AppJson<serde_json::Value>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let registry = state.plugin_registry.read().await;
    let invoke_key = format!("{plugin_id}::{capability}");
    let input_str =
        serde_json::to_string(&input).map_err(|e| anyhow::anyhow!("invalid input JSON: {e}"))?;
    let output = registry
        .invoke(&invoke_key, &input_str)
        .map_err(|e| anyhow::anyhow!("plugin invocation failed: {e}"))?;
    let result: serde_json::Value =
        serde_json::from_str(&output).unwrap_or(serde_json::Value::String(output));
    Ok(Json(json!({ "result": result })))
}
