use std::time::{Duration, Instant};

use crate::api::client::WechatApiClient;
use crate::auth::credential;

const QR_LOGIN_BASE_URL: &str = "https://ilinkai.weixin.qq.com";
const MAX_QR_REFRESH: u32 = 3;

#[derive(Debug, Clone)]
pub struct QrLoginSession {
    pub session_key: String,
    pub qrcode: String,
    pub qr_url: String,
    pub started_at: Instant,
    current_base_url: String,
    pending_verify_code: Option<String>,
}

#[derive(Debug)]
pub enum LoginStatus {
    Waiting,
    Scanned,
    NeedVerifyCode,
    Confirmed {
        bot_token: String,
        account_id: String,
        base_url: String,
        user_id: Option<String>,
    },
    AlreadyConnected,
    Expired,
    VerifyCodeBlocked,
    Error(String),
}

pub struct LoginResult {
    pub status: LoginStatus,
    pub message: String,
}

pub async fn start_login(existing_tokens: &[String]) -> anyhow::Result<QrLoginSession> {
    let qr_resp = WechatApiClient::fetch_qr_code(existing_tokens).await?;

    Ok(QrLoginSession {
        session_key: uuid::Uuid::new_v4().to_string(),
        qrcode: qr_resp.qrcode,
        qr_url: qr_resp.qrcode_img_content,
        started_at: Instant::now(),
        current_base_url: QR_LOGIN_BASE_URL.to_string(),
        pending_verify_code: None,
    })
}

pub async fn poll_login(session: &mut QrLoginSession) -> LoginStatus {
    match WechatApiClient::poll_qr_status(
        &session.current_base_url,
        &session.qrcode,
        session.pending_verify_code.as_deref(),
    )
    .await
    {
        Ok(resp) => {
            match resp.status.as_str() {
                "wait" => LoginStatus::Waiting,
                "scaned" => {
                    session.pending_verify_code = None;
                    LoginStatus::Scanned
                }
                "confirmed" => {
                    if let (Some(token), Some(bot_id)) = (resp.bot_token, resp.ilink_bot_id) {
                        LoginStatus::Confirmed {
                            bot_token: token,
                            account_id: bot_id,
                            base_url: resp.baseurl.unwrap_or_else(|| session.current_base_url.clone()),
                            user_id: resp.ilink_user_id,
                        }
                    } else {
                        LoginStatus::Error("confirmed but missing bot_token or bot_id".into())
                    }
                }
                "expired" => LoginStatus::Expired,
                "need_verifycode" => LoginStatus::NeedVerifyCode,
                "verify_code_blocked" => {
                    session.pending_verify_code = None;
                    LoginStatus::VerifyCodeBlocked
                }
                "binded_redirect" => LoginStatus::AlreadyConnected,
                "scaned_but_redirect" => {
                    if let Some(host) = resp.redirect_host {
                        session.current_base_url = format!("https://{host}");
                        tracing::info!(new_host = %host, "IDC redirect during QR login");
                    }
                    LoginStatus::Scanned
                }
                other => LoginStatus::Error(format!("unknown QR status: {other}")),
            }
        }
        Err(e) => LoginStatus::Error(format!("poll error: {e}")),
    }
}

pub fn set_verify_code(session: &mut QrLoginSession, code: &str) {
    session.pending_verify_code = Some(code.to_string());
}

pub async fn refresh_qr(
    session: &mut QrLoginSession,
    existing_tokens: &[String],
) -> anyhow::Result<()> {
    let qr_resp = WechatApiClient::fetch_qr_code(existing_tokens).await?;
    session.qrcode = qr_resp.qrcode;
    session.qr_url = qr_resp.qrcode_img_content;
    session.started_at = Instant::now();
    session.current_base_url = QR_LOGIN_BASE_URL.to_string();
    session.pending_verify_code = None;
    Ok(())
}

/// Full blocking login flow for CLI usage.
/// `on_qr_refresh` is called after QR code is refreshed (expired/blocked) with the new URL.
pub async fn wait_for_login<F>(
    session: &mut QrLoginSession,
    timeout: Duration,
    on_qr_refresh: F,
) -> LoginResult
where
    F: Fn(&str),
{
    let deadline = Instant::now() + timeout;
    let mut qr_refresh_count = 0u32;

    while Instant::now() < deadline {
        let status = poll_login(session).await;
        match status {
            LoginStatus::Waiting => {}
            LoginStatus::Scanned => {
                tracing::info!("QR code scanned, waiting for confirmation...");
            }
            LoginStatus::Confirmed {
                bot_token,
                account_id,
                base_url,
                user_id,
            } => {
                let normalized_id = credential::normalize_account_id(&account_id);
                let cred = credential::WechatCredential {
                    token: bot_token,
                    base_url,
                    user_id,
                    cdn_base_url: None,
                    created_at: Some(chrono::Utc::now().to_rfc3339()),
                };
                if let Err(e) = credential::save_credential(&normalized_id, &cred) {
                    tracing::error!(error = %e, "failed to save credential");
                }
                return LoginResult {
                    status: LoginStatus::Confirmed {
                        bot_token: cred.token,
                        account_id: normalized_id,
                        base_url: cred.base_url,
                        user_id: cred.user_id,
                    },
                    message: "已将此 XiaoLin 连接到微信。".into(),
                };
            }
            LoginStatus::AlreadyConnected => {
                return LoginResult {
                    status: LoginStatus::AlreadyConnected,
                    message: "已连接过此 XiaoLin，无需重复连接。".into(),
                };
            }
            LoginStatus::Expired | LoginStatus::VerifyCodeBlocked => {
                qr_refresh_count += 1;
                if qr_refresh_count > MAX_QR_REFRESH {
                    return LoginResult {
                        status: LoginStatus::Error("QR expired too many times".into()),
                        message: "二维码多次失效，连接流程已停止。".into(),
                    };
                }
                tracing::info!(refresh = qr_refresh_count, "refreshing QR code");
                if let Err(e) = refresh_qr(session, &[]).await {
                    return LoginResult {
                        status: LoginStatus::Error(format!("refresh failed: {e}")),
                        message: format!("刷新二维码失败: {e}"),
                    };
                }
                on_qr_refresh(&session.qr_url);
            }
            LoginStatus::NeedVerifyCode => {
                return LoginResult {
                    status: LoginStatus::NeedVerifyCode,
                    message: "请输入手机微信显示的配对数字".into(),
                };
            }
            LoginStatus::Error(e) => {
                return LoginResult {
                    status: LoginStatus::Error(e.clone()),
                    message: format!("登录失败: {e}"),
                };
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    LoginResult {
        status: LoginStatus::Error("timeout".into()),
        message: "登录超时，请重试。".into(),
    }
}

/// Display QR code in terminal.
/// Uses Unicode block characters with explicit ANSI 256-color codes (232=black, 255=white)
/// for maximum compatibility across terminal themes.
pub fn display_qr_terminal(url: &str) {
    use qrcode::QrCode;
    match QrCode::new(url) {
        Ok(code) => {
            let matrix = code.render::<char>()
                .quiet_zone(true)
                .module_dimensions(2, 1)
                .dark_color('█')
                .light_color(' ')
                .build();
            // Render each line with explicit white background to avoid terminal theme issues.
            // Use ANSI 256-color: \x1b[38;5;232m = fg black (color 232), \x1b[48;5;255m = bg white (color 255)
            for line in matrix.lines() {
                // Dark modules → black text on white background ("█" looks dark on white)
                // Light modules → space on white background
                println!("\x1b[38;5;232m\x1b[48;5;255m{line}\x1b[0m");
            }
        }
        Err(_) => {
            qr2term::print_qr(url).ok();
        }
    }
    println!("若二维码未能显示，请访问：{url}");
}
