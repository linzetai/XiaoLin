## 1. 技术 Spike & 基础设施

- [ ] 1.1 **Spike: WebView + Panel 隐藏策略验证**
  创建最小 Tauri 2 示例，在主窗口内 `add_child` 一个外部 URL WebView。验证：
  - 定位、显示、关闭正常（Linux WebKitGTK）
  - `set_position(-9999, -9999)` 后 JS `setInterval` 是否继续执行（写日志验证）
  - 恢复 `set_position()` 后页面状态是否保持
  - **预期产出**: 确认策略可行或记录替代方案
- [ ] 1.2 **Spike: Custom Protocol 通信验证**（首选）
  测试 `register_asynchronous_uri_scheme_handler("xiaolin-internal", ...)` 在 Browser WebView 中的可用性：
  - JS `fetch('xiaolin-internal://callback')` 能否到达 Rust handler
  - POST body 能否正常传递 JSON
  - Response body 能否返回数据给 JS
  - Custom protocol 是否受 capability 隔离影响
  - **降级**: 如不可行，回退到 title hack + 全局变量方案（Spike 1.2b）
- [ ] 1.3 **Spike: Cookie 持久化 + HttpOnly**
  创建 WebView 配置 `data_directory`，访问需要登录的网站，关闭重开确认：
  - Cookie 持久化
  - HttpOnly cookie 不暴露给 `document.cookie`
  - 多个 WebView 共享同一 `data_directory` 时 cookie 共享
  - macOS 使用 `data_store_identifier` 验证同等行为
- [ ] 1.4 **Spike: 原生截图 API**
  通过 `with_webview()` 访问 WebKitGTK 的 `webkit_web_view_get_snapshot()`，确认能获取 PNG 数据
- [ ] 1.5 **Spike: HiDPI 坐标一致性**
  在 2x 显示器上验证 `getBoundingClientRect()` CSS px 与 Tauri `LogicalPosition` 一致性
- [ ] 1.6 **Spike: IPC 隔离验证**
  验证 `webviews: ["main"]` capability 配置下：
  - Browser WebView 是否真的无法调用 Tauri IPC 命令
  - Browser WebView 中 `window.__TAURI_INTERNALS__` 是否不存在或不可用
- [ ] 1.7 **Spike: Custom Protocol + Capability 兼容性**
  验证 `register_asynchronous_uri_scheme_handler` 与 capability 隔离是否互相影响
  - Browser WebView 零 IPC 权限时 custom protocol 是否仍然可用
- [ ] 1.8 **Spike: Object.freeze 保护有效性**
  验证 `Object.freeze` + `configurable:false` 在 WebKitGTK 的 `initialization_script` 中：
  - 恶意页面 JS 能否覆盖 `window.__XIAOLIN__`
  - `initialization_script` 是否确实在页面 JS 之前执行

## 2. Rust 后端 — BrowserPanelManager

- [x] 2.1 新增 `crates/xiaolin-app/src-tauri/src/browser_panel.rs`：BrowserPanelManager + BrowserPage struct（含 PageVisibility/PageLoadState 正交状态）
- [x] 2.2 IPC: `browser_open_page` — 创建 Child WebView
  - `data_directory` 配置
  - `on_navigation` deny-by-default 过滤
  - `on_page_load` / `on_document_title_changed` 回调
  - `register_asynchronous_uri_scheme_handler("xiaolin-internal", ...)` 注册
  - `initialization_script` 注入 Layer 0-3
- [x] 2.3 IPC: `browser_close_page` — 关闭 WebView 并清理资源
- [x] 2.4 IPC: `browser_navigate` — 对指定 page 调用 navigate
- [x] 2.5 IPC: `browser_go_back` / `browser_go_forward` / `browser_reload`
- [x] 2.6 IPC: `browser_resize_webview` — 使用 `LogicalPosition` / `LogicalSize`（前端传 CSS px）
- [x] 2.7 IPC: `browser_list_pages`
- [x] 2.8 IPC: `browser_show_page` / `browser_hide_all_pages` — 显示/隐藏控制（屏幕外定位策略）
- [x] 2.9 IPC: `browser_eval_js` — 在指定页面 WebView 中执行 JS
- [x] 2.10 注册所有 IPC 命令，添加 BrowserPanelManager 为 Tauri managed state
- [x] 2.11 `on_new_window` 回调：拦截 window.open，在内置浏览器中新建页面
- [x] 2.12 `on_navigation` 回调：deny-by-default 白名单过滤
  - 允许: `http://`, `https://`
  - 拒绝: `file://`, `javascript:`, `data:` (顶级导航), `tauri://`, `ipc://`, `asset://`
  - 未知协议: 拒绝 + `tracing::warn!` 记录
- [x] 2.13 Custom Protocol handler: `xiaolin-internal://callback`
  - 白名单消息类型: ready/snapshot/console/network/selection/dialog
  - 请求体大小限制
  - 未知类型返回 403
- [x] 2.14 `on_download` 回调：下载检测 + 保存 + 通知前端
- [x] 2.15 BROWSER_INIT_SCRIPT Layer 0-3（~3KB）
  - Layer 0: `__XIAOLIN__` 命名空间 + custom protocol send/notify + Object.freeze 保护
  - Layer 1: Console/Error 钩子
  - Layer 2: Network 监控钩子（fetch + XHR）
  - Layer 3: Dialog 劫持

## 3. 前端 — Browser Store 和 UI 组件

- [x] 3.1 `lib/stores/browser-store.ts`：BrowserPage 接口 + Zustand store
  - pages, activePageId, layoutMode, chatPanelWidth, chatPanelCollapsed
  - openPage, closePage, navigate, setLayoutMode, toggleChatPanel
  - Agent control 状态 per-page
- [x] 3.2 Tauri Event 监听注册（browser-url-changed、browser-title-changed、browser-loading、browser-user-action、browser-download-*）
- [x] 3.3 `components/browser/BrowserAddressBar.tsx`
  - 后退、前进、刷新 + URL 输入框 + 安全指示器（HTTPS 锁图标）
  - Agent Control Mode 状态条 + [取回控制] 按钮
  - [全宽/侧栏] 切换按钮
- [x] 3.4 `components/browser/BrowserPageTabs.tsx`：页面标签栏 + 新建按钮 + Agent 控制标记 (🤖)
- [x] 3.5 `components/browser/BrowserPlaceholder.tsx`：WebView 占位 div + ResizeObserver + IPC 定位（CSS px → LogicalPosition）
- [x] 3.6 `components/browser/BrowserTabContent.tsx`：组合 AddressBar + PageTabs + Placeholder + DownloadBar
- [x] 3.7 `components/browser/BrowserFullPanel.tsx`：全宽模式的 Browser 容器（复用 BrowserTabContent 内部组件）
- [x] 3.8 `components/browser/ChatSidePanel.tsx`：全宽模式下的 Chat 左侧面板（可拖拽、可折叠）
- [x] 3.9 `components/browser/DownloadNotificationBar.tsx`：下载通知栏
- [x] 3.10 AppShell.tsx 注册 Browser Tab（order: 6）
- [x] 3.11 Tab 切换隐显逻辑：切离时 `browser_hide_all_pages()`，切回时 `browser_show_page(activePageId)`
- [x] 3.12 页面切换隐显逻辑
- [x] 3.13 Panel 关闭/打开联动：`panelOpen` 变化时调用 `browser_hide_all_pages()` / `browser_show_page()`
- [x] 3.14 全宽模式实现：ContentBlock 条件渲染 + CSS transition + WebView 快照动画
- [x] 3.15 Agent Control Mode UI：半透明遮罩 + toast 拦截 + [中止 Agent] 按钮

## 4. Chat 链接拦截

- [x] 4.1 `MarkdownContent.tsx` Link 组件：读取用户配置，http/https 默认内置浏览器打开，Shift+Click 反转
- [x] 4.2 同步修改 `StreamingMarkdown.tsx` / `MarkdownViewer.tsx`（如有链接组件）
- [x] 4.3 新增用户配置项：链接打开方式（内置浏览器 / 外部浏览器），存储于 settings store

## 5. BrowserEngine 抽象 & Agent 工具迁移

- [x] 5.1 `crates/xiaolin-tools-browser/src/engine/mod.rs`：定义 BrowserEngine trait
- [x] 5.2 `engine/cdp_engine.rs`：封装现有 headless_chrome 为 CdpEngine
- [x] 5.3 `engine/webview_engine.rs`：TauriWebViewEngine 通过 AppHandle 操作内置 WebView
- [x] 5.4 重构 BrowserTool：从直接使用 headless_chrome 改为 BrowserEngine trait
- [x] 5.5 迁移导航类 actions（navigate、go_back、go_forward、reload）
- [x] 5.6 迁移交互类 actions（click、fill、fill_form、type_text、press_key、hover、scroll、select）→ JS injection + Agent Control Mode 联动
- [x] 5.7 迁移快照类 actions（take_snapshot、get_content）→ WebView eval + `untrusted_webpage` 标记
- [x] 5.8 迁移截图 action → 平台原生 API + html2canvas fallback
- [x] 5.9 迁移页面管理 actions（list_pages、select_page、new_page、close_page）→ BrowserPanelManager
- [x] 5.10 迁移 DevTools actions → custom protocol 回传 / initialization_script 捕获数据
- [x] 5.11 迁移 cookies action → `document.cookie`（仅非 HttpOnly）
- [x] 5.12 迁移 wait_for → JS 轮询
- [x] 5.13 迁移 drag、handle_dialog、interact、emulate、resize_page、pdf、upload_file
- [x] 5.14 操作可视化高亮 JS（Layer 5, eval 注入）
- [x] 5.15 Agent Control Mode 拦截 JS（initialization_script 中的 capture phase 事件监听）
- [x] 5.16 gateway 启动逻辑分支：Tauri → TauriWebViewEngine，纯 gateway → CdpEngine

## 6. 网络配置 — Host 映射 & 代理

- [ ] 6.1 `xiaolin-network-proxy/src/config.rs` 新增 HostMapping + 通配符标签边界匹配（规则 #42）
- [ ] 6.2 代理连接阶段 DNS 重写（缓存解析结果复用，规则 #41）
- [ ] 6.3 BrowserNetworkConfig struct + 持久化
- [ ] 6.4 browser tool actions: set_hosts、set_proxy、get_network_config、clear_hosts
- [ ] 6.5 Agent 网络变更用户确认机制
- [ ] 6.6 前端 BrowserNetworkSettings 组件
- [ ] 6.7 前端 HostMappingConfirmPanel 组件

## 7. Browser ↔ Chat 内容交互

- [ ] 7.1 BROWSER_INIT_SCRIPT Layer 6（选中文本浮动工具栏，eval 注入）
- [ ] 7.2 Custom Protocol 通信：选中文本/引用 → `xiaolin-internal://callback` → Rust emit → 主 WebView
- [ ] 7.3 前端 browser-user-action 事件 → Chat 输入框填充
- [ ] 7.4 浏览器上下文自动注入 Agent 上下文
- [ ] 7.5 Agent 操作日志面板

## 8. Tauri 配置 & 权限

- [x] 8.1 `capabilities/default.json`: `windows: ["main"]` → `webviews: ["main"]`（关键安全变更）
- [x] 8.2 确认 Browser WebView label 格式 `browser-{uuid}` 不匹配任何 capability
- [x] 8.3 `tauri.conf.json` CSP 调整（child WebView 不受主 WebView CSP 限制——确认 CSP 是 per-webview 的）
- [x] 8.4 评估 macos-proxy feature，更新 Cargo.toml

## 9. 测试 & 验证

### 9.1 Spike 验收（Gate：全部通过后才进入 Phase 2）
- [ ] 9.1.1 Spike 1.1: WebView 隐藏策略
- [ ] 9.1.2 Spike 1.2: Custom Protocol 通信
- [ ] 9.1.3 Spike 1.3: Cookie 持久化
- [ ] 9.1.4 Spike 1.4: 原生截图
- [ ] 9.1.5 Spike 1.5: HiDPI 坐标
- [ ] 9.1.6 Spike 1.6: IPC 隔离
- [ ] 9.1.7 Spike 1.7: Custom Protocol + Capability
- [ ] 9.1.8 Spike 1.8: Object.freeze 保护

### 9.2 安全测试
- [ ] 9.2.1 E2E: Browser WebView 无法调用 Tauri IPC
- [ ] 9.2.2 E2E: URL 过滤（file://、javascript:、tauri:// 被拦截）
- [ ] 9.2.3 E2E: Custom Protocol 未知类型被拒绝
- [ ] 9.2.4 E2E: `__XIAOLIN__` 对象不可被覆盖

### 9.3 功能测试
- [ ] 9.3.1 E2E: Browser Panel 打开页面、导航、多标签切换、关闭
- [ ] 9.3.2 E2E: Panel 关闭后 WebView 隐藏，重开后恢复
- [ ] 9.3.3 E2E: Cookie 持久化（登录 → 关闭 → 重开 → 登录状态保持）
- [ ] 9.3.4 E2E: Agent browser 工具操作在 Panel 中可见
- [ ] 9.3.5 E2E: Agent Control Mode 进入/退出/用户接管
- [ ] 9.3.6 E2E: Host 映射生效
- [ ] 9.3.7 E2E: Chat 链接在内置浏览器打开（含配置切换）
- [ ] 9.3.8 E2E: 选中文本发送给 Agent
- [ ] 9.3.9 E2E: 全宽布局模式切换 + Chat 面板折叠/展开
- [ ] 9.3.10 E2E: 下载检测 + 通知 + 打开文件

### 9.4 跨平台
- [ ] 9.4.1 macOS 核心功能验证
- [ ] 9.4.2 Windows 核心功能验证
