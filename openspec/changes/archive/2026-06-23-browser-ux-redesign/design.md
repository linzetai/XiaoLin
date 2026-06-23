## Context

XiaoLin 内置浏览器（built-in-browser 变更）已完成 Tier 1 功能实现，包括多标签 WebView、全宽布局模式、Cookie 持久化、Agent 操作可视化等。当前全宽模式的布局为 `ChatSidePanel(左) → BrowserFullPanel(中) → WorkspacePanel(右)`，Chat 在左侧将浏览器挤到中间位置。导航体验方面，仅有地址栏 reload 按钮旋转和标签页小 spinner，内容区域无任何加载反馈。

关键约束：
- WebView 是 OS 级原生视图，不受 CSS 控制，通过 `set_position/set_size` 命令式定位
- Tauri `on_page_load` 只提供 `Started/Finished` 两个事件，无真实加载进度百分比
- 进度条必须叠加在 WebView 占位 div 上（不是在 WebView 内），因为进度条是 React 组件
- `ContentBlock.tsx` 控制全宽/面板模式的整体布局

## Goals / Non-Goals

**Goals:**
- 全宽模式下浏览器成为绝对主体，Chat 在右侧辅助
- 页面导航时有明确的视觉反馈（进度条），与主流浏览器体验一致
- 后退/前进/刷新操作有即时反馈
- 浏览器 chrome 结构与主流浏览器对齐（标签在上、地址栏在下）

**Non-Goals:**
- 不重新设计面板（Panel）模式的布局——面板模式保持现状
- 不修改 WebView 的创建/销毁/定位逻辑——复用现有 ResizeObserver + IPC 同步机制
- 不引入真实的页面加载进度（需要 WebView 内部 hook，复杂度高）
- 不增加 Tauri IPC 命令——复用现有命令集

## Decisions

### D1: 全宽模式布局顺序——浏览器在左、Chat 在右

**选择**: `ContentBlock` 全宽模式渲染顺序从 `<ChatSidePanel> → <BrowserFullPanel>` 翻转为 `<BrowserFullPanel> → <ChatSidePanel>`

**替代方案**:
- 保持 Chat 在左：不符合浏览器为主体的心智模型，浏览器被夹在中间
- Chat 以 Overlay 覆盖在浏览器上：遮挡网页内容，影响浏览
- Chat 作为底部面板：垂直空间对聊天界面不友好

**理由**: 从左到右的阅读顺序中，浏览器应该是第一视觉焦点。Chat 在右侧类似 Chrome DevTools 的布局，用户已有成熟的心智模型。`ChatSidePanel` 的 `borderRight` 改为 `borderLeft`，拖拽手柄从右侧移到左侧。

### D2: Chat 折叠态——0px 隐藏 + 浮动 Toggle

**选择**: Chat 折叠时宽度为 0px（完全不占空间），通过地址栏右端的 Chat toggle 按钮（💬 图标）展开

**替代方案**:
- 保持 48px 折叠态（当前）：占用空间但无实际用途，只显示一个图标
- 右侧固定 toggle 栏（类似 VS Code Activity Bar）：额外增加一栏，过度设计

**理由**: 48px 的折叠态既不能显示 Chat 内容，又占用浏览器宽度。地址栏的 toggle 按钮位置直觉（用户在操作浏览器时自然会看到地址栏），不需要额外的视觉元素。未读消息通过 badge 在 toggle 按钮上显示。

**Children 挂载策略**: 折叠态改为 `width: 0; overflow: hidden` 但 **children（MessageStream 等）保持挂载**，不使用条件渲染的独立 return 分支。这样折叠时流式消息、scroll 位置、composer 状态保持，展开后无需重挂载。与 built-in-browser D10「Agent 交互不中断」一致。

**选区填 Chat 联动**: 当全宽模式 + Chat 折叠时，`fillChatFromBrowserSelection` 需自动调用 `toggleChatPanel()` 展开 Chat，确保用户选中文本点「询问」后能看到 Chat 输入框。

### D3: 进度条——纯 CSS 动画的模拟进度

**选择**: 在 `BrowserPlaceholder` 顶部叠加一个 2px 高的进度条，使用 CSS animation 模拟加载进度

**进度动画时间轴**:
1. `loading: true` → 进度条出现，0% → 30%（200ms ease-out），30% → 60%（2s ease-out），60% → 85%（8s linear），停在 85% 等待
2. `loading: false` → 85% → 100%（200ms ease-out），然后 opacity fade out（150ms）
3. 超过 15s 仍在 loading → 进度条回退到 trickling 模式（85% 附近微小抖动），告诉用户仍在加载

**替代方案**:
- 使用 NProgress.js 库：引入额外依赖，且 NProgress 是全局的（叠加在页面最顶部），不适合叠加在特定区域
- 在 WebView 内注入进度条 JS：进度条成为网页内容的一部分，可能被网页 CSS 影响
- 使用 Skeleton/Shimmer 全屏覆盖：会遮挡旧页面内容（浏览器的标准行为是旧页面保持可见）

**理由**: 纯 CSS 动画零依赖、性能好。进度条在 React 层渲染（`BrowserPlaceholder` 内的 absolute 定位元素），不影响 OS 级 WebView。时间轴参考 Chrome 的行为模式。

**实现要点**:
- 进度条的 `z-index` 必须高于 WebView 占位 div 但不影响用户与 WebView 的交互（`pointer-events: none`）
- 实际上进度条在 React 的占位 div 内，而 WebView 是 OS 级视图叠加在其上方——所以进度条需要通过 `position: absolute; top: 0` 且足够窄（2px），在 WebView 顶部边缘微微露出。由于 WebView 精确覆盖占位 div，进度条可能被遮挡
- **解决方案**: 在占位 div 的上方（BrowserTabContent 内、占位 div 之前）放置进度条，这样进度条在浏览器 chrome 和 WebView 之间，不会被 WebView 遮挡

### D4: 地址栏 Stop/Reload 切换

**选择**: loading 状态时将 ↻ ArrowClockwise 图标替换为 ✕ X 图标，点击执行 `window.stop()`

**实现**: 在 `BrowserAddressBar.tsx` 中根据 `isLoading` 条件渲染：
- `isLoading`: 显示 X 图标 + `onClick: browserStopLoading(pageId)` → 内部调用 `browser_eval_js(pageId, "window.stop()")`
- `!isLoading`: 显示 ArrowClockwise + `onClick: browserReload(pageId)`
- 去掉当前的 `animation: browser-spin` 旋转效果（用 X 图标替代了旋转的视觉反馈）

**Stop 兜底**: `browserStopLoading` 在 eval 后**乐观**设 `loadState: "ready"`。若 500ms 内又收到 `browser-loading` Started 事件则再切回 loading。这样即使 `window.stop()` 无效或 WebKit 不 emit Finished 事件，前端也不会卡在 loading 状态。

**理由**: 这是所有主流浏览器的标准行为，用户期望 loading 时能停止加载。

### D5: 后退/前进乐观 Loading 状态

**选择**: `browserGoBack` / `browserGoForward` 调用前立即将当前页面的 `loadState` 设为 `{ state: "loading" }`

**替代方案**:
- 只依赖后端 `on_page_load` 事件：有网络延迟，用户点击后几百毫秒没有反馈
- 在后端 `browser_go_back` 命令中主动发射 loading 事件：增加 IPC 调用复杂度

**理由**: 乐观更新让进度条和 spinner 在用户点击的瞬间就响应。

**超时策略**:
- 乐观设置 loading 后，启动 5s 超时定时器（per-page，存储在 `Map<pageId, timeoutId>` 中）
- 收到**任意** `browser-loading` 事件（含 `Started`）时，立即取消超时定时器（因为后端已接管状态）
- 仅在「5s 内无任何后端 loading 事件」时恢复 `ready`——对应 `history.back()` 在栈顶/栈底时的 noop
- 快速连续操作（连点后退）时，新的乐观操作 clear 旧 timeout 再设新 timeout
- 超时恢复时，若当前 `loadState` 已经不是 `loading`（被后端事件覆盖），跳过恢复

### D6: 浏览器 Chrome 顺序——标签在上、地址栏在下

**选择**: `BrowserTabContent` 内将 `BrowserAddressBar` 和 `BrowserPageTabs` 的渲染顺序交换

**当前**: AddressBar → PageTabs → Placeholder
**目标**: PageTabs → AddressBar → ProgressBar → Placeholder

**理由**: Chrome、Firefox、Safari、Edge 无一例外都是标签在上、地址栏在下。用户对这个布局有深度肌肉记忆。

### D7: WorkspacePanel 全宽模式处理

**选择**: 全宽浏览器模式下，WorkspacePanel 暂时不显示在同一行。用户需要切回 Panel 模式才能使用 WorkspacePanel。

**替代方案**:
- WorkspacePanel 以 Overlay 形式滑出：实现复杂（需要重新设计 WorkspacePanel 的 position 策略），且与 OS 级 WebView 的 z-index 冲突
- 保持三栏并列（当前）：浏览器被挤压，体验差

**理由**: 全宽模式的核心诉求是最大化浏览器面积。WorkspacePanel overlay 可以作为后续优化，当前先保证浏览器主体体验。全宽模式下 WorkspacePanel 的 Tab（如 Terminal、Files）仍可通过快捷键切回 Panel 模式使用。

### D8: Chat toggle 按钮位置

**选择**: Chat toggle 按钮整合到地址栏最右端（与 Network settings 和原全宽切换按钮同行）

**按钮布局**: `← → [↻/✕] [🔒 URL ] [🌐] [💬] [⇲]`
- 🌐 Network settings
- 💬 Chat toggle（带未读 badge）
- ⇲ 全宽/面板切换（保留）

**替代方案**:
- 右下角浮动按钮：遮挡网页内容
- 窗口标题栏按钮：Tauri 自定义标题栏实现复杂
- BrowserPageTabs 行末按钮：标签栏已经比较拥挤

**理由**: 地址栏是浏览器 chrome 中用户最频繁交互的区域，把 Chat toggle 放在这里保证了可发现性和可及性。

### D9: 标签页 Favicon 获取与显示

**选择**: 页面加载完成后通过 `on_page_load Finished` 内的 eval JS 提取 `<link rel="icon">` 的 `href`，将 URL 通过 `browser-favicon-changed` 事件通知前端。前端在 `BrowserPageTabs` 中用 `<img>` 显示 favicon，加载失败时 fallback 到 Globe 图标。

**Favicon 提取 JS**（通过 eval 在 browser WebView 中执行，将 favicon 转为 data URL 回传）:
```javascript
(function(){
  var el = document.querySelector('link[rel*="icon"]');
  var url = el ? el.href : (location.origin + '/favicon.ico');
  var img = new Image();
  img.crossOrigin = 'anonymous';
  img.onload = function(){
    var c = document.createElement('canvas');
    c.width = 16; c.height = 16;
    c.getContext('2d').drawImage(img, 0, 0, 16, 16);
    try {
      __XIAOLIN__.notify('favicon', { dataUrl: c.toDataURL('image/png') });
    } catch(e) {
      __XIAOLIN__.notify('favicon', { url: url });
    }
  };
  img.onerror = function(){ __XIAOLIN__.notify('favicon', { url: url }); };
  img.src = url;
})()
```

**替代方案**:
- Google S2 API（`https://www.google.com/s2/favicons?domain=xxx`）：依赖外部服务，隐私问题
- 前端主 WebView 直接 `<img src={url}>`：主 WebView 不走 browser WebView 的代理/cookie，需登录的 favicon 会 403
- 后端 Rust reqwest 下载：增加后端复杂度

**理由**: 在 browser WebView 内 eval JS，利用 WebView 自身的 cookie/代理上下文获取 favicon，通过 canvas 转为 data URL 后回传。data URL 可直接作为 `<img src>` 使用，不受跨域限制。canvas 转换失败时 fallback 到原始 URL（公开 favicon 无跨域问题）。

**后端改动**:
- `browser_panel.rs`：`ALLOWED_INTERNAL_MESSAGE_TYPES` 新增 `"favicon"`
- `browser_panel.rs` / `commands/browser.rs`：**专用** match arm 处理 `"favicon"` 类型，emit 扁平结构（不复用 generic wrapper）：
  ```json
  {
    "pageId": "<page_id>",
    "dataUrl": "<base64 png 或 null>",
    "url": "<fallback url 或 null>"
  }
  ```
  前端 listener 读取路径：`ev.payload.dataUrl ?? ev.payload.url`。与 console/network 的嵌套 `{ pageId, type, data }` 结构不同，避免 implementer 套用 generic wrapper 导致字段访问路径错误

**存储**: `BrowserPage` 接口新增 `faviconUrl?: string` 字段，emit `browser-favicon-changed` 事件更新 store。

### D10: 浏览器标准键盘快捷键

**选择**: 在 `BrowserTabContent` 的 `useEffect` 中注册 `window` 级 `keydown` 监听。**作用域**：不使用 DOM `contains` 检查（因为 child WebView 是 OS 原生视图，用户在网页内操作时 `document.activeElement` 不落在 BrowserTabContent DOM 内）。改用状态判定：

```
const isBrowserVisible =
  layoutMode === 'fullwidth' ||
  (browserPanelOpen && activeWorkspaceTab === 'browser');
const isEditableFocused = ['INPUT', 'TEXTAREA', 'SELECT'].includes(
  document.activeElement?.tagName ?? ''
) || document.activeElement?.isContentEditable;

if (!isBrowserVisible || isEditableFocused) return;
```

无修饰键的快捷键（F5、Escape）额外检查 `isEditableFocused`，避免在 Chat Composer 中拦截：

| 快捷键 | 动作 | 已有 |
|--------|------|------|
| Ctrl+Tab | 下一个标签页 | ❌ 新增 |
| Ctrl+Shift+Tab | 上一个标签页 | ❌ 新增 |
| Ctrl+1~8 | 跳转到第 N 个标签页 | ❌ 新增 |
| Ctrl+9 | 跳转到最后一个标签页 | ❌ 新增 |
| F5 / Ctrl+R | 刷新当前页面 | ❌ 新增（复用 `browserReload`） |
| Escape | 停止加载 | ❌ 新增（复用 `browserStopLoading`，需 D4） |
| Ctrl+T | 新建标签页 | ✅ 已有 |
| Ctrl+W | 关闭当前标签页 | ✅ 已有 |
| Ctrl+L | 聚焦地址栏 | ✅ 已有 |
| Ctrl+Shift+F | 切换全宽/面板模式 | ✅ 已有 |

**标签页切换逻辑**: Ctrl+Tab 从当前活动标签向后循环（到末尾跳回第一个），Ctrl+Shift+Tab 反向。Ctrl+1~8 按页面列表的索引顺序（非创建顺序），Ctrl+9 固定跳到最后一个。

**理由**: 这些是所有主流浏览器的标准快捷键，用户有深度肌肉记忆。不实现会让用户频繁感到「怎么这个也不行」的挫败感。

## Risks / Trade-offs

### R1: 进度条被 WebView 遮挡
- **风险**: WebView 是 OS 级视图，渲染在所有 React 内容之上。放在 `BrowserPlaceholder` 内的进度条可能被 WebView 完全遮挡
- **缓解**: 将进度条放在 `BrowserTabContent` 中、WebView 占位 div 之前渲染。进度条是浏览器 chrome 的一部分，位于标签栏/地址栏和 WebView 区域之间。这样进度条完全在 React 层，不与 WebView 重叠

### R2: 后退/前进乐观 Loading 的误触发
- **风险**: 用户在第一页点后退，`history.back()` 不会触发导航，但前端已经设置了 loading 状态
- **缓解**: 设置 5s 超时——如果 5s 内没有收到后端 `browser-loading` 事件，自动恢复 `loadState: "ready"`。收到任何后端事件后立即取消该超时计时器

### R3: Chat 右移后的拖拽方向反转
- **风险**: 拖拽手柄从右侧移到左侧，拖拽方向语义变为「向左拖 = 面板变宽，向右拖 = 面板变窄」
- **缓解**: 调整 `handleResizeStart` 的 delta 计算：`startWidth - delta`（当前是 `startWidth + delta`）

### R4: 全宽模式下无 WorkspacePanel
- **风险**: 用户习惯在全宽浏览时查看 Terminal 或 Files
- **缓解**: 快捷键可快速切换回 Panel 模式。后续迭代可增加 WorkspacePanel overlay 模式

### R5: 模式切换时 WebView 闪烁
- **风险**: 布局翻转（Chat 从左到右）触发 ResizeObserver，WebView 重新定位时可能闪烁
- **缓解**: 复用现有 `layoutTransitioning` 机制——切换期间 WebView 移到 -9999，400ms 动画后恢复。与原始 D10 的快照过渡（未实现）不冲突，后续可叠加

### R6: Favicon 加载失败、不存在或 SPA 不更新
- **风险**: 部分网站无 favicon，canvas 转换因 CORS 失败，或 SPA 软导航后 favicon 不更新（`on_page_load Finished` 不再触发）
- **缓解**:
  - 加载失败：`<img>` 的 `onError` fallback 到 Globe 图标；限制图标显示尺寸 14×14
  - canvas CORS 失败：fallback 到原始 URL（公开 favicon 大多无 CORS 问题）
  - SPA 软导航：`browser-url-changed` 仅在后端 `on_navigation` 触发时 emit，SPA 的 `history.pushState/replaceState` **不触发 `on_navigation`**，因此 MVP 阶段 SPA 内部软导航后 favicon 不自动更新。后续可通过在 `BROWSER_INIT_SCRIPT` 中 hook `pushState/replaceState/popstate` 并 notify 解决，但这属于增强 init script 范畴（Non-Goal）。当前 fallback：SPA 页面的 favicon 通常不变（同一 host 同一 favicon），影响有限

### R7: 键盘快捷键与系统/Tauri 冲突
- **风险**: F5 可能被 Tauri 或系统拦截；Ctrl+数字可能与其他功能冲突
- **缓解**: 快捷键监听使用 `window` 级 `keydown`，通过 `isBrowserVisible`（布局模式 + 面板状态）+ `isEditableFocused`（INPUT/TEXTAREA/SELECT/contentEditable）判定是否响应（与 D10 一致）。F5/Escape 在编辑区域聚焦时不拦截
