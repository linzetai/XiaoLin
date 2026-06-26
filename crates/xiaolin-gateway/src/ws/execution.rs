use axum::extract::ws::{Message, WebSocket};
use serde_json::json;

use crate::state::AppState;

use super::send_resp;
use super::types::WsResponse;

/// Set execution mode for a session (agent vs plan mode).
pub async fn handle_execution_set_mode(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    params: serde_json::Value,
    bg_tx: Option<&tokio::sync::mpsc::Sender<WsResponse>>,
) {
    let Some(mode_str) = params.get("mode").and_then(|v| v.as_str()) else {
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "error".into(),
                data: None,
                error: Some(
                    json!({"code": -32602, "message": "mode required ('agent' or 'plan')"}),
                ),
            },
        )
        .await;
        return;
    };

    let session_id = params
        .get("sessionId")
        .and_then(|v| v.as_str())
        .unwrap_or("default");

    use xiaolin_core::types::ExecutionMode;
    let target = match mode_str {
        "plan" => ExecutionMode::Plan,
        "agent" => ExecutionMode::Agent,
        _ => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "error".into(),
                    data: None,
                    error: Some(json!({"code": -32602, "message": "Invalid mode. Expected 'agent' or 'plan'."})),
                },
            )
            .await;
            return;
        }
    };

    let (from, to) = state.rt.session_modes.transition(session_id, target);

    if from != to {
        let synthetic = match to {
            ExecutionMode::Plan => "[系统: 用户已切换到规划模式]",
            ExecutionMode::Agent => "[系统: 用户已切换到执行模式]",
            ExecutionMode::Coordinator => "[系统: 用户已切换到协调模式]",
        };
        let msg = xiaolin_core::types::ChatMessage {
            role: xiaolin_core::types::Role::User,
            content: Some(serde_json::Value::String(synthetic.to_string())),
            ..Default::default()
        };
        if let Err(e) = state
            .store
            .session_store
            .append_message(session_id, &msg)
            .await
        {
            tracing::warn!(error = %e, session_id, "failed to inject synthetic mode switch message");
        }
    }

    send_resp(
        sender,
        &WsResponse {
            id: req_id,
            msg_type: "execution.set_mode".into(),
            data: Some(json!({"ok": true, "from": format!("{from}"), "to": format!("{to}")})),
            error: None,
        },
    )
    .await;

    if from != to {
        if let Some(tx) = bg_tx {
            let _ = tx
                .send(WsResponse {
                    id: None,
                    msg_type: "mode_change".into(),
                    data: Some(json!({
                        "sessionId": session_id,
                        "from": format!("{from}"),
                        "to": format!("{to}"),
                    })),
                    error: None,
                })
                .await;

            let plan_store = &state.rt.plan_file_store;
            if let Some(plan_path) = plan_store.plan_path_if_exists(session_id) {
                let plan_exists = plan_path.exists();
                let _ = tx
                    .send(WsResponse {
                        id: None,
                        msg_type: "plan_file_update".into(),
                        data: Some(json!({
                            "sessionId": session_id,
                            "path": plan_path.to_string_lossy().to_string(),
                            "exists": plan_exists,
                        })),
                        error: None,
                    })
                    .await;
            }
        }
    }
}

/// Approve plan: transition mode and broadcast events.
/// Supports optional `feedback` (injected as user message before transition)
/// and `clearContext` (creates a new session with the plan injected).
pub async fn handle_execution_approve_plan(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    params: serde_json::Value,
    bg_tx: &tokio::sync::mpsc::Sender<WsResponse>,
) {
    let session_id = params
        .get("sessionId")
        .and_then(|v| v.as_str())
        .unwrap_or("default");

    let mode_str = params
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("agent");

    let feedback = params
        .get("feedback")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());

    let clear_context = params
        .get("clearContext")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    use xiaolin_core::types::ExecutionMode;
    let target = match mode_str {
        "plan" => ExecutionMode::Plan,
        "agent" => ExecutionMode::Agent,
        _ => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "error".into(),
                    data: None,
                    error: Some(json!({"code": -32602, "message": "Invalid mode. Expected 'agent' or 'plan'."})),
                },
            )
            .await;
            return;
        }
    };

    if let Some(text) = feedback {
        let msg = xiaolin_core::types::ChatMessage {
            role: xiaolin_core::types::Role::User,
            content: Some(serde_json::Value::String(text.to_string())),
            ..Default::default()
        };
        if let Err(e) = state
            .store
            .session_store
            .append_message(session_id, &msg)
            .await
        {
            tracing::warn!(error = %e, session_id, "failed to inject approval feedback");
        }
    }

    if clear_context {
        let old_session = state
            .store
            .session_store
            .get_session(session_id)
            .await
            .ok()
            .flatten();
        let agent_id = old_session
            .as_ref()
            .map(|s| s.agent_id.as_str())
            .unwrap_or("main");
        let work_dir = old_session.as_ref().and_then(|s| s.work_dir.as_deref());

        let new_id = uuid::Uuid::new_v4().to_string();
        match state
            .store
            .session_store
            .create_session_with_work_dir(&new_id, agent_id, None, work_dir)
            .await
        {
            Ok(_) => {
                if let Some(slug) = state.rt.plan_file_store.get_slug(session_id) {
                    state.rt.plan_file_store.set_slug(&new_id, &slug);
                }

                if let Some(plan_content) = state.rt.plan_file_store.read_plan(session_id) {
                    let guidance = build_approval_guidance(&plan_content);
                    let context_msg = format!("[Plan Context]\n\n{plan_content}\n\n{guidance}");
                    let plan_msg = xiaolin_core::types::ChatMessage {
                        role: xiaolin_core::types::Role::User,
                        content: Some(serde_json::Value::String(context_msg)),
                        ..Default::default()
                    };
                    let _ = state
                        .store
                        .session_store
                        .append_message(&new_id, &plan_msg)
                        .await;
                }

                {
                    let mode_state = state.rt.session_modes.get_or_create(&new_id);
                    mode_state.transition(xiaolin_core::types::ExecutionMode::Plan);
                    mode_state.transition(target);
                }

                send_resp(
                    sender,
                    &WsResponse {
                        id: req_id,
                        msg_type: "execution.approve_plan".into(),
                        data: Some(json!({
                            "ok": true,
                            "from": "plan",
                            "to": format!("{target}"),
                            "newSessionId": new_id,
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
                        error: Some(
                            json!({"message": format!("Failed to create new session: {e}")}),
                        ),
                    },
                )
                .await;
            }
        }
        return;
    }

    let (from, to) = state.rt.session_modes.transition(session_id, target);

    if from != to && from == xiaolin_core::types::ExecutionMode::Plan {
        if let Some(plan_content) = state.rt.plan_file_store.read_plan(session_id) {
            let guidance = build_approval_guidance(&plan_content);
            let msg = xiaolin_core::types::ChatMessage {
                role: xiaolin_core::types::Role::User,
                content: Some(serde_json::Value::String(guidance)),
                ..Default::default()
            };
            if let Err(e) = state
                .store
                .session_store
                .append_message(session_id, &msg)
                .await
            {
                tracing::warn!(error = %e, session_id, "failed to inject approval guidance");
            }
        }
    }

    send_resp(
        sender,
        &WsResponse {
            id: req_id,
            msg_type: "execution.approve_plan".into(),
            data: Some(json!({"ok": true, "from": format!("{from}"), "to": format!("{to}")})),
            error: None,
        },
    )
    .await;

    if from != to {
        let _ = bg_tx
            .send(WsResponse {
                id: None,
                msg_type: "mode_change".into(),
                data: Some(json!({
                    "sessionId": session_id,
                    "from": format!("{from}"),
                    "to": format!("{to}"),
                })),
                error: None,
            })
            .await;

        let plan_store = &state.rt.plan_file_store;
        if let Some(plan_path) = plan_store.plan_path_if_exists(session_id) {
            let plan_exists = plan_path.exists();
            let _ = bg_tx
                .send(WsResponse {
                    id: None,
                    msg_type: "plan_file_update".into(),
                    data: Some(json!({
                        "sessionId": session_id,
                        "path": plan_path.to_string_lossy().to_string(),
                        "exists": plan_exists,
                    })),
                    error: None,
                })
                .await;
        }
    }
}

/// Get plan file content for a session.
pub async fn handle_execution_get_plan(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    params: serde_json::Value,
) {
    let session_id = params
        .get("sessionId")
        .and_then(|v| v.as_str())
        .unwrap_or("default");

    let plan_store = &state.rt.plan_file_store;
    let (path_str, exists, content) = match plan_store.plan_path_if_exists(session_id) {
        Some(path) => {
            let e = path.exists();
            let c = if e {
                std::fs::read_to_string(&path).ok()
            } else {
                None
            };
            (path.to_string_lossy().to_string(), e, c)
        }
        None => (String::new(), false, None),
    };

    send_resp(
        sender,
        &WsResponse {
            id: req_id,
            msg_type: "execution.get_plan".into(),
            data: Some(json!({
                "path": path_str,
                "content": content,
                "exists": exists,
            })),
            error: None,
        },
    )
    .await;
}

/// Reject plan: stay in Plan mode, optionally inject feedback as user message.
pub async fn handle_execution_reject_plan(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    params: serde_json::Value,
    _bg_tx: &tokio::sync::mpsc::Sender<WsResponse>,
) {
    let session_id = params
        .get("sessionId")
        .and_then(|v| v.as_str())
        .unwrap_or("default");

    let feedback = params
        .get("feedback")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());

    let mut feedback_injected = false;
    if let Some(text) = feedback {
        let msg = xiaolin_core::types::ChatMessage {
            role: xiaolin_core::types::Role::User,
            content: Some(serde_json::Value::String(format!("[Plan Feedback] {text}"))),
            ..Default::default()
        };
        match state
            .store
            .session_store
            .append_message(session_id, &msg)
            .await
        {
            Ok(()) => {
                feedback_injected = true;
                tracing::info!(session_id, "plan rejected with feedback injected");
            }
            Err(e) => {
                tracing::warn!(error = %e, session_id, "failed to inject plan feedback");
            }
        }
    }

    send_resp(
        sender,
        &WsResponse {
            id: req_id,
            msg_type: "execution.reject_plan".into(),
            data: Some(json!({
                "ok": true,
                "feedbackInjected": feedback_injected,
            })),
            error: None,
        },
    )
    .await;
}

/// Return plan metadata for a session: execution mode, plan file path/exists.
/// Used by the frontend to hydrate state after refresh/reconnect/session switch.
pub async fn handle_execution_get_plan_meta(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    params: serde_json::Value,
) {
    let session_id = params
        .get("sessionId")
        .and_then(|v| v.as_str())
        .unwrap_or("default");

    let registry_mode = state.rt.session_modes.current_mode(session_id);

    let mode = if registry_mode == xiaolin_core::types::ExecutionMode::Agent {
        let inferred = infer_mode_from_messages(&state.store.session_store, session_id).await;
        if inferred != xiaolin_core::types::ExecutionMode::Agent {
            state.rt.session_modes.transition(session_id, inferred);
        }
        inferred
    } else {
        registry_mode
    };

    let plan_store = &state.rt.plan_file_store;
    let (plan_file_path, plan_file_exists) = match plan_store.plan_path_if_exists(session_id) {
        Some(path) => {
            let exists = path.exists();
            (Some(path.to_string_lossy().to_string()), exists)
        }
        None => (None, false),
    };

    let mode_str = match mode {
        xiaolin_core::types::ExecutionMode::Plan => "plan",
        xiaolin_core::types::ExecutionMode::Coordinator => "coordinator",
        _ => "agent",
    };

    send_resp(
        sender,
        &WsResponse {
            id: req_id,
            msg_type: "execution.get_plan_meta".into(),
            data: Some(json!({
                "executionMode": mode_str,
                "planFilePath": plan_file_path,
                "planFileExists": plan_file_exists,
            })),
            error: None,
        },
    )
    .await;
}

/// Infer execution mode from the most recent messages in a session.
/// Scans the last few messages for synthetic mode-switch markers or
/// plan tool calls to determine the mode the session was likely in.
async fn infer_mode_from_messages(
    session_store: &xiaolin_session::SessionStore,
    session_id: &str,
) -> xiaolin_core::types::ExecutionMode {
    let tail = match session_store.load_tail_messages(session_id, 10).await {
        Ok(msgs) => msgs,
        Err(e) => {
            tracing::warn!(error = %e, session_id, "failed to load messages for mode inference");
            return xiaolin_core::types::ExecutionMode::Agent;
        }
    };

    for msg in tail.iter().rev() {
        if let Some(text) = &msg.content {
            if text.contains("[系统: 用户已切换到规划模式]") {
                return xiaolin_core::types::ExecutionMode::Plan;
            }
            if text.contains("[系统: 用户已切换到执行模式]") {
                return xiaolin_core::types::ExecutionMode::Agent;
            }
        }

        if let Some(tc_json) = &msg.tool_calls_json {
            if let Ok(tool_calls) = serde_json::from_str::<Vec<serde_json::Value>>(tc_json) {
                for tc in &tool_calls {
                    let name = tc
                        .get("function")
                        .and_then(|f| f.get("name"))
                        .and_then(|n| n.as_str())
                        .unwrap_or("");
                    if name == "enter_plan_mode" {
                        return xiaolin_core::types::ExecutionMode::Plan;
                    }
                    if name == "exit_plan_mode" {
                        return xiaolin_core::types::ExecutionMode::Agent;
                    }
                }
            }
        }
    }

    xiaolin_core::types::ExecutionMode::Agent
}

/// Build a guidance message to inject after plan approval.
fn build_approval_guidance(plan_content: &str) -> String {
    use xiaolin_agent::runtime::post_compact_restore::parse_plan_progress;

    let mut guidance = String::from(
        "[Plan Approved — Implementation Guidance]\n\n\
         The user has approved the plan. Start implementing step by step.\n\n",
    );

    if let Some(p) = parse_plan_progress(plan_content) {
        guidance.push_str(&format!(
            "Plan has {} steps ({} completed, {} in progress, {} pending).\n",
            p.total, p.completed, p.in_progress, p.pending,
        ));
        if let Some(ref next) = p.next_step {
            guidance.push_str(&format!("Start with: {next}\n"));
        }
    }

    guidance.push_str(
        "\nWorkflow:\n\
         1. Call `update_plan` at the start to set the first step to `in_progress`\n\
         2. Implement each step, then call `update_plan` to mark it `completed`\n\
         3. After all steps, run the verification described in the plan\n",
    );

    guidance
}
