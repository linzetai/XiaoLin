# Visual Redesign — 任务清单

## Phase 1: CSS Token 系统

- [x] 1.1 在 `index.css` `:root` 块中添加 `--tc-*` 工具类别色彩 tokens（light mode 7 类 × 2 = 14 变量）
- [x] 1.2 在 `index.css` `[data-theme="dark"]` 块中添加对应的 dark mode `--tc-*` tokens
- [x] 1.3 更新 `--step-height` 从 `28px` → `36px`，`--step-gap` 从 `2px` → `3px`
- [x] 1.4 新增 `--step-border`, `--step-radius`, `--step-icon-size`, `--step-icon-radius` tokens

## Phase 2: 工具调用卡片 (StepIndicator)

- [x] 2.1 在 `StepIndicator.tsx` 中新增 `ToolCategory` 类型和 `getToolCategory(name)` 映射函数
- [x] 2.2 将 StepIndicator 主行从无边框 `<button>` 改为带边框的卡片结构（`1px solid var(--step-border)`, `border-radius: var(--step-radius)`）
- [x] 2.3 将左侧 14px 状态图标(Check/X/Spinner) + 工具图标 替换为单个 24px 分类色图标徽章（`--tc-{category}-bg/fg`）
- [x] 2.4 将右侧状态改为 5px 状态点（green dot / red dot / spinner）
- [x] 2.5 调整 keyInfo 显示为 mono 字体灰色截断文本
- [x] 2.6 展开区域使用 `grid-template-rows` 动画过渡替代当前的直接显隐
- [x] 2.7 展开区域内的代码/输出/diff/错误区域对齐原型配色

## Phase 3: 工具调用分组 (StepGroup)

- [x] 3.1 StepGroup 摘要行适配新的视觉高度和间距
- [x] 3.2 展开后的嵌套 StepIndicator 使用紧凑变体（去掉外层边框避免双重嵌套）
- [x] 3.3 确保 SubAgentCard 内复用的 StepIndicator 正确应用新样式

## Phase 4: 会话列表项 (SessionList)

- [x] 4.1 为每个会话项添加 30px 图标盒（默认 bg-secondary + separator border）
- [x] 4.2 active 状态图标盒改为 accent 填充 + 白色图标
- [x] 4.3 添加第二行预览文本（最后一条消息摘要 / 占位文本）
- [x] 4.4 调整项间距和 hover 过渡

## Phase 5: 输入栏 (StreamFooter)

- [x] 5.1 移除 frosted glass 效果（`backdrop-filter`, 半透明 `background`）
- [x] 5.2 外壳改为 `1.5px solid var(--separator)` 边框 + `border-radius: 18px` + `var(--bg-surface)` 背景
- [x] 5.3 添加 focus-within 发光效果（`box-shadow: 0 0 0 4px tint-8%`）
- [x] 5.4 模式切换从 toggle pill 改为 segmented control（Agent | Plan）

## Phase 6: 消息头像 (MessageRenderer / UserInput)

- [x] 6.1 创建 `MessageAvatar` 组件（用户: 渐变圆 + "U"；AI: border 圆 + ClawIcon）
- [x] 6.2 MessageRenderer 中为 AI 消息添加头像，调整布局为 `flex gap-14` 横排
- [x] 6.3 UserInput 中为用户消息添加头像 + 气泡样式（`border-radius: 14px 14px 14px 4px`）
- [x] 6.4 消息头显示: 名称 + 时间 + (AI) 耗时 pill
- [x] 6.5 hover 显示 action buttons（复制、赞、踩、重试），默认 `opacity: 0`

## Phase 7: NavRail 增强

- [x] 7.1 `--nav-rail-w` 从 `48px` → `54px`，按钮从 28/32px → 36px
- [x] 7.2 active 按钮添加左侧 3px accent 指示条（`::after` 伪元素）
- [x] 7.3 添加 `data-tooltip` 属性和 CSS hover tooltip（暗色弹出，右侧 12px 偏移）
- [x] 7.4 技能按钮添加 notification dot（7px 红色圆，右上角定位）

## Phase 8: 收尾

- [x] 8.1 在 `index.css` 中添加 `@keyframes mIn` 消息入场动画
- [x] 8.2 全局检查：确保所有新增的 CSS 变量在 light/dark 都有定义
- [x] 8.3 `pnpm build` 编译验证前端无错误
- [x] 8.4 `cargo check` 确认 Rust 侧无影响
- [x] 8.5 Tauri 桌面端视觉回归：NavRail、SessionList、消息流、输入栏、设置面板

## 实施顺序建议

视觉影响力从高到低：
1. **Phase 2 (StepIndicator)** — 对话流中出现最频繁，改造效果最明显
2. **Phase 5 (StreamFooter)** — 输入栏是用户持续注视的区域
3. **Phase 6 (消息头像)** — 增强消息身份识别
4. **Phase 4 (SessionList)** — 侧边栏信息密度提升
5. **Phase 7 (NavRail)** — 交互增强
6. **Phase 1 & 3** — 支撑性工作
7. **Phase 8** — 收尾验证
