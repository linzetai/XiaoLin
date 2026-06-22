use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use xiaolin_core::history_compat;
use xiaolin_core::types::ChatMessage;

use crate::state::AppState;

use super::error::AppError;

#[derive(Deserialize)]
pub(super) struct PaginationParams {
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    20
}

pub(super) async fn list_sessions(
    State(state): State<AppState>,
    Query(params): Query<PaginationParams>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let sessions = state
        .store
        .session_store
        .list_sessions(params.limit, params.offset)
        .await?;
    Ok(Json(json!({ "sessions": sessions })))
}

pub(super) async fn get_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let session = state
        .store
        .session_store
        .get_session(&session_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("session not found: {session_id}")))?;
    Ok(Json(json!(session)))
}

pub(super) async fn delete_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let deleted = state
        .store
        .session_store
        .delete_session(&session_id)
        .await?;
    if deleted {
        state.cleanup_session_plan_state(&session_id);
        Ok(Json(json!({ "deleted": true })))
    } else {
        Err(AppError::NotFound(format!(
            "session not found: {session_id}"
        )))
    }
}

pub(super) async fn get_session_messages(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let messages = state.store.session_store.load_messages(&session_id).await?;
    Ok(Json(json!({ "messages": messages })))
}

/// Resolved session metadata.
pub struct ResolvedSession {
    pub session_id: String,
    pub messages: Vec<ChatMessage>,
    /// `true` when this session has no title yet (needs auto-titling).
    pub needs_title: bool,
}

/// Resolve or create a session. Returns session ID, context messages, and whether
/// the session still needs a title — avoiding a separate `get_session` call later.
pub async fn resolve_session_context(
    state: &AppState,
    session_id: Option<&str>,
    agent_id: &str,
) -> anyhow::Result<ResolvedSession> {
    if let Some(sid) = session_id {
        if let Some(session) = state.store.session_store.get_session(sid).await? {
            let history_items = state.store.session_store.load_history(sid).await?;
            let messages = if history_items.is_empty() {
                let arc = state.store.session_store.load_chat_messages(sid).await?;
                std::sync::Arc::try_unwrap(arc).unwrap_or_else(|a| (*a).clone())
            } else {
                history_compat::history_items_to_chat_messages(&history_items)
            };
            return Ok(ResolvedSession {
                session_id: sid.to_string(),
                messages,
                needs_title: session.title.is_none(),
            });
        }
    }

    let new_id = session_id
        .map(String::from)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    state
        .store
        .session_store
        .create_session(&new_id, agent_id, None)
        .await?;

    let mut messages = Vec::new();
    state
        .store
        .context_engine
        .bootstrap(&mut messages, agent_id)
        .await?;
    Ok(ResolvedSession {
        session_id: new_id,
        messages,
        needs_title: true,
    })
}

