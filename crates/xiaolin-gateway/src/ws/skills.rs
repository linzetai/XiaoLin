use axum::extract::ws::{Message, WebSocket};
use serde_json::json;

use crate::state::AppState;
use xiaolin_protocol::SkillsListParams;

use super::send_resp;
use super::types::WsResponse;

/// List skills available to an agent.
pub async fn handle_skills_list(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    params: SkillsListParams,
) {
    let agent_id = params
        .agent_id
        .as_deref()
        .or_else(|| params.extra.get("agentId").and_then(|v| v.as_str()))
        .unwrap_or("main");

    let registry = state.skill_registry_for(agent_id);
    let skills: Vec<_> = registry
        .list()
        .into_iter()
        .filter(|s| s.frontmatter.enabled.unwrap_or(true))
        .map(|s| {
            json!({
                "id": s.id,
                "name": s.name,
                "description": s.description,
                "tags": s.frontmatter.tags,
            })
        })
        .collect();

    send_resp(
        sender,
        &WsResponse {
            id: req_id,
            msg_type: "skills.list".into(),
            data: Some(json!({"skills": skills, "count": skills.len()})),
            error: None,
        },
    )
    .await;
}

/// Refresh skills from disk.
pub async fn handle_skills_refresh(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
) {
    match state.reload_skills() {
        Ok(count) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "skills.refresh".into(),
                    data: Some(json!({"refreshed": true, "count": count})),
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
                    msg_type: "skills.refresh".into(),
                    data: Some(json!({"refreshed": false, "count": 0})),
                    error: Some(json!({"code": 500, "message": format!("failed to reload skills: {}", e)})),
                },
            )
            .await;
        }
    }
}
