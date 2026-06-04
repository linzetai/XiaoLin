## MODIFIED Requirements

Git integration SHALL contribute a **Review** tab to the right-side **WorkspacePanel** (multi-purpose panel with a tab system). Git integration does not replace or own the entire panel—only the Review tab's content and footer.

### Requirement: Review tab registration
Git integration SHALL register the Review tab via the WorkspacePanel tab system:

```ts
{ id: "review", label: "Review", icon: FileIcon, component: ReviewTabContent, footer: ReviewTabFooter }
```

#### Scenario: Tab visible in WorkspacePanel
- **WHEN** the WorkspacePanel is rendered
- **THEN** the Review tab SHALL appear in the tab bar with label "Review" and FileIcon
- **AND** selecting the tab SHALL render `ReviewTabContent` in the panel body and `ReviewTabFooter` in the panel footer slot

#### Scenario: Notification badge on hidden tab
- **WHEN** a `git.status_changed` event arrives and the Review tab is not the active tab
- **AND** the updated status has staged or unstaged changes
- **THEN** a notification badge SHALL appear on the Review tab icon
- **AND** the badge SHALL clear when the user switches to the Review tab

### Requirement: File change list
Review tab content区顶部 SHALL 显示文件变更列表，分为 "Staged" 和 "Unstaged" 两个分组。数据来源从 mock 改为 git-store 的 `staged` 和 `unstaged` 数组。

#### Scenario: Unstaged files display
- **WHEN** git-store 的 `unstaged` 数组不为空
- **THEN** 显示 "Unstaged" 分组，列出所有变更文件名
- **AND** 每个文件名旁显示增删行数统计（绿色 +N / 红色 -N），数据来自 git-store

#### Scenario: Staged files display
- **WHEN** git-store 的 `staged` 数组不为空
- **THEN** 显示 "Staged" 分组，列出所有暂存的文件
- **AND** 每个文件名旁显示增删行数统计

#### Scenario: File selection
- **WHEN** 用户点击某个文件名
- **THEN** 调用 git-store 的 `selectFile(path, staged)` 加载该文件的 diff
- **AND** Review tab 内容区下方显示从后端获取的真实 diff 内容

#### Scenario: Empty state
- **WHEN** git-store 的 `staged` 和 `unstaged` 都为空
- **THEN** 显示 "No changes" 空状态提示

#### Scenario: Non-git project
- **WHEN** git-store 的 `isGitRepo` 为 false
- **THEN** Review tab 内容区显示 "Not a git repository" 提示

### Requirement: Inline diff rendering
选中文件后 SHALL 在 Review tab 内容区显示该文件的 inline diff，数据来自 git-store 的 `selectedDiff` hunks。

#### Scenario: Diff line display
- **WHEN** 选中一个有变更的文件且 git-store 返回 diff hunks
- **THEN** 显示 monospace 字体的 diff 内容
- **AND** 新增行（`+`）使用 `--green-line` 背景 + `--green-text` 文字色
- **AND** 删除行（`-`）使用 `--red-line` 背景 + `--red-text` 文字色
- **AND** 上下文行使用 `--text-3` 灰色文字
- **AND** 每行左侧显示行号（10px, 灰色, 右对齐, 32px 宽）

#### Scenario: Loading state
- **WHEN** diff 数据正在从后端加载
- **THEN** 显示 diff 区域的 loading skeleton

#### Scenario: Binary file
- **WHEN** 选中的文件是二进制文件（git-store 返回 `binary: true`）
- **THEN** 显示 "Binary file, diff not available" 提示

### Requirement: Footer action bar
Review tab 的 `ReviewTabFooter` SHALL 显示在 WorkspacePanel 底部操作栏，按钮执行真实 Git 操作。

#### Scenario: Stage all button
- **WHEN** 用户点击 "Stage all" 按钮
- **THEN** 调用 git-store 的 `stageFiles(["."])` 暂存所有变更
- **AND** 按钮显示 loading 状态直到操作完成

#### Scenario: Revert all button
- **WHEN** 用户点击 "Revert all" 按钮
- **THEN** 显示确认对话框 "确定要撤销所有未暂存的更改吗？此操作不可恢复。"
- **AND** 用户确认后调用 git-store 的 `revertFiles(["."])`

#### Scenario: Per-file stage/unstage
- **WHEN** 用户点击 unstaged 文件旁的 "+" 按钮
- **THEN** 调用 git-store 的 `stageFiles([path])` 暂存该文件
- **WHEN** 用户点击 staged 文件旁的 "-" 按钮
- **THEN** 调用 git-store 的 `unstageFiles([path])` 取消暂存该文件
