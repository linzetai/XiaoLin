use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;
use xiaolin_core::config_access::{navigate_config, persist_config_key, set_nested_key};
use xiaolin_core::tool::{Tool, ToolGroup, ToolKind, ToolParameterSchema, ToolResult};

use crate::state::AppState;

const SUPPORTED_CHANNELS: &[&str] = &[
    "feishu", "slack", "discord", "telegram", "whatsapp", "matrix", "msteams", "wechat",
];

pub struct ListChannelsTool {
    state: AppState,
}

impl ListChannelsTool {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }
}

#[async_trait]
impl Tool for ListChannelsTool {
    fn name(&self) -> &str {
        "list_channels"
    }

    fn description(&self) -> &str {
        "List all configured channel integrations (Feishu, Slack, Discord, Telegram, etc.) with their status. \
         Returns each channel's id, enabled state, connection mode, and whether credentials are configured. \
         Use this to show the user which channels are set up before adding or modifying one."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: HashMap::new(),
            required: vec![],
        }
    }

    async fn execute(&self, _arguments: &str) -> ToolResult {
        let live: serde_json::Value = (**self.state.cfg.config_live.load()).clone();

        let channels = navigate_config(&live, "channels");
        let bindings = navigate_config(&live, "bindings");

        let registry = self.state.ext.channel_registry.read().await;
        let mut result = Vec::new();
        if let Some(obj) = channels.as_object() {
            for (id, cfg) in obj {
                let enabled = cfg
                    .get("enabled")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let has_creds = cfg.get("appId").and_then(|v| v.as_str()).is_some()
                    || cfg.get("appSecret").and_then(|v| v.as_str()).is_some();
                let mode = cfg
                    .get("connectionMode")
                    .and_then(|v| v.as_str())
                    .unwrap_or("webhook");
                let agent = cfg
                    .get("agentId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("main");
                let running = registry.get(id).is_some();
                result.push(json!({
                    "id": id,
                    "enabled": enabled,
                    "running": running,
                    "hasCredentials": has_creds,
                    "connectionMode": mode,
                    "agentId": agent,
                }));
            }
        }
        drop(registry);

        if result.is_empty() {
            ToolResult::ok(json!({
                "channels": [],
                "bindings": bindings,
                "hint": "No channels configured. Supported: feishu, slack, discord, telegram, whatsapp, matrix, msteams. Use add_channel to set one up."
            }).to_string())
        } else {
            ToolResult::ok(
                json!({
                    "channels": result,
                    "bindings": bindings,
                })
                .to_string(),
            )
        }
    }
}

pub struct AddChannelTool {
    state: AppState,
}

impl AddChannelTool {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }
}

#[async_trait]
impl Tool for AddChannelTool {
    fn name(&self) -> &str {
        "add_channel"
    }

    fn description(&self) -> &str {
        "Add or update a channel integration. Supported channels: feishu, slack, discord, telegram, whatsapp, matrix, msteams.\n\
         Required fields per channel:\n\
         - feishu: appId, appSecret (from Feishu Open Platform), optional connectionMode (websocket|webhook), domain, replyMode\n\
         - slack: appSecret (Bot User OAuth Token xoxb-...), optional verificationToken (Signing Secret), appId\n\
         - discord: appSecret (Bot Token), appId (Application ID)\n\
         - telegram: appSecret (Bot Token from @BotFather)\n\
         - whatsapp: appId (Phone Number ID), appSecret (Permanent Token), verificationToken (Webhook Verify Token)\n\
         - matrix: domain (Homeserver URL), appId (User ID @bot:server), appSecret (Access Token)\n\
         - msteams: appId (Bot App ID), appSecret (Bot App Password)\n\n\
         Ask the user for credentials step by step. Never guess or fabricate credentials.\n\
         The channel is saved, a default binding is auto-created, and the plugin is started immediately."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "channel".to_string(),
            json!({
                "type": "string",
                "enum": SUPPORTED_CHANNELS,
                "description": "Channel type to add"
            }),
        );
        props.insert(
            "config".to_string(),
            json!({
                "type": "object",
                "description": "Channel configuration object with camelCase keys: enabled (bool), appId, appSecret, verificationToken, encryptKey, agentId, connectionMode, domain, replyMode, userAccessToken. Only include fields the user has provided."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["channel".to_string(), "config".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("invalid JSON: {e}")),
        };

        let channel = match args.get("channel").and_then(|v| v.as_str()) {
            Some(c) => c.to_string(),
            None => return ToolResult::err("missing required field 'channel'".to_string()),
        };

        if !SUPPORTED_CHANNELS.contains(&channel.as_str()) {
            return ToolResult::err(format!(
                "unknown channel '{channel}'. Supported: {}",
                SUPPORTED_CHANNELS.join(", ")
            ));
        }

        let mut config = match args.get("config") {
            Some(v) if v.is_object() => v.clone(),
            _ => return ToolResult::err("missing or invalid 'config' object".to_string()),
        };

        if config.get("enabled").is_none() {
            config["enabled"] = json!(true);
        }

        let agent_id = config
            .get("agentId")
            .and_then(|v| v.as_str())
            .unwrap_or("main")
            .to_string();

        // 1. Save channel config
        let ch_key = format!("channels.{channel}");
        {
            let mut live: serde_json::Value = (**self.state.cfg.config_live.load()).clone();
            if set_nested_key(&mut live, &ch_key, config.clone()).is_err() {
                return ToolResult::err("failed to update config".to_string());
            }
            self.state.cfg.config_live.store(Arc::new(live));
        }
        if let Err(e) = persist_config_key(&ch_key, &config) {
            tracing::warn!(channel = %channel, error = %e, "add_channel: failed to persist channel config");
        }

        // 2. Auto-create binding if none exists for this channel
        self.ensure_binding(&channel, &agent_id).await;

        // 3. Start the channel plugin
        let mut status_parts = vec![format!("Channel '{channel}' configured and saved.")];
        match self.state.reload_channel(&channel).await {
            Ok(()) => {
                status_parts.push("Plugin started successfully.".to_string());

                // Register channel-specific tools as channel-scoped (visible only to channel requests)
                let registry = self.state.ext.channel_registry.read().await;
                if let Some(plugin) = registry.get(&channel) {
                    for tool in plugin.tools() {
                        self.state.rt.tool_registry.register_channel_scoped(tool);
                    }
                }
            }
            Err(e) => {
                status_parts.push(format!(
                    "Plugin start failed: {e}. A restart may be needed."
                ));
            }
        }
        status_parts.push(format!("Webhook URL: /webhook/{channel}"));

        // Broadcast event so the UI refreshes channel lists
        let event = json!({
            "type": "event",
            "event": "channels.changed",
            "data": { "channelId": channel, "action": "added" }
        });
        let _ = self.state.strm.ws_broadcast.send(event.to_string());

        tracing::info!(channel = %channel, "channel added/updated via tool");
        ToolResult::ok(status_parts.join(" "))
    }
}

impl AddChannelTool {
    async fn ensure_binding(&self, channel: &str, agent_id: &str) {
        let live_snapshot = self.state.cfg.config_live.load();
        let bindings = live_snapshot.get("bindings").cloned().unwrap_or(json!([]));
        if let Some(arr) = bindings.as_array() {
            let already = arr.iter().any(|b| {
                b.get("match")
                    .and_then(|m| m.get("channel"))
                    .and_then(|c| c.as_str())
                    == Some(channel)
            });
            if already {
                return;
            }
        }

        let mut live: serde_json::Value = (**self.state.cfg.config_live.load()).clone();
        let binding = json!({
            "agentId": agent_id,
            "match": { "channel": channel }
        });
        let mut new_bindings = bindings.as_array().cloned().unwrap_or_default();
        new_bindings.push(binding);
        let bindings_val = serde_json::Value::Array(new_bindings);

        if set_nested_key(&mut live, "bindings", bindings_val.clone()).is_ok() {
            self.state.cfg.config_live.store(Arc::new(live));
            let _ = persist_config_key("bindings", &bindings_val);
            tracing::info!(channel, agent_id, "auto-created binding for channel");
        }
    }
}

pub struct RemoveChannelTool {
    state: AppState,
}

impl RemoveChannelTool {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }
}

#[async_trait]
impl Tool for RemoveChannelTool {
    fn name(&self) -> &str {
        "remove_channel"
    }

    fn description(&self) -> &str {
        "Remove a channel integration by disabling it. \
         Use list_channels first to see what's configured. \
         This persists the change — the channel will not reconnect on next restart."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "channel".to_string(),
            json!({
                "type": "string",
                "description": "Channel id to remove (e.g. feishu, slack, discord)"
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["channel".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("invalid JSON: {e}")),
        };

        let channel = match args.get("channel").and_then(|v| v.as_str()) {
            Some(c) => c.to_string(),
            None => return ToolResult::err("missing required field 'channel'".to_string()),
        };

        let disabled = json!({"enabled": false});
        let key = format!("channels.{channel}");

        {
            let mut live: serde_json::Value = (**self.state.cfg.config_live.load()).clone();
            if set_nested_key(&mut live, &key, disabled.clone()).is_err() {
                return ToolResult::err("failed to update config".to_string());
            }
            self.state.cfg.config_live.store(Arc::new(live));
        }

        if let Err(e) = persist_config_key(&key, &disabled) {
            return ToolResult::ok(format!(
                "Channel '{channel}' disabled in memory but failed to persist: {e}"
            ));
        }

        // Stop the running plugin if any
        {
            let mut reg = self.state.ext.channel_registry.write().await;
            if let Some(plugin) = reg.get(&channel) {
                if let Err(e) = plugin.stop().await {
                    tracing::warn!(channel = %channel, error = %e, "failed to stop channel plugin on remove");
                }
                reg.unregister(&channel);
            }
        }

        let event = json!({
            "type": "event",
            "event": "channels.changed",
            "data": { "channelId": channel, "action": "removed" }
        });
        let _ = self.state.strm.ws_broadcast.send(event.to_string());

        tracing::info!(channel = %channel, "channel removed via tool");
        ToolResult::ok(format!(
            "Channel '{channel}' disabled, stopped, and saved. It will not reconnect on restart."
        ))
    }
}

/// Tool that lets agents proactively push messages to IM channels.
///
/// Bridges the gap between coding sessions and IM: an agent can call this tool
/// to send build results, test reports, or any notification to a Feishu/Slack/etc.
/// chat. Works from any session type (HTTP, WebSocket, IM, or cron).
pub struct NotifyChannelTool {
    state: AppState,
}

impl NotifyChannelTool {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }
}

#[async_trait]
impl Tool for NotifyChannelTool {
    fn name(&self) -> &str {
        "notify_channel"
    }

    fn description(&self) -> &str {
        "Send a message to an IM channel (Feishu, Slack, Discord, WeChat, etc.). \
         Use this to push notifications, build results, test reports, images, files, or any content \
         to a team chat. Requires a running channel plugin.\n\n\
         Parameters:\n\
         - channel_id: The channel plugin id (e.g. \"feishu\", \"wechat\", \"slack\")\n\
         - target_id: The chat/group/user id to send to (platform-specific)\n\
         - message: The text message to send (supports the channel's native formatting)\n\
         - target_type: \"p2p\" for direct message, \"group\" for group chat (default: \"p2p\")\n\
         - attachments: Optional array of file attachments to send. Each item has \"file_path\" (absolute path) and optional \"mime_type\". Use this to send images, files, documents etc.\n\n\
         Use list_channels first if you don't know which channels are available."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "channel_id".to_string(),
            json!({
                "type": "string",
                "description": "Channel plugin id (e.g. \"feishu\", \"wechat\", \"slack\")"
            }),
        );
        props.insert(
            "target_id".to_string(),
            json!({
                "type": "string",
                "description": "The chat/group/user id on the platform to send the message to"
            }),
        );
        props.insert(
            "message".to_string(),
            json!({
                "type": "string",
                "description": "The text message to send"
            }),
        );
        props.insert(
            "target_type".to_string(),
            json!({
                "type": "string",
                "enum": ["p2p", "group"],
                "description": "Message target type: \"p2p\" for direct message, \"group\" for group chat. Default: \"p2p\""
            }),
        );
        props.insert(
            "attachments".to_string(),
            json!({
                "type": "array",
                "description": "Optional file attachments to send (images, documents, etc.)",
                "items": {
                    "type": "object",
                    "properties": {
                        "file_path": {
                            "type": "string",
                            "description": "Absolute path to the file to send"
                        },
                        "mime_type": {
                            "type": "string",
                            "description": "MIME type (e.g. \"image/png\"). Auto-detected from extension if omitted."
                        }
                    },
                    "required": ["file_path"]
                }
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec![
                "channel_id".to_string(),
                "target_id".to_string(),
                "message".to_string(),
            ],
        }
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }

    fn group(&self) -> ToolGroup {
        ToolGroup::Communication
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("invalid JSON: {e}")),
        };

        let channel_id = match args.get("channel_id").and_then(|v| v.as_str()) {
            Some(c) => c.to_string(),
            None => return ToolResult::err("missing required field 'channel_id'".to_string()),
        };

        let target_id = match args.get("target_id").and_then(|v| v.as_str()) {
            Some(t) => t.to_string(),
            None => return ToolResult::err("missing required field 'target_id'".to_string()),
        };

        let message = match args.get("message").and_then(|v| v.as_str()) {
            Some(m) if !m.trim().is_empty() => m.to_string(),
            _ => return ToolResult::err("missing or empty 'message' field".to_string()),
        };

        let target_type = args
            .get("target_type")
            .and_then(|v| v.as_str())
            .unwrap_or("p2p")
            .to_string();

        let registry = self.state.ext.channel_registry.read().await;
        let channel = match registry.get(&channel_id) {
            Some(ch) => ch.clone(),
            None => {
                let available: Vec<_> = registry.list().iter().map(|m| m.id.clone()).collect();
                return ToolResult::err(format!(
                    "channel '{channel_id}' not found or not running. Available: {}",
                    if available.is_empty() {
                        "(none)".to_string()
                    } else {
                        available.join(", ")
                    }
                ));
            }
        };
        drop(registry);

        let attachments: Vec<xiaolin_core::channel::Attachment> = args
            .get("attachments")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| {
                        let file_path = item.get("file_path")?.as_str()?.to_string();
                        let mime_type = item
                            .get("mime_type")
                            .and_then(|v| v.as_str())
                            .map(String::from);
                        let file_name = std::path::Path::new(&file_path)
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string());
                        Some(xiaolin_core::channel::Attachment {
                            file_path,
                            mime_type,
                            file_name,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let outbound = xiaolin_core::channel::OutboundMessage {
            target_id: target_id.clone(),
            target_type: target_type.clone(),
            text: message.clone(),
            reply_to: None,
            image_key: None,
            attachments,
        };

        let attachment_count = outbound.attachments.len();
        match channel.send_message(&outbound).await {
            Ok(_) => {
                tracing::info!(
                    channel = %channel_id,
                    target = %target_id,
                    target_type = %target_type,
                    msg_len = message.len(),
                    attachment_count,
                    "notify_channel: message sent"
                );
                ToolResult::ok(format!(
                    "Message sent to {channel_id} (target: {target_id}, type: {target_type})"
                ))
            }
            Err(e) => {
                tracing::warn!(
                    channel = %channel_id,
                    target = %target_id,
                    error = %e,
                    "notify_channel: send failed"
                );
                ToolResult::err(format!("Failed to send message via {channel_id}: {e}"))
            }
        }
    }
}
