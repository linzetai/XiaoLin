## Why

XiaoLin agent 在完成复杂任务时 token 消耗过高，任务成功率不稳定。通过与 Claude Code 和 Codex CLI 的深度对比，发现核心瓶颈在三个方面：
1. **工具引导精度不足**——`shell_exec` 无 `prompt()`（仅 350 字 desc），模型频繁用 shell 替代内置工具；`todo_write` 缺叙事示例，任务追踪不精确
2. **上下文效率低**——MCP 工具无 defer/search 机制，全量 schema 每轮发送；Context Collapse 已实现但未启用
3. **错误恢复薄弱**——`prompt_too_long` 连接失败路径虽有 reactive compact 但缺单次 guard（可无限重试）；max_output 恢复耗尽后 truncation hook 仍触发导致潜在无限循环；goal/todo continuation hooks 在 context 临界时缺防护

## What Changes

- **新增 `shell_exec` 的 `prompt()`**：从 Claude Code BashTool（8k+ 字）精炼出工具路由规则、git 操作规范、并行命令指引、反模式清单，对齐 FS 工具的引导精度
- **增强 `todo_write` prompt**：补充 3-5 个 `<example>` + `<reasoning>` 叙事示例，强化「何时用/不用 todo」的 LLM 行为 conditioning
- **实现 ToolSearch + MCP defer_loading**：`ToolDefinition` 增加 `defer_loading` / `search_hint` 字段，MCP 工具默认 deferred，新增 `tool_search` 内置工具按需加载
- **激活 Context Collapse**：启用已实现的 `CollapseStore` / `CollapseEngine`，读/搜操作折叠为 summary，减少 context 膨胀
- **PTL 连接失败 compact guard**：连接失败路径共用 `has_attempted_reactive_compact`，确保 reactive compact 仅执行一次（流内 withhold 路径已有此 guard）
- **Stop hooks 防护**：max_output 恢复耗尽后跳过 truncation hook；context 临界 + 已 compact 时跳过 goal/todo continuation hooks，防止间接 PTL 死循环（FatalError 路径已正确绕过 hooks，此修复针对 EndTurn 成功路径）
- **Stale detection 增强**：edit_file 的 stale 检测增加全文 content 比对 fallback，防 mtime 假阳性
- **Post-compact 恢复完善**：补全 deferred tools delta 重发、async agent status 恢复

## Capabilities

### New Capabilities
- `tool-search-defer`: MCP 工具延迟加载与按需搜索发现机制
- `shell-tool-prompt`: shell_exec 工具的详细行为引导 prompt

### Modified Capabilities
- `tool-exposure`: 增加 defer_loading / search_hint 字段支持
- `goal-continuation-loop`: stop hooks 增加 API error 跳过逻辑
- `deferred-pipeline`: context collapse 激活与 autocompact 互斥逻辑

## Impact

- **crates/xiaolin-core/src/tool.rs** — ToolDefinition 结构体扩展
- **crates/xiaolin-tools-fs/src/shell.rs** — shell_exec prompt() 新增
- **crates/xiaolin-agent/src/builtin_tools/** — tool_search 工具、todo_write prompt 增强
- **crates/xiaolin-agent/src/runtime/** — stop_hooks.rs、llm_call.rs、context_compressor.rs、post_compact_restore.rs
- **crates/xiaolin-mcp/src/** — MCP 工具 defer_loading 标记
- **crates/xiaolin-tools-fs/src/file_state_cache.rs** — stale detection content fallback
