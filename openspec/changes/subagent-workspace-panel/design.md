## Architecture

统一 SubAgents tab 放置在 WorkspacePanel 中，替代原有的底部抽屉和独立 Coordinator tab。

### 组件层级

```
AppShell
  ├── ContentBlock
  │     ├── main (chat area) ← 不再有 SubAgentMonitor 覆盖
  │     └── WorkspacePanel
  │           └── Tab: "SubAgents" (SubAgentsTabContent)
  │                 ├── Coordinator Header (if exists) + Steering Input
  │                 └── All RunItems (sorted: active first)
```

### 数据流

- `useActiveSubAgentRuns()` → Zustand store → 所有 sub-agent run 数据
- `AppShell` useEffect 监听 runs 变化 → 自动注册/注销 tab
- Tab 内部直接消费 store 数据渲染 RunItem 列表

### 自动打开策略

1. 首个 sub-agent 出现 → registerTab("subagents") + setActiveTab + setPanelOpen(true)
2. 所有 sub-agent 完成/消失 → 保持 tab 存在（用户可手动关闭）
3. 下次有新 sub-agent → 自动切换到 subagents tab

### UI 结构

- **顶部**：Coordinator 信息栏（如果有 coordinator run）— 任务描述、worker 统计、steering input
- **主体**：RunItem 列表，按状态排序（running > pending > completed > failed）
- **RunItem**：可展开，展示 notifications、result、tool calls
- **底部**：空状态或汇总信息

## Key Decisions

1. **合并而非并存**：底部抽屉完全移除，所有信息统一在 WorkspacePanel
2. **自动打开但不自动关闭**：sub-agent 结束后 tab 保留，方便查看结果
3. **复用 RunItem 组件逻辑**：从 SubAgentMonitor 中提取，功能不变
