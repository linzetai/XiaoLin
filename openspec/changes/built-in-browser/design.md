## Context

XiaoLin 是一个基于 Tauri 2 的桌面 AI 助手，前端使用 React 19 + Zustand 5，后端使用 Rust + Axum 嵌入式 gateway。当前架构中：

- **Browser 工具**（`xiaolin-tools-browser`）通过 `headless_chrome` crate 使用 CDP 协议控制外部 Chrome 进程，支持 30+ actions（navigate、snapshot、screenshot、click、fill、cookies 等）。用户完全看不到 Agent 的浏览器操作。
- **WorkspacePanel** 已有成熟的多 Tab 系统（Files、Terminal、Plan、Review 等），支持动态注册、宽度调节、窗口自动扩展、per-session 记忆。
- **Chat 链接** 全部走 `target="_blank"` 打开系统默认浏览器。
- **网络代理**（`xiaolin-network-proxy`）提供 HTTP/SOCKS5 代理、域名 Allow/Deny 策略、MITM 抓包能力。
- **CSP** 当前设置 `frame-ancestors 'none'`，禁止 iframe 嵌入外部内容。

Tauri 2 支持在同一窗口内通过 `window.add_child(WebviewBuilder, position, size)` 嵌入多个独立 WebView，每个 WebView 有自己的进程、cookie 存储、CSP 策略。

## Goals / Non-Goals

**Goals:**
- 在 WorkspacePanel 中提供完整的多标签页浏览器体验
- Agent browser 工具直接操作内置 WebView，用户可实时看到操作过程
- Cookie 和 LocalStorage 跨会话持久化
- 支持 Host 映射和代理配置（复用现有 network-proxy 基础设施）
- Agent 可以通过 browser 工具设置 Host/代理（需用户确认）
- Chat 链接在内置浏览器中打开
- 浏览器内容可以发送给 Agent 分析

**Non-Goals (MVP 阶段):**
- 不支持浏览器扩展安装和管理
- 不支持移动端（iOS/Android 的 multi-webview 限制大）
- 不做浏览器级别的标签页拖拽分离成独立窗口
- 第一阶段不做 MITM 抓包可视化面板
- 不做跨设备书签/密码同步
- 不做多 Profile（隐身模式除外）

**Product Tier:**
- **Tier 1 (MVP)**: Agent 浏览可视化 + Chat 链接统一入口 + 基本多标签浏览 + Cookie 持久化 + Host/代理 + 全宽布局模式 + 下载管理（基础） + Agent/用户接管模式 + 安全隔离
- **Tier 2 (日常可用)**: 搜索引擎 Omnibox + 历史记录 + 书签 + 页内查找 + 缩放 + 右键菜单（混合方案） + 隐身模式 + 下载管理（进度条）
- **Tier 3 (完善)**: 标签恢复 + 崩溃恢复 + 媒体控制 + 密码保存 + 证书管理 + 打印

## Decisions

### D1: 使用 Tauri Child WebView（而非 iframe 或独立窗口）

**选择**: `window.add_child(WebviewBuilder::new(..., WebviewUrl::External(url)))` 嵌入子 WebView

**替代方案**:
- iframe：大量主流网站设置 `X-Frame-Options: DENY` 禁止嵌入，可行性极低
- 独立 Tauri 窗口：稳定但不在 Panel 内，打破 WorkspacePanel 统一布局
- CDP 镜像投射：只能展示截图，不是真正的浏览器体验

**理由**: Child WebView 是操作系统原生视图，有完整的浏览器能力（Cookie、JS、CSS、HTTPS），且可以精确定位到 WorkspacePanel 内的区域。API 在 Tauri 2.11+ 已稳定。

### D2: 所有 Browser WebView 共享同一 data_directory

**选择**: 所有页面共享 `~/.local/share/xiaolin/browser-data/` 作为 data_directory

**替代方案**:
- 每个页面独立 data_directory：Cookie 不共享，用户需要在每个标签页分别登录
- 按域名分组 data_directory：实现复杂，且用户期望与普通浏览器一致

**理由**: 用户期望登录一次后所有标签页都保持登录状态，与普通浏览器行为一致。

### D3: 默认走 XiaoLin 内置代理（XiaolinProxy 模式）

**选择**: Browser WebView 的 `proxy_url` 始终指向内置 `xiaolin-network-proxy`，通过代理层实现 Host 映射和上游代理

**替代方案**:
- 直接使用 WebView `proxy_url` 指向用户的代理：无法实现 Host 映射，且 `proxy_url` 创建时固定，运行时切换需要重建 WebView
- 不走代理直连：无法实现 Host 映射，无法热切换代理配置

**理由**: 内置代理层提供了 Host 映射（DNS 重写）、代理热切换（无需重建 WebView）、域名过滤等能力。代理层本地 loopback 延迟极低（< 1ms）。

### D4: BrowserEngine trait 抽象，保留 CDP fallback

**选择**: 抽象 `BrowserEngine` trait，实现 `TauriWebViewEngine`（内置 WebView）和 `CdpEngine`（现有 headless_chrome），Tauri 环境默认使用前者，纯服务器环境 fallback 到后者

**替代方案**:
- 完全删除 CDP 实现：纯服务器模式（无 Tauri GUI）将失去 browser 能力
- 同时运行两个引擎：资源浪费，状态不一致

**理由**: XiaoLin 同时支持桌面模式（有 Tauri）和纯 gateway 模式（无 GUI），两种模式需要不同的浏览器引擎。

### D5: 通过 Tauri Event 桥接 WebView 状态到前端

**选择**: 使用 `on_navigation`、`on_page_load`、`on_document_title_changed` 回调 + `app_handle.emit()` 将 WebView 状态变化通知到主 WebView（React）

**替代方案**:
- JS postMessage 跨 WebView 通信：child WebView 与 main WebView 不共享 origin，postMessage 不可行
- Rust 轮询 eval：延迟高且不可靠
- title-based hack 通道：仅用于用户操作（如选中文本发送给 Agent）等无原生回调的场景

**理由**: Tauri 的 WebView 回调已经覆盖了 URL 变化、标题变化、加载状态等主要事件，且是推送模式，延迟低、可靠性高。

### D6: Agent 操作通过 JS injection 替代 CDP

**选择**: click、fill、scroll 等交互操作通过 `webview.eval(js)` 注入 JS 执行，take_snapshot 和 console/network 监控通过 `initialization_script` 注入钩子

**替代方案**:
- 继续用 CDP 控制内置 WebView：WebKitGTK 不支持 CDP
- 使用 WebDriver/Selenium：需要额外进程，复杂度高

**理由**: WebView 的 `eval()` 和 `initialization_script()` 提供了足够的 JS 执行能力，可以覆盖 30+ actions 中的绝大多数。少数需要原生 API 的操作（screenshot、cookies）通过 `with_webview()` 访问平台底层 API。

### D7: WebView 定位同步使用 ResizeObserver + IPC + scaleFactor

**选择**: React 端通过 ResizeObserver 监听占位 div 的位置/尺寸变化，通过 IPC 通知 Rust 端调用 `webview.set_position()` + `webview.set_size()`。所有坐标必须使用 LogicalPosition/LogicalSize（CSS 像素），Tauri 内部处理 HiDPI 缩放。

**理由**: Child WebView 是操作系统级视图，不受 CSS 布局控制，必须通过命令式 API 设置位置。ResizeObserver 能捕获 Panel resize、窗口缩放、Tab 切换等所有场景。

**关键细节**:
- `getBoundingClientRect()` 返回 CSS 像素，与 Tauri 的 `LogicalPosition` 一致
- Panel 关闭（`panelOpen=false`）时，必须将所有 Browser WebView 移出可见区域（`set_position(-9999, -9999)`），因为 React 树卸载但 OS 级 WebView 仍存在
- 窗口最小化/失焦时，WebView 自动跟随窗口状态，无需额外处理

### D9: Panel 关闭时 WebView 隐藏策略

**选择**: 当 WorkspacePanel 关闭或切换到非 Browser Tab 时，将 WebView 移到屏幕外 (`set_position(-9999, -9999)`) 而非 `set_size(0, 0)`

**替代方案**:
- `set_size(0, 0)`: WebKitGTK 可能将零尺寸视为 hidden，暂停 JS 执行，导致 Agent 后台操作超时
- 保持可见但覆盖 z-index: Child WebView 的 z-index 不受 CSS 控制

**理由**: 移到屏幕外保持 WebView「可见」状态（OS 层面），避免 WebKitGTK 的 JS 节流问题。同时用户看不到浮动的 WebView。

### D10: Browser 布局模式——突破 700px Panel 限制

**选择**: 支持两种布局模式：
1. **Panel 模式（默认）**: 在 WorkspacePanel 右侧面板中显示，受 700px 上限限制
2. **全宽模式**: Browser 成为 ContentBlock 的主内容区域，Chat 收缩为左侧可折叠面板（280-500px 可拖拽）

**替代方案**:
- 浮动气泡 + 弹出层：Agent 长回复时体验差（需反复展开/收起）
- 上下分屏：垂直空间对浏览器同样不足
- 覆盖式（Chat 半透明覆盖 Browser）：遮挡网页内容

**全宽模式布局**:
- Chat 成为左侧面板（280-500px，可拖拽，可折叠到 48px 窄条）
- Browser 占据 flex:1 剩余空间
- WorkspacePanel 仍可正常使用（但 Browser Tab 自动隐藏，因为 Browser 已在全宽区域）
- Chat 折叠时显示窄条：Chat 图标 + 未读 badge + pulse 动画

**模式切换实现**:
- 不重建 WebView——仅通过 ResizeObserver 检测新占位容器的位置/尺寸并 IPC 更新
- 切换动画：先截取 WebView 快照显示为 `<img>`，动画期间 WebView 移到屏幕外，动画完成后恢复
- 触发方式：Browser Tab 内按钮、快捷键 Ctrl+Shift+F、双击拖拽条

**理由**: 700px 对日常浏览严重不足。侧边 Chat 面板保持 Chat 可见，用户可以边浏览边对话，同时 Agent 交互不中断。

### D11: Session 策略——全局 Browser + per-chat 上下文注入

**选择**: Browser 页面是全局的（不随 Chat session 切换而消失），但 Agent 上下文注入是 per-chat 的

**理由**: 用户浏览是跨会话的（打开一个文档页面，切换到不同的 Chat 继续讨论），不应该因为切换 Chat session 就丢失所有浏览器标签。

### D8: Agent 设置 Host/代理需要用户确认

**选择**: Agent 调用 `set_hosts` / `set_proxy` 时，通过 WS event 弹出确认面板，等待用户批准后才执行。默认超时 30 秒自动拒绝。

**理由**: Host 映射和代理设置是安全敏感操作，恶意 prompt 可能诱导 Agent 将金融网站指向钓鱼服务器。用户确认是必要的安全门。

### D12: Custom Protocol 作为 Browser WebView ↔ Rust 主通信通道

**选择**: 使用 `register_asynchronous_uri_scheme_handler("xiaolin-internal", ...)` 注册自定义协议，Browser WebView 内的 JS 通过 `fetch('xiaolin-internal://callback', { method: 'POST', body: JSON.stringify(data) })` 与 Rust 通信

**替代方案**:
- title hack（`document.title` 临时改写 → `on_document_title_changed` 回调）：标题闪烁、数据大小受限、无双向通信、存在竞态
- 全局变量 + 二次 eval：延迟高、不可靠
- Tauri IPC：Browser WebView 被 capability 隔离，无 IPC 权限

**优势**:
- 无闪烁、无副作用
- 支持任意大小数据（POST body）
- 支持双向通信（response 中返回数据）
- 无竞态问题

**安全**:
- 白名单消息类型（ready/snapshot/console/network/selection/dialog）
- 请求体大小限制（MAX_IPC_MESSAGE_BYTES）
- Spike 1.7 验证 custom protocol handler 是否受 capability 隔离影响

**降级**: 如果 Spike 验证 custom protocol 不可用，退回 title hack + 全局变量方案。

### D13: BROWSER_INIT_SCRIPT 分层架构

**选择**: 注入脚本分为两类：
1. `initialization_script`（自动注入每个页面，~3KB minified）: Layer 0-3
2. `webview.eval()`（按需注入）: Layer 4-7

**Layer 划分**:
- **Layer 0 (~800B)**: 基础通信框架（`__XIAOLIN__` 命名空间、custom protocol send/notify、页面 ready 信号）
- **Layer 1 (~600B)**: Console/Error 钩子（console.log/warn/error 劫持、onerror、unhandledrejection）
- **Layer 2 (~1.2KB)**: Network 监控钩子（fetch 包装器、XHR 代理，只记录 method/url/status/timing）
- **Layer 3 (~400B)**: Dialog 劫持（alert/confirm/prompt 重写，通过 custom protocol 通知 Rust）
- **Layer 4 (~2KB, eval)**: UID 标记系统（MutationObserver 增量更新，take_snapshot 时注入）
- **Layer 5 (~1KB, eval)**: Agent 交互高亮（元素高亮 CSS、操作动画效果）
- **Layer 6 (~2.5KB, eval)**: 选中文本浮动工具栏（selectionchange 监听、浮动 UI）
- **Layer 7 (~5KB, eval)**: 内容提取（精简版 readability 算法）

**安全保护**:
- `Object.freeze(__XIAOLIN__)` + `Object.defineProperty(window, '__XIAOLIN__', { writable:false, configurable:false, enumerable:false })`
- 内部数据通过闭包封装，外部只能通过 frozen 方法访问
- `initialization_script` 在页面 JS 之前执行，不可被覆盖

### D14: Agent/用户接管模式

**选择**: 三种操作模式 + 平滑转换：

1. **Free Mode（默认）**: 用户完全控制浏览器，Agent 无操作进行中
2. **Agent Control Mode**: Agent 调用 browser tool 时自动进入
   - 用户可以：滚动、选中文本、切换其他 browser tab
   - 用户点击/输入会被拦截，弹出 "Agent 操作中" 提示 + [中止 Agent] 按钮
   - UI 标记：页面标签 🤖 前缀、地址栏状态条、半透明蓝色遮罩（pointer-events:none）
3. **User Takeover**: 用户确认中止 Agent → Agent 当前 action 返回 `user_takeover` 错误

**关键设计**:
- 接管模式是 per-page 的（一个页面的 Agent 控制不影响其他页面）
- 防闪烁：连续 Agent action 之间 500ms 内没有新 action 才退出 Agent Control Mode
- 操作可视化：click/fill 等操作前高亮目标元素（橙色脉冲 300ms → 绿色闪烁完成）
- 操作日志面板实时更新

### D15: 下载管理（Tier 1）

**选择**: 通过 Tauri 2 的 `on_download` 回调实现基础下载管理

**Tier 1 能力**:
- `DownloadEvent::Requested`: 检测下载开始 → 弹出底部通知栏（文件名 + 状态）
- `DownloadEvent::Finished`: 下载完成 → 提供 [打开文件] [打开目录] 按钮
- 无进度条（Tauri on_download API 不提供 Progress 事件）

**Tier 2 增强路径**:
- 通过 `with_webview()` 访问 WebKitGTK 的 `WebKitDownload` 的 `received-data` signal 获取进度
- 或拦截 URL → Rust reqwest 自行下载（有进度回调）

### D16: 五层安全模型

**Layer 0 — Tauri Capability 隔离**:
修改 `capabilities/default.json` 的 `windows` 为 `webviews: ["main"]`。Browser WebView（label: `browser-{uuid}`）不匹配任何 capability → 零 IPC 权限。阻止恶意网站 JS 调用 Tauri 命令。

**Layer 1 — Custom Protocol 白名单**:
`xiaolin-internal://` 只处理白名单消息类型（ready/snapshot/console/network/selection/dialog），请求体大小限制。

**Layer 2 — JS 对象保护**:
`Object.freeze` + `configurable:false` + 闭包封装，阻止恶意网页读取/篡改 `__XIAOLIN__` 对象。

**Layer 3 — 导航 URL 过滤**:
deny-by-default 白名单（http/https only），阻止 file://、javascript:、data:、tauri:// 等协议。

**Layer 4 — Agent 操作用户确认**:
Host 映射、代理设置需要用户确认。Agent 操作审计日志。take_snapshot 返回内容标记为"不受信任来源"。

**Layer 5 — 网络层保护**:
DNS rebinding 防护（缓存解析结果复用）、域名通配符标签边界匹配。

### D17: WebView 生命周期状态模型

BrowserPage 使用两个正交状态维度：

**可见性维度 (PageVisibility)**:
- `Active`: 在 Panel/全宽中可见，接收用户输入
- `Hidden`: 屏幕外（pos -9999,-9999），JS 仍执行，Agent 可操作

**加载维度 (PageLoadState)**:
- `Loading`: 导航中
- `Ready`: 加载完成
- `Failed(String)`: 加载失败

同一时刻最多一个 WebView 处于 Active 状态。Hidden 状态的 WebView JS 必须可执行。Destroyed 后的引用必须立即清除。

## Risks / Trade-offs

### R1: WebKitGTK 对 hidden WebView 的 JS 暂停
- **风险**: WebKitGTK 可能暂停不可见 WebView 的 JS 执行，导致 Agent 操作超时
- **缓解**: 使用 D9 的屏幕外定位策略（`set_position(-9999, -9999)`）而非隐藏，保持 WebView 在 OS 层面为「可见」状态。Spike 1.1 中必须验证此策略。

### R2: WebView 定位同步的精度和延迟
- **风险**: Panel resize 时 WebView 位置可能有短暂的偏移/闪烁
- **缓解**: 使用 `requestAnimationFrame` 批量更新；resize 期间暂时隐藏 WebView

### R3: `webview.eval()` 是 fire-and-forget
- **风险**: Tauri 的 `webview.eval()` 不返回 JS 执行结果，Agent 工具依赖返回值
- **缓解**: 使用 D12 的 Custom Protocol 通道——Agent eval 的 JS 通过 `fetch('xiaolin-internal://callback')` 回传结果，支持任意大小数据和双向通信。如 custom protocol 不可用，降级到全局变量 + 二次 eval 方案。

### R4: 截图功能缺乏原生 API
- **风险**: Tauri WebView 没有直接的 `screenshot()` 方法
- **缓解**: 分层策略——优先通过 `with_webview()` 访问 WebKitGTK 的 `webkit_web_view_get_snapshot()`；降级使用 `html2canvas` JS 库

### R5: 内置代理增加网络路径复杂度
- **风险**: 所有请求经过本地代理可能影响性能或引入新的故障点
- **缓解**: loopback 延迟极低（< 1ms）；代理层已经过生产验证；失败时可降级到直连模式

### R6: 多平台 Cookie 持久化行为差异
- **风险**: `data_directory` 在 Linux/Windows/macOS 上行为不完全一致（macOS 使用 `data_store_identifier` 替代）
- **缓解**: 在代码中按平台分支处理；macOS 使用 `data_store_identifier([u8; 16])` 而非 `data_directory`

### R7: Agent/用户操作冲突
- **风险**: Agent 操作期间用户交互可能导致 DOM 状态变化，Agent 后续操作失败
- **缓解**: 使用 D14 的接管模式，在 Agent Control 期间拦截用户的破坏性操作（click/input），允许非破坏性操作（scroll/select）。用户可随时中止 Agent。

### R8: Custom Protocol 被恶意网页调用
- **风险**: 任何在 Browser WebView 中运行的 JS 都可以调用 `fetch('xiaolin-internal://...')`
- **缓解**: 白名单消息类型 + 大小限制 + 最坏情况影响有限（只能注入虚假 console/network 记录）

### R9: 全宽模式切换的 WebView 闪烁
- **风险**: 模式切换时 WebView 需要改变尺寸和位置，可能出现闪烁或空白
- **缓解**: 切换前截取快照显示为 `<img>`，动画期间 WebView 移到屏幕外，动画完成后恢复
