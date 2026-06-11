## ADDED Requirements

### Requirement: Shell output reference in chat
ChatPane 中的 Agent 工具调用卡片（shell_exec）SHALL 支持"在终端查看"链接，点击后跳转到 Terminal tab 并滚动到对应命令位置。

#### Scenario: Terminal link on shell tool card
- **WHEN** Agent 执行 shell_exec 且 WorkspacePanel 的 Terminal tab 已有终端面板
- **THEN** 工具调用卡片中显示 "在终端中查看 →" 链接
- **AND** 工具卡片中仍保留摘要输出（截断的前几行）

#### Scenario: Click terminal link
- **WHEN** 用户点击工具调用卡片中的 "在终端中查看 →" 链接
- **THEN** WorkspacePanel 切换到 Terminal tab
- **AND** 滚动到对应命令的位置（Phase 1/2 通过 entry id 定位，Phase 3 通过 Shell Integration 标记定位）

### Requirement: Simplified tool card for streaming commands
当 shell 命令正在流式执行时，ChatPane 中的工具调用卡片 SHALL 显示简化的实时状态而非完整输出。

#### Scenario: Streaming command card
- **WHEN** shell_exec 正在执行中（已收到 tool_executing 但未收到 tool_result）
- **AND** 终端面板已在 WorkspacePanel 中渲染
- **THEN** 工具卡片显示 "⚡ 正在执行: {command}" + 旋转 spinner
- **AND** 不在卡片内重复显示流式输出（输出在 Terminal tab 可见）

#### Scenario: Completed command card
- **WHEN** shell_exec 执行完成（收到 tool_result）
- **THEN** 工具卡片显示命令 + exit code + "在终端中查看 →" 链接
- **AND** 保留截断的输出摘要（最多 4 行）
