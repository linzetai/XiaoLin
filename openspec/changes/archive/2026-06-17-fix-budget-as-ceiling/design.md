# Design: Token Budget CEILING-only 模式

## 架构决策

### AD-1: Budget 角色重定义

```
之前:
  Model end_turn → budget < target? → YES → 续命（FLOOR）
  Model end_turn → budget >= target? → YES → 停止

之后:
  Model end_turn → 直接停止（信任 model）
  Model 还在运行 → budget > 120% target? → YES → hard stop（CEILING）
  Model 还在运行 → budget > 100% target? → YES → soft nudge（提示收尾）
```

### AD-2: BudgetTracker 简化

移除 `BudgetDecision::Continue` 和 `BudgetDecision::ForceStopAfterNext`。
只保留 `BudgetDecision::WithinBudget` 和 `BudgetDecision::BudgetMet`。

实际上 `BudgetTracker::check()` 在 end_turn 中不再被调用——它只在
iteration_check 中作为 CEILING 检查使用。

### AD-3: System Prompt 修正

```diff
- The user specified a token target of {N} tokens. Your output token count
- will be tracked each iteration. Keep working until you approach the target —
- plan your work to fill it productively. The target is a hard minimum, not a
- suggestion. If you stop early, the system will automatically continue you.

+ The user set a token budget of {N} tokens as a safety ceiling.
+ Complete your task naturally — stop when done, do not pad output.
+ If you approach the budget limit, wrap up your current work.
```

### AD-4: 保留 CEILING 机制（iteration_check.rs 不变）

```rust
// iteration_check.rs — 每次 LLM 调用前检查
if pct >= 120 {
    // hard stop: 注入强制终止消息
    ms.query_loop.force_stop_after_next = true;
} else if pct >= 100 && !tracker.soft_nudge_sent {
    // soft nudge: 注入收尾提示
    tracker.soft_nudge_sent = true;
}
```

这部分逻辑正确且必要——它防止 agent 在一个 turn 中无限消耗 token。

### AD-5: end_turn.rs 中移除 budget continuation

```rust
// 移除这整个分支:
// let should_continue_for_budget = if !hook_result.should_continue {
//     if let Some(ref mut tracker) = ms.budget_tracker {
//         match tracker.check(output_tokens) {
//             BudgetDecision::Continue { ... } => true,
//             ...
//         }
//     }
// };
```

当 stop hooks 说"停止"且 model 说 end_turn 时，就应该停止。
Budget 不应该推翻这个决定。

### AD-6: Goal Mode 负责跨 turn 衔接

Goal mode 的 inter-turn 逻辑已经存在且正确：
1. `check_active_goal` stop hook 检查 goal 状态
2. 如果 goal 仍然 Active 且有 pending TODOs → 续轮
3. 这基于任务完成度，不是 token 计数

## 边界情况

| 场景 | 预期行为 |
|------|---------|
| `+1k 创建文件` → 文件创建后 model end_turn | 直接停止，不续命 |
| `+500k 做 markdown viewer` + goal mode | Model 自然停 → goal 判断未完成 → 续轮 |
| 无 budget 的普通消息 | 和以前一样（无 budget tracker） |
| `+20k` 的中型任务，model 做了 50 个 tool calls | 到 100% nudge → 120% hard stop |
| `+1k` 但 model 一直不停（工具循环） | 100% nudge → 120% hard stop（安全阀生效） |

## 实现顺序

1. 修改 `turn_setup.rs` 中的 system prompt 文案
2. 移除 `end_turn.rs` 中的 budget continuation 分支
3. 简化 `token_budget.rs`（删除 Continue 相关代码）
4. 保留 `iteration_check.rs` CEILING 逻辑不变
5. cargo check + clippy 验证
