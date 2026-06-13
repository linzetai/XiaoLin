# Proposal: Token Budget 定位修正 — 从 FLOOR 改为 CEILING

## 问题

当前 token budget 实现有根本性设计缺陷：当 agent 在任务完成后自然说 "end_turn"，
budget continuation 会强制 agent 继续产出 token，导致 agent 在无明确目标时做无关探索。

**复现：** `+1k 创建一个文件` → agent 创建文件后，budget 55% → 续命 → agent 开始
浏览项目结构（与任务无关）→ 工作目标跑偏。

## 根因

Token budget 被设计为 FLOOR（下限 = 必须填满），而非 CEILING（上限 = 超了才停）。
这与 "让 agent 自主完成任务" 的目标矛盾。

## 方案

### 1. 移除 budget 作为 FLOOR 的续命逻辑

`end_turn.rs` 中，当 model 说 end_turn 时：
- **当前行为：** budget < 90% → 强制续命
- **目标行为：** 信任 model 的 end_turn，不续命

### 2. 保留 budget 作为 CEILING 的停止逻辑

`iteration_check.rs` 中，每次 iteration 前检查：
- **保留：** budget 100% soft nudge + 120% hard stop
- 这是安全阀，防止单 turn 无限运行

### 3. 修正 system prompt

- **移除：** "The target is a hard minimum... the system will automatically continue you"
- **改为：** "You have a token budget of X. This is a safety ceiling — stop naturally when
  your task is complete. Do NOT pad output to reach the target."

### 4. Goal 层负责跨 turn 续命（已有机制）

Goal mode 下 turn 结束后：
- `check_active_goal` stop hook 判断 goal 是否完成
- 未完成 → 自动续轮（现有逻辑，不变）
- 这是基于**任务状态**的续命，不是基于 token 计数

## 影响范围

| 文件 | 变更 |
|------|------|
| `crates/xiaolin-agent/src/runtime/end_turn.rs` | 移除 `BudgetDecision::Continue` 分支 |
| `crates/xiaolin-agent/src/runtime/turn_setup.rs` | 修改 budget system prompt |
| `crates/xiaolin-agent/src/runtime/token_budget.rs` | 移除 `BudgetTracker::check()` 中的 Continue 逻辑 |
| `crates/xiaolin-agent/src/runtime/iteration_check.rs` | 保留 soft nudge + hard stop（不变） |
| 前端 `useMessageStreamChat.ts` | 保留 budget_reached UI（不变） |

## 非目标

- 不删除 token budget 功能本身（解析、注入、CEILING 保护仍有价值）
- 不修改 goal mode 的 inter-turn 逻辑（那部分是对的）
- 不修改 diminishing returns 检测（可保留作为 CEILING 侧的辅助判断）
