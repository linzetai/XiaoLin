## Tasks

- [ ] 1. 重构 `CoordinatorPanel.tsx` 为 `SubAgentsTabContent`
  - 合并 SubAgentMonitor 的 RunItem 组件
  - 顶部保留 coordinator header + steering input（条件渲染）
  - 主体为所有 runs 的列表
  - 支持展开/收起、取消、结果查看

- [ ] 2. 修改 `AppShell.tsx` tab 注册逻辑
  - 注册条件从 `hasCoordinator` 改为 `hasAnySubAgent`
  - Tab id 改为 "subagents"，label 改为 "SubAgents"
  - 首次出现时自动 setActiveTab + setPanelOpen(true)
  - 使用 prevRef 避免重复打开

- [ ] 3. 移除 `MessageStream.tsx` 中的 SubAgentMonitor
  - 删除 `<SubAgentMonitor />` 引用和 import

- [ ] 4. 清理 `SubAgentMonitor.tsx`
  - 确认无其他引用后删除文件

- [ ] 5. MCP 验证
  - 触发 sub-agent 运行
  - 确认 WorkspacePanel 自动打开
  - 确认 tab 内容完整显示
