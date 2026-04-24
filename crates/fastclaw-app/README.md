# fastclaw-app

FastClaw 跨平台桌面应用 — 基于 Tauri 2 + React 19 的 AI Agent IM 客户端。

## 架构

```
fastclaw-app/
├── src-tauri/           # Rust 侧（Tauri 后端）
│   ├── src/
│   │   ├── lib.rs       # Tauri 应用初始化、系统托盘、全局快捷键
│   │   ├── commands.rs  # IPC 命令（网关、Agent、会话、聊天、技能、配置等）
│   │   ├── embedded.rs  # 内嵌网关启动（进程内 Axum）
│   │   └── main.rs      # 入口
│   ├── resources/lsp/   # 内置 rust-analyzer 等 LSP 二进制
│   └── capabilities/    # Tauri 权限配置
├── src/                 # React 前端
│   ├── App.tsx          # 根组件
│   ├── components/      # UI 组件（聊天流、Agent 列表、设置面板等）
│   └── lib/             # 状态管理 (Zustand)、API 层、WebSocket 传输
├── package.json         # pnpm + Vite + React 19 + Tailwind 4
└── vite.config.ts
```

## 核心特性

- **零配置启动** — 内嵌 `fastclaw-gateway` 进程内运行，双击即用
- **系统托盘** — 最小化到托盘，show/quit 菜单
- **全局快捷键** — Ctrl+Shift+Space 显示/隐藏主窗口
- **流式聊天** — Tauri Channel 推送 delta/tool.start/tool.done/complete/error 事件
- **人机交互** — `ask_question` 工具回环，Agent 可向用户征求结构化决策
- **技能管理** — 查看、启用/禁用、上传技能（文件夹或 zip）
- **Agent CRUD** — 创建、编辑、删除 Agent，per-agent 工具 allow/deny
- **上下文用量指示器** — 聊天流末尾显示 `ctx 10.2k / 128k` 实时用量（绿/黄/红三色编码：<50% / 50–80% / >80%），可在 Agent 设置与全局模型设置中编辑 `contextWindow`
- **配置安全** — 通过 `config_access` ACL 过滤敏感信息

## 开发

```bash
cd crates/fastclaw-app
pnpm install
cargo tauri dev
```

## 构建发布

```bash
cargo tauri build
```

发布包位于 `src-tauri/target/release/bundle/`。CI 使用 `tauri-apps/tauri-action` 自动构建并发布到 GitHub Releases（标签 `app-v*`）。
