## ADDED Requirements

### Requirement: 代理模式选择
系统 SHALL 支持四种代理模式：不使用代理、跟随系统代理、自定义代理、XiaoLin 内置代理（默认）。

#### Scenario: XiaolinProxy 默认模式
- **WHEN** 用户未修改代理设置
- **THEN** 所有 Browser WebView 的 proxy_url 指向 xiaolin-network-proxy 的本地端口

#### Scenario: 自定义代理
- **WHEN** 用户在设置中选择自定义代理并输入 socks5://proxy:1080
- **THEN** 新创建的 WebView 使用该代理 URL

#### Scenario: 无代理模式
- **WHEN** 用户选择不使用代理
- **THEN** 新创建的 WebView 不设置 proxy_url，直连

### Requirement: Host 映射
系统 SHALL 支持自定义域名到 IP 的映射，通过内置代理层实现 DNS 重写。

#### Scenario: 添加 Host 映射
- **WHEN** 用户添加映射 api.dev.com → 192.168.1.100
- **THEN** 内置代理在收到 api.dev.com 的请求时，连接到 192.168.1.100

#### Scenario: 通配符映射
- **WHEN** 用户添加映射 *.internal.corp → 172.16.0.1
- **THEN** 内置代理对 a.internal.corp、b.internal.corp 等请求都连接到 172.16.0.1

#### Scenario: 映射即时生效
- **WHEN** 用户修改 Host 映射
- **THEN** 无需重建 WebView，下一个请求立即使用新映射

#### Scenario: 域名通配符边界匹配
- **WHEN** 映射为 *.example.com
- **THEN** notexample.com 不受影响，仅 example.com 及其子域名匹配

### Requirement: 代理热切换
系统 SHALL 支持在不重建 WebView 的情况下切换上游代理配置（仅在 XiaolinProxy 模式下）。

#### Scenario: 切换上游代理
- **WHEN** 用户在 XiaolinProxy 模式下修改上游代理地址
- **THEN** 新请求通过新的上游代理，已有页面无感知

### Requirement: 网络配置持久化
系统 SHALL 将代理模式、自定义代理地址和 Host 映射持久化到配置文件。

#### Scenario: 配置保存
- **WHEN** 用户修改网络配置
- **THEN** 配置写入 app data 目录下的配置文件

#### Scenario: 启动时恢复
- **WHEN** 应用启动
- **THEN** 读取持久化的网络配置并应用

### Requirement: Agent 设置 Host 映射
系统 SHALL 允许 Agent 通过 browser 工具的 set_hosts action 设置 Host 映射，但 MUST 要求用户确认。

#### Scenario: Agent 请求设置 Host
- **WHEN** Agent 调用 browser tool 的 set_hosts action
- **THEN** 前端弹出确认面板，显示映射详情和原因

#### Scenario: 用户批准
- **WHEN** 用户在确认面板点击"允许"
- **THEN** Host 映射生效，标记为临时（会话结束清除）

#### Scenario: 用户拒绝
- **WHEN** 用户在确认面板点击"拒绝"或超时 30 秒
- **THEN** Host 映射不生效，Agent 收到拒绝消息

### Requirement: Agent 设置代理
系统 SHALL 允许 Agent 通过 browser 工具的 set_proxy action 设置代理，但 MUST 要求用户确认。

#### Scenario: Agent 请求设置代理
- **WHEN** Agent 调用 browser tool 的 set_proxy action
- **THEN** 前端弹出确认面板，显示代理地址和原因

#### Scenario: 安全验证
- **WHEN** Agent 请求设置 Host 映射目标 IP 为 0.0.0.0 或 127.0.0.1
- **THEN** 安全检查拒绝该请求
