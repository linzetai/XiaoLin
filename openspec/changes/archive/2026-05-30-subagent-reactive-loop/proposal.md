## Why

当前 sub-agent 系统采用 LLM 手动编排模型（spawn → 手动 wait_agent → 手动 subagent_get），主 agent 的 LLM 需要自己管理等待循环和结果拉取，导致：
1. LLM 不积极使用 sub-agent（编排成本高、容易遗忘 poll）
2. Sub-agent 完成后结果无法即时回流到主 agent context
3. Turn 可能在 sub-agent 仍在运行时错误结束
4. 用户无法实时观察 sub-agent 的执行状态

需要将 sub-agent 编排从"LLM 手动管理"升级为"Harness 自动响应式循环"，让 LLM 只负责 spawn 决策，harness 自动完成 wait → notify → re-prompt 流程。

## What Changes

- **新执行模型**: Harness 级 supervised reactive loop — 主 agent spawn 后自动进入等待，每当有 sub-agent 完成时 harness 注入 completion notification 并 re-prompt 主 LLM
- **Turn 结束语义变更**: Turn 不再在 LLM 停止输出时立即结束，而是在所有活跃 sub-agent 完成且 LLM 不再 spawn 新任务时才结束
- **工具集简化**: 移除 `wait_agent` 和 `subagent_get` 的主动轮询职责（harness 接管），保留 `spawn_subagent`、`subagent_list`、`cancel_subagent`
- **Completion notification 格式**: 结构化的 sub-agent 结果注入（summary + files_touched + full_result + remaining status）
- **Prompt 增强**: 更强的 delegation trigger signal，让 LLM 更积极地使用 sub-agent
- **前端状态监控面板**: 聊天区域旁边的实时 sub-agent 状态面板（auto show/hide）
- **Batch completion**: 短时间窗口内完成的多个 sub-agent 结果合并为一次 re-prompt

## Capabilities

### New Capabilities
- `subagent-reactive-loop`: Harness 级响应式编排循环，包括 supervised wait、completion-driven re-prompt、turn 结束条件守卫
- `subagent-monitor-panel`: 前端聊天区旁的实时 sub-agent 状态监控面板

### Modified Capabilities
<!-- 无现有 spec 需要修改 -->

## Impact

- **xiaolin-agent runtime 执行循环** (`runtime/mod.rs`, `session_bridge.rs`): 核心改动，需要在 agentic loop 中加入 sub-agent completion 检测和 re-prompt 逻辑
- **xiaolin-agent prompt** (`prompt_builder.rs`, `prompt_sections/dynamic.rs`): 增强 delegation guidance
- **xiaolin-agent tools** (`subagent.rs`): 简化工具集，`wait_agent` 降级或移除
- **xiaolin-protocol events** (`event.rs`): 可能新增 `SubAgentNotification` 事件类型
- **xiaolin-app 前端**: 新增 `SubAgentMonitor` 组件，修改 `MessageStream` 布局
- **xiaolin-app stores** (`types.ts`): 增强 `SubAgentRunUI` 类型，支持实时进度数据
