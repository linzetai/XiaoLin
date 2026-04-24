use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use crate::extract::AppJson;
use crate::state::AppState;

use super::error::AppError;
use super::session::PaginationParams;

fn get_agent_memory<'a>(
    state: &'a AppState,
    agent_id: Option<&str>,
) -> Option<(
    &'a std::sync::Arc<fastclaw_memory::EpisodicMemory>,
    &'a std::sync::Arc<fastclaw_memory::SemanticMemory>,
)> {
    let aid = agent_id.unwrap_or("main");
    let ep = state.agent_episodic.get(aid)?;
    let sem = state.agent_semantic.get(aid)?;
    Some((ep, sem))
}

#[derive(Deserialize)]
pub(super) struct AgentQuery {
    #[serde(default = "default_agent_id")]
    pub agent_id: String,
}

fn default_agent_id() -> String {
    "main".to_string()
}

fn validate_agent_id(agent_id: &str) -> Result<(), AppError> {
    if agent_id.is_empty() || agent_id.len() > 128 {
        return Err(AppError::BadRequest(
            "agent_id must be 1–128 characters".into(),
        ));
    }
    if !agent_id
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(AppError::BadRequest(
            "agent_id contains disallowed characters; only [a-zA-Z0-9._-] permitted".into(),
        ));
    }
    Ok(())
}

pub(super) async fn list_episodes(
    State(state): State<AppState>,
    Query(params): Query<PaginationParams>,
    Query(agent): Query<AgentQuery>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    validate_agent_id(&agent.agent_id)?;
    let (ep, _) = get_agent_memory(&state, Some(&agent.agent_id))
        .ok_or_else(|| AppError::NotFound(format!("agent '{}' not found", agent.agent_id)))?;
    let episodes = ep.recent(Some(&agent.agent_id), params.limit).await?;
    Ok(Json(
        json!({ "episodes": episodes, "agent_id": agent.agent_id }),
    ))
}

#[derive(Deserialize)]
pub(super) struct SearchParams {
    pub q: String,
    pub limit: Option<i64>,
}

#[derive(Deserialize)]
pub(super) struct FactUpsertBody {
    pub id: String,
    pub category: String,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub confidence: f32,
    #[serde(default)]
    pub source_session: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

pub(super) async fn search_episodes(
    State(state): State<AppState>,
    Query(params): Query<SearchParams>,
    Query(agent): Query<AgentQuery>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    validate_agent_id(&agent.agent_id)?;
    let (ep, _) = get_agent_memory(&state, Some(&agent.agent_id))
        .ok_or_else(|| AppError::NotFound(format!("agent '{}' not found", agent.agent_id)))?;
    let episodes = ep.search(&params.q, params.limit.unwrap_or(20)).await?;
    Ok(Json(
        json!({ "episodes": episodes, "agent_id": agent.agent_id }),
    ))
}

pub(super) async fn list_facts(
    State(state): State<AppState>,
    Query(params): Query<PaginationParams>,
    Query(agent): Query<AgentQuery>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    validate_agent_id(&agent.agent_id)?;
    let (_, sem) = get_agent_memory(&state, Some(&agent.agent_id))
        .ok_or_else(|| AppError::NotFound(format!("agent '{}' not found", agent.agent_id)))?;
    let facts = sem.list(params.offset, params.limit).await?;
    Ok(Json(json!({ "facts": facts, "agent_id": agent.agent_id })))
}

pub(super) async fn search_facts(
    State(state): State<AppState>,
    Query(params): Query<SearchParams>,
    Query(agent): Query<AgentQuery>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    validate_agent_id(&agent.agent_id)?;
    let (_, sem) = get_agent_memory(&state, Some(&agent.agent_id))
        .ok_or_else(|| AppError::NotFound(format!("agent '{}' not found", agent.agent_id)))?;
    let facts = sem.search(&params.q, params.limit.unwrap_or(20)).await?;
    Ok(Json(json!({ "facts": facts, "agent_id": agent.agent_id })))
}

pub(super) async fn upsert_fact(
    State(state): State<AppState>,
    Query(agent): Query<AgentQuery>,
    AppJson(body): AppJson<FactUpsertBody>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    validate_agent_id(&agent.agent_id)?;
    let (_, sem) = get_agent_memory(&state, Some(&agent.agent_id))
        .ok_or_else(|| AppError::NotFound(format!("agent '{}' not found", agent.agent_id)))?;
    tracing::info!(
        agent_id = %agent.agent_id,
        fact_id = %body.id,
        "memory:upsert_fact"
    );
    let now = chrono::Utc::now().to_rfc3339();
    let fact = fastclaw_memory::Fact {
        id: body.id,
        category: body.category,
        subject: body.subject,
        predicate: body.predicate,
        object: body.object,
        confidence: body.confidence,
        source_session: body.source_session,
        created_at: body.created_at.unwrap_or_else(|| now.clone()),
        updated_at: body.updated_at.unwrap_or(now),
    };
    sem.upsert(&fact).await?;
    Ok(Json(json!({ "ok": true, "agent_id": agent.agent_id })))
}

pub(super) async fn delete_fact(
    State(state): State<AppState>,
    Path(fact_id): Path<String>,
    Query(agent): Query<AgentQuery>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    validate_agent_id(&agent.agent_id)?;
    let (_, sem) = get_agent_memory(&state, Some(&agent.agent_id))
        .ok_or_else(|| AppError::NotFound(format!("agent '{}' not found", agent.agent_id)))?;
    tracing::info!(
        agent_id = %agent.agent_id,
        fact_id = %fact_id,
        "memory:delete_fact"
    );
    let deleted = sem.delete(&fact_id).await?;
    Ok(Json(
        json!({ "deleted": deleted, "agent_id": agent.agent_id }),
    ))
}

/// Auto-record a lightweight episode from the assistant's response.
pub async fn auto_record_episode(
    state: &AppState,
    session_id: &str,
    agent_id: &str,
    content: &str,
) {
    if !state.config.memory.enabled {
        return;
    }
    let summary = if content.len() > 200 {
        let end = content
            .char_indices()
            .map(|(i, _)| i)
            .take_while(|&i| i <= 200)
            .last()
            .unwrap_or(0);
        format!("{}...", &content[..end])
    } else {
        content.to_string()
    };

    let importance = fastclaw_memory::ImportanceScorer::score_single(&summary);

    let episode = fastclaw_memory::Episode {
        id: uuid::Uuid::new_v4().to_string(),
        session_id: session_id.to_string(),
        agent_id: agent_id.to_string(),
        summary,
        importance,
        tags: String::new(),
        created_at: chrono::Utc::now().to_rfc3339(),
        dreamed_at: None,
    };

    if let Some(ep) = state.agent_episodic.get(agent_id) {
        if let Err(e) = ep.record(&episode).await {
            tracing::warn!(error = %e, "failed to auto-record episode");
        }
    }
}
