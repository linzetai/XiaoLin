use std::time::Duration;

use crate::{AppData, GatewayStartupState};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct ProxyRequest {
    pub method: String,
    pub path: String,
    pub body: Option<serde_json::Value>,
}

#[derive(Serialize)]
pub struct ProxyResponse {
    pub status: u16,
    pub body: serde_json::Value,
}

const MAX_PROXY_RESPONSE_BYTES: usize = 10 * 1024 * 1024;

fn validate_proxy_path(path: &str) -> Result<(), String> {
    if path.contains("..") {
        return Err("path must not contain '..'".into());
    }
    if !path.starts_with("/v1/") && !path.starts_with("/api/") && !path.starts_with("/health") {
        return Err(
            "path not allowed: must start with /v1/, /api/, or /health".into(),
        );
    }
    Ok(())
}

#[tauri::command]
pub async fn http_proxy(
    state: tauri::State<'_, AppData>,
    request: ProxyRequest,
) -> Result<ProxyResponse, String> {
    let rx = state.startup_watch.clone();
    let info = match rx.borrow().clone() {
        GatewayStartupState::Running { info } => info,
        _ => return Err("gateway not ready".into()),
    };

    validate_proxy_path(&request.path)?;

    let url = format!("{}{}", info.http_url, request.path);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| {
            tracing::error!(error = %e, "http_proxy: failed to build HTTP client");
            String::from("failed to create HTTP client")
        })?;

    let mut req_builder = match request.method.to_uppercase().as_str() {
        "GET" => client.get(&url),
        "POST" => client.post(&url),
        "PUT" => client.put(&url),
        "DELETE" => client.delete(&url),
        "PATCH" => client.patch(&url),
        other => return Err(format!("unsupported method: {other}")),
    };

    if let Some(body) = request.body {
        req_builder = req_builder
            .header("Content-Type", "application/json")
            .json(&body);
    }

    let resp = req_builder.send().await.map_err(|e| {
        tracing::warn!(
            error = %e,
            method = %request.method,
            path = %request.path,
            "http_proxy request failed"
        );
        String::from("proxy request failed")
    })?;

    let status = resp.status().as_u16();
    let raw = resp.bytes().await.map_err(|e| {
        tracing::warn!(
            error = %e,
            method = %request.method,
            path = %request.path,
            status,
            "http_proxy: failed to read response body"
        );
        String::from("proxy request failed")
    })?;
    if raw.len() > MAX_PROXY_RESPONSE_BYTES {
        tracing::warn!(
            size = raw.len(),
            method = %request.method,
            path = %request.path,
            status,
            "http_proxy: response body too large"
        );
        return Err("response too large".into());
    }
    let body: serde_json::Value = match serde_json::from_slice(&raw) {
        Ok(body) => body,
        Err(e) => {
            tracing::warn!(
                error = %e,
                method = %request.method,
                path = %request.path,
                status,
                "http_proxy: failed to parse response JSON"
            );
            serde_json::Value::Null
        }
    };

    Ok(ProxyResponse { status, body })
}
