## Why

XiaoLin 当前的浏览器能力存在两个断裂：1) Agent 的 browser 工具通过外部 Chrome CDP 操作，用户无法实时看到 Agent 在做什么；2) Chat 中的链接全部通过 `target="_blank"` 在外部浏览器打开，导致用户在 XiaoLin 和系统浏览器之间频繁切换。内置浏览器可以将 Agent 浏览、用户浏览、Chat 上下文三者统一在一个界面中，实现真正的 AI 原生浏览体验。

**产品定位**: 内置浏览器不仅是 Agent 工具的可视化窗口，也是用户的**日常浏览器**。用户应能用它完成日常网页浏览（阅读文档、查资料、登录网站等），同时随时与 AI Agent 协作（选中内容分析、页面操作自动化等）。实现分三个 Tier 渐进交付。

## What Changes

- **新增 WorkspacePanel Browser Tab**：在右侧面板中新增浏览器标签页，支持地址栏、多页面标签、导航控制
- **Tauri 多 WebView 嵌入**：使用 `window.add_child()` 在主窗口内嵌入独立的 Tauri WebView 来渲染网页内容
- **替换 browser 工具引擎**：将 Agent 的 browser 工具从外部 Chrome CDP 迁移到内置 WebView，用户可实时看到 Agent 操作
- **Chat 链接拦截**：Chat 中的 http/https 链接点击后在内置浏览器中打开，而非外部浏览器
- **Cookie/Storage 持久化**：通过 WebView `data_directory` 配置实现跨会话的 Cookie、LocalStorage 持久化
- **网络配置能力**：支持 Host 映射、HTTP/SOCKS5 代理配置，复用 `xiaolin-network-proxy` 模块
- **Agent 网络控制**：Agent 可通过 browser 工具设置 Host 映射和代理（需用户确认）
- **Browser ↔ Chat 上下文共享**：选中网页内容可发送给 Agent 分析；Agent 自动感知当前浏览器状态

## Capabilities

### New Capabilities
- `browser-panel`: WorkspacePanel 中的 Browser Tab UI，包括地址栏、多页面标签、WebView 占位容器、加载状态、快捷键、全宽布局模式（侧边 Chat 面板）
- `browser-webview-manager`: Tauri 后端的 WebView 生命周期管理，包括创建、销毁、定位同步、事件桥接、data_directory 配置、Custom Protocol 通信通道
- `browser-network-config`: 内置浏览器的网络配置能力，包括代理模式选择、Host 映射、SSL 配置，与 xiaolin-network-proxy 集成
- `browser-agent-engine`: Agent browser 工具的 WebView 引擎实现，将 30+ CDP actions 迁移到 JS injection + Tauri WebView API
- `browser-content-interaction`: 浏览器与 Chat 的内容交互，包括选中文本发送给 Agent、网页内容提取、Agent 操作可视化
- `browser-security`: 五层安全模型——Capability IPC 隔离、Custom Protocol 白名单、JS 对象保护、URL 过滤、Agent 操作审计
- `browser-agent-takeover`: Agent/用户接管模式——Free Mode、Agent Control Mode、User Takeover 三种操作模式及其转换
- `browser-download`: 下载管理——下载检测、保存、通知 UI、目录配置

### Modified Capabilities
- `chat-link-behavior`: Chat 中链接点击行为从外部浏览器改为内置浏览器打开（可配置）

## Impact

- **前端 crates/xiaolin-app/src**：新增 `components/browser/` 目录（~10 个组件）、`lib/stores/browser-store.ts`；修改 `MarkdownContent.tsx` 链接行为；修改 `AppShell.tsx` 注册 Browser Tab；修改 `ContentBlock.tsx` 支持全宽模式
- **后端 crates/xiaolin-app/src-tauri**：新增 `browser_panel.rs` 模块和 ~10 个 IPC 命令；修改 `lib.rs` 注册命令；新增 Custom Protocol handler
- **Browser 工具 crates/xiaolin-tools-browser**：重构为引擎抽象层，新增 `TauriWebViewEngine` 实现，保留 `CdpEngine` 作为 fallback
- **网络代理 crates/xiaolin-network-proxy**：新增 Host 映射功能
- **Tauri 配置**：
  - `capabilities/default.json`: `windows` → `webviews` 字段变更（关键安全变更）
  - `tauri.conf.json`: CSP 确认（child WebView CSP 独立于主 WebView）
- **依赖变更**：可能需要启用 Tauri 的 `macos-proxy` feature
- **Spike 前置**：8 个技术验证项必须全部通过后才进入实现阶段
