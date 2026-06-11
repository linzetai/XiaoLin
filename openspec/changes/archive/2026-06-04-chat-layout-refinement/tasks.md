## 1. CSS Tokens 调整

- [x] 1.1 将 `--content-max-w` 从 720px 改为 660px（standard tier）
- [x] 1.2 将 `--content-max-w` 从 860px 改为 760px（wide tier）
- [x] 1.3 新增 `--chat-min-w: 480px` CSS 变量

## 2. 消息居中布局

- [x] 2.1 修改 `MessageRenderer.tsx`：消息行（`.m-ai`, user input 等）从 `px-6` 改为弹性 padding `padding: 0 clamp(24px, 5%, 80px)`
- [x] 2.2 修改 `MessageRenderer.tsx`：AI body 和 content 容器添加 `margin: 0 auto` 实现居中
- [x] 2.3 修改 `MessageStream.tsx`：空状态区域（`StreamEmptyState`）padding 改为弹性居中
- [x] 2.4 修改 `MessageStream.tsx`：搜索栏等辅助 UI 同步居中对齐
- [x] 2.5 验证 compact tier 下消息布局不受影响

## 3. ChatPane 最小宽度

- [x] 3.1 修改 `ContentBlock.tsx`：ChatPane 容器 div 添加 `minWidth: var(--chat-min-w)`
- [x] 3.2 验证面板打开时 ChatPane 不会被压缩到 480px 以下

## 4. 面板开关联动窗口 Resize

- [x] 4.1 在 `workspace-tabs.ts` store 中新增 `prePanelWidth` 状态字段，用于记录面板打开前的窗口宽度
- [x] 4.2 实现 `resizeWindowForPanel(open: boolean)` 异步函数：打开时记录当前宽度并扩展 360px，关闭时恢复到记录的宽度
- [x] 4.3 在 `resizeWindowForPanel` 中添加屏幕边界检测：通过 `availableMonitors()` / `currentMonitor()` 获取可用宽度，若不足则跳过 resize
- [x] 4.4 在 `resizeWindowForPanel` 中添加最大化检测：若窗口已最大化则跳过 resize
- [x] 4.5 在 `resizeWindowForPanel` 中添加 Tauri 环境检测：非 Tauri 环境直接返回
- [x] 4.6 修改 `togglePanel` 方法：调用 `resizeWindowForPanel` 联动窗口尺寸
- [x] 4.7 验证面板打开/关闭时窗口宽度正确变化

## 5. 边界情况处理

- [x] 5.1 验证窗口最大化时面板开关不触发 resize，使用内部压缩模式
- [x] 5.2 验证用户在面板打开时手动调整窗口后关闭面板，恢复到 prePanelWidth
- [x] 5.3 验证浏览器模式（非 Tauri）下面板正常工作（内部压缩模式）
- [x] 5.4 验证窗口位于屏幕右边缘时面板打开不超出屏幕

## 6. 视觉微调与一致性

- [x] 6.1 确认 Composer（输入栏）也居中对齐，与消息一致
- [x] 6.2 确认 StickyContextBar 宽度与消息一致
- [x] 6.3 确认代码块在新 max-width 下横向滚动正常
- [x] 6.4 dark mode 下验证布局一致性
