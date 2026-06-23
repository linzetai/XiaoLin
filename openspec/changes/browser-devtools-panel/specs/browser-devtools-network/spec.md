## ADDED Requirements

### Requirement: Network 请求列表展示
系统 SHALL 在 DevTools Network Tab 中展示当前活动页面的 fetch/XHR 网络请求记录。

#### Scenario: fetch 请求记录显示
- **WHEN** 浏览器页面内发起 `fetch('/api/data')` 并返回 200
- **THEN** Network Tab SHALL 显示一条记录：`GET /api/data 200 {timing}ms fetch`

#### Scenario: 请求失败记录显示
- **WHEN** 浏览器页面内发起 fetch 请求但网络错误
- **THEN** Network Tab SHALL 显示该记录并使用红色高亮

#### Scenario: 切换标签页时过滤请求
- **WHEN** 用户从标签页 A 切换到标签页 B
- **THEN** Network Tab SHALL 仅显示标签页 B 的网络请求

### Requirement: 状态码颜色编码
Network 面板 SHALL 对 HTTP 状态码使用颜色编码。

#### Scenario: 2xx 成功请求
- **WHEN** 网络请求返回 2xx 状态码
- **THEN** 状态码 SHALL 显示为绿色

#### Scenario: 4xx/5xx 错误请求
- **WHEN** 网络请求返回 4xx 或 5xx 状态码
- **THEN** 状态码 SHALL 显示为红色（5xx）或橙色（4xx）

### Requirement: 耗时颜色编码
Network 面板 SHALL 对请求耗时使用颜色编码。

#### Scenario: 慢速请求高亮
- **WHEN** 网络请求耗时超过 1000ms
- **THEN** 耗时数值 SHALL 显示为红色

### Requirement: Network 清空与上限
系统 SHALL 支持清空网络请求记录，且每个页面最多保留 200 条记录。

#### Scenario: 清空网络记录
- **WHEN** 用户点击清空按钮
- **THEN** 当前页面的所有网络请求 SHALL 被移除

#### Scenario: 超过 200 条记录
- **WHEN** 当前页面的网络请求达到 200 条且有新请求到达
- **THEN** 最旧的请求记录 SHALL 被移除

### Requirement: Network 覆盖范围标注
Network Tab SHALL 明确标注其仅覆盖 Fetch/XHR 请求，不包括子资源加载。

#### Scenario: Tab 标题标注
- **WHEN** Network Tab 激活
- **THEN** Tab 内容区域 SHALL 显示「Fetch / XHR」标注
