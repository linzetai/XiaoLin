## MODIFIED Requirements

### Requirement: WorkspacePanel toggle
WorkspacePanel SHALL 支持通过 AppHeader 中的布局按钮或键盘快捷键打开/关闭。关闭时不渲染 WorkspacePanel，ContentBlock 只包含 ChatPane。

打开/关闭面板时，系统 SHALL 联动窗口尺寸调整（通过 panel-window-resize 能力），使聊天区宽度不受影响。

#### Scenario: Toggle WorkspacePanel with window resize
- **WHEN** 用户点击 AppHeader 的分栏布局按钮或触发快捷键，且窗口未最大化
- **THEN** WorkspacePanel 在打开和关闭之间切换
- **AND** 窗口宽度随面板状态同步增减 360px
- **AND** 切换时有平滑的宽度过渡动画

#### Scenario: Toggle WorkspacePanel while maximized
- **WHEN** 用户在窗口最大化状态下切换面板
- **THEN** WorkspacePanel 在打开和关闭之间切换
- **AND** 窗口保持最大化，面板从 ChatPane 内部空间分配
