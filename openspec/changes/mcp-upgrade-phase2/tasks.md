# MCP 升级 Phase 2 — 任务清单

> 基于 Codex CLI / Claude Code 源码审计（2026-06-16），补齐安全缺口、协议差距和平台能力。
> 每个任务标注对应 spec 和前置依赖。

## 1. P0 — MCP 工具审批修复

> Spec: `mcp-tool-approval/spec.md` | Design: D1

- [x] 1.1 在 `dispatcher.rs` 中为 MCP 工具增加审批分支 — `is_mcp_tool()` 判断 + `requires_confirmation()` 检查，新增 `execute_mcp_with_approval()` 方法
- [x] 1.2 `execute_mcp_with_approval` 实现 — 复用 `OrchestrationDecision` 流程（`Interactive`/`AutoApprove`/`DenyAll`），通过 `approval_tx` 发送审批请求，等待响应后调用 `ToolRegistry::call`
- [x] 1.3 MCP 审批与 `ApprovalCache` 集成 — `ApprovedForSession` / `ApprovedAllForSession` 对 MCP 工具生效，避免重复确认
- [x] 1.4 前端审批 UI 适配 — `ApprovalCard` 组件识别 MCP 工具（`mcp__` 前缀），显示服务器名 + 工具名 + 参数摘要
- [x] 1.5 验证 — 单元测试 `tools_ask: ["mcp__*"]` 模式下 MCP 工具触发审批；`tools_allow` 模式下自动执行

## 2. P1 — Progress 通知处理

> Spec: `mcp-progress-notifications/spec.md` | Design: D2

- [x] 2.1 在 `tools/call` 请求中注入 `_meta.progressToken` — `call_tool_with_progress()` 新方法支持可选 token
- [x] 2.2 Notification watcher 增加 `notifications/progress` match arm — 提取 `progressToken`/`progress`/`total`/`message`，通过 `ws_broadcast` 发送 `plugins.tool_progress` 事件
- [x] 2.3 `xiaolin-protocol/src/op.rs` 已有 `ToolProgress` 事件类型，WS 事件通过 `plugins.tool_progress` 直接推送
- [x] 2.4 前端 `StepIndicator` 已有进度条 UI，新增 `plugins.tool_progress` WS 事件订阅，将 MCP progress 路由到 `setToolProgress`
- [ ] 2.5 验证 — 用 mock MCP server 发送 progress 通知，确认前端进度条更新

## 3. P1 — Notification 缓存刷新补全

> Spec: `mcp-notification-refresh/spec.md` | Design: D3

- [x] 3.1 `resources/list_changed` handler — 调用 `client.list_resources()`，发送 `plugins.resources_changed` WS 事件
- [x] 3.2 `prompts/list_changed` handler — 调用 `client.list_prompts()` 刷新缓存，保留 `plugins.prompts_changed` WS 事件
- [x] 3.3 前端 `usePluginStore` 响应 `resources_changed` / `prompts_changed` 事件 — 自动 `fetchPlugins()` 刷新
- [ ] 3.4 验证 — 连接测试 MCP server，动态添加/移除 resource，确认前端列表实时更新

## 4. P1 — OAuth DCR 实现

> Spec: `mcp-oauth-dcr/spec.md` | Design: D4

- [x] 4.1 `oauth.rs` 新增 `register_client()` 函数 — POST `registration_endpoint` with `client_name`/`redirect_uris`/`grant_types`/`response_types`，返回 `ClientRegistration { client_id, client_secret }`
- [x] 4.2 修改 `handle_plugins_oauth_login()` 流程 — 有 `registration_endpoint` 时先调 `register_client()`，再用返回的 client_id 走 PKCE 流程
- [x] 4.3 DCR 凭据持久化 — 存储到 `{server_id}__dcr` token 文件，后续登录复用
- [x] 4.4 fallback 逻辑 — DCR 失败时降级为 server URL 作为 client_id（当前行为），记录 warning
- [ ] 4.5 验证 — 对有 DCR 端点的 MCP server 测试完整 OAuth 流程

## 5. P1 — Token 安全存储

> Spec: `mcp-token-secure-storage/spec.md` | Design: D5

- [x] 5.1 定义 `TokenStore` trait — `load(server_id)`, `save(server_id, token)`, `delete(server_id)` 异步方法
- [x] 5.2 实现 `FileTokenStore` — 包装现有明文 JSON 逻辑，实现 `TokenStore` trait
- [ ] 5.3 实现 `StrongholdTokenStore` — 需 `tauri-plugin-stronghold` 集成，延后至 Tauri 插件链完善后
- [ ] 5.4 自动迁移逻辑 — 依赖 5.3
- [ ] 5.5 `oauth.rs` 重构 — 依赖 5.3
- [ ] 5.6 验证 — 依赖 5.3

## 6. P2 — roots/list 响应

> Spec: `mcp-roots-response/spec.md` | Design: D6 补充

- [x] 6.1 MCP client initialize 时声明 `capabilities.roots: { listChanged: true }` — 两处（正常 + session recovery）
- [x] 6.2 增加 server request handler — `spawn_server_request_watcher` 监听 `roots/list`，返回 workspace root URI
- [x] 6.3 workspace path 注入 — 使用 `detect_workspace_root(cwd)` 获取当前工作区路径
- [ ] 6.4 验证 — mock MCP server 发送 `roots/list` 请求，确认收到正确的工作区 URI

## 7. P2 — 反向 MCP 服务器

> Spec: `mcp-reverse-server/spec.md` | Design: D6

- [~] 7.1 ~~CLI 入口 `xiaolin mcp serve`~~ — 已取消：XiaoLin 是桌面应用，不提供独立 CLI；反向 MCP 服务器通过 Tauri 应用内 API 暴露
- [x] 7.2 `McpServer` 与 `ToolRegistry` 集成 — `create_xiaolin_mcp_server()` 已过滤 `mcp__*` 前缀工具
- [x] 7.3 `tools/call` 路由到 `ToolRegistry::call` — 已在 `create_xiaolin_mcp_server()` 中实现
- [x] 7.4 Server 能力声明 — `McpServer::handle_initialize()` 已返回 `tools` + `serverInfo`
- [ ] 7.5 验证 — 用 `npx @anthropic-ai/mcp-inspector` 连接 XiaoLin MCP server，列出并调用工具

## 8. P2 — WebSocket MCP 传输

> Spec: `mcp-websocket-transport/spec.md` | Design: D7

- [x] 8.1 添加 `tokio-tungstenite` 依赖到 `xiaolin-mcp/Cargo.toml`
- [x] 8.2 实现 `McpClient::connect_websocket(url)` — WebSocket 连接 + subprotocol `mcp` + JSON-RPC text frame 收发
- [x] 8.3 notification dispatch — `websocket_reader_loop` 分流 response/notification，复用 `dispatch_incoming`
- [x] 8.4 `McpTransportType` 枚举新增 `WebSocket` 变体 — `connect_mcp_server` 路由 + `register_mcp_tools_websocket`
- [x] 8.5 断线重连 — Close/Error 时发送 `xiaolin/transport_disconnected` 通知，复用 SSE 重连逻辑
- [x] 8.6 前端 `AddServerModal` 新增 WebSocket 传输选项 + `AddMcpServerParams` 类型更新
- [ ] 8.7 验证 — 连接 WebSocket MCP server，确认 tools/list + tools/call + notification 正常
