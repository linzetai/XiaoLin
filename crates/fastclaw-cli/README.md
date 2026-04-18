# fastclaw-cli

FastClaw 的命令行入口二进制，提供完整的 Agent 编排引擎运维与交互能力。

## 功能

- **`fastclaw serve`** — 前台启动 HTTP/WebSocket 网关（等同 `gateway run`）
- **`fastclaw gateway start|stop|restart|status|health`** — 后台守护进程管理
- **`fastclaw tui`** — 终端交互式聊天（通过 WebSocket 连接网关）
- **`fastclaw mcp-server`** — stdio MCP 服务，将 FastClaw 工具暴露给外部 Agent 宿主
- **`fastclaw setup|onboard`** — 引导式初始化
- **`fastclaw config get|set|check|file|path`** — 配置管理
- **`fastclaw sessions|agents|tools`** — 会话、Agent、工具查询
- **`fastclaw doctor`** — 环境诊断
- **`fastclaw completions <shell>`** — Shell 补全脚本生成

## 依赖

核心引用 `fastclaw-gateway`、`fastclaw-core`、`fastclaw-agent`、`fastclaw-observe`、`fastclaw-session`、`fastclaw-collab` 等 workspace crate；TUI 基于 `ratatui` + `crossterm`，WebSocket 使用 `tokio-tungstenite`。

## 构建

```bash
cargo build --release -p fastclaw-cli
# 二进制位于 target/release/fastclaw
```
