---
title: 工具与插件
summary: 内置工具、WASM 插件、MCP、权限模型与扩展开发指引。
---

# 工具与插件

## 内置工具列表

FastClaw 在 `fastclaw_agent::builtin_tools` 中注册一批常用工具（文件读写、终端、网络搜索等）。查看本机实际列表：

```bash
fastclaw tools list
```

JSON 输出：

```bash
fastclaw tools list --json
```

HTTP：`GET /api/v1/tools`（需与部署认证策略一致）。

## 文件工具增强（高频推荐）

### `read_file`（已增强）

- 支持按行窗口读取：`offset` + `limit`
- 支持行号前缀：`number_lines: true`
- 支持输出上限调节：`max_chars`（默认 `32768`，最大 `256000`）

示例：

```json
{
  "path": "crates/fastclaw-gateway/src/ws.rs",
  "offset": 120,
  "limit": 80,
  "number_lines": true
}
```

### `write_file`（已增强）

- `mode` 支持：
  - `overwrite`（默认，原子覆盖）
  - `append`（末尾追加）
  - `create_new`（文件存在即失败）
- `expected_content`：乐观锁，防止并发覆盖

示例：

```json
{
  "path": "notes/change-log.md",
  "content": "新增一条记录\n",
  "mode": "append"
}
```

### `edit_file`（精准替换）

- 用 `old_string` / `new_string` 做精确替换
- 默认只替换 1 处，若多处命中需显式设置 `replace_all` 或 `expected_replacements`

示例：

```json
{
  "path": "src/lib.rs",
  "old_string": "fn old_name()",
  "new_string": "fn new_name()",
  "expected_replacements": 1
}
```

### `apply_patch`（新）

- 一次请求可对同一文件应用多段编辑
- 全部编辑在内存完成后一次原子写入，减少多次写导致的不一致
- 支持 `expected_content` 乐观锁

示例：

```json
{
  "path": "src/config.rs",
  "edits": [
    { "old_string": "let retry = 1;", "new_string": "let retry = 3;" },
    { "old_string": "let timeout_ms = 1000;", "new_string": "let timeout_ms = 3000;" }
  ]
}
```

### `search_in_files`（新）

- 类 ripgrep 的代码检索工具（正则模式）
- 可限制目录范围（`path`）和文件过滤（`glob`）
- 返回结构化命中（路径、行、列、命中文本）

示例：

```json
{
  "pattern": "spawn_subagent",
  "path": "crates/fastclaw-agent/src",
  "glob": "*.rs",
  "max_results": 100
}
```

## 代码智能工具（LSP 集成）

FastClaw 内置 LSP 客户端（通过 `LspSessionManager`），为 Agent 提供语义级代码理解能力。当 LSP 不可用时自动降级为文本搜索。

### `workspace_symbols`（符号搜索）

- 在工作区范围内搜索符号（函数、类、接口等）
- 优先使用 LSP `workspace/symbol`，不可用时回退到正则启发式搜索

示例：

```json
{
  "query": "ChatRequest",
  "path": "crates/fastclaw-core/src"
}
```

### `go_to_definition`（跳转定义）

- 跳转到符号的定义位置
- 依赖 LSP `textDocument/definition`，回退时使用文本搜索

示例：

```json
{
  "symbol": "resolve_state_dir_from",
  "path": "crates/fastclaw-core/src/paths.rs",
  "line": 42
}
```

### `find_references`（查找引用）

- 查找符号在工作区中的所有引用位置
- 依赖 LSP `textDocument/references`

示例：

```json
{
  "symbol": "AppState",
  "path": "crates/fastclaw-gateway/src/state.rs",
  "line": 15
}
```

## 人机交互工具

### `ask_question`（向用户提问）

- Agent 在任务执行中向用户发起结构化问题
- 支持单选/多选（`allow_multiple`）、超时（`timeout_secs`）
- 问题通过 `StreamEvent::AskQuestion` 推送到桌面应用或 WebSocket 客户端
- 用户回答后结果回灌到 Agent 上下文

示例：

```json
{
  "question": "该文件已存在，如何处理？",
  "options": [
    { "id": "overwrite", "label": "覆盖" },
    { "id": "skip", "label": "跳过" },
    { "id": "rename", "label": "重命名" }
  ],
  "allow_multiple": false,
  "timeout_secs": 60
}
```

## WASM 插件工具

- 插件放在配置指定的 **`plugins` 目录**（见示例 `plugins.directory`）。
- 宿主为 `fastclaw-plugin`，可对 **内存、执行时间、Fuel** 等设上限（`plugins.defaults`）。
- 支持 **热重载**：变更 `.wasm` 或清单后由网关监听并刷新注册表。

调用路径：`POST /api/v1/plugins/:plugin_id/invoke/:capability`。

## MCP 集成

`fastclaw mcp-server`（子命令 **`McpServer`**，stdio 传输）将 FastClaw 工具暴露给 **外部 Agent**（如 Cursor、Claude Desktop）。在对方应用中配置 MCP 指向该可执行文件及参数即可。

## 工具权限（allow / deny）

在 Agent 配置中使用 `tools` 字段：

```json
{
  "id": "main",
  "tools": {
    "allow": ["read_file", "web_search"],
    "deny": ["shell_exec"]
  }
}
```

规则：**`deny` 优先**；若仅配置 `allow`，则等价白名单；若仅 `deny`，则黑名单之外的工具需结合网关默认策略理解。

## 工具开发指南

1. **内置工具（Rust）**：在 `fastclaw-agent` 中实现 `ToolDefinition` + 执行闭包，并注册到 `ToolRegistry`。
2. **WASM 插件**：遵循插件清单格式（`fastclaw.plugin.json`），导出约定 capability；注意沙箱配额与错误传播。
3. **MCP 工具**：由外部进程实现，FastClaw 侧负责会话桥接与权限审计。

开发完成后务必补充 **单元测试** 与 **最小权限** 配置示例。

## 相关文档

- [Agent 概念](../concepts/agents.md)
- [安全：WASM 与注入](../security/index.md)
- [REST API：插件与工具](../reference/api.md)
