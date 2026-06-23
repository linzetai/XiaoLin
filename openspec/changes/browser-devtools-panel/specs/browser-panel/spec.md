## MODIFIED Requirements

### Requirement: AgentOperationLog 整合到 DevTools 面板
AgentOperationLog 组件 SHALL 作为 DevTools 底部面板的 Agent Tab 内容，而非独立渲染在 BrowserTabContent 中。

#### Scenario: Agent Tab 展示操作记录
- **WHEN** 用户切换到 DevTools 的 Agent Tab 且有操作记录
- **THEN** 面板 SHALL 展示与当前 AgentOperationLog 相同的操作列表内容（不含独立折叠头和 160px 高度限制）

#### Scenario: Agent Tab 无操作时显示空态
- **WHEN** 用户切换到 DevTools 的 Agent Tab 且无操作记录
- **THEN** 面板 SHALL 显示「暂无 Agent 操作」占位文案
- **THEN** DevToolsPanel Tab 栏 SHALL 保持可见（不因 Agent 内容为空而隐藏整个面板）

#### Scenario: BrowserTabContent 中不再独立渲染 AgentOperationLog
- **WHEN** BrowserTabContent 渲染
- **THEN** 底部区域 SHALL 渲染 DevToolsPanel（包含 Agent/Console/Network Tab）
- **THEN** AgentOperationLog SHALL 不作为独立组件渲染
