use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::state::AppState;
use xiaolin_pty::PtySessionConfig;

#[derive(Deserialize)]
pub struct PtyQueryParams {
    pub cols: Option<u16>,
    pub rows: Option<u16>,
    pub shell: Option<String>,
    pub cwd: Option<String>,
}

#[derive(Serialize)]
struct PtyControlResponse {
    #[serde(rename = "type")]
    msg_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    exit_code: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cwd: Option<String>,
}

fn pty_response_text(resp: &PtyControlResponse) -> Option<String> {
    match serde_json::to_string(resp) {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::warn!(error = %e, "failed to serialize PTY control response");
            None
        }
    }
}

pub async fn pty_ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Query(params): Query<PtyQueryParams>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_pty_socket(socket, state, params))
}

async fn handle_pty_socket(socket: WebSocket, state: AppState, params: PtyQueryParams) {
    let cwd = params.cwd.or_else(|| std::env::var("HOME").ok());
    let initial_cwd = cwd.clone();

    let config = PtySessionConfig {
        shell: params.shell,
        cwd,
        cols: params.cols.unwrap_or(80),
        rows: params.rows.unwrap_or(24),
        env: Vec::new(),
        source: "user".to_string(),
    };

    let (session_id, mut broadcast_rx) =
        match state.strm.pty_manager.create_session_with_subscriber(config) {
            Ok(pair) => pair,
            Err(e) => {
                let (mut sender, _) = socket.split();
                let resp = PtyControlResponse {
                    msg_type: "error".into(),
                    session_id: None,
                    error: Some(e),
                    exit_code: None,
                    cwd: None,
                };
                if let Some(text) = pty_response_text(&resp) {
                    let _ = sender.send(Message::Text(text)).await;
                }
                return;
            }
        };

    let (mut ws_sender, mut ws_receiver) = socket.split();

    let resp = PtyControlResponse {
        msg_type: "session_created".into(),
        session_id: Some(session_id.clone()),
        error: None,
        exit_code: None,
        cwd: initial_cwd,
    };
    if let Some(text) = pty_response_text(&resp) {
        if ws_sender.send(Message::Text(text)).await.is_err() {
            state.strm.pty_manager.close_session(&session_id);
            return;
        }
    } else {
        state.strm.pty_manager.close_session(&session_id);
        return;
    }

    // Poll /proc/PID/cwd for working directory changes (Linux)
    let (cwd_tx, mut cwd_rx) = mpsc::channel::<String>(4);
    let pty_mgr_cwd = state.strm.pty_manager.clone();
    let sid_cwd = session_id.clone();
    tokio::spawn(async move {
        let pid = pty_mgr_cwd
            .get_session(&sid_cwd, |s| s.process_id())
            .flatten();
        let Some(pid) = pid else { return };
        let proc_path = format!("/proc/{pid}/cwd");
        let mut last_cwd = String::new();
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            match tokio::fs::read_link(&proc_path).await {
                Ok(path) => {
                    let cwd = path.to_string_lossy().to_string();
                    if cwd != last_cwd {
                        last_cwd.clone_from(&cwd);
                        if cwd_tx.send(cwd).await.is_err() {
                            break;
                        }
                    }
                }
                Err(_) => break,
            }
        }
    });

    let pty_mgr_write = state.strm.pty_manager.clone();
    let sid_write = session_id.clone();
    let pty_mgr_close = state.strm.pty_manager.clone();
    let sid_close = session_id.clone();

    loop {
        tokio::select! {
            biased;

            result = broadcast_rx.recv() => {
                match result {
                    Ok(data) => {
                        state.strm.pty_manager
                            .get_session(&sid_write, |s| s.touch())
                            .unwrap_or(());
                        if ws_sender.send(Message::Binary(data)).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(session_id = %sid_write, lagged = n, "broadcast consumer lagged");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }

            Some(cwd) = cwd_rx.recv() => {
                let resp = PtyControlResponse {
                    msg_type: "cwd_changed".into(),
                    session_id: Some(sid_write.clone()),
                    error: None,
                    exit_code: None,
                    cwd: Some(cwd),
                };
                let Some(text) = pty_response_text(&resp) else {
                    break;
                };
                if ws_sender.send(Message::Text(text)).await.is_err() {
                    break;
                }
            }

            msg = ws_receiver.next() => {
                match msg {
                    Some(Ok(Message::Binary(data))) => {
                        let write_result = pty_mgr_write.get_session(&sid_write, |s| {
                            s.write_input(&data)
                        });
                        if write_result.is_none() || write_result.unwrap().is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(ctrl) = serde_json::from_str::<serde_json::Value>(&text.to_string()) {
                            match ctrl.get("type").and_then(|t| t.as_str()) {
                                Some("resize") => {
                                    let cols = ctrl.get("cols").and_then(|v| v.as_u64()).unwrap_or(80) as u16;
                                    let rows = ctrl.get("rows").and_then(|v| v.as_u64()).unwrap_or(24) as u16;
                                    let _ = pty_mgr_write.with_session_mut(&sid_write, |s| {
                                        s.resize(cols, rows)
                                    });
                                }
                                Some("ping") => {
                                    let resp = PtyControlResponse {
                                        msg_type: "pong".into(),
                                        session_id: Some(sid_write.clone()),
                                        error: None,
                                        exit_code: None,
                                        cwd: None,
                                    };
                                    if let Some(text) = pty_response_text(&resp) {
                                        let _ = ws_sender.send(Message::Text(text)).await;
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Ping(data))) => {
                        let _ = ws_sender.send(Message::Pong(data)).await;
                    }
                    _ => {}
                }
            }
        }
    }

    let exit_code = pty_mgr_close.get_session(&sid_close, |s| s.exit_code()).flatten();
    let resp = PtyControlResponse {
        msg_type: "session_closed".into(),
        session_id: Some(sid_close.clone()),
        error: None,
        exit_code,
        cwd: None,
    };
    if let Some(text) = pty_response_text(&resp) {
        let _ = ws_sender.send(Message::Text(text)).await;
    }
    pty_mgr_close.close_session(&sid_close);

    tracing::info!(session_id = %sid_close, "PTY WebSocket session ended");
}

pub async fn pty_list_handler(State(state): State<AppState>) -> impl IntoResponse {
    let sessions = state.strm.pty_manager.list_sessions();
    let list: Vec<serde_json::Value> = sessions
        .iter()
        .map(|s| {
            serde_json::json!({
                "id": s.id,
                "source": s.source,
                "alive": s.alive,
                "cols": s.cols,
                "rows": s.rows,
                "idle_secs": s.idle_secs,
            })
        })
        .collect();
    axum::Json(serde_json::json!({ "sessions": list }))
}
