## Context

Phase 1 (`mcp-capability-upgrade`) 完成了 100 个任务：传输统一、命名管线、notification dispatch、deferred loading、OAuth PKCE、Resources/Prompts/Elicitation 客户端、Plugin Panel 全功能 UI。

源码审计发现的核心问题：
1. `dispatcher.rs:134` 的 `runtime_registry.has()` 判断导致所有 MCP 工具走 `execute_unguarded` — 完全绕过审批流程
2. `notifications/progress` 在 notification watcher 中没有 match arm
3. `resources/list_changed` 仅 log，`prompts/list_changed` 仅发 WS 事件但不刷新服务端缓存
4. `oauth.rs` 已解析 `registration_endpoint` 但 DCR 流程未实现
5. Token 存储为明文 JSON（`~/.local/share/com.xiaolin.desktop/mcp-tokens/*.json`）
6. `McpServer` 代码存在但无生产入口（`run_stdio` 是死代码）
7. 无 WebSocket MCP 传输（只有应用层 WS）

## Goals / Non-Goals

**Goals:**
- 修复 MCP 工具审批缺口，使 `tools_ask` 配置生效
- 补全三类 `list_changed` + `progress` 通知的完整处理链路
- 实现 OAuth DCR，提升与标准 MCP 服务器的兼容性
- Token 迁移到加密存储
- 上线反向 MCP 服务器，让 XiaoLin 可被外部 AI host 调用
- 新增 WebSocket MCP 传输，覆盖更多服务器类型

**Non-Goals:**
- 企业托管 MCP（`managed-mcp.json` 独占模式）— 当前用户规模不需要
- CIMD (SEP-991 `clientMetadataUrl`) — 仅 Claude Code 使用
- XAA 跨应用认证 — 无应用生态
- MCP sampling/subscribe — 三家都未实现
- 多层配置 stack（7-8 层）— 2 层足够
- per-tool `approval_mode` 细粒度配置（Codex 模式）— glob 已够用

## Decisions

### D1: MCP 工具审批 — 通过 `McpToolBridge` 注入审批检查，而非修改 dispatcher 路由

**备选方案:**
- A: 将 MCP 工具注册到 `RuntimeRegistry` → 需要为每个 MCP 工具实现 `ToolRuntime` trait，工作量大且工具动态变化
- B: 在 `execute_unguarded` 中添加 MCP 特殊检查 → 破坏 unguarded 语义
- **C (选定): 在 `ToolDispatcher::dispatch` 中，对 `is_mcp_tool` 的工具单独走 `requires_confirmation` + 审批流程** → 最小改动，复用现有 `ApprovalStrategy` + `approval_cache`

**实现**: 在 `dispatcher.rs:132-138` 之间增加 MCP 工具判断分支：
```rust
let result = if self.runtime_registry.has(&tool_name) {
    self.execute_guarded(&effective_tc, ctx).await
} else if naming::is_mcp_tool(&tool_name) && self.requires_confirmation(&tool_name) {
    self.execute_mcp_with_approval(&effective_tc, ctx).await
} else {
    self.execute_unguarded(&effective_tc, ctx).await
};
```

新方法 `execute_mcp_with_approval` 复用 `OrchestrationDecision` 流程但跳过 `ToolRuntime::execute`，直接调用 `ToolRegistry::call`。

### D2: Progress 通知 — 转发到前端 WS 事件

MCP `notifications/progress` 包含 `progressToken` + `progress` + `total` + `message`。

方案：在 notification watcher 增加 match arm，通过 `ws_broadcast` 发送 `plugins.tool_progress` 事件。前端 `ToolCallCard` 组件接收并显示进度条。

复用现有的 `tool_progress` 事件格式（已用于内置工具），保持一致。

### D3: resources/prompts list_changed — 补全刷新 + 缓存

当前 `resources/list_changed` 仅 log，`prompts/list_changed` 仅发 WS 事件。

方案：
- `resources/list_changed`: 调用 `client.list_resources()` 刷新，重新注册 `mcp__list_resources`/`mcp__read_resource` 延迟工具，发送 `plugins.resources_changed` WS 事件
- `prompts/list_changed`: 调用 `client.list_prompts()` 刷新，发送已有的 `plugins.prompts_changed` WS 事件

### D4: OAuth DCR — 在 `oauth.rs` 中实现 `register_client()`

当 `McpServerConfig` 无 `oauth.client_id` 且服务器元数据有 `registration_endpoint` 时，自动执行 RFC 7591 动态客户端注册。

注册请求包含 `client_name: "XiaoLin"` + `redirect_uris` + `grant_types: ["authorization_code"]` + `response_types: ["code"]`。返回的 `client_id` + `client_secret` 持久化到 Token 存储。

### D5: Token 安全存储 — Tauri stronghold plugin

**备选方案:**
- A: 系统 keyring (`keyring` crate) → 跨平台兼容性问题（Linux 需 `libsecret`）
- **B (选定): `tauri-plugin-stronghold`** → Tauri 原生加密存储，已在 XiaoLin 依赖中，跨平台一致
- C: 加密文件（AES-256-GCM + 主密钥）→ 需自行管理密钥

抽象 `TokenStore` trait，当前 `FileTokenStore` 保留为 fallback，新增 `StrongholdTokenStore` 作为默认。

### D6: 反向 MCP 服务器 — 复用 `McpServer` 内部 API

`McpServer` 已有 `tools/list`、`tools/call`、`resources/list/read`、`prompts/list/get` 的完整实现。

方案：
- `create_xiaolin_mcp_server(tool_registry)` 构建 `McpServer` 实例
- `McpServer::run_stdio()` 可用于 stdio 传输
- 工具列表来自 `ToolRegistry` 快照（仅暴露内置工具，过滤 `mcp__*` 前缀避免循环）
- **不提供独立 CLI 命令**：XiaoLin 是桌面应用，反向 MCP 服务器通过应用内 API 暴露

### D7: WebSocket MCP 传输 — `tokio-tungstenite` + JSON-RPC 帧

新增 `McpClient::connect_websocket(url)` 方法。WebSocket 传输使用 subprotocol `mcp`，每个文本帧是一个 JSON-RPC 消息。

复用 `connect_streamable_http` 的 session 管理模式：发送 `initialize` → `initialized` → ready。断线重连复用 SSE 的指数退避逻辑。

`McpTransportType` 枚举新增 `WebSocket` 变体。

## Risks / Trade-offs

**[MCP 审批 UX 降级] → 缓解: 默认 `auto` 模式 + 会话缓存**
引入审批后用户每次 MCP 工具调用都需确认会严重影响体验。缓解措施：默认行为模式（"Auto edit"）下 MCP 工具自动执行；仅 "Suggest edits" 模式下需确认。`ApprovalCache` 的 `ApprovedForSession` 和 `ApprovedAllForSession` 避免重复确认。

**[DCR 注册信息泄露] → 缓解: 只在用户主动触发 OAuth 时注册**
DCR 向服务器发送 `client_name` + `redirect_uris`。仅在用户点击 "OAuth Login" 时触发，不在连接时自动注册。

**[反向 MCP 服务器安全风险] → 缓解: 仅暴露内置工具 + 需显式启动**
不自动启动 MCP server；需在设置中显式开启。暴露的工具列表不包含已连接的远程 MCP 工具（防循环调用）。

**[Stronghold 迁移兼容性] → 缓解: 自动迁移 + FileStore fallback**
首次启动时自动将 `mcp-tokens/*.json` 迁移到 stronghold，保留旧文件 30 天。如果 stronghold 初始化失败，回退到文件存储。
