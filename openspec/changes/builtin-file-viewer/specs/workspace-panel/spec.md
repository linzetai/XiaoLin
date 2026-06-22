## MODIFIED Requirements

### Requirement: Auto-show behavior

WorkspacePanel SHALL 默认在新会话（无消息）时隐藏。当 agent 产生文件变更时，面板自动打开并切换到 Files 标签（而非原 spec 中的 Review 标签）。

#### Scenario: Auto-open on file changes
- **WHEN** agent 在会话中产生文件变更（通过 `file_artifact` WS event 检测）
- **THEN** WorkspacePanel 自动打开
- **AND** 激活 Files 标签
- **AND** 被操作的文件在查看器中自动打开

#### Scenario: Hidden on new chat
- **WHEN** 活跃会话没有任何消息
- **THEN** WorkspacePanel 不显示

#### Scenario: Explicit toggle overrides auto-hide
- **WHEN** 用户通过 AppHeader 或快捷键显式打开 WorkspacePanel
- **THEN** 即使会话无消息，面板也可显示
