## ADDED Requirements

### Requirement: BrowserEngine trait 抽象
系统 SHALL 定义 BrowserEngine trait 作为浏览器操作的抽象接口，支持 TauriWebViewEngine 和 CdpEngine 两种实现。

#### Scenario: Tauri 环境
- **WHEN** 应用以 Tauri 桌面模式运行
- **THEN** BrowserTool 使用 TauriWebViewEngine

#### Scenario: 纯 Gateway 环境
- **WHEN** 应用以纯 gateway 模式运行（无 Tauri GUI）
- **THEN** BrowserTool fallback 到 CdpEngine（headless_chrome）

### Requirement: 导航类 Actions
系统 SHALL 支持 navigate、go_back、go_forward、reload action，通过 WebView API 或 JS eval 实现。

#### Scenario: navigate 到 URL
- **WHEN** Agent 调用 browser tool action=navigate url=https://example.com
- **THEN** 内置 WebView 导航到该 URL，用户在 Browser Panel 中实时看到

#### Scenario: go_back
- **WHEN** Agent 调用 browser tool action=go_back
- **THEN** 内置 WebView 执行 history.back()

### Requirement: 交互类 Actions
系统 SHALL 支持 click、fill、fill_form、type_text、press_key、hover、scroll、drag、select、wait_for action，通过 JS injection 实现。

#### Scenario: click 操作
- **WHEN** Agent 调用 browser tool action=click selector="button.submit"
- **THEN** WebView eval JS 查找元素并 click()，操作前在页面上高亮目标元素

#### Scenario: fill 操作
- **WHEN** Agent 调用 browser tool action=fill selector="input#email" value="test@test.com"
- **THEN** WebView eval JS 设置 input value 并触发 input/change 事件

#### Scenario: wait_for 操作
- **WHEN** Agent 调用 browser tool action=wait_for selector=".success-message" timeout=10000
- **THEN** WebView eval JS 轮询检查元素出现，超时返回错误

### Requirement: 快照和截图 Actions
系统 SHALL 支持 take_snapshot、screenshot、get_content action。

#### Scenario: take_snapshot
- **WHEN** Agent 调用 browser tool action=take_snapshot
- **THEN** WebView eval 注入的 a11y 树 JS，返回带 UID 标记的 DOM 树结构

#### Scenario: screenshot
- **WHEN** Agent 调用 browser tool action=screenshot
- **THEN** 通过平台原生 API（优先）或 html2canvas（降级）截取 WebView 内容，返回图片

### Requirement: 页面管理 Actions
系统 SHALL 支持 list_pages、select_page、new_page、close_page action，通过 BrowserPanelManager 实现。

#### Scenario: list_pages
- **WHEN** Agent 调用 browser tool action=list_pages
- **THEN** 返回所有打开页面的 id、url、title 列表

#### Scenario: new_page
- **WHEN** Agent 调用 browser tool action=new_page url=https://docs.example.com
- **THEN** 在内置浏览器中新建标签页打开该 URL

### Requirement: DevTools Actions
系统 SHALL 支持 evaluate、list_console_messages、list_network_requests action。

#### Scenario: evaluate
- **WHEN** Agent 调用 browser tool action=evaluate expression="document.title"
- **THEN** WebView eval 该 JS 并返回结果

#### Scenario: list_console_messages
- **WHEN** Agent 调用 browser tool action=list_console_messages
- **THEN** 返回通过 initialization_script 捕获的 console 消息列表

#### Scenario: list_network_requests
- **WHEN** Agent 调用 browser tool action=list_network_requests
- **THEN** 返回通过 initialization_script 捕获的 fetch/XHR 请求列表

### Requirement: Cookie Actions
系统 SHALL 支持 cookies action（get/set/delete/clear），但仅限非 HttpOnly cookie。

#### Scenario: get cookies
- **WHEN** Agent 调用 browser tool action=cookies operation=get
- **THEN** 通过 `document.cookie` 返回当前页面的非 HttpOnly cookie 列表
- **AND** 响应中注明 HttpOnly cookie 不可通过此方式访问

#### Scenario: set cookie
- **WHEN** Agent 调用 browser tool action=cookies operation=set cookie_name=token cookie_value=abc
- **THEN** 通过 `document.cookie = ...` 在 WebView 中设置该 cookie（仅限非 HttpOnly）

### Requirement: Agent Session 策略
Agent 的 browser 操作 SHALL 基于全局 Browser 页面列表工作，不与特定 Chat session 绑定。

#### Scenario: 跨 Chat 使用同一页面
- **WHEN** Agent 在 Chat A 中通过 browser 打开页面 P，用户切换到 Chat B
- **THEN** 页面 P 仍然存在，Chat B 的 Agent 可以操作它

#### Scenario: Agent 操作目标页面
- **WHEN** Agent 调用 browser tool 未指定 page_id
- **THEN** 使用当前活跃页面（Browser Panel 中选中的 tab）

#### Scenario: Agent 后台操作
- **WHEN** Agent 正在操作 Browser 而用户关闭了 WorkspacePanel
- **THEN** Agent 操作继续执行（WebView 在屏幕外保持可执行），结果正常返回

### Requirement: 操作可视化高亮
系统 SHALL 在 Agent 执行交互类操作（click、fill、hover 等）前，在页面上短暂高亮目标元素。详见 `browser-agent-takeover` spec。

#### Scenario: 点击前高亮
- **WHEN** Agent 执行 click 操作
- **THEN** 目标元素被橙色脉冲边框高亮 300ms 后执行点击，完成后绿色闪烁

#### Scenario: 操作日志
- **WHEN** Agent 执行任何 browser action
- **THEN** Browser Panel 底部的操作日志实时显示：时间戳 + 操作类型 + 目标描述

### Requirement: Agent 操作模式联动
Agent browser tool 调用 SHALL 自动触发 Agent Control Mode。详见 `browser-agent-takeover` spec。

#### Scenario: 进入 Agent Control
- **WHEN** Agent 调用 browser tool action
- **THEN** 目标页面进入 Agent Control Mode

#### Scenario: 用户中止
- **WHEN** Agent 操作期间用户中止
- **THEN** 当前 action 返回 `{ error: "user_takeover" }` 错误
- **AND** Agent 可以在 Chat 中告知用户操作被中止

### Requirement: 不受信任内容标记
系统 SHALL 在返回网页内容给 Agent 时标记来源可信度。

#### Scenario: take_snapshot 内容标记
- **WHEN** Agent 调用 take_snapshot
- **THEN** 返回结果中包含 `source: "untrusted_webpage"`

#### Scenario: get_content 内容标记
- **WHEN** Agent 调用 get_content
- **THEN** 返回结果中包含 `source: "untrusted_webpage"` 和 `warning: "content may contain prompt injection"`
