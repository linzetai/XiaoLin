## ADDED Requirements

### Requirement: execute_as_stream returns composable Stream

`AgentRuntime` SHALL 提供 `execute_as_stream(ctx: AgentContext) -> impl Stream<Item=AgentStep>` 方法，使 agent 执行产出可迭代的事件流。

#### Scenario: Basic LLM turn without tools
- **WHEN** 调用 `execute_as_stream` 且 LLM 返回纯文本（无 tool_calls）
- **THEN** stream 依次 yield `AgentStep::TurnStart` → 多个 `AgentStep::Delta(text)` → `AgentStep::TurnEnd(reason: "completed")`

#### Scenario: LLM turn with tool calls
- **WHEN** LLM 返回包含 tool_calls 的 response
- **THEN** stream yield `Delta` → `ToolExecuting { name, args }` → `ToolResult { name, result }` → 继续下一轮 LLM 调用

#### Scenario: Multi-turn loop with tool rounds
- **WHEN** LLM 连续产出 tool_calls（多轮工具调用）
- **THEN** stream 在每个 tool-round boundary 完成所有 tool results 后再开始下一轮 LLM 调用，保持正确的消息顺序

### Requirement: AgentStep enum covers all execution events

`AgentStep` 枚举 SHALL 覆盖 agent 执行全生命周期事件。

#### Scenario: Event type coverage
- **GIVEN** AgentStep 枚举
- **THEN** 至少包含: `TurnStart`, `Delta(ContentDelta)`, `ToolExecuting { id, name, args }`, `ToolResult { id, name, result, success }`, `TurnEnd { reason, summary }`, `Error(anyhow::Error)`

### Requirement: AgentContext consolidates parameters

所有 `execute_unified` 的 13+ 参数 SHALL 合并为单一 `AgentContext` struct。

#### Scenario: Context construction
- **GIVEN** 调用方需要执行 agent
- **WHEN** 构建 `AgentContext`
- **THEN** 必须提供 `config`, `request`, `tool_registry`；其他字段为 Optional

### Requirement: execute_unified backward compatibility

现有 `execute_unified` API SHALL 保留为兼容层，内部调用 `execute_as_stream` 并 collect。

#### Scenario: Existing callers unchanged
- **GIVEN** gateway/session_bridge 等调用 `execute_unified`
- **WHEN** 重构完成后
- **THEN** 所有现有调用方无需修改，行为不变

#### Scenario: Event forwarding to mpsc channel
- **GIVEN** `execute_unified` 接收 `tx: mpsc::Sender<AgentEvent>`
- **WHEN** 内部 stream 产出 `AgentStep`
- **THEN** 兼容层将每个 step 转换为对应的 `AgentEvent` 并 send 到 tx

### Requirement: Stream cancellation via drop

Stream 被 drop 时 SHALL 优雅终止当前执行。

#### Scenario: Parent drops child stream
- **WHEN** 父级 agent 不再需要子 agent 结果（如超时），drop stream
- **THEN** 内部 LLM 调用被取消（abort），已分配的资源被释放，SpawnReservation 通过 RAII drop 释放 slot

### Requirement: Tool-round boundary is explicit in stream

Stream 在每个 tool round 结束后 SHALL yield 一个 boundary marker，内部按固定顺序执行注入。

#### Scenario: Boundary detection
- **WHEN** 所有 tool results 收集完毕、下一次 LLM 调用之前
- **THEN** stream yield `AgentStep::ToolRoundBoundary`
- **AND** 按以下顺序处理：
  1. 检查 abort/cancel 信号
  2. drain MessageQueue（priority <= Next）
  3. 将 drained messages 作为 user messages 追加到对话历史
  4. yield `AgentStep::SteeringInjected` (如有注入)
  5. 继续下一轮 LLM 调用
- **NOTE** 此顺序参考 claude-code query.ts L1530-1773：abort check → drain queue → inject attachments → update state → continue

### Requirement: Internal state tracks transition reason

Stream 内部 SHALL 维护 transition state，记录为何继续循环。

#### Scenario: Normal tool-round continuation
- **WHEN** LLM 返回 tool_calls，工具执行完毕
- **THEN** 内部 state 记录 `transition_reason: "next_turn"`

#### Scenario: TurnEnd exposes reason
- **WHEN** stream 即将终止
- **THEN** yield `AgentStep::TurnEnd { reason }` 其中 reason 可为: "completed", "max_turns", "cancelled", "error"

### Requirement: needsFollowUp determined by tool_use presence

是否继续循环 SHALL 仅由 LLM response 中是否存在 `tool_use` 块决定。

#### Scenario: No tool_use blocks
- **WHEN** LLM streaming 完成后，response 中无 `tool_use` content block
- **THEN** `needs_follow_up = false`，进入终止路径

#### Scenario: Has tool_use blocks
- **WHEN** response 中存在至少一个 `tool_use` content block
- **THEN** `needs_follow_up = true`，执行工具后继续循环
- **NOTE** 不信任 LLM 的 stop_reason 字段（参考 claude-code: "stop_reason === 'tool_use' is unreliable"）

## MODIFIED Requirements

### Requirement: SpawnController reservation integrates with Stream lifecycle

SpawnReservation SHALL 与 stream 生命周期绑定。

#### Scenario: Reservation released on stream completion
- **WHEN** 子 agent stream 完成（自然结束或被 drop）
- **THEN** SpawnReservation 自动释放 global + session slots
