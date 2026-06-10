## Why

XiaoLin 当前的 goal 系统仅提供内存中的 CRUD 工具（get/create/update_goal），缺乏核心的**自动续轮循环**：agent 完成一个 turn 后不会自动继续推进 goal，token 使用量也没有实际跟踪。对标 Codex 的 goal 模式，XiaoLin 无法支持「设定目标 → agent 自治推进 → 预算控制 → 完成标记」的长程任务流程。这是支撑开发回归、多步骤自动化任务的基础能力。

## What Changes

- **自动续轮循环**：当 goal 处于 active 状态且 turn 结束时，agent 自动注入 continuation prompt 并开启下一轮，无需用户手动触发
- **Token / 时间 accounting**：在每个 turn 完成后，将实际消耗的 token 和 wall-clock 时间累计到 goal 记录中
- **预算控制与 steering**：当 token 使用达到 budget 上限时，注入 budget_limit prompt 引导 model 收尾，并停止自动续轮
- **Goal 持久化**：将 goal 数据从内存 Vec 迁移到 SQLite，支持跨 session 恢复
- **Prompt templates**：创建 continuation / budget_limit / objective_updated 三个 prompt 模板，引导 model 正确推进和完成目标
- **前端 Goal 状态展示**：在聊天界面展示当前 goal 状态（active/paused/complete）、token 使用量和进度
- **用户操控**：支持 pause / resume / edit / clear goal 操作

## Capabilities

### New Capabilities
- `goal-continuation-loop`: 自动续轮循环机制 — turn 完成后检测 active goal 并注入 continuation prompt 自动开启下一轮
- `goal-token-accounting`: Token 和时间消耗跟踪 — 在 turn lifecycle 中桥接 LLM response 的 token usage 到 goal store
- `goal-budget-steering`: 预算控制与 prompt steering — budget 到达时注入 wrap-up prompt 并停止自动续轮
- `goal-persistence`: Goal 数据持久化 — SQLite 存储 + 跨 session 恢复
- `goal-prompt-templates`: 三个 prompt 模板（continuation / budget_limit / objective_updated）的设计与注入
- `goal-frontend-status`: 前端 goal 状态展示 — 进度条、状态标签、token 使用量

### Modified Capabilities
- `stop-interrupt`: 在 stop hook 机制中增加 goal-based continuation hook，在 agent 中断时自动 pause active goal

## Impact

- **后端 crates**: `xiaolin-agent`（stop_hooks、runtime loop、goal store 重构）、`xiaolin-core`（SQLite schema 扩展）
- **前端**: 聊天界面新增 goal status 组件
- **Protocol**: `xiaolin-protocol` 新增 goal 相关的 WebSocket 事件（GoalUpdated、GoalCleared）
- **E2E 测试**: 现有 `11-goal-todo.ts` 需要扩展覆盖自动续轮和预算控制场景
