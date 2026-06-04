use axum::extract::ws::{Message, WebSocket};
use futures::stream::SplitSink;
use serde_json::json;
use std::path::PathBuf;

use crate::state::AppState;
use xiaolin_tools_fs::git;

use super::send_resp;
use super::types::WsResponse;

async fn resolve_project_dir(state: &AppState, project_id: &str) -> Result<PathBuf, String> {
    let project = state
        .store
        .session_store
        .get_project(project_id)
        .await
        .map_err(|e| format!("db error: {e}"))?
        .ok_or_else(|| format!("project not found: {project_id}"))?;
    Ok(PathBuf::from(project.root_path))
}

fn error_resp(req_id: Option<String>, msg: &str) -> WsResponse {
    WsResponse {
        id: req_id,
        msg_type: "error".into(),
        data: None,
        error: Some(json!({"code": -32000, "message": msg})),
    }
}

pub async fn handle_git_status(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    project_id: &str,
) {
    let dir = match resolve_project_dir(state, project_id).await {
        Ok(d) => d,
        Err(e) => {
            send_resp(sender, &error_resp(req_id, &e)).await;
            return;
        }
    };

    match git::git_status(&dir).await {
        Ok(status) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "git.status".into(),
                    data: Some(serde_json::to_value(&status).unwrap_or_default()),
                    error: None,
                },
            )
            .await;
        }
        Err(e) => {
            send_resp(sender, &error_resp(req_id, &e)).await;
        }
    }
}

pub async fn handle_git_diff(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    project_id: &str,
    path: &str,
    staged: bool,
) {
    let dir = match resolve_project_dir(state, project_id).await {
        Ok(d) => d,
        Err(e) => {
            send_resp(sender, &error_resp(req_id, &e)).await;
            return;
        }
    };

    match git::file_diff(&dir, path, staged).await {
        Ok(hunks) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "git.diff".into(),
                    data: Some(json!({ "hunks": hunks })),
                    error: None,
                },
            )
            .await;
        }
        Err(e) => {
            send_resp(sender, &error_resp(req_id, &e)).await;
        }
    }
}

pub async fn handle_git_branches(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    project_id: &str,
) {
    let dir = match resolve_project_dir(state, project_id).await {
        Ok(d) => d,
        Err(e) => {
            send_resp(sender, &error_resp(req_id, &e)).await;
            return;
        }
    };

    match git::branch_list(&dir).await {
        Ok(branches) => {
            let current = git::current_branch(&dir).await.unwrap_or_default();
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "git.branches".into(),
                    data: Some(json!({ "branches": branches, "current": current })),
                    error: None,
                },
            )
            .await;
        }
        Err(e) => {
            send_resp(sender, &error_resp(req_id, &e)).await;
        }
    }
}

pub async fn handle_git_log(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    project_id: &str,
    limit: u32,
) {
    let dir = match resolve_project_dir(state, project_id).await {
        Ok(d) => d,
        Err(e) => {
            send_resp(sender, &error_resp(req_id, &e)).await;
            return;
        }
    };

    match git::git_log(&dir, limit).await {
        Ok(commits) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "git.log".into(),
                    data: Some(json!({ "commits": commits })),
                    error: None,
                },
            )
            .await;
        }
        Err(e) => {
            send_resp(sender, &error_resp(req_id, &e)).await;
        }
    }
}

pub async fn handle_git_stage(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    project_id: &str,
    files: &[String],
) {
    let dir = match resolve_project_dir(state, project_id).await {
        Ok(d) => d,
        Err(e) => {
            send_resp(sender, &error_resp(req_id, &e)).await;
            return;
        }
    };

    match git::git_stage(&dir, files).await {
        Ok(()) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "git.stage".into(),
                    data: Some(json!({"success": true})),
                    error: None,
                },
            )
            .await;
            state
                .strm
                .git_watcher_manager
                .trigger_refresh(project_id, &dir)
                .await;
        }
        Err(e) => {
            send_resp(sender, &error_resp(req_id, &e)).await;
        }
    }
}

pub async fn handle_git_unstage(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    project_id: &str,
    files: &[String],
) {
    let dir = match resolve_project_dir(state, project_id).await {
        Ok(d) => d,
        Err(e) => {
            send_resp(sender, &error_resp(req_id, &e)).await;
            return;
        }
    };

    match git::git_unstage(&dir, files).await {
        Ok(()) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "git.unstage".into(),
                    data: Some(json!({"success": true})),
                    error: None,
                },
            )
            .await;
            state
                .strm
                .git_watcher_manager
                .trigger_refresh(project_id, &dir)
                .await;
        }
        Err(e) => {
            send_resp(sender, &error_resp(req_id, &e)).await;
        }
    }
}

pub async fn handle_git_commit(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    project_id: &str,
    message: &str,
) {
    let dir = match resolve_project_dir(state, project_id).await {
        Ok(d) => d,
        Err(e) => {
            send_resp(sender, &error_resp(req_id, &e)).await;
            return;
        }
    };

    match git::git_commit(&dir, message).await {
        Ok(result) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "git.commit".into(),
                    data: Some(serde_json::to_value(&result).unwrap_or_default()),
                    error: None,
                },
            )
            .await;
            state
                .strm
                .git_watcher_manager
                .trigger_refresh(project_id, &dir)
                .await;
        }
        Err(e) => {
            send_resp(sender, &error_resp(req_id, &e)).await;
        }
    }
}

pub async fn handle_git_revert(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    project_id: &str,
    files: &[String],
) {
    let dir = match resolve_project_dir(state, project_id).await {
        Ok(d) => d,
        Err(e) => {
            send_resp(sender, &error_resp(req_id, &e)).await;
            return;
        }
    };

    match git::git_revert_files(&dir, files).await {
        Ok(()) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "git.revert".into(),
                    data: Some(json!({"success": true})),
                    error: None,
                },
            )
            .await;
            state
                .strm
                .git_watcher_manager
                .trigger_refresh(project_id, &dir)
                .await;
        }
        Err(e) => {
            send_resp(sender, &error_resp(req_id, &e)).await;
        }
    }
}

pub async fn handle_git_init(
    sender: &mut SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    project_id: &str,
) {
    let dir = match resolve_project_dir(state, project_id).await {
        Ok(d) => d,
        Err(e) => {
            send_resp(sender, &error_resp(req_id, &e)).await;
            return;
        }
    };

    match git::run_git(&dir, &["init"]).await {
        Ok(_) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "git.init".into(),
                    data: Some(json!({"success": true})),
                    error: None,
                },
            )
            .await;
            state
                .strm
                .git_watcher_manager
                .ensure_watcher(project_id, &dir)
                .await;
            state
                .strm
                .git_watcher_manager
                .trigger_refresh(project_id, &dir)
                .await;
        }
        Err(e) => {
            send_resp(sender, &error_resp(req_id, &e)).await;
        }
    }
}
