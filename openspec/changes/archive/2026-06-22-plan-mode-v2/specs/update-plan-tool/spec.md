## ADDED Requirements

### Requirement: update_plan 结构化步骤跟踪工具

Agent SHALL 拥有 `update_plan` 内置工具，用于维护结构化的步骤列表并实时推送到前端。该工具在 Plan 模式和 Agent 模式下均可用。

#### Scenario: 工具参数定义
- **GIVEN** `update_plan` 工具被注册到 ToolRegistry
- **THEN** 参数 schema 为 `{ steps: PlanStep[], explanation?: string }`
- **AND** `PlanStep` 为 `{ step: string, status: "pending" | "in_progress" | "completed" }`
- **AND** `steps` 必填，`explanation` 可选
- **AND** steps 最多 15 条，最少 1 条

#### Scenario: 工具执行流程
- **WHEN** agent 调用 `update_plan` 并传入有效参数
- **THEN** SHALL 更新 `PlanStepStore` 的内存状态
- **AND** SHALL 通过 `EventTxMap` 发送 `AgentEvent::PlanUpdate` 事件到前端
- **AND** SHALL 返回进度摘要（如 "Plan updated: 2/5 completed, 1 in progress."）

#### Scenario: 事件传输
- **WHEN** `PlanUpdate` 事件被序列化
- **THEN** JSON 格式为 `{ "type": "plan_update", "turn_id": "...", "session_id": "...", "explanation": "...", "steps": [...] }`
- **AND** `PlanStepStatus` 序列化为 snake_case（`"pending"`, `"in_progress"`, `"completed"`）

#### Scenario: Task-local 上下文要求
- **WHEN** `update_plan` 在 `tokio::spawn` 的子任务中被调用
- **THEN** SHALL 通过 `ASK_QUESTION_STREAM_KEY` task-local 获取 stream key
- **AND** 若 task-local 不可用则返回错误 "update_plan not available outside chat stream context"

### Requirement: 前端 PlanChecklist 渲染

PlanPanel SHALL 在收到 `plan_update` 事件时渲染结构化 checklist 视图。

#### Scenario: Checklist 基本布局
- **WHEN** 收到 `plan_update` 事件且 steps 非空
- **THEN** PlanPanel 顶部 SHALL 显示 PlanChecklist 组件
- **AND** 包含：进度条（百分比）、步骤列表、可选 explanation

#### Scenario: 步骤状态图标
- **GIVEN** 每个步骤有 3 种状态
- **THEN** `pending` → 空心圆圈（Circle）
- **AND** `in_progress` → 旋转圆圈（CircleNotch，spin 动画）
- **AND** `completed` → 实心对勾圆（CheckCircle，绿色）

#### Scenario: 步骤文本样式
- **WHEN** 步骤状态为 `completed`
- **THEN** 文本颜色降为 `--fill-tertiary` 并添加删除线
- **WHEN** 步骤状态为 `in_progress`
- **THEN** 文本颜色为 `--plan-tint` 且 font-weight 500
- **WHEN** 步骤状态为 `pending`
- **THEN** 文本颜色为 `--fill-secondary`

#### Scenario: 进度条
- **WHEN** 有 N 个 completed 步骤，总计 M 步
- **THEN** 进度条宽度为 `(N/M * 100)%`
- **AND** 颜色为 `--plan-tint`
- **AND** 右侧显示 `N/M` 文字

#### Scenario: Checklist 与 Plan 文件内容共存
- **WHEN** 同时存在 plan_update 步骤和 plan 文件内容
- **THEN** checklist 显示在文件内容之上
- **AND** 两者独立更新互不干扰

### Requirement: update_plan 在 Plan 模式和 Agent 模式均可用

#### Scenario: Plan 模式下不被阻塞
- **GIVEN** `update_plan` 的 ToolKind 为 `Think`
- **AND** Plan 模式仅阻塞 `Edit` 和 `Execute` 类型工具
- **THEN** `update_plan` 在 Plan 模式下正常执行

#### Scenario: Agent 模式下追踪实施进度
- **WHEN** agent 在 Agent 模式执行多步骤任务
- **THEN** agent 可调用 `update_plan` 实时更新步骤状态
- **AND** 前端 PlanPanel（若打开）实时显示进度

### Requirement: 工具 prompt 指引

Agent 的 Plan 模式 prompt 中 SHALL 明确列出 `update_plan` 为可用工具。

#### Scenario: enter_plan_mode prompt 列出 update_plan
- **WHEN** agent 查看 enter_plan_mode 的 prompt
- **THEN** "Available tools (read-only)" 列表中 SHALL 包含 `update_plan (structured step tracking with status)`

## Implementation Reference

### 架构决策

**为什么选择 `update_plan` 工具而非 Markdown 解析？**

| 维度 | Markdown 解析方案 | update_plan 工具方案（采用） |
|------|------------------|---------------------------|
| 稳定性 | 依赖 LLM 输出格式，容易因格式偏差失败 | 结构化 JSON schema，类型安全 |
| 实时性 | 需等文件写完后解析 | 每次工具调用即时推送前端 |
| 与竞品对标 | 无先例 | Codex CLI 采用相同模式 |
| 前端实现 | 复杂的正则/AST 解析 | 直接渲染 typed object |
| 局限性 | 仅适用于有 plan 文件的场景 | 任何多步骤任务都可使用 |

**与 Codex CLI 的对比：**

| 特性 | Codex `update_plan` | XiaoLin `update_plan` |
|------|---------------------|----------------------|
| 参数 | `{ steps: [{title, status}] }` | `{ steps: [{step, status}], explanation? }` |
| 状态值 | `pending/in-progress/completed` | `pending/in_progress/completed` |
| 持久化 | 内存 + terminal title | `PlanStepStore` (内存) + WS 事件 |
| 前端渲染 | Terminal spinner/checkmark | PlanPanel checklist + 进度条 |
| 模式限制 | 无（始终可用） | 无（Think 类工具，不被 Plan 模式阻塞） |

### 数据流

```
Agent LLM → tool_call(update_plan, {...})
  → UpdatePlanTool::execute()
    → PlanStepStore::update() (内存写入)
    → ASK_QUESTION_STREAM_KEY.try_with() (获取 stream key)
    → EventTxMap.get(stream_key) (获取 sender)
    → tx.send(AgentEvent::PlanUpdate { ... })
      → WebSocket → 前端 onWsEvent("plan_update")
        → setPlanSteps() / setPlanExplanation()
          → <PlanChecklist /> 重新渲染
```

### 文件清单

| 文件 | 变更类型 | 职责 |
|------|---------|------|
| `xiaolin-protocol/src/event.rs` | Modified | `PlanStepStatus`, `PlanStep`, `AgentEvent::PlanUpdate` |
| `xiaolin-protocol/src/lib.rs` | Modified | Re-export `PlanStep`, `PlanStepStatus` |
| `xiaolin-agent/src/builtin_tools/update_plan.rs` | Created | `UpdatePlanTool`, `PlanStepStore` |
| `xiaolin-agent/src/builtin_tools/mod.rs` | Modified | `pub mod update_plan`, pub use, register fn |
| `xiaolin-agent/src/builtin_tools/plan_mode.rs` | Modified | prompt 添加 update_plan |
| `xiaolin-gateway/src/state/builder.rs` | Modified | 注册 update_plan 工具 |
| `xiaolin-app/src/lib/stores/types.ts` | Modified | `PlanStep`, `PlanStepStatus`, `PlanUpdateData` |
| `xiaolin-app/src/lib/transport.ts` | Modified | 添加 "plan_update" 事件 |
| `xiaolin-app/src/components/.../useMessageStreamChat.ts` | Modified | plan_update case |
| `xiaolin-app/src/components/.../PlanPanel.tsx` | Modified | `PlanChecklist`, `StepIcon` |
