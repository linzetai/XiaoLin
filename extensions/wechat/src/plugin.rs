use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use dashmap::DashMap;
use serde_json::json;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use xiaolin_core::channel::{
    ChannelCapabilities, ChannelMeta, ChannelPlugin, InboundMessage, OutboundMessage,
    WebhookResult,
};
use xiaolin_core::tool::Tool;

use crate::api::client::WechatApiClient;
use crate::auth::credential;
use crate::config::WechatChannelConfig;
use crate::media::download::cleanup_old_media;
use crate::message::{outbound_to_weixin, outbound_to_weixin_with_media};
use crate::monitor::WechatMonitor;
use crate::typing::TypingManager;

struct AccountState {
    client: WechatApiClient,
    cancel: CancellationToken,
    monitor_handle: Option<JoinHandle<()>>,
}

/// Maps a message_id → (chat_id, context_token) so that reply_message can
/// look up the correct recipient and context from a numeric message ID.
const REPLY_CACHE_MAX_ENTRIES: usize = 10_000;

pub struct ReplyCache {
    entries: DashMap<String, (String, Option<String>)>,
}

impl Default for ReplyCache {
    fn default() -> Self {
        Self {
            entries: DashMap::new(),
        }
    }
}

impl ReplyCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, message_id: &str, chat_id: &str, context_token: Option<&str>) {
        if self.entries.len() > REPLY_CACHE_MAX_ENTRIES {
            let target_len = REPLY_CACHE_MAX_ENTRIES / 2;
            let to_remove = self.entries.len().saturating_sub(target_len);
            tracing::warn!(
                len = self.entries.len(),
                max = REPLY_CACHE_MAX_ENTRIES,
                evicting = to_remove,
                "wechat reply cache exceeded capacity, evicting oldest entries"
            );
            let keys: Vec<String> = self
                .entries
                .iter()
                .take(to_remove)
                .map(|entry| entry.key().clone())
                .collect();
            for key in keys {
                self.entries.remove(&key);
            }
        }
        self.entries.insert(
            message_id.to_string(),
            (chat_id.to_string(), context_token.map(String::from)),
        );
    }

    pub fn get(&self, message_id: &str) -> Option<(String, Option<String>)> {
        self.entries.get(message_id).map(|v| v.clone())
    }

    pub fn remove(&self, message_id: &str) {
        self.entries.remove(message_id);
    }
}

/// Per-account (account_id, peer_id) → context_token mapping.
/// Mirrors openclaw-weixin's strategy: in-memory + disk persistence.
pub struct ContextTokenCache {
    tokens: DashMap<(String, String), String>,
}

impl Default for ContextTokenCache {
    fn default() -> Self {
        Self {
            tokens: DashMap::new(),
        }
    }
}

impl ContextTokenCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(&self, account_id: &str, peer_id: &str, token: &str) {
        self.tokens.insert(
            (account_id.to_string(), peer_id.to_string()),
            token.to_string(),
        );
        self.persist(account_id);
    }

    pub fn get(&self, account_id: &str, peer_id: &str) -> Option<String> {
        self.tokens
            .get(&(account_id.to_string(), peer_id.to_string()))
            .map(|v| v.clone())
    }

    /// Restore persisted tokens for an account from disk (call once at startup).
    pub fn restore(&self, account_id: &str) {
        let path = Self::file_path(account_id);
        let Ok(data) = std::fs::read_to_string(&path) else {
            return;
        };
        let map: std::collections::HashMap<String, String> = match serde_json::from_str(&data) {
            Ok(m) => m,
            Err(_) => return,
        };
        let mut count = 0usize;
        for (peer_id, token) in map {
            self.tokens
                .insert((account_id.to_string(), peer_id), token);
            count += 1;
        }
        tracing::info!(account_id, count, "restored context tokens from disk");
    }

    fn persist(&self, account_id: &str) {
        let prefix = account_id.to_string();
        let mut map = std::collections::HashMap::new();
        for entry in &self.tokens {
            let (ref aid, ref pid) = *entry.key();
            if aid == &prefix {
                map.insert(pid.clone(), entry.value().clone());
            }
        }
        let path = Self::file_path(account_id);
        if let Ok(json) = serde_json::to_string(&map) {
            tokio::task::spawn_blocking(move || {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).ok();
                }
                if let Err(e) = std::fs::write(&path, json) {
                    tracing::warn!(error = %e, path = %path.display(), "failed to persist wechat context tokens");
                }
            });
        }
    }

    fn file_path(account_id: &str) -> std::path::PathBuf {
        xiaolin_core::paths::resolve_state_dir()
            .join("data")
            .join(format!("wechat-ctx-tokens-{account_id}.json"))
    }
}

pub struct WechatPlugin {
    meta: ChannelMeta,
    config: WechatChannelConfig,
    accounts: DashMap<String, AccountState>,
    pub context_tokens: Arc<ContextTokenCache>,
    pub typing_manager: Arc<TypingManager>,
    pub reply_cache: Arc<ReplyCache>,
    typing_tasks: DashMap<String, CancellationToken>,
}

impl WechatPlugin {
    pub fn new(config: WechatChannelConfig) -> Self {
        Self {
            meta: ChannelMeta {
                id: "wechat".to_string(),
                name: "WeChat".to_string(),
                description: "WeChat channel via getUpdates long-poll".to_string(),
                aliases: vec!["weixin".to_string()],
            },
            config,
            accounts: DashMap::new(),
            context_tokens: Arc::new(ContextTokenCache::new()),
            typing_manager: Arc::new(TypingManager::new()),
            reply_cache: Arc::new(ReplyCache::new()),
            typing_tasks: DashMap::new(),
        }
    }

    fn find_client_for_target(&self, target_id: &str) -> Option<(String, WechatApiClient)> {
        self.accounts
            .iter()
            .find(|entry| self.context_tokens.get(entry.key(), target_id).is_some())
            .map(|entry| (entry.key().clone(), entry.value().client.clone()))
    }
}

#[async_trait]
impl ChannelPlugin for WechatPlugin {
    fn meta(&self) -> &ChannelMeta {
        &self.meta
    }

    fn capabilities(&self) -> ChannelCapabilities {
        ChannelCapabilities {
            direct_message: true,
            group_chat: false,
            media: true,
            reactions: false,
            threads: false,
            streaming: false,
        }
    }

    #[allow(clippy::unnecessary_literal_bound)]
    fn connection_mode(&self) -> &str {
        "longpoll"
    }

    async fn verify_webhook(
        &self,
        _headers: &BTreeMap<String, String>,
        _raw_body: &[u8],
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn handle_webhook(
        &self,
        _payload: serde_json::Value,
    ) -> anyhow::Result<WebhookResult> {
        Ok(WebhookResult::Ignored)
    }

    async fn start(
        &self,
        inbound_tx: mpsc::UnboundedSender<InboundMessage>,
    ) -> anyhow::Result<()> {
        match cleanup_old_media(Duration::from_secs(24 * 3600)).await {
            Ok(n) if n > 0 => tracing::info!(count = n, "cleaned up old wechat media files"),
            Err(e) => tracing::debug!(error = %e, "media cleanup skipped"),
            _ => {}
        }

        let creds = credential::list_credentials();

        if creds.is_empty() {
            tracing::info!("no wechat credentials found, channel idle");
            return Ok(());
        }

        let poll_timeout = Duration::from_millis(self.config.long_poll_timeout_ms);

        for (account_id, _cred) in &creds {
            self.context_tokens.restore(account_id);
        }

        for (account_id, cred) in creds {
            let client = WechatApiClient::new(
                &cred.base_url,
                &cred.token,
                self.config.bot_agent.as_deref(),
                poll_timeout,
            )?;

            let cancel = CancellationToken::new();
            let monitor = WechatMonitor::new(
                client.clone(),
                account_id.clone(),
                inbound_tx.clone(),
                cancel.clone(),
                poll_timeout,
                self.reply_cache.clone(),
                self.context_tokens.clone(),
                self.config.cdn_base_url.clone(),
            );

            let handle = tokio::spawn(async move {
                monitor.run().await;
            });

            self.accounts.insert(
                account_id.clone(),
                AccountState {
                    client,
                    cancel,
                    monitor_handle: Some(handle),
                },
            );

            tracing::info!(account_id = %account_id, "started wechat monitor");
        }

        Ok(())
    }

    async fn stop(&self) -> anyhow::Result<()> {
        for entry in &self.accounts {
            entry.value().cancel.cancel();
        }
        for mut entry in &mut self.accounts.iter_mut() {
            if let Some(handle) = entry.value_mut().monitor_handle.take() {
                handle.await.ok();
            }
        }
        self.accounts.clear();
        tracing::info!("all wechat monitors stopped");
        Ok(())
    }

    async fn send_message(&self, msg: &OutboundMessage) -> anyhow::Result<serde_json::Value> {
        let (account_id, client) = self
            .find_client_for_target(&msg.target_id)
            .ok_or_else(|| anyhow::anyhow!("no wechat account available for target"))?;

        let context_token = self.context_tokens.get(&account_id, &msg.target_id);
        if msg.attachments.is_empty() {
            let weixin_msg = outbound_to_weixin(msg, context_token.as_deref());
            client.send_message(weixin_msg).await?;
        } else {
            let weixin_msgs = outbound_to_weixin_with_media(
                msg,
                context_token.as_deref(),
                &client,
                &self.config.cdn_base_url,
            )
            .await?;
            for weixin_msg in weixin_msgs {
                client.send_message(weixin_msg).await?;
            }
        }

        Ok(json!({"ok": true}))
    }

    async fn on_processing_start(&self, chat_id: &str, _message_id: &str) {
        if let Some((account_id, client)) = self.find_client_for_target(chat_id) {
            let ctx_token = self.context_tokens.get(&account_id, chat_id);
            if let Err(e) = self
                .typing_manager
                .start_typing(&client, &account_id, chat_id, ctx_token.as_deref())
                .await
            {
                tracing::debug!(error = %e, chat_id, "failed to send typing start (ignored)");
                return;
            }

            let cancel = CancellationToken::new();
            self.typing_tasks
                .insert(chat_id.to_string(), cancel.clone());

            let typing_mgr = self.typing_manager.clone();
            let ctx_tokens = self.context_tokens.clone();
            let chat_id_owned = chat_id.to_string();
            let account_id_owned = account_id.clone();
            let client_owned = client.clone();

            tokio::spawn(async move {
                let interval = Duration::from_secs(5);
                loop {
                    tokio::select! {
                        () = tokio::time::sleep(interval) => {}
                        () = cancel.cancelled() => break,
                    }
                    let ctx = ctx_tokens.get(&account_id_owned, &chat_id_owned);
                    if let Err(e) = typing_mgr
                        .start_typing(
                            &client_owned,
                            &account_id_owned,
                            &chat_id_owned,
                            ctx.as_deref(),
                        )
                        .await
                    {
                        tracing::debug!(error = %e, "typing keepalive failed (ignored)");
                        break;
                    }
                }
            });
        }
    }

    async fn on_processing_end(&self, chat_id: &str, _message_id: &str) {
        if let Some((_, cancel)) = self.typing_tasks.remove(chat_id) {
            cancel.cancel();
        }
        if let Some((account_id, client)) = self.find_client_for_target(chat_id) {
            let ctx_token = self.context_tokens.get(&account_id, chat_id);
            if let Err(e) = self
                .typing_manager
                .stop_typing(&client, &account_id, chat_id, ctx_token.as_deref())
                .await
            {
                tracing::debug!(error = %e, chat_id, "failed to send typing stop (ignored)");
            }
        }
    }

    async fn reply_message(
        &self,
        message_id: &str,
        text: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let (chat_id, context_token) = self
            .reply_cache
            .get(message_id)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "no reply cache entry for message_id={message_id}, cannot determine recipient"
                )
            })?;

        tracing::info!(
            message_id,
            chat_id = %chat_id,
            has_context_token = context_token.is_some(),
            text_len = text.len(),
            "reply_message: sending reply"
        );

        let (_, client) = self
            .find_client_for_target(&chat_id)
            .ok_or_else(|| anyhow::anyhow!("no wechat account available for target"))?;

        let msg = OutboundMessage {
            target_id: chat_id.clone(),
            target_type: "p2p".to_string(),
            text: text.to_string(),
            reply_to: None,
            image_key: None,
            attachments: vec![],
        };

        let weixin_msg = outbound_to_weixin(&msg, context_token.as_deref());
        client.send_message(weixin_msg).await?;
        self.reply_cache.remove(message_id);

        tracing::info!(message_id, chat_id = %chat_id, "reply_message: sent OK");
        Ok(json!({"ok": true}))
    }

    fn tools(&self) -> Vec<Arc<dyn Tool>> {
        vec![]
    }

    async fn probe(&self) -> anyhow::Result<bool> {
        Ok(!self.accounts.is_empty())
    }

    fn supports_interactive_questions(&self) -> bool {
        false
    }
}
