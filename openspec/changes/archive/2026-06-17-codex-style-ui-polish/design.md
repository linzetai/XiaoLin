## Context

XiaoLin 聊天界面当前已实现 Agent 状态可视化：`ReasoningBlock` 显示推理过程、`PhaseIndicator` 显示阶段状态、`StepIndicator` 显示工具调用、iteration divider 显示迭代边界。

当前问题：视觉风格偏"重"——卡片式边框 + 背景色的推理块、三环 SVG 旋转动画、全宽线分隔符占用过多视觉注意力。对标 Codex CLI 的极简设计：左侧竖线标记、脉冲圆点、紧凑内联布局。

核心约束：
- 仅修改前端 React 组件和 CSS，无后端变更
- 保持现有数据流不变（`StreamSegment` 类型、事件分发逻辑不改）
- 支持暗色/亮色主题通过 CSS 变量适配

## Goals / Non-Goals

**Goals:**
- 降低推理块视觉权重：左竖线 + 脉冲圆点替代卡片式边框
- 流式推理时提供固定高度面板 + 自动底部跟随，避免页面频繁跳动
- PhaseIndicator 视觉极简化：单脉冲圆点 + 文字 + 计时器
- 只读工具（file_read 等）可选以紧凑内联方式渲染在思考区域内
- 迭代分隔符轻量化为三圆点
- StepIndicator 边框/间距/动画微调

**Non-Goals:**
- 不重构事件系统或 segment 分组逻辑
- 不改变工具调用的数据模型
- 不做 ReasoningBlock 的内容格式化（如代码高亮）
- 不做国际化文案调整（保持现有 i18n key）

## Decisions

### Decision 1: ReasoningBlock 采用左竖线 + 固定高度面板

**选择**: 去除 `border` + `backgroundColor` 卡片样式，改为 `border-left: 2px solid var(--tint)`，顶部加 6px 脉冲圆点。流式阶段 maxHeight 200px + overflow-y: auto + 自动 scrollToBottom。完成后折叠动画用 `max-height` transition (300ms ease-out)。

**替代方案**: 保持卡片式但减小 padding — 仍然偏重，不够极简。

**理由**: 左竖线是 Codex 标志性设计元素，视觉权重极低，同时保持与正文的清晰分界。固定高度面板避免长推理内容挤占聊天区域。

### Decision 2: PhaseIndicator 使用 CSS-only 动画

**选择**: 移除 `OrbitSpinner` SVG 组件，改为 `<span>` 元素 + CSS `@keyframes pulse` (scale 0.8→1.2 + opacity 0.4→1)。保留文字标签 + 新增 `ElapsedTimer` 显示从进入阶段开始的耗时。

**替代方案**: 保留 SVG 但简化为单环 — 仍比纯 CSS 圆点重。

**理由**: 单个 DOM 元素 + CSS 动画，无 SVG 开销，且与 Codex 风格一致。计时器提供时间感知，替代动画复杂度来传达"正在工作"的信息。

### Decision 3: 轻量工具内联——条件性紧凑渲染

**选择**: 在 `groupConsecutiveSegments` 分组后的渲染阶段，对连续只读工具（category === "read" 或 "search"）在特定上下文（紧跟在 reasoning 段后且无 text 段间隔时）渲染为单行内联样式：图标 + 文件名，无边框无展开。

**替代方案**: 在 StepGroup 内部区分紧凑模式 — 耦合度高。

**理由**: 保持分组逻辑不变，仅在渲染层面按条件选择紧凑或完整渲染，改动范围最小。

### Decision 4: 迭代分隔符三圆点居中

**选择**: 用三个 4px 圆点水平排列（gap 6px），替代全宽 `h-px` 横线 + "Step N" 文字。不再显示 iteration number（信息冗余，工具组已能表达迭代边界）。

**替代方案**: 保持横线但改为虚线 — 视觉噪音仍较高。

### Decision 5: StepIndicator 微调

**选择**: 
- 边框从 `1px solid` 改为 `0.5px solid` 或完全去除（仅保留 hover 背景变化）
- step-gap 从当前值缩小 2px
- 运行态动画从 `background tint 4%` 闪烁改为仅 status dot 动画

## Risks / Trade-offs

- **[信息密度 vs 可发现性]** → 紧凑内联工具可能让用户不注意到某些操作已执行。保留 hover 时的 tooltip 作为缓解。
- **[动画减少 vs 状态感知]** → 去除 OrbitSpinner 后 PhaseIndicator 视觉弱化。脉冲圆点 + 计时器提供替代信号。
- **[固定高度推理面板 vs 内容被截断]** → 用户可能想看全部推理内容。保留 click-to-expand 交互恢复全高。
- **[CSS-only 方案]** → 未来若需更复杂动画可能要回退到 SVG。当前需求下 CSS 足够。
