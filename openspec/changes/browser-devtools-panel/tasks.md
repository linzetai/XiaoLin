## 1. 数据层——事件监听与存储

- [ ] 1.1 `browser-store.ts`：新增接口定义：
  ```typescript
  interface ConsoleEntry {
    level: 'log' | 'warn' | 'error' | 'info' | 'debug';
    args: string[];
    ts: number; // 前端 Date.now()，不用 payload.ts
  }
  interface NetworkEntry {
    type: 'fetch' | 'xhr';
    method: string;
    url: string;
    status: number;
    timing: number;
    error?: string;
    ts: number; // 前端 Date.now()
  }
  ```
- [ ] 1.2 `browser-store.ts`：BrowserState 新增字段（含默认值）：
  - `consoleEntries: Record<string, ConsoleEntry[]>` (默认 `{}`)
  - `networkEntries: Record<string, NetworkEntry[]>` (默认 `{}`)
  - `devtoolsActiveTab: 'agent' | 'console' | 'network'` (默认 `'agent'`)
  - `devtoolsPanelHeight: number` (默认 `200`)
  - `devtoolsPanelCollapsed: boolean` (默认 `false`)
  `consoleErrorCount` 不单独存储，在组件中通过 `useMemo` 从 `consoleEntries[activePageId]` 派生。**Zustand selector 注意**：禁止 `s.consoleEntries[activePageId] ?? []` 在 selector 内（每次返回新 `[]` 引用），应在组件体内用 `const entries = useStore(s => s.consoleEntries[activePageId]); const safeEntries = entries ?? emptyArray;`（`emptyArray` 为模块级 `const emptyArray: ConsoleEntry[] = []`）
- [ ] 1.3 `browser-store.ts`：`initBrowserEvents()` 新增 `listen("browser-console", ...)` 监听。**payload 结构**：`ev.payload = { pageId, type: "console", data: { level, args }, ts? }`——字段在 `payload.data` 内，非顶层。解析时 `const { pageId, data } = ev.payload`，构造 `ConsoleEntry`（`ts: Date.now()`）后 **append 到 `consoleBuffer`**（含 pageId），**不直接** push 到 store state。由 task 1.5 的 rAF 机制统一 flush 到 `consoleEntries[pageId]`
- [ ] 1.4 `browser-store.ts`：`initBrowserEvents()` 新增 `listen("browser-network", ...)` 监听。**payload 结构**：`ev.payload = { pageId, type: "network", data: { type, method, url, status, timing, error? }, ts? }`——同上从 `payload.data` 解析。与 Console 类似，使用模块级 `networkBuffer: { pageId: string; entry: NetworkEntry }[]` + rAF 批量 flush 到 `networkEntries[pageId]`（200 条上限 FIFO）
- [ ] 1.5 `browser-store.ts`：Console 消息使用**模块级变量**做 rAF 节流批量 flush 到 state。buffer 类型为 `{ pageId: string; entry: ConsoleEntry }[]`（携带 pageId 以支持多标签页同时产生消息）。rAF 回调中按 pageId 分组后批量 push 到各自的 `consoleEntries[pageId]`，同时执行 500 条 FIFO 淘汰。禁止在 store 模块中使用 React Hooks（`useRef` 等）
- [ ] 1.6 `browser-store.ts`：新增 actions：`clearConsole(pageId)`、`clearNetwork(pageId)`、`setDevtoolsActiveTab(tab)`、`setDevtoolsPanelHeight(height)`、`toggleDevtoolsPanel()`

## 2. DevToolsPanel 容器组件

- [ ] 2.1 新增 `components/browser/DevToolsPanel.tsx`：底部面板容器，包含 Tab 栏（Agent/Console/Network）+ 内容区
- [ ] 2.2 Tab 栏：28px 高，Tab 切换更新 `devtoolsActiveTab`
- [ ] 2.3 Console Tab badge：`useMemo` 派生 errorCount = `entries.filter(e => e.level === 'error').length`。Console Tab 非激活 && errorCount > 0 时显示红色 badge；激活时隐藏；切回其他 Tab 时若有 error 则重新显示（total count 模式）
- [ ] 2.4 面板高度拖拽：顶部 4px 拖拽条，范围 100px ~ 50vh，存储到 `devtoolsPanelHeight`
- [ ] 2.5 面板折叠：双击拖拽条或折叠按钮，折叠到仅 Tab 栏（28px），再次操作恢复

## 3. Console 面板

- [ ] 3.1 新增 `components/browser/ConsolePanel.tsx`：Console 消息列表，使用虚拟列表渲染（如 `react-window` 或自定义 viewport 虚拟化），自动滚动到底部
- [ ] 3.2 级别过滤器：`[All] [Errors] [Warnings] [Info] [Debug]` 按钮组
- [ ] 3.3 消息样式：error 红色背景、warn 黄色背景、info/log/debug 默认样式
- [ ] 3.4 时间戳显示：`HH:MM:SS.ms` 格式
- [ ] 3.5 清空按钮：调用 `clearConsole(activePageId)`
- [ ] 3.6 自动滚动：新消息时自动滚动到底部，用户手动上滚时中断自动滚动

## 4. Network 面板

- [ ] 4.1 新增 `components/browser/NetworkPanel.tsx`：网络请求列表
- [ ] 4.2 列表列：Method、URL（截断）、Status、Timing、Type（fetch/xhr）
- [ ] 4.3 状态码颜色：2xx 绿色、3xx 蓝色、4xx 橙色、5xx 红色、`status === 0` 或 `error` 字段非空时整行红色高亮
- [ ] 4.4 耗时颜色：< 200ms 绿色、200-1000ms 橙色、> 1000ms 红色
- [ ] 4.5 「Fetch / XHR」覆盖范围标注
- [ ] 4.6 清空按钮：调用 `clearNetwork(activePageId)`
- [ ] 4.7 自动滚动：与 Console 一致——新请求时自动滚动到底部，用户手动上滚时中断

## 5. BrowserTabContent 整合

- [ ] 5.1 `BrowserTabContent.tsx`：用 `DevToolsPanel` 替换独立的 `AgentOperationLog`
- [ ] 5.2 重构 `AgentOperationLog`：移除其独立折叠头（「Agent 操作 (N) ▼」）和 160px 高度限制，仅保留操作列表 + 清空按钮作为 DevToolsPanel Agent Tab 的内容。空态时显示「暂无 Agent 操作」占位。避免 DevToolsPanel Tab 栏 + AgentOperationLog 自带折叠头的 UI 重复
- [ ] 5.3 `browser-store.ts`：在 `browser-page-closed` 事件处理中清理已关闭页面的 `consoleEntries[pageId]` 和 `networkEntries[pageId]`，同时过滤 `consoleBuffer` 中该 pageId 的 pending 条目（防止下一帧 rAF flush 把已关闭页的消息写回 state）

## 6. 验证

- [ ] 6.1 验证 Console 面板正确显示 console.log/warn/error 消息
- [ ] 6.2 验证 Console 级别过滤正常工作
- [ ] 6.3 验证 Network 面板正确显示 fetch/XHR 请求
- [ ] 6.4 验证状态码和耗时的颜色编码正确
- [ ] 6.5 验证 Tab 切换、面板折叠/拖拽调整高度正常
- [ ] 6.6 验证 per-page 隔离——切换标签页后只显示当前页面的消息
- [ ] 6.7 验证 Agent Tab 保持与原 AgentOperationLog 相同列表功能（无独立折叠头）
- [ ] 6.8 验证大量消息时的性能（节流更新 + FIFO 上限）
- [ ] 6.9 验证 Error badge 三态：Console Tab 非激活时显示、激活时隐藏、清空 Console 后消失
- [ ] 6.10 验证关闭标签页后 consoleEntries/networkEntries 清理（无幽灵数据）
- [ ] 6.11 验证 Agent Tab 空态显示「暂无 Agent 操作」占位且 Tab 栏保持可见
- [ ] 6.12 确认后端 `ALLOWED_INTERNAL_MESSAGE_TYPES` 白名单含 `console`/`network`，两条 IPC 路径均能 emit 事件到前端
