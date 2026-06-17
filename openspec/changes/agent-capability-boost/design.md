## Context

XiaoLin agent 的架构已与 Claude Code 高度对齐（9 段 compact prompt、threshold 常量、post-restore 预算、tier 微压缩），但在三个关键维度存在差距导致 token 浪费和任务失败：

1. **工具引导精度**：shell_exec 无 prompt()（350 字 desc vs CC BashTool 8k+），模型频繁用 shell 替代内置工具
2. **上下文效率**：MCP 工具默认 eager 注册（仅超 128 才 defer），ToolSearch 已有但 MCP 未接入；Context Collapse 库层 88% 完成但 agent 集成仅 5%
3. **错误恢复**：PTL 连接失败无单次 compact guard；max_output 耗尽后 truncation hook 仍触发导致潜在无限循环

## Goals / Non-Goals

**Goals:**
- 通过工具 prompt 增强减少模型误用内置工具（预期减少 ~30% 重试 token）
- MCP 工具默认 defer，消除系统 prompt 双重注入（每轮节省数千 prompt token）
- 修复 API error 防护缺口，防止 stop hook 死循环
- 为 Context Collapse 完整激活铺路（长期 token 节省 ~25%）

**Non-Goals:**
- 不实现 Anthropic API 的 `defer_loading` 原生字段（中期目标，本轮用客户端 activate 方案）
- 不实现 Context Collapse 的 async 后台模式（先用同步阻塞）
- 不改变 apply_patch / edit_file 的编辑机制（已够强，fuzzy match 6 级）
- 不实现 StopFailure 外部 hook 事件（可选，非核心）

## Decisions

### D1: shell_exec prompt 策略 — 从 system prompt 提取 + CC BashTool 精炼

**选择**：在 `shell.rs` 新增 `prompt()` 方法，内容从三个来源合成：
1. XiaoLin 现有 system prompt 的 `using_tools` 决策树（shell 相关规则）
2. Claude Code BashTool/prompt.ts 核心规则（工具路由、git、反模式）
3. Codex 的 shell 安全规则

**替代方案**：仅扩充 system prompt 决策树 → 弃，因为 system prompt 每轮都发且不受 ToolProfile 控制，放到 tool prompt 可按 Plan mode 灵活调控。

**规模**：~2k 字（CC 的 8k 太长，精炼核心规则即可）

### D2: TodoWrite 示例增强 — 3 段 example+reasoning

**选择**：在 todo_write 的 prompt 中追加 3 个典型场景的叙事示例，采用 Claude Code `<example>` + `<reasoning>` 格式。

**要点**：
- 正面示例：复杂多步任务 → 创建 todo
- 负面示例：简单单步任务 → 不创建 todo
- 边界示例：用户明确要求 → 创建 todo（即使简单）

### D3: MCP 默认 defer — McpToolBridge.exposure() override

**选择方案 B（改动最小）**：MCP 工具默认 `ToolExposure::Deferred`，`_meta.alwaysLoad` 例外。

**数据流变化**：
```
之前: McpToolBridge → registry.register() → eager → 每轮 tool_defs
之后: McpToolBridge → registry.register() → exposure()==Deferred → deferred set
                                           → force_eager()==true → eager（alwaysLoad）
```

**消除双重注入**：`inject_mcp_tools_prompt()` 中 eager MCP 工具不再重复描述 schema，只保留工具名列表。

**阈值调整**：`maybe_defer_mcp_tools()` 从固定 128 改为 safety net（仅处理 legacy 场景），MCP 本身已默认 deferred。

**不做 Anthropic defer_loading 原生字段**：需要 API beta header + provider 层改造，留给中期。

### D4: API error 防护 — 三处修复

**修复 1**：PTL 连接失败路径（`llm_call.rs`）共用 `has_attempted_reactive_compact` guard，防止无限 compact+retry。

**修复 2**：`max_output_tokens` 恢复耗尽后（recovery_count >= 3），在 `evaluate_stop_hooks` 中跳过 Hook 2 `output_truncated`。方式：新增 `ms.query_loop.max_output_recovery_exhausted: bool`。

**修复 3**：评估 goal/todo hook 前检查 context 占用，若 `has_attempted_reactive_compact && context > 85%` 则跳过 continuation hooks，防止注入 continuation prompt 后再次 PTL。

**不做**：主 loop 接入 `with_retry`（已有 API 层 retry，主 loop 改造风险大）。

### D5: Context Collapse 激活路径 — 分两步

**Step 1（本轮）**：
- 实现 `CollapseSummarizer` 桥接到现有 LLM provider
- 在 `unified_compact.rs` Step 5 后调用 `CollapseEngine::collapse()` + `project()`
- 配置：`BehaviorConfig` 增加 `enable_collapse: bool`（默认 false），不再仅靠 env
- 不做 session persist（简化）

**Step 2（后续）**：
- Session persistence（save/load collapse_state）
- Async collapse 模式
- Snip 与 Collapse 冲突协调（collapse 启用时 snip 仅删 collapse 已覆盖的 round）

### D6: Stale detection content fallback — check_stale() 增加内容比对

在 `FileStateCache::check_stale()` 中：mtime 变但 `content_hash` 匹配则返回 `Fresh`（已有 `content_hash` 字段但未用于 stale check）。

## Risks / Trade-offs

| 风险 | 影响 | 缓解 |
|------|------|------|
| shell prompt 过长增加 tool token | 每轮 ~500 extra tokens | 精炼到 2k 字，Plan mode 可 demote |
| MCP 默认 defer 导致模型找不到工具 | 任务失败率暂时上升 | tool_search prompt 明确引导；alwaysLoad 例外 |
| Context Collapse + Snip 冲突 | 消息投影错误 | Step 1 不启用 snip；Step 2 协调 |
| stop hook 防护过严导致 goal 过早暂停 | 长任务中断 | 只在 `has_attempted_reactive_compact` 时触发，正常流程不受影响 |
| Stale content 比对 I/O 开销 | 大文件 hash 耗时 | 仅在 mtime 变化时执行，且复用已缓存的 hash |
