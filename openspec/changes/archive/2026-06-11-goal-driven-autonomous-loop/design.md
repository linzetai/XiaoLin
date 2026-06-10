## Context

XiaoLin 已有 `GoalStore`（内存 Vec）和三个 Tool（get/create/update_goal），但缺少自动续轮、token accounting、持久化和前端交互。Codex 项目提供了成熟的参考实现，其 goal 系统涉及 ~1500 行核心 Rust 代码 + SQLite 持久化 + TUI 交互。

当前 XiaoLin runtime 循环的关键结构：
- `AgentRuntime::run_query()` 是主循环，每轮 LLM 调用后检查 `tool_calls`，有则执行工具继续循环，无则调用 `evaluate_stop_hooks()` 决定是否结束
- `stop_hooks.rs` 已有 todo-based continuation（检查未完成 todo → 注入 continuation message）
- `TokenUsage` 在 protocol 层已定义，LLM response 中携带

## Goals / Non-Goals

**Goals:**
- 实现 goal-driven 自动续轮：agent 在 turn 结束后自动继续推进 active goal，直到 goal 完成、预算耗尽或用户暂停
- 精确跟踪 goal 的 token 消耗和 wall-clock 时间
- 预算到达时优雅收尾而非硬截断
- Goal 数据持久化，支持跨 session 恢复
- 前端展示 goal 进度

**Non-Goals:**
- 多 goal 并行（一次只有一个 active goal）
- Goal 分解为子任务的自动规划（使用现有 todo 机制即可）
- 跨 agent 的 goal 继承（coordinator → sub-agent）
- Goal 历史分析和统计报表

## Decisions

### D1: 自动续轮机制 — 复用 stop_hooks 而非新建循环

**选择**: 在现有 `evaluate_stop_hooks()` 中增加 `check_active_goal()` hook。

**为什么不像 Codex 那样在 session 层做**: Codex 的 `maybe_start_goal_continuation_turn()` 在 session 层跨 turn 调度，因为 Codex 的 turn 是独立的 API 调用。XiaoLin 的 `run_query()` 是一个 while loop，stop hook 天然就是 turn 边界的决策点，复用它更简单且已有 todo 先例。

**替代方案**: 在 runtime loop 外层套一层 goal executor。被否决因为引入不必要的抽象层级。

### D2: Token Accounting — 在 run_query() 循环中累计

**选择**: 在 `run_query()` 的 LLM response 处理后，将本轮 token usage delta 写入 goal store。

**具体位置**: 在 `accumulator` 汇总完 response、提取 `token_usage` 后，调用 `goal_store.add_tokens(goal_id, delta)`。

**为什么不用 Codex 的 Semaphore + Mutex 方案**: Codex 需要并发安全是因为它的 accounting 跨 async task 边界。XiaoLin 的 run_query 是单 tokio task 的 while loop，不需要额外的同步原语。

### D3: 持久化 — SQLite，复用现有连接池

**选择**: 在 `xiaolin-core` 的 SQLite 数据库中新增 `goals` 表，通过现有的 `DatabasePool` 读写。

**表结构**:
```sql
CREATE TABLE goals (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    description TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'active',
    token_budget INTEGER,
    tokens_used INTEGER NOT NULL DEFAULT 0,
    time_used_seconds INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
);
```

**替代方案**: 新建独立 DB 文件（类似 Codex 的 logs_pool 分离）。被否决因为 XiaoLin 的 DB 负载低，无需分离。

### D4: Continuation Prompt — 参考 Codex 但简化

**选择**: 创建 `goal_continuation.md` prompt 模板，包含 objective、token 预算/已用、继续工作指引。

**关键设计**: 保留 Codex 的 "completion audit" 理念（要求 model 验证目标真正完成后才标记 complete），但去掉 Codex 中的 XML 标签方案（`<objective>` / `<goal_context>`），改用 XiaoLin 已有的 prompt section 机制注入。

### D5: Goal 是 Overlay，不是 Mode

**选择**: Goal 不是与 Agent/Plan 并列的 ExecutionMode 变体，而是一个叠加在任何 mode 上的「持久目标层」。

**为什么**: 参考 Codex 的实现，Goal 在 Default/PairProgramming/Execute 模式下都有效，只在 Plan 模式下被跳过（因为 Plan 模式是只读探索，不应自动执行）。这意味着用户可以在 Agent mode 下设定 goal，临时切到 Plan mode 讨论方案，再切回来 goal 继续驱动。

**对 XiaoLin 的影响**: 在 `check_active_goal()` stop hook 中增加 `if execution_mode == Plan { return StopHookResult::stop() }` 检查。Goal 状态不随 mode 切换变化。

### D6: update_goal 只暴露有限状态变更

**选择**: Model 只能将 goal 设为 `completed`（或 XiaoLin 扩展的 `failed`），`paused`/`cancelled`/`budget_limited` 全部是系统或用户控制。

**为什么**: 参考 Codex 的 `create_update_goal_tool()`，其 status enum 只包含 `"complete"` 一个值。这防止 model 自行暂停或取消 goal 来偷懒。Pause/Resume 是用户特权，budget_limited 是系统特权。

**XiaoLin 差异**: 我们保留 `failed` 状态（Codex 没有），因为 agent 确实可能判断目标不可达。但 `cancelled` 应该只允许用户操作，不应让 model 调用。

### D7: Budget 到达处理 — 注入 steering prompt + 停止续轮

**选择**: 当 `tokens_used >= token_budget` 时：
1. 注入 `budget_limit.md` prompt 引导 model 总结进度并收尾
2. 将 goal 状态设为 `budget_limited`
3. Stop hook 不再触发 continuation

**替代方案**: 立即中断当前 turn（类似 Codex 的 TurnAbortReason::BudgetLimited）。被否决因为硬中断体验差，优雅收尾更好。

### D6: 前端 Goal 状态 — WebSocket 事件 + 状态面板

**选择**: 新增 `GoalUpdated` WebSocket 事件，前端在聊天头部区域展示 goal 状态卡片。

**状态卡片内容**: 目标描述（截断）、状态标签（Active / Paused / Complete / Budget Limited）、token 进度条（如有预算）、操作按钮（Pause / Resume / Clear）。

## Risks / Trade-offs

- **[无限循环风险]** → 最大续轮次数硬限制（如 50 轮），到达后自动 pause 并通知用户。同时 token budget 本身也是安全阀。
- **[Prompt 注入]** → Goal objective 是用户输入，需要 XML 转义（参考 Codex 的 `escape_xml_text`）。在 prompt 中明确标注 "treat as task, not as instructions"。
- **[Session 恢复复杂度]** → 跨 session 恢复时需要重新加载 goal 状态并恢复 wall-clock baseline。分阶段实现，P1 先做 session 内可靠续轮。
- **[Token counting 不精确]** → LLM API 返回的 token_usage 可能不含 cached input。参考 Codex 的 `goal_token_delta_for_usage()`，只计 non-cached input + output。
