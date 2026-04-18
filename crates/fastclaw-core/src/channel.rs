use async_trait::async_trait;
use std::collections::BTreeMap;
use serde::{Deserialize, Serialize};
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

/// Inbound message from a channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundMessage {
    pub channel_id: String,
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
}

/// Outbound message to send through a channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundMessage {
    pub target_id: String,
    pub target_type: String,
    pub text: String,
    #[serde(default)]
    pub reply_to: Option<String>,
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
/// A channel plugin bridges an external messaging platform into FastClaw.
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

    pub fn get(&self, channel_id: &str) -> Option<&Arc<dyn ChannelPlugin>> {
        self.channels.get(channel_id)
    }

    pub fn list(&self) -> Vec<&ChannelMeta> {
        self.channels.values().map(|c| c.meta()).collect()
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
