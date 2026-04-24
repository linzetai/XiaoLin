use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use fastclaw_core::config_access::{navigate_config, persist_config_key, set_nested_key};
use fastclaw_core::tool::{Tool, ToolParameterSchema, ToolResult};
use serde_json::json;

use crate::state::AppState;

const SUPPORTED_CHANNELS: &[&str] = &[
    "feishu", "slack", "discord", "telegram", "whatsapp", "matrix", "msteams",
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
            self.state
                .cfg
                .config_live
                .store(Arc::new(live));
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
            }
            Err(e) => {
                status_parts.push(format!(
                    "Plugin start failed: {e}. A restart may be needed."
                ));
            }
        }
        status_parts.push(format!("Webhook URL: /webhook/{channel}"));

        tracing::info!(channel = %channel, "channel added/updated via tool");
        ToolResult::ok(status_parts.join(" "))
    }
}

impl AddChannelTool {
    async fn ensure_binding(&self, channel: &str, agent_id: &str) {
        let live_snapshot = self.state.cfg.config_live.load();
        let bindings = live_snapshot
            .get("bindings")
            .cloned()
            .unwrap_or(json!([]));
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
            self.state
                .cfg
                .config_live
                .store(Arc::new(live));
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
            self.state
                .cfg
                .config_live
                .store(Arc::new(live));
        }

        if let Err(e) = persist_config_key(&key, &disabled) {
            return ToolResult::ok(format!(
                "Channel '{channel}' disabled in memory but failed to persist: {e}"
            ));
        }

        tracing::info!(channel = %channel, "channel removed via tool");
        ToolResult::ok(format!(
            "Channel '{channel}' disabled and saved. It will not reconnect on restart. \
             Note: an already running plugin continues until the process restarts."
        ))
    }
}
