## Why

当前 SubAgent 执行信息分散在两处：底部悬浮抽屉（SubAgentMonitor）和 WorkspacePanel 的 Coordinator tab。底部抽屉覆盖聊天内容，视觉干扰大；Coordinator tab 只显示 coordinator 类型的 sub-agent，普通 sub-agent 无法在 WorkspacePanel 中查看。需要统一为 WorkspacePanel 中的单个 tab，提供完整、不遮挡的 sub-agent 监控体验。

## What Changes

- **合并 SubAgentMonitor 和 CoordinatorTabContent**：统一为 WorkspacePanel 中的 "SubAgents" tab
- **自动注册与打开**：当任何 sub-agent 启动时自动注册 tab 并打开 WorkspacePanel
- **移除底部抽屉**：从 MessageStream 中删除 SubAgentMonitor 组件
- **保留全部功能**：coordinator steering input、worker 列表、取消按钮、状态/耗时/通知/结果展示

## Capabilities

### New Capabilities
- `unified-subagent-tab`: 合并后的 WorkspacePanel tab，显示所有 sub-agent（包括 coordinator）的完整执行信息

### Removed Capabilities
- SubAgentMonitor 底部抽屉（被 unified-subagent-tab 替代）
- 独立的 Coordinator tab（合并进 unified-subagent-tab）

## Impact

- 前端组件：`CoordinatorPanel.tsx`（重构）、`AppShell.tsx`（注册逻辑）、`MessageStream.tsx`（移除引用）
- 可删除文件：`SubAgentMonitor.tsx`
- 无后端变更、无 API 变更
