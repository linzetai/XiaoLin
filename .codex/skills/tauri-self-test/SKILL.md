---
name: tauri-self-test
description: >-
  Test a Tauri v2 project by integrating tauri-plugin-connector, starting the app,
  connecting via MCP WebSocket, and verifying UI through DOM snapshots and JS execution.
  Use when: building a Tauri project and need to verify it works, user asks to "测试这个 Tauri 应用",
  "自测", "验证 UI", "MCP 测试", or Goal mode plan includes a verification step for a Tauri project.
  Triggers: "自测 Tauri", "验证 Tauri 应用", "测试构建结果", "MCP 验证".
---

# Tauri 项目自测指南

通过 `tauri-plugin-connector` + `manage_mcp_server` 实现 Tauri 应用的端到端自测。

## 前提条件

- 目标项目是 Tauri v2 应用（存在 `src-tauri/` 目录）
- 已安装 Rust 工具链和 Node.js
- 系统有可用的 WebKitGTK（Linux）或 WebView2（Windows）

## 完整自测流程

### Step 1: 集成 tauri-plugin-connector

在目标项目中添加 MCP Bridge 支持。

**1.1 添加 Rust 依赖**

在 `src-tauri/Cargo.toml` 的 `[dependencies]` 中添加：

```toml
tauri-plugin-connector = "0.11"
```

**1.2 注册插件**

在 `src-tauri/src/lib.rs` 的 `tauri::Builder::default()` 链中添加：

```rust
.plugin(tauri_plugin_connector::init())
```

放在其他 `.plugin(...)` 调用之后、`.invoke_handler(...)` 之前。

**1.3 添加权限**

在 `src-tauri/capabilities/default.json` 的 `permissions` 数组中添加：

```json
"connector:default"
```

### Step 2: 启动应用

```bash
# 清理残留进程（避免端口冲突）
pkill -f "目标二进制名" 2>/dev/null
sleep 1

# 启动 dev server
cd /path/to/project
cargo tauri dev
```

等待日志出现：
- `Finished` — Rust 编译完成
- `Running` — 应用已启动
- 等待约 3 秒让 MCP Bridge 初始化

### Step 3: 连接 MCP

使用内置 `manage_mcp_server` 工具：

```
manage_mcp_server action=add id=tauri-test transport=websocket url=ws://127.0.0.1:9555
```

如果 9555 端口被占用，Bridge 会自动尝试 9556-9655。连接失败时检查：
- 应用是否正在运行
- 端口是否被防火墙阻挡
- 尝试端口 9556、9557 等

### Step 4: 验证 UI

连接成功后，使用以下 MCP 工具验证：

**4.1 DOM 快照 — 确认 UI 已加载**

```
mcp__tauri-test__webview_dom_snapshot mode=ai
```

检查返回的 DOM 树中是否包含预期的 UI 元素。

**4.2 JS 执行 — 验证数据和交互**

```
mcp__tauri-test__webview_execute_js script="document.title"
```

**4.3 元素查找 — 验证特定组件**

```
mcp__tauri-test__webview_find_element selector="button" strategy=css
```

**4.4 交互测试 — 点击、输入**

对于 React 受控组件的输入，必须使用 JS 注入方式：

```javascript
// mcp__tauri-test__webview_execute_js
(() => {
  const el = document.querySelector('textarea');
  const nativeSetter = Object.getOwnPropertyDescriptor(
    window.HTMLTextAreaElement.prototype, 'value'
  ).set;
  nativeSetter.call(el, '测试文本');
  const tracker = el._valueTracker;
  if (tracker) tracker.setValue('');
  el.dispatchEvent(new Event('input', { bubbles: true }));
  return el.value;
})()
```

对于按钮点击：

```
mcp__tauri-test__webview_interact action=click selector="button.submit"
```

### Step 5: 清理

测试完成后移除 MCP 连接并终止应用：

```
manage_mcp_server action=remove id=tauri-test
```

```bash
pkill -f "目标二进制名"
```

## 常见验证场景模板

### 验证页面基本渲染

```
1. webview_dom_snapshot mode=ai → 确认关键 UI 元素存在
2. webview_execute_js "document.querySelectorAll('button').length" → 确认按钮数量
3. webview_execute_js "document.title" → 确认页面标题
```

### 验证 IPC 命令

```
1. webview_execute_js "window.__TAURI_INTERNALS__.invoke('command_name', {arg: 'value'})" → 调用后端命令
2. 检查返回值是否符合预期
```

注意：Tauri v2 的内部 API 是 `window.__TAURI_INTERNALS__`，不是 `window.__TAURI__`。如果前端使用了 `@tauri-apps/api`，则通过 `import { invoke } from '@tauri-apps/api/core'` 来调用。

### 验证文件操作

```
1. 通过 shell 创建测试文件
2. 通过 MCP JS 执行触发前端文件加载逻辑
3. 通过 DOM 快照验证文件内容已渲染
```

## 注意事项

- MCP Bridge 默认端口 9555，如端口被占则自动递增至 9655
- WebKitGTK（Linux）上隐藏窗口会暂停 JS 执行，所有 MCP 操作会超时
- React 受控组件不响应合成的 `input` 事件，必须用 `nativeSetter` 方式注入值
- `webview_interact click` 在 WebKitGTK 上不触发原生 focus，用 `webview_execute_js` 设焦点
- Goal 模式下自测时，测试完成后务必 `update_goal` 标记完成，否则 continuation 会无限循环

## 故障排查

| 症状 | 原因 | 修复 |
|------|------|------|
| MCP 连接超时 | 应用未启动或端口不对 | 检查进程和端口 |
| webview 操作超时 | 窗口不可见 | 重启应用 |
| DOM 快照为空 | 前端未加载 | 检查 devUrl 和 vite 端口 |
| IPC 调用失败 | 权限未配置 | 检查 capabilities/default.json |
| connector 编译失败 | Tauri v1 项目 | 本 Skill 仅支持 Tauri v2 |
