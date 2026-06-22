## ADDED Requirements

### Requirement: Files workspace tab

WorkspacePanel SHALL 注册一个 "Files" 标签页，提供文件浏览和查看能力。

#### Scenario: Tab 注册
- **WHEN** 应用启动
- **THEN** "Files" 标签在 `AppShell.tsx` 中通过 `registerTab()` 注册
- **AND** 使用 `Files` Phosphor 图标
- **AND** `order` 为 2（Review 改为 1，Files 为 2，Goal 改为 3，Terminal 改为 4）

#### Scenario: Tab 自动打开
- **WHEN** Agent 在当前 session 中通过工具调用创建或修改文件
- **THEN** Files tab 自动激活并打开 WorkspacePanel（若未打开）
- **AND** 被操作的文件自动在查看器中打开

#### Scenario: Tab badge
- **WHEN** Agent 在当前 session 中操作了文件且 Files tab 非当前活跃 tab
- **THEN** Files tab 显示 badge（文件操作计数）
- **AND** 切换到 Files tab 后 badge 清除

### Requirement: 分栏布局

Files tab 内容区 SHALL 采用左右分栏布局：左侧文件列表 + 右侧文件查看器。

#### Scenario: 默认布局
- **WHEN** 打开 Files tab 且面板宽度 ≥ 400px
- **THEN** 左侧文件列表占 180px，右侧查看器占剩余空间
- **AND** 中间有 1px 分隔线

#### Scenario: 窄面板布局
- **WHEN** Files tab 打开且面板宽度 < 400px
- **THEN** 文件列表默认折叠为 36px 图标条（仅显示文件图标）
- **AND** 查看器占满剩余空间
- **AND** 点击图标条可展开文件列表（overlay 模式，不挤压查看器）

#### Scenario: 手动折叠文件列表
- **WHEN** 用户点击文件列表区域的折叠按钮
- **THEN** 列表折叠为 36px 图标条
- **AND** 查看器获得更多空间

### Requirement: 多文件 Tab 栏

查看器区域上方 SHALL 显示已打开文件的 tab 栏，支持多文件切换。

#### Scenario: 打开新文件
- **WHEN** 用户在文件列表中点击一个未打开的文件
- **THEN** 在 tab 栏新增一个 tab（显示文件名）
- **AND** 自动切换到该 tab
- **AND** 查看器渲染该文件内容

#### Scenario: 切换文件 tab
- **WHEN** 用户点击 tab 栏中的另一个文件 tab
- **THEN** 查看器切换到该文件
- **AND** 保留之前的滚动位置和折叠状态

#### Scenario: 关闭文件 tab
- **WHEN** 用户点击 tab 上的 × 关闭按钮
- **THEN** 关闭该 tab
- **AND** 自动切换到相邻的 tab（优先右侧，其次左侧）
- **AND** 如果是最后一个 tab，显示空状态提示

#### Scenario: LRU 自动关闭
- **WHEN** 打开的文件 tab 超过 10 个
- **THEN** 自动关闭最久未访问的 tab
- **AND** 不关闭当前活跃的 tab

### Requirement: 空状态

Files tab 在无内容时 SHALL 显示引导性空状态。

#### Scenario: 无 session artifact 且无打开文件
- **WHEN** 当前 session 无 artifact 记录且未手动打开任何文件
- **THEN** 显示居中空状态：文件图标 + "尚无文件" 提示文字
- **AND** 如果 `workDir` 已设置，显示"浏览工作目录"按钮

### Requirement: 首次打开自动扩展面板宽度

Files tab 首次激活时 SHALL 尝试将面板宽度扩展至 500px。

#### Scenario: 面板宽度扩展
- **WHEN** Files tab 首次被激活且当前面板宽度 < 500px
- **THEN** 自动将面板宽度调整为 500px（如果屏幕空间允许）
- **AND** 如果屏幕空间不足以扩展到 500px，保持当前宽度不变
