use axum::extract::{Path, Query, State};
use axum::Extension;
use axum::Json;
use serde::Deserialize;
use serde_json::json;

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
    let deleted = state.session_store.delete_session(&session_id).await?;
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
    let messages = state.session_store.load_messages(&session_id).await?;
    Ok(Json(json!({ "messages": messages })))
}

/// Resolve or create a session. Returns (session_id, context_messages).
/// Compaction and assemble-phase hooks run in [`crate::chat_pipeline::setup_chat`] once the
/// inbound user turn is merged into the outbound message list.
pub async fn resolve_session_context(
    state: &AppState,
    session_id: Option<&str>,
    agent_id: &str,
) -> anyhow::Result<(String, Vec<ChatMessage>)> {
    if let Some(sid) = session_id {
        if let Some(_session) = state.session_store.get_session(sid).await? {
            let history = state.session_store.load_chat_messages(sid).await?;
            return Ok((sid.to_string(), history));
        }
    }

    let new_id = session_id
        .map(String::from)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    state
        .session_store
        .create_session(&new_id, agent_id, None)
        .await?;

    let mut messages = Vec::new();
    state
        .context_engine
        .bootstrap(&mut messages, agent_id)
        .await?;
    Ok((new_id, messages))
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
