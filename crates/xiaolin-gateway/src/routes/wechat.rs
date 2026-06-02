use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::response::sse::{Event, Sse};
use axum::response::IntoResponse;
use axum::Json;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::json;

use xiaolin_core::config_access::{persist_config_key, set_nested_key};
use xiaolin_wechat::auth::credential;
use xiaolin_wechat::auth::qr_login;

use crate::state::AppState;

static LOGIN_SESSIONS: std::sync::LazyLock<DashMap<String, qr_login::QrLoginSession>> =
    std::sync::LazyLock::new(DashMap::new);

#[derive(Serialize)]
struct LoginStartResponse {
    session_key: String,
    qr_url: String,
    message: String,
}

#[derive(Deserialize)]
pub struct VerifyCodeRequest {
    pub code: String,
}

#[derive(Serialize)]
struct AccountInfo {
    account_id: String,
    base_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    created_at: Option<String>,
}

/// Write the "wechat" channel entry into config_live + persist to disk, then hot-reload.
async fn activate_wechat_channel(state: &AppState, base_url: &str) {
    let config_val = json!({
        "enabled": true,
        "connectionMode": "longpoll",
        "domain": base_url,
    });

    {
        let mut live: serde_json::Value = (**state.cfg.config_live.load()).clone();
        if set_nested_key(&mut live, "channels.wechat", config_val.clone()).is_ok() {
            state.cfg.config_live.store(Arc::new(live));
        }
    }
    if let Err(e) = persist_config_key("channels.wechat", &config_val) {
        tracing::warn!(error = %e, "failed to persist wechat channel config");
    }

    // Also ensure a binding exists
    {
        let live_snapshot = state.cfg.config_live.load();
        let bindings = live_snapshot.get("bindings").cloned().unwrap_or(json!([]));
        let already = bindings.as_array().is_some_and(|arr| {
            arr.iter().any(|b| {
                b.get("match")
                    .and_then(|m| m.get("channel"))
                    .and_then(|c| c.as_str())
                    == Some("wechat")
            })
        });
        if !already {
            let mut live: serde_json::Value = (**state.cfg.config_live.load()).clone();
            let binding = json!({
                "agentId": "main",
                "match": { "channel": "wechat" }
            });
            let mut new_bindings = bindings.as_array().cloned().unwrap_or_default();
            new_bindings.push(binding);
            let bindings_val = serde_json::Value::Array(new_bindings);
            if set_nested_key(&mut live, "bindings", bindings_val.clone()).is_ok() {
                state.cfg.config_live.store(Arc::new(live));
                let _ = persist_config_key("bindings", &bindings_val);
                tracing::info!("auto-created binding for wechat channel");
            }
        }
    }

    if let Err(e) = state.reload_channel("wechat").await {
        tracing::warn!(error = %e, "failed to hot-reload wechat channel after login");
    }
}

pub async fn login_start(State(_state): State<AppState>) -> impl IntoResponse {
    let existing_tokens: Vec<String> = credential::list_credentials()
        .into_iter()
        .map(|(_, c)| c.token)
        .collect();

    match qr_login::start_login(&existing_tokens).await {
        Ok(session) => {
            let resp = LoginStartResponse {
                session_key: session.session_key.clone(),
                qr_url: session.qr_url.clone(),
                message: "请扫描二维码登录微信".into(),
            };
            LOGIN_SESSIONS.insert(session.session_key.clone(), session);
            Json(serde_json::json!(resp)).into_response()
        }
        Err(e) => {
            let body = serde_json::json!({"error": format!("{e}")});
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response()
        }
    }
}

pub async fn login_status(
    State(state): State<AppState>,
    Path(session_key): Path<String>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>>> {
    let stream = async_stream::stream! {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(480);

        loop {
            if tokio::time::Instant::now() >= deadline {
                let data = serde_json::json!({"status": "timeout"});
                yield Ok(Event::default().data(data.to_string()));
                break;
            }

            let status = {
                let mut session = match LOGIN_SESSIONS.get_mut(&session_key) {
                    Some(s) => s,
                    None => {
                        let data = serde_json::json!({"status": "error", "message": "session not found"});
                        yield Ok(Event::default().data(data.to_string()));
                        break;
                    }
                };
                qr_login::poll_login(&mut session).await
            };

            match status {
                qr_login::LoginStatus::Waiting => {
                    let data = serde_json::json!({"status": "waiting"});
                    yield Ok(Event::default().data(data.to_string()));
                }
                qr_login::LoginStatus::Scanned => {
                    let data = serde_json::json!({"status": "scanned"});
                    yield Ok(Event::default().data(data.to_string()));
                }
                qr_login::LoginStatus::NeedVerifyCode => {
                    let data = serde_json::json!({"status": "need_verifycode"});
                    yield Ok(Event::default().data(data.to_string()));
                }
                qr_login::LoginStatus::Confirmed { bot_token, account_id, base_url, user_id } => {
                    let normalized = credential::normalize_account_id(&account_id);
                    let cred = credential::WechatCredential {
                        token: bot_token,
                        base_url: base_url.clone(),
                        user_id: user_id.clone(),
                        cdn_base_url: None,
                        created_at: Some(chrono::Utc::now().to_rfc3339()),
                    };
                    credential::save_credential(&normalized, &cred).ok();

                    activate_wechat_channel(&state, &base_url).await;

                    let data = serde_json::json!({
                        "status": "confirmed",
                        "account_id": normalized
                    });
                    yield Ok(Event::default().data(data.to_string()));
                    LOGIN_SESSIONS.remove(&session_key);
                    break;
                }
                qr_login::LoginStatus::AlreadyConnected => {
                    let data = serde_json::json!({"status": "already_connected"});
                    yield Ok(Event::default().data(data.to_string()));
                    LOGIN_SESSIONS.remove(&session_key);
                    break;
                }
                qr_login::LoginStatus::Expired => {
                    let refreshed = {
                        let mut session = LOGIN_SESSIONS.get_mut(&session_key).unwrap();
                        qr_login::refresh_qr(&mut session, &[]).await
                    };
                    match refreshed {
                        Ok(()) => {
                            let qr_url = LOGIN_SESSIONS.get(&session_key).map(|s| s.qr_url.clone()).unwrap_or_default();
                            let data = serde_json::json!({"status": "expired", "qr_url": qr_url});
                            yield Ok(Event::default().data(data.to_string()));
                        }
                        Err(e) => {
                            let data = serde_json::json!({"status": "error", "message": format!("{e}")});
                            yield Ok(Event::default().data(data.to_string()));
                            LOGIN_SESSIONS.remove(&session_key);
                            break;
                        }
                    }
                }
                qr_login::LoginStatus::VerifyCodeBlocked => {
                    let data = serde_json::json!({"status": "verify_code_blocked"});
                    yield Ok(Event::default().data(data.to_string()));
                }
                qr_login::LoginStatus::Error(msg) => {
                    let data = serde_json::json!({"status": "error", "message": msg});
                    yield Ok(Event::default().data(data.to_string()));
                    LOGIN_SESSIONS.remove(&session_key);
                    break;
                }
            }

            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    };

    Sse::new(stream)
}

pub async fn login_verify(
    Path(session_key): Path<String>,
    Json(body): Json<VerifyCodeRequest>,
) -> impl IntoResponse {
    if let Some(mut session) = LOGIN_SESSIONS.get_mut(&session_key) {
        qr_login::set_verify_code(&mut session, &body.code);
        Json(serde_json::json!({"ok": true}))
    } else {
        Json(serde_json::json!({"error": "session not found"}))
    }
}

pub async fn list_accounts() -> impl IntoResponse {
    let creds = credential::list_credentials();
    let accounts: Vec<AccountInfo> = creds
        .into_iter()
        .map(|(id, c)| AccountInfo {
            account_id: id,
            base_url: c.base_url,
            user_id: c.user_id,
            created_at: c.created_at,
        })
        .collect();
    Json(accounts)
}

pub async fn delete_account(Path(account_id): Path<String>) -> impl IntoResponse {
    let deleted = credential::delete_credential(&account_id);
    Json(serde_json::json!({"deleted": deleted}))
}

/// Manually trigger a hot-reload of the wechat channel.
/// Re-reads config from disk so changes made by CLI are picked up.
pub async fn reload_channel(State(state): State<AppState>) -> impl IntoResponse {
    // Refresh config_live from disk so that CLI-written changes are visible
    let mode = crate::get_config_mode();
    match xiaolin_core::config::load_config(mode) {
        Ok(fresh) => {
            let fresh_val = serde_json::to_value(&fresh).unwrap_or_default();
            state.cfg.config_live.store(Arc::new(fresh_val));
        }
        Err(e) => {
            return Json(json!({"ok": false, "error": format!("failed to reload config: {e}")}));
        }
    }

    match state.reload_channel("wechat").await {
        Ok(()) => Json(json!({"ok": true, "message": "wechat channel reloaded"})),
        Err(e) => Json(json!({"ok": false, "error": format!("{e}")})),
    }
}
