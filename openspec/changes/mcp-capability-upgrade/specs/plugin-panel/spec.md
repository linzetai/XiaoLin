## MODIFIED Requirements

### Requirement: Plugin list display
`PluginPanel` SHALL 展示已配置 MCP 插件列表；每项包含：显示名称（id）、状态徽章、scope 标签（user / project）、启用/禁用 toggle。

#### Scenario: List shows configured plugins
- **WHEN** `plugins.list` 返回至少一条插件
- **THEN** 列表渲染所有插件行，按名称排序
- **AND** 每行显示 id 作为标题、scope 标签、状态徽章、enable toggle

#### Scenario: Status badge connected
- **WHEN** 插件 `status` 为 `connected`
- **THEN** 显示绿色徽章（文案如「Connected」）

#### Scenario: Status badge error
- **WHEN** 插件 `status` 为 `failed` 且存在 `lastError`
- **THEN** 显示红色徽章（文案如「Error」）

#### Scenario: Status badge disabled
- **WHEN** 插件 `enabled` 为 false 或 `status` 为 `disabled`
- **THEN** 显示灰色徽章（文案如「Disabled」）

#### Scenario: Status badge connecting
- **WHEN** 插件 `status` 为 `connecting`
- **THEN** 显示中性/黄色进行中指示（可选 spinner）

#### Scenario: Status badge needs-auth
- **WHEN** 插件 `status` 为 `needs_auth`
- **THEN** 显示黄色认证徽章（文案如「Needs Auth」），行内 SHALL 显示"登录"操作按钮

## ADDED Requirements

### Requirement: OAuth 登录按钮
当插件状态为 `needs_auth` 时，`PluginRow` SHALL 显示"登录"按钮。点击触发 OAuth 授权流程。

#### Scenario: 点击登录
- **WHEN** 用户点击 `needs_auth` 状态插件的"登录"按钮
- **THEN** 系统 SHALL 启动 OAuth 流程（打开浏览器授权页），按钮切换为"授权中..."加载状态

#### Scenario: 登录成功
- **WHEN** OAuth 授权完成
- **THEN** 插件 SHALL 自动重新连接，状态变为 `connecting` 然后 `connected`

### Requirement: Resources 和 Prompts 展示
插件详情展开区域 SHALL 新增 Resources 和 Prompts 子标签（在现有 Tools 标签旁）。

#### Scenario: Resources 标签
- **WHEN** 用户展开一个声明了 resources 能力的插件
- **THEN** SHALL 显示 Resources 子标签，列出该服务器的可用资源（name、uri、description）

#### Scenario: Prompts 标签
- **WHEN** 用户展开一个声明了 prompts 能力的插件
- **THEN** SHALL 显示 Prompts 子标签，列出该服务器的可用 prompts（name、description、arguments）

#### Scenario: 无 Resources/Prompts 能力
- **WHEN** MCP 服务器未声明 resources 或 prompts 能力
- **THEN** 对应标签 SHALL 不显示
