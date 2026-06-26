---
name: tauri-ui-debug
description: >-
  Debug and test XiaoLin Tauri app using MCP tools (screenshot, DOM snapshot, JS execution, interaction).
  Covers end-to-end testing, WebView debugging, performance analysis, and common failure troubleshooting.
  Use when debugging UI issues, running E2E tests, taking screenshots, inspecting DOM, or when Tauri MCP
  connection fails. Triggers: "调试 UI", "E2E 测试", "截图", "MCP 连接失败", "webview 超时".
---

# XiaoLin Tauri 调试与端到端测试

通过 Tauri MCP Bridge 直接操作运行中的 WebView，进行真实 UI 调试和端到端测试。

## 连接流程

```
1. 确保 dev server 运行:  cargo tauri dev
2. 连接 MCP Bridge:       driver_session start
3. 验证窗口可见:           manage_window info → visible: true
4. 验证 WebView 响应:      webview_execute_js "(() => document.title)()"
5. DOM 快照确认 UI 状态:    webview_dom_snapshot mode=ai
```

**关键检查点**：步骤 3 的 `visible: true` 是所有 webview 操作的前提。

## 启动前检查

启动 `cargo tauri dev` 前必须确认无残留进程，否则会因端口/单实例冲突导致新进程在 4-7 秒后自动退出：

```bash
pkill -f "xiaolin-app" 2>/dev/null; sleep 1
cargo tauri dev
```

症状：gateway ready + webview client connected 后秒退（exit code 1），无 panic 或错误日志。

## 故障排查决策树

```
webview 操作超时 (2000ms)
├─ manage_window info → visible: false?
│   ├─ 是 → 窗口被隐藏，WebKitGTK 暂停了 JS 执行
│   │   修复: 重启 dev server (kill → cargo tauri dev)
│   │   根因: on_window_event 中 CloseRequested 调用 window.hide()
│   └─ 否 (visible: true) → 检查前端是否加载
│       ├─ 检查 vite dev server (port 1430) 是否运行
│       ├─ 检查控制台错误: read_logs source=console
│       └─ 检查 gateway 是否就绪: 终端日志应有 "gateway ready"
├─ driver_session status → connected: false?
│   └─ 重新连接: driver_session start
├─ webview_act_and_verify → "Script execution timeout (eval path)"?
│   └─ 通常是 waitForSelector 使用了 :focus 伪类
│       修复: 改用 waitForText 或无伪类的选择器（见下方说明）
└─ ipc_get_backend_state 正常但 webview 不响应?
    └─ MCP Bridge JS 注入失败，重启 dev server
```

## React 控制组件交互（核心）

XiaoLin 前端使用 React 18 + 受控组件。MCP 的 `webview_interact` 和 `webview_keyboard`
在 WebKitGTK 上与 React 存在三个已知兼容性问题：

### 问题 1: MCP click 不设置焦点

`webview_interact action=click` 分发的合成 click 事件不触发 WebKitGTK 的原生焦点行为。
后续 `webview_keyboard` 会发送到 `body` 而非目标元素。

**解决方案**：用 `webview_execute_js` 调用 `.focus()`：

```javascript
webview_execute_js "(() => {
  document.querySelector('textarea.mention-textarea').focus();
  return document.activeElement?.tagName;
})()"
// → "TEXTAREA"
```

### 问题 2: webview_keyboard type 不触发 React onChange

`webview_keyboard action=type` 会更新 DOM 的 `.value`，但 React 受控组件使用内部
`_valueTracker` 机制跟踪变化。MCP keyboard 输入不走 React event delegation，因此
React 的 `onChange` 不被调用，内部状态保持空字符串。

**解决方案**：使用 Native Value Setter + _valueTracker 失效 + input 事件：

```javascript
webview_execute_js "(() => {
  const el = document.querySelector('textarea.mention-textarea');
  const nativeSetter = Object.getOwnPropertyDescriptor(
    window.HTMLTextAreaElement.prototype, 'value'
  ).set;
  nativeSetter.call(el, '你要输入的文本');
  // 失效 React 的值追踪器，使其检测到变化
  const tracker = el._valueTracker;
  if (tracker) tracker.setValue('');
  // 触发 React 的 onChange（React 监听 input 事件）
  el.dispatchEvent(new Event('input', { bubbles: true }));
  return el.value;
})()"
```

对于 `<input>` 元素，将 `HTMLTextAreaElement` 替换为 `HTMLInputElement`。

### 问题 3: 合成 KeyboardEvent 不触发 React onKeyDown

React 18 的 event delegation 不处理通过 `new KeyboardEvent(...)` 分发的事件。
即使 MCP keyboard `press Enter` 能到达目标元素，React 的 `onKeyDown` 也不会执行。

**解决方案**：直接点击提交按钮：

```
webview_interact action=click selector="button[title='发送 ↩']"
```

### 完整 E2E 发送消息流程（推荐模式）

```javascript
// 步骤 1: 设置文本（React-compatible）
webview_execute_js "(() => {
  const el = document.querySelector('textarea.mention-textarea');
  const nativeSetter = Object.getOwnPropertyDescriptor(
    window.HTMLTextAreaElement.prototype, 'value'
  ).set;
  nativeSetter.call(el, '要发送的消息');
  const tracker = el._valueTracker;
  if (tracker) tracker.setValue('');
  el.dispatchEvent(new Event('input', { bubbles: true }));
  return el.value;
})()"

// 步骤 2: 点击发送按钮
webview_interact action=click selector="button[title='发送 ↩']"

// 步骤 3: 等待消息出现
webview_wait_for text="要发送的消息" timeout=5000

// 步骤 4: 验证响应
webview_wait_for selector="[data-streaming='false']" timeout=60000
```

### webview_act_and_verify 使用注意

- **避免** `waitForSelector` 中使用 `:focus`、`:hover` 等伪类（因为 MCP click 不设焦点）
- **推荐** 使用 `waitForText` 验证操作结果
- **推荐** 使用无伪类的存在性选择器，如 `waitForSelector: "textarea.mention-textarea"`

## 前端 JS Helper（快捷操作）

前端在 `window` 上暴露了两个 helper 函数（定义在 `MessageStream.tsx`），可直接通过
`webview_execute_js` 调用，比操作 UI 按钮更可靠。

### 设置工作目录

```javascript
webview_execute_js "window.__xiaolin_setWorkDir('/home/user/my-project')"
// 返回: { chatId: "xxx", messageCount: 0 }

// 清除工作目录（恢复为"本地工作"）
webview_execute_js "window.__xiaolin_setWorkDir(null)"
```

直接设置当前活跃会话的工作目录，等同于用户通过"本地工作▾"按钮选择目录。
避免了原生文件对话框（Tauri dialog plugin）无法通过 MCP 交互的问题。

### 切换执行模式

```javascript
// 切换到 Goal 模式
webview_execute_js "window.__xiaolin_setMode('goal')"
// 返回: { chatId: "xxx", goalMode: true, executionMode: "agent" }

// 切换到 Plan 模式
webview_execute_js "window.__xiaolin_setMode('plan')"
// 返回: { chatId: "xxx", goalMode: false, executionMode: "plan" }

// 切换到 Agent 模式
webview_execute_js "window.__xiaolin_setMode('agent')"
// 返回: { chatId: "xxx", goalMode: false, executionMode: "agent" }
```

三种模式的区别：
- **Agent**: 默认模式，LLM 逐步执行工具调用
- **Plan**: 只读规划模式，LLM 只分析和规划，不执行写操作
- **Goal**: 自主目标模式，LLM 围绕目标自主迭代直到完成

注意：这两个 helper 只操作**当前活跃会话**。如果需要为特定会话设置，先通过侧边栏切换到
该会话。`__xiaolin_setMode` 是 async 函数（返回 Promise），因为需要通过 IPC 同步到后端。

### 完整 E2E 测试初始化流程（推荐）

```
1. driver_session start
2. manage_window info → 确认 visible: true
3. webview_execute_js "window.__xiaolin_setWorkDir('/target/dir')"
4. webview_execute_js "window.__xiaolin_setMode('goal')"  // 或 'plan'/'agent'
5. (设置审批模式 → 见下方 UI 按钮操作)
6. (输入文本 → 见"完整 E2E 发送消息流程")
7. (发送并等待结果)
```

### 审批模式切换（UI 按钮操作）

审批模式（建议修改/完全自动等）目前没有 JS helper，需要通过 UI 操作：

```javascript
// 1. 点击审批模式按钮打开下拉菜单
webview_execute_js "(() => {
  const btn = Array.from(document.querySelectorAll('button'))
    .find(b => ['建议修改','完全自动','自动编辑文件','仅规划']
      .includes(b.textContent?.trim()));
  if (btn) { btn.click(); return btn.textContent.trim(); }
})()"

// 2. 选择目标模式（例如"完全自动"）
webview_execute_js "(() => {
  const btn = Array.from(document.querySelectorAll('button'))
    .find(b => b.textContent?.includes('完全自动'));
  if (btn) { btn.click(); return 'selected'; }
})()"
```

四种审批模式：
| 模式 | 说明 |
|------|------|
| 建议修改 | 所有写操作需要用户确认 |
| 自动编辑文件 | 文件编辑自动通过，shell 命令仍需确认 |
| 完全自动 | YOLO 模式，所有操作自动通过 |
| 仅规划 | 只读，阻止所有写入（与 Plan 执行模式不同） |

## 端到端测试方法

### 测试交互功能

```
1. webview_dom_snapshot mode=ai               # 获取可交互元素 (ref IDs)
2. webview_interact action=click selector="..." # 点击目标元素
3. webview_dom_snapshot mode=ai               # 验证结果
```

### 测试表单输入（React 受控组件）

```
1. 使用 webview_execute_js 设置 input 值（见"完整 E2E 发送消息流程"）
2. 点击提交按钮: webview_interact action=click selector="button[type=submit]"
3. webview_wait_for text="期望的响应文本"    # 验证结果
```

### 测试组件渲染

```
1. webview_dom_snapshot mode=structure selector=".target-component"
2. webview_find_element selector=".expected-child"
3. webview_get_styles selector=".target-component" properties=["display","opacity"]
```

### 监听 IPC 事件

```
1. ipc_monitor action=start                    # 开始监听
2. (执行 UI 操作触发 IPC 调用)
3. ipc_get_captured                             # 获取捕获的调用
4. ipc_monitor action=stop
```

## 截图功能

`webview_screenshot` 需要前端加载 `@zumer/snapdom` 库。如果报错
"snapdom not available"，说明库未在前端代码中导入（仅在 package.json 中列出不够）。

替代方案（按优先级）：
1. `webview_dom_snapshot mode=ai` — 获取结构化 DOM 快照，对调试最有用
2. `webview_execute_js` — 检查具体元素状态
3. `webview_search_snapshot pattern="关键词"` — 搜索 DOM 快照中的内容

## WebView 调试技巧

### 检查前端状态

```javascript
// 获取 WebSocket 连接状态
webview_execute_js "(() => { const s = document.querySelector('[data-ws-status]'); return s?.dataset?.wsStatus || 'not found'; })()"

// 获取当前路由/视图
webview_execute_js "(() => window.location.hash || window.location.pathname)()"

// 检查 React 组件的内部状态
webview_execute_js "(() => {
  const el = document.querySelector('.target-element');
  const reactProps = Object.keys(el).find(k => k.startsWith('__reactProps$'));
  return reactProps ? el[reactProps] : null;
})()"
```

### CSS 调试

```
webview_get_styles selector=".problematic-element" properties=["display","visibility","opacity","z-index","overflow","position"]
```

### 元素选择器调试

```
# 通过文本内容查找
webview_find_element selector="Agent" strategy=text

# 通过 CSS 选择器
webview_find_element selector="button.primary-action"

# 通过无障碍树定位
webview_dom_snapshot mode=accessibility selector=".form-container"
```

## 性能分析

### 前端渲染性能

```javascript
webview_execute_js "(() => {
  const entries = performance.getEntriesByType('navigation');
  if (!entries.length) return 'no navigation entries';
  const n = entries[0];
  return JSON.stringify({
    domContentLoaded: Math.round(n.domContentLoadedEventEnd - n.startTime),
    loadComplete: Math.round(n.loadEventEnd - n.startTime),
    firstPaint: Math.round(performance.getEntriesByType('paint').find(p => p.name === 'first-paint')?.startTime || 0)
  });
})()"
```

### 内存使用

```javascript
webview_execute_js "(() => {
  if (!performance.memory) return 'memory API not available (WebKitGTK)';
  return JSON.stringify({
    usedHeap: Math.round(performance.memory.usedJSHeapSize / 1048576) + 'MB',
    totalHeap: Math.round(performance.memory.totalJSHeapSize / 1048576) + 'MB'
  });
})()"
```

## 平台注意事项

### Linux (WebKitGTK)

- **Hidden 窗口 JS 暂停**：`visible: false` 时 WebView 不执行 JS，所有 MCP webview 操作超时
- **Click 不设焦点**：`webview_interact click` 不触发原生 focus，必须用 JS `.focus()`
- **React 受控组件**：需要 native setter + _valueTracker 失效模式（见上方详细说明）
- **截图**：需要前端 import `@zumer/snapdom`；否则用 DOM snapshot 替代
- **Wayland**：`xdotool` 不可用，不要用它发送按键；使用 MCP 工具代替
- **全局快捷键**：注册可能在某些 Wayland compositors 上失败
- **残留进程**：启动前必须 `pkill -f "xiaolin-app"` 杀死旧实例，否则新实例会秒退

### macOS (WKWebView)

- 透明窗口需要 `macOSPrivateApi: true`
- 窗口阴影需要手动设置 `window.set_shadow(true)`

## 禁止的做法

- ❌ 写 Python/Node.js 脚本直连 WebSocket 进行"E2E 测试"
- ❌ 发现 MCP 连接失败就切换到脚本方案
- ❌ 用 `xdotool` / `ydotool` 代替 MCP webview 交互工具
- ❌ 用 `curl` 直接调 HTTP API 声称是端到端测试
- ❌ 用 `webview_keyboard type` 直接输入 React 受控组件（值不会同步到 React 状态）
- ❌ 用 `webview_act_and_verify` 的 `waitForSelector` 配合 `:focus` 伪类
- ❌ 用合成 KeyboardEvent 触发 React 的 onKeyDown 处理器（如 Enter 提交表单）
