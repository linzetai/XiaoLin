## MODIFIED Requirements

### Requirement: Four-region layout architecture
应用 SHALL 采用四区域布局架构：AppHeader（顶部）+ AppSidebar（左侧）+ ChatPane（中间）+ WorkspacePanel（右侧），其中 ChatPane 和 WorkspacePanel 包裹在统一的 ContentBlock 容器内。

ChatPane 容器 SHALL 设置 `minWidth: 480px`，防止在面板打开时被过度压缩。

#### Scenario: Default layout rendering
- **WHEN** 应用启动并完成加载，且活跃会话有消息
- **THEN** 渲染 AppHeader（44px 高）+ AppSidebar（210px 宽）+ ContentBlock（flex-1，内含 ChatPane + WorkspacePanel）
- **AND** ContentBlock 使用 `--bg-card` 背景色，AppSidebar 和 AppHeader 使用 `--bg-shell` 背景色

#### Scenario: Layout without WorkspacePanel
- **WHEN** WorkspacePanel 处于关闭状态
- **THEN** ContentBlock 仅包含 ChatPane，占满剩余宽度
- **AND** ContentBlock 的 border-radius 改为四角圆角 `var(--card-r)`

#### Scenario: ChatPane minimum width protection
- **WHEN** WorkspacePanel 打开且窗口宽度不足以同时容纳 ChatPane(480px) + Panel(360px) + Sidebar(210px)
- **THEN** ChatPane 保持 480px 最小宽度不被压缩
