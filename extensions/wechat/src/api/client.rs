use std::sync::LazyLock;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use rand::Rng;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use tokio_util::sync::CancellationToken;

use super::types::*;

const DEFAULT_API_TIMEOUT: Duration = Duration::from_secs(15);
const DEFAULT_LONG_POLL_TIMEOUT: Duration = Duration::from_secs(35);
const DEFAULT_CONFIG_TIMEOUT: Duration = Duration::from_secs(10);

const QR_LOGIN_BASE_URL: &str = "https://ilinkai.weixin.qq.com";
const DEFAULT_BOT_TYPE: &str = "3";

const CHANNEL_VERSION: &str = env!("CARGO_PKG_VERSION");
const DEFAULT_BOT_AGENT: &str = "XiaoLin";

static PRODUCT_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"^[A-Za-z0-9_.\-]{1,32}/[A-Za-z0-9_.+\-]{1,32}$")
        .expect("invalid product regex")
});

#[derive(Clone)]
pub struct WechatApiClient {
    http: reqwest::Client,
    long_poll_client: reqwest::Client,
    base_url: String,
    token: String,
    bot_agent: String,
}

impl WechatApiClient {
    pub fn new(
        base_url: &str,
        token: &str,
        bot_agent: Option<&str>,
        long_poll_timeout: Duration,
    ) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(DEFAULT_API_TIMEOUT)
            .build()
            .map_err(|e| anyhow::anyhow!("failed to build reqwest client: {e}"))?;
        let long_poll_client = reqwest::Client::builder()
            .timeout(long_poll_timeout + Duration::from_secs(5))
            .build()
            .map_err(|e| anyhow::anyhow!("failed to build long-poll reqwest client: {e}"))?;

        Ok(Self {
            http,
            long_poll_client,
            base_url: base_url.trim_end_matches('/').to_string(),
            token: token.to_string(),
            bot_agent: sanitize_bot_agent(bot_agent.unwrap_or(DEFAULT_BOT_AGENT)),
        })
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn set_base_url(&mut self, url: &str) {
        self.base_url = url.trim_end_matches('/').to_string();
    }

    fn build_base_info(&self) -> BaseInfo {
        BaseInfo {
            channel_version: Some(CHANNEL_VERSION.to_string()),
            bot_agent: Some(self.bot_agent.clone()),
        }
    }

    fn build_headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(
            "AuthorizationType",
            HeaderValue::from_static("ilink_bot_token"),
        );
        if !self.token.is_empty() {
            if let Ok(v) = HeaderValue::from_str(&format!("Bearer {}", self.token)) {
                headers.insert("Authorization", v);
            }
        }
        let uin = random_wechat_uin();
        if let Ok(v) = HeaderValue::from_str(&uin) {
            headers.insert("X-WECHAT-UIN", v);
        }
        headers
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{}", self.base_url, path)
    }

    // ── Message APIs ──────────────────────────────────────────────────────

    pub async fn get_updates(
        &self,
        cursor: &str,
        _timeout: Duration,
        cancel: &CancellationToken,
    ) -> anyhow::Result<GetUpdatesResp> {
        let body = serde_json::to_string(&GetUpdatesReq {
            get_updates_buf: cursor.to_string(),
            base_info: self.build_base_info(),
        })?;

        let req = self
            .long_poll_client
            .post(self.url("ilink/bot/getupdates"))
            .headers(self.build_headers())
            .body(body);

        tokio::select! {
            result = req.send() => {
                let http_resp: reqwest::Response = result?;
                let text = http_resp.text().await?;
                Ok(serde_json::from_str(&text)?)
            }
            () = cancel.cancelled() => {
                Ok(GetUpdatesResp {
                    ret: Some(0),
                    msgs: Some(vec![]),
                    get_updates_buf: Some(cursor.to_string()),
                    ..Default::default()
                })
            }
        }
    }

    pub async fn send_message(&self, msg: WeixinMessage) -> anyhow::Result<()> {
        let to = msg.to_user_id.clone().unwrap_or_default();
        let has_ctx = msg.context_token.is_some();
        let message_type = msg.message_type;

        let req = SendMessageReq {
            msg,
            base_info: self.build_base_info(),
        };
        let body = serde_json::to_string(&req)?;

        tracing::debug!(
            to = %to,
            has_context_token = has_ctx,
            message_type = ?message_type,
            body_len = body.len(),
            "sendMessage request"
        );

        let resp = self
            .http
            .post(self.url("ilink/bot/sendmessage"))
            .headers(self.build_headers())
            .body(body)
            .send()
            .await?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();

        if !status.is_success() {
            tracing::error!(status = %status, body = %text, "sendMessage HTTP error");
            anyhow::bail!("sendMessage failed: {status} {text}");
        }

        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&text) {
            let ret = parsed.get("ret").and_then(|v| v.as_i64()).unwrap_or(0);
            if ret != 0 {
                tracing::error!(ret, body = %text, "sendMessage API error");
                anyhow::bail!("sendMessage API error: ret={ret} body={text}");
            }
        }

        tracing::debug!(to = %to, "sendMessage success");
        Ok(())
    }

    pub async fn get_upload_url(&self, req: GetUploadUrlReq) -> anyhow::Result<GetUploadUrlResp> {
        let body = serde_json::to_string(&req)?;
        let resp = self
            .http
            .post(self.url("ilink/bot/getuploadurl"))
            .headers(self.build_headers())
            .body(body)
            .send()
            .await?
            .text()
            .await?;
        tracing::debug!(response = %resp, "get_upload_url raw response");
        Ok(serde_json::from_str(&resp)?)
    }

    pub async fn get_config(
        &self,
        user_id: &str,
        context_token: Option<&str>,
    ) -> anyhow::Result<GetConfigResp> {
        let body = serde_json::to_string(&GetConfigReq {
            ilink_user_id: user_id.to_string(),
            context_token: context_token.map(String::from),
            base_info: self.build_base_info(),
        })?;

        let client = reqwest::Client::builder()
            .timeout(DEFAULT_CONFIG_TIMEOUT)
            .build()?;

        let resp = client
            .post(self.url("ilink/bot/getconfig"))
            .headers(self.build_headers())
            .body(body)
            .send()
            .await?
            .text()
            .await?;
        Ok(serde_json::from_str(&resp)?)
    }

    pub async fn send_typing(&self, req: SendTypingReq) -> anyhow::Result<()> {
        let body = serde_json::to_string(&req)?;
        let client = reqwest::Client::builder()
            .timeout(DEFAULT_CONFIG_TIMEOUT)
            .build()?;
        client
            .post(self.url("ilink/bot/sendtyping"))
            .headers(self.build_headers())
            .body(body)
            .send()
            .await?;
        Ok(())
    }

    pub async fn notify_start(&self) -> anyhow::Result<NotifyResp> {
        let body = serde_json::to_string(&NotifyReq {
            base_info: self.build_base_info(),
        })?;
        let resp = self
            .http
            .post(self.url("ilink/bot/msg/notifystart"))
            .headers(self.build_headers())
            .body(body)
            .send()
            .await?
            .text()
            .await?;
        Ok(serde_json::from_str(&resp)?)
    }

    pub async fn notify_stop(&self) -> anyhow::Result<NotifyResp> {
        let body = serde_json::to_string(&NotifyReq {
            base_info: self.build_base_info(),
        })?;
        let resp = self
            .http
            .post(self.url("ilink/bot/msg/notifystop"))
            .headers(self.build_headers())
            .body(body)
            .send()
            .await?
            .text()
            .await?;
        Ok(serde_json::from_str(&resp)?)
    }

    // ── QR Login APIs ─────────────────────────────────────────────────────

    pub async fn fetch_qr_code(
        existing_tokens: &[String],
    ) -> anyhow::Result<QrCodeResponse> {
        let client = reqwest::Client::builder()
            .timeout(DEFAULT_API_TIMEOUT)
            .build()?;

        let body = serde_json::to_string(&QrLoginStartReq {
            local_token_list: existing_tokens.to_vec(),
        })?;

        let url = format!("{QR_LOGIN_BASE_URL}/ilink/bot/get_bot_qrcode?bot_type={DEFAULT_BOT_TYPE}");

        let resp = client
            .post(&url)
            .header(CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await?
            .text()
            .await?;

        Ok(serde_json::from_str(&resp)?)
    }

    pub async fn poll_qr_status(
        base_url: &str,
        qrcode: &str,
        verify_code: Option<&str>,
    ) -> anyhow::Result<QrStatusResponse> {
        let client = reqwest::Client::builder()
            .timeout(DEFAULT_LONG_POLL_TIMEOUT + Duration::from_secs(5))
            .build()?;

        let mut url = format!(
            "{}/ilink/bot/get_qrcode_status?qrcode={}",
            base_url.trim_end_matches('/'),
            urlencoding::encode(qrcode)
        );
        if let Some(code) = verify_code {
            use std::fmt::Write;
            write!(url, "&verify_code={}", urlencoding::encode(code)).ok();
        }

        match client.get(&url).send().await {
            Ok(resp) => {
                let text = resp.text().await?;
                Ok(serde_json::from_str(&text)?)
            }
            Err(e) if e.is_timeout() => Ok(QrStatusResponse {
                status: "wait".to_string(),
                ..Default::default()
            }),
            Err(e) => {
                tracing::warn!(error = %e, "pollQRStatus network error, treating as wait");
                Ok(QrStatusResponse {
                    status: "wait".to_string(),
                    ..Default::default()
                })
            }
        }
    }
}

impl Default for QrStatusResponse {
    fn default() -> Self {
        Self {
            status: "wait".to_string(),
            bot_token: None,
            ilink_bot_id: None,
            baseurl: None,
            ilink_user_id: None,
            redirect_host: None,
        }
    }
}

fn random_wechat_uin() -> String {
    let n: u32 = rand::thread_rng().gen();
    BASE64.encode(n.to_string().as_bytes())
}

/// Sanitize bot_agent to UA-style format. Invalid tokens are dropped.
fn sanitize_bot_agent(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return DEFAULT_BOT_AGENT.to_string();
    }

    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    let accepted: Vec<&str> = tokens.into_iter().filter(|t| PRODUCT_RE.is_match(t)).collect();

    if accepted.is_empty() {
        DEFAULT_BOT_AGENT.to_string()
    } else {
        let joined = accepted.join(" ");
        if joined.len() <= 256 {
            joined
        } else {
            DEFAULT_BOT_AGENT.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_bot_agent() {
        assert_eq!(sanitize_bot_agent(""), DEFAULT_BOT_AGENT);
        assert_eq!(sanitize_bot_agent("MyBot/1.2.0"), "MyBot/1.2.0");
        assert_eq!(
            sanitize_bot_agent("MyBot/1.2.0 Extra/3.0"),
            "MyBot/1.2.0 Extra/3.0"
        );
        assert_eq!(sanitize_bot_agent("!!!invalid!!!"), DEFAULT_BOT_AGENT);
    }
}
