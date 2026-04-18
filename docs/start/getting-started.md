---
title: 快速开始
summary: 安装 FastClaw、启动网关、通过 TUI 完成首次对话与后续学习路径。
---

# 快速开始

## FastClaw 是什么

FastClaw 是用 Rust 实现的 **AI Agent 编排引擎**：在统一网关后接入多种即时通讯渠道与工具，将会话路由到配置的 Agent，由 LLM 与内置/WASM/MCP 工具协同完成自动化任务。同时提供 **Tauri 跨平台桌面应用**，内嵌网关进程，双击即用。设计目标包括与 OpenClaw 配置形态兼容、可观测、可扩展的工作流（DAG）、记忆与多 Agent 协作。

## 快速安装

### 方式一：桌面应用（推荐本地使用）

从 [GitHub Releases](https://github.com/example/fastclaw/releases)（标签 `app-v*`）下载对应平台安装包，双击安装后即可使用。桌面应用内嵌网关，无需额外启动服务。

从源码构建：

```bash
cd crates/fastclaw-app
pnpm install
cargo tauri build
```

### 方式二：CLI

在仓库根目录执行：

```bash
cd /path/to/FastClaw
cargo build --release
```

二进制默认位于 `target/release/fastclaw`。可将该目录加入 `PATH`，或使用 `cargo install --path crates/fastclaw-cli`（若已配置为可安装 crate）。

可选：使用开发态目录隔离配置与数据：

```bash
fastclaw --dev doctor
```

## 首次运行

> FastClaw 首启会自动创建 `~/.fastclaw/` 目录结构与 `config/default.json`（最小默认配置）。  
> 创建或更新 Agent 时，会自动初始化该 Agent 的 `SOUL.md`、`USER.md`、`AGENTS.md` 身份文件。

### 桌面应用

打开应用后，若无 Agent 存在，首屏显示"欢迎使用 FastClaw"引导卡片：

1. 点击「去配置模型」 — 设置 LLM 提供商与 API Key
2. 点击「一键创建默认 Agent」 — 自动创建 `main` Agent 并进入聊天界面

桌面应用内嵌网关（进程内启动，无需手动 `fastclaw serve`），支持系统托盘、全局快捷键 **Ctrl+Shift+Space** 显示/隐藏窗口。

### 启动网关（CLI 模式）

前台运行（等同于 `gateway run`）：

```bash
fastclaw serve
```

或显式子命令：

```bash
fastclaw gateway run
```

默认监听 `127.0.0.1:18789`（可在配置中修改 `gateway.port`）。后台进程：

```bash
fastclaw gateway start
fastclaw gateway status
```

### 终端对话（TUI）

当前 CLI 提供的交互式聊天入口为 **`fastclaw tui`**（通过 WebSocket 连接网关）。若你习惯口语化称呼「chat」，请以子命令 **`tui`** 为准。

```bash
fastclaw tui --url ws://127.0.0.1:18789/ws
```

可选参数：

- `--token`：网关启用了 API Key 认证时传入。
- `--session`：恢复指定会话 ID。

在 TUI 中输入内容并发送，即向网关路由后的 **默认 Agent**（配置中 `agents.list` 里 `default: true` 的项，通常为 `main`）发起一轮对话。

## Hello World：给默认 Agent 发一条消息

1. 确保 `config/default.json`（或 `~/.fastclaw/config/default.json`）中已配置模型与凭证。
2. 若桌面端首屏显示“欢迎使用 FastClaw”引导卡片，可点击「去配置模型」后再点「一键创建默认 Agent」。
3. 在 CLI/网关模式下，确认至少有一个可用 Agent（通常为 `main`）。
4. `fastclaw serve` 启动网关。
5. 另开终端：`fastclaw tui`，输入 `你好，请用一句话介绍你能做什么` 并发送。

若收到流式或完整回复，说明 **消息 → 网关 → 路由 → Agent → LLM → 响应** 全链路已打通。

也可使用 HTTP API（需按部署开启认证），例如：

```bash
curl -sS -X POST http://127.0.0.1:18789/api/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -d '{"model":"main","messages":[{"role":"user","content":"ping"}],"stream":false}'
```

具体字段与流式行为见 [REST API 参考](../reference/api.md)。

## 下一步

- [文档地图（Hubs）](./hubs.md) — 按主题浏览全部文档
- [系统架构概览](../concepts/architecture.md)
- [工具与插件](../tools/index.md) — 内置工具、LSP 集成、WASM 插件
- [代码智能](../code/index.md) — LSP、Tree-sitter、代码图
- [网关配置](../gateway/configuration.md)
- [CLI 参考](../cli/index.md)
- [常见问题](../help/faq.md)
