## Context

XiaoLin 内置浏览器的 `BROWSER_INIT_SCRIPT` Layer 1 已劫持 `console.log/warn/error/info/debug`，Layer 2 已包裹 `fetch` 和 `XMLHttpRequest`。这些数据通过 `__XIAOLIN__.notify()` 发送到 Rust 后端，后端通过 `browser-console` 和 `browser-network` 两个 Tauri Event emit 到主 WebView。但 `browser-store.ts` 的 `initBrowserEvents()` 中**没有 listen 这两个事件**。

现有的底部面板 `AgentOperationLog` 已占据 `BrowserTabContent` 底部位置，最大高度 160px，展示 Agent 操作记录。

后端已有的数据结构：
- **Console**: `{ level, args: string[] }` — 参数截断到 10 个字符串（注意：`timestamp` 字段在 IPC 路径中可能缺失，前端需用接收时间戳 `Date.now()` 补充）
- **Network**: `{ type: 'fetch'|'xhr', method, url, status, timing, error? }` — 仅 fetch/XHR，无 request/response body，无子资源（img/script/css）

## Goals / Non-Goals

**Goals:**
- 在浏览器底部提供 Console 和 Network 面板，消费已有的后端事件
- Console 支持按 level 过滤、清空、per-page 隔离
- Network 支持请求列表、状态码高亮、耗时显示、per-page 隔离
- 底部面板支持 Agent/Console/Network 三个 Tab 切换
- 面板高度可拖拽调整，可折叠

**Non-Goals:**
- 不实现 Elements/DOM 检查器
- 不实现 Sources/调试器
- 不实现 Network request/response body 查看（需要代理 MITM 或增强 Layer 2）
- 不实现 Performance profiling
- 不增加后端 Rust 代码——复用已有事件管线
- 不修改 BROWSER_INIT_SCRIPT——现有 Layer 1/2 数据足够 MVP

## Decisions

### D1: 底部面板容器设计——多 Tab 替代单一 AgentOperationLog

**选择**: 新增 `DevToolsPanel` 容器组件，包含 Agent、Console、Network 三个 Tab。`AgentOperationLog` 成为 Agent Tab 的内容。

**Agent Tab 空态**: `AgentOperationLog` 原来在无操作时 `return null`（整个底部面板消失）。整合后 `DevToolsPanel` 容器**始终渲染 Tab 栏**（28px），Agent Tab 内容为空时显示「暂无 Agent 操作」占位文案，不影响 Console/Network Tab 的可见性和切换。

**替代方案**:
- Console/Network 作为 WorkspacePanel Tab：全宽模式下 WorkspacePanel 被隐藏（browser-ux-redesign D7），DevTools 也看不到
- 浮动窗口：额外的窗口管理复杂度
- 仅在 Agent Tab 内混合展示：信息密度过高，难以过滤

**理由**: 底部面板是 Chrome DevTools 的标准位置，用户熟悉。多 Tab 让每种信息有独立空间。

### D2: 面板高度——可拖拽 + 预设值

**选择**: 默认高度 200px，可通过顶部拖拽条在 100px~50%（视口高度的一半）之间调整。折叠时高度为 0px（仅显示 Tab 栏 28px）。

**理由**: 200px 够显示 5-6 条记录，不过分侵占 WebView 空间。最大 50% 与 Chrome DevTools 类似。

### D3: Console 面板数据结构与过滤

**选择**: Console 消息存储在 `browser-store.ts` 的 `consoleEntries: Record<string, ConsoleEntry[]>` 中，key 为 `pageId`，实现 per-page 隔离。支持按 level（log/warn/error/info/debug）过滤。最大保留 500 条 per-page，超出时 FIFO 淘汰。

**Zustand selector 安全性**：组件中使用 `useStore(s => s.consoleEntries[activePageId])` 取出当前页面数组（引用稳定，仅在该页面有新消息时变化），再通过 `useMemo` 做 level 过滤。禁止 `useStore(s => s.consoleEntries.filter(...))` 这类每次返回新引用的 selector。

**数据流**: `browser-console` event → `initBrowserEvents()` listen → push to `consoleEntries[pageId]` → Console Tab 渲染

**时间戳策略**: 即使 payload 中包含 `ts` 字段（BROWSER_INIT_SCRIPT Layer 1 的 `notify()` 会附加 `ts: Date.now()`），前端**统一使用接收时间** `Date.now()`，不使用 payload.ts。原因：(1) invoke 路径可能不含 ts；(2) 避免两条 IPC 路径的时间不一致；(3) console 面板主要看相对顺序和大致时间，接收时间精度够用。

**界面元素**:
- 过滤器：`[All] [Errors] [Warnings] [Info] [Debug]` 级别按钮
- 清空按钮：🚫 清空当前页面的 console
- 每条消息：`[HH:MM:SS.ms] [level icon] message args...`
- error 级别：红色背景 + 红色文字
- warn 级别：黄色背景
- 自动滚动到底部（新消息时），手动滚动中断自动滚动

### D4: Network 面板数据结构

**选择**: Network 请求存储在 `networkEntries: Record<string, NetworkEntry[]>` 中，key 为 `pageId`，per-page 隔离。最大保留 200 条 per-page。

**界面元素**:
- 列表：`[Method] [URL(截断)] [Status] [Timing] [Type]`
- Status 颜色：2xx 绿色、3xx 蓝色、4xx 橙色、5xx 红色、error 红色
- Timing 颜色：< 200ms 绿色、200-1000ms 橙色、> 1000ms 红色
- 清空按钮
- 自动滚动到底部

### D5: 面板激活方式

**选择**: Console 和 Network Tab 默认不显示 badge。当有 console.error 时 Console Tab 显示红色 error 计数 badge。

**Error 计数来源**: 不单独存储 `consoleErrorCount`，在 `DevToolsPanel` 组件中通过 `useMemo(() => entries.filter(e => e.level === 'error').length, [entries])` 从当前页面的 `consoleEntries[activePageId]` 派生。

**Badge 显示语义（方案 A：简单模式）**:
- Console Tab **非激活** 且 errorCount > 0 时显示红色 badge
- Console Tab **激活** 时隐藏 badge（用户已在看 console）
- 切回 Agent/Network Tab 后，若仍有 error，badge **重新出现**
- 这是 total count 模式，不是 unread 模式——不需要额外的 `lastSeenIndex` 状态

**理由**: 简单模式避免了 unread 语义的复杂性（需要额外 per-page 的 seen 状态），且 error 本身就值得持续提示。用户清空 console 后 errorCount 归零、badge 消失。

## Risks / Trade-offs

### R1: Console 参数截断
- **风险**: BROWSER_INIT_SCRIPT Layer 1 将参数截断到 10 个字符串，复杂对象显示为 `[object Object]`
- **缓解**: MVP 阶段可接受。后续可增强 Layer 1 使用 `JSON.stringify` 序列化（需注意循环引用）

### R2: Network 仅覆盖 fetch/XHR
- **风险**: `<img>`、`<script>`、`<link>` 等子资源请求不可见，用户可能误以为只有这些请求
- **缓解**: Network Tab 标题显示「Fetch/XHR」标注覆盖范围；后续可通过代理审计或 PerformanceObserver 增强

### R3: 大量 console/network 消息的性能
- **风险**: SPA 页面可能产生大量 console 和 network 消息，FIFO 上限和频繁 setState 可能影响性能
- **缓解**: Console 和 Network 均使用**模块级变量** buffer + rAF 节流批量更新（参考规则 #25 但适配 store 模块上下文）；ConsolePanel 使用虚拟列表渲染（`react-window` 或自定义 viewport 虚拟化）

### R4: 页面关闭后条目泄漏
- **风险**: 关闭标签页后 `consoleEntries[pageId]` 和 `networkEntries[pageId]` 不清理会导致内存增长
- **缓解**: 在 `browser-page-closed` 事件处理中 `delete consoleEntries[pageId]` 和 `delete networkEntries[pageId]`
