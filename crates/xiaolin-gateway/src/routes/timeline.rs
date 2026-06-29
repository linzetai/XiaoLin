//! Timeline HTTP endpoints.
//!
//! Query access to the canonical turn timeline and materialized display nodes.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Json;
use serde::Deserialize;

use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct PaginationParams {
    pub after_seq: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct ErrorResponse {
    code: u16,
    message: String,
}

impl ErrorResponse {
    fn internal(msg: impl Into<String>) -> (StatusCode, Json<Self>) {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(Self {
                code: 500,
                message: msg.into(),
            }),
        )
    }
}

// ── Timeline event endpoints ────────────────────────────────────────────────

pub async fn get_session_timeline(
    Path(session_id): Path<String>,
    Query(params): Query<PaginationParams>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    let events = state
        .store
        .timeline_store
        .query_by_session(&session_id, params.after_seq, params.limit)
        .await
        .map_err(|e| ErrorResponse::internal(format!("failed to query timeline: {e}")))?;

    Ok(Json(serde_json::json!({
        "session_id": session_id,
        "events": events,
        "count": events.len(),
    })))
}

pub async fn get_timeline_max_seq(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    let max_seq = state
        .store
        .timeline_store
        .max_seq(&session_id)
        .await
        .map_err(|e| ErrorResponse::internal(format!("failed to get max seq: {e}")))?;

    Ok(Json(serde_json::json!({
        "session_id": session_id,
        "max_seq": max_seq,
    })))
}

pub async fn get_turn_timeline(
    Path((session_id, turn_id)): Path<(String, String)>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    let events = state
        .store
        .timeline_store
        .query_by_turn(&session_id, &turn_id)
        .await
        .map_err(|e| ErrorResponse::internal(format!("failed to query turn timeline: {e}")))?;

    Ok(Json(serde_json::json!({
        "session_id": session_id,
        "turn_id": turn_id,
        "events": events,
        "count": events.len(),
    })))
}

// ── Display node endpoints ──────────────────────────────────────────────────

pub async fn get_session_display_nodes(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    let nodes = state
        .store
        .timeline_store
        .materialize_display_nodes(&session_id)
        .await
        .map_err(|e| {
            ErrorResponse::internal(format!("failed to materialize display nodes: {e}"))
        })?;

    Ok(Json(serde_json::json!({
        "session_id": session_id,
        "nodes": nodes,
        "count": nodes.len(),
    })))
}

pub async fn get_turn_display_nodes(
    Path((session_id, turn_id)): Path<(String, String)>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    let nodes = state
        .store
        .timeline_store
        .materialize_display_nodes_for_turn(&session_id, &turn_id)
        .await
        .map_err(|e| {
            ErrorResponse::internal(format!("failed to materialize display nodes: {e}"))
        })?;

    Ok(Json(serde_json::json!({
        "session_id": session_id,
        "turn_id": turn_id,
        "nodes": nodes,
        "count": nodes.len(),
    })))
}

// ── Tool output detail endpoint ─────────────────────────────────────────────

const TOOL_OUTPUT_DETAIL_MAX_BYTES: usize = 64 * 1024;

#[derive(Debug, Deserialize)]
pub struct ToolOutputParams {
    pub range_start: Option<i64>,
    pub range_end: Option<i64>,
    pub tail_lines: Option<usize>,
}

/// GET /sessions/:session_id/tool-output/:handle
///
/// UI-authorized read-only view over an existing ToolOutputAsset.
/// Never returns an unbounded full blob; supports ranged reads,
/// tail reads, and bounded head reads with continuation metadata.
pub async fn get_tool_output_detail(
    Path((session_id, handle)): Path<(String, String)>,
    Query(params): Query<ToolOutputParams>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    use xiaolin_session::tool_output_store::ToolOutputAssetStore;

    let pool = state.store.session_store.pool();
    let store = ToolOutputAssetStore::open(pool)
        .await
        .map_err(|e| ErrorResponse::internal(format!("failed to open output store: {e}")))?;

    let asset = store.get_asset(&handle, &session_id).await.map_err(|e| {
        let msg = format!("{e:?}");
        if msg.contains("NotFound") || msg.contains("Not found") {
            let code = 404u16;
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    code,
                    message: format!("tool output not found: {handle}"),
                }),
            )
        } else if msg.contains("Expired") {
            (
                StatusCode::GONE,
                Json(ErrorResponse {
                    code: 410,
                    message: format!("tool output expired: {handle}"),
                }),
            )
        } else if msg.contains("Unauthorized") {
            (
                StatusCode::FORBIDDEN,
                Json(ErrorResponse {
                    code: 403,
                    message: format!("unauthorized access: {handle}"),
                }),
            )
        } else {
            ErrorResponse::internal(format!("failed to get asset: {e}"))
        }
    })?;

    let metadata = serde_json::json!({
        "handle": &handle,
        "session_id": &session_id,
        "tool_name": &asset.tool_name,
        "tool_call_id": &asset.tool_call_id,
        "byte_count": asset.byte_count,
        "line_count": asset.line_count,
        "size_class": format!("{:?}", asset.size_class),
    });

    // Ranged read
    if let (Some(start), Some(end)) = (params.range_start, params.range_end) {
        let s = (start.max(0) as usize).min(asset.byte_count);
        let requested_end = (end.max(0) as usize).min(asset.byte_count);
        let e = requested_end.min(s.saturating_add(TOOL_OUTPUT_DETAIL_MAX_BYTES));
        let content = if s < e {
            store
                .read_blob_range(&asset, &session_id, s, e)
                .await
                .map_err(|e| ErrorResponse::internal(format!("read range: {e}")))?
        } else {
            String::new()
        };
        let truncated = e < asset.byte_count;
        return Ok(Json(serde_json::json!({
            "metadata": metadata,
            "range": { "start": s, "end": e },
            "content": content,
            "truncated": truncated,
            "total_bytes": asset.byte_count,
            "continuation": if truncated {
                Some(serde_json::json!({"next_offset": e, "hint": "increment range_start/range_end"}))
            } else { None }
        })));
    }

    // Tail read
    if let Some(tail_lines) = params.tail_lines {
        let limit = tail_lines.max(1).min(1000);
        let start = asset
            .byte_count
            .saturating_sub(TOOL_OUTPUT_DETAIL_MAX_BYTES);
        let end = asset.byte_count;
        let tail_window = if start < end {
            store
                .read_blob_range(&asset, &session_id, start, end)
                .await
                .map_err(|e| ErrorResponse::internal(format!("read tail range: {e}")))?
        } else {
            String::new()
        };
        let all_lines: Vec<&str> = tail_window.lines().collect();
        let content = if all_lines.len() > limit {
            all_lines[all_lines.len() - limit..].join("\n")
        } else {
            tail_window
        };
        let truncated = asset.line_count > limit;
        return Ok(Json(serde_json::json!({
            "metadata": metadata,
            "tail_lines": limit,
            "content": content,
            "truncated": truncated,
            "total_lines": asset.line_count,
            "window": {"start": start, "end": end},
        })));
    }

    // Default: bounded head read (64 KiB limit)
    let max_bytes = TOOL_OUTPUT_DETAIL_MAX_BYTES;
    let (content, truncated, next_offset) = if asset.byte_count <= max_bytes {
        (
            store
                .read_blob(&asset, &session_id)
                .await
                .map_err(|e| ErrorResponse::internal(format!("read: {e}")))?,
            false,
            None::<usize>,
        )
    } else {
        (
            store
                .read_blob_range(&asset, &session_id, 0, max_bytes)
                .await
                .map_err(|e| ErrorResponse::internal(format!("read range: {e}")))?,
            true,
            Some(max_bytes),
        )
    };

    Ok(Json(serde_json::json!({
        "metadata": metadata,
        "content": content,
        "truncated": truncated,
        "total_bytes": asset.byte_count,
        "total_lines": asset.line_count,
        "continuation": next_offset.map(|off| serde_json::json!({
            "next_offset": off,
            "hint": "use range_start and range_end query params for paginated reads"
        })),
    })))
}
