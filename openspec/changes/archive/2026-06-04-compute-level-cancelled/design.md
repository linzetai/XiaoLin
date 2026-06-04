## Context

当前算力/模型路由层次：

1. **`ComplexityTier`**（`xiaolin-core/src/complexity.rs`）：5 档离散复杂度（Tiny → Frontier），用于模型候选过滤
2. **`AgentConfig.min_tier` / `max_tier`**：Agent 级配置，限制 `ModelRouter` 可选模型窗口（`RouteTierConstraints`）
3. **`ModelRouter`**（`xiaolin-model-router`）：根据 workload 估计 + `agent_min_tier` / `agent_max_tier` 计算 `(floor, cap)` 窗口后筛选候选模型
4. **`ModelSelector`**（前端 `StreamFooter.tsx`）：用户手动选择具体模型，与 tier 约束独立

原型图 InputBar 工具栏顺序：`+` | 权限 | 刷新 | 模型 | **Extra High ▾** | 附件 | 发送。计算等级选择器填补「用户可调算力下限」这一缺口，不替代模型选择器。

依赖关系：
- 依赖 `layout-overhaul` 的 InputBar 工具栏布局
- 模式与 `permission-presets` 的 per-session 覆盖一致，可复用同类 WS / store 模式

## Goals / Non-Goals

**Goals:**
- 用户可通过 InputBar 一键切换算力档位，无需编辑 `config.toml`
- 5 档友好标签映射到现有 `ComplexityTier`，默认「High」（Medium tier）平衡质量与速度
- Per-session 覆盖，session 切换时 UI 与后端状态同步
- 算力变更在下一轮 turn 的路由中生效（与权限预设一致）

**Non-Goals:**
- 不持久化 session 算力覆盖到 SQLite（有意识选择，关闭 session 重置）
- 不替代 `ModelSelector` 的具体模型选择
- 不修改 `max_tier`（算力档位仅抬高 `min_tier` 下限，不设上限 cap）
- 不做 per-turn 算力（仅 session 粒度）

## Decisions

### D1: 用户标签与 ComplexityTier 映射

**决策**：定义 `ComputeLevel` 枚举，5 个用户面向标签一一映射到 `ComplexityTier`：

| 用户标签 | `ComputeLevel` ID | `ComplexityTier` | 定位 |
|----------|-------------------|------------------|------|
| Low | `low` | Tiny | 最快，适合简单问答 |
| Medium | `medium` | Small | 较快，轻量单步任务 |
| High | `high` | Medium | **默认**，质量与速度平衡 |
| Extra High | `extra_high` | Large | 深度分析、大改动 |
| Max | `max` | Frontier | 最强算力，成本最高 |

**理由**：与 Codex 原型「Extra High」命名一致；内部仍用已有 `ComplexityTier`，无需改 model router 核心类型。
**替代方案**：直接暴露 Tiny/Frontier 等技术名。否决，对普通用户不友好。

### D2: Per-session 覆盖机制

**决策**：在 Session 关联状态中增加 `compute_level_override: Option<ComputeLevel>`，仅存内存。

```
effective_min_tier(session) =
  session_override.map(|l| l.to_tier())
    .or(global_default_compute_level.to_tier())
```

- 有 override → 用 override 映射的 tier 作为 `agent_min_tier`
- 无 override → 用全局默认（`high` → Medium）
- Session 关闭或应用重启后 override 清除

**理由**：与 `permission-presets` 的 D2 一致，用户在不同对话中可有不同算力需求；不持久化避免无意留下高成本默认值。
**替代方案**：写入 `config.toml`。否决，算力应为对话中的临时意图而非全局永久配置。

### D3: 算力影响 min_tier，不选具体模型

**决策**：`ComputeLevel` 仅设置传给 `ModelRouter` 的 `RouteTierConstraints.agent_min_tier`（由 override 或默认解析），不修改 `AgentConfig.model` 或前端 `ModelSelector` 的选中模型。

路由窗口逻辑（已有）：
```
floor = agent_min_tier.unwrap_or(Tiny).max(estimated_workload)
cap   = agent_max_tier.unwrap_or(Frontier)
```

算力档位提高 `floor`，迫使路由器在不低于该 tier 的模型中选型； workload 估计仍可抬高 floor。

**理由**：算力 =「至少用多强的模型」，与「选哪个具体模型」正交；用户可固定模型 + 调算力，或让路由器在窗口内择优。
**替代方案**：算力直接绑定模型列表。否决，与现有 tier 窗口机制重复且更僵化。

### D4: 默认档位为 High（Medium tier）

**决策**：全局默认 `ComputeLevel::High` → `ComplexityTier::Medium`。

**理由**：与 `ComplexityTier::parse_loose` 未知值回退 Medium 一致；大多数对话任务需要适度推理，Tiny 过弱、Frontier 过贵。

### D5: WS API 设计

**决策**：复用现有 WS 通道，新增：

| 方法 | 请求 | 响应 / 副作用 |
|------|------|----------------|
| `compute_level.get` | `{ session_id }` | `{ level, level_label, is_override, levels: [...] }` |
| `compute_level.set` | `{ session_id, level }` | 设置 override；`level: null` 清除；广播 `compute_level.changed` |

`levels` 数组每项：`{ id, label, description, tier }`。

**理由**：与 `permissions.get/set` 对称，前端 store 可复用相同订阅模式。

### D6: ComputeLevelSelector UI

**决策**：InputBar 工具栏中，位于模型选择器右侧。

外观：`⚡ Extra High ▾`（图标 + 当前标签 +  chevron）

下拉 5 项，每项：名称 + 一句速度/质量描述；当前项 ✓；**Max** 档位需确认对话框（类似 Full-auto 警告）。

Max 生效时：选择器与 InputBar 底部显示琥珀色提示「最高算力，成本与延迟显著增加」。

**理由**：对齐原型 `ib-chip`；Max 有成本风险需显式确认。

## Risks / Trade-offs

**[R1] 运行中切换算力可能导致同一 session 内模型行为不一致** → 与权限预设相同：当前 turn 继续使用切换前的 `min_tier` snapshot，下一 turn 生效；前端提示「算力将在下一轮对话生效」。

**[R2] min_tier 抬高后无满足 tier 的候选模型** → `ModelRouter` 已有 `filter_candidates_by_tier` 回退逻辑；若过滤后为空，记录 warn 并放宽或回退到最近可用 tier（保持现有 router 行为，不在本变更引入新逻辑）。

**[R3] 与手动 ModelSelector 的交互** → 用户选的模型若 tier 低于 `min_tier`，路由器仍以 window 为准选路由模型；前端可在设置页说明「算力档位影响自动路由，手动模型选择在 Fixed 策略下优先」。初始版本不在 UI 强制禁用低 tier 模型。

**[R4] 默认 High 与 config.toml 中 `min_tier` 冲突** → session 无 override 时：`effective = max(config.min_tier, global_default.to_tier())` 或 override 完全取代 config（实现时二选一，推荐 **override 取代 config min_tier，无 override 时用 config 再 fallback 默认 High**）。在 `ComputeLevelResolver` 文档中明确优先级。

## Open Questions

- 是否在 Settings 增加「默认计算等级」持久化？（本变更 Non-Goal，可后续扩展）
- `agent_max_tier` 是否随算力档位联动？（当前决策：否，仅 min）
