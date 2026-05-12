use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// MCP protocol types following the Model Context Protocol specification.
///
/// FastClaw supports MCP as both server (exposing tools to external agents)
/// and client (consuming tools from external MCP servers).

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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpTool {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<serde_json::Value>,
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
pub struct McpResource {
    pub uri: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
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

type McpResourceFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<serde_json::Value>> + Send>>;
type McpPromptFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<serde_json::Value>> + Send>>;

type ToolHandlerFn = Box<dyn Fn(&serde_json::Value) -> McpToolFuture + Send + Sync>;
type ResourceHandlerFn = Box<dyn Fn(&serde_json::Value) -> McpResourceFuture + Send + Sync>;
type PromptHandlerFn = Box<dyn Fn(&serde_json::Value) -> McpPromptFuture + Send + Sync>;

/// An MCP server that exposes FastClaw's tools, resources, and prompts over JSON-RPC 2.0.
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
            protocol_version: "2024-11-05".into(),
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

enum McpTransport {
    Stdio {
        process: Box<tokio::process::Child>,
        stdin: tokio::process::ChildStdin,
        reader: tokio::io::BufReader<tokio::process::ChildStdout>,
    },
    Sse {
        client: reqwest::Client,
        post_url: String,
        pending:
            Arc<tokio::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<JsonRpcResponse>>>>,
        reader_task: tokio::task::JoinHandle<anyhow::Result<()>>,
    },
}

/// A client that connects to an external MCP server and discovers/invokes its tools.
pub struct McpClient {
    server_name: String,
    tools: Vec<McpTool>,
    transport: McpTransport,
    next_id: std::sync::atomic::AtomicU64,
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

        let mut client = Self {
            server_name: command.to_string(),
            tools: Vec::new(),
            transport: McpTransport::Stdio {
                process: Box::new(process),
                stdin,
                reader,
            },
            next_id: std::sync::atomic::AtomicU64::new(1),
        };

        client.initialize().await?;
        client.discover_tools().await?;

        Ok(client)
    }

    /// Connect via HTTP SSE (GET stream) and JSON-RPC POST to the companion `/message` path.
    ///
    /// `url` must be the full SSE URL whose path ends with `/sse`; POST target is the same path
    /// with the `/sse` suffix replaced by `/message`.
    pub async fn connect_sse(url: &str) -> anyhow::Result<Self> {
        let sse_url = reqwest::Url::parse(url.trim())?;
        let post_url = companion_post_url(&sse_url)?.to_string();
        let sse_url_str = sse_url.to_string();

        let client = reqwest::Client::new();
        let pending: Arc<
            tokio::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<JsonRpcResponse>>>,
        > = Arc::new(tokio::sync::Mutex::new(HashMap::new()));

        let pending_reader = pending.clone();
        let client_reader = client.clone();
        let sse_reader_url = sse_url_str.clone();
        let reader_task = tokio::spawn(async move {
            Self::sse_reader_loop(&client_reader, &sse_reader_url, pending_reader).await
        });

        let mut mcp = Self {
            server_name: sse_url_str,
            tools: Vec::new(),
            transport: McpTransport::Sse {
                client: client.clone(),
                post_url,
                pending: pending.clone(),
                reader_task,
            },
            next_id: std::sync::atomic::AtomicU64::new(1),
        };

        tokio::task::yield_now().await;
        tokio::task::yield_now().await;

        mcp.initialize().await?;
        mcp.discover_tools().await?;

        Ok(mcp)
    }

    async fn sse_reader_loop(
        client: &reqwest::Client,
        sse_url: &str,
        pending: Arc<
            tokio::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<JsonRpcResponse>>>,
        >,
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
                    if let Ok(resp) = serde_json::from_str::<JsonRpcResponse>(&data) {
                        let key = json_rpc_id_key(&resp.id);
                        if let Some(tx) = pending.lock().await.remove(&key) {
                            let _ = tx.send(resp);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    async fn send_request(
        &mut self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> anyhow::Result<JsonRpcResponse> {
        use std::sync::atomic::Ordering;
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let id_val = serde_json::Value::Number(id.into());
        let request = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: id_val.clone(),
            method: method.into(),
            params,
        };

        match &mut self.transport {
            McpTransport::Stdio {
                stdin,
                reader,
                process,
                ..
            } => {
                let json = serde_json::to_string(&request)?;
                stdin.write_all(json.as_bytes()).await?;
                stdin.write_all(b"\n").await?;
                stdin.flush().await?;

                let mut line = String::new();
                let n = reader.read_line(&mut line).await?;
                if n == 0 {
                    let stderr_msg = if let Some(mut stderr) = process.stderr.take() {
                        use tokio::io::AsyncReadExt;
                        let mut buf = vec![0u8; 4096];
                        let bytes_read = stderr.read(&mut buf).await.unwrap_or(0);
                        String::from_utf8_lossy(&buf[..bytes_read]).to_string()
                    } else {
                        String::new()
                    };
                    anyhow::bail!(
                        "MCP subprocess produced no output for method '{}' (process may have crashed). stderr: {}",
                        method,
                        if stderr_msg.is_empty() { "(empty)" } else { stderr_msg.trim() }
                    );
                }
                serde_json::from_str(&line).map_err(|e| {
                    anyhow::anyhow!(
                        "MCP server response is not valid JSON for method '{}': {e}. Raw line: {:?}",
                        method,
                        line.chars().take(200).collect::<String>()
                    )
                })
            }
            McpTransport::Sse {
                client,
                post_url,
                pending,
                ..
            } => {
                let id_key = json_rpc_id_key(&id_val);
                let (tx, rx) = tokio::sync::oneshot::channel();
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

                match tokio::time::timeout(Duration::from_secs(60), rx).await {
                    Ok(Ok(resp)) => Ok(resp),
                    Ok(Err(_)) => {
                        let _ = pending.lock().await.remove(&id_key);
                        anyhow::bail!("MCP SSE reply channel closed")
                    }
                    Err(_) => {
                        let _ = pending.lock().await.remove(&id_key);
                        anyhow::bail!("MCP SSE response timed out")
                    }
                }
            }
        }
    }

    async fn initialize(&mut self) -> anyhow::Result<()> {
        let params = serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "FastClaw",
                "version": env!("CARGO_PKG_VERSION"),
            }
        });

        let response = self.send_request("initialize", Some(params)).await?;
        if let Some(error) = response.error {
            anyhow::bail!("MCP initialize failed: {}", error.message);
        }

        if let Some(result) = response.result {
            let info: InitializeResult = serde_json::from_value(result)?;
            self.server_name = info.server_info.name;
            tracing::info!(server = %self.server_name, version = %info.server_info.version, "MCP server connected");
        }

        let notification = JsonRpcNotification {
            jsonrpc: "2.0".into(),
            method: "notifications/initialized".into(),
            params: None,
        };
        match &mut self.transport {
            McpTransport::Stdio { stdin, .. } => {
                use tokio::io::AsyncWriteExt;
                let json = serde_json::to_string(&notification)?;
                stdin.write_all(json.as_bytes()).await?;
                stdin.write_all(b"\n").await?;
                stdin.flush().await?;
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
        }

        Ok(())
    }

    async fn discover_tools(&mut self) -> anyhow::Result<()> {
        let response = self.send_request("tools/list", None).await?;
        if let Some(result) = response.result {
            let tool_list: ToolListResult = serde_json::from_value(result)?;
            tracing::info!(count = tool_list.tools.len(), server = %self.server_name, "discovered MCP tools");
            self.tools = tool_list.tools;
        }
        Ok(())
    }

    /// Get the list of discovered tools.
    pub fn tools(&self) -> &[McpTool] {
        &self.tools
    }

    /// Call a tool on the remote MCP server.
    pub async fn call_tool(
        &mut self,
        name: &str,
        arguments: serde_json::Value,
    ) -> anyhow::Result<CallToolResult> {
        let params = serde_json::json!({
            "name": name,
            "arguments": arguments,
        });

        let response = self.send_request("tools/call", Some(params)).await?;
        if let Some(error) = response.error {
            anyhow::bail!("tool call failed: {}", error.message);
        }

        match response.result {
            Some(result) => Ok(serde_json::from_value(result)?),
            None => anyhow::bail!("empty result from tool call"),
        }
    }

    /// Get the server name.
    pub fn server_name(&self) -> &str {
        &self.server_name
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        match &mut self.transport {
            McpTransport::Stdio { process, .. } => {
                let _ = process.start_kill();
            }
            McpTransport::Sse { reader_task, .. } => {
                reader_task.abort();
            }
        }
    }
}

// --- Bridge: Expose FastClaw tools as MCP ---

/// Create an MCP server pre-populated with FastClaw's built-in tools.
pub fn create_fastclaw_mcp_server(
    tool_registry: &Arc<fastclaw_core::tool::ToolRegistry>,
) -> McpServer {
    let mut server = McpServer::new("FastClaw", env!("CARGO_PKG_VERSION"));

    for def in tool_registry.definitions().iter() {
        let tool_name = def.function.name.clone();
        let mcp_tool = McpTool {
            name: tool_name.clone(),
            description: Some(def.function.description.clone()),
            input_schema: Some(serde_json::to_value(&def.function.parameters).unwrap_or_default()),
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

// --- MCP Tool Bridge: expose remote MCP tools as FastClaw Tools ---

use tokio::sync::Mutex;

/// Wraps an McpClient behind a Mutex so it can be shared across tool instances.
pub type SharedMcpClient = Arc<Mutex<McpClient>>;

/// A FastClaw `Tool` that delegates execution to a remote MCP server via `McpClient`.
pub struct McpToolBridge {
    tool_name: String,
    description: String,
    schema: serde_json::Value,
    client: SharedMcpClient,
    server_prefix: String,
}

impl McpToolBridge {
    fn new(mcp_tool: &McpTool, client: SharedMcpClient, server_prefix: &str) -> Self {
        Self {
            tool_name: format!("{server_prefix}{}", mcp_tool.name),
            description: mcp_tool.description.clone().unwrap_or_default(),
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
impl fastclaw_core::tool::Tool for McpToolBridge {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> fastclaw_core::tool::ToolParameterSchema {
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
        fastclaw_core::tool::ToolParameterSchema {
            schema_type: "object".into(),
            properties,
            required,
        }
    }

    async fn execute(&self, arguments: &str) -> fastclaw_core::tool::ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => {
                return fastclaw_core::tool::ToolResult::err(format!("invalid JSON arguments: {e}"))
            }
        };

        let original_name = self
            .tool_name
            .strip_prefix(&self.server_prefix)
            .unwrap_or(&self.tool_name);

        let call_result = {
            let mut client = self.client.lock().await;
            client.call_tool(original_name, args).await
        };

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
                    fastclaw_core::tool::ToolResult::err(text)
                } else {
                    fastclaw_core::tool::ToolResult::ok(text)
                }
            }
            Err(e) => fastclaw_core::tool::ToolResult::err(format!("MCP call failed: {e}")),
        }
    }
}

/// Connect to an MCP server and register all its tools into a FastClaw ToolRegistry.
/// Returns the shared McpClient handle so it can be managed/closed later.
///
/// Tools are registered with a `server_prefix` to avoid name collisions (e.g. `"mcp_myserver_"`).
pub async fn register_mcp_tools<S: std::hash::BuildHasher>(
    command: &str,
    args: &[&str],
    registry: &fastclaw_core::tool::ToolRegistry,
    server_prefix: &str,
    extra_env: &std::collections::HashMap<String, String, S>,
) -> anyhow::Result<SharedMcpClient> {
    let client = McpClient::connect_stdio(command, args, extra_env).await?;
    let tools = client.tools().to_vec();
    let shared = Arc::new(Mutex::new(client));

    let mut registered = 0usize;
    let mut seen = std::collections::HashSet::new();
    for tool in &tools {
        let prefixed = format!("{server_prefix}{}", tool.name);
        if !seen.insert(prefixed.clone()) {
            tracing::warn!(tool = %prefixed, "skipping duplicate MCP tool within same server");
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
        "registered MCP tools into FastClaw"
    );
    Ok(shared)
}

/// Register tools from an MCP server reachable via HTTP SSE.
///
/// `url` is the SSE endpoint (e.g. `http://host:port/sse`).
pub async fn register_mcp_tools_sse(
    url: &str,
    registry: &fastclaw_core::tool::ToolRegistry,
    server_prefix: &str,
) -> anyhow::Result<SharedMcpClient> {
    let client = McpClient::connect_sse(url).await?;
    let tools = client.tools().to_vec();
    let shared = Arc::new(Mutex::new(client));

    let mut registered = 0usize;
    let mut seen = std::collections::HashSet::new();
    for tool in &tools {
        let prefixed = format!("{server_prefix}{}", tool.name);
        if !seen.insert(prefixed.clone()) {
            tracing::warn!(tool = %prefixed, "skipping duplicate MCP tool within same server");
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
        "registered MCP tools (SSE) into FastClaw"
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
        assert_eq!(result["protocolVersion"], "2024-11-05");
    }

    #[tokio::test]
    async fn mcp_server_tools_list() {
        let mut server = McpServer::new("test", "1.0");
        server.tools.push(McpTool {
            name: "echo".into(),
            description: Some("echo input".into()),
            input_schema: None,
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
                "arguments": { "name": "FastClaw" },
            })),
        };
        let resp = server.handle_request(&req).await;
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["content"][0]["text"], "Hello, FastClaw!");
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
        let client = McpClient::connect_sse(&url).await.expect("sse connect");
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
        let client = McpClient::connect_sse(&url)
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
        let Err(err) = McpClient::connect_sse(&url).await else {
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
        let Err(err) = McpClient::connect_sse(url).await else {
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
}
