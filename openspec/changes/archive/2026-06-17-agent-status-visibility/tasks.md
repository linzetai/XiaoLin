## 1. Backend: Reasoning Delta 流式发送

- [x] 1.1 在 `llm_call.rs` 中 emit `AgentStep::ReasoningDelta`（line 457-459 附近，reasoning_content 积累后 send_step）
- [x] 1.2 验证 `ReasoningDelta` 在 `into_agent_events()` 中正确转为 `AgentEvent::ReasoningDelta`（已实现，无需修改）
- [x] 1.3 确认 reasoning_delta 在 WebSocket chat.rs 中不被过滤（已透传，无需修改）

## 2. Backend: Iteration Boundary 事件

- [x] 2.1 在 `event.rs` 中添加 `IterationBoundary { turn_id, iteration: u32 }` 变体到 `AgentEvent` enum
- [x] 2.2 修改 `agent_step.rs` 的 `into_agent_events()`：`ToolRoundBoundary { turn_id, iteration }` → `vec![AgentEvent::IterationBoundary { turn_id, iteration }]`
- [x] 2.3 在 `iteration_check.rs` 的 `begin_iteration()` 后 emit `ToolRoundBoundary`

## 3. Frontend: reasoning_delta Handler

- [x] 3.1 在 `types.ts` 的 `StreamSegment` 中添加 `type: "reasoning"` 变体
- [x] 3.2 在 `useMessageStreamChat.ts` 中添加 `reasoning_delta` case：创建/追加 reasoning segment
- [x] 3.3 reasoningTokenCount 由 ReasoningBlock 组件直接从 content.length 计算（无需单独 state）

## 4. Frontend: ReasoningBlock 组件

- [x] 4.1 创建 `ReasoningBlock.tsx`：折叠式块，默认收起显示 "思考中... ▼ N chars"
- [x] 4.2 展开态渲染 reasoning 原文（monospace、opacity-70）
- [x] 4.3 当 content_delta 或 tool_executing 到来时自动折叠（autoCollapse prop）
- [x] 4.4 在 `MessageRenderer.tsx` 的 streaming row 和 AiMessage 中渲染 reasoning segment

## 5. Frontend: PhaseIndicator 组件

- [x] 5.1 改造 `ThinkingIndicator.tsx` 为接受 `phase` prop 的 `PhaseIndicator`
- [x] 5.2 实现 phase 状态机：connecting → thinking → planning
- [x] 5.3 在 `MessageRenderer.tsx` 中替换 `Typing()` 为 `PhaseIndicator`，根据 segments 状态选择 phase
- [x] 5.4 添加 i18n key: thinking_connecting, thinking_planning（内联 fallback）

## 6. Frontend: Iteration 计数器

- [x] 6.1 在 `useMessageStreamChat.ts` 中添加 `iteration_boundary` handler
- [x] 6.2 在 `types.ts` 中添加 `type: "iteration_boundary"` segment 变体
- [x] 6.3 在 `MessageRenderer.tsx` streaming row 中渲染迭代分隔线 + "Step N" 标记
- [x] 6.4 `StepGroup` 的 `groupConsecutiveSegments` 识别迭代分隔并输出为独立 group

## 7. Frontend: StepIndicator 消费 tool_progress

- [x] 7.1 在 `useStreamStore` 中添加 `toolProgress: Record<string, { progress?: number; message?: string }>` 
- [x] 7.2 在 `useMessageStreamChat.ts` 中将 `tool_progress` 事件写入 store（按 call_id）
- [x] 7.3 `StepIndicator` 通过 selector 订阅对应 call_id 的 progress
- [x] 7.4 渲染进度条（progress 字段）和/或消息文本（message 字段）

## 8. 验证与清理

- [x] 8.1 删除未使用的 `Typing()` 组件
- [x] 8.2 cargo clippy + tsc --noEmit 零警告
- [x] 8.3 通过 Tauri MCP 验证：发送消息 → 观察 PhaseIndicator 状态转换
- [x] 8.4 使用支持 reasoning 的模型验证 ReasoningBlock 显示
