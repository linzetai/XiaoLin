# Notification Dispatch + Stdio Reader 改造 — 详细技术方案

> **实现状态**：变更 1-4 已完成（T6 ✅），变更 5 已完成（T7 ✅），变更 6-7 待做（T19）。

## 现状分析

### 当前 stdio_reader_loop（第 699-739 行）

```rust
async fn stdio_reader_loop(
    mut reader: BufReader<ChildStdout>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<JsonRpcResponse>>>>,
) {
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => break,
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() { continue; }
                // 只尝试反序列化为 Response
                if let Ok(resp) = serde_json::from_str::<JsonRpcResponse>(trimmed) {
                    let key = json_rpc_id_key(&resp.id);
                    if let Some(tx) = pending.lock().await.remove(&key) {
                        let _ = tx.send(resp);
                    }
                }
                // notification → 静默丢弃！
            }
            Err(e) => { break; }
        }
    }
}
```

**三个问题**：

1. **Notification 丢弃** — `tools/list_changed`、`notifications/progress`、`notifications/message` 等全部被忽略
2. **无 stderr reader** — stderr 被 piped 但没人读，可能导致子进程阻塞
3. **SSE reader 同样** — `sse_reader_loop` 也只解析 Response，丢弃 notification

### JSON-RPC Message 类型判断

| 消息类型 | 有 `id` | 有 `method` | 有 `result`/`error` |
|----------|---------|-------------|---------------------|
| Request | ✅ | ✅ | ❌ |
| Response | ✅ | ❌ | ✅ |
| Notification | ❌ | ✅ | ❌ |

当前代码只检测 Response（有 `id`），Notification（无 `id`、有 `method`）被 `from_str::<JsonRpcResponse>` 失败后丢弃。

### 对比（更新于 2026-06-15 深度分析）

| | XiaoLin | Claude Code | Codex |
|---|---|---|---|
| Notification dispatch | ❌ 静默丢弃 | ✅ `setNotificationHandler` per capability | ✅ `ClientHandler` trait（但几乎不处理） |
| tools/list_changed | ❌ | ✅ 清缓存 + 重新 fetch + 更新 AppState | ⚠️ **仅 `info!` 日志，不刷新工具列表** |
| prompts/list_changed | ❌ | ✅ 刷新 commands + MCP skills | ⚠️ 仅 `info!` 日志 |
| resources/list_changed | ❌ | ✅ 刷新 resources + 联动 skills/commands 缓存 | ⚠️ 仅 `info!` 日志 |
| progress | ❌ | ✅ | ✅ logging |
| logging/message | ❌ | ✅ | ✅ 按级别转发 |
| stderr capture | ✅ 已完成（T7） | ✅ | ✅ |

### 关键洞察

1. **Codex 的 `tools/list_changed` 处理是一个已知缺口**：`codex-rs/rmcp-client/src/logging_client_handler.rs` 中 `on_tool_list_changed` 仅 `info!("MCP server tool list changed")`，不触发工具重新拉取。这意味着 **XiaoLin 做好 T6+T19 即在 Notification 处理上超越 Codex**。

2. **Claude Code 的完整链路**：`useManageMCPConnections.ts` L616-748 按 server capability 注册 handler，`tools/list_changed` 时：
   - `fetchToolsForClient.cache.delete(client.name)` 清缓存
   - `await fetchToolsForClient(client)` 重新 fetch
   - `updateServer({ ...client, tools: newTools })` 更新 AppState
   - 前端自动刷新工具列表

3. **Claude Code 连续错误触发重连**：`MAX_ERRORS_BEFORE_RECONNECT = 3`，检测 ECONNRESET/ETIMEDOUT，3 次连续错误后完整重连。XiaoLin 可参考。

4. **Codex 的 `cancelled` notification**：`on_cancelled` 也只是 `warn!` 日志，未取消对应 pending request。

## 设计方案

### 变更 1: Notification Channel ✅ 已实现

在 `McpClient` 和 `McpTransport` 中增加 notification channel：

```rust
use tokio::sync::broadcast;

pub struct McpClient {
    server_name: String,
    tools: Vec<McpTool>,
    transport: McpTransport,
    next_id: AtomicU64,
    notification_tx: broadcast::Sender<McpNotification>,  // 新增
}

/// MCP server → client notification
#[derive(Debug, Clone)]
pub struct McpNotification {
    pub method: String,
    pub params: Option<serde_json::Value>,
}
```

`broadcast` channel 允许多个订阅者（gateway 可以订阅处理 `tools/list_changed`，
前端可以订阅显示 progress）。

### 变更 2: stdio_reader_loop 改造 ✅ 已实现

```rust
async fn stdio_reader_loop(
    mut reader: BufReader<ChildStdout>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<JsonRpcResponse>>>>,
    notification_tx: broadcast::Sender<McpNotification>,  // 新增
) {
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => break,
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() { continue; }

                // 先尝试解析为通用 JSON 判断类型
                let value: serde_json::Value = match serde_json::from_str(trimmed) {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!("MCP: unparseable line: {e}");
                        continue;
                    }
                };

                if value.get("id").is_some() {
                    // Response（有 id）
                    if let Ok(resp) = serde_json::from_value::<JsonRpcResponse>(value) {
                        let key = json_rpc_id_key(&resp.id);
                        if let Some(tx) = pending.lock().await.remove(&key) {
                            let _ = tx.send(resp);
                        }
                    }
                } else if let Some(method) = value.get("method").and_then(|m| m.as_str()) {
                    // Notification（有 method、无 id）
                    let params = value.get("params").cloned();
                    let _ = notification_tx.send(McpNotification {
                        method: method.to_string(),
                        params,
                    });
                    tracing::debug!(method, "MCP notification received");
                }
            }
            Err(e) => {
                tracing::warn!("MCP stdio reader error: {e}");
                break;
            }
        }
    }

    // 连接断开，通知所有 pending 请求
    let mut guard = pending.lock().await;
    for (_, tx) in guard.drain() {
        let _ = tx.send(JsonRpcResponse::error(
            serde_json::Value::Null,
            -32603,
            "MCP subprocess exited",
        ));
    }
}
```

### 变更 3: SSE reader 同步改造 ✅ 已实现

`sse_reader_loop` 同样加入 notification dispatch：

```rust
for data in extract_sse_data_lines(&event_block) {
    let value: serde_json::Value = match serde_json::from_str(&data) {
        Ok(v) => v,
        Err(_) => continue,
    };

    if value.get("id").is_some() {
        if let Ok(resp) = serde_json::from_value::<JsonRpcResponse>(value) {
            let key = json_rpc_id_key(&resp.id);
            if let Some(tx) = pending.lock().await.remove(&key) {
                let _ = tx.send(resp);
            }
        }
    } else if let Some(method) = value.get("method").and_then(|m| m.as_str()) {
        let params = value.get("params").cloned();
        let _ = notification_tx.send(McpNotification {
            method: method.to_string(),
            params,
        });
    }
}
```

### 变更 4: McpClient 公开 notification 订阅 ✅ 已实现

```rust
impl McpClient {
    /// Subscribe to server notifications.
    pub fn subscribe_notifications(&self) -> broadcast::Receiver<McpNotification> {
        self.notification_tx.subscribe()
    }
}
```

### 变更 5: stderr capture ✅ 已完成

> T7 已实现。`stderr_reader_loop` 已在 `xiaolin-mcp/src/lib.rs` 中。

在 `connect_stdio` 中添加 stderr reader（参考实现）：

```rust
let stderr = process.stderr.take()
    .ok_or_else(|| anyhow::anyhow!("failed to open stderr"))?;

let server_name_for_stderr = command.to_string();
tokio::spawn(async move {
    let mut reader = tokio::io::BufReader::new(stderr);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => break,
            Ok(_) => {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    tracing::debug!(
                        target: "mcp_stderr",
                        server = %server_name_for_stderr,
                        "{trimmed}"
                    );
                }
            }
            Err(_) => break,
        }
    }
});
```

### 变更 6: Gateway 订阅 tools/list_changed

在 `register_mcp_tools` 返回 `SharedMcpClient` 后，gateway 订阅 notification：

```rust
// state/mod.rs — 连接 MCP server 后

// prefix 由 connect_mcp_server 内部派生（naming::mcp_server_prefix(&cfg.id)）
let client = connect_mcp_server(cfg, &registry, /* deferred */ false).await?;

// 订阅 tools/list_changed
let mut rx = client.subscribe_notifications();
let registry_clone = tool_registry.clone();
let prefix_clone = prefix.clone();
let client_clone = client.clone();

tokio::spawn(async move {
    while let Ok(notif) = rx.recv().await {
        match notif.method.as_str() {
            "notifications/tools/list_changed" => {
                tracing::info!(
                    server = %prefix_clone,
                    "tools/list_changed received, refreshing tools"
                );
                // 重新 tools/list → 更新 registry
                let tools = client_clone.list_tools().await;
                match tools {
                    Ok(new_tools) => {
                        let count = new_tools.len();
                        registry_clone.unregister_by_prefix(&prefix_clone);
                        for tool in &new_tools {
                            let bridge = McpToolBridge::new(tool, client_clone.clone(), &prefix_clone);
                            registry_clone.register(Arc::new(bridge));
                        }
                        tracing::info!(
                            server = %prefix_clone,
                            count,
                            "refreshed MCP tools after list_changed"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            server = %prefix_clone,
                            error = %e,
                            "failed to refresh tools after list_changed"
                        );
                    }
                }
            }
            "notifications/message" => {
                if let Some(params) = &notif.params {
                    let level = params.get("level").and_then(|l| l.as_str()).unwrap_or("info");
                    let data = params.get("data");
                    tracing::info!(
                        target: "mcp_server_log",
                        server = %prefix_clone,
                        level,
                        ?data,
                        "MCP server log"
                    );
                }
            }
            _ => {
                tracing::debug!(
                    server = %prefix_clone,
                    method = %notif.method,
                    "unhandled MCP notification"
                );
            }
        }
    }
});
```

### 变更 7: McpClient 增加 list_tools 方法 ✅ 已实现（命名为 `refresh_tools`）

当前 `tools()` 返回缓存，需要增加强制刷新版本：

```rust
impl McpClient {
    /// Re-fetch tools from the server (for handling tools/list_changed).
    pub async fn list_tools(&self) -> anyhow::Result<Vec<McpTool>> {
        let resp = self.send_request("tools/list", None).await?;
        let result: ToolListResult = serde_json::from_value(
            resp.result.ok_or_else(|| anyhow::anyhow!("no result"))?
        )?;
        Ok(result.tools)
    }
}
```

## 数据流

```
MCP Server Process
  ├── stdout ──→ stdio_reader_loop
  │                ├── has "id"? → Response → pending.remove(id).send(resp)
  │                └── has "method"? → Notification → broadcast::send(McpNotification)
  │                                      │
  │                                      ├── "notifications/tools/list_changed"
  │                                      │     → tools/list → unregister + re-register
  │                                      │
  │                                      ├── "notifications/message"
  │                                      │     → tracing::info (mcp_server_log target)
  │                                      │
  │                                      └── other → tracing::debug
  │
  └── stderr ──→ stderr_reader (spawn) → tracing::debug (mcp_stderr target)
```

## MCP Notification 清单

| Notification | 优先级 | 处理方式 |
|---|---|---|
| `notifications/tools/list_changed` | P1 (关键) | 重新 tools/list + 更新 registry |
| `notifications/message` | P1 | 按 level 转发到 tracing |
| `notifications/progress` | P2 | 可推送到前端显示进度 |
| `notifications/resources/list_changed` | P3 | XiaoLin 暂不使用 resources |
| `notifications/resources/updated` | P3 | 同上 |
| `notifications/prompts/list_changed` | P3 | XiaoLin 暂不使用 prompts |
| `notifications/cancelled` | P2 | 取消对应 pending request |

## 影响的文件

| 文件 | 变更 | 对应任务 |
|------|------|---------|
| `xiaolin-mcp/src/lib.rs` | `McpNotification` 结构体；`McpClient` 增加 `notification_tx` + `subscribe_notifications` + `list_tools`；`stdio_reader_loop` 增加 notification 分发；`sse_reader_loop` 同步改造 | T6 |
| `xiaolin-mcp/src/lib.rs` | `connect_stdio` 增加 stderr reader | T7 ✅ 已完成 |
| `xiaolin-gateway/src/state/mod.rs` | 连接后订阅 notification，处理 `tools/list_changed`（变更 6） | T19 |

## 依赖关系

此变更是多个其他功能的**前置条件**：

```
notification dispatch (本变更)
  ├── tools/list_changed 处理 (D6)
  ├── progress 前端展示 (P2)
  ├── logging/message 转发 (P2)
  └── cancelled notification 处理 (P2)
```

## 测试计划

1. **单测**：收到 JSON-RPC notification（无 id、有 method）→ broadcast 发送
2. **单测**：收到 JSON-RPC response（有 id）→ pending 路由不变
3. **单测**：收到无效 JSON → warn 日志，不 panic
4. **单测**：subscribe_notifications 多订阅者独立接收
5. **集成测试**：mock MCP server 发送 `tools/list_changed` → registry 更新
6. **集成测试**：stderr 输出被捕获到 tracing
7. **集成测试**：子进程退出后 pending 请求收到错误

## broadcast vs mpsc 选择理由

使用 `tokio::sync::broadcast` 而非 `mpsc`：

- **多订阅者**：gateway + 前端 progress 推送 + 未来扩展可各自订阅
- **无阻塞**：发送方不等待接收方（`send` 是非阻塞的）
- **丢弃容忍**：接收方慢了可以丢弃旧消息（notification 天然幂等）
- Channel 容量建议 64：MCP notification 频率低，不需要大 buffer
