mod agents;
mod chat;
mod config;
mod mcp;
mod session;
mod types;

pub use types::WsQueryParams;

use types::{WsRequest, WsResponse};

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Extension, Query, State,
    },
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

use crate::state::AppState;
use fastclaw_agent::QueryEngine;
use fastclaw_security::ApiKeyAuth;

static CONN_COUNTER: AtomicU64 = AtomicU64::new(0);

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
const CLIENT_TIMEOUT: Duration = Duration::from_secs(90);
const MAX_MESSAGE_SIZE: usize = 4 * 1024 * 1024;

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Extension(auth): Extension<ApiKeyAuth>,
    Query(params): Query<WsQueryParams>,
) -> impl IntoResponse {
    let pre_authed = match &params.token {
        Some(token) => auth.validate_key(token),
        None => !auth.is_enabled(),
    };
    ws.max_message_size(MAX_MESSAGE_SIZE)
        .on_upgrade(move |socket| handle_socket(socket, state, auth, pre_authed))
}

async fn send_resp(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    resp: &WsResponse,
) -> bool {
    match serde_json::to_string(resp) {
        Ok(json) => sender.send(Message::Text(json)).await.is_ok(),
        Err(e) => {
            tracing::error!(error = %e, "failed to serialize WsResponse");
            false
        }
    }
}

async fn handle_socket(socket: WebSocket, state: AppState, auth: ApiKeyAuth, pre_authed: bool) {
    let conn_id = CONN_COUNTER.fetch_add(1, Ordering::SeqCst);
    fastclaw_observe::record_ws_connection(1);
    tracing::info!(conn_id, pre_authed, "websocket client connected");

    let (mut sender, mut receiver) = socket.split();
    let mut authenticated = pre_authed;
    let mut last_activity = Instant::now();
    let mut broadcast_rx = state.strm.ws_broadcast.subscribe();
    let mut subscriptions: HashSet<String> = HashSet::new();
    let mut owned_sessions: HashSet<String> = HashSet::new();
    let mut query_engines: HashMap<String, QueryEngine> = HashMap::new();
    let cancel = CancellationToken::new();
    let active_chat_cancels: Arc<tokio::sync::Mutex<HashMap<String, CancellationToken>>> =
        Arc::new(tokio::sync::Mutex::new(HashMap::new()));

    // Channel for background tasks (chat streaming) to send responses back to the client.
    // This decouples streaming from the main loop so heartbeat/ping continue working.
    let (bg_tx, mut bg_rx) = tokio::sync::mpsc::channel::<WsResponse>(128);

    let auth_required = auth.is_enabled();
    send_resp(
        &mut sender,
        &WsResponse {
            id: None,
            msg_type: "connected".into(),
            data: Some(json!({
                "version": env!("CARGO_PKG_VERSION"),
                "connId": conn_id,
                "protocol": "fastclaw-ws/1",
                "methods": ["ping", "chat", "agents", "auth",
                            "sessions.list", "sessions.get", "sessions.messages", "sessions.delete",
                            "sessions.new", "sessions.claim", "sessions.update_title",
                            "chat.cancel", "chat.answer", "chat.set_mode",
                            "models.list", "config.get", "config.set",
                            "mcp.status", "mcp.reload", "mcp.add", "mcp.remove",
                            "subscribe", "unsubscribe"],
                "authRequired": auth_required && !authenticated,
            })),
            error: None,
        },
    )
    .await;

    let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
    heartbeat.tick().await;

    loop {
        tokio::select! {
            biased;

            // Background task responses (streaming chat events)
            Some(resp) = bg_rx.recv() => {
                if resp.msg_type == "chat.complete" {
                    if let Some(sid) = resp
                        .data
                        .as_ref()
                        .and_then(|d| d.get("sessionId"))
                        .and_then(|v| v.as_str())
                    {
                        owned_sessions.insert(sid.to_string());
                    }
                }
                last_activity = Instant::now();
                if !send_resp(&mut sender, &resp).await {
                    break;
                }
            }

            // Heartbeat tick
            _ = heartbeat.tick() => {
                if last_activity.elapsed() > CLIENT_TIMEOUT {
                    tracing::info!(conn_id, "client timeout, closing");
                    let _ = sender.send(Message::Close(Some(axum::extract::ws::CloseFrame {
                        code: 4000,
                        reason: "timeout".into(),
                    }))).await;
                    break;
                }
                send_resp(&mut sender, &WsResponse {
                    id: None, msg_type: "heartbeat".into(),
                    data: Some(json!({"ts": chrono::Utc::now().to_rfc3339()})),
                    error: None,
                }).await;
            }

            // Broadcast events for subscriptions (wildcard "*" not allowed for security)
            Ok(event_json) = broadcast_rx.recv() => {
                if let Ok(event) = serde_json::from_str::<serde_json::Value>(&event_json) {
                    let name = event.get("event").and_then(|v| v.as_str()).unwrap_or("");
                    if subscriptions.contains(name) {
                        let _ = sender.send(Message::Text(event_json)).await;
                    }
                }
            }

            // Client messages
            msg = receiver.next() => {
                let msg = match msg {
                    Some(Ok(msg)) => msg,
                    Some(Err(e)) => {
                        tracing::warn!(conn_id, error = %e, "websocket receive error");
                        break;
                    }
                    None => break,
                };

                last_activity = Instant::now();

                let text = match msg {
                    Message::Text(t) => t.to_string(),
                    Message::Close(_) => break,
                    Message::Ping(data) => { let _ = sender.send(Message::Pong(data)).await; continue; }
                    Message::Pong(_) => continue,
                    _ => continue,
                };

                let req: WsRequest = match serde_json::from_str(&text) {
                    Ok(r) => r,
                    Err(e) => {
                        send_resp(&mut sender, &WsResponse {
                            id: None, msg_type: "error".into(), data: None,
                            error: Some(json!({"code": -32700, "message": format!("parse error: {e}")})),
                        }).await;
                        continue;
                    }
                };

                // Auth method always available
                if req.method == "auth" {
                    let token = req.params.get("token").and_then(|v| v.as_str()).unwrap_or("");
                    if auth.validate_key(token) {
                        authenticated = true;
                        send_resp(&mut sender, &WsResponse {
                            id: req.id, msg_type: "auth.ok".into(),
                            data: Some(json!({"authenticated": true})), error: None,
                        }).await;
                    } else {
                        send_resp(&mut sender, &WsResponse {
                            id: req.id, msg_type: "auth.failed".into(), data: None,
                            error: Some(json!({"code": 401, "message": "invalid token"})),
                        }).await;
                    }
                    continue;
                }

                // Auth gate (ping always allowed)
                if auth_required && !authenticated {
                    if req.method == "ping" {
                        send_resp(&mut sender, &WsResponse {
                            id: req.id, msg_type: "pong".into(),
                            data: Some(json!({"ts": chrono::Utc::now().to_rfc3339()})),
                            error: None,
                        }).await;
                        continue;
                    }
                    send_resp(&mut sender, &WsResponse {
                        id: req.id, msg_type: "error".into(), data: None,
                        error: Some(json!({"code": 401, "message": "authentication required"})),
                    }).await;
                    continue;
                }

                dispatch(
                    &mut sender,
                    &state,
                    &mut subscriptions,
                    &mut owned_sessions,
                    &mut query_engines,
                    &bg_tx,
                    &cancel,
                    active_chat_cancels.clone(),
                    req,
                )
                .await;
            }
        }
    }

    cancel.cancel();
    fastclaw_observe::record_ws_connection(-1);
    tracing::info!(conn_id, "websocket session ended");
}

#[allow(clippy::too_many_arguments)]
async fn dispatch(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    subscriptions: &mut HashSet<String>,
    owned_sessions: &mut HashSet<String>,
    query_engines: &mut HashMap<String, QueryEngine>,
    bg_tx: &tokio::sync::mpsc::Sender<WsResponse>,
    cancel: &CancellationToken,
    active_chat_cancels: Arc<tokio::sync::Mutex<HashMap<String, CancellationToken>>>,
    req: WsRequest,
) {
    let id = req.id;
    match req.method.as_str() {
        "ping" => {
            send_resp(
                sender,
                &WsResponse {
                    id,
                    msg_type: "pong".into(),
                    data: Some(json!({"ts": chrono::Utc::now().to_rfc3339()})),
                    error: None,
                },
            )
            .await;
        }
        "agents" => agents::handle_agents(sender, state, id).await,
        "chat" => {
            chat::spawn_chat(
                state,
                owned_sessions,
                bg_tx.clone(),
                cancel.clone(),
                active_chat_cancels.clone(),
                id,
                req.params,
            )
            .await
        }
        "chat.submit" => {
            chat::handle_chat_submit(
                sender,
                state,
                query_engines,
                owned_sessions,
                bg_tx.clone(),
                id,
                req.params,
            )
            .await
        }
        "chat.cancel" => {
            chat::handle_chat_cancel(sender, id, req.params, active_chat_cancels.clone()).await
        }
        "chat.answer" => chat::handle_chat_answer(sender, state, id, req.params).await,
        "chat.set_mode" => chat::handle_chat_set_mode(sender, state, id, req.params).await,
        "sessions.list" => session::handle_sessions_list(sender, state, id, req.params).await,
        "sessions.get" => {
            session::handle_session_scoped(sender, state, owned_sessions, id, req.params, "get")
                .await
        }
        "sessions.messages" => {
            session::handle_session_scoped(
                sender,
                state,
                owned_sessions,
                id,
                req.params,
                "messages",
            )
            .await
        }
        "sessions.delete" => {
            session::handle_session_scoped(sender, state, owned_sessions, id, req.params, "delete")
                .await
        }
        "sessions.new" => {
            session::handle_sessions_new(sender, state, owned_sessions, id, req.params).await
        }
        "sessions.claim" => {
            session::handle_sessions_claim(sender, state, owned_sessions, id, req.params).await
        }
        "sessions.update_title" => {
            session::handle_session_scoped(
                sender,
                state,
                owned_sessions,
                id,
                req.params,
                "update_title",
            )
            .await
        }
        "models.list" => config::handle_models_list(sender, state, id).await,
        "config.get" => config::handle_config_get(sender, state, id, req.params).await,
        "config.set" => config::handle_config_set(sender, state, id, req.params).await,
        "mcp.status" => mcp::handle_mcp_status(sender, state, id).await,
        "mcp.reload" => mcp::handle_mcp_reload(sender, state, id).await,
        "mcp.add" => mcp::handle_mcp_add(sender, state, id, req.params).await,
        "mcp.remove" => mcp::handle_mcp_remove(sender, state, id, req.params).await,
        "subscribe" => {
            let events: Vec<String> = req
                .params
                .get("events")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();
            for e in &events {
                subscriptions.insert(e.clone());
            }
            send_resp(
                sender,
                &WsResponse {
                    id,
                    msg_type: "subscribe.ok".into(),
                    data: Some(json!({"subscriptions": subscriptions.iter().collect::<Vec<_>>()})),
                    error: None,
                },
            )
            .await;
        }
        "unsubscribe" => {
            let events: Vec<String> = req
                .params
                .get("events")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();
            for e in &events {
                subscriptions.remove(e);
            }
            send_resp(
                sender,
                &WsResponse {
                    id,
                    msg_type: "unsubscribe.ok".into(),
                    data: Some(json!({"subscriptions": subscriptions.iter().collect::<Vec<_>>()})),
                    error: None,
                },
            )
            .await;
        }
        other => {
            send_resp(
                sender,
                &WsResponse {
                    id,
                    msg_type: "error".into(),
                    data: None,
                    error: Some(
                        json!({"code": -32601, "message": format!("unknown method: {other}")}),
                    ),
                },
            )
            .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::chat::event_to_response;
    use crate::state::AppState;
    use fastclaw_core::config_access::{
        filter_config_for_read, mask_secret_values, navigate_config, set_nested_key,
        CONFIG_READABLE_KEYS, CONFIG_WRITABLE_KEYS,
    };
    use fastclaw_core::types::StreamEvent;
    use serde_json::json;

    #[test]
    fn set_nested_key_simple() {
        let mut root = json!({"a": {"b": 1}});
        set_nested_key(&mut root, "a.b", json!(2)).unwrap();
        assert_eq!(root["a"]["b"], 2);
    }

    #[test]
    fn set_nested_key_creates_intermediate() {
        let mut root = json!({});
        set_nested_key(&mut root, "x.y", json!("hello")).unwrap();
        assert_eq!(root["x"]["y"], "hello");
    }

    #[test]
    fn navigate_config_returns_nested_value() {
        let cfg = json!({"gateway": {"port": 18789}});
        let val = navigate_config(&cfg, "gateway.port");
        assert_eq!(val, 18789);
    }

    #[test]
    fn navigate_config_missing_key_returns_null() {
        let cfg = json!({"gateway": {}});
        let val = navigate_config(&cfg, "gateway.missing");
        assert!(val.is_null());
    }

    #[test]
    fn filter_config_only_includes_readable_keys() {
        let cfg = json!({
            "gateway": {"port": 18789},
            "logging": {"level": "info"},
            "dangerousInternal": "should not appear"
        });
        let filtered = filter_config_for_read(&cfg);
        assert!(filtered.get("gateway").is_some());
        assert!(filtered.get("logging").is_some());
        assert!(filtered.get("dangerousInternal").is_none());
    }

    #[test]
    fn mask_secret_values_masks_api_keys() {
        let val = json!({
            "openai": {
                "apiKey": "sk-1234567890abcdef",
                "baseUrl": "https://api.openai.com/v1"
            }
        });
        let masked = mask_secret_values(&val);
        let key = masked["openai"]["apiKey"].as_str().unwrap();
        assert!(key.contains("…"));
        assert!(!key.contains("1234567890"));
        assert_eq!(masked["openai"]["baseUrl"], "https://api.openai.com/v1");
    }

    #[test]
    fn mask_secret_values_short_key() {
        let val = json!({"apiKey": "short"});
        let masked = mask_secret_values(&val);
        assert_eq!(masked["apiKey"], "****");
    }

    #[test]
    fn mask_secret_values_empty_key_unchanged() {
        let val = json!({"apiKey": ""});
        let masked = mask_secret_values(&val);
        assert_eq!(masked["apiKey"], "");
    }

    #[test]
    fn config_writable_keys_include_web_search() {
        assert!(CONFIG_WRITABLE_KEYS.contains(&"webSearch"));
        assert!(CONFIG_WRITABLE_KEYS.contains(&"credentials"));
        assert!(CONFIG_WRITABLE_KEYS.contains(&"models"));
    }

    #[test]
    fn config_readable_keys_include_web_search_and_credentials() {
        assert!(CONFIG_READABLE_KEYS.contains(&"webSearch"));
        assert!(CONFIG_READABLE_KEYS.contains(&"credentials"));
        assert!(CONFIG_READABLE_KEYS.contains(&"modelRouter"));
    }

    #[test]
    fn event_to_response_ask_question_format() {
        use fastclaw_core::types::AskQuestionOption;
        let event = StreamEvent::AskQuestion {
            request_id: "q1".into(),
            question: "Pick one".into(),
            options: vec![
                AskQuestionOption {
                    id: "a".into(),
                    label: "Option A".into(),
                },
                AskQuestionOption {
                    id: "b".into(),
                    label: "Option B".into(),
                },
            ],
            timeout_secs: 30,
            allow_multiple: false,
        };
        let state = {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let tmp = tempfile::tempdir().unwrap();
            rt.block_on(async {
                AppState::for_test(Box::new(NullProvider), tmp.path())
                    .await
                    .unwrap()
            })
        };
        let resp = event_to_response(&event, &Some("r1".into()), &state, None);
        assert_eq!(resp.msg_type, "chat.ask_question");
        let data = resp.data.unwrap();
        assert_eq!(data["requestId"], "q1");
        assert_eq!(data["question"], "Pick one");
        assert_eq!(data["options"].as_array().unwrap().len(), 2);
        assert_eq!(data["timeoutSecs"], 30);
    }

    #[test]
    fn event_to_response_done_includes_session_id() {
        let event = StreamEvent::Done {
            session_id: Some("sess-123".into()),
            tool_calls_made: 2,
            iterations: 1,
            final_tool_calls: None,
            usage: None,
            elapsed_ms: 0,
            context_tokens: None,
            context_window: None,
        };
        let state = {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let tmp = tempfile::tempdir().unwrap();
            rt.block_on(async {
                AppState::for_test(Box::new(NullProvider), tmp.path())
                    .await
                    .unwrap()
            })
        };
        let resp = event_to_response(&event, &Some("r2".into()), &state, None);
        assert_eq!(resp.msg_type, "chat.complete");
        let data = resp.data.unwrap();
        assert_eq!(data["sessionId"], "sess-123");
        assert_eq!(data["toolCallsMade"], 2);
        assert_eq!(data["iterations"], 1);
    }

    struct NullProvider;
    #[async_trait::async_trait]
    impl fastclaw_agent::LlmProvider for NullProvider {
        async fn chat_completion(
            &self,
            _params: &fastclaw_agent::CompletionParams<'_>,
        ) -> anyhow::Result<fastclaw_core::types::ChatResponse> {
            Ok(fastclaw_core::types::ChatResponse {
                id: "null".into(),
                object: "chat.completion".into(),
                created: 0,
                model: "null".into(),
                choices: vec![],
                usage: None,
            })
        }
        async fn chat_completion_stream(
            &self,
            _params: &fastclaw_agent::CompletionParams<'_>,
        ) -> anyhow::Result<
            futures::stream::BoxStream<'static, anyhow::Result<fastclaw_core::types::StreamDelta>>,
        > {
            Ok(Box::pin(futures::stream::empty()))
        }
    }
}
