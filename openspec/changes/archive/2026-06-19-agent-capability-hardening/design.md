## Context

XiaoLin 的 Sub-agent 系统通过 `SubAgentDef` 定义类型（explore/code/shell/research/coordinator），每个 def 声明了 `model: Option<AgentModelConfig>` 字段用于独立模型配置。但当前执行路径（`subagent.rs` → `SubAgentTool::execute`）在构建 child `AgentConfig` 时从未读取 `def.model`，导致所有 sub-agent 无条件继承 main agent 的模型。

同样，`sidechain::truncate_result` 使用硬编码 `MAX_RESULT_CHARS = 32768`，无法按 sub-agent 类型定制。

此外，`SubAgentManager::cleanup_session()` 遗漏了 `session_event_senders` 的清理，可能导致已销毁 session 的 sender 残留在内存中。

## Goals / Non-Goals

**Goals:**
- Sub-agent 执行时，若 `SubAgentDef.model` 非空，则用该模型配置覆盖继承的 agent config
- 结果截断上限可通过 `SubAgentDef.max_result_chars` 字段配置，默认保持 32768
- `cleanup_session` 清理 `session_event_senders` 条目
- 所有变更向后兼容，现有配置无需修改

**Non-Goals:**
- 不实现运行时模型路由（根据任务复杂度动态选模型）
- 不修改 MCP 审批逻辑（已确认已实现）
- 不修改 session 删除的文件系统清理（已确认已实现）

## Decisions

### 1. Model override 应用点

**选择**: 在 `subagent.rs` 的 `SubAgentTool::execute` 中，构建 `AgentConfig` 后、调用 `spawn()` 前应用 `def.model` 覆盖。

**替代方案**: 在 `SubAgentManager::run_subagent` 中应用。
**理由**: `execute` 是构建 config 的唯一入口，在这里应用更清晰，且不影响 `run_subagent` 的通用性。

### 2. max_result_chars 配置位置

**选择**: 在 `SubAgentDef` 新增 `max_result_chars: Option<usize>` 字段，默认 `None` 表示使用全局 `MAX_RESULT_CHARS`。在 `SubAgentManager::spawn` 的 forwarder 中传递该值到 `truncate_result`。

**替代方案**: 在 `SubAgentPolicy` 中配置。
**理由**: 不同类型的 sub-agent 可能需要不同的上限（如 code agent 需要更大上限来传递完整文件内容），放在 def 级别更灵活。

### 3. truncate_result 参数化方式

**选择**: 将 `truncate_result` 改为接受 `max_chars: usize` 参数，保持 `MAX_RESULT_CHARS` 作为默认值常量。

**替代方案**: 引入一个 config struct。
**理由**: 当前只有一个参数需要配置，简单参数就够了。

## Risks / Trade-offs

- **[Model override 可能增加成本]** → 文档中明确说明 `SubAgentDef.model` 的计费影响，由配置者自行决定
- **[max_result_chars 过大导致 context window 膨胀]** → 设置合理上限（如 128KB），在 serde 反序列化中校验
- **[向后兼容]** → 所有新字段均为 `Option` 且 `serde(default)`，现有 JSON/Markdown def 文件无需修改
