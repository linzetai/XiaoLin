## Summary

将 SubAgentMonitor（底部抽屉）和 CoordinatorTabContent 合并为统一的 WorkspacePanel tab "SubAgents"。

## Requirements

1. 显示所有类型的 sub-agent runs（general、explore、shell、browser、coordinator）
2. Coordinator 类型显示专属 header 和 steering input
3. 每个 RunItem 可展开查看 notifications、result、tool calls
4. 支持取消正在运行的 sub-agent
5. 按状态排序：running > pending > completed > failed > cancelled
6. 耗时实时跳动（active runs 每秒更新）

## Acceptance Criteria

- [ ] 当有 sub-agent 运行时，WorkspacePanel 自动打开并显示 SubAgents tab
- [ ] 所有 sub-agent（包括 coordinator 和 worker）在同一个列表中展示
- [ ] Coordinator 的 steering input 可用
- [ ] 取消按钮可用
- [ ] MessageStream 中不再有底部悬浮抽屉
- [ ] sub-agent 完成后 tab 保留，可查看结果
