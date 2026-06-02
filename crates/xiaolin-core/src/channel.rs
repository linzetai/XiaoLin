use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::tool::Tool;

/// Metadata describing a channel plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelMeta {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub aliases: Vec<String>,
}

/// Capabilities a channel supports.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChannelCapabilities {
    #[serde(default)]
    pub direct_message: bool,
    #[serde(default)]
    pub group_chat: bool,
    #[serde(default)]
    pub media: bool,
    #[serde(default)]
    pub reactions: bool,
    #[serde(default)]
    pub threads: bool,
    #[serde(default)]
    pub streaming: bool,
}

/// A file attachment (local path + MIME type).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub file_path: String,
    #[serde(default)]
    pub mime_type: Option<String>,
    #[serde(default)]
    pub file_name: Option<String>,
}

/// Inbound message from a channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundMessage {
    pub channel_id: String,
    /// Which account this message came from (for multi-account routing).
    #[serde(default)]
    pub account_id: Option<String>,
    pub sender_id: String,
    pub chat_id: String,
    pub message_id: String,
    pub text: String,
    #[serde(default)]
    pub msg_type: String,
    /// "p2p" for direct messages, "group" for group chats.
    #[serde(default)]
    pub chat_type: String,
    /// Whether the bot was @mentioned in this message.
    #[serde(default)]
    pub bot_mentioned: bool,
    #[serde(default)]
    pub extra: serde_json::Value,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
}

/// Outbound message to send through a channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundMessage {
    pub target_id: String,
    pub target_type: String,
    pub text: String,
    #[serde(default)]
    pub reply_to: Option<String>,
    /// Image key for channels that support image messages (e.g., Feishu image_key).
    #[serde(default)]
    pub image_key: Option<String>,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
}

/// Result of handling a webhook.
#[derive(Debug)]
pub enum WebhookResult {
    Challenge(serde_json::Value),
    Messages(Vec<InboundMessage>),
    Ignored,
}

/// Trait for channel plugins (Feishu, Slack, Discord, etc.).
///
/// A channel plugin bridges an external messaging platform into XiaoLin.
/// It handles inbound webhooks, provides outbound messaging, and
/// registers channel-specific tools for agents.
#[async_trait]
pub trait ChannelPlugin: Send + Sync {
    /// Unique channel identifier (e.g., "feishu", "slack").
    fn meta(&self) -> &ChannelMeta;

    /// Channel capabilities.
    fn capabilities(&self) -> ChannelCapabilities;

    /// Verify the authenticity of an inbound webhook using platform-specific
    /// signature or token validation. Called **before** `handle_webhook` with the
    /// raw request body bytes and HTTP headers so that channels can perform HMAC,
    /// Ed25519, or other verification.
    ///
    /// `headers` maps **lower-cased** header names to their string values.
    ///
    /// Return `Ok(())` to accept the request, or `Err(...)` to reject it.
    /// The default implementation accepts all requests (no verification).
    async fn verify_webhook(
        &self,
        _headers: &BTreeMap<String, String>,
        _raw_body: &[u8],
    ) -> anyhow::Result<()> {
        Ok(())
    }

    /// Parse an inbound webhook payload into messages.
    /// Returns `WebhookResult::Challenge` for verification challenges,
    /// `WebhookResult::Messages` for parsed messages, or `Ignored` otherwise.
    async fn handle_webhook(&self, payload: serde_json::Value) -> anyhow::Result<WebhookResult>;

    /// Send a message through this channel.
    async fn send_message(&self, msg: &OutboundMessage) -> anyhow::Result<serde_json::Value>;

    /// Reply to a specific message.
    async fn reply_message(
        &self,
        message_id: &str,
        text: &str,
    ) -> anyhow::Result<serde_json::Value>;

    /// Send a streaming-friendly placeholder reply (e.g. a card message).
    /// Channels that support streaming should override this to send an updatable
    /// message type (e.g. Feishu interactive cards). Returns the message data
    /// including a `message_id` that can be passed to `update_message`.
    async fn reply_streaming_placeholder(
        &self,
        message_id: &str,
        text: &str,
    ) -> anyhow::Result<serde_json::Value> {
        self.reply_message(message_id, text).await
    }

    /// Update (edit) an existing message in-place. Used for streaming output:
    /// send a placeholder first, then progressively update it with new content.
    /// Returns the updated message data, or an error if the channel doesn't support it.
    async fn update_message(
        &self,
        message_id: &str,
        text: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let _ = (message_id, text);
        anyhow::bail!("update_message not supported by this channel")
    }

    /// Return tools this channel provides to agents.
    fn tools(&self) -> Vec<Arc<dyn Tool>>;

    /// Check if this channel is properly configured and can operate.
    async fn probe(&self) -> anyhow::Result<bool> {
        Ok(true)
    }

    /// Start any background tasks (e.g. WebSocket long connections).
    /// The `inbound_tx` sender is used to push messages received from
    /// the external platform into the gateway's processing pipeline.
    /// Default implementation does nothing (pure webhook channels).
    async fn start(
        &self,
        _inbound_tx: mpsc::UnboundedSender<InboundMessage>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    /// Stop any background tasks and clean up resources.
    /// Called when a channel is being replaced or removed.
    /// Default implementation does nothing.
    async fn stop(&self) -> anyhow::Result<()> {
        Ok(())
    }

    /// Send an interactive card message for ask_question support.
    /// `target_id` is the chat/user ID, `target_type` hints the ID type (e.g. "chat_id", "open_id").
    /// Returns the message_id of the sent card.
    /// Default implementation returns an error (not supported).
    async fn send_interactive_card(
        &self,
        _target_id: &str,
        _target_type: &str,
        _card: &serde_json::Value,
    ) -> anyhow::Result<String> {
        anyhow::bail!("Interactive cards not supported by this channel")
    }

    /// Update an interactive card message (e.g., after user answers).
    /// Default implementation returns an error (not supported).
    async fn update_interactive_card(
        &self,
        _message_id: &str,
        _card: &serde_json::Value,
    ) -> anyhow::Result<()> {
        anyhow::bail!("Interactive cards not supported by this channel")
    }

    /// Check if this channel supports interactive questions (ask_question).
    fn supports_interactive_questions(&self) -> bool {
        false
    }

    /// Called when the gateway starts processing an inbound message (e.g. before
    /// the LLM call). Channels can use this to send a "typing" indicator.
    /// `chat_id` is the conversation, `message_id` the triggering inbound message.
    async fn on_processing_start(&self, _chat_id: &str, _message_id: &str) {}

    /// Called when the gateway finishes processing an inbound message (after the
    /// reply has been sent). Channels can cancel the "typing" indicator here.
    async fn on_processing_end(&self, _chat_id: &str, _message_id: &str) {}

    /// Connection mode this channel uses. Informational.
    fn connection_mode(&self) -> &str {
        "webhook"
    }
}

/// Registry holding all loaded channel plugins.
pub struct ChannelRegistry {
    channels: HashMap<String, Arc<dyn ChannelPlugin>>,
}

impl ChannelRegistry {
    pub fn new() -> Self {
        Self {
            channels: HashMap::new(),
        }
    }

    pub fn register(&mut self, channel: Arc<dyn ChannelPlugin>) {
        let id = channel.meta().id.clone();
        tracing::info!(channel_id = %id, name = %channel.meta().name, "registered channel plugin");
        self.channels.insert(id, channel);
    }

    /// Remove a channel from the registry. Returns the removed plugin if it existed.
    pub fn unregister(&mut self, channel_id: &str) -> Option<Arc<dyn ChannelPlugin>> {
        let removed = self.channels.remove(channel_id);
        if removed.is_some() {
            tracing::info!(channel_id = channel_id, "unregistered channel plugin");
        }
        removed
    }

    pub fn get(&self, channel_id: &str) -> Option<&Arc<dyn ChannelPlugin>> {
        self.channels.get(channel_id)
    }

    pub fn list(&self) -> Vec<&ChannelMeta> {
        self.channels.values().map(|c| c.meta()).collect()
    }

    pub fn all_plugins(&self) -> Vec<&Arc<dyn ChannelPlugin>> {
        self.channels.values().collect()
    }

    pub fn all_tools(&self) -> Vec<Arc<dyn Tool>> {
        self.channels.values().flat_map(|ch| ch.tools()).collect()
    }

    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }
}

impl Default for ChannelRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Resolved account configuration after merging top-level defaults with account-specific overrides.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedAccountConfig {
    /// Account ID (None for single-account channels).
    pub account_id: Option<String>,
    pub app_id: Option<String>,
    pub app_secret: Option<String>,
    pub verification_token: Option<String>,
    pub encrypt_key: Option<String>,
    pub domain: Option<String>,
    pub reply_mode: Option<String>,
}

/// Resolve merged config for a specific account.
/// Top-level fields are defaults; account fields override.
pub fn resolve_account_config(
    channel_config: &crate::config::ChannelConfig,
    account_id: Option<&str>,
) -> Option<ResolvedAccountConfig> {
    // If no accounts defined, use top-level as single account
    if channel_config.accounts.is_empty() {
        return Some(ResolvedAccountConfig {
            account_id: None,
            app_id: channel_config.app_id.clone(),
            app_secret: channel_config.app_secret.clone(),
            verification_token: channel_config.verification_token.clone(),
            encrypt_key: channel_config.encrypt_key.clone(),
            domain: channel_config.domain.clone(),
            reply_mode: channel_config.reply_mode.clone(),
        });
    }

    // Resolve account_id: explicit → default_account → first account
    let resolved_id = account_id
        .or(channel_config.default_account.as_deref())
        .or_else(|| channel_config.accounts.keys().next().map(|s| s.as_str()));

    let acc_id = resolved_id?;
    let acc = channel_config.accounts.get(acc_id)?;

    // Merge: top-level defaults + account overrides
    Some(ResolvedAccountConfig {
        account_id: Some(acc_id.to_string()),
        app_id: acc.app_id.clone().or(channel_config.app_id.clone()),
        app_secret: acc.app_secret.clone().or(channel_config.app_secret.clone()),
        verification_token: acc
            .verification_token
            .clone()
            .or(channel_config.verification_token.clone()),
        encrypt_key: acc
            .encrypt_key
            .clone()
            .or(channel_config.encrypt_key.clone()),
        domain: acc.domain.clone().or(channel_config.domain.clone()),
        reply_mode: acc.reply_mode.clone().or(channel_config.reply_mode.clone()),
    })
}
