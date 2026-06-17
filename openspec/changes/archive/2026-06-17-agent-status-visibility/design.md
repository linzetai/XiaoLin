## Context

XiaoLin agent 使用双通道架构（step_tx + event_tx）将执行状态发送到前端。前端通过 WebSocket 订阅 `AgentEvent` 并在 `useMessageStreamChat.ts` 中分发到 UI。

当前问题：
- `AgentStep::ReasoningDelta` 已定义但 `llm_call.rs` 从未 emit
- `AgentStep::ToolRoundBoundary` emit 后在 `into_agent_events()` 中被过滤（返回空 vec）
- 前端有 `ThinkingIndicator` 组件但从未接入消息流
- `tool_progress` 事件仅转发到 terminal store，`StepIndicator` 不消费

## Goals / Non-Goals

**Goals:**
- 消除所有超过 500ms 的静默期，让用户始终能感知 agent 在工作
- 流式展示 reasoning 内容（折叠式，不干扰主输出）
- 用明确的阶段文字替代匿名弹跳圆点
- 显示迭代进度（Step N）让用户知道任务复杂度
- 对非 shell 工具也展示执行进度

**Non-Goals:**
- 不改变 agent 执行逻辑本身
- 不改变 sub-agent 监控面板（已有独立 UX）
- 不添加用户可配置的详细日志级别
- 不改变 content_delta 的流式渲染（已有光标动画）

## Decisions

### D1: Reasoning 展示策略

**选择:** 折叠式块（默认收起，仅显示"思考中... ▼ N tokens"）

**替代方案:**
- A) 始终展开 → 太吵，reasoning 往往很长且重复
- B) 完全不展示只用状态文字 → 对好奇型用户不够
- C) 侧边栏展示 → 增加布局复杂度

**理由:** 折叠默认收起给出"正在思考"的信号，同时允许 power user 展开查看。当 content 或 tool 输出到来时自动折叠，保持主流干净。

### D2: Phase Indicator 设计

**选择:** 改造现有 `ThinkingIndicator` 为带 `phase` prop 的组件

**Phase 状态机:**
```
turn_start → "connecting"（300ms 延迟后才显示）
reasoning_delta 到达 → "thinking"
content_delta 到达 → 隐藏（markdown cursor 已足够）
tool_executing → 隐藏（StepIndicator 接管）
tool_result + 无 content → "planning"
```

**理由:** 复用现有组件减少新增代码量；300ms 延迟避免快速响应时闪烁。

### D3: ToolRoundBoundary 转发策略

**选择:** 新增 `IterationBoundary` 事件类型（而非复用现有 event）

**理由:** `ToolRoundBoundary` 是内部实现细节（iteration 计数），转为语义化的 `IterationBoundary { iteration: u32 }` 更清晰，前端可直接使用。

### D4: tool_progress 消费方式

**选择:** StepIndicator 通过全局 store 订阅 tool_progress（按 call_id 匹配）

**替代方案:**
- A) Props 从 useMessageStreamChat 透传 → 需要改动多层组件签名
- B) 新 Zustand store → 简单但增加 store 数量

**理由:** 利用现有 `useStreamStore` 的 toolProgress map，StepIndicator 通过 selector 按 call_id 获取。

### D5: Reasoning 丢弃策略（channel backpressure）

**选择:** `ReasoningDelta` 标记为 lossy（可在背压下丢弃）

**理由:** reasoning 是补充信息，丢几个 chunk 不影响功能。与 content_delta 的 lossy 策略一致。

## Risks / Trade-offs

- **[性能] reasoning_delta 高频发送** → 使用 lossy channel + 前端 RAF 节流渲染，与 content_delta 相同路径
- **[UX] phase 切换闪烁** → 300ms debounce 阈值；快速操作不显示中间状态
- **[兼容性] 不支持 reasoning 的模型** → phase 直接从 connecting 跳到 StepIndicator/content，reasoning block 不出现
- **[数据量] reasoning 文本很长** → 折叠块设 maxHeight + 虚拟滚动；超过阈值截断显示
