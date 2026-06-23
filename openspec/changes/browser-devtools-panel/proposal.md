## Why

内置浏览器的 `BROWSER_INIT_SCRIPT` 已经实现了 Layer 1（Console 钩子）和 Layer 2（Network fetch/XHR 监控），后端也已将这些数据通过 `browser-console` 和 `browser-network` 事件 emit 到主 WebView。但前端**完全没有消费这些事件**——所有的 console.log/error 和网络请求数据都被浪费了。用户和开发者无法在 XiaoLin 内查看页面的 console 输出或网络请求记录，需要打开外部浏览器的 DevTools 来调试——这违背了内置浏览器「日常可用」的定位。

## What Changes

- **新增 Console 面板**: 在浏览器底部面板中展示 console.log/warn/error/info/debug 消息，支持按级别过滤和清空
- **新增 Network 面板**: 在浏览器底部面板中展示 fetch/XHR 请求记录，包括 method、URL、状态码、耗时
- **底部面板多 Tab 化**: 将现有的 `AgentOperationLog` 扩展为多 Tab 底部面板，新增 Console 和 Network Tab
- **前端事件监听**: 在 `initBrowserEvents()` 中接入 `browser-console`、`browser-network` 事件
- **后端微调**: 在 custom protocol 白名单中确认 `console` 和 `network` 类型已被正确处理（已有，验证即可）

## Capabilities

### New Capabilities
- `browser-devtools-console`: Console 面板——展示页面 JS console 输出、按级别过滤、清空、per-page 隔离
- `browser-devtools-network`: Network 面板——展示 fetch/XHR 请求列表、状态码高亮、耗时显示、per-page 隔离
- `browser-devtools-panel`: 底部多 Tab 面板容器——Agent/Console/Network Tab 切换、可折叠/拖拽调整高度

### Modified Capabilities
- `browser-panel`: AgentOperationLog 从独立组件改为底部面板的一个 Tab

## Impact

- **前端 `crates/xiaolin-app/src/`**:
  - 新增 `components/browser/DevToolsPanel.tsx`: 底部多 Tab 面板容器
  - 新增 `components/browser/ConsolePanel.tsx`: Console 面板 UI
  - 新增 `components/browser/NetworkPanel.tsx`: Network 面板 UI
  - 修改 `components/browser/BrowserTabContent.tsx`: 用 DevToolsPanel 替换 AgentOperationLog
  - 修改 `lib/stores/browser-store.ts`: 新增 consoleEntries、networkEntries 状态 + 事件监听
- **后端无需改动**: `browser-console` 和 `browser-network` 事件已在 `browser_panel.rs` 中实现，仅需验证白名单完整性
- **无依赖变更**: 纯前端 UI 组件
