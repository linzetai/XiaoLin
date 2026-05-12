//! Process-based channel plugin implementation.
//!
//! Provides a `ChannelPlugin` that delegates all operations to an external
//! process via the JSON-RPC dispatcher.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;
use tokio::sync::mpsc;

use fastclaw_core::channel::{
    ChannelCapabilities, ChannelMeta, ChannelPlugin, InboundMessage, OutboundMessage, WebhookResult,
};
use fastclaw_core::channel_plugin::{ChannelPluginConfig, ProcessChannelConfig};
use fastclaw_core::tool::{Tool, ToolKind, ToolParameterSchema, ToolResult};

use crate::rpc::JsonRpcDispatcher;

// ---------------------------------------------------------------------------
// ProcessChannelPlugin
// ---------------------------------------------------------------------------

/// A `ChannelPlugin` backed by an external process communicating over JSON-RPC.
pub struct ProcessChannelPlugin {
    config: ChannelPluginConfig,
    dispatcher: JsonRpcDispatcher,
    meta: ChannelMeta,
}

impl ProcessChannelPlugin {
    pub fn new(config: ChannelPluginConfig) -> Self {
        let meta = ChannelMeta {
            id: config.id.clone(),
            name: config.name.clone(),
            description: config.description.clone().unwrap_or_default(),
            aliases: vec![],
        };
        let dispatcher = JsonRpcDispatcher::new(&config.id);
        Self {
            config,
            dispatcher,
            meta,
        }
    }

    /// Spawn the plugin process and perform the initialization handshake.
    pub async fn initialize(&self, account_config: serde_json::Value) -> anyhow::Result<()> {
        let process_config = self.config.process.as_ref().ok_or_else(|| {
            anyhow::anyhow!("channel plugin '{}' has no process config", self.config.id)
        })?;

        self.dispatcher
            .ensure_process(
                &process_config.command,
                &process_config.args,
                &process_config.env,
            )
            .await?;

        // Send initialize request with account config.
        let result = self
            .dispatcher
            .call(
                "initialize",
                json!({
                    "config": account_config,
                    "protocolVersion": "1.0"
                }),
            )
            .await?;

        tracing::info!(
            plugin_id = %self.config.id,
            ?result,
            "channel plugin initialized"
        );

        // Start the background reader for responses and notifications.
        self.dispatcher.start_reader();

        Ok(())
    }

    fn process_config(&self) -> anyhow::Result<&ProcessChannelConfig> {
        self.config.process.as_ref().ok_or_else(|| {
            anyhow::anyhow!("channel plugin '{}' has no process config", self.config.id)
        })
    }
}

#[async_trait]
impl ChannelPlugin for ProcessChannelPlugin {
    fn meta(&self) -> &ChannelMeta {
        &self.meta
    }

    fn capabilities(&self) -> ChannelCapabilities {
        // Could be fetched from plugin via JSON-RPC, but for now use defaults.
        ChannelCapabilities::default()
    }

    async fn verify_webhook(
        &self,
        headers: &BTreeMap<String, String>,
        raw_body: &[u8],
    ) -> anyhow::Result<()> {
        let body_str = String::from_utf8_lossy(raw_body).to_string();
        let headers_map: serde_json::Value = headers
            .iter()
            .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
            .collect::<serde_json::Map<String, serde_json::Value>>()
            .into();

        self.dispatcher
            .call(
                "verify_webhook",
                json!({
                    "headers": headers_map,
                    "body": body_str
                }),
            )
            .await?;
        Ok(())
    }

    async fn handle_webhook(&self, payload: serde_json::Value) -> anyhow::Result<WebhookResult> {
        let result = self
            .dispatcher
            .call("handle_webhook", json!({ "payload": payload }))
            .await?;

        // Parse the result into a WebhookResult.
        if let Some(challenge) = result.get("challenge") {
            return Ok(WebhookResult::Challenge(challenge.clone()));
        }

        if let Some(messages) = result.get("messages") {
            let msgs: Vec<InboundMessage> =
                serde_json::from_value(messages.clone()).unwrap_or_default();
            return Ok(WebhookResult::Messages(msgs));
        }

        Ok(WebhookResult::Ignored)
    }

    async fn send_message(&self, msg: &OutboundMessage) -> anyhow::Result<serde_json::Value> {
        self.dispatcher
            .call("send_message", serde_json::to_value(msg)?)
            .await
    }

    async fn reply_message(
        &self,
        message_id: &str,
        text: &str,
    ) -> anyhow::Result<serde_json::Value> {
        self.dispatcher
            .call(
                "reply_message",
                json!({
                    "messageId": message_id,
                    "text": text
                }),
            )
            .await
    }

    async fn reply_streaming_placeholder(
        &self,
        message_id: &str,
        text: &str,
    ) -> anyhow::Result<serde_json::Value> {
        match self
            .dispatcher
            .call(
                "reply_streaming_placeholder",
                json!({
                    "messageId": message_id,
                    "text": text
                }),
            )
            .await
        {
            Ok(v) => Ok(v),
            Err(_) => {
                // Fallback to regular reply if not supported.
                self.reply_message(message_id, text).await
            }
        }
    }

    async fn update_message(
        &self,
        message_id: &str,
        text: &str,
    ) -> anyhow::Result<serde_json::Value> {
        self.dispatcher
            .call(
                "update_message",
                json!({
                    "messageId": message_id,
                    "text": text
                }),
            )
            .await
    }

    fn tools(&self) -> Vec<Arc<dyn Tool>> {
        // Clone the dispatcher for each tool.
        let dispatcher = self.dispatcher.clone();
        self.config
            .tools
            .iter()
            .map(|tool_def| {
                Arc::new(ChannelProxyTool {
                    channel_id: self.config.id.clone(),
                    tool_name: tool_def.name.clone(),
                    tool_description: tool_def.description.clone(),
                    tool_parameters: tool_def.parameters.clone(),
                    dispatcher: dispatcher.clone(),
                }) as Arc<dyn Tool>
            })
            .collect()
    }

    async fn probe(&self) -> anyhow::Result<bool> {
        match self.dispatcher.call("probe", json!({})).await {
            Ok(v) => Ok(v.as_bool().unwrap_or(true)),
            Err(e) => {
                tracing::warn!(
                    plugin_id = %self.config.id,
                    error = %e,
                    "channel plugin probe failed"
                );
                Ok(false)
            }
        }
    }

    async fn start(&self, inbound_tx: mpsc::UnboundedSender<InboundMessage>) -> anyhow::Result<()> {
        // Set up a notification handler that converts inbound_message notifications
        // to InboundMessage and forwards them.
        let (notif_tx, mut notif_rx) = mpsc::unbounded_channel();
        self.dispatcher.set_notification_channel(notif_tx).await;

        let channel_id = self.config.id.clone();
        tokio::spawn(async move {
            while let Some(notif) = notif_rx.recv().await {
                if notif.method == "inbound_message" {
                    if let Some(params) = notif.params {
                        match serde_json::from_value::<InboundMessage>(params) {
                            Ok(msg) => {
                                if inbound_tx.send(msg).is_err() {
                                    tracing::debug!(
                                        channel_id = %channel_id,
                                        "inbound channel closed, stopping notification handler"
                                    );
                                    break;
                                }
                            }
                            Err(e) => {
                                tracing::warn!(
                                    channel_id = %channel_id,
                                    error = %e,
                                    "failed to parse inbound_message notification"
                                );
                            }
                        }
                    }
                }
            }
        });

        self.dispatcher.call("start", json!({})).await?;
        Ok(())
    }

    async fn stop(&self) -> anyhow::Result<()> {
        let _ = self.dispatcher.call("stop", json!({})).await;
        self.dispatcher.shutdown().await
    }

    fn connection_mode(&self) -> &str {
        self.process_config()
            .map(|pc| match pc.transport {
                fastclaw_core::channel_plugin::ProcessTransport::Stdio => "websocket",
                fastclaw_core::channel_plugin::ProcessTransport::Http => "webhook",
            })
            .unwrap_or("unknown")
    }
}

// ---------------------------------------------------------------------------
// ChannelProxyTool — proxy tool that delegates to the plugin process
// ---------------------------------------------------------------------------

/// A tool exposed by a process-based channel plugin.
/// Delegates execution to the plugin via JSON-RPC.
pub struct ChannelProxyTool {
    #[allow(dead_code)]
    channel_id: String,
    tool_name: String,
    tool_description: String,
    tool_parameters: serde_json::Value,
    dispatcher: JsonRpcDispatcher,
}

#[async_trait]
impl Tool for ChannelProxyTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        &self.tool_description
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        serde_json::from_value(self.tool_parameters.clone()).unwrap_or(ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: Default::default(),
            required: vec![],
        })
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Other
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let params: serde_json::Value = serde_json::from_str(arguments).unwrap_or(json!({}));
        match self
            .dispatcher
            .call(
                "execute_tool",
                json!({
                    "toolName": self.tool_name,
                    "params": params
                }),
            )
            .await
        {
            Ok(result) => {
                let output = result.to_string();
                ToolResult::ok(output)
            }
            Err(e) => ToolResult::err(format!("channel tool '{}' error: {e}", self.tool_name)),
        }
    }
}
