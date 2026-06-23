## ADDED Requirements

### Requirement: 浏览器为主体的全宽布局
全宽模式下，浏览器 SHALL 占据主内容区域的左侧（flex: 1），Chat 面板 SHALL 位于右侧。布局顺序为 `BrowserFullPanel → ChatSidePanel`。

#### Scenario: 进入全宽模式
- **WHEN** 用户从 Panel 模式切换到全宽模式
- **THEN** 浏览器 SHALL 从右侧工作区面板移动到主内容区域的左侧
- **THEN** Chat 面板 SHALL 显示在浏览器的右侧
- **THEN** 浏览器 SHALL 占据除 Chat 面板之外的所有剩余水平空间

#### Scenario: 全宽模式下浏览器为视觉焦点
- **WHEN** 全宽模式激活且 Chat 面板展开
- **THEN** 浏览器 SHALL 在布局中位于 Chat 面板的左侧（而非右侧或中间）

### Requirement: Chat 面板完全隐藏式折叠
Chat 面板折叠时 SHALL 完全隐藏（宽度 0px），不占用任何水平空间。

#### Scenario: 折叠 Chat 面板
- **WHEN** 用户折叠 Chat 面板
- **THEN** Chat 面板宽度 SHALL 过渡到 0px
- **THEN** 浏览器 SHALL 扩展到占据全部可用宽度

#### Scenario: Chat 折叠后 toggle 按钮可见
- **WHEN** Chat 面板处于折叠状态
- **THEN** 地址栏右端的 Chat toggle 按钮 SHALL 保持可见且可点击

### Requirement: Chat toggle 按钮整合到地址栏
全宽模式下，Chat 面板的展开/折叠 SHALL 通过地址栏右端的 toggle 按钮控制。

#### Scenario: 通过地址栏按钮切换 Chat
- **WHEN** 用户点击地址栏的 Chat toggle 按钮（💬 图标）
- **THEN** Chat 面板 SHALL 在展开和折叠之间切换
- **THEN** 按钮 SHALL 在 Chat 面板展开时显示为激活状态

#### Scenario: 未读消息指示
- **WHEN** Chat 面板折叠且有未读消息
- **THEN** Chat toggle 按钮 SHALL 显示未读消息 badge

### Requirement: Chat 面板拖拽调整宽度
Chat 面板展开时 SHALL 支持通过左侧边缘拖拽手柄调整宽度，范围 280-500px。

#### Scenario: 拖拽调整 Chat 宽度
- **WHEN** 用户拖拽 Chat 面板左侧边缘
- **THEN** Chat 面板宽度 SHALL 跟随鼠标移动，在 280-500px 范围内变化
- **THEN** 浏览器 SHALL 实时调整宽度以填充剩余空间

#### Scenario: 拖拽方向语义
- **WHEN** 用户向左拖拽 Chat 面板左侧边缘
- **THEN** Chat 面板 SHALL 变宽（因为面板在右侧，左边缘向左移动增加宽度）

### Requirement: 全宽模式下 WorkspacePanel 不显示
全宽浏览器模式下，WorkspacePanel SHALL 不在同一行显示，以最大化浏览器面积。

#### Scenario: 全宽模式隐藏 WorkspacePanel
- **WHEN** 全宽模式激活
- **THEN** WorkspacePanel SHALL 不渲染在 ContentBlock 内
- **THEN** 用户 SHALL 可通过快捷键或切换回 Panel 模式使用 WorkspacePanel

### Requirement: Chat 折叠时 children 保持挂载
Chat 面板折叠时 SHALL 保持 children（MessageStream 等）挂载，不使用条件渲染的独立 return 分支。

#### Scenario: 折叠后展开 Chat 状态保持
- **WHEN** Chat 面板从展开折叠到 0px 后再展开
- **THEN** Chat 的消息列表滚动位置、composer 输入内容、流式消息状态 SHALL 保持不变
- **THEN** 不出现组件重挂载导致的闪烁或状态丢失

#### Scenario: 折叠时流式消息持续接收
- **WHEN** Agent 正在回复消息且用户折叠 Chat 面板
- **THEN** 折叠期间流式消息 SHALL 继续接收和更新
- **THEN** 展开后 SHALL 立即显示最新的消息内容

### Requirement: 选区发送 Chat 时自动展开 Chat
全宽模式 + Chat 折叠时，用户从浏览器选中文本发送到 Chat SHALL 自动展开 Chat 面板。

#### Scenario: 折叠态选区询问自动展开
- **WHEN** 布局模式为全宽且 Chat 面板已折叠
- **AND** 用户在浏览器中选中文本并点击「询问」
- **THEN** Chat 面板 SHALL 自动展开
- **THEN** 选中的文本 SHALL 填入 Chat 输入框
