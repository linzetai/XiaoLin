## ADDED Requirements

### Requirement: Elicitation 能力声明

系统 SHALL 在 MCP `initialize` 请求的 `capabilities` 中声明 `elicitation` 能力，表示客户端支持处理服务器发起的用户输入请求。

#### Scenario: 声明 elicitation 能力
- **WHEN** 连接 MCP 服务器进行 `initialize` 握手
- **THEN** 请求的 `capabilities` SHALL 包含 `elicitation: {}`

### Requirement: Elicitation 请求处理

系统 SHALL 处理 MCP 服务器发送的 `elicitation/create` 请求：

1. 接收请求（包含 `message`、`requestedSchema` 描述需要收集的字段）
2. 通过 WebSocket 转发给前端
3. 前端显示表单 UI
4. 用户填写并提交后，将结果返回给 MCP 服务器

支持的 schema 字段类型：`string`、`number`、`boolean`、`enum`（`oneOf`）。

#### Scenario: 基础表单 elicitation
- **WHEN** MCP 服务器发送 `elicitation/create`，schema 包含 `{ "name": { "type": "string" } }`
- **THEN** 前端 SHALL 显示包含文本输入框的对话框，用户填写后结果回传给服务器

#### Scenario: 用户取消 elicitation
- **WHEN** 用户关闭 elicitation 对话框
- **THEN** 系统 SHALL 回复 `elicitation/create` 结果为 `{ "action": "decline" }`

#### Scenario: Elicitation 超时
- **WHEN** elicitation 请求在 5 分钟内未得到用户响应
- **THEN** 系统 SHALL 自动回复 `{ "action": "decline" }` 并关闭前端对话框

### Requirement: Elicitation 前端 UI

前端 SHALL 提供 `ElicitationDialog` 组件，用于显示 MCP 服务器发起的 elicitation 请求。

#### Scenario: 表单渲染
- **WHEN** 收到 `mcp.elicitation.request` WebSocket 事件
- **THEN** SHALL 弹出模态对话框，标题显示服务器名称和 `message`，表单根据 `requestedSchema` 渲染

#### Scenario: 多字段表单
- **WHEN** `requestedSchema` 包含多个字段（如 `username: string`、`remember: boolean`）
- **THEN** 对话框 SHALL 渲染对应的输入控件（文本框、复选框等），保持字段顺序
