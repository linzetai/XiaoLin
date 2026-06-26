---
name: tauri-titlebar-debug
description: Debug Tauri v2 custom titlebar, frameless window, window controls, drag region, and capability permission issues. Use when minimize/maximize/close buttons do not respond, drag regions swallow clicks, CSS app-region/data-tauri-drag-region causes interaction bugs, or Tauri window control permissions need auditing.
---

# Tauri 2.0 自定义标题栏调试指南

当实现 `decorations: false` + 自定义标题栏时遇到拖拽或按钮不工作的问题，按此流程排查。

## 环境前提

- Tauri 2.0+ (WebKitGTK on Linux, WebView2 on Windows, WKWebView on macOS)
- `tauri.conf.json` → `app.windows[].decorations: false`

## 常见症状与根因

### 症状 1：窗口控制按钮（最小化/最大化/关闭）不响应

**根因：权限不足**

`core:default` 不包含窗口操作权限。必须在 `capabilities/default.json` 中显式添加：

```json
{
  "permissions": [
    "core:default",
    "core:window:allow-close",
    "core:window:allow-minimize",
    "core:window:allow-maximize",
    "core:window:allow-unmaximize",
    "core:window:allow-toggle-maximize",
    "core:window:allow-start-dragging",
    "core:window:allow-start-resize-dragging",
    "core:window:allow-set-focus"
  ]
}
```

**验证方法**：在浏览器 dev server 中测试同样的按钮。若浏览器正常但 Tauri WebView 不响应 → 权限问题。

### 症状 2：标题栏整体可拖拽但按钮被"吞掉"

**根因：`data-tauri-drag-region` 放在了按钮的父元素上**

Tauri 的 `data-tauri-drag-region` 在 IPC 层会拦截目标元素及其所有子元素的鼠标事件。

**解决方案（推荐）**：不使用 `data-tauri-drag-region`，改用 `startDragging()` API：

```tsx
async function onDragMouseDown(e: MouseEvent) {
  if (e.button !== 0) return;
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  getCurrentWindow().startDragging();
}

// 只在拖拽区域绑定，按钮是兄弟节点
<div className="drag-area" onMouseDown={onDragMouseDown}>Logo + Title</div>
<button onClick={minimize}>最小化</button>  {/* 兄弟节点，不受影响 */}
```

**注意**：`startDragging()` 需要 `core:window:allow-start-dragging` 权限。

### 症状 3：CSS `app-region: drag` 不工作

**根因：Tauri 官方已撤销此特性 (PR #9860)**

`-webkit-app-region: drag` 会阻止整个区域的所有交互（右键菜单、子元素点击均失效）。**不要使用 CSS app-region。**

### 症状 4：`isTauri` 检测失败，window controls 不显示

**根因：`withGlobalTauri: false` 时 `window.__TAURI__` 不存在**

```typescript
// ❌ 错误：withGlobalTauri: false 时不存在
const isTauri = "__TAURI__" in window;

// ✅ 正确：__TAURI_INTERNALS__ 始终存在
const isTauri = "__TAURI_INTERNALS__" in window || "__TAURI__" in window;
```

## 推荐的标题栏结构

```
<header>                           ← 容器，无拖拽属性
  <div onMouseDown={startDrag}>    ← 拖拽区域（flex-1 占满剩余空间）
    Logo + Title
  </div>
  <button>设置</button>             ← 兄弟节点，不受拖拽影响
  <button>最小化</button>
  <button>最大化</button>
  <button>关闭</button>
</header>
```

关键原则：**拖拽区域和按钮必须是兄弟节点，不能是父子关系。**

## 排查清单

1. [ ] `capabilities/default.json` 包含所有 `core:window:allow-*` 权限
2. [ ] 不使用 `data-tauri-drag-region` 属性（改用 `startDragging()` API）
3. [ ] 不使用 CSS `app-region: drag`（Tauri 已撤销此特性）
4. [ ] `isTauri` 使用 `__TAURI_INTERNALS__` 检测
5. [ ] 按钮与拖拽区域是兄弟节点关系
6. [ ] 浏览器 dev server 对照测试确认是 Tauri 特有还是通用 UI 问题

## 相关 Tauri Issues

- [#9901](https://github.com/tauri-apps/tauri/issues/9901) — child elements of drag region can't trigger events
- [PR #9860](https://github.com/tauri-apps/tauri/pull/9860) — reverted `app-region: drag` from `data-tauri-drag-region`
- [#11631](https://github.com/tauri-apps/tauri/issues/11631) — Linux titlebar buttons not working
