use std::collections::HashSet;

use axum::extract::ws::{Message, WebSocket};
use serde_json::json;

use crate::state::AppState;
use xiaolin_core::skill::SkillOrigin;
use xiaolin_protocol::{SkillsDeleteParams, SkillsListParams, SkillsReadParams, SkillsUpdateParams};

use super::send_resp;
use super::types::WsResponse;

fn origin_str(origin: SkillOrigin) -> &'static str {
    match origin {
        SkillOrigin::XiaoLin => "xiaolin",
        SkillOrigin::Cursor => "cursor",
        SkillOrigin::Codex => "codex",
        SkillOrigin::SharedAgents => "shared_agents",
        SkillOrigin::Extension => "extension",
        SkillOrigin::Mcp => "mcp",
    }
}

/// List skills available to an agent (enhanced with source/layer/enabled).
pub async fn handle_skills_list(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    params: SkillsListParams,
) {
    let _agent_id = params
        .agent_id
        .as_deref()
        .or_else(|| params.extra.get("agentId").and_then(|v| v.as_str()))
        .unwrap_or("main");

    let deny_list: Vec<String> = {
        let live = state.cfg.config_live.load();
        xiaolin_core::config_access::read_live_field(&live, "skills", "deny")
    };
    let deny_set: HashSet<&str> = deny_list.iter().map(String::as_str).collect();

    let usage_counts = match state.store.skill_usage_store.usage_counts(30).await {
        Ok(counts) => counts,
        Err(e) => {
            tracing::warn!(error = %e, "failed to load skill usage counts for skills.list");
            std::collections::HashMap::new()
        }
    };

    let registry: std::sync::Arc<xiaolin_core::skill::SkillRegistry> =
        (*state.rt.unfiltered_skill_registry.load()).clone();
    let skills: Vec<_> = registry
        .list()
        .into_iter()
        .map(|s| {
            let enabled = s.frontmatter.enabled.unwrap_or(true)
                && !deny_set.contains(s.id.as_str());
            let origin = s
                .source
                .as_ref()
                .map(|src| origin_str(src.origin))
                .unwrap_or("xiaolin");
            let usage_count = usage_counts.get(&s.id).copied().unwrap_or(0);
            json!({
                "id": s.id,
                "name": s.name,
                "description": s.description,
                "tags": s.frontmatter.tags,
                "source": origin,
                "layer": format!("{:?}", s.layer),
                "enabled": enabled,
                "paths": s.frontmatter.paths,
                "conditional": s.is_conditional(),
                "usage_count": usage_count,
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

/// Read a single skill's full content and metadata.
pub async fn handle_skills_read(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    params: SkillsReadParams,
) {
    let registry: std::sync::Arc<xiaolin_core::skill::SkillRegistry> =
        (*state.rt.unfiltered_skill_registry.load()).clone();
    let deny_list: Vec<String> = {
        let live = state.cfg.config_live.load();
        xiaolin_core::config_access::read_live_field(&live, "skills", "deny")
    };
    let deny_set: HashSet<&str> = deny_list.iter().map(String::as_str).collect();

    match registry.get(&params.skill_id) {
        Some(skill) => {
            let origin = skill
                .source
                .as_ref()
                .map(|src| origin_str(src.origin))
                .unwrap_or("xiaolin");
            let enabled = skill.frontmatter.enabled.unwrap_or(true)
                && !deny_set.contains(skill.id.as_str());
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "skills.read".into(),
                    data: Some(json!({
                        "id": skill.id,
                        "name": skill.name,
                        "description": skill.description,
                        "content": skill.content,
                        "tags": skill.frontmatter.tags,
                        "tools": skill.frontmatter.tools,
                        "paths": skill.frontmatter.paths,
                        "conditional": skill.is_conditional(),
                        "source": origin,
                        "layer": format!("{:?}", skill.layer),
                        "enabled": enabled,
                        "source_path": skill.source_path.to_string_lossy(),
                    })),
                    error: None,
                },
            )
            .await;
        }
        None => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "skills.read".into(),
                    data: None,
                    error: Some(json!({"code": 404, "message": format!("skill '{}' not found", params.skill_id)})),
                },
            )
            .await;
        }
    }
}

/// Update a skill's content on disk (XiaoLin-owned only).
pub async fn handle_skills_update(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    params: SkillsUpdateParams,
) {
    let registry = state.skill_registry_for("main");

    let skill = match registry.get(&params.skill_id) {
        Some(s) => s,
        None => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "skills.update".into(),
                    data: None,
                    error: Some(json!({"code": 404, "message": format!("skill '{}' not found", params.skill_id)})),
                },
            )
            .await;
            return;
        }
    };

    let is_xiaolin = skill
        .source
        .as_ref()
        .map(|s| s.origin == SkillOrigin::XiaoLin)
        .unwrap_or(true);

    if !is_xiaolin {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "skills.update".into(),
                data: None,
                error: Some(json!({"code": 403, "message": "cannot update skills owned by another tool"})),
            },
        )
        .await;
        return;
    }

    let skill_md_path = skill.source_path.clone();
    match std::fs::write(&skill_md_path, &params.content) {
        Ok(()) => {
            let _ = state.reload_skills();
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "skills.update".into(),
                    data: Some(json!({"updated": true, "skill_id": params.skill_id})),
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
                    msg_type: "skills.update".into(),
                    data: None,
                    error: Some(json!({"code": 500, "message": format!("failed to write skill: {}", e)})),
                },
            )
            .await;
        }
    }
}

/// Delete a skill directory from disk (XiaoLin-owned only).
pub async fn handle_skills_delete(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    params: SkillsDeleteParams,
) {
    let registry = state.skill_registry_for("main");

    let skill = match registry.get(&params.skill_id) {
        Some(s) => s,
        None => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "skills.delete".into(),
                    data: None,
                    error: Some(json!({"code": 404, "message": format!("skill '{}' not found", params.skill_id)})),
                },
            )
            .await;
            return;
        }
    };

    let is_xiaolin = skill
        .source
        .as_ref()
        .map(|s| s.origin == SkillOrigin::XiaoLin)
        .unwrap_or(true);

    if !is_xiaolin {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "skills.delete".into(),
                data: None,
                error: Some(json!({"code": 403, "message": "cannot delete skills owned by another tool"})),
            },
        )
        .await;
        return;
    }

    // Delete the entire skill directory (parent of SKILL.md)
    let skill_dir = skill
        .source_path
        .parent()
        .unwrap_or(&skill.source_path);

    match std::fs::remove_dir_all(skill_dir) {
        Ok(()) => {
            let _ = state.reload_skills();
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "skills.delete".into(),
                    data: Some(json!({"deleted": true, "skill_id": params.skill_id})),
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
                    msg_type: "skills.delete".into(),
                    data: None,
                    error: Some(json!({"code": 500, "message": format!("failed to delete skill: {}", e)})),
                },
            )
            .await;
        }
    }
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

/// List evolution skills (candidate + active from SkillStore).
pub async fn handle_evolution_list(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
) {
    let store = state.store.skill_store.clone();

    match store.list_all().await {
        Ok(skills) => {
            let items: Vec<_> = skills
                .iter()
                .map(|s| {
                    json!({
                        "id": s.id,
                        "name": s.name,
                        "task_pattern": s.task_pattern,
                        "strategy_template": s.strategy_template,
                        "status": s.status,
                        "success_rate": s.success_rate,
                        "usage_count": s.usage_count,
                        "source_trajectory_ids": s.source_trajectory_ids,
                        "created_at": s.created_at,
                        "version": s.version,
                    })
                })
                .collect();
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "evolution.list".into(),
                    data: Some(json!({"skills": items, "count": items.len()})),
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
                    msg_type: "evolution.list".into(),
                    data: None,
                    error: Some(json!({"code": 500, "message": format!("failed to list evolution skills: {}", e)})),
                },
            )
            .await;
        }
    }
}

/// Patterns in evolution skill strategy that indicate potentially unsafe content.
/// Presence of these triggers a warning during `evolution.promote`.
const UNSAFE_STRATEGY_PATTERNS: &[&str] = &[
    "shell:",
    "hooks:",
    "shell_exec",
    "execute_command",
];

/// Promote an evolution skill to a static SKILL.md file.
pub async fn handle_evolution_promote(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    skill_id: &str,
) {
    let store = state.store.skill_store.clone();

    let skill = match store.get_skill(skill_id).await {
        Ok(Some(s)) => s,
        Ok(None) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "evolution.promote".into(),
                    data: None,
                    error: Some(json!({"code": 404, "message": format!("evolution skill '{}' not found", skill_id)})),
                },
            )
            .await;
            return;
        }
        Err(e) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "evolution.promote".into(),
                    data: None,
                    error: Some(json!({"code": 500, "message": format!("failed to fetch skill: {}", e)})),
                },
            )
            .await;
            return;
        }
    };

    let sanitized_name = skill
        .name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect::<String>()
        .to_lowercase();
    let skill_dir_name = if sanitized_name.is_empty() {
        skill_id.to_string()
    } else {
        sanitized_name
    };

    let content = format!(
        "---\nname: {}\ndescription: \"Promoted from evolution skill\"\ntags:\n  - evolved\nenabled: true\n---\n\n# {}\n\n**Task Pattern:** {}\n\n## Strategy\n\n{}\n",
        skill.name, skill.name, skill.task_pattern, skill.strategy_template
    );

    let strategy_lower = skill.strategy_template.to_lowercase();
    let unsafe_indicators: Vec<&str> = UNSAFE_STRATEGY_PATTERNS
        .iter()
        .copied()
        .filter(|pattern| strategy_lower.contains(pattern))
        .collect();

    let warning = if unsafe_indicators.is_empty() {
        None
    } else {
        Some(format!(
            "Promoted skill strategy contains potentially unsafe patterns: {}",
            unsafe_indicators.join(", ")
        ))
    };

    match xiaolin_core::workspace::write_global_skill(&skill_dir_name, &content) {
        Ok(path) => {
            let _ = state.reload_skills();
            let mut data = json!({
                "promoted": true,
                "skill_id": skill_id,
                "path": path.display().to_string(),
            });
            if let Some(w) = &warning {
                data["warning"] = json!(w);
            }
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "evolution.promote".into(),
                    data: Some(data),
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
                    msg_type: "evolution.promote".into(),
                    data: None,
                    error: Some(json!({"code": 500, "message": format!("failed to write promoted skill: {}", e)})),
                },
            )
            .await;
        }
    }
}
