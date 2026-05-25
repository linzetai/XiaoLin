use axum::extract::{Path, Query, State};
use axum::Extension;
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use fastclaw_core::history_compat;
use fastclaw_core::types::ChatMessage;
use fastclaw_security::ApiKeyAuth;

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

fn ensure_session_http_auth(_auth: &ApiKeyAuth) -> Result<(), AppError> {
    Ok(())
}

pub(super) async fn list_sessions(
    State(state): State<AppState>,
    Extension(auth): Extension<ApiKeyAuth>,
    Query(params): Query<PaginationParams>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    ensure_session_http_auth(&auth)?;
    let sessions = state
        .store
        .session_store
        .list_sessions(params.limit, params.offset)
        .await?;
    Ok(Json(json!({ "sessions": sessions })))
}

pub(super) async fn get_session(
    State(state): State<AppState>,
    Extension(auth): Extension<ApiKeyAuth>,
    Path(session_id): Path<String>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    ensure_session_http_auth(&auth)?;
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
    Extension(auth): Extension<ApiKeyAuth>,
    Path(session_id): Path<String>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    ensure_session_http_auth(&auth)?;
    let deleted = state
        .store
        .session_store
        .delete_session(&session_id)
        .await?;
    if deleted {
        Ok(Json(json!({ "deleted": true })))
    } else {
        Err(AppError::NotFound(format!(
            "session not found: {session_id}"
        )))
    }
}

pub(super) async fn get_session_messages(
    State(state): State<AppState>,
    Extension(auth): Extension<ApiKeyAuth>,
    Path(session_id): Path<String>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    ensure_session_http_auth(&auth)?;
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
                state.store.session_store.load_chat_messages(sid).await?
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

    let work_dir = state
        .rt
        .workspaces
        .get(agent_id)
        .map(|ws| ws.root.to_string_lossy().to_string());
    state
        .store
        .session_store
        .create_session_with_work_dir(&new_id, agent_id, None, work_dir.as_deref())
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

#[cfg(test)]
mod tests {
    use super::*;
    use fastclaw_security::AuthConfig;

    #[test]
    fn session_http_auth_allows_when_auth_disabled() {
        let auth = ApiKeyAuth::new(&AuthConfig {
            enabled: false,
            api_keys: vec![],
        });
        assert!(ensure_session_http_auth(&auth).is_ok());
    }

    #[test]
    fn session_http_auth_allows_when_auth_enabled() {
        let auth = ApiKeyAuth::new(&AuthConfig {
            enabled: true,
            api_keys: vec!["k".to_string()],
        });
        assert!(ensure_session_http_auth(&auth).is_ok());
    }
}
