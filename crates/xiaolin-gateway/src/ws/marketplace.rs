use axum::extract::ws::{Message, WebSocket};
use serde_json::json;

use crate::state::AppState;

use super::send_resp;
use super::types::WsResponse;

/// Browse or search the skill marketplace.
///
/// When `query` is `None` or empty, returns featured/popular skills.
/// Otherwise performs a keyword search via HubClient.
pub async fn handle_marketplace_browse(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    query: Option<String>,
    limit: Option<usize>,
) {
    let hub = &state.ext.hub_client;
    let limit = limit.unwrap_or(20).min(100);

    let installed = hub.list_installed().unwrap_or_default();

    let result = match query.as_deref() {
        Some(q) if !q.trim().is_empty() => hub.search(q.trim(), limit).await,
        _ => hub
            .featured(limit)
            .await
            .map(|pkgs| xiaolin_core::hub::SearchResult {
                total: pkgs.len() as u64,
                packages: pkgs,
            }),
    };

    match result {
        Ok(search_result) => {
            let skills: Vec<serde_json::Value> = search_result
                .packages
                .iter()
                .map(|pkg| {
                    let is_installed = installed.contains(&pkg.id);
                    json!({
                        "id": pkg.id,
                        "name": pkg.name,
                        "version": pkg.version,
                        "description": pkg.description,
                        "author": pkg.author,
                        "tags": pkg.tags,
                        "downloads": pkg.downloads,
                        "repository": pkg.repository,
                        "homepage": pkg.homepage,
                        "installed": is_installed,
                    })
                })
                .collect();

            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "marketplace.browse".into(),
                    data: Some(json!({
                        "skills": skills,
                        "total": search_result.total,
                        "installed_count": installed.len(),
                    })),
                    error: None,
                },
            )
            .await;
        }
        Err(e) => {
            tracing::warn!(error = %e, "marketplace browse failed");
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "marketplace.browse".into(),
                    data: Some(json!({
                        "skills": [],
                        "total": 0,
                        "installed_count": installed.len(),
                        "offline": true,
                    })),
                    error: Some(json!({
                        "code": 503,
                        "message": format!("marketplace unavailable: {e}"),
                    })),
                },
            )
            .await;
        }
    }
}

/// Install a skill from the marketplace.
pub async fn handle_marketplace_install(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    skill_id: &str,
    version: Option<&str>,
) {
    let hub = &state.ext.hub_client;

    match hub.install(skill_id, version).await {
        Ok(result) => {
            let _ = state.reload_skills();

            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "marketplace.install".into(),
                    data: Some(json!({
                        "installed": true,
                        "skill_id": result.skill_id,
                        "version": result.version,
                        "path": result.install_path.to_string_lossy(),
                        "files": result.files,
                    })),
                    error: None,
                },
            )
            .await;
        }
        Err(e) => {
            tracing::warn!(skill_id, error = %e, "marketplace install failed");
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "marketplace.install".into(),
                    data: None,
                    error: Some(json!({
                        "code": 500,
                        "message": format!("install failed: {e}"),
                    })),
                },
            )
            .await;
        }
    }
}

/// Uninstall a marketplace-installed skill.
pub async fn handle_marketplace_uninstall(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    skill_id: &str,
) {
    let hub = &state.ext.hub_client;

    match hub.uninstall(skill_id) {
        Ok(()) => {
            let _ = state.reload_skills();

            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "marketplace.uninstall".into(),
                    data: Some(json!({
                        "uninstalled": true,
                        "skill_id": skill_id,
                    })),
                    error: None,
                },
            )
            .await;
        }
        Err(e) => {
            tracing::warn!(skill_id, error = %e, "marketplace uninstall failed");
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "marketplace.uninstall".into(),
                    data: None,
                    error: Some(json!({
                        "code": 500,
                        "message": format!("uninstall failed: {e}"),
                    })),
                },
            )
            .await;
        }
    }
}
