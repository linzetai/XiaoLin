use std::collections::HashMap;
use std::sync::Arc;

use arc_swap::ArcSwap;
use async_trait::async_trait;
use xiaolin_core::agent_config::{McpServerConfig, McpTransportType};
use xiaolin_core::tool::{Tool, ToolExposure, ToolParameterSchema, ToolRegistry, ToolResult};
use xiaolin_core::types::{McpServerStatus, McpStatus};

type ConfigLive = Arc<ArcSwap<serde_json::Value>>;
type McpStatusMap = Arc<ArcSwap<HashMap<String, McpServerStatus>>>;
type McpHandles = Arc<tokio::sync::Mutex<HashMap<String, xiaolin_mcp::SharedMcpClient>>>;

/// Built-in tool allowing the LLM agent to manage MCP servers at runtime:
/// add, remove, list, reload.
pub struct ManageMcpServerTool {
    config_live: ConfigLive,
    mcp_status: McpStatusMap,
    mcp_handles: McpHandles,
    tool_registry: Arc<ToolRegistry>,
}

impl ManageMcpServerTool {
    pub fn new(
        config_live: ConfigLive,
        mcp_status: McpStatusMap,
        mcp_handles: McpHandles,
        tool_registry: Arc<ToolRegistry>,
    ) -> Self {
        Self {
            config_live,
            mcp_status,
            mcp_handles,
            tool_registry,
        }
    }
}

#[async_trait]
impl Tool for ManageMcpServerTool {
    fn name(&self) -> &str {
        "manage_mcp_server"
    }

    fn description(&self) -> &str {
        "Manage MCP (Model Context Protocol) servers at runtime. \
         Actions: \"add\" registers a new MCP server and hot-reloads; \
         \"remove\" unregisters a server; \"list\" shows all servers with connection status; \
         \"reload\" restarts all MCP connections. \
         Use this tool when the user asks to install, connect, add, remove, or check MCP servers. \
         IMPORTANT: If 'add' fails, do NOT give up. Use shell_exec to diagnose the issue: \
         run 'node --version', 'where npx' (Windows) or 'which npx' (Unix), check npm logs, \
         try clearing npm cache with 'npm cache clean --force', or install globally with \
         'npm install -g <package>' then retry with the absolute path. \
         The tool returns detailed error messages including stderr output — read them carefully to determine the fix."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "action".to_string(),
            serde_json::json!({
                "type": "string",
                "enum": ["add", "remove", "list", "reload"],
                "description": "The action to perform."
            }),
        );
        props.insert(
            "id".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Server ID (required for add/remove). Use a short kebab-case identifier, e.g. 'chrome-devtools'."
            }),
        );
        props.insert(
            "command".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Command to launch the MCP server (required for add). e.g. 'npx', 'uvx', 'node'."
            }),
        );
        props.insert(
            "args".to_string(),
            serde_json::json!({
                "type": "array",
                "items": {"type": "string"},
                "description": "Arguments to the command (optional for add). e.g. ['-y', '@anthropic-ai/chrome-devtools-mcp@latest']."
            }),
        );
        props.insert(
            "url".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "SSE URL for HTTP-based MCP servers (use with transport='sse'). e.g. 'http://localhost:3000/sse'."
            }),
        );
        props.insert(
            "transport".to_string(),
            serde_json::json!({
                "type": "string",
                "enum": ["stdio", "sse"],
                "description": "Transport type: 'stdio' (default, uses command+args) or 'sse' (uses url)."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["action".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => {
                return ToolResult::err(format!("invalid JSON arguments: {e}"));
            }
        };

        let action = match args.get("action").and_then(|v| v.as_str()) {
            Some(a) => a,
            None => return ToolResult::err("missing required field 'action'".to_string()),
        };

        match action {
            "list" => self.list_servers().await,
            "reload" => self.reload().await,
            "add" => {
                let id = match args.get("id").and_then(|v| v.as_str()) {
                    Some(s) => s.to_string(),
                    None => return ToolResult::err("'add' requires 'id' field".to_string()),
                };
                if id.contains("__") {
                    return ToolResult::err(
                        "server ID must not contain consecutive double underscores ('__') — it conflicts with the MCP tool naming convention".to_string(),
                    );
                }
                let transport: McpTransportType = args
                    .get("transport")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default();
                let url = args
                    .get("url")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let command = args
                    .get("command")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let cmd_args: Vec<String> = args
                    .get("args")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default();
                let tmp_cfg = McpServerConfig {
                    id: id.clone(),
                    command: command.clone(),
                    args: cmd_args.clone(),
                    enabled: Some(true),
                    env: Default::default(),
                    url: url.clone(),
                    transport,
                    startup_timeout_sec: None,
                    bearer_token_env_var: None,
                    http_headers: None,
                };
                if let Err(e) = tmp_cfg.validate() {
                    return ToolResult::err(e);
                }
                self.add_server(id, command, cmd_args, transport, url).await
            }
            "remove" => {
                let id = match args.get("id").and_then(|v| v.as_str()) {
                    Some(s) => s.to_string(),
                    None => return ToolResult::err("'remove' requires 'id' field".to_string()),
                };
                self.remove_server(id).await
            }
            other => ToolResult::err(format!(
                "unknown action '{other}'; valid actions: add, remove, list, reload"
            )),
        }
    }
}

impl ManageMcpServerTool {
    async fn list_servers(&self) -> ToolResult {
        let status_map: HashMap<String, McpServerStatus> = (**self.mcp_status.load()).clone();

        if status_map.is_empty() {
            return ToolResult::ok("No MCP servers configured.");
        }

        let mut lines = Vec::new();
        for (id, st) in &status_map {
            let status_str = match st.status {
                McpStatus::Connected => "✓ connected",
                McpStatus::Failed => "✗ failed",
                McpStatus::Connecting => "… connecting",
                McpStatus::Disabled => "○ disabled",
                McpStatus::PendingApproval => "⏳ pending approval",
                McpStatus::NeedsAuth => "🔑 needs auth",
            };
            let mut line = format!("- {id}: {status_str}");
            if st.tool_count > 0 {
                line.push_str(&format!(" ({} tools)", st.tool_count));
            }
            if let Some(ref err) = st.error {
                line.push_str(&format!(" — error: {err}"));
            }
            lines.push(line);
        }
        ToolResult::ok(lines.join("\n"))
    }

    async fn do_reload(&self) -> Result<(), String> {
        let desired: Vec<McpServerConfig> = {
            let live = self.config_live.load();
            let mcp_val = live
                .get("mcpServers")
                .cloned()
                .unwrap_or(serde_json::json!([]));
            serde_json::from_value(mcp_val).unwrap_or_default()
        };

        let desired_map: HashMap<String, &McpServerConfig> =
            desired.iter().map(|c| (c.id.clone(), c)).collect();

        let mut handles = self.mcp_handles.lock().await;
        let current_ids: std::collections::HashSet<String> = handles.keys().cloned().collect();
        let desired_ids: std::collections::HashSet<String> = desired_map.keys().cloned().collect();

        let to_remove: Vec<String> = current_ids.difference(&desired_ids).cloned().collect();
        for id in &to_remove {
            let prefix = xiaolin_mcp::naming::mcp_server_prefix(id);
            self.tool_registry.unregister_by_prefix(&prefix);
            handles.remove(id);
        }

        let mut new_status: HashMap<String, McpServerStatus> = HashMap::new();

        for cfg in &desired {
            if cfg.enabled == Some(false) {
                if handles.contains_key(&cfg.id) {
                    let prefix = xiaolin_mcp::naming::mcp_server_prefix(&cfg.id);
                    self.tool_registry.unregister_by_prefix(&prefix);
                    handles.remove(&cfg.id);
                }
                new_status.insert(
                    cfg.id.clone(),
                    McpServerStatus {
                        id: cfg.id.clone(),
                        status: McpStatus::Disabled,
                        error: None,
                        tool_count: 0,
                        connected_at: None,
                        ..Default::default()
                    },
                );
                continue;
            }

            if handles.contains_key(&cfg.id) {
                let prefix = xiaolin_mcp::naming::mcp_server_prefix(&cfg.id);
                self.tool_registry.unregister_by_prefix(&prefix);
                handles.remove(&cfg.id);
            }

            let tc_before = self.tool_registry.len();
            let connect_result =
                xiaolin_mcp::connect_mcp_server(cfg, &self.tool_registry).await;
            match connect_result {
                Ok(handle) => {
                    let tool_count = self.tool_registry.len() - tc_before;
                    let now = chrono::Utc::now().to_rfc3339();
                    new_status.insert(
                        cfg.id.clone(),
                        McpServerStatus {
                            id: cfg.id.clone(),
                            status: McpStatus::Connected,
                            error: None,
                            tool_count,
                            connected_at: Some(now),
                            ..Default::default()
                        },
                    );
                    handles.insert(cfg.id.clone(), handle);
                }
                Err(e) => {
                    new_status.insert(
                        cfg.id.clone(),
                        McpServerStatus {
                            id: cfg.id.clone(),
                            status: McpStatus::Failed,
                            error: Some(e.to_string()),
                            tool_count: 0,
                            connected_at: None,
                            ..Default::default()
                        },
                    );
                }
            }
        }

        for id in &to_remove {
            new_status.remove(id);
        }

        self.mcp_status.store(Arc::new(new_status));
        Ok(())
    }

    async fn reload(&self) -> ToolResult {
        match self.do_reload().await {
            Ok(()) => {
                let status: HashMap<String, McpServerStatus> = (**self.mcp_status.load()).clone();
                let connected = status
                    .values()
                    .filter(|s| s.status == McpStatus::Connected)
                    .count();
                let failed = status
                    .values()
                    .filter(|s| s.status == McpStatus::Failed)
                    .count();
                ToolResult::ok(format!(
                    "MCP servers reloaded: {connected} connected, {failed} failed, {} total.",
                    status.len()
                ))
            }
            Err(e) => ToolResult::err(format!("reload failed: {e}")),
        }
    }

    async fn add_server(
        &self,
        id: String,
        command: String,
        args: Vec<String>,
        transport: McpTransportType,
        url: Option<String>,
    ) -> ToolResult {
        let cmd_name = command.clone();
        let new_server = McpServerConfig {
            id: id.clone(),
            command,
            args,
            enabled: Some(true),
            env: Default::default(),
            url,
            transport,
            startup_timeout_sec: None,
            bearer_token_env_var: None,
            http_headers: None,
        };

        {
            let mut live: serde_json::Value = (**self.config_live.load()).clone();
            let server_val = serde_json::to_value(&new_server).unwrap_or_default();
            if let Some(arr) = live.get_mut("mcpServers").and_then(|v| v.as_array_mut()) {
                arr.retain(|v| v.get("id").and_then(|i| i.as_str()) != Some(&id));
                arr.push(server_val);
            } else {
                live["mcpServers"] = serde_json::json!([server_val]);
            }
            self.config_live.store(Arc::new(live));
        }

        self.persist_mcp_servers();

        match self.do_reload().await {
            Ok(()) => {
                let st = self.mcp_status.load().get(&id).cloned();
                match st.as_ref().map(|s| s.status) {
                    Some(McpStatus::Connected) => {
                        let tc = st.map(|s| s.tool_count).unwrap_or(0);
                        ToolResult::ok(format!(
                            "MCP server '{id}' added and connected successfully ({tc} tools registered)."
                        ))
                    }
                    Some(McpStatus::Failed) => {
                        let err = st.and_then(|s| s.error).unwrap_or_default();
                        let path_hint = std::env::var("PATH").unwrap_or_default();
                        let path_summary: String = path_hint
                            .split(if cfg!(windows) { ';' } else { ':' })
                            .filter(|p| !p.is_empty())
                            .take(10)
                            .collect::<Vec<_>>()
                            .join("; ");
                        ToolResult::err(format!(
                            "MCP server '{id}' was registered but failed to start. Error: {err}\n\
                             Hint: Use shell_exec to diagnose — try running the command directly, check 'node --version' and 'where {cmd_name}' (Windows) or 'which {cmd_name}' (Unix). \
                             First 10 PATH entries: [{path_summary}]",
                        ))
                    }
                    _ => ToolResult::ok(format!("MCP server '{id}' added.")),
                }
            }
            Err(e) => ToolResult::err(format!("added '{id}' but reload failed: {e}")),
        }
    }

    async fn remove_server(&self, id: String) -> ToolResult {
        {
            let mut live: serde_json::Value = (**self.config_live.load()).clone();
            if let Some(arr) = live.get_mut("mcpServers").and_then(|v| v.as_array_mut()) {
                arr.retain(|v| v.get("id").and_then(|i| i.as_str()) != Some(&id));
            }
            self.config_live.store(Arc::new(live));
        }

        self.persist_mcp_servers();

        match self.do_reload().await {
            Ok(()) => ToolResult::ok(format!("MCP server '{id}' removed successfully.")),
            Err(e) => ToolResult::err(format!("removed '{id}' but reload failed: {e}")),
        }
    }

    fn persist_mcp_servers(&self) {
        {
            let live = self.config_live.load();
            let mcp_val = live
                .get("mcpServers")
                .cloned()
                .unwrap_or(serde_json::json!([]));
            let home = match dirs::home_dir() {
                Some(h) => h,
                None => return,
            };
            let cfg_path = home.join(".xiaolin/config/default.json");
            let mut cfg_value: serde_json::Value = if cfg_path.exists() {
                std::fs::read_to_string(&cfg_path)
                    .ok()
                    .and_then(|t| json5::from_str(&t).ok())
                    .unwrap_or(serde_json::json!({}))
            } else {
                serde_json::json!({})
            };
            cfg_value["mcpServers"] = mcp_val;
            if let Some(parent) = cfg_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(text) = serde_json::to_string_pretty(&cfg_value) {
                let _ = std::fs::write(&cfg_path, text);
            }
        }
    }
}

/// Deferred agent tool: list resources from all MCP servers that support resources.
pub struct McpListResourcesTool {
    mcp_handles: McpHandles,
}

impl McpListResourcesTool {
    pub fn new(mcp_handles: McpHandles) -> Self {
        Self { mcp_handles }
    }
}

#[async_trait]
impl Tool for McpListResourcesTool {
    fn name(&self) -> &str {
        "mcp__list_resources"
    }

    fn description(&self) -> &str {
        "List resources from all connected MCP servers that declare resources capability. \
         Returns each resource with its server name, URI, name, description, and MIME type."
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: HashMap::new(),
            required: Vec::new(),
        }
    }

    async fn execute(&self, _arguments: &str) -> ToolResult {
        let resource_clients: Vec<(String, std::sync::Arc<xiaolin_mcp::McpClient>)> = {
            let handles = self.mcp_handles.lock().await;
            handles
                .iter()
                .filter(|(_, c)| c.has_resources())
                .map(|(id, c)| (id.clone(), c.clone()))
                .collect()
        };

        let mut all_resources = Vec::new();
        for (server_id, client) in &resource_clients {
            match client.list_resources().await {
                Ok(resources) => {
                    for r in resources {
                        all_resources.push(serde_json::json!({
                            "server": server_id,
                            "uri": r.uri,
                            "name": r.name,
                            "description": r.description,
                            "mimeType": r.mime_type,
                        }));
                    }
                }
                Err(e) => {
                    tracing::warn!(server = %server_id, error = %e, "failed to list resources");
                }
            }
        }

        ToolResult::ok(serde_json::to_string_pretty(&all_resources).unwrap_or_default())
    }
}

/// Deferred agent tool: read a specific resource from an MCP server.
pub struct McpReadResourceTool {
    mcp_handles: McpHandles,
}

impl McpReadResourceTool {
    pub fn new(mcp_handles: McpHandles) -> Self {
        Self { mcp_handles }
    }
}

#[async_trait]
impl Tool for McpReadResourceTool {
    fn name(&self) -> &str {
        "mcp__read_resource"
    }

    fn description(&self) -> &str {
        "Read a resource from a specific MCP server by URI. \
         The server_name must match one returned by mcp__list_resources. \
         Content larger than 1 MB is automatically truncated."
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "server_name".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "The MCP server name (id) that hosts the resource."
            }),
        );
        props.insert(
            "uri".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "The resource URI to read."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["server_name".to_string(), "uri".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("invalid JSON: {e}")),
        };

        let server_name = match args.get("server_name").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return ToolResult::err("missing required parameter: server_name"),
        };
        let uri = match args.get("uri").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return ToolResult::err("missing required parameter: uri"),
        };

        let handles = self.mcp_handles.lock().await;
        let client = match handles.get(server_name) {
            Some(c) => c.clone(),
            None => return ToolResult::err(format!("server '{server_name}' not found")),
        };
        drop(handles);

        match client.read_resource(uri).await {
            Ok(contents) => {
                let result: Vec<serde_json::Value> = contents
                    .into_iter()
                    .map(|c| {
                        serde_json::json!({
                            "uri": c.uri,
                            "mimeType": c.mime_type,
                            "text": c.text,
                        })
                    })
                    .collect();
                ToolResult::ok(serde_json::to_string_pretty(&result).unwrap_or_default())
            }
            Err(e) => ToolResult::err(format!("failed to read resource: {e}")),
        }
    }
}
