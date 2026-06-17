## ADDED Requirements

### Requirement: Bearer Token 环境变量支持

`McpServerConfig` SHALL 支持 `bearer_token_env_var` 字段，指定一个环境变量名。连接 HTTP-based MCP 服务器时，系统 SHALL 从该环境变量读取 token 值，作为 `Authorization: Bearer <token>` header 附加到所有 HTTP 请求。

禁止在配置中直接嵌入 token 明文（`bearer_token` 字段 SHALL 被拒绝并报错）。

#### Scenario: 环境变量存在且非空
- **WHEN** `McpServerConfig` 配置了 `bearer_token_env_var: "MY_TOKEN"` 且环境变量 `MY_TOKEN="abc123"` 存在
- **THEN** 所有 HTTP 请求 SHALL 携带 `Authorization: Bearer abc123` header

#### Scenario: 环境变量不存在
- **WHEN** `bearer_token_env_var` 指向的环境变量不存在
- **THEN** 服务器状态 SHALL 设为 `Failed`，错误信息包含环境变量名

#### Scenario: 拒绝内联 token
- **WHEN** 配置文件包含 `bearer_token: "some_value"` 字段
- **THEN** `validate()` SHALL 返回错误，提示使用 `bearer_token_env_var` 替代

### Requirement: 自定义 HTTP Headers

`McpServerConfig` SHALL 支持 `http_headers` 字段（`HashMap<String, String>`），用于在所有 HTTP 请求中附加自定义 header。

支持环境变量引用：header 值以 `$` 开头时（如 `$API_KEY`），SHALL 从对应环境变量读取实际值。

#### Scenario: 静态 header
- **WHEN** 配置 `http_headers: { "X-Custom": "value" }`
- **THEN** 所有 HTTP 请求 SHALL 携带 `X-Custom: value` header

#### Scenario: 环境变量引用 header
- **WHEN** 配置 `http_headers: { "X-Api-Key": "$MY_API_KEY" }` 且 `MY_API_KEY="key123"`
- **THEN** 所有 HTTP 请求 SHALL 携带 `X-Api-Key: key123` header

### Requirement: OAuth 2.0 PKCE 授权码流

系统 SHALL 支持 MCP 2025-06-18 规范定义的 OAuth 认证流程：

1. **Metadata Discovery**：从 MCP 服务器 URL 的 `/.well-known/oauth-authorization-server` 端点发现 OAuth 配置
2. **Authorization**：使用 PKCE（S256）生成 `code_verifier` + `code_challenge`，打开浏览器导航到授权端点
3. **Callback**：通过本地回调服务器（`127.0.0.1:随机端口`）接收授权码
4. **Token Exchange**：用授权码 + `code_verifier` 换取 `access_token` + `refresh_token`
5. **Token Refresh**：`access_token` 过期时自动用 `refresh_token` 刷新
6. **Token Storage**：token 持久化存储，优先 keyring，降级文件系统

#### Scenario: 完整 OAuth 流程
- **WHEN** 连接一个 HTTP MCP 服务器，初始请求返回 401，且服务器提供 OAuth metadata
- **THEN** 服务器状态 SHALL 设为 `NeedsAuth`，前端 SHALL 显示"登录"按钮

#### Scenario: 用户完成 OAuth 授权
- **WHEN** 用户点击"登录"按钮，完成浏览器授权流程
- **THEN** 系统 SHALL 获取并存储 token，自动重新连接服务器

#### Scenario: Token 自动刷新
- **WHEN** `access_token` 过期（HTTP 401）且 `refresh_token` 有效
- **THEN** 系统 SHALL 自动刷新 token 并重试原请求，不中断用户操作

#### Scenario: OAuth metadata 不可用
- **WHEN** 服务器返回 401 但未提供 OAuth metadata
- **THEN** 服务器状态 SHALL 设为 `Failed`，错误信息提示需要手动配置 `bearer_token_env_var`

### Requirement: NeedsAuth 状态

`McpStatus` 枚举 SHALL 新增 `NeedsAuth` 变体，表示服务器需要认证但当前无有效 token。

#### Scenario: NeedsAuth 状态展示
- **WHEN** 服务器状态为 `NeedsAuth`
- **THEN** 前端 `PluginRow` SHALL 显示黄色认证图标和"登录"操作按钮

#### Scenario: NeedsAuth 缓存
- **WHEN** 服务器进入 `NeedsAuth` 状态
- **THEN** 系统 SHALL 缓存该状态至少 15 分钟，期间不重复探测 OAuth（避免频繁 401）
