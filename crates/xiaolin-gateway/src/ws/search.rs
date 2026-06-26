use std::time::Duration;

use axum::extract::ws::{Message, WebSocket};
use serde_json::json;
use xiaolin_protocol::{SearchIndexStatusResponse, SearchQueryRequest, SearchQueryResponse};

use crate::state::AppState;

use super::send_resp;
use super::types::WsResponse;

const SEARCH_TIMEOUT: Duration = Duration::from_secs(2);
const DEFAULT_LIMIT: i64 = 10;
const MAX_LIMIT: i64 = 10;

pub async fn handle_search_query(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
    params: SearchQueryRequest,
) {
    let page = params.page.unwrap_or(0).max(0);
    let limit = params.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let offset = page * limit;

    let query = params.q.trim();
    if query.is_empty() {
        let response = SearchQueryResponse {
            results: Vec::new(),
            total_estimate: 0,
            page,
        };
        send_resp(
            sender,
            &WsResponse {
                id: req_id,
                msg_type: "search.query".into(),
                data: Some(serde_json::to_value(response).unwrap_or_default()),
                error: None,
            },
        )
        .await;
        return;
    }

    let search_index = state.store.search_index.clone();
    let filters = params.filters;
    let q = query.to_string();

    let search_result = tokio::time::timeout(SEARCH_TIMEOUT, async move {
        search_index.search(&q, &filters, limit, offset).await
    })
    .await;

    match search_result {
        Ok(Ok(results)) => {
            let total_estimate = (offset + results.len() as i64).max(0) as u64;
            let response = SearchQueryResponse {
                results,
                total_estimate,
                page,
            };
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "search.query".into(),
                    data: Some(serde_json::to_value(response).unwrap_or_default()),
                    error: None,
                },
            )
            .await;
        }
        Ok(Err(e)) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "search.query".into(),
                    data: None,
                    error: Some(json!({"code": "search_error", "message": e.to_string()})),
                },
            )
            .await;
        }
        Err(_) => {
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "search.query".into(),
                    data: None,
                    error: Some(json!({"code": "search_timeout", "message": "search timed out"})),
                },
            )
            .await;
        }
    }
}

pub async fn handle_search_index_status(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    req_id: Option<String>,
) {
    match state.store.search_index.index_status().await {
        Ok(status) => {
            let response = SearchIndexStatusResponse {
                indexed_count: status.indexed_count,
                total_count: status.total_count,
                is_indexing: status.is_indexing,
            };
            send_resp(
                sender,
                &WsResponse {
                    id: req_id,
                    msg_type: "search.index_status".into(),
                    data: Some(serde_json::to_value(response).unwrap_or_default()),
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
                    msg_type: "search.index_status".into(),
                    data: None,
                    error: Some(json!({"code": "search_status_error", "message": e.to_string()})),
                },
            )
            .await;
        }
    }
}
