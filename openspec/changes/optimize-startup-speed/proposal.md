## Why

FastClaw 当前启动链路存在明显延迟：Gateway 的 5 个初始化阶段完全串行、前端通过两层轮询等待 Gateway 就绪、3 个独立 SQLite 数据库顺序打开。用户从点击应用到看到可交互界面需要较长等待。优化启动速度直接影响用户体验和产品感知质量。

## What Changes

- **消除双重轮询等待**：用 `tokio::sync::watch` channel 替代 `probe_gateway` 的 HTTP 轮询 + `get_gateway_info` IPC 的 lock 轮询，Gateway ready 时零延迟通知
- **前端渐进加载**：不等 Gateway ready 就渲染 UI 骨架和外壳，WS 连接建立后再填充数据，缩短用户感知的等待时间
- **合并 SQLite 数据库**：将 sessions.db、evolution.db、cron.db 合并为单一连接池，减少多次 open + WAL 初始化开销

## Capabilities

### New Capabilities
- `startup-watch-notify`: 用 watch channel 替代双重轮询，实现 Gateway 就绪的零延迟通知机制
- `progressive-frontend-boot`: 前端渐进式启动，在 Gateway 未就绪时先渲染 UI 骨架
- `unified-sqlite-pool`: 将 3 个独立 SQLite 数据库合并为单一连接池

### Modified Capabilities

## Impact

- **后端**：`crates/fastclaw-app/src-tauri/src/embedded.rs` — 移除 probe_gateway 轮询，改用 watch channel
- **后端**：`crates/fastclaw-app/src-tauri/src/commands/config.rs` — get_gateway_info 改为 watch 等待
- **后端**：`crates/fastclaw-gateway/src/state/builder.rs` — 合并 SQLite 连接池
- **后端**：`crates/fastclaw-session/` — SessionStore 支持复用连接池（如需调整）
- **前端**：`crates/fastclaw-app/src/lib/store.ts` — 适配渐进加载模式
- **前端**：`crates/fastclaw-app/src/components/layout/AppLayout.tsx` — 骨架屏加载状态
