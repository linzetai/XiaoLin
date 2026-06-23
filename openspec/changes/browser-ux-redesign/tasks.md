## 1. 进度条组件

- [ ] 1.1 新增 `components/browser/BrowserProgressBar.tsx`：2px 高的进度条，接收 `loading: boolean` prop，使用 CSS keyframes 实现 0→30%→60%→85% 的模拟进度动画
- [ ] 1.2 进度条完成动画：`loading` 从 true 变为 false 时，进度条 200ms 到 100% → 150ms fade out
- [ ] 1.3 进度条快速连续导航处理：重新进入 loading 时重置到 0% 重新开始
- [ ] 1.4 进度条样式：颜色使用 `var(--tint)`，`pointer-events: none`，`position: relative` 渲染在 BrowserTabContent 中
- [ ] 1.5 进度条 15s trickling 模式：超过 15s 仍在 loading 时，在 85% 附近做 ±2% 的微小来回抖动
- [ ] 1.6 进度条 `failed` 状态处理：`loadState` 变为 `"failed"` 时立即 150ms fade out（不播 100% → fade out 完成动画），与 spec `browser-progress-bar` 对齐

## 2. 浏览器 Chrome 结构调整

- [ ] 2.1 `BrowserTabContent.tsx`：交换 `BrowserAddressBar` 和 `BrowserPageTabs` 渲染顺序（标签在上、地址栏在下）
- [ ] 2.2 `BrowserTabContent.tsx`：在地址栏和占位 div 之间插入 `BrowserProgressBar` 组件
- [ ] 2.3 `BrowserAddressBar.tsx`：loading 时将 ↻ ArrowClockwise 替换为 ✕ X 图标，点击执行 `browserStopLoading(pageId)`，并更新 title/aria-label
- [ ] 2.4 `BrowserAddressBar.tsx`：移除 `browser-spin` 旋转动画（用 Stop 按钮替代视觉反馈）
- [ ] 2.5 `browser-store.ts`：新增 `browserStopLoading(pageId)` 函数——通过 `browserEvalJs`（task 9.5）调用 `window.stop()` 后乐观设 `loadState: "ready"`，若 500ms 内又收到 `browser-loading Started` 则切回 loading

## 3. 后退/前进即时反馈

- [ ] 3.1 `browser-store.ts`：修改 `browserGoBack` / `browserGoForward`，调用前乐观设置 `loadState: { state: "loading" }`
- [ ] 3.2 `browser-store.ts`：乐观 loading 超时机制——使用 `Map<pageId, timeoutId>` 存储 per-page 定时器；5s 内无后端 `browser-loading` 事件则恢复 `ready`；新的乐观操作先 clear 旧 timeout
- [ ] 3.3 `browser-store.ts`：在 `browser-loading` 事件 listener 中取消对应 pageId 的乐观超时定时器

## 4. 全宽布局翻转

- [ ] 4.1 `ContentBlock.tsx`：全宽模式渲染顺序从 `<ChatSidePanel> → <BrowserFullPanel>` 改为 `<BrowserFullPanel> → <ChatSidePanel>`
- [ ] 4.2 `ContentBlock.tsx`：全宽模式下移除 `{showPanel && <WorkspacePanel />}`（全宽不显示 WorkspacePanel）

## 5. Chat 面板右侧化 + 折叠重设计

- [ ] 5.1 `ChatSidePanel.tsx`：`borderRight` 改为 `borderLeft`，拖拽手柄从右侧移到左侧
- [ ] 5.2 `ChatSidePanel.tsx`：拖拽方向反转——delta 计算从 `startWidth + delta` 改为 `startWidth - delta`
- [ ] 5.3 `ChatSidePanel.tsx`：折叠态从条件 return 独立分支改为统一渲染——`width: 0; overflow: hidden` 但 children 保持挂载（不卸载 MessageStream）。同时移除旧 48px 窄条 UI（`ChatCircle` + 点击展开的折叠面板），折叠/展开改由地址栏 toggle 按钮（task 6.1）负责。全宽模式下 Chat header（标题栏 + 折叠按钮）保留，作为展开后的顶部操作区
- [ ] 5.4 `browser-store.ts`：将 `COLLAPSED_CHAT_PANEL_WIDTH` 从 48 改为 0
- [ ] 5.5 `ChatSidePanel.tsx`：折叠按钮图标从 `CaretLeft` 改为 `CaretRight`（Chat 在右侧时收起方向为右）

## 6. Chat Toggle 按钮与选区联动

- [ ] 6.1 `BrowserAddressBar.tsx`：全宽模式下在地址栏右端添加 Chat toggle 按钮（ChatCircle 图标）
- [ ] 6.2 Chat toggle 按钮的未读 badge：从 `chatMetaStore.unread` 读取，显示红色圆点或数字，保留 pulse 动画
- [ ] 6.3 Chat toggle 按钮仅在 `layoutMode === "fullwidth"` 时显示
- [ ] 6.4 `browser-store.ts` / `fillChatFromBrowserSelection`：全宽 + Chat 折叠时自动调用 `toggleChatPanel()` 展开 Chat

## 7. 标签页 Favicon

- [ ] 7.1 `browser-store.ts`：`BrowserPage` 接口新增 `faviconUrl?: string` 字段
- [ ] 7.2 `browser_panel.rs`：`ALLOWED_INTERNAL_MESSAGE_TYPES` 白名单新增 `"favicon"` 条目
- [ ] 7.3 `browser_panel.rs` / `commands/browser.rs`：**专用** match arm 处理 `"favicon"` 类型，emit 扁平 JSON `{ pageId, dataUrl?, url? }`（不复用 generic `{ pageId, type, data }` wrapper）。事件名 `"browser-favicon-changed"`
- [ ] 7.4 `commands/browser.rs`：`on_page_load Finished` 回调中新增 eval JS——在 browser WebView 内提取 favicon 并通过 canvas 转为 data URL 回传（CORS 失败时 fallback 到原始 URL）。JS 脚本提取为 `FAVICON_EXTRACT_JS` 常量（位于 Rust 侧），作为唯一来源
- [ ] 7.5 `browser-store.ts`：在 `initBrowserEvents()` 中监听 `browser-favicon-changed` 事件。读取路径：`ev.payload.dataUrl ?? ev.payload.url`（扁平结构，非嵌套 `data`），更新对应页面的 `faviconUrl`
- [ ] 7.6 `BrowserPageTabs.tsx`：将 Globe 图标替换为条件渲染——有 `faviconUrl` 时显示 `<img>`（14×14），加载失败 `onError` 回退到 Globe
- [ ] 7.7 `browser-store.ts`：`browser-url-changed` 事件处理中**仅清空** `faviconUrl`（显示 Globe 回退图标），**不** re-eval favicon JS。Favicon 提取仅在 task 7.4 的 `on_page_load Finished` 路径触发——避免导航中途 eval 旧 DOM 拿到错误 favicon。SPA `pushState/replaceState` 不触发 `on_navigation` 也不触发 `on_page_load`，因此 SPA 内部导航后 favicon 不更新（MVP 可接受，同 host 通常同 favicon）

## 8. 键盘快捷键补充

- [ ] 8.1 `BrowserTabContent.tsx`：`useEffect` 中注册 `window` 级 `keydown` 监听。**作用域**：通过 `isBrowserVisible`（复用 `shouldShowBrowserWebView()`）+ `isEditableFocused`（INPUT/TEXTAREA/SELECT/contentEditable）判定，不使用 DOM `contains`（child WebView 是 OS 原生视图）
- [ ] 8.2 Ctrl+Tab / Ctrl+Shift+Tab——循环切换标签页
- [ ] 8.3 Ctrl+1~8——跳转到第 N 个标签页，Ctrl+9 跳转到最后一个
- [ ] 8.4 F5 / Ctrl+R——调用 `browserReload`。F5 无修饰键，需确保焦点在 Chat 输入框时不拦截
- [ ] 8.5 Escape——loading 时调用 `browserStopLoading`。同上，焦点在非浏览器区域时不拦截

## 9. 样式与边缘场景

- [ ] 9.1 `ContentBlock.tsx`：全宽模式移除 WorkspacePanel 后调整 borderRadius 逻辑（右侧不再有 Panel 时应有右侧圆角）
- [ ] 9.2 `BrowserAddressBar.tsx`：为 Stop/Reload 按钮添加无障碍 `aria-label`（「停止加载」/「重新加载」）
- [ ] 9.3 `BrowserProgressBar.tsx`：进度条添加 `role="progressbar"` + `aria-busy` 属性
- [ ] 9.4 `ContentBlock.tsx`/`BrowserFullPanel.tsx`：全宽首次进入或 Chat 侧移触发布局变化时，复用 `layoutTransitioning` 机制——hide all WebView → 等待 ResizeObserver 稳定 → show active，防止 WebView 跳动/闪烁（对应 R5）
- [ ] 9.5 `browser-store.ts`：新增 `browserEvalJs(pageId: string, script: string)` 封装函数（invoke `browser_eval_js` 命令），供 Stop 按钮（`window.stop()`）和后续需要在 browser WebView 中执行 JS 的场景使用

## 10. 验证

- [ ] 10.1 验证全宽模式下浏览器位于左侧、Chat 位于右侧
- [ ] 10.2 验证进度条在页面导航时正确显示和消失（含 15s trickling）
- [ ] 10.3 验证 Stop 按钮能停止页面加载，前端不卡在 loading 状态
- [ ] 10.4 验证后退/前进按钮有即时反馈，慢速网络下不误恢复 ready
- [ ] 10.5 验证 Chat 折叠时完全隐藏、通过 toggle 按钮可展开
- [ ] 10.6 验证 Chat 折叠/展开后消息状态保持（不重挂载）
- [ ] 10.7 验证拖拽调整 Chat 宽度方向正确（向左拖=变宽）
- [ ] 10.8 验证选区发送 Chat 时自动展开折叠的 Chat 面板
- [ ] 10.9 验证 Panel 模式未受影响（布局、交互保持不变）
- [ ] 10.10 验证快捷键切回 Panel 模式后 WebView 状态保持
- [ ] 10.11 验证 Favicon 在页面加载后正确显示、导航时重置、加载失败回退 Globe
- [ ] 10.12 验证新增键盘快捷键功能正确：Ctrl+Tab 切换标签、Ctrl+1~8 跳转、F5 刷新、Escape 停止
- [ ] 10.13 验证快捷键作用域：Panel 模式 Files tab 时 Ctrl+Tab 不触发；Chat Composer 聚焦时 F5/Escape 不拦截；全宽模式浏览器可见时正常工作
