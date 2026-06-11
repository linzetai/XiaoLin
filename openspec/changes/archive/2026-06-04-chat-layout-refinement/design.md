## Context

当前 `ContentBlock` 是一个 flex-row 容器，内含聊天区（flex: 1, minWidth: 0）和 `WorkspacePanel`（固定 360px）。面板打开时从聊天区内部夺取空间，导致聊天区压缩。消息行使用 `px-6`（24px）固定 padding 配合 `maxWidth: var(--content-max-w)` 左对齐，在宽屏下两侧留白不均匀。

涉及文件：
- `ContentBlock.tsx` — flex 容器
- `WorkspacePanel.tsx` / `workspace-tabs.ts` — 面板 store 与组件
- `MessageRenderer.tsx` — 消息行布局
- `MessageStream.tsx` — 滚动容器
- `index.css` — CSS tokens

## Goals / Non-Goals

**Goals:**
- 面板打开时窗口向右扩展，聊天区宽度不变
- 聊天区设置 minWidth 兜底保护
- 消息内容居中对齐，两侧留白均匀且更充裕
- 面板关闭时窗口恢复原宽度
- 窗口最大化时面板行为合理降级

**Non-Goals:**
- 不改变面板本身的内容和交互
- 不改变 Sidebar 宽度或行为
- 不做面板可拖拽调整宽度（后续可单独做）
- 不改变移动端/compact 布局下的行为

## Decisions

### D1: 面板开关时通过 Tauri API 调整窗口宽度

**选择**：在 workspace-tabs store 的 `togglePanel` 逻辑中，调用 `window.setSize()` 增减 `--panel-w`(360px)。

**替代方案**：
- 面板 overlay 浮在内容上 → 会遮挡内容，不符合"向右拓展"的预期
- 面板推动整个布局向左 → 会影响 sidebar 可见性

**理由**：窗口扩展是最直观的"向右拓展"实现，聊天区完全不受影响。

### D2: 最大化/全屏时降级为内部压缩

窗口最大化时无法再向右扩展，此时退回当前行为（面板占用内部空间），但聊天区有 minWidth 保护不会过度压缩。

### D3: 消息居中对齐

**选择**：消息行容器使用 `margin: 0 auto` 配合 `maxWidth: var(--content-max-w)` 实现居中。移除固定的 `px-6`，改为在消息行外层通过 `padding: 0 clamp(24px, 5%, 80px)` 提供弹性两侧留白。

**替代方案**：
- 仅增大 px-6 为 px-12 → 小屏浪费空间，大屏仍不够
- CSS container query 自适应 → 复杂度高，浏览器兼容性不确定

### D4: 降低 content-max-w 数值

- standard: 720px → 660px
- wide: 860px → 760px

配合居中对齐后，两侧留白更加明显和均匀。

### D5: 窗口宽度恢复策略

面板关闭时缩减窗口宽度。记录面板打开前的窗口宽度，关闭时恢复到该值（而非简单减 360px），避免用户手动调整窗口后的宽度错乱。

## Risks / Trade-offs

- **屏幕空间不足** → 面板打开时窗口右边界超出屏幕：检测可用屏幕空间，若不足则降级为内部压缩模式
- **窗口 resize 闪烁** → Tauri setSize 可能导致短暂的布局跳动：使用 CSS transition 或延迟渲染面板内容来平滑过渡
- **多显示器场景** → 窗口跨屏幕时的行为不确定：仅关注主显示器可用宽度
- **非 Tauri 环境（浏览器模式）** → 无法调用 Tauri API 调整窗口：浏览器模式下保持当前行为（内部压缩）
- **content-max-w 降低** → 代码块等长内容可能被截断更多：可保持代码块的 max-width 不变或允许横向滚动
