---
title: CLI 参考
summary: fastclaw 命令行子命令：网关、会话、Agent、工具、MCP、诊断与全局选项。
---

# CLI 参考

全局选项（节选）：

| 选项 | 说明 |
|------|------|
| `--dev` | 使用 `~/.fastclaw-dev/` 状态目录 |
| `--profile <name>` | 使用 `~/.fastclaw-<name>/` |
| `--no-color` | 关闭着色 |
| `--json` | JSON 输出（支持部分子命令） |

## `fastclaw serve`

等同于 **`fastclaw gateway run`**：前台启动网关，阻塞至进程退出。适合本地开发与 systemd `Type=simple` 单元。

```bash
fastclaw serve
```

## `fastclaw gateway`

| 子命令 | 说明 |
|--------|------|
| `run` | 前台运行 |
| `start` | 后台守护（写入 `daemon.pid`，输出写入日志文件） |
| `stop` | 停止后台实例 |
| `restart` | 重启后台实例 |
| `status` | 查看后台 PID 与存活 |
| `health` | 对运行中的实例做健康探测 |

```bash
fastclaw gateway start
fastclaw gateway status
fastclaw gateway health
```

**守护进程日志**：`gateway start` 启动的后台进程将 stdout/stderr 重定向至 `<state_dir>/logs/gateway-daemon.log`（追加模式）。启动时终端会打印日志路径。同时设置 `RUST_BACKTRACE=1` 以便调试崩溃。

## 终端对话：`fastclaw tui`

交互式 TUI，经 WebSocket 连接网关（默认 `ws://127.0.0.1:18789/ws`）。

```bash
fastclaw tui --url ws://127.0.0.1:18789/ws --token YOUR_API_KEY
```

> 说明：仓库当前 CLI **没有** `chat` 子命令；请使用 **`tui`** 作为聊天入口。

## `fastclaw mcp-server`

启动 **stdio MCP 服务**，将 FastClaw 工具暴露给外部 Agent 宿主：

```bash
fastclaw mcp-server
```

在 MCP 客户端中配置可执行文件路径及工作目录。

## `fastclaw app`

启动 Tauri 桌面应用（需从桌面安装包或 `cargo tauri build` 构建）。桌面应用内嵌网关进程，无需先执行 `fastclaw serve`。

```bash
fastclaw app
```

功能包括：系统托盘、全局快捷键（Ctrl+Shift+Space）、流式聊天、Agent/会话/技能管理、首启引导。与 CLI 共享 `~/.fastclaw/` 配置与数据目录。

## `fastclaw doctor`

环境诊断：配置路径、二进制版本、网络连通性等（具体检查项以实现为准）。

```bash
fastclaw doctor
```

## 其他常用子命令

| 命令 | 说明 |
|------|------|
| `fastclaw setup` / `onboard` | 引导式初始化 |
| `fastclaw config get KEY` / `set KEY VALUE` | 点路径读写配置 |
| `fastclaw config check` | 校验配置 |
| `fastclaw config file` / `path` | 打印合并后配置或路径 |
| `fastclaw sessions list|get|delete|cleanup` | 会话运维 |
| `fastclaw agents list` / `get <id>` | Agent 列表与详情 |
| `fastclaw tools list` | 内置工具列表 |
| `fastclaw completions <shell>` | 生成 shell 补全脚本 |

## 相关文档

- [快速开始](../start/getting-started.md)
- [REST API](../reference/api.md)
- [网关配置](../gateway/configuration.md)
