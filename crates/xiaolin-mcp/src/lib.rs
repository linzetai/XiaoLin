use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// MCP protocol types following the Model Context Protocol specification.
///
/// XiaoLin supports MCP as both server (exposing tools to external agents)
/// and client (consuming tools from external MCP servers).
pub mod naming;
pub mod oauth;
pub mod sanitize;

// --- JSON-RPC 2.0 ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: serde_json::Value,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

fn json_rpc_success_serialized(
    id: &serde_json::Value,
    result: impl serde::Serialize,
) -> JsonRpcResponse {
    match serde_json::to_value(result) {
        Ok(v) => JsonRpcResponse::success(id.clone(), v),
        Err(e) => JsonRpcResponse::error(
            id.clone(),
            -32603,
            format!("failed to serialize result: {e}"),
        ),
    }
}

impl JsonRpcResponse {
    pub fn success(id: serde_json::Value, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: serde_json::Value, code: i64, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }
}

// --- MCP Messages ---

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[derive(Default)]
pub struct ServerCapabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<ToolCapability>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourceCapability>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompts: Option<PromptCapability>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCapability {
    #[serde(default)]
    pub list_changed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceCapability {
    #[serde(default)]
    pub subscribe: bool,
    #[serde(default)]
    pub list_changed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptCapability {
    #[serde(default)]
    pub list_changed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    pub protocol_version: String,
    pub capabilities: ServerCapabilities,
    pub server_info: ServerInfo,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpTool {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<serde_json::Value>,
    /// Server-defined metadata (MCP `_meta` field).
    /// Used to carry hints like `{"alwaysLoad": true}` that influence
    /// whether a tool stays eager when others are deferred.
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "_meta")]
    pub meta: Option<serde_json::Value>,
}

impl McpTool {
    /// Whether this tool declares `_meta.alwaysLoad: true`, indicating
    /// it should remain in the eager set even when the server's tools are
    /// bulk-deferred due to token budget.
    pub fn always_load(&self) -> bool {
        self.meta
            .as_ref()
            .and_then(|m| m.get("alwaysLoad"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolListResult {
    pub tools: Vec<McpTool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallToolParams {
    pub name: String,
    #[serde(default)]
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallToolResult {
    pub content: Vec<ToolContent>,
    #[serde(default)]
    pub is_error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ToolContent {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image { data: String, mime_type: String },
}

// --- MCP Server ---

/// An MCP resource descriptor.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpResource {
    pub uri: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

/// An MCP resource template descriptor (client-side).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpResourceTemplate {
    pub uri_template: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

/// Content item returned by `resources/read`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpResourceContent {
    pub uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blob: Option<String>,
}

/// An MCP prompt descriptor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpPrompt {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub arguments: Vec<McpPromptArgument>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpPromptArgument {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub required: bool,
}

/// A message returned by `prompts/get`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpPromptMessage {
    pub role: String,
    pub content: McpPromptContent,
}

/// Content of a prompt message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum McpPromptContent {
    Text { text: String },
    Image {
        data: String,
        #[serde(alias = "mime_type", rename = "mimeType")]
        mime_type: String,
    },
    Resource { resource: McpResourceContent },
}

type McpResourceFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<serde_json::Value>> + Send>>;
type McpPromptFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<serde_json::Value>> + Send>>;

type ToolHandlerFn = Box<dyn Fn(&serde_json::Value) -> McpToolFuture + Send + Sync>;
type ResourceHandlerFn = Box<dyn Fn(&serde_json::Value) -> McpResourceFuture + Send + Sync>;
type PromptHandlerFn = Box<dyn Fn(&serde_json::Value) -> McpPromptFuture + Send + Sync>;

/// An MCP server that exposes XiaoLin's tools, resources, and prompts over JSON-RPC 2.0.
pub struct McpServer {
    server_info: ServerInfo,
    pub tools: Vec<McpTool>,
    tool_handlers: HashMap<String, ToolHandlerFn>,
    pub resources: Vec<McpResource>,
    resource_handlers: HashMap<String, ResourceHandlerFn>,
    pub prompts: Vec<McpPrompt>,
    prompt_handlers: HashMap<String, PromptHandlerFn>,
}

type McpToolFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<CallToolResult>> + Send>>;

impl McpServer {
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            server_info: ServerInfo {
                name: name.into(),
                version: version.into(),
            },
            tools: Vec::new(),
            tool_handlers: HashMap::new(),
            resources: Vec::new(),
            resource_handlers: HashMap::new(),
            prompts: Vec::new(),
            prompt_handlers: HashMap::new(),
        }
    }

    /// Register a tool with its handler.
    pub fn register_tool<F, Fut>(&mut self, tool: McpTool, handler: F)
    where
        F: Fn(serde_json::Value) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = anyhow::Result<CallToolResult>> + Send + 'static,
    {
        let name = tool.name.clone();
        self.tools.push(tool);
        self.tool_handlers.insert(
            name,
            Box::new(move |args| {
                let args = args.clone();
                Box::pin(handler(args))
            }),
        );
    }

    /// Register a resource with its read handler.
    pub fn register_resource<F, Fut>(&mut self, resource: McpResource, handler: F)
    where
        F: Fn(serde_json::Value) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = anyhow::Result<serde_json::Value>> + Send + 'static,
    {
        let uri = resource.uri.clone();
        self.resources.push(resource);
        self.resource_handlers.insert(
            uri,
            Box::new(move |args| {
                let args = args.clone();
                Box::pin(handler(args))
            }),
        );
    }

    /// Register a prompt with its get handler.
    pub fn register_prompt<F, Fut>(&mut self, prompt: McpPrompt, handler: F)
    where
        F: Fn(serde_json::Value) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = anyhow::Result<serde_json::Value>> + Send + 'static,
    {
        let name = prompt.name.clone();
        self.prompts.push(prompt);
        self.prompt_handlers.insert(
            name,
            Box::new(move |args| {
                let args = args.clone();
                Box::pin(handler(args))
            }),
        );
    }

    /// Handle an incoming JSON-RPC request.
    pub async fn handle_request(&self, request: &JsonRpcRequest) -> JsonRpcResponse {
        match request.method.as_str() {
            "initialize" => self.handle_initialize(&request.id),
            "tools/list" => self.handle_tools_list(&request.id),
            "tools/call" => {
                self.handle_tools_call(&request.id, request.params.as_ref())
                    .await
            }
            "resources/list" => self.handle_resources_list(&request.id),
            "resources/read" => {
                self.handle_resources_read(&request.id, request.params.as_ref())
                    .await
            }
            "prompts/list" => self.handle_prompts_list(&request.id),
            "prompts/get" => {
                self.handle_prompts_get(&request.id, request.params.as_ref())
                    .await
            }
            "ping" => JsonRpcResponse::success(request.id.clone(), serde_json::json!({})),
            other => JsonRpcResponse::error(
                request.id.clone(),
                -32601,
                format!("method not found: {other}"),
            ),
        }
    }

    fn handle_initialize(&self, id: &serde_json::Value) -> JsonRpcResponse {
        let result = InitializeResult {
            protocol_version: "2025-06-18".into(),
            capabilities: ServerCapabilities {
                tools: Some(ToolCapability {
                    list_changed: false,
                }),
                resources: if self.resources.is_empty() {
                    None
                } else {
                    Some(ResourceCapability {
                        subscribe: false,
                        list_changed: false,
                    })
                },
                prompts: if self.prompts.is_empty() {
                    None
                } else {
                    Some(PromptCapability {
                        list_changed: false,
                    })
                },
            },
            server_info: self.server_info.clone(),
            instructions: None,
        };
        json_rpc_success_serialized(id, result)
    }

    fn handle_tools_list(&self, id: &serde_json::Value) -> JsonRpcResponse {
        let result = ToolListResult {
            tools: self.tools.clone(),
        };
        json_rpc_success_serialized(id, result)
    }

    async fn handle_tools_call(
        &self,
        id: &serde_json::Value,
        params: Option<&serde_json::Value>,
    ) -> JsonRpcResponse {
        let params: CallToolParams = match params {
            Some(p) => match serde_json::from_value(p.clone()) {
                Ok(p) => p,
                Err(e) => {
                    return JsonRpcResponse::error(
                        id.clone(),
                        -32602,
                        format!("invalid params: {e}"),
                    )
                }
            },
            None => return JsonRpcResponse::error(id.clone(), -32602, "missing params"),
        };

        match self.tool_handlers.get(&params.name) {
            Some(handler) => match handler(&params.arguments).await {
                Ok(result) => json_rpc_success_serialized(id, result),
                Err(e) => {
                    let result = CallToolResult {
                        content: vec![ToolContent::Text {
                            text: e.to_string(),
                        }],
                        is_error: true,
                    };
                    json_rpc_success_serialized(id, result)
                }
            },
            None => {
                JsonRpcResponse::error(id.clone(), -32602, format!("unknown tool: {}", params.name))
            }
        }
    }

    fn handle_resources_list(&self, id: &serde_json::Value) -> JsonRpcResponse {
        JsonRpcResponse::success(
            id.clone(),
            serde_json::json!({ "resources": self.resources }),
        )
    }

    async fn handle_resources_read(
        &self,
        id: &serde_json::Value,
        params: Option<&serde_json::Value>,
    ) -> JsonRpcResponse {
        let uri = params
            .and_then(|p| p.get("uri"))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        match self.resource_handlers.get(uri) {
            Some(handler) => match handler(&serde_json::json!({ "uri": uri })).await {
                Ok(content) => JsonRpcResponse::success(id.clone(), content),
                Err(e) => JsonRpcResponse::error(id.clone(), -32603, e.to_string()),
            },
            None => JsonRpcResponse::error(id.clone(), -32602, format!("unknown resource: {uri}")),
        }
    }

    fn handle_prompts_list(&self, id: &serde_json::Value) -> JsonRpcResponse {
        JsonRpcResponse::success(id.clone(), serde_json::json!({ "prompts": self.prompts }))
    }

    async fn handle_prompts_get(
        &self,
        id: &serde_json::Value,
        params: Option<&serde_json::Value>,
    ) -> JsonRpcResponse {
        let name = params
            .and_then(|p| p.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        match self.prompt_handlers.get(name) {
            Some(handler) => {
                let args = params.cloned().unwrap_or(serde_json::json!({}));
                match handler(&args).await {
                    Ok(result) => JsonRpcResponse::success(id.clone(), result),
                    Err(e) => JsonRpcResponse::error(id.clone(), -32603, e.to_string()),
                }
            }
            None => JsonRpcResponse::error(id.clone(), -32602, format!("unknown prompt: {name}")),
        }
    }

    /// Run the MCP server over stdin/stdout (standard MCP stdio transport).
    pub async fn run_stdio(self) -> anyhow::Result<()> {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

        let stdin = tokio::io::stdin();
        let mut stdout = tokio::io::stdout();
        let reader = BufReader::new(stdin);
        let mut lines = reader.lines();

        tracing::info!(name = %self.server_info.name, "MCP server starting (stdio)");

        while let Ok(Some(line)) = lines.next_line().await {
            let line = line.trim().to_string();
            if line.is_empty() {
                continue;
            }

            if let Ok(notification) = serde_json::from_str::<JsonRpcNotification>(&line) {
                if notification.method == "notifications/initialized" {
                    tracing::info!("MCP client initialized");
                    continue;
                }
                continue;
            }

            match serde_json::from_str::<JsonRpcRequest>(&line) {
                Ok(request) => {
                    let response = self.handle_request(&request).await;
                    let response_json = serde_json::to_string(&response)?;
                    stdout.write_all(response_json.as_bytes()).await?;
                    stdout.write_all(b"\n").await?;
                    stdout.flush().await?;
                }
                Err(e) => {
                    let error = JsonRpcResponse::error(
                        serde_json::Value::Null,
                        -32700,
                        format!("parse error: {e}"),
                    );
                    let error_json = serde_json::to_string(&error)?;
                    stdout.write_all(error_json.as_bytes()).await?;
                    stdout.write_all(b"\n").await?;
                    stdout.flush().await?;
                }
            }
        }

        Ok(())
    }
}

// --- MCP Client ---

fn json_rpc_id_key(id: &serde_json::Value) -> String {
    serde_json::to_string(id).unwrap_or_else(|_| id.to_string())
}

fn companion_post_url(sse: &reqwest::Url) -> anyhow::Result<reqwest::Url> {
    let path = sse.path();
    let new_path = if let Some(prefix) = path.strip_suffix("/sse") {
        if prefix.is_empty() {
            "/message".to_string()
        } else {
            format!("{}/message", prefix.trim_end_matches('/'))
        }
    } else {
        anyhow::bail!("SSE url path must end with /sse (e.g. http://host/mcp/sse), got {path}",);
    };
    let mut u = sse.clone();
    u.set_path(&new_path);
    Ok(u)
}

fn extract_sse_data_lines(block: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in block.lines() {
        let line = line.trim_end();
        if let Some(rest) = line.strip_prefix("data:") {
            let s = rest.trim();
            if !s.is_empty() {
                out.push(s.to_string());
            }
        }
    }
    out
}

const SESSION_EXPIRED_ERROR_CODE: i64 = -32001;

#[derive(Debug)]
struct SessionExpired(String);

impl std::fmt::Display for SessionExpired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MCP session expired: {}", self.0)
    }
}

impl std::error::Error for SessionExpired {}

enum McpTransport {
    Stdio {
        process: Arc<std::sync::Mutex<Box<tokio::process::Child>>>,
        stdin: Arc<tokio::sync::Mutex<tokio::process::ChildStdin>>,
        pending:
            Arc<tokio::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<JsonRpcResponse>>>>,
        reader_task: tokio::task::JoinHandle<()>,
    },
    Sse {
        client: reqwest::Client,
        post_url: String,
        pending:
            Arc<tokio::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<JsonRpcResponse>>>>,
        reader_task: tokio::task::JoinHandle<anyhow::Result<()>>,
    },
    StreamableHttp {
        client: reqwest::Client,
        endpoint_url: String,
        session_id: Arc<tokio::sync::Mutex<Option<String>>>,
        pending:
            Arc<tokio::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<JsonRpcResponse>>>>,
        listener_task: Option<tokio::task::JoinHandle<anyhow::Result<()>>>,
    },
    WebSocket {
        ws_write: Arc<tokio::sync::Mutex<
            futures::stream::SplitSink<
                tokio_tungstenite::WebSocketStream<
                    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
                >,
                tokio_tungstenite::tungstenite::Message,
            >,
        >>,
        pending:
            Arc<tokio::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<JsonRpcResponse>>>>,
        reader_task: tokio::task::JoinHandle<()>,
        url: String,
    },
}

/// MCP server → client notification (JSON-RPC message without `id`).
#[derive(Debug, Clone)]
pub struct McpNotification {
    pub method: String,
    pub params: Option<serde_json::Value>,
}

/// MCP server → client request (JSON-RPC message with both `id` and `method`).
/// Unlike notifications, these require a response from the client.
#[derive(Debug, Clone)]
pub struct McpServerRequest {
    pub id: serde_json::Value,
    pub method: String,
    pub params: Option<serde_json::Value>,
}

/// A client that connects to an external MCP server and discovers/invokes its tools.
pub struct McpClient {
    server_name: std::sync::RwLock<String>,
    tools: Vec<McpTool>,
    transport: McpTransport,
    next_id: std::sync::atomic::AtomicU64,
    notification_tx: tokio::sync::broadcast::Sender<McpNotification>,
    server_request_tx: tokio::sync::broadcast::Sender<McpServerRequest>,
    /// Original SSE URL, kept for reconnection. `None` for stdio/streamable-http.
    sse_url: Option<String>,
    /// Server-provided instructions from `InitializeResult.instructions`.
    server_instructions: std::sync::RwLock<Option<String>>,
    /// Server capabilities declared during `initialize`.
    server_capabilities: std::sync::RwLock<ServerCapabilities>,
    /// Serializes session recovery so that concurrent requests only re-initialize once.
    recovery_lock: Arc<tokio::sync::Mutex<()>>,
    /// Extra HTTP headers (Bearer auth, custom headers) carried for reconnects.
    extra_headers: reqwest::header::HeaderMap,
}

impl McpClient {
    /// Spawn an MCP server subprocess and connect via stdio.
    ///
    /// `extra_env` is merged into the inherited environment before spawning.
    /// On Windows, if the direct spawn fails (e.g. `npx` resolves to `npx.cmd`),
    /// automatically retries via `cmd.exe /C` so that `.cmd`/`.bat` wrappers are found.
    pub async fn connect_stdio<S: std::hash::BuildHasher>(
        command: &str,
        args: &[&str],
        extra_env: &std::collections::HashMap<String, String, S>,
    ) -> anyhow::Result<Self> {
        tracing::info!(command, ?args, "spawning MCP server subprocess");

        let spawn_direct = || {
            let mut cmd = tokio::process::Command::new(command);
            cmd.args(args)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
            #[cfg(windows)]
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
            for (k, v) in extra_env {
                cmd.env(k, v);
            }
            cmd.spawn()
        };

        #[cfg(windows)]
        let mut process = match spawn_direct() {
            Ok(p) => p,
            Err(direct_err) if direct_err.kind() == std::io::ErrorKind::NotFound => {
                tracing::warn!(
                    command,
                    %direct_err,
                    "direct spawn failed, retrying via cmd.exe /C"
                );
                let mut shell_args: Vec<&str> = vec!["/C", command];
                shell_args.extend(args);
                let mut cmd = tokio::process::Command::new("cmd.exe");
                cmd.args(&shell_args)
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .creation_flags(0x08000000); // CREATE_NO_WINDOW
                for (k, v) in extra_env {
                    cmd.env(k, v);
                }
                cmd.spawn()
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "failed to spawn '{command}' both directly ({direct_err}) and via cmd.exe ({e})"
                        )
                    })?
            }
            Err(e) => return Err(e.into()),
        };

        #[cfg(not(windows))]
        let mut process = spawn_direct()?;

        let stdin = process
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("failed to open stdin"))?;
        let stdout = process
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("failed to open stdout"))?;
        let reader = tokio::io::BufReader::new(stdout);

        let pending: Arc<
            tokio::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<JsonRpcResponse>>>,
        > = Arc::new(tokio::sync::Mutex::new(HashMap::new()));

        let (notification_tx, _) = tokio::sync::broadcast::channel::<McpNotification>(64);
        let (server_request_tx, _) = tokio::sync::broadcast::channel::<McpServerRequest>(16);

        let pending_reader = pending.clone();
        let ntx = notification_tx.clone();
        let srtx = server_request_tx.clone();
        let reader_task = tokio::spawn(async move {
            Self::stdio_reader_loop(reader, pending_reader, ntx, srtx).await;
        });

        if let Some(stderr) = process.stderr.take() {
            let server_id = command.to_string();
            tokio::spawn(async move {
                Self::stderr_reader_loop(stderr, &server_id).await;
            });
        }

        let mut client = Self {
            server_name: std::sync::RwLock::new(command.to_string()),
            tools: Vec::new(),
            transport: McpTransport::Stdio {
                process: Arc::new(std::sync::Mutex::new(Box::new(process))),
                stdin: Arc::new(tokio::sync::Mutex::new(stdin)),
                pending,
                reader_task,
            },
            next_id: std::sync::atomic::AtomicU64::new(1),
            notification_tx,
            server_request_tx,
            sse_url: None,
            server_instructions: std::sync::RwLock::new(None),
            server_capabilities: std::sync::RwLock::new(ServerCapabilities::default()),
            recovery_lock: Arc::new(tokio::sync::Mutex::new(())),
            extra_headers: reqwest::header::HeaderMap::new(),
        };

        client.initialize().await?;
        client.discover_tools().await?;

        Ok(client)
    }

    /// Connect via HTTP SSE (GET stream) and JSON-RPC POST to the companion `/message` path.
    ///
    /// `url` must be the full SSE URL whose path ends with `/sse`; POST target is the same path
    /// with the `/sse` suffix replaced by `/message`.
    pub async fn connect_sse(
        url: &str,
        extra_headers: reqwest::header::HeaderMap,
    ) -> anyhow::Result<Self> {
        let sse_url = reqwest::Url::parse(url.trim())?;
        let post_url = companion_post_url(&sse_url)?.to_string();
        let sse_url_str = sse_url.to_string();

        let client = reqwest::Client::builder()
            .default_headers(extra_headers.clone())
            .build()?;
        let pending: Arc<
            tokio::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<JsonRpcResponse>>>,
        > = Arc::new(tokio::sync::Mutex::new(HashMap::new()));

        let (notification_tx, _) = tokio::sync::broadcast::channel::<McpNotification>(64);
        let (server_request_tx, _) = tokio::sync::broadcast::channel::<McpServerRequest>(16);

        let pending_reader = pending.clone();
        let client_reader = client.clone();
        let sse_reader_url = sse_url_str.clone();
        let ntx = notification_tx.clone();
        let srtx = server_request_tx.clone();
        let reader_task = tokio::spawn(async move {
            Self::sse_reader_loop(&client_reader, &sse_reader_url, pending_reader, ntx, srtx).await
        });

        let mut mcp = Self {
            server_name: std::sync::RwLock::new(sse_url_str.clone()),
            tools: Vec::new(),
            transport: McpTransport::Sse {
                client: client.clone(),
                post_url,
                pending: pending.clone(),
                reader_task,
            },
            next_id: std::sync::atomic::AtomicU64::new(1),
            notification_tx,
            server_request_tx,
            sse_url: Some(sse_url_str),
            server_instructions: std::sync::RwLock::new(None),
            server_capabilities: std::sync::RwLock::new(ServerCapabilities::default()),
            recovery_lock: Arc::new(tokio::sync::Mutex::new(())),
            extra_headers,
        };

        tokio::task::yield_now().await;
        tokio::task::yield_now().await;

        mcp.initialize().await?;
        mcp.discover_tools().await?;

        Ok(mcp)
    }

    /// Connect to an MCP server via the Streamable HTTP transport (MCP 2025-06-18).
    ///
    /// A single endpoint handles all JSON-RPC messages via POST.  An optional
    /// GET request opens an SSE stream for server-initiated notifications.
    pub async fn connect_streamable_http(
        url: &str,
        extra_headers: reqwest::header::HeaderMap,
    ) -> anyhow::Result<Self> {
        let endpoint_url = url.trim().to_string();
        let client = reqwest::Client::builder()
            .default_headers(extra_headers.clone())
            .build()?;
        let pending: Arc<
            tokio::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<JsonRpcResponse>>>,
        > = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let session_id: Arc<tokio::sync::Mutex<Option<String>>> =
            Arc::new(tokio::sync::Mutex::new(None));
        let (notification_tx, _) = tokio::sync::broadcast::channel::<McpNotification>(64);
        let (server_request_tx, _) = tokio::sync::broadcast::channel::<McpServerRequest>(16);

        let mut mcp = Self {
            server_name: std::sync::RwLock::new(String::new()),
            tools: Vec::new(),
            transport: McpTransport::StreamableHttp {
                client: client.clone(),
                endpoint_url: endpoint_url.clone(),
                session_id: session_id.clone(),
                pending: pending.clone(),
                listener_task: None,
            },
            next_id: std::sync::atomic::AtomicU64::new(1),
            notification_tx: notification_tx.clone(),
            server_request_tx: server_request_tx.clone(),
            sse_url: None,
            server_instructions: std::sync::RwLock::new(None),
            server_capabilities: std::sync::RwLock::new(ServerCapabilities::default()),
            recovery_lock: Arc::new(tokio::sync::Mutex::new(())),
            extra_headers,
        };

        mcp.initialize().await?;

        let sid_for_listener = session_id.lock().await.clone();
        let listener_pending = pending.clone();
        let listener_client = client.clone();
        let listener_url = endpoint_url.clone();
        let ntx = notification_tx.clone();
        let srtx = server_request_tx.clone();
        let task = tokio::spawn(async move {
            Self::streamable_http_listener(
                &listener_client,
                &listener_url,
                sid_for_listener.as_deref(),
                listener_pending,
                ntx,
                srtx,
            )
            .await
        });
        if let McpTransport::StreamableHttp {
            ref mut listener_task,
            ..
        } = mcp.transport
        {
            *listener_task = Some(task);
        }

        mcp.discover_tools().await?;
        Ok(mcp)
    }

    /// Connect to an MCP server over WebSocket (`ws://` or `wss://`).
    pub async fn connect_websocket(url: &str) -> anyhow::Result<Self> {
        use futures::StreamExt;
        use tokio_tungstenite::tungstenite;

        let request = tungstenite::http::Request::builder()
            .uri(url)
            .header("Sec-WebSocket-Protocol", "mcp")
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket")
            .header("Sec-WebSocket-Version", "13")
            .header("Sec-WebSocket-Key", tungstenite::handshake::client::generate_key())
            .header("Host", reqwest::Url::parse(url)
                .map_or_else(|_| "localhost".to_string(), |u| u.host_str().unwrap_or("localhost").to_string()))
            .body(())?;

        let (ws_stream, _) = tokio_tungstenite::connect_async(request).await
            .map_err(|e| anyhow::anyhow!("WebSocket connection failed: {e}"))?;

        let (write, read) = ws_stream.split();
        let ws_write = Arc::new(tokio::sync::Mutex::new(write));

        let pending: Arc<
            tokio::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<JsonRpcResponse>>>,
        > = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let (notification_tx, _) = tokio::sync::broadcast::channel::<McpNotification>(64);
        let (server_request_tx, _) = tokio::sync::broadcast::channel::<McpServerRequest>(16);

        let pending_reader = pending.clone();
        let ntx = notification_tx.clone();
        let srtx = server_request_tx.clone();
        let reader_task = tokio::spawn(async move {
            Self::websocket_reader_loop(read, pending_reader, ntx, srtx).await;
        });

        let mut mcp = Self {
            server_name: std::sync::RwLock::new(url.to_string()),
            tools: Vec::new(),
            transport: McpTransport::WebSocket {
                ws_write,
                pending: pending.clone(),
                reader_task,
                url: url.to_string(),
            },
            next_id: std::sync::atomic::AtomicU64::new(1),
            notification_tx,
            server_request_tx,
            sse_url: None,
            server_instructions: std::sync::RwLock::new(None),
            server_capabilities: std::sync::RwLock::new(ServerCapabilities::default()),
            recovery_lock: Arc::new(tokio::sync::Mutex::new(())),
            extra_headers: reqwest::header::HeaderMap::new(),
        };

        mcp.initialize().await?;
        mcp.discover_tools().await?;
        Ok(mcp)
    }

    async fn websocket_reader_loop(
        mut read: futures::stream::SplitStream<
            tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
        >,
        pending: Arc<
            tokio::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<JsonRpcResponse>>>,
        >,
        notification_tx: tokio::sync::broadcast::Sender<McpNotification>,
        server_request_tx: tokio::sync::broadcast::Sender<McpServerRequest>,
    ) {
        use futures::StreamExt;
        while let Some(msg_result) = read.next().await {
            match msg_result {
                Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                    match serde_json::from_str::<serde_json::Value>(&text) {
                        Ok(value) => {
                            Self::dispatch_incoming(
                                value,
                                &pending,
                                &notification_tx,
                                &server_request_tx,
                                "websocket",
                            )
                            .await;
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "invalid JSON from WebSocket MCP server");
                        }
                    }
                }
                Ok(tokio_tungstenite::tungstenite::Message::Close(_)) => {
                    tracing::info!("WebSocket MCP connection closed by server");
                    let _ = notification_tx.send(McpNotification {
                        method: "xiaolin/transport_disconnected".into(),
                        params: None,
                    });
                    break;
                }
                Err(e) => {
                    tracing::warn!(error = %e, "WebSocket read error");
                    let _ = notification_tx.send(McpNotification {
                        method: "xiaolin/transport_disconnected".into(),
                        params: None,
                    });
                    break;
                }
                _ => {}
            }
        }
        tracing::debug!("WebSocket reader loop ended");
    }

    async fn streamable_http_listener(
        client: &reqwest::Client,
        endpoint_url: &str,
        session_id: Option<&str>,
        pending: Arc<
            tokio::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<JsonRpcResponse>>>,
        >,
        notification_tx: tokio::sync::broadcast::Sender<McpNotification>,
        server_request_tx: tokio::sync::broadcast::Sender<McpServerRequest>,
    ) -> anyhow::Result<()> {
        let mut req = client
            .get(endpoint_url)
            .header("Accept", "text/event-stream");
        if let Some(sid) = session_id {
            req = req.header("Mcp-Session-Id", sid);
        }
        let response = match req.send().await {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                tracing::debug!(
                    status = %r.status(),
                    "Streamable HTTP GET for notifications not supported by server"
                );
                return Ok(());
            }
            Err(e) => {
                tracing::debug!(error = %e, "Streamable HTTP GET failed (server may not support notifications)");
                return Ok(());
            }
        };

        let mut byte_buf: Vec<u8> = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            byte_buf.extend_from_slice(&chunk);
            if std::str::from_utf8(&byte_buf).is_err() {
                continue;
            }
            while let Ok(text) = std::str::from_utf8(&byte_buf) {
                let Some(pos) = text.find("\n\n") else {
                    break;
                };
                let event_block = text[..pos].to_string();
                byte_buf.drain(..pos + 2);
                for data in extract_sse_data_lines(&event_block) {
                    let value: serde_json::Value = match serde_json::from_str(&data) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };

                    Self::dispatch_incoming(
                        value,
                        &pending,
                        &notification_tx,
                        &server_request_tx,
                        "streamable_http",
                    )
                    .await;
                }
            }
        }
        Ok(())
    }

    async fn stderr_reader_loop(mut stderr: tokio::process::ChildStderr, server_id: &str) {
        use tokio::io::AsyncBufReadExt;
        let mut reader = tokio::io::BufReader::new(&mut stderr);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) => break,
                Ok(_) => {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        tracing::warn!(mcp_server = %server_id, "[mcp:stderr] {}", trimmed);
                    }
                }
                Err(_) => break,
            }
        }
    }

    async fn stdio_reader_loop(
        mut reader: tokio::io::BufReader<tokio::process::ChildStdout>,
        pending: Arc<
            tokio::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<JsonRpcResponse>>>,
        >,
        notification_tx: tokio::sync::broadcast::Sender<McpNotification>,
        server_request_tx: tokio::sync::broadcast::Sender<McpServerRequest>,
    ) {
        use tokio::io::AsyncBufReadExt;

        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) => break,
                Ok(_) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    let value: serde_json::Value = match serde_json::from_str(trimmed) {
                        Ok(v) => v,
                        Err(e) => {
                            tracing::warn!("MCP stdio: unparseable JSON: {e}");
                            continue;
                        }
                    };

                    Self::dispatch_incoming(
                        value,
                        &pending,
                        &notification_tx,
                        &server_request_tx,
                        "stdio",
                    )
                    .await;
                }
                Err(e) => {
                    tracing::warn!("MCP stdio reader error: {e}");
                    break;
                }
            }
        }

        let mut guard = pending.lock().await;
        for (_, tx) in guard.drain() {
            let _ = tx.send(JsonRpcResponse::error(
                serde_json::Value::Null,
                -32603,
                "MCP subprocess exited",
            ));
        }
    }

    async fn sse_reader_loop(
        client: &reqwest::Client,
        sse_url: &str,
        pending: Arc<
            tokio::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<JsonRpcResponse>>>,
        >,
        notification_tx: tokio::sync::broadcast::Sender<McpNotification>,
        server_request_tx: tokio::sync::broadcast::Sender<McpServerRequest>,
    ) -> anyhow::Result<()> {
        let result =
            Self::sse_reader_loop_inner(client, sse_url, pending, &notification_tx, &server_request_tx)
                .await;

        match &result {
            Ok(()) => tracing::warn!("SSE stream ended normally, signaling disconnection"),
            Err(e) => tracing::warn!(error = %e, "SSE stream error, signaling disconnection"),
        }
        let _ = notification_tx.send(McpNotification {
            method: "xiaolin/transport_disconnected".into(),
            params: None,
        });

        result
    }

    async fn sse_reader_loop_inner(
        client: &reqwest::Client,
        sse_url: &str,
        pending: Arc<
            tokio::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<JsonRpcResponse>>>,
        >,
        notification_tx: &tokio::sync::broadcast::Sender<McpNotification>,
        server_request_tx: &tokio::sync::broadcast::Sender<McpServerRequest>,
    ) -> anyhow::Result<()> {
        let response = client
            .get(sse_url)
            .header("Accept", "text/event-stream")
            .send()
            .await?
            .error_for_status()?;

        let mut byte_buf: Vec<u8> = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            byte_buf.extend_from_slice(&chunk);
            if std::str::from_utf8(&byte_buf).is_err() {
                continue;
            }
            while let Ok(text) = std::str::from_utf8(&byte_buf) {
                let Some(pos) = text.find("\n\n") else {
                    break;
                };
                let event_block = text[..pos].to_string();
                byte_buf.drain(..pos + 2);
                for data in extract_sse_data_lines(&event_block) {
                    let value: serde_json::Value = match serde_json::from_str(&data) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };

                    Self::dispatch_incoming(
                        value,
                        &pending,
                        notification_tx,
                        server_request_tx,
                        "sse",
                    )
                    .await;
                }
            }
        }

        Ok(())
    }

    /// Classify an incoming JSON-RPC message and route it to the appropriate
    /// channel: pending response map, notification broadcast, or server request
    /// broadcast. A message with both `id` and `method` is a server-initiated
    /// request; one with only `id` is a response; one with only `method` is a
    /// notification.
    async fn dispatch_incoming(
        value: serde_json::Value,
        pending: &Arc<
            tokio::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<JsonRpcResponse>>>,
        >,
        notification_tx: &tokio::sync::broadcast::Sender<McpNotification>,
        server_request_tx: &tokio::sync::broadcast::Sender<McpServerRequest>,
        transport_label: &str,
    ) {
        let has_id = value.get("id").is_some();
        let method_str = value.get("method").and_then(|m| m.as_str()).map(String::from);

        match (has_id, method_str) {
            (true, Some(method)) => {
                let id = value.get("id").cloned().unwrap_or(serde_json::Value::Null);
                let params = value.get("params").cloned();
                tracing::info!(method = %method, %transport_label, "MCP server request received");
                let _ = server_request_tx.send(McpServerRequest { id, method, params });
            }
            (true, None) => {
                if let Ok(resp) = serde_json::from_value::<JsonRpcResponse>(value) {
                    let key = json_rpc_id_key(&resp.id);
                    if let Some(tx) = pending.lock().await.remove(&key) {
                        let _ = tx.send(resp);
                    }
                }
            }
            (false, Some(method)) => {
                let params = value.get("params").cloned();
                tracing::debug!(method = %method, %transport_label, "MCP notification received");
                let _ = notification_tx.send(McpNotification {
                    method,
                    params,
                });
            }
            (false, None) => {
                tracing::debug!(%transport_label, "MCP: ignoring JSON message without id or method");
            }
        }
    }

    /// Send a JSON-RPC response back to the server (for server-initiated
    /// requests like `elicitation/create`).
    pub async fn send_response(&self, response: JsonRpcResponse) -> anyhow::Result<()> {
        use tokio::io::AsyncWriteExt;

        let json = serde_json::to_string(&response)?;

        match &self.transport {
            McpTransport::Stdio { stdin, .. } => {
                let mut stdin_guard = stdin.lock().await;
                stdin_guard.write_all(json.as_bytes()).await?;
                stdin_guard.write_all(b"\n").await?;
                stdin_guard.flush().await?;
            }
            McpTransport::Sse { client, post_url, .. } => {
                let resp = client
                    .post(post_url.as_str())
                    .header("Content-Type", "application/json")
                    .body(json)
                    .send()
                    .await?;
                if !resp.status().is_success() {
                    let body = resp.text().await.unwrap_or_default();
                    tracing::warn!("MCP send_response POST failed: {body}");
                }
            }
            McpTransport::StreamableHttp {
                client,
                endpoint_url,
                session_id,
                ..
            } => {
                let mut req = client
                    .post(endpoint_url.as_str())
                    .header("Content-Type", "application/json")
                    .header("Accept", "application/json")
                    .header("MCP-Protocol-Version", "2025-06-18");
                if let Some(sid) = session_id.lock().await.as_ref() {
                    req = req.header("Mcp-Session-Id", sid);
                }
                let resp = req.body(json).send().await?;
                if !resp.status().is_success() {
                    let body = resp.text().await.unwrap_or_default();
                    tracing::warn!("MCP send_response StreamableHttp POST failed: {body}");
                }
            }
            McpTransport::WebSocket { ws_write, .. } => {
                use futures::SinkExt;
                let mut writer = ws_write.lock().await;
                writer
                    .send(tokio_tungstenite::tungstenite::Message::Text(json))
                    .await?;
            }
        }

        Ok(())
    }

    async fn send_request(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> anyhow::Result<JsonRpcResponse> {
        use std::sync::atomic::Ordering;
        use tokio::io::AsyncWriteExt;

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let id_val = serde_json::Value::Number(id.into());
        let request = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: id_val.clone(),
            method: method.into(),
            params,
        };

        let id_key = json_rpc_id_key(&id_val);
        let (tx, rx) = tokio::sync::oneshot::channel();

        match &self.transport {
            McpTransport::Stdio { stdin, pending, .. } => {
                {
                    let mut g = pending.lock().await;
                    g.insert(id_key.clone(), tx);
                }

                let json = serde_json::to_string(&request)?;
                {
                    let mut stdin_guard = stdin.lock().await;
                    stdin_guard.write_all(json.as_bytes()).await?;
                    stdin_guard.write_all(b"\n").await?;
                    stdin_guard.flush().await?;
                }

                Self::await_pending_response(pending, &id_key, rx, "stdio").await
            }
            McpTransport::Sse {
                client,
                post_url,
                pending,
                ..
            } => {
                {
                    let mut g = pending.lock().await;
                    g.insert(id_key.clone(), tx);
                }

                let json = serde_json::to_string(&request)?;
                let post = client
                    .post(post_url.as_str())
                    .header("Content-Type", "application/json")
                    .body(json)
                    .send()
                    .await?;

                let status = post.status();
                if !status.is_success() && status != reqwest::StatusCode::ACCEPTED {
                    let _ = pending.lock().await.remove(&id_key);
                    let body = post.text().await.unwrap_or_default();
                    anyhow::bail!("MCP POST {status}: {body}");
                }

                Self::await_pending_response(pending, &id_key, rx, "SSE").await
            }
            McpTransport::StreamableHttp {
                client,
                endpoint_url,
                session_id,
                pending,
                ..
            } => {
                let json = serde_json::to_string(&request)?;

                // Retry transport errors with exponential backoff (max 3 attempts).
                let response = {
                    const MAX_RETRIES: u32 = 3;
                    let mut last_err = None;
                    let mut resp_out = None;
                    for attempt in 0..MAX_RETRIES {
                        let mut req = client
                            .post(endpoint_url.as_str())
                            .header("Content-Type", "application/json")
                            .header("Accept", "application/json, text/event-stream")
                            .header("MCP-Protocol-Version", "2025-06-18");
                        if let Some(sid) = session_id.lock().await.as_ref() {
                            req = req.header("Mcp-Session-Id", sid);
                        }

                        match req.body(json.clone()).send().await {
                            Ok(r) => {
                                resp_out = Some(r);
                                break;
                            }
                            Err(e) if e.is_connect() || e.is_timeout() => {
                                let delay_ms = std::cmp::min(500u64 * (1u64 << attempt), 4_000);
                                tracing::warn!(
                                    attempt = attempt + 1,
                                    error = %e,
                                    "StreamableHttp transport error, retrying in {}ms",
                                    delay_ms
                                );
                                last_err = Some(e);
                                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                            }
                            Err(e) => return Err(e.into()),
                        }
                    }
                    match resp_out {
                        Some(r) => r,
                        None => return Err(last_err.unwrap().into()),
                    }
                };

                let status = response.status();
                if status == reqwest::StatusCode::NOT_FOUND {
                    return Err(
                        SessionExpired(format!("HTTP 404 from {endpoint_url}")).into(),
                    );
                }
                if !status.is_success() && status != reqwest::StatusCode::ACCEPTED {
                    anyhow::bail!(
                        "Streamable HTTP POST {status}: {}",
                        response.text().await.unwrap_or_default()
                    );
                }

                // Extract session ID from response header
                if let Some(sid) = response.headers().get("Mcp-Session-Id") {
                    if let Ok(s) = sid.to_str() {
                        *session_id.lock().await = Some(s.to_string());
                    }
                }

                let content_type = response
                    .headers()
                    .get("content-type")
                    .and_then(|h| h.to_str().ok())
                    .unwrap_or("")
                    .to_string();

                if content_type.contains("text/event-stream") {
                    // Server sends response as SSE stream — register pending and spawn reader
                    {
                        let mut g = pending.lock().await;
                        g.insert(id_key.clone(), tx);
                    }
                    let pending_clone = pending.clone();
                    let body_text = response.text().await?;
                    tokio::spawn(async move {
                        for data_line in extract_sse_data_lines(&body_text) {
                            if let Ok(resp) = serde_json::from_str::<JsonRpcResponse>(&data_line) {
                                let resp_id = resp.id.to_string();
                                let mut guard = pending_clone.lock().await;
                                if let Some(sender) = guard.remove(&resp_id) {
                                    let _ = sender.send(resp);
                                }
                            }
                        }
                    });
                    Self::await_pending_response(pending, &id_key, rx, "StreamableHTTP").await
                } else {
                    // Direct JSON response
                    drop(tx);
                    let body = response.text().await?;
                    let resp: JsonRpcResponse = serde_json::from_str(&body)?;
                    Ok(resp)
                }
            }
            McpTransport::WebSocket {
                ws_write, pending, ..
            } => {
                {
                    let mut g = pending.lock().await;
                    g.insert(id_key.clone(), tx);
                }

                let json = serde_json::to_string(&request)?;
                {
                    use futures::SinkExt;
                    let mut writer = ws_write.lock().await;
                    writer
                        .send(tokio_tungstenite::tungstenite::Message::Text(json))
                        .await?;
                }

                Self::await_pending_response(pending, &id_key, rx, "WebSocket").await
            }
        }
    }

    async fn await_pending_response(
        pending: &Arc<
            tokio::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<JsonRpcResponse>>>,
        >,
        id_key: &str,
        rx: tokio::sync::oneshot::Receiver<JsonRpcResponse>,
        transport_label: &str,
    ) -> anyhow::Result<JsonRpcResponse> {
        match tokio::time::timeout(Duration::from_secs(30), rx).await {
            Ok(Ok(resp)) => Ok(resp),
            Ok(Err(_)) => {
                let _ = pending.lock().await.remove(id_key);
                anyhow::bail!("MCP {transport_label} reply channel closed")
            }
            Err(_) => {
                let _ = pending.lock().await.remove(id_key);
                anyhow::bail!("MCP {transport_label} response timed out")
            }
        }
    }

    fn is_streamable_http(&self) -> bool {
        matches!(self.transport, McpTransport::StreamableHttp { .. })
    }

    fn is_session_expired_response(resp: &JsonRpcResponse) -> bool {
        resp.error
            .as_ref()
            .is_some_and(|e| e.code == SESSION_EXPIRED_ERROR_CODE)
    }

    /// Re-initialize a Streamable HTTP session after expiry.
    ///
    /// Acquires `recovery_lock` so concurrent callers coalesce into a single recovery.
    /// If another request already recovered (session_id changed), this is a no-op.
    async fn recover_streamable_http_session(&self) -> anyhow::Result<()> {
        let McpTransport::StreamableHttp {
            client: http_client,
            endpoint_url,
            session_id,
            ..
        } = &self.transport
        else {
            return Ok(());
        };

        let old_session = session_id.lock().await.clone();

        let _guard = self.recovery_lock.lock().await;

        let current_session = session_id.lock().await.clone();
        if current_session != old_session {
            tracing::info!(
                server = %self.server_name.read().unwrap(),
                "session already recovered by another request, skipping"
            );
            return Ok(());
        }

        tracing::warn!(
            server = %self.server_name.read().unwrap(),
            "recovering Streamable HTTP session"
        );

        *session_id.lock().await = None;

        let init_params = serde_json::json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {
                "elicitation": {},
                "roots": { "listChanged": true }
            },
            "clientInfo": {
                "name": "XiaoLin",
                "version": env!("CARGO_PKG_VERSION"),
            }
        });
        let id = self
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let request = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: serde_json::Value::Number(id.into()),
            method: "initialize".into(),
            params: Some(init_params),
        };
        let json = serde_json::to_string(&request)?;

        let response = http_client
            .post(endpoint_url.as_str())
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", "2025-06-18")
            .body(json)
            .send()
            .await?;

        if !response.status().is_success() {
            anyhow::bail!(
                "session recovery initialize failed: HTTP {}",
                response.status()
            );
        }

        if let Some(sid) = response.headers().get("Mcp-Session-Id") {
            if let Ok(s) = sid.to_str() {
                *session_id.lock().await = Some(s.to_string());
            }
        }

        let body = response.text().await?;
        if let Ok(resp) = serde_json::from_str::<JsonRpcResponse>(&body) {
            if let Some(result) = resp.result {
                if let Ok(info) = serde_json::from_value::<InitializeResult>(result) {
                    *self.server_name.write().unwrap() = info.server_info.name;
                    *self.server_instructions.write().unwrap() = info.instructions;
                }
            }
        }

        let notification = JsonRpcNotification {
            jsonrpc: "2.0".into(),
            method: "notifications/initialized".into(),
            params: None,
        };
        let notif_json = serde_json::to_string(&notification)?;
        let mut req = http_client
            .post(endpoint_url.as_str())
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream");
        if let Some(sid) = session_id.lock().await.as_ref() {
            req = req.header("Mcp-Session-Id", sid);
        }
        let _ = req.body(notif_json).send().await?;

        tracing::info!(server = %self.server_name.read().unwrap(), "Streamable HTTP session recovered");
        Ok(())
    }

    /// Like `send_request`, but transparently recovers from session expiry
    /// (HTTP 404 or JSON-RPC -32001) on the Streamable HTTP transport.
    /// Retries the original request at most once after a successful recovery.
    async fn send_request_with_session_recovery(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> anyhow::Result<JsonRpcResponse> {
        match self.send_request(method, params.clone()).await {
            Ok(resp) if self.is_streamable_http() && Self::is_session_expired_response(&resp) => {
                tracing::warn!(method, "JSON-RPC -32001 session expired, recovering");
                self.recover_streamable_http_session().await?;
                self.send_request(method, params).await
            }
            Err(e) if self.is_streamable_http() && e.downcast_ref::<SessionExpired>().is_some() => {
                tracing::warn!(method, error = %e, "HTTP 404 session expired, recovering");
                self.recover_streamable_http_session().await?;
                self.send_request(method, params).await
            }
            other => other,
        }
    }

    async fn initialize(&mut self) -> anyhow::Result<()> {
        let params = serde_json::json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {
                "elicitation": {},
                "roots": { "listChanged": true }
            },
            "clientInfo": {
                "name": "XiaoLin",
                "version": env!("CARGO_PKG_VERSION"),
            }
        });

        let response = self.send_request("initialize", Some(params)).await?;
        if let Some(error) = response.error {
            anyhow::bail!("MCP initialize failed: {}", error.message);
        }

        if let Some(result) = response.result {
            let info: InitializeResult = serde_json::from_value(result)?;
            *self.server_name.write().unwrap() = info.server_info.name;
            let has_instructions = info.instructions.is_some();
            let has_resources = info.capabilities.resources.is_some();
            let has_prompts = info.capabilities.prompts.is_some();
            *self.server_capabilities.write().unwrap() = info.capabilities;
            *self.server_instructions.write().unwrap() = info.instructions;
            tracing::info!(
                server = %self.server_name.read().unwrap(),
                version = %info.server_info.version,
                has_instructions,
                has_resources,
                has_prompts,
                "MCP server connected"
            );
        }

        let notification = JsonRpcNotification {
            jsonrpc: "2.0".into(),
            method: "notifications/initialized".into(),
            params: None,
        };
        match &self.transport {
            McpTransport::Stdio { stdin, .. } => {
                use tokio::io::AsyncWriteExt;
                let json = serde_json::to_string(&notification)?;
                let mut stdin_guard = stdin.lock().await;
                stdin_guard.write_all(json.as_bytes()).await?;
                stdin_guard.write_all(b"\n").await?;
                stdin_guard.flush().await?;
            }
            McpTransport::Sse {
                client, post_url, ..
            } => {
                let json = serde_json::to_string(&notification)?;
                let _ = client
                    .post(post_url.as_str())
                    .header("Content-Type", "application/json")
                    .body(json)
                    .send()
                    .await?;
            }
            McpTransport::StreamableHttp {
                client,
                endpoint_url,
                session_id,
                ..
            } => {
                let json = serde_json::to_string(&notification)?;
                let mut req = client
                    .post(endpoint_url.as_str())
                    .header("Content-Type", "application/json")
                    .header("Accept", "application/json, text/event-stream");
                if let Some(sid) = session_id.lock().await.as_ref() {
                    req = req.header("Mcp-Session-Id", sid);
                }
                let _ = req.body(json).send().await?;
            }
            McpTransport::WebSocket { ws_write, .. } => {
                use futures::SinkExt;
                let json = serde_json::to_string(&notification)?;
                let mut writer = ws_write.lock().await;
                writer
                    .send(tokio_tungstenite::tungstenite::Message::Text(json))
                    .await?;
            }
        }

        Ok(())
    }

    async fn discover_tools(&mut self) -> anyhow::Result<()> {
        let response = self
            .send_request_with_session_recovery("tools/list", None)
            .await?;
        if let Some(result) = response.result {
            let mut tool_list: ToolListResult = serde_json::from_value(result)?;
            for tool in &mut tool_list.tools {
                tool.name = sanitize::sanitize_unicode(&tool.name);
                if let Some(desc) = &mut tool.description {
                    *desc = sanitize::sanitize_unicode(desc);
                }
                if let Some(schema) = &mut tool.input_schema {
                    sanitize::sanitize_json_schema_descriptions(schema);
                }
            }
            tracing::info!(count = tool_list.tools.len(), server = %self.server_name.read().unwrap(), "discovered MCP tools");
            self.tools = tool_list.tools;
        }
        Ok(())
    }

    /// Get the list of discovered tools.
    pub fn tools(&self) -> &[McpTool] {
        &self.tools
    }

    /// Call a tool on the remote MCP server.
    ///
    /// If `progress_token` is provided, it is included as `_meta.progressToken`
    /// in the request so the server can send `notifications/progress` updates.
    pub async fn call_tool(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> anyhow::Result<CallToolResult> {
        self.call_tool_with_progress(name, arguments, None).await
    }

    /// Call a tool with an optional progress token for tracking long-running operations.
    pub async fn call_tool_with_progress(
        &self,
        name: &str,
        arguments: serde_json::Value,
        progress_token: Option<&str>,
    ) -> anyhow::Result<CallToolResult> {
        let mut params = serde_json::json!({
            "name": name,
            "arguments": arguments,
        });
        if let Some(token) = progress_token {
            params["_meta"] = serde_json::json!({ "progressToken": token });
        }

        let response = self
            .send_request_with_session_recovery("tools/call", Some(params))
            .await?;
        if let Some(error) = response.error {
            anyhow::bail!("tool call failed: {}", error.message);
        }

        match response.result {
            Some(result) => Ok(serde_json::from_value(result)?),
            None => anyhow::bail!("empty result from tool call"),
        }
    }

    /// Get the server name.
    pub fn server_name(&self) -> String {
        self.server_name.read().unwrap().clone()
    }

    /// Get the server-provided instructions (from `InitializeResult.instructions`).
    pub fn instructions(&self) -> Option<String> {
        self.server_instructions.read().unwrap().clone()
    }

    /// Subscribe to server-initiated notifications (JSON-RPC messages without `id`).
    ///
    /// Returns a broadcast receiver. Callers that fall behind by more than 64
    /// buffered messages will see `RecvError::Lagged`.
    pub fn subscribe_notifications(
        &self,
    ) -> tokio::sync::broadcast::Receiver<McpNotification> {
        self.notification_tx.subscribe()
    }

    /// Subscribe to server-initiated requests (JSON-RPC messages with both `id`
    /// and `method`, e.g. `elicitation/create`). The consumer is responsible for
    /// sending a response via [`send_response`](Self::send_response).
    pub fn subscribe_server_requests(
        &self,
    ) -> tokio::sync::broadcast::Receiver<McpServerRequest> {
        self.server_request_tx.subscribe()
    }

    /// Returns the original SSE URL if this client was created via `connect_sse`.
    pub fn sse_url(&self) -> Option<&str> {
        self.sse_url.as_deref()
    }

    /// Returns a clone of the extra HTTP headers used for this connection.
    pub fn extra_headers(&self) -> reqwest::header::HeaderMap {
        self.extra_headers.clone()
    }

    /// Force re-fetch the tool list from the server via `tools/list`.
    ///
    /// Useful after receiving a `notifications/tools/list_changed` notification.
    pub async fn refresh_tools(&mut self) -> anyhow::Result<&[McpTool]> {
        self.discover_tools().await?;
        Ok(&self.tools)
    }

    /// Fetch the current tool list from the server without modifying internal state.
    ///
    /// Returns the freshly fetched tools. Useful when only a shared (`Arc`) reference
    /// is available and `refresh_tools(&mut self)` cannot be called.
    pub async fn fetch_tools(&self) -> anyhow::Result<Vec<McpTool>> {
        let response = self
            .send_request_with_session_recovery("tools/list", None)
            .await?;
        if let Some(error) = response.error {
            anyhow::bail!("tools/list failed: {}", error.message);
        }
        let result = response
            .result
            .ok_or_else(|| anyhow::anyhow!("empty result from tools/list"))?;

        let mut parsed: ToolListResult = serde_json::from_value(result)?;
        for tool in &mut parsed.tools {
            tool.name = sanitize::sanitize_unicode(&tool.name);
            if let Some(desc) = &mut tool.description {
                *desc = sanitize::sanitize_unicode(desc);
            }
            if let Some(schema) = &mut tool.input_schema {
                sanitize::sanitize_json_schema_descriptions(schema);
            }
        }
        Ok(parsed.tools)
    }

    /// Returns the server capabilities declared during `initialize`.
    pub fn server_capabilities(&self) -> ServerCapabilities {
        self.server_capabilities.read().unwrap().clone()
    }

    /// Whether the server declared `capabilities.resources`.
    pub fn has_resources(&self) -> bool {
        self.server_capabilities.read().unwrap().resources.is_some()
    }

    /// Whether the server declared `capabilities.prompts`.
    pub fn has_prompts(&self) -> bool {
        self.server_capabilities.read().unwrap().prompts.is_some()
    }

    /// List resources exposed by the server (`resources/list`).
    ///
    /// Returns an empty list if the server didn't declare `capabilities.resources`.
    pub async fn list_resources(&self) -> anyhow::Result<Vec<McpResource>> {
        if !self.has_resources() {
            return Ok(Vec::new());
        }
        let response = self
            .send_request_with_session_recovery("resources/list", None)
            .await?;
        if let Some(error) = response.error {
            anyhow::bail!("resources/list failed: {}", error.message);
        }
        let result = response
            .result
            .ok_or_else(|| anyhow::anyhow!("empty result from resources/list"))?;

        #[derive(Deserialize)]
        struct ResourceListResult {
            #[serde(default)]
            resources: Vec<McpResource>,
        }
        let mut parsed: ResourceListResult = serde_json::from_value(result)?;
        for r in &mut parsed.resources {
            r.name = sanitize::sanitize_unicode(&r.name);
            r.uri = sanitize::sanitize_unicode(&r.uri);
            if let Some(desc) = &mut r.description {
                *desc = sanitize::sanitize_unicode(desc);
            }
        }
        Ok(parsed.resources)
    }

    /// Read a resource by URI (`resources/read`).
    ///
    /// Content exceeding 1 MB is truncated with a `[truncated]` suffix.
    pub async fn read_resource(&self, uri: &str) -> anyhow::Result<Vec<McpResourceContent>> {
        if !self.has_resources() {
            anyhow::bail!("server does not support resources");
        }
        let params = serde_json::json!({ "uri": uri });
        let response = self
            .send_request_with_session_recovery("resources/read", Some(params))
            .await?;
        if let Some(error) = response.error {
            anyhow::bail!("resources/read failed: {}", error.message);
        }
        let result = response
            .result
            .ok_or_else(|| anyhow::anyhow!("empty result from resources/read"))?;

        #[derive(Deserialize)]
        struct ReadResult {
            #[serde(default)]
            contents: Vec<McpResourceContent>,
        }
        let mut parsed: ReadResult = serde_json::from_value(result)?;

        const MAX_CONTENT_BYTES: usize = 1_048_576; // 1 MB
        for c in &mut parsed.contents {
            if let Some(text) = &mut c.text {
                if text.len() > MAX_CONTENT_BYTES {
                    let boundary = text.floor_char_boundary(MAX_CONTENT_BYTES);
                    text.truncate(boundary);
                    text.push_str("\n[truncated]");
                }
            }
        }
        Ok(parsed.contents)
    }

    /// List resource templates from the server (`resources/templates/list`).
    ///
    /// Returns an empty list if the server didn't declare `capabilities.resources`.
    pub async fn list_resource_templates(&self) -> anyhow::Result<Vec<McpResourceTemplate>> {
        if !self.has_resources() {
            return Ok(Vec::new());
        }
        let response = self
            .send_request_with_session_recovery("resources/templates/list", None)
            .await?;
        if let Some(error) = response.error {
            anyhow::bail!("resources/templates/list failed: {}", error.message);
        }
        let result = response
            .result
            .ok_or_else(|| anyhow::anyhow!("empty result from resources/templates/list"))?;

        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct TemplateListResult {
            #[serde(default)]
            resource_templates: Vec<McpResourceTemplate>,
        }
        let mut parsed: TemplateListResult = serde_json::from_value(result)?;
        for t in &mut parsed.resource_templates {
            t.name = sanitize::sanitize_unicode(&t.name);
            t.uri_template = sanitize::sanitize_unicode(&t.uri_template);
            if let Some(desc) = &mut t.description {
                *desc = sanitize::sanitize_unicode(desc);
            }
        }
        Ok(parsed.resource_templates)
    }

    /// List prompts from the server (`prompts/list`).
    ///
    /// Returns an empty list if the server didn't declare `capabilities.prompts`.
    pub async fn list_prompts(&self) -> anyhow::Result<Vec<McpPrompt>> {
        if !self.has_prompts() {
            return Ok(Vec::new());
        }
        let response = self
            .send_request_with_session_recovery("prompts/list", None)
            .await?;
        if let Some(error) = response.error {
            anyhow::bail!("prompts/list failed: {}", error.message);
        }
        let result = response
            .result
            .ok_or_else(|| anyhow::anyhow!("empty result from prompts/list"))?;

        #[derive(Deserialize)]
        struct PromptListResult {
            #[serde(default)]
            prompts: Vec<McpPrompt>,
        }
        let mut parsed: PromptListResult = serde_json::from_value(result)?;
        for p in &mut parsed.prompts {
            p.name = sanitize::sanitize_unicode(&p.name);
            if let Some(desc) = &mut p.description {
                *desc = sanitize::sanitize_unicode(desc);
            }
            for arg in &mut p.arguments {
                arg.name = sanitize::sanitize_unicode(&arg.name);
                if let Some(desc) = &mut arg.description {
                    *desc = sanitize::sanitize_unicode(desc);
                }
            }
        }
        Ok(parsed.prompts)
    }

    /// Get a rendered prompt from the server (`prompts/get`).
    ///
    /// Returns the prompt messages with the given arguments applied.
    pub async fn get_prompt(
        &self,
        name: &str,
        arguments: Option<std::collections::HashMap<String, String>>,
    ) -> anyhow::Result<Vec<McpPromptMessage>> {
        if !self.has_prompts() {
            anyhow::bail!("server does not support prompts");
        }
        let mut params = serde_json::json!({ "name": name });
        if let Some(args) = arguments {
            params["arguments"] = serde_json::to_value(args)?;
        }
        let response = self
            .send_request_with_session_recovery("prompts/get", Some(params))
            .await?;
        if let Some(error) = response.error {
            anyhow::bail!("prompts/get failed: {}", error.message);
        }
        let result = response
            .result
            .ok_or_else(|| anyhow::anyhow!("empty result from prompts/get"))?;

        #[derive(Deserialize)]
        struct GetPromptResult {
            #[serde(default)]
            messages: Vec<McpPromptMessage>,
        }
        let parsed: GetPromptResult = serde_json::from_value(result)?;
        Ok(parsed.messages)
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        match &mut self.transport {
            McpTransport::Stdio {
                process,
                reader_task,
                ..
            } => {
                reader_task.abort();
                if let Ok(mut proc) = process.lock() {
                    let _ = proc.start_kill();
                }
            }
            McpTransport::Sse { reader_task, .. } => {
                reader_task.abort();
            }
            McpTransport::StreamableHttp {
                listener_task, ..
            } => {
                if let Some(task) = listener_task.take() {
                    task.abort();
                }
            }
            McpTransport::WebSocket { reader_task, url, .. } => {
                tracing::debug!(url = %url, "dropping WebSocket MCP client");
                reader_task.abort();
            }
        }
    }
}

// --- Bridge: Expose XiaoLin tools as MCP ---

/// Create an MCP server pre-populated with XiaoLin's built-in tools.
///
/// Tools with the `mcp__` prefix are excluded to avoid circular delegation
/// (remote MCP tools re-exported through the reverse server).
pub fn create_xiaolin_mcp_server(
    tool_registry: &Arc<xiaolin_core::tool::ToolRegistry>,
) -> McpServer {
    let mut server = McpServer::new("XiaoLin", env!("CARGO_PKG_VERSION"));

    for def in tool_registry.definitions().iter() {
        let tool_name = def.function.name.clone();
        if naming::is_mcp_tool(&tool_name) {
            continue;
        }
        let mcp_tool = McpTool {
            name: tool_name.clone(),
            description: Some(def.function.description.clone()),
            input_schema: Some(serde_json::to_value(&def.function.parameters).unwrap_or_default()),
            meta: None,
        };

        let registry = tool_registry.clone();
        server.register_tool(mcp_tool, move |args: serde_json::Value| {
            let registry = registry.clone();
            let name = tool_name.clone();
            async move {
                let tool = registry
                    .get(&name)
                    .ok_or_else(|| anyhow::anyhow!("tool not found: {name}"))?;
                let args_str = serde_json::to_string(&args)?;
                let result = tool.execute(&args_str).await;
                Ok(CallToolResult {
                    content: vec![ToolContent::Text {
                        text: result.output,
                    }],
                    is_error: !result.success,
                })
            }
        });
    }

    server
}

// --- MCP Tool Bridge: expose remote MCP tools as XiaoLin Tools ---

/// Shared handle to an MCP client; safe for concurrent tool calls.
pub type SharedMcpClient = Arc<McpClient>;

/// Max characters for an MCP tool description to prevent prompt bloat.
const MCP_TOOL_DESC_MAX_CHARS: usize = 2048;

/// A XiaoLin `Tool` that delegates execution to a remote MCP server via `McpClient`.
pub struct McpToolBridge {
    tool_name: String,
    description: String,
    schema: serde_json::Value,
    client: SharedMcpClient,
    server_prefix: String,
    hint: String,
    keep_eager: bool,
}

impl McpToolBridge {
    fn new(mcp_tool: &McpTool, client: SharedMcpClient, server_prefix: &str) -> Self {
        let raw_desc = mcp_tool.description.clone().unwrap_or_default();
        let desc: String = if raw_desc.len() > MCP_TOOL_DESC_MAX_CHARS {
            let truncated: String = raw_desc.chars().take(MCP_TOOL_DESC_MAX_CHARS).collect();
            tracing::warn!(
                tool = %mcp_tool.name,
                original_len = raw_desc.len(),
                "MCP tool description truncated to {MCP_TOOL_DESC_MAX_CHARS} chars"
            );
            truncated
        } else {
            raw_desc.clone()
        };
        Self {
            tool_name: format!("{server_prefix}{}", naming::sanitize_for_api(&mcp_tool.name)),
            hint: format!("{} {}", mcp_tool.name, raw_desc),
            keep_eager: mcp_tool.always_load(),
            description: desc,
            schema: mcp_tool.input_schema.clone().unwrap_or(serde_json::json!({
                "type": "object",
                "properties": {},
            })),
            client,
            server_prefix: server_prefix.to_string(),
        }
    }
}

#[async_trait::async_trait]
impl xiaolin_core::tool::Tool for McpToolBridge {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn search_hint(&self) -> &str {
        &self.hint
    }

    fn exposure(&self) -> xiaolin_core::tool::ToolExposure {
        if self.keep_eager {
            xiaolin_core::tool::ToolExposure::Direct
        } else {
            xiaolin_core::tool::ToolExposure::Deferred
        }
    }

    fn force_eager(&self) -> bool {
        self.keep_eager
    }

    fn parameters_schema(&self) -> xiaolin_core::tool::ToolParameterSchema {
        let properties = self
            .schema
            .get("properties")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect();
        let required = self
            .schema
            .get("required")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        xiaolin_core::tool::ToolParameterSchema {
            schema_type: "object".into(),
            properties,
            required,
        }
    }

    async fn execute(&self, arguments: &str) -> xiaolin_core::tool::ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => {
                return xiaolin_core::tool::ToolResult::err(format!("invalid JSON arguments: {e}"))
            }
        };

        let original_name = self
            .tool_name
            .strip_prefix(&self.server_prefix)
            .unwrap_or(&self.tool_name);

        let call_result = self.client.call_tool(original_name, args).await;

        match call_result {
            Ok(result) => {
                let text = result
                    .content
                    .iter()
                    .map(|c| match c {
                        ToolContent::Text { text } => text.clone(),
                        ToolContent::Image { data, mime_type } => {
                            format!("![image](data:{mime_type};base64,{data})")
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");

                if result.is_error {
                    xiaolin_core::tool::ToolResult::err(text)
                } else {
                    xiaolin_core::tool::ToolResult::ok(text)
                }
            }
            Err(e) => xiaolin_core::tool::ToolResult::err(format!("MCP call failed: {e}")),
        }
    }
}

/// Error returned when an MCP server requires OAuth authentication.
/// The gateway should set the server status to `NeedsAuth` when it sees this.
#[derive(Debug)]
pub struct NeedsOAuth {
    pub server_id: String,
    pub message: String,
}

impl std::fmt::Display for NeedsOAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MCP server '{}' needs OAuth: {}", self.server_id, self.message)
    }
}

impl std::error::Error for NeedsOAuth {}

/// Unified entry point: connect to an MCP server and register its tools.
///
/// Dispatches to the correct transport (stdio/SSE/streamable HTTP) based on
/// `cfg.transport`. The tool-name prefix is derived internally from `cfg.id`.
///
/// The entire connection is wrapped in a timeout derived from
/// `cfg.startup_timeout_sec` (default 30 s).
///
/// On HTTP 401, attempts to use a stored OAuth token (refresh if expired).
/// If no valid token is available, returns a `NeedsOAuth` error so the gateway
/// can set the server status to `NeedsAuth`.
static NEEDS_AUTH_CACHE: std::sync::LazyLock<
    std::sync::Mutex<HashMap<String, std::time::Instant>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

const NEEDS_AUTH_TTL: Duration = Duration::from_secs(15 * 60);

/// Clear the NeedsAuth TTL cache for a specific server (e.g. after a user-initiated OAuth login).
pub fn clear_needs_auth_cache(server_id: &str) {
    if let Ok(mut cache) = NEEDS_AUTH_CACHE.lock() {
        cache.remove(server_id);
    }
}

pub async fn connect_mcp_server(
    cfg: &xiaolin_core::agent_config::McpServerConfig,
    registry: &xiaolin_core::tool::ToolRegistry,
) -> anyhow::Result<SharedMcpClient> {
    cfg.validate().map_err(|e| anyhow::anyhow!(e))?;

    if let Ok(cache) = NEEDS_AUTH_CACHE.lock() {
        if let Some(&ts) = cache.get(&cfg.id) {
            if ts.elapsed() < NEEDS_AUTH_TTL {
                return Err(NeedsOAuth {
                    server_id: cfg.id.clone(),
                    message: "server requires OAuth (cached, retry after TTL)".into(),
                }
                .into());
            }
        }
    }
    let timeout_secs = cfg.startup_timeout_sec.unwrap_or(30);
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs as u64),
        connect_mcp_server_inner(cfg, registry),
    )
    .await;

    let client = match result {
        Ok(Ok(c)) => c,
        Ok(Err(e)) => {
            let is_http = matches!(
                cfg.transport.effective(),
                xiaolin_core::agent_config::McpTransportType::Sse
                    | xiaolin_core::agent_config::McpTransportType::StreamableHttp
            );
            let is_401 = e.to_string().contains("401")
                || e.downcast_ref::<reqwest::Error>()
                    .and_then(|re| re.status())
                    .is_some_and(|s| s == reqwest::StatusCode::UNAUTHORIZED);

            if is_http && is_401 {
                return try_oauth_recovery(cfg, registry).await;
            }
            return Err(e);
        }
        Err(_) => anyhow::bail!(
            "MCP server '{}' failed to connect within {}s (transport: {:?})",
            cfg.id,
            timeout_secs,
            cfg.transport.effective()
        ),
    };

    registry.set_mcp_instructions(&cfg.id, client.instructions().as_deref());
    Ok(client)
}

/// Attempt OAuth token recovery: stored token refresh, or signal NeedsAuth.
async fn try_oauth_recovery(
    cfg: &xiaolin_core::agent_config::McpServerConfig,
    registry: &xiaolin_core::tool::ToolRegistry,
) -> anyhow::Result<SharedMcpClient> {
    let url = cfg.url.as_deref().unwrap_or("");
    tracing::info!(server = %cfg.id, "HTTP 401 — attempting OAuth token recovery");

    if let Some(stored) = oauth::load_stored_token(&cfg.id) {
        if let Some(ref refresh) = stored.refresh_token {
            let mut oauth_client = oauth::McpOAuthClient::new(url);
            if oauth_client.discover_metadata().await.is_ok() {
                match oauth_client.refresh_token(refresh).await {
                    Ok(new_token) => {
                        let now_secs = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs();
                        let stored_token = oauth::StoredToken {
                            access_token: new_token.access_token.clone(),
                            refresh_token: new_token
                                .refresh_token
                                .or(Some(refresh.clone())),
                            expires_at: new_token.expires_in.map(|e| now_secs + e),
                            server_url: url.to_string(),
                        };
                        let _ = oauth::save_stored_token(&cfg.id, &stored_token);

                        let mut headers = resolve_mcp_http_headers(cfg)?;
                        let auth_val = reqwest::header::HeaderValue::from_str(&format!(
                            "Bearer {}",
                            new_token.access_token
                        ))
                        .map_err(|e| anyhow::anyhow!("invalid token: {e}"))?;
                        headers.insert(reqwest::header::AUTHORIZATION, auth_val);

                        let prefix = naming::mcp_server_prefix(&cfg.id);
                        let client = match cfg.transport.effective() {
                            xiaolin_core::agent_config::McpTransportType::Sse => {
                                register_mcp_tools_sse(url, registry, &prefix, headers).await?
                            }
                            _ => {
                                register_mcp_tools_streamable_http(url, registry, &prefix, headers)
                                    .await?
                            }
                        };
                        registry.set_mcp_instructions(
                            &cfg.id,
                            client.instructions().as_deref(),
                        );
                        tracing::info!(server = %cfg.id, "OAuth token refreshed, reconnected");
                        return Ok(client);
                    }
                    Err(e) => {
                        tracing::warn!(server = %cfg.id, error = %e, "token refresh failed");
                        oauth::remove_stored_token(&cfg.id);
                    }
                }
            }
        }
    }

    if let Ok(mut cache) = NEEDS_AUTH_CACHE.lock() {
        cache.insert(cfg.id.clone(), std::time::Instant::now());
    }

    Err(NeedsOAuth {
        server_id: cfg.id.clone(),
        message: "server returned 401, no valid OAuth token available".into(),
    }
    .into())
}

/// Build a `reqwest::header::HeaderMap` from the MCP server config's
/// `bearer_token_env_var` and `http_headers` fields.
///
/// - `bearer_token_env_var`: reads the named env var and adds `Authorization: Bearer <value>`.
///   Returns an error if the env var is missing.
/// - `http_headers`: values starting with `$` are treated as env var references
///   (`$API_KEY` → reads env var `API_KEY`). Missing env vars are skipped with a warning.
pub fn resolve_mcp_http_headers(
    cfg: &xiaolin_core::agent_config::McpServerConfig,
) -> anyhow::Result<reqwest::header::HeaderMap> {
    let mut headers = reqwest::header::HeaderMap::new();

    if let Some(ref var_name) = cfg.bearer_token_env_var {
        match std::env::var(var_name) {
            Ok(token) if !token.is_empty() => {
                let val = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))
                    .map_err(|e| anyhow::anyhow!("invalid bearer token value: {e}"))?;
                headers.insert(reqwest::header::AUTHORIZATION, val);
                tracing::debug!(server = %cfg.id, "injected Authorization header from env var");
            }
            Ok(_) => {
                anyhow::bail!(
                    "MCP server '{}': env var '{}' is empty (expected a Bearer token)",
                    cfg.id,
                    var_name
                );
            }
            Err(_) => {
                anyhow::bail!(
                    "MCP server '{}': env var '{}' not found (required for bearer_token_env_var)",
                    cfg.id,
                    var_name
                );
            }
        }
    }

    if let Some(ref extra) = cfg.http_headers {
        for (key, raw_value) in extra {
            let resolved = if let Some(var_name) = raw_value.strip_prefix('$') {
                match std::env::var(var_name) {
                    Ok(v) => v,
                    Err(_) => {
                        tracing::warn!(
                            server = %cfg.id,
                            header = %key,
                            env_var = %var_name,
                            "skipping HTTP header: env var not found"
                        );
                        continue;
                    }
                }
            } else {
                raw_value.clone()
            };

            match (
                reqwest::header::HeaderName::from_bytes(key.as_bytes()),
                reqwest::header::HeaderValue::from_str(&resolved),
            ) {
                (Ok(name), Ok(value)) => {
                    headers.insert(name, value);
                }
                _ => {
                    tracing::warn!(
                        server = %cfg.id,
                        header = %key,
                        "skipping HTTP header: invalid header name or value"
                    );
                }
            }
        }
    }

    Ok(headers)
}

async fn connect_mcp_server_inner(
    cfg: &xiaolin_core::agent_config::McpServerConfig,
    registry: &xiaolin_core::tool::ToolRegistry,
) -> anyhow::Result<SharedMcpClient> {
    use xiaolin_core::agent_config::McpTransportType;

    cfg.validate()
        .map_err(|e| anyhow::anyhow!("invalid MCP config: {e}"))?;

    let prefix = naming::mcp_server_prefix(&cfg.id);
    match cfg.transport.effective() {
        McpTransportType::Stdio => {
            let args_ref: Vec<&str> = cfg.args.iter().map(|s| s.as_str()).collect();
            register_mcp_tools(&cfg.command, &args_ref, registry, &prefix, &cfg.env).await
        }
        McpTransportType::Sse => {
            let url = cfg.url.as_deref().unwrap_or("");
            let headers = resolve_mcp_http_headers(cfg)?;
            register_mcp_tools_sse(url, registry, &prefix, headers).await
        }
        McpTransportType::StreamableHttp => {
            let url = cfg.url.as_deref().unwrap_or("");
            let headers = resolve_mcp_http_headers(cfg)?;
            register_mcp_tools_streamable_http(url, registry, &prefix, headers).await
        }
        McpTransportType::WebSocket => {
            let url = cfg.url.as_deref().unwrap_or("");
            register_mcp_tools_websocket(url, registry, &prefix).await
        }
        McpTransportType::Http => unreachable!("effective() normalizes Http → StreamableHttp"),
    }
}

/// Re-register MCP tools from a fresh tool list into the registry.
///
/// First removes all existing tools with the given prefix, then registers
/// the new tools. Returns the number of tools registered.
pub fn re_register_tools(
    tools: &[McpTool],
    client: &SharedMcpClient,
    registry: &xiaolin_core::tool::ToolRegistry,
    server_prefix: &str,
) -> usize {
    registry.unregister_by_prefix(server_prefix);
    if let Some(server_id) = naming::parse_server_id_from_prefix(server_prefix) {
        registry.set_mcp_instructions(server_id, client.instructions().as_deref());
    }
    let mut registered = 0usize;
    let mut seen = std::collections::HashSet::new();
    for tool in tools {
        let sanitized = format!("{server_prefix}{}", naming::sanitize_for_api(&tool.name));
        if !seen.insert(sanitized.clone()) {
            tracing::warn!(tool = %sanitized, "skipping duplicate MCP tool within same server (post-sanitize collision)");
            continue;
        }
        let bridge = McpToolBridge::new(tool, client.clone(), server_prefix);
        registry.register(Arc::new(bridge));
        registered += 1;
    }
    registered
}

/// Connect to an MCP server and register all its tools into a XiaoLin ToolRegistry.
/// Returns the shared McpClient handle so it can be managed/closed later.
///
/// Tools are registered with a `server_prefix` to avoid name collisions (e.g. `"mcp__myserver__"`).
pub async fn register_mcp_tools<S: std::hash::BuildHasher>(
    command: &str,
    args: &[&str],
    registry: &xiaolin_core::tool::ToolRegistry,
    server_prefix: &str,
    extra_env: &std::collections::HashMap<String, String, S>,
) -> anyhow::Result<SharedMcpClient> {
    let client = McpClient::connect_stdio(command, args, extra_env).await?;
    let tools = client.tools().to_vec();
    let shared = Arc::new(client);

    let mut registered = 0usize;
    let mut seen = std::collections::HashSet::new();
    for tool in &tools {
        let sanitized = format!("{server_prefix}{}", naming::sanitize_for_api(&tool.name));
        if !seen.insert(sanitized.clone()) {
            tracing::warn!(tool = %sanitized, "skipping duplicate MCP tool within same server (post-sanitize collision)");
            continue;
        }
        let bridge = McpToolBridge::new(tool, shared.clone(), server_prefix);
        registry.register(Arc::new(bridge));
        registered += 1;
    }
    let count = registered;

    tracing::info!(
        count,
        prefix = server_prefix,
        "registered MCP tools into XiaoLin"
    );
    Ok(shared)
}

/// Register tools from an MCP server reachable via HTTP SSE.
///
/// `url` is the SSE endpoint (e.g. `http://host:port/sse`).
pub async fn register_mcp_tools_sse(
    url: &str,
    registry: &xiaolin_core::tool::ToolRegistry,
    server_prefix: &str,
    extra_headers: reqwest::header::HeaderMap,
) -> anyhow::Result<SharedMcpClient> {
    let client = McpClient::connect_sse(url, extra_headers).await?;
    let tools = client.tools().to_vec();
    let shared = Arc::new(client);

    let mut registered = 0usize;
    let mut seen = std::collections::HashSet::new();
    for tool in &tools {
        let sanitized = format!("{server_prefix}{}", naming::sanitize_for_api(&tool.name));
        if !seen.insert(sanitized.clone()) {
            tracing::warn!(tool = %sanitized, "skipping duplicate MCP tool within same server (post-sanitize collision)");
            continue;
        }
        let bridge = McpToolBridge::new(tool, shared.clone(), server_prefix);
        registry.register(Arc::new(bridge));
        registered += 1;
    }
    let count = registered;

    tracing::info!(
        count,
        prefix = server_prefix,
        url,
        "registered MCP tools (SSE) into XiaoLin"
    );
    Ok(shared)
}

/// Register MCP tools from a Streamable HTTP MCP server.
pub async fn register_mcp_tools_streamable_http(
    url: &str,
    registry: &xiaolin_core::tool::ToolRegistry,
    server_prefix: &str,
    extra_headers: reqwest::header::HeaderMap,
) -> anyhow::Result<SharedMcpClient> {
    let client = McpClient::connect_streamable_http(url, extra_headers).await?;
    let tools = client.tools().to_vec();
    let shared = Arc::new(client);

    let mut registered = 0usize;
    let mut seen = std::collections::HashSet::new();
    for tool in &tools {
        let sanitized = format!("{server_prefix}{}", naming::sanitize_for_api(&tool.name));
        if !seen.insert(sanitized.clone()) {
            tracing::warn!(tool = %sanitized, "skipping duplicate MCP tool within same server (post-sanitize collision)");
            continue;
        }
        let bridge = McpToolBridge::new(tool, shared.clone(), server_prefix);
        registry.register(Arc::new(bridge));
        registered += 1;
    }

    tracing::info!(
        count = registered,
        prefix = server_prefix,
        url,
        "registered MCP tools (Streamable HTTP) into XiaoLin"
    );
    Ok(shared)
}

/// Register MCP tools from a WebSocket MCP server.
pub async fn register_mcp_tools_websocket(
    url: &str,
    registry: &xiaolin_core::tool::ToolRegistry,
    server_prefix: &str,
) -> anyhow::Result<SharedMcpClient> {
    let client = McpClient::connect_websocket(url).await?;
    let tools = client.tools().to_vec();
    let shared = Arc::new(client);

    let mut registered = 0usize;
    let mut seen = std::collections::HashSet::new();
    for tool in &tools {
        let sanitized = format!("{server_prefix}{}", naming::sanitize_for_api(&tool.name));
        if !seen.insert(sanitized.clone()) {
            tracing::warn!(tool = %sanitized, "skipping duplicate MCP tool within same server (post-sanitize collision)");
            continue;
        }
        let bridge = McpToolBridge::new(tool, shared.clone(), server_prefix);
        registry.register(Arc::new(bridge));
        registered += 1;
    }

    tracing::info!(
        count = registered,
        prefix = server_prefix,
        url,
        "registered MCP tools (WebSocket) into XiaoLin"
    );
    Ok(shared)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_rpc_response_success() {
        let resp = JsonRpcResponse::success(serde_json::json!(1), serde_json::json!({"ok": true}));
        assert!(resp.error.is_none());
        assert_eq!(resp.result.unwrap()["ok"], true);
    }

    #[test]
    fn json_rpc_response_error() {
        let resp = JsonRpcResponse::error(serde_json::json!(1), -32601, "not found");
        assert!(resp.result.is_none());
        assert_eq!(resp.error.unwrap().code, -32601);
    }

    #[test]
    fn resolve_headers_empty_config() {
        let cfg = xiaolin_core::agent_config::McpServerConfig {
            id: "test".into(),
            command: String::new(),
            args: vec![],
            enabled: Some(true),
            env: Default::default(),
            url: Some("http://localhost/mcp".into()),
            transport: xiaolin_core::agent_config::McpTransportType::StreamableHttp,
            startup_timeout_sec: None,
            bearer_token_env_var: None,
            http_headers: None,
        };
        let headers = resolve_mcp_http_headers(&cfg).unwrap();
        assert!(headers.is_empty());
    }

    #[test]
    fn resolve_headers_bearer_from_env() {
        let unique_var = format!("__TEST_BEARER_{}", std::process::id());
        std::env::set_var(&unique_var, "s3cret");
        let cfg = xiaolin_core::agent_config::McpServerConfig {
            id: "test".into(),
            command: String::new(),
            args: vec![],
            enabled: Some(true),
            env: Default::default(),
            url: Some("http://localhost/mcp".into()),
            transport: xiaolin_core::agent_config::McpTransportType::StreamableHttp,
            startup_timeout_sec: None,
            bearer_token_env_var: Some(unique_var.clone()),
            http_headers: None,
        };
        let headers = resolve_mcp_http_headers(&cfg).unwrap();
        assert_eq!(
            headers.get(reqwest::header::AUTHORIZATION).unwrap(),
            "Bearer s3cret"
        );
        std::env::remove_var(&unique_var);
    }

    #[test]
    fn resolve_headers_bearer_missing_env() {
        let cfg = xiaolin_core::agent_config::McpServerConfig {
            id: "test".into(),
            command: String::new(),
            args: vec![],
            enabled: Some(true),
            env: Default::default(),
            url: Some("http://localhost/mcp".into()),
            transport: xiaolin_core::agent_config::McpTransportType::StreamableHttp,
            startup_timeout_sec: None,
            bearer_token_env_var: Some("__NONEXISTENT_VAR_12345__".into()),
            http_headers: None,
        };
        let err = resolve_mcp_http_headers(&cfg).unwrap_err();
        assert!(err.to_string().contains("not found"), "err: {err}");
    }

    #[test]
    fn resolve_headers_static_and_env_ref() {
        let unique_var = format!("__TEST_HDR_{}", std::process::id());
        std::env::set_var(&unique_var, "resolved_value");
        let mut hmap = std::collections::HashMap::new();
        hmap.insert("X-Static".to_string(), "hello".to_string());
        hmap.insert("X-Dynamic".to_string(), format!("${unique_var}"));
        hmap.insert("X-Missing".to_string(), "$__NO_SUCH_ENV_999__".to_string());
        let cfg = xiaolin_core::agent_config::McpServerConfig {
            id: "test".into(),
            command: String::new(),
            args: vec![],
            enabled: Some(true),
            env: Default::default(),
            url: Some("http://localhost/mcp".into()),
            transport: xiaolin_core::agent_config::McpTransportType::StreamableHttp,
            startup_timeout_sec: None,
            bearer_token_env_var: None,
            http_headers: Some(hmap),
        };
        let headers = resolve_mcp_http_headers(&cfg).unwrap();
        assert_eq!(headers.get("X-Static").unwrap(), "hello");
        assert_eq!(headers.get("X-Dynamic").unwrap(), "resolved_value");
        assert!(headers.get("X-Missing").is_none(), "missing env var should be skipped");
        std::env::remove_var(&unique_var);
    }

    #[tokio::test]
    async fn mcp_server_initialize() {
        let server = McpServer::new("test-server", "1.0.0");
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: serde_json::json!(1),
            method: "initialize".into(),
            params: None,
        };
        let resp = server.handle_request(&req).await;
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["serverInfo"]["name"], "test-server");
        assert_eq!(result["protocolVersion"], "2025-06-18");
    }

    #[tokio::test]
    async fn mcp_server_tools_list() {
        let mut server = McpServer::new("test", "1.0");
        server.tools.push(McpTool {
            name: "echo".into(),
            description: Some("echo input".into()),
            input_schema: None,
            meta: None,
        });

        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: serde_json::json!(2),
            method: "tools/list".into(),
            params: None,
        };
        let resp = server.handle_request(&req).await;
        let result = resp.result.unwrap();
        assert_eq!(result["tools"].as_array().unwrap().len(), 1);
        assert_eq!(result["tools"][0]["name"], "echo");
    }

    #[tokio::test]
    async fn mcp_server_ping() {
        let server = McpServer::new("test", "1.0");
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: serde_json::json!(3),
            method: "ping".into(),
            params: None,
        };
        let resp = server.handle_request(&req).await;
        assert!(resp.error.is_none());
    }

    #[tokio::test]
    async fn mcp_server_unknown_method() {
        let server = McpServer::new("test", "1.0");
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: serde_json::json!(4),
            method: "unknown/method".into(),
            params: None,
        };
        let resp = server.handle_request(&req).await;
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, -32601);
    }

    #[tokio::test]
    async fn mcp_server_tool_call() {
        let mut server = McpServer::new("test", "1.0");
        let tool = McpTool {
            name: "greet".into(),
            description: Some("greet someone".into()),
            input_schema: None,
            meta: None,
        };
        server.register_tool(tool, |args| async move {
            let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("world");
            Ok(CallToolResult {
                content: vec![ToolContent::Text {
                    text: format!("Hello, {name}!"),
                }],
                is_error: false,
            })
        });

        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: serde_json::json!(5),
            method: "tools/call".into(),
            params: Some(serde_json::json!({
                "name": "greet",
                "arguments": { "name": "XiaoLin" },
            })),
        };
        let resp = server.handle_request(&req).await;
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["content"][0]["text"], "Hello, XiaoLin!");
        assert_eq!(result["is_error"], false);
    }

    #[test]
    fn companion_post_url_maps_path() {
        let u = reqwest::Url::parse("http://127.0.0.1:9/api/mcp/sse").unwrap();
        let p = super::companion_post_url(&u).unwrap();
        assert_eq!(p.as_str(), "http://127.0.0.1:9/api/mcp/message");
    }

    #[test]
    fn sse_extract_data_payloads() {
        let block = "event: x\ndata: {\"jsonrpc\":\"2.0\",\"id\":1}\n\n";
        let lines = super::extract_sse_data_lines(block);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("jsonrpc"));
    }

    #[tokio::test]
    async fn mcp_client_sse_mock_server() {
        use axum::body::{Body, Bytes};
        use axum::extract::State;
        use axum::http::StatusCode;
        use axum::response::Response;
        use axum::routing::{get, post};
        use axum::Router;
        use std::sync::Arc;
        use tokio::sync::broadcast;

        #[derive(Clone)]
        struct AppState {
            b: broadcast::Sender<Vec<u8>>,
        }

        async fn get_sse(State(s): State<Arc<AppState>>) -> Response<Body> {
            let rx = s.b.subscribe();
            let stream = futures::stream::unfold(rx, |mut rx| async move {
                loop {
                    match rx.recv().await {
                        Ok(msg) => {
                            return Some((
                                Ok::<Bytes, std::convert::Infallible>(Bytes::from(msg)),
                                rx,
                            ))
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {}
                        Err(broadcast::error::RecvError::Closed) => return None,
                    }
                }
            });
            Response::builder()
                .header(axum::http::header::CONTENT_TYPE, "text/event-stream")
                .body(Body::from_stream(stream))
                .unwrap()
        }

        async fn post_msg(State(s): State<Arc<AppState>>, body: Bytes) -> StatusCode {
            let text = String::from_utf8_lossy(&body);
            let v: serde_json::Value = match serde_json::from_str(&text) {
                Ok(v) => v,
                Err(_) => return StatusCode::BAD_REQUEST,
            };
            if v.get("method").and_then(|m| m.as_str()) == Some("notifications/initialized") {
                return StatusCode::ACCEPTED;
            }
            let req: JsonRpcRequest = match serde_json::from_value(v) {
                Ok(r) => r,
                Err(_) => return StatusCode::BAD_REQUEST,
            };
            let resp = match req.method.as_str() {
                "initialize" => JsonRpcResponse::success(
                    req.id.clone(),
                    serde_json::to_value(InitializeResult {
                        protocol_version: "2024-11-05".into(),
                        capabilities: ServerCapabilities {
                            tools: Some(ToolCapability {
                                list_changed: false,
                            }),
                            resources: None,
                            prompts: None,
                        },
                        server_info: ServerInfo {
                            name: "mock-sse".into(),
                            version: "1.0.0".into(),
                        },
                        instructions: None,
                    })
                    .unwrap(),
                ),
                "tools/list" => JsonRpcResponse::success(
                    req.id.clone(),
                    serde_json::to_value(ToolListResult {
                        tools: vec![McpTool {
                            name: "t1".into(),
                            description: None,
                            input_schema: None,
                            meta: None,
                        }],
                    })
                    .unwrap(),
                ),
                _ => JsonRpcResponse::error(req.id.clone(), -32601, "unknown method"),
            };
            let sse_line = format!("data: {}\n\n", serde_json::to_string(&resp).unwrap());
            let _ = s.b.send(sse_line.into_bytes());
            StatusCode::ACCEPTED
        }

        let (tx, _) = broadcast::channel::<Vec<u8>>(32);
        let app = Router::new()
            .route("/mcp/sse", get(get_sse))
            .route("/mcp/message", post(post_msg))
            .with_state(Arc::new(AppState { b: tx }));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        tokio::time::sleep(Duration::from_millis(80)).await;

        let url = format!("http://127.0.0.1:{}/mcp/sse", addr.port());
        let client = McpClient::connect_sse(&url, Default::default()).await.expect("sse connect");
        assert_eq!(client.server_name(), "mock-sse");
        assert_eq!(client.tools().len(), 1);
        assert_eq!(client.tools()[0].name, "t1");
    }

    /// SSE may carry non-JSON `data:` lines (e.g. keep-alives); the reader must ignore them.
    #[tokio::test]
    async fn sse_reader_ignores_non_json_data_lines() {
        use axum::body::{Body, Bytes};
        use axum::extract::State;
        use axum::http::StatusCode;
        use axum::response::Response;
        use axum::routing::{get, post};
        use axum::Router;
        use std::sync::Arc;
        use tokio::sync::broadcast;

        #[derive(Clone)]
        struct AppState {
            b: broadcast::Sender<Vec<u8>>,
        }

        async fn get_sse(State(s): State<Arc<AppState>>) -> Response<Body> {
            let rx = s.b.subscribe();
            let stream = futures::stream::unfold(rx, |mut rx| async move {
                loop {
                    match rx.recv().await {
                        Ok(msg) => {
                            return Some((
                                Ok::<Bytes, std::convert::Infallible>(Bytes::from(msg)),
                                rx,
                            ))
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {}
                        Err(broadcast::error::RecvError::Closed) => return None,
                    }
                }
            });
            Response::builder()
                .header(axum::http::header::CONTENT_TYPE, "text/event-stream")
                .body(Body::from_stream(stream))
                .unwrap()
        }

        async fn post_msg(State(s): State<Arc<AppState>>, body: Bytes) -> StatusCode {
            let text = String::from_utf8_lossy(&body);
            let v: serde_json::Value = match serde_json::from_str(&text) {
                Ok(v) => v,
                Err(_) => return StatusCode::BAD_REQUEST,
            };
            if v.get("method").and_then(|m| m.as_str()) == Some("notifications/initialized") {
                return StatusCode::ACCEPTED;
            }
            let req: JsonRpcRequest = match serde_json::from_value(v) {
                Ok(r) => r,
                Err(_) => return StatusCode::BAD_REQUEST,
            };
            let resp = match req.method.as_str() {
                "initialize" => JsonRpcResponse::success(
                    req.id.clone(),
                    serde_json::to_value(InitializeResult {
                        protocol_version: "2024-11-05".into(),
                        capabilities: ServerCapabilities {
                            tools: Some(ToolCapability {
                                list_changed: false,
                            }),
                            resources: None,
                            prompts: None,
                        },
                        server_info: ServerInfo {
                            name: "mock-sse-ping".into(),
                            version: "1.0.0".into(),
                        },
                        instructions: None,
                    })
                    .unwrap(),
                ),
                "tools/list" => JsonRpcResponse::success(
                    req.id.clone(),
                    serde_json::to_value(ToolListResult {
                        tools: vec![McpTool {
                            name: "after_ping".into(),
                            description: None,
                            input_schema: None,
                            meta: None,
                        }],
                    })
                    .unwrap(),
                ),
                _ => JsonRpcResponse::error(req.id.clone(), -32601, "unknown method"),
            };
            let _ = s.b.send(b"data: ping\n\n".to_vec());
            let sse_line = format!("data: {}\n\n", serde_json::to_string(&resp).unwrap());
            let _ = s.b.send(sse_line.into_bytes());
            StatusCode::ACCEPTED
        }

        let (tx, _) = broadcast::channel::<Vec<u8>>(32);
        let app = Router::new()
            .route("/mcp/sse", get(get_sse))
            .route("/mcp/message", post(post_msg))
            .with_state(Arc::new(AppState { b: tx }));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        tokio::time::sleep(Duration::from_millis(80)).await;

        let url = format!("http://127.0.0.1:{}/mcp/sse", addr.port());
        let client = McpClient::connect_sse(&url, Default::default())
            .await
            .expect("sse connect with ping noise");
        assert_eq!(client.server_name(), "mock-sse-ping");
        assert_eq!(client.tools().len(), 1);
        assert_eq!(client.tools()[0].name, "after_ping");
    }

    #[tokio::test]
    async fn sse_post_error_surfaces_in_send_request() {
        use axum::body::{Body, Bytes};
        use axum::http::StatusCode;
        use axum::response::Response;
        use axum::routing::{get, post};
        use axum::Router;

        async fn get_sse_hang() -> Response<Body> {
            let stream = futures::stream::pending::<Result<Bytes, std::convert::Infallible>>();
            Response::builder()
                .header(axum::http::header::CONTENT_TYPE, "text/event-stream")
                .body(Body::from_stream(stream))
                .unwrap()
        }

        async fn post_always_500(_body: Bytes) -> StatusCode {
            StatusCode::INTERNAL_SERVER_ERROR
        }

        let app = Router::new()
            .route("/mcp/sse", get(get_sse_hang))
            .route("/mcp/message", post(post_always_500));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        tokio::time::sleep(Duration::from_millis(80)).await;

        let url = format!("http://127.0.0.1:{}/mcp/sse", addr.port());
        let Err(err) = McpClient::connect_sse(&url, Default::default()).await else {
            panic!("initialize POST should fail")
        };
        let msg = format!("{err:#}");
        assert!(
            msg.contains("500") || msg.contains("Internal Server Error"),
            "expected HTTP status in error: {msg}"
        );
        assert!(
            msg.contains("MCP POST"),
            "expected MCP POST prefix in error: {msg}"
        );
    }

    #[tokio::test]
    async fn companion_post_url_rejects_non_sse_path() {
        let url = "http://127.0.0.1:9/mcp/stream";
        let Err(err) = McpClient::connect_sse(url, Default::default()).await else {
            panic!("non-/sse URL should be rejected")
        };
        let msg = format!("{err:#}");
        assert!(
            msg.contains("/sse"),
            "expected /sse requirement in error: {msg}"
        );
    }

    // ---- Resources tests ----

    fn rpc(id: i32, method: &str, params: Option<serde_json::Value>) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: serde_json::json!(id),
            method: method.into(),
            params,
        }
    }

    #[tokio::test]
    async fn resources_list_empty() {
        let server = McpServer::new("test", "1.0");
        let resp = server.handle_request(&rpc(1, "resources/list", None)).await;
        let result = resp.result.unwrap();
        assert_eq!(result["resources"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn resources_list_returns_registered() {
        let mut server = McpServer::new("test", "1.0");
        server.register_resource(
            McpResource {
                uri: "file:///a.txt".into(),
                name: "a.txt".into(),
                description: Some("test".into()),
                mime_type: Some("text/plain".into()),
            },
            |_args| async {
                Ok(serde_json::json!({"contents": [{"uri": "file:///a.txt", "text": "hi"}]}))
            },
        );
        let resp = server.handle_request(&rpc(1, "resources/list", None)).await;
        let resources = resp.result.unwrap()["resources"]
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(resources.len(), 1);
        assert_eq!(resources[0]["uri"], "file:///a.txt");
    }

    #[tokio::test]
    async fn resources_read_success() {
        let mut server = McpServer::new("test", "1.0");
        server.register_resource(
            McpResource {
                uri: "file:///b.txt".into(),
                name: "b.txt".into(),
                description: None,
                mime_type: None,
            },
            |_args| async {
                Ok(serde_json::json!({"contents": [{"uri": "file:///b.txt", "text": "content"}]}))
            },
        );
        let resp = server
            .handle_request(&rpc(
                2,
                "resources/read",
                Some(serde_json::json!({"uri": "file:///b.txt"})),
            ))
            .await;
        let result = resp.result.unwrap();
        assert!(result["contents"].is_array());
    }

    #[tokio::test]
    async fn resources_read_unknown_uri() {
        let server = McpServer::new("test", "1.0");
        let resp = server
            .handle_request(&rpc(
                3,
                "resources/read",
                Some(serde_json::json!({"uri": "file:///missing"})),
            ))
            .await;
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, -32602);
    }

    // ---- Prompts tests ----

    #[tokio::test]
    async fn prompts_list_empty() {
        let server = McpServer::new("test", "1.0");
        let resp = server.handle_request(&rpc(1, "prompts/list", None)).await;
        let result = resp.result.unwrap();
        assert_eq!(result["prompts"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn prompts_list_returns_registered() {
        let mut server = McpServer::new("test", "1.0");
        server.register_prompt(
            McpPrompt {
                name: "summarize".into(),
                description: Some("Summarize text".into()),
                arguments: vec![McpPromptArgument {
                    name: "text".into(),
                    description: None,
                    required: true,
                }],
            },
            |_args| async { Ok(serde_json::json!({"messages": [{"role": "user", "content": {"type": "text", "text": "summarize this"}}]})) },
        );
        let resp = server.handle_request(&rpc(1, "prompts/list", None)).await;
        let prompts = resp.result.unwrap()["prompts"].as_array().unwrap().clone();
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0]["name"], "summarize");
    }

    #[tokio::test]
    async fn prompts_get_success() {
        let mut server = McpServer::new("test", "1.0");
        server.register_prompt(
            McpPrompt {
                name: "greet".into(),
                description: None,
                arguments: vec![],
            },
            |_args| async { Ok(serde_json::json!({"messages": [{"role": "user", "content": {"type": "text", "text": "hello"}}]})) },
        );
        let resp = server
            .handle_request(&rpc(
                2,
                "prompts/get",
                Some(serde_json::json!({"name": "greet"})),
            ))
            .await;
        let result = resp.result.unwrap();
        assert!(result["messages"].is_array());
    }

    #[tokio::test]
    async fn prompts_get_unknown_name() {
        let server = McpServer::new("test", "1.0");
        let resp = server
            .handle_request(&rpc(
                3,
                "prompts/get",
                Some(serde_json::json!({"name": "nope"})),
            ))
            .await;
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, -32602);
    }

    // ---- Initialize capability advertisement tests ----

    #[tokio::test]
    async fn initialize_advertises_resources_when_present() {
        let mut server = McpServer::new("test", "1.0");
        server.register_resource(
            McpResource {
                uri: "file:///x".into(),
                name: "x".into(),
                description: None,
                mime_type: None,
            },
            |_| async { Ok(serde_json::json!({})) },
        );
        let resp = server.handle_request(&rpc(1, "initialize", None)).await;
        let caps = &resp.result.unwrap()["capabilities"];
        assert!(caps["resources"].is_object());
    }

    #[tokio::test]
    async fn initialize_omits_resources_when_empty() {
        let server = McpServer::new("test", "1.0");
        let resp = server.handle_request(&rpc(1, "initialize", None)).await;
        let caps = &resp.result.unwrap()["capabilities"];
        assert!(caps["resources"].is_null());
    }

    // ---- Streamable HTTP transport tests ----

    #[tokio::test]
    async fn streamable_http_mock_json_response() {
        use axum::body::Bytes;
        use axum::extract::State as AxState;
        use axum::http::{HeaderMap, StatusCode};
        use axum::routing::post;
        use axum::Router;
        use std::sync::Arc;
        use tokio::sync::Mutex;

        #[derive(Clone, Default)]
        struct St {
            session: Arc<Mutex<Option<String>>>,
        }

        async fn handle_post(
            AxState(st): AxState<Arc<St>>,
            headers: HeaderMap,
            body: Bytes,
        ) -> (StatusCode, HeaderMap, String) {
            let text = String::from_utf8_lossy(&body);
            let v: serde_json::Value = serde_json::from_str(&text).unwrap();

            let accept = headers
                .get("accept")
                .and_then(|h| h.to_str().ok())
                .unwrap_or("");
            assert!(
                accept.contains("application/json"),
                "Streamable HTTP requests must include application/json in Accept"
            );

            let method = v.get("method").and_then(|m| m.as_str()).unwrap_or("");

            if method == "notifications/initialized" {
                return (StatusCode::ACCEPTED, HeaderMap::new(), String::new());
            }

            let req: JsonRpcRequest = serde_json::from_value(v).unwrap();
            let resp = match req.method.as_str() {
                "initialize" => {
                    let sid = "test-session-123".to_string();
                    *st.session.lock().await = Some(sid.clone());
                    let mut h = HeaderMap::new();
                    h.insert("Mcp-Session-Id", sid.parse().unwrap());
                    let body = serde_json::to_string(&JsonRpcResponse::success(
                        req.id.clone(),
                        serde_json::to_value(InitializeResult {
                            protocol_version: "2025-06-18".into(),
                            capabilities: ServerCapabilities {
                                tools: Some(ToolCapability { list_changed: false }),
                                resources: None,
                                prompts: None,
                            },
                            server_info: ServerInfo {
                                name: "mock-streamable".into(),
                                version: "2.0.0".into(),
                            },
                            instructions: None,
                        })
                        .unwrap(),
                    ))
                    .unwrap();
                    return (StatusCode::OK, h, body);
                }
                "tools/list" => JsonRpcResponse::success(
                    req.id.clone(),
                    serde_json::to_value(ToolListResult {
                        tools: vec![
                            McpTool {
                                name: "alpha".into(),
                                description: Some("first tool".into()),
                                input_schema: None,
                                meta: None,
                            },
                            McpTool {
                                name: "beta".into(),
                                description: None,
                                input_schema: None,
                                meta: None,
                            },
                        ],
                    })
                    .unwrap(),
                ),
                _ => JsonRpcResponse::error(req.id.clone(), -32601, "unknown"),
            };
            let body = serde_json::to_string(&resp).unwrap();
            (StatusCode::OK, HeaderMap::new(), body)
        }

        let state = Arc::new(St::default());
        let app = Router::new()
            .route("/mcp", post(handle_post))
            .with_state(state);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        tokio::time::sleep(Duration::from_millis(80)).await;

        let url = format!("http://127.0.0.1:{}/mcp", addr.port());
        let client = McpClient::connect_streamable_http(&url, Default::default())
            .await
            .expect("streamable http connect");

        assert_eq!(client.server_name(), "mock-streamable");
        assert_eq!(client.tools().len(), 2);
        assert_eq!(client.tools()[0].name, "alpha");
        assert_eq!(client.tools()[1].name, "beta");
    }

    #[tokio::test]
    async fn streamable_http_session_id_propagated() {
        use axum::body::Bytes;
        use axum::extract::State as AxState;
        use axum::http::{HeaderMap, StatusCode};
        use axum::routing::post;
        use axum::Router;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        #[derive(Clone)]
        struct St {
            session_seen_in_tools_list: Arc<AtomicBool>,
        }

        async fn handle_post(
            AxState(st): AxState<Arc<St>>,
            headers: HeaderMap,
            body: Bytes,
        ) -> (StatusCode, HeaderMap, String) {
            let text = String::from_utf8_lossy(&body);
            let v: serde_json::Value = serde_json::from_str(&text).unwrap();
            let method = v.get("method").and_then(|m| m.as_str()).unwrap_or("");

            if method == "notifications/initialized" {
                return (StatusCode::ACCEPTED, HeaderMap::new(), String::new());
            }

            let req: JsonRpcRequest = serde_json::from_value(v).unwrap();
            match req.method.as_str() {
                "initialize" => {
                    let mut h = HeaderMap::new();
                    h.insert("Mcp-Session-Id", "sess-abc".parse().unwrap());
                    let body = serde_json::to_string(&JsonRpcResponse::success(
                        req.id.clone(),
                        serde_json::to_value(InitializeResult {
                            protocol_version: "2025-06-18".into(),
                            capabilities: ServerCapabilities {
                                tools: Some(ToolCapability { list_changed: false }),
                                resources: None,
                                prompts: None,
                            },
                            server_info: ServerInfo {
                                name: "session-test".into(),
                                version: "1.0.0".into(),
                            },
                            instructions: None,
                        })
                        .unwrap(),
                    ))
                    .unwrap();
                    (StatusCode::OK, h, body)
                }
                "tools/list" => {
                    if let Some(sid) = headers.get("Mcp-Session-Id") {
                        if sid.to_str().unwrap_or("") == "sess-abc" {
                            st.session_seen_in_tools_list.store(true, Ordering::SeqCst);
                        }
                    }
                    let body = serde_json::to_string(&JsonRpcResponse::success(
                        req.id.clone(),
                        serde_json::to_value(ToolListResult { tools: vec![] }).unwrap(),
                    ))
                    .unwrap();
                    (StatusCode::OK, HeaderMap::new(), body)
                }
                _ => {
                    let body = serde_json::to_string(&JsonRpcResponse::error(
                        req.id.clone(),
                        -32601,
                        "unknown",
                    ))
                    .unwrap();
                    (StatusCode::OK, HeaderMap::new(), body)
                }
            }
        }

        let state = Arc::new(St {
            session_seen_in_tools_list: Arc::new(AtomicBool::new(false)),
        });
        let check = state.session_seen_in_tools_list.clone();
        let app = Router::new()
            .route("/mcp", post(handle_post))
            .with_state(state);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        tokio::time::sleep(Duration::from_millis(80)).await;

        let url = format!("http://127.0.0.1:{}/mcp", addr.port());
        let _client = McpClient::connect_streamable_http(&url, Default::default())
            .await
            .expect("connect");

        assert!(
            check.load(Ordering::SeqCst),
            "Mcp-Session-Id from initialize response must be sent in subsequent requests"
        );
    }

    #[test]
    fn notification_dispatch_logic() {
        let (notification_tx, mut notification_rx) =
            tokio::sync::broadcast::channel::<McpNotification>(64);

        let notification_json =
            r#"{"jsonrpc":"2.0","method":"notifications/tools/list_changed","params":{}}"#;
        let response_json =
            r#"{"jsonrpc":"2.0","id":"1","result":{"protocolVersion":"2025-06-18"}}"#;

        let mut dispatched_response: Option<JsonRpcResponse> = None;

        for raw in [notification_json, response_json] {
            let value: serde_json::Value = serde_json::from_str(raw).unwrap();
            if value.get("id").is_some() {
                let resp: JsonRpcResponse = serde_json::from_value(value).unwrap();
                dispatched_response = Some(resp);
            } else if let Some(method) = value.get("method").and_then(|m| m.as_str()) {
                let params = value.get("params").cloned();
                let _ = notification_tx.send(McpNotification {
                    method: method.to_string(),
                    params,
                });
            }
        }

        let notif = notification_rx.try_recv().expect("should have received notification");
        assert_eq!(notif.method, "notifications/tools/list_changed");
        assert!(notif.params.is_some());

        let resp = dispatched_response.expect("should have dispatched response");
        assert!(resp.error.is_none());
        assert_eq!(resp.id, serde_json::json!("1"));
    }

    #[test]
    fn mcp_notification_clone_and_debug() {
        let n = McpNotification {
            method: "notifications/message".to_string(),
            params: Some(serde_json::json!({"level": "info", "data": "hello"})),
        };
        let n2 = n.clone();
        assert_eq!(n.method, n2.method);
        assert_eq!(format!("{:?}", n).contains("notifications/message"), true);
    }

    #[test]
    fn subscribe_notifications_returns_receiver() {
        let (tx, _) = tokio::sync::broadcast::channel::<McpNotification>(16);
        let mut rx = tx.subscribe();
        let _ = tx.send(McpNotification {
            method: "test".to_string(),
            params: None,
        });
        let msg = rx.try_recv().expect("should receive");
        assert_eq!(msg.method, "test");
        assert!(msg.params.is_none());
    }

    #[test]
    fn session_expired_response_detection() {
        let expired = JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id: serde_json::json!(1),
            result: None,
            error: Some(JsonRpcError {
                code: SESSION_EXPIRED_ERROR_CODE,
                message: "Session expired".into(),
                data: None,
            }),
        };
        assert!(McpClient::is_session_expired_response(&expired));

        let normal_error = JsonRpcResponse::error(serde_json::json!(2), -32601, "not found");
        assert!(!McpClient::is_session_expired_response(&normal_error));

        let success =
            JsonRpcResponse::success(serde_json::json!(3), serde_json::json!({"ok": true}));
        assert!(!McpClient::is_session_expired_response(&success));
    }

    #[test]
    fn session_expired_error_type() {
        let err: anyhow::Error = SessionExpired("HTTP 404".into()).into();
        assert!(err.downcast_ref::<SessionExpired>().is_some());
        assert!(format!("{err}").contains("session expired"));
    }

    /// Mock server that returns 404 once then succeeds, testing automatic session recovery.
    #[tokio::test]
    async fn streamable_http_recovers_from_404() {
        use axum::body::Bytes;
        use axum::extract::State as AxState;
        use axum::http::{HeaderMap, StatusCode};
        use axum::routing::post;
        use axum::Router;
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;

        #[derive(Clone)]
        struct St {
            call_count: Arc<AtomicU32>,
        }

        async fn handle_post(
            AxState(st): AxState<Arc<St>>,
            _headers: HeaderMap,
            body: Bytes,
        ) -> (StatusCode, HeaderMap, String) {
            let text = String::from_utf8_lossy(&body);
            let v: serde_json::Value = match serde_json::from_str(&text) {
                Ok(v) => v,
                Err(_) => return (StatusCode::BAD_REQUEST, HeaderMap::new(), String::new()),
            };
            let method = v.get("method").and_then(|m| m.as_str()).unwrap_or("");

            if method == "notifications/initialized" {
                return (StatusCode::ACCEPTED, HeaderMap::new(), String::new());
            }

            let req: JsonRpcRequest = match serde_json::from_value(v) {
                Ok(r) => r,
                Err(_) => return (StatusCode::BAD_REQUEST, HeaderMap::new(), String::new()),
            };

            match req.method.as_str() {
                "initialize" => {
                    let mut h = HeaderMap::new();
                    let count = st.call_count.fetch_add(1, Ordering::SeqCst);
                    let sid = format!("session-{count}");
                    h.insert("Mcp-Session-Id", sid.parse().unwrap());
                    let body = serde_json::to_string(&JsonRpcResponse::success(
                        req.id.clone(),
                        serde_json::to_value(InitializeResult {
                            protocol_version: "2025-06-18".into(),
                            capabilities: ServerCapabilities {
                                tools: Some(ToolCapability { list_changed: false }),
                                resources: None,
                                prompts: None,
                            },
                            server_info: ServerInfo {
                                name: "recovery-test".into(),
                                version: "1.0.0".into(),
                            },
                            instructions: None,
                        })
                        .unwrap(),
                    ))
                    .unwrap();
                    (StatusCode::OK, h, body)
                }
                "tools/list" => {
                    let count = st.call_count.load(Ordering::SeqCst);
                    if count <= 1 {
                        return (StatusCode::NOT_FOUND, HeaderMap::new(), String::new());
                    }
                    let body = serde_json::to_string(&JsonRpcResponse::success(
                        req.id.clone(),
                        serde_json::to_value(ToolListResult {
                            tools: vec![McpTool {
                                name: "recovered_tool".into(),
                                description: None,
                                input_schema: None,
                                meta: None,
                            }],
                        })
                        .unwrap(),
                    ))
                    .unwrap();
                    (StatusCode::OK, HeaderMap::new(), body)
                }
                _ => {
                    let body = serde_json::to_string(&JsonRpcResponse::error(
                        req.id.clone(),
                        -32601,
                        "unknown",
                    ))
                    .unwrap();
                    (StatusCode::OK, HeaderMap::new(), body)
                }
            }
        }

        let state = Arc::new(St {
            call_count: Arc::new(AtomicU32::new(0)),
        });
        let app = Router::new()
            .route("/mcp", post(handle_post))
            .with_state(state);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        tokio::time::sleep(Duration::from_millis(80)).await;

        let url = format!("http://127.0.0.1:{}/mcp", addr.port());
        let client = McpClient::connect_streamable_http(&url, Default::default())
            .await
            .expect("should recover from 404 during tools/list");

        assert_eq!(client.server_name(), "recovery-test");
        assert_eq!(client.tools().len(), 1);
        assert_eq!(client.tools()[0].name, "recovered_tool");
    }

    /// Mock server that returns JSON-RPC -32001 once then succeeds.
    #[tokio::test]
    async fn streamable_http_recovers_from_json_rpc_32001() {
        use axum::body::Bytes;
        use axum::extract::State as AxState;
        use axum::http::{HeaderMap, StatusCode};
        use axum::routing::post;
        use axum::Router;
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;

        #[derive(Clone)]
        struct St {
            tools_list_count: Arc<AtomicU32>,
        }

        async fn handle_post(
            AxState(st): AxState<Arc<St>>,
            _headers: HeaderMap,
            body: Bytes,
        ) -> (StatusCode, HeaderMap, String) {
            let text = String::from_utf8_lossy(&body);
            let v: serde_json::Value = match serde_json::from_str(&text) {
                Ok(v) => v,
                Err(_) => return (StatusCode::BAD_REQUEST, HeaderMap::new(), String::new()),
            };
            let method = v.get("method").and_then(|m| m.as_str()).unwrap_or("");

            if method == "notifications/initialized" {
                return (StatusCode::ACCEPTED, HeaderMap::new(), String::new());
            }

            let req: JsonRpcRequest = match serde_json::from_value(v) {
                Ok(r) => r,
                Err(_) => return (StatusCode::BAD_REQUEST, HeaderMap::new(), String::new()),
            };

            match req.method.as_str() {
                "initialize" => {
                    let mut h = HeaderMap::new();
                    h.insert("Mcp-Session-Id", "sess-new".parse().unwrap());
                    let body = serde_json::to_string(&JsonRpcResponse::success(
                        req.id.clone(),
                        serde_json::to_value(InitializeResult {
                            protocol_version: "2025-06-18".into(),
                            capabilities: ServerCapabilities {
                                tools: Some(ToolCapability { list_changed: false }),
                                resources: None,
                                prompts: None,
                            },
                            server_info: ServerInfo {
                                name: "rpc-recovery-test".into(),
                                version: "1.0.0".into(),
                            },
                            instructions: None,
                        })
                        .unwrap(),
                    ))
                    .unwrap();
                    (StatusCode::OK, h, body)
                }
                "tools/list" => {
                    let count = st.tools_list_count.fetch_add(1, Ordering::SeqCst);
                    if count == 0 {
                        let body = serde_json::to_string(&JsonRpcResponse::error(
                            req.id.clone(),
                            SESSION_EXPIRED_ERROR_CODE,
                            "Session expired",
                        ))
                        .unwrap();
                        return (StatusCode::OK, HeaderMap::new(), body);
                    }
                    let body = serde_json::to_string(&JsonRpcResponse::success(
                        req.id.clone(),
                        serde_json::to_value(ToolListResult {
                            tools: vec![McpTool {
                                name: "after_recovery".into(),
                                description: None,
                                input_schema: None,
                                meta: None,
                            }],
                        })
                        .unwrap(),
                    ))
                    .unwrap();
                    (StatusCode::OK, HeaderMap::new(), body)
                }
                _ => {
                    let body = serde_json::to_string(&JsonRpcResponse::error(
                        req.id.clone(),
                        -32601,
                        "unknown",
                    ))
                    .unwrap();
                    (StatusCode::OK, HeaderMap::new(), body)
                }
            }
        }

        let state = Arc::new(St {
            tools_list_count: Arc::new(AtomicU32::new(0)),
        });
        let app = Router::new()
            .route("/mcp", post(handle_post))
            .with_state(state);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        tokio::time::sleep(Duration::from_millis(80)).await;

        let url = format!("http://127.0.0.1:{}/mcp", addr.port());
        let client = McpClient::connect_streamable_http(&url, Default::default())
            .await
            .expect("should recover from -32001 during tools/list");

        assert_eq!(client.server_name(), "rpc-recovery-test");
        assert_eq!(client.tools().len(), 1);
        assert_eq!(client.tools()[0].name, "after_recovery");
    }

    #[test]
    fn resource_list_result_parses() {
        let json = serde_json::json!({
            "resources": [
                {
                    "uri": "file:///logs/app.log",
                    "name": "App Log",
                    "description": "Application log file",
                    "mimeType": "text/plain"
                },
                {
                    "uri": "db://users",
                    "name": "Users",
                }
            ]
        });
        #[derive(Deserialize)]
        struct ResourceListResult {
            #[serde(default)]
            resources: Vec<McpResource>,
        }
        let parsed: ResourceListResult = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.resources.len(), 2);
        assert_eq!(parsed.resources[0].uri, "file:///logs/app.log");
        assert_eq!(parsed.resources[0].name, "App Log");
        assert!(parsed.resources[0].description.is_some());
        assert_eq!(parsed.resources[0].mime_type.as_deref(), Some("text/plain"));
        assert!(parsed.resources[1].description.is_none());
        assert!(parsed.resources[1].mime_type.is_none());
    }

    #[test]
    fn resource_template_parses() {
        let json = serde_json::json!({
            "resourceTemplates": [
                {
                    "uriTemplate": "file:///{path}",
                    "name": "File",
                    "description": "Read any file"
                }
            ]
        });
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct TemplateListResult {
            #[serde(default)]
            resource_templates: Vec<McpResourceTemplate>,
        }
        let parsed: TemplateListResult = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.resource_templates.len(), 1);
        assert_eq!(parsed.resource_templates[0].uri_template, "file:///{path}");
        assert_eq!(parsed.resource_templates[0].name, "File");
    }

    #[test]
    fn resource_content_parses() {
        let json = serde_json::json!({
            "contents": [
                {
                    "uri": "file:///x.txt",
                    "mimeType": "text/plain",
                    "text": "hello world"
                }
            ]
        });
        #[derive(Deserialize)]
        struct ReadResult {
            #[serde(default)]
            contents: Vec<McpResourceContent>,
        }
        let parsed: ReadResult = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.contents.len(), 1);
        assert_eq!(parsed.contents[0].text.as_deref(), Some("hello world"));
        assert!(parsed.contents[0].blob.is_none());
    }

    #[test]
    fn resource_content_truncation() {
        let big_text = "A".repeat(2_000_000);
        let mut content = McpResourceContent {
            uri: "test://x".into(),
            mime_type: None,
            text: Some(big_text),
            blob: None,
        };

        const MAX: usize = 1_048_576;
        if let Some(text) = &mut content.text {
            if text.len() > MAX {
                let boundary = text.floor_char_boundary(MAX);
                text.truncate(boundary);
                text.push_str("\n[truncated]");
            }
        }

        let text = content.text.unwrap();
        assert!(text.len() < 2_000_000);
        assert!(text.ends_with("[truncated]"));
        assert!(text.len() > MAX);
    }

    #[test]
    fn server_capabilities_default() {
        let caps = ServerCapabilities::default();
        assert!(caps.tools.is_none());
        assert!(caps.resources.is_none());
        assert!(caps.prompts.is_none());
    }

    #[tokio::test]
    async fn mcp_server_resources_list() {
        let mut server = McpServer::new("test", "1.0");
        server.register_resource(
            McpResource {
                uri: "file:///a.txt".into(),
                name: "a".into(),
                description: Some("file a".into()),
                mime_type: Some("text/plain".into()),
            },
            |_| async { Ok(serde_json::json!({"contents": [{"uri":"file:///a.txt","text":"hi"}]})) },
        );

        let resp = server
            .handle_request(&rpc(10, "resources/list", None))
            .await;
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        let resources = result["resources"].as_array().unwrap();
        assert_eq!(resources.len(), 1);
        assert_eq!(resources[0]["name"], "a");
    }

    #[tokio::test]
    async fn mcp_server_resources_read() {
        let mut server = McpServer::new("test", "1.0");
        server.register_resource(
            McpResource {
                uri: "test://data".into(),
                name: "data".into(),
                description: None,
                mime_type: None,
            },
            |_| async {
                Ok(serde_json::json!({
                    "contents": [{"uri": "test://data", "text": "the content"}]
                }))
            },
        );

        let resp = server
            .handle_request(&rpc(
                11,
                "resources/read",
                Some(serde_json::json!({"uri": "test://data"})),
            ))
            .await;
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        let contents = result["contents"].as_array().unwrap();
        assert_eq!(contents[0]["text"], "the content");
    }

    #[test]
    fn prompt_list_result_parses() {
        let json = serde_json::json!({
            "prompts": [
                {
                    "name": "code_review",
                    "description": "Review code changes",
                    "arguments": [
                        { "name": "code", "description": "The code to review", "required": true },
                        { "name": "language", "description": "Programming language" }
                    ]
                },
                {
                    "name": "summarize",
                    "description": "Summarize text"
                }
            ]
        });
        #[derive(Deserialize)]
        struct PromptListResult {
            #[serde(default)]
            prompts: Vec<McpPrompt>,
        }
        let parsed: PromptListResult = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.prompts.len(), 2);
        assert_eq!(parsed.prompts[0].name, "code_review");
        assert_eq!(parsed.prompts[0].arguments.len(), 2);
        assert!(parsed.prompts[0].arguments[0].required);
        assert!(!parsed.prompts[0].arguments[1].required);
        assert_eq!(parsed.prompts[1].name, "summarize");
        assert!(parsed.prompts[1].arguments.is_empty());
    }

    #[test]
    fn prompt_message_text_parses() {
        let json = serde_json::json!({
            "messages": [
                {
                    "role": "user",
                    "content": { "type": "text", "text": "Hello world" }
                },
                {
                    "role": "assistant",
                    "content": { "type": "text", "text": "Hi there" }
                }
            ]
        });
        #[derive(Deserialize)]
        struct GetPromptResult {
            #[serde(default)]
            messages: Vec<McpPromptMessage>,
        }
        let parsed: GetPromptResult = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.messages.len(), 2);
        assert_eq!(parsed.messages[0].role, "user");
        match &parsed.messages[0].content {
            McpPromptContent::Text { text } => assert_eq!(text, "Hello world"),
            _ => panic!("expected text content"),
        }
    }

    #[test]
    fn prompt_message_image_parses() {
        let json = serde_json::json!({
            "role": "user",
            "content": {
                "type": "image",
                "data": "iVBORw0KGgo...",
                "mimeType": "image/png"
            }
        });
        let msg: McpPromptMessage = serde_json::from_value(json).unwrap();
        match &msg.content {
            McpPromptContent::Image { data, mime_type } => {
                assert_eq!(data, "iVBORw0KGgo...");
                assert_eq!(mime_type, "image/png");
            }
            _ => panic!("expected image content"),
        }
    }

    #[test]
    fn prompt_message_resource_parses() {
        let json = serde_json::json!({
            "role": "user",
            "content": {
                "type": "resource",
                "resource": {
                    "uri": "file:///test.txt",
                    "mimeType": "text/plain",
                    "text": "file content here"
                }
            }
        });
        let msg: McpPromptMessage = serde_json::from_value(json).unwrap();
        match &msg.content {
            McpPromptContent::Resource { resource } => {
                assert_eq!(resource.uri, "file:///test.txt");
                assert_eq!(resource.text.as_deref(), Some("file content here"));
            }
            _ => panic!("expected resource content"),
        }
    }

    #[test]
    fn prompt_with_empty_arguments() {
        let json = serde_json::json!({
            "name": "simple_prompt",
            "description": "A simple prompt"
        });
        let prompt: McpPrompt = serde_json::from_value(json).unwrap();
        assert_eq!(prompt.name, "simple_prompt");
        assert!(prompt.arguments.is_empty());
    }

    #[tokio::test]
    async fn dispatch_incoming_routes_server_request() {
        let pending: Arc<
            tokio::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<JsonRpcResponse>>>,
        > = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let (ntx, _) = tokio::sync::broadcast::channel::<McpNotification>(16);
        let (srtx, mut srrx) = tokio::sync::broadcast::channel::<McpServerRequest>(16);

        let server_request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 42,
            "method": "elicitation/create",
            "params": {
                "message": "Please enter your name",
                "requestedSchema": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" }
                    }
                }
            }
        });

        McpClient::dispatch_incoming(server_request, &pending, &ntx, &srtx, "test").await;

        let req = srrx.try_recv().expect("should receive server request");
        assert_eq!(req.method, "elicitation/create");
        assert_eq!(req.id, serde_json::json!(42));
        assert!(req.params.is_some());
        let params = req.params.unwrap();
        assert_eq!(params["message"], "Please enter your name");
    }

    #[tokio::test]
    async fn dispatch_incoming_routes_response() {
        let pending: Arc<
            tokio::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<JsonRpcResponse>>>,
        > = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let (ntx, mut nrx) = tokio::sync::broadcast::channel::<McpNotification>(16);
        let (srtx, mut srrx) = tokio::sync::broadcast::channel::<McpServerRequest>(16);

        let (tx, rx) = tokio::sync::oneshot::channel();
        pending.lock().await.insert("99".to_string(), tx);

        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 99,
            "result": { "tools": [] }
        });

        McpClient::dispatch_incoming(response, &pending, &ntx, &srtx, "test").await;

        let resp = rx.await.expect("should receive response");
        assert!(resp.error.is_none());
        assert_eq!(resp.id, serde_json::json!(99));

        assert!(nrx.try_recv().is_err());
        assert!(srrx.try_recv().is_err());
    }

    #[tokio::test]
    async fn dispatch_incoming_routes_notification() {
        let pending: Arc<
            tokio::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<JsonRpcResponse>>>,
        > = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let (ntx, mut nrx) = tokio::sync::broadcast::channel::<McpNotification>(16);
        let (srtx, mut srrx) = tokio::sync::broadcast::channel::<McpServerRequest>(16);

        let notification = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/tools/list_changed",
            "params": {}
        });

        McpClient::dispatch_incoming(notification, &pending, &ntx, &srtx, "test").await;

        let notif = nrx.try_recv().expect("should receive notification");
        assert_eq!(notif.method, "notifications/tools/list_changed");

        assert!(srrx.try_recv().is_err());
    }

    #[test]
    fn server_request_clone_and_debug() {
        let req = McpServerRequest {
            id: serde_json::json!(1),
            method: "elicitation/create".to_string(),
            params: Some(serde_json::json!({"message": "hello"})),
        };
        let req2 = req.clone();
        assert_eq!(req.method, req2.method);
        assert_eq!(format!("{:?}", req).contains("elicitation/create"), true);
    }

    #[test]
    fn elicitation_response_serializes_correctly() {
        let accept = JsonRpcResponse::success(
            serde_json::json!(42),
            serde_json::json!({
                "action": "accept",
                "content": { "name": "Alice" }
            }),
        );
        let json = serde_json::to_value(&accept).unwrap();
        assert_eq!(json["id"], 42);
        assert_eq!(json["result"]["action"], "accept");
        assert_eq!(json["result"]["content"]["name"], "Alice");

        let decline = JsonRpcResponse::success(
            serde_json::json!(43),
            serde_json::json!({ "action": "decline" }),
        );
        let json = serde_json::to_value(&decline).unwrap();
        assert_eq!(json["result"]["action"], "decline");
    }

    // ---- [2.5] Progress notification tests ----

    #[tokio::test]
    async fn progress_notification_dispatched_to_subscriber() {
        let pending: Arc<
            tokio::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<JsonRpcResponse>>>,
        > = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let (ntx, mut nrx) = tokio::sync::broadcast::channel::<McpNotification>(16);
        let (srtx, _) = tokio::sync::broadcast::channel::<McpServerRequest>(16);

        let progress_notif = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/progress",
            "params": {
                "progressToken": "tok-42",
                "progress": 3,
                "total": 10,
                "message": "Processing item 3/10"
            }
        });

        McpClient::dispatch_incoming(progress_notif, &pending, &ntx, &srtx, "test-server").await;

        let notif = nrx.try_recv().expect("should receive progress notification");
        assert_eq!(notif.method, "notifications/progress");
        let params = notif.params.unwrap();
        assert_eq!(params["progressToken"], "tok-42");
        assert_eq!(params["progress"], 3);
        assert_eq!(params["total"], 10);
        assert_eq!(params["message"], "Processing item 3/10");
    }

    #[tokio::test]
    async fn call_tool_with_progress_injects_meta_token() {
        use axum::body::Bytes;
        use axum::extract::State as AxState;
        use axum::http::{HeaderMap, StatusCode};
        use axum::routing::post;
        use axum::Router;
        use std::sync::Arc;
        use tokio::sync::Mutex;

        #[derive(Clone, Default)]
        struct St {
            captured_meta: Arc<Mutex<Option<serde_json::Value>>>,
        }

        async fn handle_post(
            AxState(st): AxState<Arc<St>>,
            _headers: HeaderMap,
            body: Bytes,
        ) -> (StatusCode, HeaderMap, String) {
            let text = String::from_utf8_lossy(&body);
            let v: serde_json::Value = serde_json::from_str(&text).unwrap();
            let method = v.get("method").and_then(|m| m.as_str()).unwrap_or("");

            if method == "notifications/initialized" {
                return (StatusCode::ACCEPTED, HeaderMap::new(), String::new());
            }

            let req: JsonRpcRequest = serde_json::from_value(v.clone()).unwrap();
            match req.method.as_str() {
                "initialize" => {
                    let mut h = HeaderMap::new();
                    h.insert("Mcp-Session-Id", "progress-sess".parse().unwrap());
                    let body = serde_json::to_string(&JsonRpcResponse::success(
                        req.id.clone(),
                        serde_json::to_value(InitializeResult {
                            protocol_version: "2025-06-18".into(),
                            capabilities: ServerCapabilities {
                                tools: Some(ToolCapability { list_changed: false }),
                                resources: None,
                                prompts: None,
                            },
                            server_info: ServerInfo {
                                name: "progress-test".into(),
                                version: "1.0.0".into(),
                            },
                            instructions: None,
                        })
                        .unwrap(),
                    ))
                    .unwrap();
                    (StatusCode::OK, h, body)
                }
                "tools/list" => {
                    let body = serde_json::to_string(&JsonRpcResponse::success(
                        req.id.clone(),
                        serde_json::to_value(ToolListResult {
                            tools: vec![McpTool {
                                name: "slow_op".into(),
                                description: Some("A slow operation".into()),
                                input_schema: None,
                                meta: None,
                            }],
                        })
                        .unwrap(),
                    ))
                    .unwrap();
                    (StatusCode::OK, HeaderMap::new(), body)
                }
                "tools/call" => {
                    if let Some(params) = v.get("params") {
                        if let Some(meta) = params.get("_meta") {
                            *st.captured_meta.lock().await = Some(meta.clone());
                        }
                    }
                    let body = serde_json::to_string(&JsonRpcResponse::success(
                        req.id.clone(),
                        serde_json::to_value(CallToolResult {
                            content: vec![ToolContent::Text {
                                text: "done".into(),
                            }],
                            is_error: false,
                        })
                        .unwrap(),
                    ))
                    .unwrap();
                    (StatusCode::OK, HeaderMap::new(), body)
                }
                _ => {
                    let body = serde_json::to_string(&JsonRpcResponse::error(
                        req.id.clone(),
                        -32601,
                        "unknown",
                    ))
                    .unwrap();
                    (StatusCode::OK, HeaderMap::new(), body)
                }
            }
        }

        let state = Arc::new(St::default());
        let check = state.captured_meta.clone();
        let app = Router::new()
            .route("/mcp", post(handle_post))
            .with_state(state);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        tokio::time::sleep(Duration::from_millis(80)).await;

        let url = format!("http://127.0.0.1:{}/mcp", addr.port());
        let client = McpClient::connect_streamable_http(&url, Default::default())
            .await
            .expect("connect");

        let result = client
            .call_tool_with_progress("slow_op", serde_json::json!({"input": "x"}), Some("my-token"))
            .await
            .expect("tool call should succeed");
        match &result.content[0] {
            ToolContent::Text { text } => assert_eq!(text, "done"),
            other => panic!("expected Text content, got {:?}", other),
        }

        let meta = check.lock().await;
        let meta = meta.as_ref().expect("_meta should have been captured by server");
        assert_eq!(meta["progressToken"], "my-token");
    }

    // ---- [3.4] Cache refresh notification tests ----

    #[tokio::test]
    async fn resources_list_changed_notification_dispatched() {
        let pending: Arc<
            tokio::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<JsonRpcResponse>>>,
        > = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let (ntx, mut nrx) = tokio::sync::broadcast::channel::<McpNotification>(16);
        let (srtx, _) = tokio::sync::broadcast::channel::<McpServerRequest>(16);

        let notif = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/resources/list_changed",
            "params": {}
        });
        McpClient::dispatch_incoming(notif, &pending, &ntx, &srtx, "test").await;

        let received = nrx.try_recv().expect("should receive notification");
        assert_eq!(received.method, "notifications/resources/list_changed");
    }

    #[tokio::test]
    async fn prompts_list_changed_notification_dispatched() {
        let pending: Arc<
            tokio::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<JsonRpcResponse>>>,
        > = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let (ntx, mut nrx) = tokio::sync::broadcast::channel::<McpNotification>(16);
        let (srtx, _) = tokio::sync::broadcast::channel::<McpServerRequest>(16);

        let notif = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/prompts/list_changed",
            "params": {}
        });
        McpClient::dispatch_incoming(notif, &pending, &ntx, &srtx, "test").await;

        let received = nrx.try_recv().expect("should receive notification");
        assert_eq!(received.method, "notifications/prompts/list_changed");
    }

    // ---- [6.4] roots/list server request tests ----

    #[tokio::test]
    async fn roots_list_request_dispatched_as_server_request() {
        let pending: Arc<
            tokio::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<JsonRpcResponse>>>,
        > = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let (ntx, _) = tokio::sync::broadcast::channel::<McpNotification>(16);
        let (srtx, mut srrx) = tokio::sync::broadcast::channel::<McpServerRequest>(16);

        let roots_request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 77,
            "method": "roots/list",
            "params": {}
        });
        McpClient::dispatch_incoming(roots_request, &pending, &ntx, &srtx, "test").await;

        let req = srrx.try_recv().expect("should receive roots/list as server request");
        assert_eq!(req.method, "roots/list");
        assert_eq!(req.id, serde_json::json!(77));
    }

    // ---- [7.5] Reverse MCP server tests ----

    #[tokio::test]
    async fn create_xiaolin_mcp_server_filters_mcp_prefix() {
        use xiaolin_core::tool::{ToolRegistry, ToolParameterSchema, Tool, ToolResult};
        use std::sync::Arc;
        use async_trait::async_trait;

        struct DummyTool(String);
        #[async_trait]
        impl Tool for DummyTool {
            fn name(&self) -> &str { &self.0 }
            fn description(&self) -> &str { "dummy" }
            fn parameters_schema(&self) -> ToolParameterSchema {
                ToolParameterSchema {
                    schema_type: "object".into(),
                    properties: Default::default(),
                    required: vec![],
                }
            }
            async fn execute(&self, _args: &str) -> ToolResult {
                ToolResult {
                    output: format!("result from {}", self.0),
                    success: true,
                    display_output: None,
                    error_type: None,
                    metadata: None,
                    images: vec![],
                }
            }
        }

        let registry = ToolRegistry::new();
        registry.register(Arc::new(DummyTool("read_file".into())));
        registry.register(Arc::new(DummyTool("write_file".into())));
        registry.register(Arc::new(DummyTool("mcp__github__list_repos".into())));
        registry.register(Arc::new(DummyTool("mcp__slack__send_message".into())));

        let registry = Arc::new(registry);
        let server = create_xiaolin_mcp_server(&registry);

        let resp = server.handle_request(&rpc(1, "initialize", None)).await;
        let result = resp.result.unwrap();
        assert_eq!(result["serverInfo"]["name"], "XiaoLin");

        let resp = server.handle_request(&rpc(2, "tools/list", None)).await;
        let tools = resp.result.unwrap()["tools"].as_array().unwrap().clone();
        let tool_names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(tool_names.contains(&"read_file"));
        assert!(tool_names.contains(&"write_file"));
        assert!(!tool_names.iter().any(|n| n.starts_with("mcp__")),
            "mcp__ prefixed tools should be filtered out, but found: {:?}", tool_names);
    }

    #[tokio::test]
    async fn create_xiaolin_mcp_server_tool_call_works() {
        use xiaolin_core::tool::{ToolRegistry, ToolParameterSchema, Tool, ToolResult};
        use std::sync::Arc;
        use async_trait::async_trait;

        struct EchoTool;
        #[async_trait]
        impl Tool for EchoTool {
            fn name(&self) -> &str { "echo" }
            fn description(&self) -> &str { "Echo back the input" }
            fn parameters_schema(&self) -> ToolParameterSchema {
                ToolParameterSchema {
                    schema_type: "object".into(),
                    properties: Default::default(),
                    required: vec![],
                }
            }
            async fn execute(&self, args: &str) -> ToolResult {
                let v: serde_json::Value = serde_json::from_str(args).unwrap_or_default();
                let msg = v.get("msg").and_then(|m| m.as_str()).unwrap_or("empty");
                ToolResult {
                    output: format!("Echo: {msg}"),
                    success: true,
                    display_output: None,
                    error_type: None,
                    metadata: None,
                    images: vec![],
                }
            }
        }

        let registry = Arc::new(ToolRegistry::new());
        registry.register(Arc::new(EchoTool));

        let server = create_xiaolin_mcp_server(&registry);
        let resp = server
            .handle_request(&rpc(
                5,
                "tools/call",
                Some(serde_json::json!({"name": "echo", "arguments": {"msg": "hello"}})),
            ))
            .await;
        assert!(resp.error.is_none(), "tool call should succeed: {:?}", resp.error);
        let result = resp.result.unwrap();
        assert_eq!(result["content"][0]["text"], "Echo: hello");
        assert_eq!(result["is_error"], false);
    }

    // ---- [8.7] WebSocket transport tests ----

    async fn accept_ws_with_mcp_subprotocol(
        stream: tokio::net::TcpStream,
    ) -> tokio_tungstenite::WebSocketStream<tokio::net::TcpStream> {
        use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};

        tokio_tungstenite::accept_hdr_async(stream, |req: &Request, mut resp: Response| {
            if let Some(protos) = req.headers().get("Sec-WebSocket-Protocol") {
                let val = protos.to_str().unwrap_or("");
                if val.contains("mcp") {
                    resp.headers_mut().insert(
                        "Sec-WebSocket-Protocol",
                        "mcp".parse().unwrap(),
                    );
                }
            }
            Ok(resp)
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn websocket_mcp_client_connect_and_tools_list() {
        use futures::{SinkExt, StreamExt};
        use tokio::net::TcpListener;
        use tokio_tungstenite::tungstenite::Message;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let ws_stream = accept_ws_with_mcp_subprotocol(stream).await;
            let (mut write, mut read) = ws_stream.split();

            while let Some(Ok(msg)) = read.next().await {
                if let Message::Text(text) = msg {
                    let v: serde_json::Value = match serde_json::from_str(&text) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    let method = v.get("method").and_then(|m| m.as_str()).unwrap_or("");

                    if method == "notifications/initialized" {
                        continue;
                    }

                    let req: JsonRpcRequest = match serde_json::from_value(v) {
                        Ok(r) => r,
                        Err(_) => continue,
                    };
                    let resp = match req.method.as_str() {
                        "initialize" => JsonRpcResponse::success(
                            req.id.clone(),
                            serde_json::to_value(InitializeResult {
                                protocol_version: "2025-06-18".into(),
                                capabilities: ServerCapabilities {
                                    tools: Some(ToolCapability { list_changed: true }),
                                    resources: None,
                                    prompts: None,
                                },
                                server_info: ServerInfo {
                                    name: "mock-ws".into(),
                                    version: "1.0.0".into(),
                                },
                                instructions: None,
                            })
                            .unwrap(),
                        ),
                        "tools/list" => JsonRpcResponse::success(
                            req.id.clone(),
                            serde_json::to_value(ToolListResult {
                                tools: vec![
                                    McpTool {
                                        name: "ws_tool_a".into(),
                                        description: Some("First WS tool".into()),
                                        input_schema: None,
                                        meta: None,
                                    },
                                    McpTool {
                                        name: "ws_tool_b".into(),
                                        description: None,
                                        input_schema: None,
                                        meta: None,
                                    },
                                ],
                            })
                            .unwrap(),
                        ),
                        _ => JsonRpcResponse::error(req.id.clone(), -32601, "unknown"),
                    };
                    let resp_text = serde_json::to_string(&resp).unwrap();
                    let _ = write.send(Message::Text(resp_text.into())).await;
                }
            }
        });

        tokio::time::sleep(Duration::from_millis(100)).await;

        let url = format!("ws://127.0.0.1:{}", addr.port());
        let client = McpClient::connect_websocket(&url).await.expect("WS connect should succeed");

        assert_eq!(client.server_name(), "mock-ws");
        assert_eq!(client.tools().len(), 2);
        assert_eq!(client.tools()[0].name, "ws_tool_a");
        assert_eq!(client.tools()[1].name, "ws_tool_b");
    }

    #[tokio::test]
    async fn websocket_mcp_client_tool_call() {
        use futures::{SinkExt, StreamExt};
        use tokio::net::TcpListener;
        use tokio_tungstenite::tungstenite::Message;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let ws_stream = accept_ws_with_mcp_subprotocol(stream).await;
            let (mut write, mut read) = ws_stream.split();

            while let Some(Ok(msg)) = read.next().await {
                if let Message::Text(text) = msg {
                    let v: serde_json::Value = match serde_json::from_str(&text) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    let method = v.get("method").and_then(|m| m.as_str()).unwrap_or("");
                    if method == "notifications/initialized" {
                        continue;
                    }

                    let req: JsonRpcRequest = match serde_json::from_value(v) {
                        Ok(r) => r,
                        Err(_) => continue,
                    };
                    let resp = match req.method.as_str() {
                        "initialize" => JsonRpcResponse::success(
                            req.id.clone(),
                            serde_json::to_value(InitializeResult {
                                protocol_version: "2025-06-18".into(),
                                capabilities: ServerCapabilities {
                                    tools: Some(ToolCapability { list_changed: false }),
                                    resources: None,
                                    prompts: None,
                                },
                                server_info: ServerInfo {
                                    name: "ws-call-test".into(),
                                    version: "1.0.0".into(),
                                },
                                instructions: None,
                            })
                            .unwrap(),
                        ),
                        "tools/list" => JsonRpcResponse::success(
                            req.id.clone(),
                            serde_json::to_value(ToolListResult {
                                tools: vec![McpTool {
                                    name: "add".into(),
                                    description: Some("Add two numbers".into()),
                                    input_schema: None,
                                    meta: None,
                                }],
                            })
                            .unwrap(),
                        ),
                        "tools/call" => {
                            let params = req.params.unwrap_or_default();
                            let args = params.get("arguments").cloned().unwrap_or_default();
                            let a = args.get("a").and_then(|v| v.as_i64()).unwrap_or(0);
                            let b = args.get("b").and_then(|v| v.as_i64()).unwrap_or(0);
                            JsonRpcResponse::success(
                                req.id.clone(),
                                serde_json::to_value(CallToolResult {
                                    content: vec![ToolContent::Text {
                                        text: format!("{}", a + b),
                                    }],
                                    is_error: false,
                                })
                                .unwrap(),
                            )
                        }
                        _ => JsonRpcResponse::error(req.id.clone(), -32601, "unknown"),
                    };
                    let resp_text = serde_json::to_string(&resp).unwrap();
                    let _ = write.send(Message::Text(resp_text.into())).await;
                }
            }
        });

        tokio::time::sleep(Duration::from_millis(100)).await;

        let url = format!("ws://127.0.0.1:{}", addr.port());
        let client = McpClient::connect_websocket(&url).await.expect("WS connect");

        let result = client
            .call_tool("add", serde_json::json!({"a": 17, "b": 25}))
            .await
            .expect("tool call should succeed");
        match &result.content[0] {
            ToolContent::Text { text } => assert_eq!(text, "42"),
            other => panic!("expected Text content, got {:?}", other),
        }
        assert!(!result.is_error);
    }

    #[tokio::test]
    async fn websocket_notification_received() {
        use futures::{SinkExt, StreamExt};
        use tokio::net::TcpListener;
        use tokio_tungstenite::tungstenite::Message;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let (trigger_tx, mut trigger_rx) = tokio::sync::mpsc::channel::<()>(1);

        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let ws_stream = accept_ws_with_mcp_subprotocol(stream).await;
            let (write, mut read) = ws_stream.split();
            let write = Arc::new(tokio::sync::Mutex::new(write));

            let write_clone = write.clone();
            tokio::spawn(async move {
                let _ = trigger_rx.recv().await;
                tokio::time::sleep(Duration::from_millis(50)).await;
                let notif = serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": "notifications/tools/list_changed",
                    "params": {}
                });
                let _ = write_clone.lock().await
                    .send(Message::Text(serde_json::to_string(&notif).unwrap().into()))
                    .await;
            });

            while let Some(Ok(msg)) = read.next().await {
                if let Message::Text(text) = msg {
                    let v: serde_json::Value = match serde_json::from_str(&text) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    let method = v.get("method").and_then(|m| m.as_str()).unwrap_or("");
                    if method == "notifications/initialized" {
                        continue;
                    }

                    let req: JsonRpcRequest = match serde_json::from_value(v) {
                        Ok(r) => r,
                        Err(_) => continue,
                    };
                    let resp = match req.method.as_str() {
                        "initialize" => JsonRpcResponse::success(
                            req.id.clone(),
                            serde_json::to_value(InitializeResult {
                                protocol_version: "2025-06-18".into(),
                                capabilities: ServerCapabilities {
                                    tools: Some(ToolCapability { list_changed: true }),
                                    resources: None,
                                    prompts: None,
                                },
                                server_info: ServerInfo {
                                    name: "ws-notif-test".into(),
                                    version: "1.0.0".into(),
                                },
                                instructions: None,
                            })
                            .unwrap(),
                        ),
                        "tools/list" => JsonRpcResponse::success(
                            req.id.clone(),
                            serde_json::to_value(ToolListResult { tools: vec![] }).unwrap(),
                        ),
                        _ => JsonRpcResponse::error(req.id.clone(), -32601, "unknown"),
                    };
                    let resp_text = serde_json::to_string(&resp).unwrap();
                    let _ = write.lock().await.send(Message::Text(resp_text.into())).await;
                }
            }
        });

        tokio::time::sleep(Duration::from_millis(100)).await;

        let url = format!("ws://127.0.0.1:{}", addr.port());
        let client = McpClient::connect_websocket(&url).await.expect("WS connect");
        let mut rx = client.subscribe_notifications();

        let _ = trigger_tx.send(()).await;

        let notif = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("should receive notification within timeout")
            .expect("recv should succeed");
        assert_eq!(notif.method, "notifications/tools/list_changed");
    }
}
