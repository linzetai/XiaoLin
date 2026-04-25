use serde::{Deserialize, Serialize};

/// Feishu/Lark API client for sending and receiving messages.
pub struct FeishuClient {
    app_id: String,
    app_secret: String,
    base_url: String,
    http: reqwest::Client,
    tenant_access_token: tokio::sync::RwLock<Option<CachedToken>>,
    /// User access token (OAuth) for user-scoped APIs; optional.
    user_access_token: Option<String>,
}

struct CachedToken {
    token: String,
    expires_at: std::time::Instant,
}

#[derive(Debug, Serialize)]
struct TokenRequest<'a> {
    app_id: &'a str,
    app_secret: &'a str,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    code: i32,
    msg: String,
    tenant_access_token: Option<String>,
    expire: Option<u64>,
}

#[derive(Debug, Serialize)]
struct SendMessageRequest<'a> {
    receive_id: &'a str,
    msg_type: &'a str,
    content: &'a str,
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    code: i32,
    msg: String,
    data: Option<serde_json::Value>,
}

impl FeishuClient {
    pub fn new(app_id: &str, app_secret: &str) -> Self {
        Self::with_base_url_user_token(
            app_id,
            app_secret,
            "https://open.feishu.cn/open-apis",
            None,
        )
    }

    pub fn new_with_user_token(
        app_id: &str,
        app_secret: &str,
        user_access_token: Option<String>,
    ) -> Self {
        Self::with_base_url_user_token(
            app_id,
            app_secret,
            "https://open.feishu.cn/open-apis",
            user_access_token,
        )
    }

    pub fn with_base_url(app_id: &str, app_secret: &str, base_url: &str) -> Self {
        Self::with_base_url_user_token(app_id, app_secret, base_url, None)
    }

    pub fn with_base_url_user_token(
        app_id: &str,
        app_secret: &str,
        base_url: &str,
        user_access_token: Option<String>,
    ) -> Self {
        Self {
            app_id: app_id.to_string(),
            app_secret: app_secret.to_string(),
            base_url: base_url.trim_end_matches('/').to_string(),
            http: reqwest::Client::builder()
                .user_agent("FastClaw/0.1.0")
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            tenant_access_token: tokio::sync::RwLock::new(None),
            user_access_token,
        }
    }

    /// Whether a non-empty user OAuth access token is configured (user-scoped APIs).
    pub fn user_oauth_configured(&self) -> bool {
        self.user_access_token
            .as_ref()
            .map(|t| !t.trim().is_empty())
            .unwrap_or(false)
    }

    fn require_user_token(&self) -> anyhow::Result<&str> {
        let t = self
            .user_access_token
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        match t {
            Some(tok) => Ok(tok),
            None => Err(anyhow::anyhow!(crate::oauth::OAuthConfig::missing_user_token_message())),
        }
    }

    async fn parse_envelope(resp: reqwest::Response) -> anyhow::Result<serde_json::Value> {
        let v: serde_json::Value = resp.json().await?;
        let code = v.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
        if code != 0 {
            let msg = v
                .get("msg")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error");
            anyhow::bail!("Feishu API error ({code}): {msg}");
        }
        Ok(v.get("data").cloned().unwrap_or(serde_json::Value::Null))
    }

    /// GET request authenticated with the user access token.
    pub async fn user_get(&self, path: &str) -> anyhow::Result<serde_json::Value> {
        self.user_get_query(path, &[]).await
    }

    /// GET with extra query pairs (both keys and values must be encodable as UTF-8).
    pub async fn user_get_query(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> anyhow::Result<serde_json::Value> {
        let token = self.require_user_token()?;
        let path = if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{path}")
        };
        let mut url = reqwest::Url::parse(&format!("{}{}", self.base_url, path))
            .map_err(|e| anyhow::anyhow!("invalid URL for Feishu request: {e}"))?;
        {
            let mut pairs = url.query_pairs_mut();
            for (k, v) in query {
                pairs.append_pair(k, v);
            }
        }
        let resp = self
            .http
            .get(url.as_str())
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await?;
        Self::parse_envelope(resp).await
    }

    /// POST JSON body authenticated with the user access token.
    pub async fn user_post_json(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let token = self.require_user_token()?;
        let path = if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{path}")
        };
        let url = format!("{}{}", self.base_url, path);
        let resp = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {token}"))
            .json(body)
            .send()
            .await?;
        Self::parse_envelope(resp).await
    }

    /// POST `multipart/form-data` authenticated with the user access token.
    pub async fn user_post_multipart(
        &self,
        path: &str,
        form: reqwest::multipart::Form,
    ) -> anyhow::Result<serde_json::Value> {
        let token = self.require_user_token()?;
        let path = if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{path}")
        };
        let url = format!("{}{}", self.base_url, path);
        let resp = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {token}"))
            .multipart(form)
            .send()
            .await?;
        Self::parse_envelope(resp).await
    }

    /// Download a message attachment / resource using the user token; streams chunks into memory.
    pub async fn download_message_resource(
        &self,
        message_id: &str,
        file_key: &str,
        resource_type: &str,
    ) -> anyhow::Result<Vec<u8>> {
        use futures::StreamExt;
        let token = self.require_user_token()?;
        let url = format!(
            "{}/im/v1/messages/{message_id}/resources/{file_key}",
            self.base_url
        );
        let resp = self
            .http
            .get(&url)
            .query(&[("type", resource_type)])
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await?;
        if !resp.status().is_success() {
            anyhow::bail!("download resource failed: HTTP {}", resp.status());
        }
        let mut stream = resp.bytes_stream();
        let mut out = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            out.extend_from_slice(&chunk);
        }
        Ok(out)
    }

    pub async fn get_tenant_token(&self) -> anyhow::Result<String> {
        {
            let cached = self.tenant_access_token.read().await;
            if let Some(ref t) = *cached {
                if t.expires_at > std::time::Instant::now() {
                    return Ok(t.token.clone());
                }
            }
        }

        let url = format!("{}/auth/v3/tenant_access_token/internal", self.base_url);
        let body = TokenRequest {
            app_id: &self.app_id,
            app_secret: &self.app_secret,
        };

        let resp: TokenResponse = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await?
            .json()
            .await?;

        if resp.code != 0 {
            anyhow::bail!("Feishu token error ({}): {}", resp.code, resp.msg);
        }

        let token = resp
            .tenant_access_token
            .ok_or_else(|| anyhow::anyhow!("no token in response"))?;
        let expire = resp.expire.unwrap_or(7200);

        let cached = CachedToken {
            token: token.clone(),
            expires_at: std::time::Instant::now()
                + std::time::Duration::from_secs(expire.saturating_sub(300)),
        };

        *self.tenant_access_token.write().await = Some(cached);
        tracing::info!("refreshed Feishu tenant access token");
        Ok(token)
    }

    /// Send a text message to a user or chat.
    pub async fn send_message(
        &self,
        receive_id: &str,
        receive_id_type: &str,
        text: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_token().await?;
        let url = format!(
            "{}/im/v1/messages?receive_id_type={}",
            self.base_url, receive_id_type
        );
        let content = serde_json::json!({"text": text}).to_string();
        let body = SendMessageRequest {
            receive_id,
            msg_type: "text",
            content: &content,
        };

        let resp: ApiResponse = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .json(&body)
            .send()
            .await?
            .json()
            .await?;

        if resp.code != 0 {
            anyhow::bail!("Feishu send error ({}): {}", resp.code, resp.msg);
        }

        Ok(resp.data.unwrap_or(serde_json::Value::Null))
    }

    /// Reply to a specific message.
    pub async fn reply_message(
        &self,
        message_id: &str,
        text: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_token().await?;
        let url = format!("{}/im/v1/messages/{}/reply", self.base_url, message_id);
        let content = serde_json::json!({"text": text}).to_string();
        let body = serde_json::json!({
            "msg_type": "text",
            "content": content,
        });

        let resp: ApiResponse = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .json(&body)
            .send()
            .await?
            .json()
            .await?;

        if resp.code != 0 {
            anyhow::bail!("Feishu reply error ({}): {}", resp.code, resp.msg);
        }

        Ok(resp.data.unwrap_or(serde_json::Value::Null))
    }

    /// Reply with an interactive card message (used as streaming placeholder).
    pub async fn reply_card_message(
        &self,
        message_id: &str,
        content: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_token().await?;
        let url = format!("{}/im/v1/messages/{}/reply", self.base_url, message_id);
        let card = Self::build_card(content);
        let body = serde_json::json!({
            "msg_type": "interactive",
            "content": card,
        });

        let resp: ApiResponse = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .json(&body)
            .send()
            .await?
            .json()
            .await?;

        if resp.code != 0 {
            anyhow::bail!("Feishu reply card error ({}): {}", resp.code, resp.msg);
        }

        Ok(resp.data.unwrap_or(serde_json::Value::Null))
    }

    /// Update (PATCH) an existing card message. Only `interactive` messages support PATCH.
    pub async fn update_message(
        &self,
        message_id: &str,
        text: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_token().await?;
        let url = format!("{}/im/v1/messages/{}", self.base_url, message_id);
        let card = Self::build_card(text);
        let body = serde_json::json!({
            "msg_type": "interactive",
            "content": card,
        });

        let resp: ApiResponse = self
            .http
            .patch(&url)
            .header("Authorization", format!("Bearer {}", token))
            .json(&body)
            .send()
            .await?
            .json()
            .await?;

        if resp.code != 0 {
            anyhow::bail!("Feishu update error ({}): {}", resp.code, resp.msg);
        }

        Ok(resp.data.unwrap_or(serde_json::Value::Null))
    }

    /// Build a Feishu interactive card JSON string from markdown-like text.
    fn build_card(content: &str) -> String {
        serde_json::json!({
            "elements": [{
                "tag": "div",
                "text": {
                    "tag": "lark_md",
                    "content": content
                }
            }]
        })
        .to_string()
    }

    /// Upload an image to Feishu and return the image_key.
    /// `image_type` is one of: "image/png", "image/jpeg", "image/gif", "image/webp".
    /// `data` is the raw image bytes.
    pub async fn upload_image(
        &self,
        image_type: &str,
        data: &[u8],
    ) -> anyhow::Result<String> {
        let token = self.get_tenant_token().await?;
        let url = format!("{}/im/v1/images", self.base_url);

        let form = reqwest::multipart::Form::new()
            .part("image_type", reqwest::multipart::Part::text(image_type.to_string()))
            .part(
                "image",
                reqwest::multipart::Part::bytes(data.to_vec())
                    .file_name("image.png")
                    .mime_str(image_type)?,
            );

        let resp: ApiResponse = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .multipart(form)
            .send()
            .await?
            .json()
            .await?;

        if resp.code != 0 {
            anyhow::bail!("Feishu upload image error ({}): {}", resp.code, resp.msg);
        }

        let data = resp.data.ok_or_else(|| anyhow::anyhow!("no data in upload image response"))?;
        let image_key = data
            .get("image_key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("no image_key in upload image response"))?;

        Ok(image_key.to_string())
    }

    /// Send an image message to a chat/user.
    pub async fn send_image(&self, receive_id: &str, receive_id_type: &str, image_key: &str) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_token().await?;
        let url = format!("{}/im/v1/messages?receive_id_type={}", self.base_url, receive_id_type);

        let content = serde_json::json!({ "image_key": image_key }).to_string();

        let body = serde_json::json!({
            "receive_id": receive_id,
            "msg_type": "image",
            "content": content,
        });

        let resp: ApiResponse = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .json(&body)
            .send()
            .await?
            .json()
            .await?;

        if resp.code != 0 {
            anyhow::bail!("Feishu send image error ({}): {}", resp.code, resp.msg);
        }

        Ok(resp.data.unwrap_or(serde_json::Value::Null))
    }

    /// Reply with an image to a specific message.
    pub async fn reply_image(&self, message_id: &str, image_key: &str) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_token().await?;
        let url = format!("{}/im/v1/messages/{}/reply", self.base_url, message_id);

        let content = serde_json::json!({ "image_key": image_key }).to_string();

        let body = serde_json::json!({
            "msg_type": "image",
            "content": content,
        });

        let resp: ApiResponse = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .json(&body)
            .send()
            .await?
            .json()
            .await?;

        if resp.code != 0 {
            anyhow::bail!("Feishu reply image error ({}): {}", resp.code, resp.msg);
        }

        Ok(resp.data.unwrap_or(serde_json::Value::Null))
    }

    /// Retrieve recent messages from a chat.
    pub async fn get_chat_messages(
        &self,
        chat_id: &str,
        page_size: u32,
    ) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_token().await?;
        let url = format!(
            "{}/im/v1/messages?container_id_type=chat&container_id={}&page_size={}",
            self.base_url, chat_id, page_size
        );

        let resp: ApiResponse = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await?
            .json()
            .await?;

        if resp.code != 0 {
            anyhow::bail!("Feishu get messages error ({}): {}", resp.code, resp.msg);
        }

        Ok(resp.data.unwrap_or(serde_json::Value::Null))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_new() {
        let client = FeishuClient::new("test_id", "test_secret");
        assert_eq!(client.app_id, "test_id");
    }
}
