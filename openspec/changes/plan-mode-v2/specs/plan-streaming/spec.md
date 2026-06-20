## ADDED Requirements

### Requirement: Phase 1 — PlanFileUpdate 带内容推送
作为流式方案的过渡，`PlanFileUpdate` 事件 SHALL 新增可选的 `content` 字段。当 plan 文件被写入时，事件 SHALL 包含文件的完整内容，前端无需额外 HTTP 请求即可渲染。

#### Scenario: write_file 完成后推送全量内容
- **WHEN** `write_file` 工具成功写入 plan 文件
- **THEN** `PlanFileUpdate` 事件 SHALL 包含 `content` 字段，值为 plan 文件的完整文本内容

#### Scenario: 前端消除 HTTP refetch
- **WHEN** 前端 PlanPanel 收到带 `content` 字段的 `plan_file_update` 事件
- **THEN** SHALL 直接使用该 content 渲染，不调用 `getPlanFile()` HTTP 请求
- **WHEN** 收到不带 `content` 字段的 `plan_file_update` 事件
- **THEN** SHALL 降级为调用 `getPlanFile()` 获取内容（兼容旧版后端）

### Requirement: Phase 2 — PlanArgInterceptor 从工具参数流中提取 plan 内容
Agent runtime SHALL 在 LLM 工具参数流式累积期间，当检测到 `write_file` 或 `edit_file` 工具且目标路径匹配当前会话的 plan 文件路径时，实时从 `content` 参数值中提取增量文本（含 JSON 字符串反转义），并通过 `AgentEvent::PlanDelta` 发送到事件通道。

#### Scenario: write_file 到 plan 文件时流式发送 PlanDelta
- **WHEN** LLM 在 Plan 模式下输出 `write_file` 工具调用，且 `file_path` 参数匹配当前会话的 plan 文件路径
- **THEN** 每个 `content` 参数的增量 chunk SHALL 被解析为文本 delta（反转义 JSON 字符串），并作为 `AgentEvent::PlanDelta` 事件发送到前端

#### Scenario: 非 plan 文件的 write_file 不触发 PlanDelta
- **WHEN** LLM 输出 `write_file` 工具调用，但 `file_path` 不匹配 plan 文件路径
- **THEN** 不 SHALL 发送任何 `PlanDelta` 事件

#### Scenario: JSON 转义字符正确还原
- **WHEN** content 参数值包含 `\n`、`\t`、`\"`、`\\`、`\uXXXX` 等 JSON 转义序列
- **THEN** PlanDelta 中的文本 SHALL 是反转义后的原始文本（`\n` → 换行符，`\"` → 引号等）

#### Scenario: path 在 content 之后到达
- **WHEN** LLM 先输出 `content` 参数再输出 `file_path` 参数
- **THEN** content delta SHALL 被缓存（最多 200 字符），path 到达并确认匹配后 flush 缓存的 delta；path 不匹配时 discard 缓存

### Requirement: AgentEvent::PlanDelta 协议事件
协议层 SHALL 定义 `AgentEvent::PlanDelta` 事件变体，包含 `turn_id`、`session_id`、`delta`（文本增量）字段。

#### Scenario: PlanDelta 通过 WebSocket 转发
- **WHEN** agent runtime 发送 `AgentEvent::PlanDelta` 事件
- **THEN** gateway SHALL 将其作为 `plan_delta` 类型的 WebSocket 消息转发到所有订阅该 session 的客户端

### Requirement: PlanPanel 流式 Markdown 渲染（Phase 2 前端）
前端 PlanPanel 组件 SHALL 在收到 `plan_delta` WebSocket 事件时，将 delta 按行累积并实时渲染 markdown。

#### Scenario: 按行 commit 策略
- **WHEN** 收到的 delta 不含换行符
- **THEN** SHALL 只追加到 buffer，不触发 react-markdown 重渲染
- **WHEN** 收到的 delta 含换行符
- **THEN** SHALL 将 buffer 中最后一个换行符之前的内容 commit 到 stableContent 状态，触发 react-markdown 渲染
- 此策略参考 Codex 的 PlanStreamController：delta 含 `\n` 时才 commit，避免半成品行闪烁

#### Scenario: 流式渲染中显示光标
- **WHEN** PlanPanel 正在接收 `plan_delta` 事件
- **THEN** SHALL 在 markdown 末尾显示闪烁光标（2px 宽竖线，0.8s 周期 blink 动画）

#### Scenario: PlanFileUpdate 后停止流式
- **WHEN** 收到 `plan_file_update` 事件
- **THEN** PlanPanel SHALL 停止流式模式，移除光标，使用事件中的 `content` 字段（如有）替换当前内容确保与磁盘一致

#### Scenario: 新行渐入动画
- **WHEN** 流式模式下有新行被 commit 到 stableContent
- **THEN** 新行 SHALL 以 fadeSlideIn 动画（0.15s ease-out，从 opacity:0 translateY:4px → opacity:1 translateY:0）出现

#### Scenario: 自动滚动与用户中断
- **WHEN** 流式模式下新内容超出 PlanPanel 可视区域
- **THEN** SHALL 自动滚动到底部
- **WHEN** 用户手动向上滚动（scrollTop + clientHeight < scrollHeight - threshold）
- **THEN** SHALL 暂停自动滚动，直到用户滚回底部附近

## Implementation Reference

### 与竞品的技术对标

| 技术点 | Codex | XiaoLin Phase 1 | XiaoLin Phase 2 |
|--------|-------|------------------|------------------|
| Plan 内容获取 | PlanDelta 事件 | plan_file_update + content 字段 | plan_delta 事件 |
| 渲染触发 | 含 `\n` 的 delta | 整文替换 | 含 `\n` 的 delta commit |
| Markdown 渲染 | `append_markdown_agent_with_cwd` 全量 | react-markdown 全量 | react-markdown（stableContent 变化时） |
| 视觉反馈 | commit 动画（行滑入 scrollback） | 无（瞬间替换） | 闪烁光标 + fadeSlideIn |
| 完成态 | ItemCompleted 权威文本 | plan_file_update content | plan_file_update content |

### Codex 核心设计模式参考

1. **TaggedLineParser**：通过 `<proposed_plan>` XML 标签从 assistant text 中分离 plan 内容。XiaoLin 不需要此组件，因为 plan 走 write_file 工具调用。
2. **PlanStreamController**：delta 累积 → 行 commit → 全量 markdown 重渲染 → stable/tail 分区。XiaoLin Phase 2 采用类似策略（按行 commit），但无 stable/tail 分区（GUI 不需要 scrollback 概念）。
3. **Agent Message Defer**：Plan 模式下 plan-only 输出时延迟 ItemStarted，避免空气泡。XiaoLin 可选实施。

### Claude Code 核心设计模式参考

1. **分层展示**：transcript 中只显示 "Updated plan" + "/plan to preview"，完整内容在 ExitPlanMode 对话框中用 Markdown 渲染。XiaoLin 的 PlanPanel side panel 天然实现了分层（chat 流 + 独立面板），但 chat 流中的工具结果仍需简化。
2. **不在 transcript 刷屏**：plan 文件内容不嵌入 chat 流。XiaoLin 应将 write_file(plan.md) 的工具结果简化为轻量 hint。
