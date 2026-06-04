## MODIFIED Requirements

### Requirement: Below-input metadata
输入框下方 SHALL 显示执行环境和分支信息，分支数据来自 git-store。

#### Scenario: Git project metadata display
- **WHEN** git-store 的 `isGitRepo` 为 true
- **THEN** 下方显示 "Work locally" chip 和当前分支名 chip（从 git-store 的 `branch` 字段获取）
- **AND** 分支 chip 显示 git branch 图标 + 分支名

#### Scenario: Non-git project metadata display
- **WHEN** git-store 的 `isGitRepo` 为 false
- **THEN** 下方仅显示 "Work locally" chip
- **AND** 分支 chip SHALL NOT be rendered

#### Scenario: Branch selector dropdown
- **WHEN** 用户点击分支 chip
- **THEN** 显示分支列表下拉菜单，数据来自 git-store 的 `branches` 数组
- **AND** 当前分支高亮显示
- **AND** 用户选择后暂无操作（分支切换功能预留，需要后续 git checkout 集成）
