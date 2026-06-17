## Why

Phase 1 (`mcp-capability-upgrade`) 已完成 100 个任务，将 XiaoLin MCP 从 ~30 分提升到 ~72 分。但基于 Codex CLI (`../codex`) 和 Claude Code (`../claude-code`) 的源码级审计（2026-06-16），仍存在三类关键差距：

1. **安全缺口 (P0)**：MCP 工具走 `execute_unguarded` 路径，`tools_ask` 配置形同虚设 — Codex 有 `AppToolApproval` 三级模式，Claude Code 有 passthrough + permission rules
2. **协议覆盖不足 (P1)**：`notifications/progress` 未处理、`resources/list_changed` 仅 log、DCR 未实际执行、Token 使用明文 JSON 存储
3. **平台能力差距 (P2)**：无反向 MCP 服务器、无 WebSocket MCP 传输、无企业托管模式

目标：修复 P0 安全缺口，补齐 P1 协议差距，使 XiaoLin 综合评分从 7.2 提升到 ~8.5（接近 Claude Code 的 8.3）。

## What Changes

### 安全修复
- **MCP 工具审批执行**：`dispatcher.rs` 中 MCP 工具不再走 `execute_unguarded`，接入 `requires_confirmation` 检查流程
- **Token 安全存储**：从明文 JSON 迁移到 Tauri stronghold 或系统 keyring

### 协议补齐
- **`notifications/progress` 处理**：长时间 MCP 工具调用显示进度到前端
- **`resources/list_changed` 完整链路**：收到通知后刷新缓存并通知前端
- **`prompts/list_changed` 完整链路**：收到通知后刷新缓存
- **OAuth DCR 实现**：当无显式 `client_id` 时执行动态客户端注册
- **`roots/list` 响应**：响应服务端的 `roots/list` 请求，返回当前工作区路径

### 平台能力
- **反向 MCP 服务器**：上线 `xiaolin mcp serve`，将内置工具通过 stdio MCP 暴露给外部 host
- **WebSocket MCP 传输**：新增 `ws` 传输类型，支持 `wss://` MCP 服务器

## Capabilities

### New Capabilities
- `mcp-tool-approval`: MCP 工具调用的审批执行机制 — 接入现有 approval pipeline，支持 per-server 和 glob 模式匹配
- `mcp-progress-notifications`: MCP progress 通知处理 — 转发到前端显示长时间工具调用进度
- `mcp-notification-refresh`: resources/list_changed 和 prompts/list_changed 完整缓存刷新链路
- `mcp-oauth-dcr`: OAuth 动态客户端注册 — 无 client_id 时自动注册
- `mcp-token-secure-storage`: Token 安全存储 — 从明文 JSON 迁移到加密存储
- `mcp-roots-response`: roots/list 服务端请求响应 — 返回当前工作区信息
- `mcp-reverse-server`: 反向 MCP 服务器 — 将 XiaoLin 内置工具通过 stdio 暴露
- `mcp-websocket-transport`: WebSocket MCP 传输 — 支持 ws/wss 协议连接 MCP 服务器

### Modified Capabilities
- `plugin-panel`: NeedsAuth 状态下显示 DCR 流程提示；progress 通知 UI 显示

## Impact

- **`xiaolin-agent/src/runtime/dispatcher.rs`**：MCP 工具进入 guarded 路径或独立审批检查
- **`xiaolin-mcp/src/lib.rs`**：新增 progress notification handler、roots/list handler、DCR 流程、WebSocket transport
- **`xiaolin-mcp/src/oauth.rs`**：DCR 实现 + Token 安全存储接口
- **`xiaolin-gateway/src/state/mod.rs`**：notification handler 补全 resources/prompts refresh
- **`xiaolin-gateway/src/mcp_tool.rs`**：MCP 审批桥接
- **`xiaolin-protocol/src/op.rs`**：新增 progress 事件
- **前端**：progress 进度条 UI、审批确认对话框
- **新 crate 可能**：`xiaolin-mcp-server`（反向 MCP）或复用现有 `McpServer`
- **依赖**：可能新增 `keyring`/`tauri-plugin-stronghold`、`tokio-tungstenite`（WebSocket）
