## ADDED Requirements

### Requirement: WebView 生命周期管理
系统 SHALL 在 Rust 后端提供 BrowserPanelManager，负责 Tauri Child WebView 的创建、定位、隐藏和销毁。

#### Scenario: 创建 WebView
- **WHEN** 前端通过 IPC 请求打开一个 URL
- **THEN** 后端创建一个 Tauri Child WebView，设置 data_directory、导航到 URL，返回 page_id

#### Scenario: 销毁 WebView
- **WHEN** 前端通过 IPC 请求关闭一个页面
- **THEN** 后端调用 webview.close() 销毁对应 WebView，释放资源

#### Scenario: 页面数量上限
- **WHEN** 已有 8 个 WebView 实例，请求创建新的
- **THEN** 返回错误提示超过上限

### Requirement: WebView 状态模型
系统 SHALL 为每个 BrowserPage 维护两个正交状态维度：可见性（Active/Hidden）和加载状态（Loading/Ready/Failed）。

#### Scenario: 状态不变式——单活跃
- **WHEN** 切换活跃页面
- **THEN** 同一时刻最多一个 WebView 处于 Active 状态

#### Scenario: Hidden 状态 JS 可执行
- **WHEN** WebView 处于 Hidden 状态（屏幕外）
- **THEN** JS 定时器、网络请求等继续执行（Spike 1.1 验证）

#### Scenario: 页面导航状态转换
- **WHEN** 已加载页面发生导航（用户点击链接或 Agent navigate）
- **THEN** load_state 从 Ready 变为 Loading，导航完成后回到 Ready

### Requirement: Custom Protocol 通信通道
系统 SHALL 通过 `xiaolin-internal://` 自定义协议在 Browser WebView 和 Rust 之间建立通信。

#### Scenario: 注册协议处理器
- **WHEN** 创建 Browser WebView
- **THEN** 注册 `xiaolin-internal` URI scheme handler

#### Scenario: JS → Rust 通信
- **WHEN** Browser WebView 内的 JS 调用 `fetch('xiaolin-internal://callback', { method: 'POST', body: JSON.stringify({type, data}) })`
- **THEN** Rust 端 `register_asynchronous_uri_scheme_handler` 接收并处理

#### Scenario: Rust → JS 响应
- **WHEN** Rust 端处理完 custom protocol 请求
- **THEN** 可以在 HTTP response body 中返回数据给 JS（双向通信）

### Requirement: WebView 定位同步
系统 SHALL 将 Child WebView 精确定位到前端 WorkspacePanel 中 Browser 占位容器的位置。

#### Scenario: 初始定位
- **WHEN** WebView 创建后，前端获取占位 div 的 BoundingClientRect
- **THEN** 通过 IPC 调用 webview.set_position() 和 set_size() 将 WebView 定位到正确位置

#### Scenario: Panel 大小变化
- **WHEN** 用户拖拽调整 WorkspacePanel 宽度
- **THEN** 前端通过 ResizeObserver 检测变化并更新 WebView 尺寸

#### Scenario: Tab 切换隐藏
- **WHEN** 用户从 Browser Tab 切换到 Files Tab
- **THEN** 所有 Browser WebView 移到屏幕外（set_position(-9999, -9999)），保持 JS 可执行

#### Scenario: Tab 切换回显示
- **WHEN** 用户切换回 Browser Tab
- **THEN** 活跃页面的 WebView 恢复到正确位置，非活跃页面保持屏幕外

#### Scenario: Panel 整体关闭
- **WHEN** 用户关闭 WorkspacePanel（panelOpen=false）
- **THEN** 所有 Browser WebView 移到屏幕外，但保持存活（不销毁）

#### Scenario: Panel 重新打开
- **WHEN** 用户重新打开 WorkspacePanel 且 Browser Tab 为活跃 Tab
- **THEN** 活跃页面的 WebView 恢复到占位容器的正确位置

### Requirement: Cookie 和 Storage 持久化
系统 SHALL 通过 WebView data_directory 配置实现 Cookie（包括 HttpOnly）和 LocalStorage 的持久化，所有页面共享同一存储。

data_directory 由系统 WebView 引擎管理，XiaoLin 不直接读写 cookie 文件——HttpOnly cookie 由 WebView 进程在网络层自动处理，不暴露给 JS 也不需要应用层参与。

#### Scenario: Cookie 跨会话保持
- **WHEN** 用户在页面 A 登录，关闭应用，重新打开并访问同一网站
- **THEN** 登录状态保持（包括 HttpOnly session cookie）

#### Scenario: Cookie 跨标签共享
- **WHEN** 用户在页面 A 登录某网站，在页面 B 打开同一网站
- **THEN** 页面 B 也处于登录状态

#### Scenario: macOS data_store_identifier
- **WHEN** 在 macOS 上运行
- **THEN** 使用 data_store_identifier 替代 data_directory 实现持久化

#### Scenario: Agent 读 Cookie（仅非 HttpOnly）
- **WHEN** Agent 请求读取当前页面的 cookie
- **THEN** 通过 webview.eval("document.cookie") 返回非 HttpOnly 的 cookie
- **AND** 不尝试读取 HttpOnly cookie（无法也不应访问）

### Requirement: WebView 事件桥接
系统 SHALL 将 Child WebView 的关键事件通过 Tauri Event 转发给主 WebView。

#### Scenario: URL 变化
- **WHEN** WebView 发生导航（用户点击链接、JS 跳转、history API 等）
- **THEN** emit "browser-url-changed" 事件到主 WebView

#### Scenario: 标题变化
- **WHEN** WebView 页面标题变化
- **THEN** emit "browser-title-changed" 事件到主 WebView

#### Scenario: 加载状态
- **WHEN** WebView 开始/完成页面加载
- **THEN** emit "browser-loading" 事件到主 WebView

### Requirement: 导航安全过滤
系统 SHALL 通过 on_navigation 回调拦截非安全 URL，使用白名单策略（deny-by-default）。

#### Scenario: 阻止 file 协议
- **WHEN** WebView 尝试导航到 file:// URL
- **THEN** 导航被阻止

#### Scenario: 阻止 javascript 协议
- **WHEN** WebView 尝试导航到 javascript: URL
- **THEN** 导航被阻止

#### Scenario: 阻止 data 协议（大文档）
- **WHEN** WebView 尝试导航到 data: URL（非内联小资源）
- **THEN** 顶级导航被阻止（iframe 内嵌的小型 data: URI 允许）

#### Scenario: 阻止自定义协议绕过
- **WHEN** WebView 尝试导航到 tauri://, ipc://, asset:// 等内部协议
- **THEN** 导航被阻止

#### Scenario: 允许 HTTP/HTTPS
- **WHEN** WebView 尝试导航到 http:// 或 https:// URL
- **THEN** 导航正常进行

#### Scenario: 未知协议 deny-by-default
- **WHEN** WebView 尝试导航到未知协议（ftp://, smb://, custom:// 等）
- **THEN** 导航被阻止，日志记录被拦截的 URL

### Requirement: window.open 处理
系统 SHALL 通过 on_new_window 回调拦截 window.open 请求，在内置浏览器中打开新页面而非弹出系统窗口。

#### Scenario: 页面内 window.open
- **WHEN** 网页 JS 调用 window.open("https://example.com")
- **THEN** 在内置浏览器中新建一个页面标签打开该 URL
