# Proposal: 简化为单进程架构

## 概述

将 XiaoLin 从"Gateway + Desktop App + CLI + TUI"多进程分体架构，简化为**单进程桌面应用**。Gateway 完全内嵌在 App 中，移除 CLI/TUI 作为独立入口，用户无需理解"网关"概念。

## 动机

当前多进程架构带来了大量工程复杂度和用户体验问题：

1. **安装新包不更新 Gateway** — daemon 独立运行，新 app 连上旧 gateway
2. **sandbox-exec 错误** — daemon 启动环境与 app 不同，继承错误的 CWD/权限
3. **孤儿进程管理** — gateway daemon 可能泄漏、端口冲突
4. **概念负担** — 用户不需要也不应该理解 gateway/daemon/CLI 的关系
5. **配置分裂** — 多进程间的状态同步（gateway.json）增加了失败面

## 产品定位

XiaoLin 是**住在你电脑里的全能 AI 助手**：
- 用户通过 Desktop App（或系统托盘）与 AI 交互
- 通过飞书/远程渠道可远程操控（App 在后台运行即可）
- 用户视角只有一个概念："XiaoLin 开着 = AI 在线"

## 方案

### 移除的组件

| 组件 | 当前状态 | 处理方式 |
|------|---------|---------|
| `xiaolin-cli` | 独立二进制，含 TUI/gateway 管理/config 等 | **删除整个 crate** |
| TUI (ratatui) | CLI 子命令 | **删除** |
| `EmbedMode` 枚举 | Auto/Always/Never | **强制 Always，移除枚举** |
| `GatewayState` (gateway.json) | daemon 发现机制 | **删除** |
| `find_xiaolin_cli()` | 定位 CLI 二进制 | **删除** |
| `start_daemon()` | 启动外部 daemon | **删除** |
| daemon 模式 | fork + PID file + log file | **删除** |

### 保留/改造的组件

| 组件 | 改造方式 |
|------|---------|
| `xiaolin-gateway` (library) | 保留为 lib，被 app 直接内嵌调用 |
| `embedded.rs` | 简化为直接启动内嵌 gateway（移除 daemon fallback） |
| 飞书 channel | 保留，连接内嵌 gateway 的 WS 端口 |
| MCP Server/Client | 保留，作为工具扩展机制 |
| Cron/定时任务 | 保留，在内嵌 gateway 中运行 |
| 系统托盘 | 保留，App 最小化到托盘 = gateway 仍在运行 |
| `build-macos.sh` | 简化（不再需要注入 CLI 二进制） |

### 架构变化

**Before:**
```
┌─────────┐     ┌──────────────┐     ┌─────────┐
│  App    │────▶│   Gateway    │◀────│   CLI   │
│ (Tauri) │     │  (daemon)    │     │  (TUI)  │
└─────────┘     └──────────────┘     └─────────┘
     3 个进程，通过 WS/gateway.json 发现
```

**After:**
```
┌───────────────────────────────────┐
│         XiaoLin App              │
│  ┌─────────────────────────────┐  │
│  │     React Frontend          │  │
│  └─────────────┬───────────────┘  │
│                │ WS (in-process)   │
│  ┌─────────────▼───────────────┐  │
│  │     Gateway (embedded)      │  │
│  │  • Agent Runtime            │  │
│  │  • Tool System              │  │
│  │  • Session/Memory           │  │
│  │  • Cron                     │  │
│  │  • 飞书 Channel             │  │
│  └─────────────────────────────┘  │
│         1 个进程，系统托盘常驻      │
└───────────────────────────────────┘
```

## 非目标

- **不移除 gateway crate 本身** — 它作为 library 仍有独立价值（模块化）
- **不移除远程接入能力** — 飞书 channel 等仍通过 WS 连接内嵌 gateway
- **不影响 MCP 功能** — MCP server/client 保留
- **不影响 WASM 插件** — 仍在 gateway 内运行
- **不阻塞未来重新引入 CLI** — 如果未来需要 headless 模式（CI/CD、服务器场景），可以重新基于 gateway lib 构建

## 影响范围

### Crates 变动

- **删除**: `xiaolin-cli`
- **修改**: `xiaolin-app` (简化 embedded.rs)
- **修改**: `xiaolin-core` (移除 EmbedMode、GatewayState)
- **修改**: `xiaolin-gateway` (移除 daemon 相关的 startup 逻辑，保留 lib)
- **修改**: `scripts/build-macos.sh` (不再注入 CLI)
- **修改**: `Cargo.toml` workspace members

### 配置变动

- 移除 `gateway.embed` 配置项
- 移除 `gateway.json` 状态文件机制
- App 启动时直接内嵌 gateway，端口从 config 读取

## 风险

1. **飞书 channel 需要 App 在运行** — 可接受，系统托盘 + 开机自启已覆盖
2. **无 headless 模式** — 暂不需要，未来如有 CI 场景可重新构建
3. **开发调试** — 开发者可用 `cargo run -p xiaolin-gateway` 单独跑 gateway 调试
