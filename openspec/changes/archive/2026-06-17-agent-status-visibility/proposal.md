## Why

Agent 执行长任务时存在多个"静默期"——用户看不到任何状态变化，产生"卡住"的感知。对比 Claude Code 等产品，每个执行阶段都有明确的状态标签（Thinking... / Reading file... / Step N），用户始终知道 agent 在做什么。当前 XiaoLin 在以下时刻缺乏反馈：

1. LLM 首个 token 到达前（仅 3 弹跳圆点）
2. 模型 reasoning 阶段（内容被丢弃，前端不展示）
3. Tool call JSON 流式积累中（无任何输出）
4. Tool 执行中（仅 shell 有增量输出，其他工具只有计时器）
5. 工具完成到下一轮 LLM 调用之间（无迭代分隔）

## What Changes

- 后端流式发送 `reasoning_delta` 事件（当前已定义但从未 emit）
- 后端发送 `iteration_boundary` 事件（当前 `ToolRoundBoundary` 被过滤不转发前端）
- 前端新增 reasoning 折叠式展示块
- 前端将通用 `Typing()` 3圆点替换为带阶段文字的 `PhaseIndicator`（连接中/思考中/规划下一步）
- 前端 `StepIndicator` 消费 `tool_progress` 事件显示进度条和增量消息
- 前端在 streaming row 显示迭代计数 "Step N"

## Capabilities

### New Capabilities
- `reasoning-stream-display`: 实时展示 LLM reasoning/thinking 内容的折叠式 UI 块
- `agent-phase-indicator`: 根据当前执行阶段显示状态文字标签（替代通用弹跳圆点）
- `iteration-progress-display`: 迭代分隔线和计数器，让用户知道 agent 在第几步

### Modified Capabilities
- `card-step-indicator`: 增加 tool_progress 进度条和增量消息展示
- `streaming-incremental-render`: 新增 reasoning segment 类型和 iteration boundary 处理

## Impact

- 后端: `crates/xiaolin-agent/src/runtime/llm_call.rs`（emit reasoning_delta）
- 后端: `crates/xiaolin-agent/src/runtime/agent_step.rs`（转发 ToolRoundBoundary）
- 后端: `crates/xiaolin-protocol/src/event.rs`（新增 IterationBoundary 事件）
- 前端: `useMessageStreamChat.ts`（reasoning_delta handler + iteration state）
- 前端: `ThinkingIndicator.tsx` → `PhaseIndicator.tsx`（改造）
- 前端: `MessageRenderer.tsx`（替换 Typing 为 PhaseIndicator）
- 前端: `StepIndicator.tsx`（消费 tool_progress）
- 前端: 新组件 `ReasoningBlock.tsx`
- 前端: `types.ts`（StreamSegment 新 type）
