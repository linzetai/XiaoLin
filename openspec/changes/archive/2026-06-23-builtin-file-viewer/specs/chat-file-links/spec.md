## ADDED Requirements

### Requirement: Chat 消息中的文件路径可点击

Chat 消息中出现的文件路径 SHALL 渲染为可点击链接，点击后在 Files tab 中打开该文件。

#### Scenario: Markdown inline code 文件路径
- **WHEN** Assistant 消息的 Markdown 中包含 inline code 且匹配文件路径模式（`FILE_PATH_RE` 或 `CODE_EXT`）
- **THEN** 渲染为可点击样式（保留现有 `.md-file-path` 样式 + 添加 hover underline + cursor: pointer）
- **AND** 点击时 dispatch `xiaolin:open-file` 事件，携带 `{ path: <文件路径>, source: "markdown-inline" }`

#### Scenario: FileChangesCard 文件行点击
- **WHEN** 用户在 FileChangesCard 中点击某个文件行
- **THEN** dispatch `xiaolin:open-file` 事件（替代原有的 `xiaolin:open-review`）
- **AND** 携带 `{ path: <完整文件路径>, source: "file-changes-card" }`
- **AND** Files tab 自动激活并打开该文件

#### Scenario: DiffCard 查看完整文件
- **WHEN** DiffCard 中展示了一个文件的编辑结果
- **THEN** 文件名行旁边显示"查看文件"图标按钮
- **AND** 点击按钮 dispatch `xiaolin:open-file` 事件

### Requirement: 统一文件打开事件

所有"打开文件"的触发点 SHALL 使用同一个 CustomEvent。

#### Scenario: 事件格式
- **WHEN** 任何组件需要在 Files tab 中打开文件
- **THEN** dispatch `new CustomEvent("xiaolin:open-file", { detail: { path, line?, workDir?, source? } })`
- **AND** `path` 为文件路径（相对或绝对）
- **AND** `line` 为可选的跳转行号
- **AND** `workDir` 为可选的工作目录覆盖
- **AND** `source` 为触发来源标识

#### Scenario: 事件监听
- **WHEN** Files tab 组件挂载
- **THEN** 注册 `xiaolin:open-file` 全局事件监听
- **AND** 收到事件时调用 `fileViewerStore.openFile(path, line)`
- **AND** 自动激活 Files tab（`workspaceTabs.setActiveTab("files")`）

#### Scenario: 路径解析
- **WHEN** 事件中的 `path` 是相对路径
- **THEN** 使用当前 session 的 `workDir` 解析为绝对路径
- **AND** 如果事件携带了 `workDir`，优先使用事件中的 `workDir`
- **AND** 如果无法解析（workDir 为 null 且路径为相对），显示错误提示
