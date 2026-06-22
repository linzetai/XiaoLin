## ADDED Requirements

### Requirement: Plan 色系统 CSS Token
项目 SHALL 定义统一的 Plan 模式颜色 CSS custom properties，与全局 `--tint`（Agent 蓝色）和 `--orange`（Goal 橙色）形成三色区分。所有 Plan 相关 UI 元素 SHALL 使用这些 token 而非硬编码颜色或 `var(--tint)`。

#### Scenario: Light 主题 Plan 色
- **THEN** 以下 CSS token SHALL 在 `:root` 下定义：
  - `--plan-tint: #0D9488`（teal-600，主色）
  - `--plan-tint-bg: rgba(13, 148, 136, 0.06)`（背景色）
  - `--plan-tint-subtle: rgba(13, 148, 136, 0.03)`（微妙背景）
  - `--plan-tint-border: rgba(13, 148, 136, 0.3)`（边框色）
  - `--plan-tint-text: #115E59`（深色文字）

#### Scenario: Dark 主题 Plan 色
- **THEN** 以下 CSS token SHALL 在 `[data-theme="dark"]` 下定义：
  - `--plan-tint: #2DD4BF`（teal-400）
  - `--plan-tint-bg: rgba(45, 212, 191, 0.12)`
  - `--plan-tint-subtle: rgba(45, 212, 191, 0.06)`
  - `--plan-tint-border: rgba(45, 212, 191, 0.3)`
  - `--plan-tint-text: #99F6E4`

#### Scenario: ModeSelector Plan 选项色更新
- **WHEN** ModeSelector 渲染 Plan 选项
- **THEN** 选项颜色 SHALL 使用 `var(--plan-tint)` 而非当前的 `oklch(56% 0.18 310)` 紫色

### Requirement: 消息流模式标识
在 Plan 模式下，消息流中的 assistant 消息 SHALL 显示模式标识，与 Agent 模式下的消息视觉区分。

#### Scenario: Plan 模式下 assistant 消息带左边框
- **WHEN** 当前处于 Plan 模式且 assistant 发送消息
- **THEN** 消息 SHALL 在左侧有 2px 的 `var(--plan-tint-border)` 边框

#### Scenario: Plan 模式下 assistant 消息带徽章
- **WHEN** 当前处于 Plan 模式且 assistant 发送消息
- **THEN** 消息顶部 SHALL 显示小型规划徽章（🧭 图标 + "Plan" 文字，`var(--plan-tint)` 色，8px 字号）

### Requirement: 统一模式入口合成消息
当用户通过 UI ModeSelector 切换到 Plan 模式时，系统 SHALL 向对话历史注入合成用户消息，通知模型模式已切换。

#### Scenario: UI 切换注入合成消息
- **WHEN** 用户通过 ModeSelector 从 Agent 切换到 Plan
- **THEN** SHALL 注入合成用户消息 "[系统: 用户已切换到规划模式]" 到对话历史

#### Scenario: Agent 工具切换不重复注入
- **WHEN** agent 通过 `enter_plan_mode` 工具切换到 Plan 模式
- **THEN** SHALL 不注入额外的合成消息（工具结果已提供上下文）

### Requirement: Composer Plan 模式横幅增强
Plan 模式下 Composer 区域的横幅 SHALL 使用 Plan 色系统，更醒目地标识当前模式。

#### Scenario: Plan 模式横幅样式
- **WHEN** 当前处于 Plan 模式
- **THEN** Composer 上方横幅 SHALL 使用以下样式：
  - 背景: `var(--plan-tint-bg)` 而非 `color-mix(var(--tint) 6%)`
  - 下边框: `var(--plan-tint-border)` 而非 `color-mix(var(--tint) 15%)`
  - 文字和图标: `var(--plan-tint)` 而非 `var(--tint)`
- **THEN** 横幅 SHALL 包含：指南针图标、"Plan Mode — Read-only" 文字、plan 文件路径、右侧文件图标（点击打开 PlanPanel）

### Requirement: Composer 边框模式色
Plan 模式下 Composer 容器的边框 SHALL 使用 `var(--plan-tint-border)`，提供持久的模式视觉提示。

#### Scenario: Composer 边框颜色切换
- **WHEN** 切换到 Plan 模式
- **THEN** Composer 容器边框 SHALL 从默认色过渡到 `var(--plan-tint-border)`（300ms transition）
- **WHEN** 切换回 Agent 模式
- **THEN** Composer 容器边框 SHALL 过渡回默认色（300ms transition）

### Requirement: 模式切换过渡动画
模式切换时 SHALL 有平滑的视觉过渡，避免突兀的状态跳变。

#### Scenario: Agent → Plan 过渡动画
- **WHEN** 从 Agent 模式切换到 Plan 模式
- **THEN** 以下动画 SHALL 依次执行：
  1. Plan 横幅从下方滑入（slideDown, 200ms ease-out）
  2. Composer 边框颜色过渡到 `var(--plan-tint-border)`（300ms）
  3. ModeSelector 图标和文字变色到 `var(--plan-tint)`（200ms）

#### Scenario: Plan → Agent 过渡动画
- **WHEN** 从 Plan 模式切换到 Agent 模式
- **THEN** 以下动画 SHALL 依次执行：
  1. Plan 横幅 fadeOut + slideUp（150ms）
  2. Composer 边框颜色过渡回默认色（300ms）
  3. ModeSelector 图标和文字变色回 `var(--tint)`（200ms）

#### Scenario: PlanPanel 自动开关动画
- **WHEN** 切换到 Plan 模式且 plan 文件存在
- **THEN** PlanPanel 可选自动打开，从右侧滑入（slideFromRight, 250ms ease-out）
- **WHEN** 切换回 Agent 模式
- **THEN** PlanPanel 如果已打开，可选自动关闭，向右滑出（slideToRight, 200ms ease-in）

### Requirement: 工具 Badge 增强
`enter_plan_mode` 和 `exit_plan_mode` 工具的结果显示 SHALL 使用专用样式而非通用工具结果样式。

#### Scenario: enter_plan_mode 工具结果
- **WHEN** `enter_plan_mode` 工具执行成功
- **THEN** 工具结果 SHALL 渲染为简洁的状态行：`● 已进入 Plan 模式` + 副文案 "只读探索 · plan 文件路径"
- **THEN** 颜色 SHALL 使用 `var(--plan-tint)`

#### Scenario: exit_plan_mode 工具结果（有 approval）
- **WHEN** `exit_plan_mode` 工具执行成功且 `metadata.approval_pending = true`
- **THEN** 工具结果 SHALL 渲染为 PlanApprovalCard（已有），不显示原始工具文本

#### Scenario: exit_plan_mode 工具结果（无 plan 文件）
- **WHEN** `exit_plan_mode` 工具执行成功但无 plan 文件
- **THEN** 工具结果 SHALL 渲染为简洁状态行：`● 已退出 Plan 模式`

### Requirement: Placeholder 文字模式区分
Plan 模式下输入框 placeholder SHALL 明确传达只读属性。

#### Scenario: Plan 模式 placeholder
- **WHEN** 当前处于 Plan 模式
- **THEN** 输入框 placeholder SHALL 为 "探索代码、讨论方案...（只读模式）" 或等效文字
- **THEN** placeholder 颜色 SHALL 使用 `var(--plan-tint)` 低透明度

## Implementation Reference

### 当前实现中需修改的位置

| 文件 | 修改内容 |
|------|---------|
| `index.css` | 添加 `--plan-tint-*` CSS token（light + dark） |
| `ComposerCore.tsx:330` | ModeSelector Plan 选项色从 `oklch(56% 0.18 310)` 改为 `var(--plan-tint)` |
| `ComposerCore.tsx:635-648` | Plan Banner 样式从 `var(--tint)` 改为 `var(--plan-tint-*)` |
| `PlanPanel.tsx:66-70` | PlanPanel 头部样式从 `var(--tint)` 改为 `var(--plan-tint-*)` |
| `PlanApprovalCard.tsx:101-104` | 审批卡片样式从 `var(--tint)` 改为 `var(--plan-tint-*)` |
| Composer 容器 | 添加 Plan 模式边框色和 transition |
| 消息渲染组件 | 根据 executionMode 添加 Plan 消息左边框和徽章 |

### 色系统选择依据

选择 Teal（`#0D9488` / `#2DD4BF`）作为 Plan 模式色：
- **竞品验证**: Claude Code 使用 `rgb(0,102,102)` (teal) 经大规模使用验证
- **与 Agent 蓝色区分**: 蓝色 → 青绿色，色相差足够辨识但不突兀
- **三色体系**: Agent (蓝 `--tint`) / Plan (青 `--plan-tint`) / Goal (橙 `--orange`)
- **明暗主题兼容**: teal 在 light/dark 下均有良好可见度

### 三色体系一览

| 模式 | 主色 (Light) | 主色 (Dark) | 图标 |
|------|-------------|-------------|------|
| Agent | `#2563EB` | `#60A5FA` | `<Code />` |
| Plan | `#0D9488` | `#2DD4BF` | `<Compass />` |
| Goal | `#FF9500` | `#FF9F0A` | `<Crosshair />` |

### 动画 CSS Token 参考

```css
@keyframes plan-banner-enter {
  from { opacity: 0; transform: translateY(8px); }
  to { opacity: 1; transform: translateY(0); }
}

@keyframes plan-banner-exit {
  from { opacity: 1; transform: translateY(0); }
  to { opacity: 0; transform: translateY(-4px); }
}

@keyframes plan-panel-enter {
  from { opacity: 0; transform: translateX(20px); }
  to { opacity: 1; transform: translateX(0); }
}

@keyframes plan-panel-exit {
  from { opacity: 1; transform: translateX(0); }
  to { opacity: 0; transform: translateX(20px); }
}

.plan-mode-composer {
  border-color: var(--plan-tint-border);
  transition: border-color 300ms ease;
}
```
