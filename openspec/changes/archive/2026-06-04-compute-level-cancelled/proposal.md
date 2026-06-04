## Why

Codex 原型图在 InputBar 工具栏提供了「Extra High ▾」计算等级选择器，让用户在对话中直观控制 Agent 使用的算力档位（速度 vs 质量权衡）。当前 XiaoLin 已有 `ComplexityTier`（Tiny/Small/Medium/Large/Frontier）和 `AgentConfig.min_tier` / `max_tier` 模型路由约束，但缺少面向用户的 UI 与 per-session 覆盖机制——用户无法在不改配置文件的情况下临时调高或调低算力。

## What Changes

- **新增 ComputeLevel 用户概念**：将 5 档 `ComplexityTier` 映射为友好标签（Low / Medium / High / Extra High / Max）
- **InputBar 计算等级选择器**：在工具栏模型选择器右侧增加「{当前等级} ▾」下拉，实时切换当前 session 的算力档位
- **Per-session 算力覆盖**：计算等级可在 session 粒度覆盖全局默认，不影响其他 session（内存存储，不持久化）
- **WS API 同步**：`compute_level.get` / `compute_level.set` 获取与设置 session 算力，变更时广播 `compute_level.changed`
- **模型路由约束**：算力档位影响传给 `ModelRouter` 的 `agent_min_tier` 下限，不直接指定具体模型

## Capabilities

### New Capabilities
- `compute-level-selector`: InputBar 中的计算等级选择器组件 + Zustand store
- `compute-level-api`: 计算等级相关 WS API（get/set/changed 事件）

### Modified Capabilities
- `chat-input-bar`: InputBar 工具栏在模型选择器右侧增加计算等级选择器

## Impact

- **后端**：
  - `xiaolin-core`：新增 `ComputeLevel` 枚举与用户标签 ↔ `ComplexityTier` 映射
  - `xiaolin-gateway/src/ws/`：新增 `compute_level.rs` handler
  - Session 状态：增加 `compute_level_override: Option<ComputeLevel>`（内存）
  - `xiaolin-model-router`：路由时 `agent_min_tier` 来自 session 有效算力档位解析结果
- **前端**：
  - 新增 `ComputeLevelSelector` 组件
  - `useComputeLevelStore` (Zustand) 管理 per-session 算力状态
  - 修改 InputBar / `StreamFooter` 工具栏集成选择器
- **依赖**：依赖 `layout-overhaul` 的 InputBar 新布局；可与 `permission-presets` 并行，工具栏顺序为：权限 → 刷新 → 模型 → **计算等级**
