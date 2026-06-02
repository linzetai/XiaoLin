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
        Self::with_base_url_user_token(app_id, app_secret, "https://open.feishu.cn/open-apis", None)
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
                .user_agent("XiaoLin/0.1.0")
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
            None => Err(anyhow::anyhow!(
                crate::oauth::OAuthConfig::missing_user_token_message()
            )),
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

    /// PUT JSON body authenticated with the user access token.
    pub async fn user_put_json(
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
            .put(&url)
            .header("Authorization", format!("Bearer {token}"))
            .json(body)
            .send()
            .await?;
        Self::parse_envelope(resp).await
    }

    /// PATCH JSON body authenticated with the user access token.
    pub async fn user_patch_json(
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
            .patch(&url)
            .header("Authorization", format!("Bearer {token}"))
            .json(body)
            .send()
            .await?;
        Self::parse_envelope(resp).await
    }

    /// DELETE authenticated with the user access token.
    pub async fn user_delete(&self, path: &str) -> anyhow::Result<serde_json::Value> {
        let token = self.require_user_token()?;
        let path = if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{path}")
        };
        let url = format!("{}{}", self.base_url, path);
        let resp = self
            .http
            .delete(&url)
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await?;
        Self::parse_envelope(resp).await
    }

    /// DELETE with JSON body authenticated with the user access token.
    pub async fn user_delete_with_body(
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
            .delete(&url)
            .header("Authorization", format!("Bearer {token}"))
            .json(body)
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

    /// Feishu error codes indicating the reply target was withdrawn or not found.
    const WITHDRAWN_REPLY_CODES: &'static [i32] = &[230011, 231003];

    fn is_withdrawn_reply_error(code: i32, msg: &str) -> bool {
        if Self::WITHDRAWN_REPLY_CODES.contains(&code) {
            return true;
        }
        let lower = msg.to_lowercase();
        lower.contains("withdrawn") || lower.contains("not found")
    }

    /// Reply to a specific message. If `fallback_chat_id` is provided and the
    /// reply target was withdrawn/deleted, automatically falls back to a direct
    /// send to that chat.
    pub async fn reply_message(
        &self,
        message_id: &str,
        text: &str,
    ) -> anyhow::Result<serde_json::Value> {
        self.reply_message_with_fallback(message_id, text, None)
            .await
    }

    /// Reply with optional fallback to direct send when the target is withdrawn.
    pub async fn reply_message_with_fallback(
        &self,
        message_id: &str,
        text: &str,
        fallback_chat_id: Option<&str>,
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
            if let Some(chat_id) = fallback_chat_id {
                if Self::is_withdrawn_reply_error(resp.code, &resp.msg) {
                    tracing::warn!(
                        message_id,
                        code = resp.code,
                        "reply target withdrawn, falling back to direct send"
                    );
                    return self.send_message(chat_id, "chat_id", text).await;
                }
            }
            anyhow::bail!("Feishu reply error ({}): {}", resp.code, resp.msg);
        }

        Ok(resp.data.unwrap_or(serde_json::Value::Null))
    }

    /// Reply with an interactive card message (used as streaming placeholder).
    /// If `fallback_chat_id` is provided and the target was withdrawn, falls
    /// back to sending the card as a new message to that chat.
    pub async fn reply_card_message(
        &self,
        message_id: &str,
        content: &str,
    ) -> anyhow::Result<serde_json::Value> {
        self.reply_card_message_with_fallback(message_id, content, None)
            .await
    }

    /// Reply card with optional fallback.
    pub async fn reply_card_message_with_fallback(
        &self,
        message_id: &str,
        content: &str,
        fallback_chat_id: Option<&str>,
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
            if let Some(chat_id) = fallback_chat_id {
                if Self::is_withdrawn_reply_error(resp.code, &resp.msg) {
                    tracing::warn!(
                        message_id,
                        code = resp.code,
                        "reply card target withdrawn, falling back to direct card send"
                    );
                    return self
                        .send_card(chat_id, "chat_id", &serde_json::from_str(&card)?)
                        .await;
                }
            }
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
    /// Uses Card Kit 2.0 format with full-width markdown element for proper streaming display.
    fn build_card(content: &str) -> String {
        serde_json::json!({
            "schema": "2.0",
            "config": {
                "streaming_mode": true,
                "summary": { "content": "[Streaming...]" },
                "streaming_config": {
                    "print_frequency_ms": { "default": 50 },
                    "print_step": { "default": 1 }
                }
            },
            "body": {
                "elements": [{
                    "tag": "markdown",
                    "content": content,
                    "element_id": "content"
                }]
            }
        })
        .to_string()
    }

    /// Upload an image to Feishu and return the image_key.
    /// `image_type` is one of: "image/png", "image/jpeg", "image/gif", "image/webp".
    /// `data` is the raw image bytes.
    pub async fn upload_image(&self, image_type: &str, data: &[u8]) -> anyhow::Result<String> {
        let token = self.get_tenant_token().await?;
        let url = format!("{}/im/v1/images", self.base_url);

        let form = reqwest::multipart::Form::new()
            .part(
                "image_type",
                reqwest::multipart::Part::text(image_type.to_string()),
            )
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

        let data = resp
            .data
            .ok_or_else(|| anyhow::anyhow!("no data in upload image response"))?;
        let image_key = data
            .get("image_key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("no image_key in upload image response"))?;

        Ok(image_key.to_string())
    }

    /// Send an image message to a chat/user.
    pub async fn send_image(
        &self,
        receive_id: &str,
        receive_id_type: &str,
        image_key: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_token().await?;
        let url = format!(
            "{}/im/v1/messages?receive_id_type={}",
            self.base_url, receive_id_type
        );

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
    pub async fn reply_image(
        &self,
        message_id: &str,
        image_key: &str,
    ) -> anyhow::Result<serde_json::Value> {
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

    // -----------------------------------------------------------------------
    // IM enhanced methods (rich text, files, management, threads, reactions, pins)
    // -----------------------------------------------------------------------

    /// Send a rich text (post) message with structured content.
    /// `post_content` should be a JSON object with `zh_cn` / `en_us` keys containing
    /// `title` and `content` (array of paragraph arrays).
    pub async fn send_rich_text(
        &self,
        receive_id: &str,
        receive_id_type: &str,
        post_content: &serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_token().await?;
        let url = format!(
            "{}/im/v1/messages?receive_id_type={}",
            self.base_url, receive_id_type
        );
        let content = post_content.to_string();
        let body = SendMessageRequest {
            receive_id,
            msg_type: "post",
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
            anyhow::bail!("Feishu send rich text error ({}): {}", resp.code, resp.msg);
        }
        Ok(resp.data.unwrap_or(serde_json::Value::Null))
    }

    /// Send an interactive card message (for markdown rendering).
    pub async fn send_card(
        &self,
        receive_id: &str,
        receive_id_type: &str,
        card: &serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_token().await?;
        let url = format!(
            "{}/im/v1/messages?receive_id_type={}",
            self.base_url, receive_id_type
        );
        let content = card.to_string();
        let body = SendMessageRequest {
            receive_id,
            msg_type: "interactive",
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
            anyhow::bail!("Feishu send card error ({}): {}", resp.code, resp.msg);
        }
        Ok(resp.data.unwrap_or(serde_json::Value::Null))
    }

    /// Update an interactive card message by message_id.
    /// Uses the Feishu PATCH /im/v1/messages/:message_id API.
    pub async fn update_card_message(
        &self,
        message_id: &str,
        card: &serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_token().await?;
        let url = format!("{}/im/v1/messages/{}", self.base_url, message_id);
        let body = serde_json::json!({
            "content": serde_json::to_string(card).unwrap_or_default(),
            "msg_type": "interactive"
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
            anyhow::bail!("Feishu update card error ({}): {}", resp.code, resp.msg);
        }
        Ok(resp.data.unwrap_or_default())
    }

    /// Upload a file to Feishu and return the file_key.
    /// `file_type`: opus, mp4, pdf, doc, xls, ppt, stream.
    pub async fn upload_file(
        &self,
        file_type: &str,
        data: &[u8],
        file_name: &str,
    ) -> anyhow::Result<String> {
        let token = self.get_tenant_token().await?;
        let url = format!("{}/im/v1/files", self.base_url);
        let form = reqwest::multipart::Form::new()
            .part(
                "file_type",
                reqwest::multipart::Part::text(file_type.to_string()),
            )
            .part(
                "file_name",
                reqwest::multipart::Part::text(file_name.to_string()),
            )
            .part(
                "file",
                reqwest::multipart::Part::bytes(data.to_vec()).file_name(file_name.to_string()),
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
            anyhow::bail!("Feishu upload file error ({}): {}", resp.code, resp.msg);
        }
        let data = resp
            .data
            .ok_or_else(|| anyhow::anyhow!("no data in upload file response"))?;
        data.get("file_key")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow::anyhow!("no file_key in upload file response"))
    }

    /// Send a file message.
    pub async fn send_file(
        &self,
        receive_id: &str,
        receive_id_type: &str,
        file_key: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_token().await?;
        let url = format!(
            "{}/im/v1/messages?receive_id_type={}",
            self.base_url, receive_id_type
        );
        let content = serde_json::json!({ "file_key": file_key }).to_string();
        let body = SendMessageRequest {
            receive_id,
            msg_type: "file",
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
            anyhow::bail!("Feishu send file error ({}): {}", resp.code, resp.msg);
        }
        Ok(resp.data.unwrap_or(serde_json::Value::Null))
    }

    /// Reply with a file to a specific message.
    pub async fn reply_file(
        &self,
        message_id: &str,
        file_key: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_token().await?;
        let url = format!("{}/im/v1/messages/{}/reply", self.base_url, message_id);
        let content = serde_json::json!({ "file_key": file_key }).to_string();
        let body = serde_json::json!({
            "msg_type": "file",
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
            anyhow::bail!("Feishu reply file error ({}): {}", resp.code, resp.msg);
        }
        Ok(resp.data.unwrap_or(serde_json::Value::Null))
    }

    /// Get a single message by its ID.
    pub async fn get_message(&self, message_id: &str) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_token().await?;
        let url = format!("{}/im/v1/messages/{}", self.base_url, message_id);
        let resp: ApiResponse = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await?
            .json()
            .await?;
        if resp.code != 0 {
            anyhow::bail!("Feishu get message error ({}): {}", resp.code, resp.msg);
        }
        Ok(resp.data.unwrap_or(serde_json::Value::Null))
    }

    /// Edit an existing text or post message in-place.
    pub async fn edit_message(
        &self,
        message_id: &str,
        msg_type: &str,
        content: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_token().await?;
        let url = format!("{}/im/v1/messages/{}", self.base_url, message_id);
        let body = serde_json::json!({
            "msg_type": msg_type,
            "content": content,
        });
        let resp: ApiResponse = self
            .http
            .put(&url)
            .header("Authorization", format!("Bearer {}", token))
            .json(&body)
            .send()
            .await?
            .json()
            .await?;
        if resp.code != 0 {
            anyhow::bail!("Feishu edit message error ({}): {}", resp.code, resp.msg);
        }
        Ok(resp.data.unwrap_or(serde_json::Value::Null))
    }

    /// Delete (recall) a message.
    pub async fn delete_message(&self, message_id: &str) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_token().await?;
        let url = format!("{}/im/v1/messages/{}", self.base_url, message_id);
        let resp: ApiResponse = self
            .http
            .delete(&url)
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await?
            .json()
            .await?;
        if resp.code != 0 {
            anyhow::bail!("Feishu delete message error ({}): {}", resp.code, resp.msg);
        }
        Ok(resp.data.unwrap_or(serde_json::Value::Null))
    }

    /// Forward a message to another chat/user.
    pub async fn forward_message(
        &self,
        message_id: &str,
        receive_id: &str,
        receive_id_type: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_token().await?;
        let url = format!(
            "{}/im/v1/messages/{}/forward?receive_id_type={}",
            self.base_url, message_id, receive_id_type
        );
        let body = serde_json::json!({ "receive_id": receive_id });
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
            anyhow::bail!("Feishu forward message error ({}): {}", resp.code, resp.msg);
        }
        Ok(resp.data.unwrap_or(serde_json::Value::Null))
    }

    /// List messages in a thread/topic.
    pub async fn get_thread_messages(
        &self,
        thread_id: &str,
        page_size: u32,
        page_token: Option<&str>,
    ) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_token().await?;
        let mut url = format!(
            "{}/im/v1/messages?container_id_type=thread&container_id={}&page_size={}",
            self.base_url, thread_id, page_size
        );
        if let Some(pt) = page_token {
            url.push_str(&format!("&page_token={pt}"));
        }
        let resp: ApiResponse = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await?
            .json()
            .await?;
        if resp.code != 0 {
            anyhow::bail!(
                "Feishu get thread messages error ({}): {}",
                resp.code,
                resp.msg
            );
        }
        Ok(resp.data.unwrap_or(serde_json::Value::Null))
    }

    /// Reply to a message with optional reply_in_thread.
    pub async fn reply_message_ext(
        &self,
        message_id: &str,
        text: &str,
        reply_in_thread: bool,
    ) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_token().await?;
        let url = format!("{}/im/v1/messages/{}/reply", self.base_url, message_id);
        let content = serde_json::json!({"text": text}).to_string();
        let mut body = serde_json::json!({
            "msg_type": "text",
            "content": content,
        });
        if reply_in_thread {
            body["reply_in_thread"] = serde_json::Value::Bool(true);
        }
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

    // -----------------------------------------------------------------------
    // Reactions
    // -----------------------------------------------------------------------

    /// Add an emoji reaction to a message.
    pub async fn add_reaction(
        &self,
        message_id: &str,
        emoji_type: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_token().await?;
        let url = format!("{}/im/v1/messages/{}/reactions", self.base_url, message_id);
        let body = serde_json::json!({
            "reaction_type": { "emoji_type": emoji_type }
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
            anyhow::bail!("Feishu add reaction error ({}): {}", resp.code, resp.msg);
        }
        Ok(resp.data.unwrap_or(serde_json::Value::Null))
    }

    /// Remove an emoji reaction from a message.
    pub async fn remove_reaction(
        &self,
        message_id: &str,
        reaction_id: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_token().await?;
        let url = format!(
            "{}/im/v1/messages/{}/reactions/{}",
            self.base_url, message_id, reaction_id
        );
        let resp: ApiResponse = self
            .http
            .delete(&url)
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await?
            .json()
            .await?;
        if resp.code != 0 {
            anyhow::bail!("Feishu remove reaction error ({}): {}", resp.code, resp.msg);
        }
        Ok(resp.data.unwrap_or(serde_json::Value::Null))
    }

    /// List reactions on a message.
    pub async fn list_reactions(
        &self,
        message_id: &str,
        emoji_type: Option<&str>,
    ) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_token().await?;
        let mut url = format!("{}/im/v1/messages/{}/reactions", self.base_url, message_id);
        if let Some(et) = emoji_type {
            url.push_str(&format!("?reaction_type={et}"));
        }
        let resp: ApiResponse = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await?
            .json()
            .await?;
        if resp.code != 0 {
            anyhow::bail!("Feishu list reactions error ({}): {}", resp.code, resp.msg);
        }
        Ok(resp.data.unwrap_or(serde_json::Value::Null))
    }

    // -----------------------------------------------------------------------
    // Pins
    // -----------------------------------------------------------------------

    /// Pin a message.
    pub async fn create_pin(&self, message_id: &str) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_token().await?;
        let url = format!("{}/im/v1/pins", self.base_url);
        let body = serde_json::json!({ "message_id": message_id });
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
            anyhow::bail!("Feishu create pin error ({}): {}", resp.code, resp.msg);
        }
        Ok(resp.data.unwrap_or(serde_json::Value::Null))
    }

    /// Unpin a message.
    pub async fn remove_pin(&self, message_id: &str) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_token().await?;
        let url = format!("{}/im/v1/pins/{}", self.base_url, message_id);
        let resp: ApiResponse = self
            .http
            .delete(&url)
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await?
            .json()
            .await?;
        if resp.code != 0 {
            anyhow::bail!("Feishu remove pin error ({}): {}", resp.code, resp.msg);
        }
        Ok(resp.data.unwrap_or(serde_json::Value::Null))
    }

    /// List pinned messages in a chat.
    pub async fn list_pins(
        &self,
        chat_id: &str,
        page_size: Option<u32>,
        page_token: Option<&str>,
    ) -> anyhow::Result<serde_json::Value> {
        let token = self.get_tenant_token().await?;
        let ps = page_size.unwrap_or(20).clamp(1, 100);
        let mut url = format!(
            "{}/im/v1/pins?chat_id={}&page_size={}",
            self.base_url, chat_id, ps
        );
        if let Some(pt) = page_token {
            url.push_str(&format!("&page_token={pt}"));
        }
        let resp: ApiResponse = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await?
            .json()
            .await?;
        if resp.code != 0 {
            anyhow::bail!("Feishu list pins error ({}): {}", resp.code, resp.msg);
        }
        Ok(resp.data.unwrap_or(serde_json::Value::Null))
    }

    // -----------------------------------------------------------------------
    // Card Kit 2.0 streaming
    // -----------------------------------------------------------------------

    /// Create a Card Kit 2.0 streaming card entity. Returns the card_id.
    pub async fn create_streaming_card(
        &self,
        card_json: &serde_json::Value,
    ) -> anyhow::Result<String> {
        let token = self.get_tenant_token().await?;
        let url = format!("{}/cardkit/v1/cards", self.base_url);
        let body = serde_json::json!({
            "type": "card_json",
            "data": card_json.to_string(),
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
            anyhow::bail!(
                "Feishu create streaming card error ({}): {}",
                resp.code,
                resp.msg
            );
        }
        resp.data
            .as_ref()
            .and_then(|d| d.get("card_id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow::anyhow!("no card_id in create streaming card response"))
    }

    /// Update a streaming card element with new content.
    pub async fn update_streaming_element(
        &self,
        card_id: &str,
        element_id: &str,
        content: &str,
    ) -> anyhow::Result<()> {
        let token = self.get_tenant_token().await?;
        let url = format!(
            "{}/cardkit/v1/cards/{}/elements/{}",
            self.base_url, card_id, element_id
        );
        let body = serde_json::json!({
            "content": content,
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
            anyhow::bail!(
                "Feishu update streaming element error ({}): {}",
                resp.code,
                resp.msg
            );
        }
        Ok(())
    }

    /// Finalize a streaming card (set final content and disable streaming).
    pub async fn finalize_streaming_card(
        &self,
        card_id: &str,
        card_json: &serde_json::Value,
    ) -> anyhow::Result<()> {
        let token = self.get_tenant_token().await?;
        let url = format!("{}/cardkit/v1/cards/{}", self.base_url, card_id);
        let body = serde_json::json!({
            "type": "card_json",
            "data": card_json.to_string(),
        });
        let resp: ApiResponse = self
            .http
            .put(&url)
            .header("Authorization", format!("Bearer {}", token))
            .json(&body)
            .send()
            .await?
            .json()
            .await?;
        if resp.code != 0 {
            anyhow::bail!(
                "Feishu finalize streaming card error ({}): {}",
                resp.code,
                resp.msg
            );
        }
        Ok(())
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
