## 1. Goal 持久化基础

- [x] 1.1 在 xiaolin-core 的 SQLite migration 中添加 `goals` 表（id, session_id, description, status, token_budget, tokens_used, time_used_seconds, created_at, updated_at）
- [x] 1.2 在 xiaolin-core 中实现 GoalRepository trait（CRUD + query by session_id + update_tokens + update_status）
- [x] 1.3 重构 `GoalStore`：从内存 `Vec<Goal>` 改为包装 `GoalRepository`，所有操作走 SQLite
- [x] 1.4 确保 session 删除时级联删除关联 goal（FK ON DELETE CASCADE）

## 2. Token & 时间 Accounting

- [x] 2.1 在 `Goal` struct 中增加 `time_used_seconds` 字段
- [x] 2.2 实现 `goal_token_delta()` 函数：delta = (input_tokens - cached_input_tokens) + output_tokens
- [x] 2.3 在 `run_query()` 的 LLM response 处理后调用 `goal_store.add_tokens(goal_id, delta)` 累计 token
- [x] 2.4 在 turn 开始/结束时记录 wall-clock 时间，累计到 goal 的 `time_used_seconds`
- [x] 2.5 Budget 校验：create_goal 时验证 token_budget > 0（如果提供了的话）

## 3. 自动续轮循环

- [x] 3.1 在 `stop_hooks.rs` 中新增 `check_active_goal()` hook 函数
- [x] 3.2 实现 hook 逻辑：检查 GoalStore 有 active goal → 返回 should_continue=true + continuation prompt
- [x] 3.3 在 `check_active_goal()` 中增加 Plan mode 跳过逻辑：`if execution_mode == Plan { return None }`
- [x] 3.4 将 goal hook 加入 `evaluate_stop_hooks()` 的 hook 链（优先级在 todo hook 之前）
- [x] 3.5 在 `run_query()` 中为 goal continuation 维护一个 round counter，达到 MAX_ROUNDS（50）时自动 pause
- [x] 3.6 处理用户中断：检测到用户新消息或 stop 信号时，将 active goal 设为 paused
- [x] 3.7 限制 `update_goal` 工具 model 可设置的状态为 completed/failed（移除 cancelled）

## 4. Prompt Templates

- [x] 4.1 创建 `goal_continuation.md` 模板：包含 objective、token budget/used/remaining、completion audit 指引
- [x] 4.2 创建 `goal_budget_limit.md` 模板：引导 model wrap up、总结进度、识别 remaining work
- [x] 4.3 实现 `escape_xml_text()` 工具函数用于转义 objective 中的 < > & 字符
- [x] 4.4 实现 prompt 渲染函数：接收 Goal struct，填充模板变量，返回完整 prompt 字符串
- [x] 4.5 在 budget 到达时注入 budget_limit prompt 到当前 conversation

## 5. Protocol 层扩展

- [x] 5.1 在 xiaolin-protocol 中定义 `GoalUpdated` 事件类型（包含 goal 全量数据）
- [x] 5.2 在 xiaolin-protocol 中定义 `GoalCleared` 事件类型
- [x] 5.3 在 gateway 的 WebSocket handler 中支持转发 goal 事件到前端
- [x] 5.4 在 goal 状态变更时（create/update/delete/accounting）发送 GoalUpdated 事件

## 6. 前端 Goal 状态展示

- [x] 6.1 创建 `GoalStatusCard` React 组件：显示 description（截断）、status badge、token 进度条
- [x] 6.2 实现 Pause / Resume / Clear 按钮的前端交互
- [x] 6.3 在 chat store（zustand）中添加 goal 状态管理
- [x] 6.4 监听 WebSocket GoalUpdated/GoalCleared 事件，实时更新 UI
- [x] 6.5 在 ChatView 中集成 GoalStatusCard（active goal 时显示在聊天区域顶部）

## 7. 后端 Goal 操控 API

- [x] 7.1 在 gateway 中实现 `goal/pause` 命令（用户侧暂停 active goal）
- [x] 7.2 在 gateway 中实现 `goal/resume` 命令（恢复 paused goal 并触发 continuation）
- [x] 7.3 在 gateway 中实现 `goal/clear` 命令（删除当前 goal）
- [x] 7.4 Session 恢复时加载持久化 goal 状态（active → paused，其他保持）

## 8. 测试与验证

- [x] 8.1 单元测试：GoalRepository CRUD + cascade delete
- [x] 8.2 单元测试：goal_token_delta 计算逻辑
- [x] 8.3 单元测试：stop hook 的 goal continuation 触发/不触发条件
- [x] 8.4 单元测试：budget limit 检测和 prompt injection
- [x] 8.5 集成测试：完整的 goal lifecycle（create → auto-continue → complete）
- [x] 8.6 集成冒烟：cargo check + clippy 通过（1 个预存在的 migration 测试失败，非本次变更引入）
