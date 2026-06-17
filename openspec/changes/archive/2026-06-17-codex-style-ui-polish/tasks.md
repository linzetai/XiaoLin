## 1. ReasoningBlock 重设计

- [x] 1.1 移除卡片式外边框和背景色，改为 border-left 2px solid var(--tint) 样式
- [x] 1.2 添加顶部 6px 脉冲圆点（isStreaming 时显示，完成后隐藏）
- [x] 1.3 流式阶段固定 max-height 200px + overflow-y auto
- [x] 1.4 实现 auto-scroll-to-bottom（useRef + scrollTop 跟随）和用户手动滚上暂停逻辑
- [x] 1.5 autoCollapse 时使用 max-height CSS transition 300ms ease-out 动画折叠
- [x] 1.6 完成态左竖线颜色降级为 var(--fill-quaternary)

## 2. PhaseIndicator 简化

- [x] 2.1 移除 OrbitSpinner SVG 组件，替换为 8px span 元素 + CSS @keyframes pulse 动画
- [x] 2.2 新增 ElapsedTimer 显示从 mount 开始的秒数，紧跟在 label 文字后
- [x] 2.3 确认暗色/亮色主题下圆点颜色正确（使用 var(--tint)）

## 3. 轻量工具内联

- [x] 3.1 在 MessageRenderer 渲染层添加 compact 判断：只读/搜索类工具紧跟 reasoning 段时用 CompactToolLine 渲染
- [x] 3.2 创建 CompactToolLine 组件：12px 图标 + 截断路径(40ch) + 尾部 4px 状态圆点，无边框
- [x] 3.3 非只读工具仍使用完整 StepIndicator 渲染（保持不变）

## 4. 迭代分隔符轻量化

- [x] 4.1 替换全宽 h-px 横线 + "Step N" 文字为三个 4px 圆点居中布局
- [x] 4.2 设置垂直间距 my-3，确保总高度 ≤ 32px
- [x] 4.3 同步更新 streaming 路径和历史消息路径中的迭代分隔渲染

## 5. StepIndicator 微调

- [x] 5.1 边框从 1px 改为 0.5px，或改为无边框 + hover 背景
- [x] 5.2 减小 --step-gap CSS 变量值 2px
- [x] 5.3 running 态移除 tinted background，仅保留 status dot spin 动画

## 6. 验证

- [x] 6.1 npx tsc --noEmit 无错误
- [x] 6.2 MCP 截图对比改造前后效果，确认各组件视觉正确
