# SSE/HTTP 传输修复 — 详细技术方案

## 现状分析

### Bug：两条路径行为不一致

XiaoLin 有**三条** MCP 连接路径，transport 处理各不相同：

| 路径 | 文件 | transport 路由 | 现状 |
|------|------|---------------|------|
| 启动 | `state/mod.rs:915` | ❌ 始终 stdio | **BUG** |
| 热重载 `reload_mcp_servers` | `state/mod.rs:378` | ❌ 始终 stdio | **BUG** |
| ManageMcpServerTool `do_reload` | `mcp_tool.rs:259` | ✅ 有 transport 判断 | OK |

SSE 配置在启动和热重载时**静默失败**（stdio 连接因无 command 而报错，但错误消息不提示 transport 问题）。

### 当前代码

**有路由的（mcp_tool.rs:259）**：
```rust
let connect_result = if cfg.transport == "sse" {
    let url = cfg.url.as_deref().unwrap_or("");
    xiaolin_mcp::register_mcp_tools_sse(url, &self.tool_registry, &prefix).await
} else {
    // ... stdio ...
};
```

**无路由的（state/mod.rs:915）**：
```rust
let result = xiaolin_mcp::register_mcp_tools(
    &command, &args_ref, &registry, &prefix, &env,
).await;
// 不检查 transport，始终 stdio
```

### Transport 类型支持对比

| Transport | XiaoLin (client) | XiaoLin (gateway) | Claude Code | Codex |
|-----------|-----------------|-------------------|-------------|-------|
| stdio | ✅ `connect_stdio` | ✅ 全路径 | ✅ | ✅ |
| SSE | ✅ `connect_sse` | ⚠️ 仅 mcp_tool.rs | ✅ | ❌ |
| Streamable HTTP | ❌ | ❌ | ✅ | ✅ |
| WebSocket | ❌ | ❌ | ✅ (IDE only) | ❌ |

### `McpServerConfig` 的 transport 字段

```rust
pub struct McpServerConfig {
    pub transport: String,  // "stdio" (default) or "sse"
    pub url: Option<String>,  // SSE URL
    // ... command, args (stdio) ...
}
```

`transport` 是 `String`，没有枚举约束，也没有验证逻辑。

## 设计方案

### 变更 1: Transport 枚举化

```rust
// xiaolin-core/src/agent_config.rs

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum McpTransportType {
    #[default]
    Stdio,
    Sse,
    // 预留
    // Http,  // Streamable HTTP (P2)
}

pub struct McpServerConfig {
    pub id: String,
    pub command: String,
    pub args: Vec<String>,
    pub enabled: Option<bool>,
    pub env: HashMap<String, String>,
    pub url: Option<String>,
    #[serde(default)]
    pub transport: McpTransportType,  // 从 String 改为枚举
}
```

**向后兼容**：`#[serde(rename_all = "lowercase")]` 确保 JSON 中 `"stdio"` / `"sse"` 仍能正确反序列化。未知值会报错（而非静默默认 stdio），这是**期望行为**——提前暴露配置错误。

### 变更 2: 抽取共用 `connect_mcp_server` 函数

这是 D7（统一热重载）和 D3（传输修复）的交汇点。

```rust
// xiaolin-mcp/src/lib.rs

/// Connect to an MCP server based on transport type and register its tools.
pub async fn connect_mcp_server(
    cfg: &McpServerConfig,
    registry: &ToolRegistry,
    prefix: &str,
    deferred: bool,
) -> anyhow::Result<SharedMcpClient> {
    match cfg.transport {
        McpTransportType::Stdio => {
            let args_ref: Vec<&str> = cfg.args.iter().map(|s| s.as_str()).collect();
            register_mcp_tools_with_options(
                &cfg.command, &args_ref, registry, prefix, &cfg.env, deferred,
            ).await
        }
        McpTransportType::Sse => {
            let url = cfg.url.as_deref()
                .ok_or_else(|| anyhow::anyhow!(
                    "MCP server '{}' configured as SSE but missing 'url' field", cfg.id
                ))?;
            register_mcp_tools_sse_with_options(
                url, registry, prefix, deferred,
            ).await
        }
    }
}
```

### 变更 3: 启动路径修复

```rust
// state/mod.rs — register_mcp_and_subagent_tools

let futs: Vec<_> = to_connect
    .iter()
    .map(|(mcp_cfg, scope)| {
        let cfg = (*mcp_cfg).clone();
        let prefix = naming::mcp_server_prefix(&cfg.id);
        let scope = scope.to_string();
        let registry = tool_registry.clone();
        async move {
            let id = cfg.id.clone();
            // 使用统一函数，自动路由 transport
            let result = xiaolin_mcp::connect_mcp_server(
                &cfg, &registry, &prefix, false,
            ).await;
            (id, scope, result)
        }
    })
    .collect();
```

### 变更 4: 热重载路径修复

```rust
// state/mod.rs — reload_mcp_servers

// Before:
match xiaolin_mcp::register_mcp_tools(&cfg.command, ...).await {

// After:
match xiaolin_mcp::connect_mcp_server(cfg, &self.rt.tool_registry, &prefix, false).await {
```

### 变更 5: 删除 mcp_tool.rs 中的重复逻辑

`mcp_tool.rs` 的 `do_reload` 已有 transport 路由，改为调用 `connect_mcp_server`：

```rust
// Before (mcp_tool.rs:259-272):
let connect_result = if cfg.transport == "sse" {
    let url = cfg.url.as_deref().unwrap_or("");
    xiaolin_mcp::register_mcp_tools_sse(url, &self.tool_registry, &prefix).await
} else {
    let args_ref: Vec<&str> = cfg.args.iter().map(|s| s.as_str()).collect();
    xiaolin_mcp::register_mcp_tools(&cfg.command, &args_ref, ...).await
};

// After:
let connect_result = xiaolin_mcp::connect_mcp_server(
    cfg, &self.tool_registry, &prefix, false,
).await;
```

### 变更 6: 配置验证

在连接前验证配置完整性：

```rust
impl McpServerConfig {
    pub fn validate(&self) -> Result<(), String> {
        match self.transport {
            McpTransportType::Stdio => {
                if self.command.is_empty() {
                    return Err(format!(
                        "MCP server '{}': stdio transport requires 'command'", self.id
                    ));
                }
            }
            McpTransportType::Sse => {
                if self.url.is_none() || self.url.as_ref().map_or(true, |u| u.is_empty()) {
                    return Err(format!(
                        "MCP server '{}': SSE transport requires 'url'", self.id
                    ));
                }
            }
        }
        if self.id.is_empty() {
            return Err("MCP server id cannot be empty".to_string());
        }
        if self.id.contains("__") {
            return Err(format!(
                "MCP server id '{}' cannot contain '__' (reserved delimiter)", self.id
            ));
        }
        Ok(())
    }
}
```

## Streamable HTTP 路线图（P2）

MCP spec 2025-06-18 推荐 Streamable HTTP 替代 SSE。Codex 已实现，Claude Code 也支持。

### 预留设计

```rust
pub enum McpTransportType {
    Stdio,
    Sse,
    Http,  // Streamable HTTP — P2 实现
}
```

Streamable HTTP 与 SSE 的区别：

| | SSE | Streamable HTTP |
|---|---|---|
| 连接方式 | GET 长连接 + POST 请求 | 仅 POST（可选 SSE 升级） |
| Session ID | 通过 SSE 事件获取 | `Mcp-Session-Id` header |
| 无状态 | 需要持续连接 | 可无状态 |
| 恢复 | 不支持 | `Last-Event-ID` 恢复 |
| 双向 | SSE → client / POST → server | POST 可双向 |

P2 实现时需要：
1. `McpClient::connect_http(url, headers)` 方法
2. Session 管理（header 提取 + 回传）
3. `McpServerConfig` 增加 `headers` 字段（bearer token 等）

### 配置示例（P2）

```json
{
  "mcpServers": {
    "remote-api": {
      "transport": "http",
      "url": "https://api.example.com/mcp",
      "headers": {
        "Authorization": "Bearer ${API_TOKEN}"
      }
    }
  }
}
```

## 数据流

```
McpServerConfig
  ├── transport: Stdio
  │     ├── command: "npx"
  │     └── args: ["-y", "@mcp/server"]
  │           ↓
  │     connect_mcp_server → McpClient::connect_stdio
  │           ↓
  │     stdin/stdout JSON-RPC
  │
  └── transport: Sse
        ├── url: "http://localhost:3000/sse"
        │     ↓
        │ connect_mcp_server → McpClient::connect_sse
        │     ↓
        │ GET /sse (SSE stream) + POST (requests)
        │
        └── (P2) transport: Http
              ├── url: "https://api.example.com/mcp"
              │     ↓
              │ connect_mcp_server → McpClient::connect_http
              │     ↓
              │ POST /mcp (JSON-RPC + optional SSE upgrade)
```

## 影响的文件

| 文件 | 变更 |
|------|------|
| `xiaolin-core/src/agent_config.rs` | `McpTransportType` 枚举；`McpServerConfig.transport` 改为枚举；`validate()` 方法 |
| `xiaolin-mcp/src/lib.rs` | 新增 `connect_mcp_server` 统一入口 |
| `xiaolin-gateway/src/state/mod.rs` | 启动 + 热重载路径改用 `connect_mcp_server` |
| `xiaolin-gateway/src/mcp_tool.rs` | `do_reload` 改用 `connect_mcp_server` |
| `xiaolin-gateway/src/state/builder.rs` | `phase4_channels_mcp` 改用统一入口 |

## 测试计划

1. **单测**：`McpTransportType::Stdio` serde roundtrip（`"stdio"` ↔ `Stdio`）
2. **单测**：`McpTransportType::Sse` serde roundtrip
3. **单测**：未知 transport 字符串反序列化失败（而非默认 stdio）
4. **单测**：`validate()` — stdio 无 command → 错误
5. **单测**：`validate()` — sse 无 url → 错误
6. **单测**：`validate()` — id 包含 `__` → 错误
7. **集成测试**：启动时 SSE 配置走 `connect_sse` 路径
8. **集成测试**：热重载时 SSE 配置走 `connect_sse` 路径
9. **E2E**：添加 SSE MCP server → 连接成功
