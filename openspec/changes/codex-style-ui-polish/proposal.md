## Why

当前 Agent 状态可视化组件（ReasoningBlock、PhaseIndicator、StepIndicator、iteration divider）虽功能完整，但视觉设计过于"重"——带边框的卡片式推理块、多环 SVG 动画、全宽分隔线等让聊天界面显得拥挤。参考 OpenAI Codex CLI 的极简界面风格（左侧竖线 + 脉冲圆点思考指示、紧凑内联工具状态、无边框布局），对现有组件做 Codex 风格的视觉轻量化改造，提升信息密度同时降低视觉噪音。

## What Changes

- **ReasoningBlock 重设计**：去除卡片边框和背景，改用左侧 2px 竖线 + 顶部脉冲圆点；流式阶段固定高度滚动面板自动跟随底部；完成后平滑折叠带高度动画过渡
- **PhaseIndicator 简化**：移除多环 OrbitSpinner SVG，改为单个脉冲圆点 + 文字 + 经过时间计时器，更轻量
- **轻量工具嵌套**：只读类工具（file_read、list_directory 等）在 reasoning/thinking 区域内以紧凑单行内联显示，不占用独立卡片空间
- **迭代分隔符轻量化**：从全宽横线 + 文字改为居中三圆点分隔符，减少视觉干扰
- **StepIndicator 微调**：减少边框/间距，动画更含蓄，整体更紧凑

## Capabilities

### New Capabilities
- `codex-reasoning-block`: ReasoningBlock 组件重设计——左竖线风格、固定高度流式面板、自动滚动、折叠动画
- `codex-phase-indicator`: PhaseIndicator 简化——脉冲圆点 + 文字 + 计时器，移除 SVG
- `codex-tool-nesting`: 轻量工具内联显示——只读工具在思考区域内紧凑渲染
- `codex-iteration-divider`: 迭代分隔符样式——三圆点居中分隔
- `codex-step-polish`: StepIndicator 微调——边框/间距/动画精简

### Modified Capabilities

## Impact

- 前端组件：`ReasoningBlock.tsx`、`ThinkingIndicator.tsx`（PhaseIndicator）、`StepIndicator.tsx`、`StepGroup.tsx`、`MessageRenderer.tsx`
- CSS/tokens：可能需要新增 CSS 变量或 keyframe 动画
- 无后端变更、无 API 变更、无破坏性变更
