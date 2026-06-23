## ADDED Requirements

### Requirement: 选中文本发送给 Agent
系统 SHALL 在 Browser WebView 中支持用户选中文本后通过浮动工具栏将内容发送到 Chat。

#### Scenario: 浮动工具栏出现
- **WHEN** 用户在 Browser WebView 中选中超过 5 个字符的文本
- **THEN** 选区上方出现浮动工具栏，包含 [🤖 问 Agent] [📋 复制] [💬 引用] 按钮

#### Scenario: 问 Agent
- **WHEN** 用户点击浮动工具栏的 "🤖 问 Agent" 按钮
- **THEN** 选中文本和页面 URL 作为引用块填入 Chat 输入框，Chat 输入框获焦

#### Scenario: 引用到 Chat
- **WHEN** 用户点击浮动工具栏的 "💬 引用" 按钮
- **THEN** 选中文本作为 blockquote 格式添加到 Chat 输入框

### Requirement: 网页内容提取
系统 SHALL 通过 initialization_script 注入内容提取函数，Agent 可通过 evaluate action 调用。

#### Scenario: 提取纯文本
- **WHEN** Agent 调用 evaluate 执行 __xiaolin_extract.text()
- **THEN** 返回页面的可读纯文本内容（去除 script/style/nav 等）

#### Scenario: 提取表格数据
- **WHEN** Agent 调用 evaluate 执行 __xiaolin_extract.tables()
- **THEN** 返回页面中所有 table 元素的结构化数据（headers + rows）

#### Scenario: 提取链接
- **WHEN** Agent 调用 evaluate 执行 __xiaolin_extract.links()
- **THEN** 返回页面中所有 http/https 链接及其文本

#### Scenario: 提取元数据
- **WHEN** Agent 调用 evaluate 执行 __xiaolin_extract.metadata()
- **THEN** 返回页面的 title、description、OpenGraph 标签、JSON-LD 数据

### Requirement: 浏览器上下文自动注入
系统 SHALL 在 Agent 处理用户消息时，如果 Browser Panel 有活跃页面，自动将当前浏览器状态注入到上下文中。

#### Scenario: 上下文注入
- **WHEN** 用户在 Chat 中发送消息，且 Browser Panel 有活跃页面
- **THEN** Agent 的上下文中包含当前页面的 URL、标题、页面数量等信息

### Requirement: Agent 接管/用户接管切换
系统 SHALL 支持 Agent 控制和用户控制之间的切换。

#### Scenario: Agent 请求用户接管
- **WHEN** Agent 遇到需要人工操作的场景（如验证码）
- **THEN** Browser Panel 显示 "Agent 请求你操作" 提示，用户可以手动操作页面

#### Scenario: 用户完成并恢复 Agent
- **WHEN** 用户完成手动操作，点击 "继续"
- **THEN** Agent 恢复浏览器控制，继续执行任务
