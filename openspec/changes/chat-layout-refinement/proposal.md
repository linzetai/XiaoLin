## Why

聊天区域在右侧面板打开时被压缩，缺乏最小宽度保护；消息内容水平方向"撑满"容器，两侧留白不足，阅读体验局促。需要优化布局使聊天区更有呼吸感，并让面板开启不影响聊天区宽度。

## What Changes

- 右侧面板（WorkspacePanel）打开时，通过 Tauri 窗口 API 向右扩展窗口宽度，聊天区宽度保持不变
- 聊天区设置最小宽度保护（兜底），防止极端情况下被过度压缩
- 消息内容从左对齐改为居中对齐，两侧留白均匀分布
- 降低 `--content-max-w` 数值，增大消息区域两侧留白空间
- 面板关闭时窗口宽度恢复

## Capabilities

### New Capabilities
- `panel-window-resize`: 面板开关时通过 Tauri API 动态调整窗口宽度，保证聊天区不被压缩
- `centered-message-layout`: 消息内容居中对齐，两侧均匀留白，提升阅读体验

### Modified Capabilities
- `app-shell-layout`: 聊天区增加 minWidth 保护，防止极端压缩
- `workspace-panel`: 面板开关行为与窗口尺寸联动

## Impact

- 前端组件：`ContentBlock.tsx`、`MessageRenderer.tsx`、`MessageStream.tsx`、`WorkspacePanel.tsx`（或 workspace-tabs store）
- CSS tokens：`--content-max-w` 数值调整
- Tauri API 依赖：`@tauri-apps/api/window` 的 `setSize` / `innerSize`
- 需要处理窗口最大化、屏幕边界等边界情况
