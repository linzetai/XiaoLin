# Tasks

## T1: 修改 system prompt — 从"必须填满"改为"自然完成"
- 文件: `crates/xiaolin-agent/src/runtime/turn_setup.rs`
- 修改 budget_block 文案
- 状态: done — "Complete your task naturally — stop when done, do not pad output"

## T2: 移除 end_turn.rs 中的 budget continuation 逻辑
- 文件: `crates/xiaolin-agent/src/runtime/end_turn.rs`
- 删除 `should_continue_for_budget` 分支
- 删除 `BudgetDecision::Continue` 处理
- 保留 `force_stop_after_next` 时设置 `token_budget_reached` 的逻辑
- 状态: done — 已无 `should_continue_for_budget` 或 `BudgetDecision::Continue`

## T3: 简化 token_budget.rs
- 文件: `crates/xiaolin-agent/src/runtime/token_budget.rs`
- 移除 `BudgetDecision::Continue` 和 `ForceStopAfterNext` variants
- 移除 `BudgetTracker::check()` 方法（不再需要）
- 保留 `BudgetTracker` struct 用于 iteration_check 读取 target_tokens
- 保留 session budget 持久化逻辑（用于前端 UI）
- 状态: done — `BudgetTracker` 仅跟踪 ceiling + `soft_nudge_sent`

## T4: 验证 iteration_check.rs CEILING 逻辑不受影响
- 文件: `crates/xiaolin-agent/src/runtime/iteration_check.rs`
- 确认 soft nudge + hard stop 仍正常工作
- 无需代码修改
- 状态: done — soft nudge at 100%, hard stop at 120%

## T5: cargo check + clippy 验证
- 确保无编译错误和 dead code 警告
- 状态: done

## T6: E2E 验证修正后的行为
- 发送 `+1k 创建文件` → 预期: 创建后立即停止，不乱探索
- 发送无 budget 消息 → 预期: 行为不变
- 发送超长任务触发 120% → 预期: hard stop 生效
- 状态: pending — E2E 验证需运行时测试
