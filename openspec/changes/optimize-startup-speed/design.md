## Context

FastClaw 采用 Tauri 嵌入式架构：Tauri 主进程 spawn 一个 async task 启动 Gateway（axum HTTP/WS 服务），前端 WebView 通过 IPC 获取 Gateway 地址后建立 WebSocket 连接。

当前启动链路存在三个独立但可叠加的延迟源：

1. **双重轮询**：`probe_gateway` 每 100ms HTTP 探测 + `get_gateway_info` IPC 每 500ms 锁查询
2. **串行 DB**：sessions.db、evolution.db、cron.db 三个 SQLite 连续打开
3. **前端阻塞**：WebView 在 `mode=connecting` 时只渲染空白 Loading，直到 Gateway + WS + syncBackendData 全部完成

## Goals / Non-Goals

**Goals:**
- 将启动到可交互的时间减少 30%+
- 消除轮询带来的随机延迟（最差 600ms）
- 用户在 Gateway 启动过程中就能看到 UI 骨架
- 保持架构简洁，不引入新的外部依赖

**Non-Goals:**
- 不做 StateBuilder 5 阶段的并行化拆分（留作后续优化）
- 不做 Skills 扫描缓存（留作后续优化）
- 不改变前端框架或路由架构
- 不做冷启动编译优化（binary size / LTO 等）

## Decisions

### D1: Watch Channel 替代双重轮询

**选择**：用 `tokio::sync::watch<GatewayStartupState>` 替代 `probe_gateway` HTTP 轮询 + `get_gateway_info` lock 轮询。

**当前**：
```
embedded.rs: loop { probe_gateway(port).await; sleep(100ms) }  // HTTP GET /health
config.rs:   loop { state.gateway.lock().await; sleep(500ms) }  // IPC poll
```

**改为**：
- `lib.rs` setup 中创建 `watch::channel(GatewayStartupState::Starting)`
- Gateway async task 在 ready/failed 时 `tx.send()`
- `embedded.rs` 的 `GatewayProcess::start` 不再自己轮询，直接 `rx.changed().await`
- `get_gateway_info` IPC 命令也改为 `rx.changed().await`（带 timeout）

**理由**：watch channel 是零延迟通知，没有轮询间隔开销，也不需要额外的 HTTP 请求。

**替代方案**：
- `oneshot` channel — 只能用一次，不支持多个接收者
- `broadcast` channel — 过重，watch 更适合状态同步场景

### D2: 合并 SQLite 为单一连接池

**选择**：将 sessions.db、evolution.db、cron.db 合并到单一 SQLite 文件（`fastclaw.db`）中。

**当前**：
- Phase 1 打开 sessions.db（SessionStore + EventLog）
- Phase 2 打开 evolution.db（FeedbackStore + TrajectoryStore + SkillStore + PromptDistiller）
- Phase 5 打开 cron.db（CronJobStore + NotificationStore）

**改为**：
- Phase 1 打开 `fastclaw.db`，创建共享 `SqlitePool`
- Phase 2/5 复用同一个 pool，各自的 `ensure_table` / `open` 接受 pool 参数
- 表名保持不变，无数据迁移问题（新安装直接建在同一 DB，已有安装需迁移脚本）

**理由**：每次 `SqlitePool::connect` 包含文件打开、WAL 初始化、连接池预热，节省 2 次相当于节省约 100-300ms。

**替代方案**：
- 保持 3 个 DB + 并行打开 — 并行化有依赖复杂度，且 SQLite 单文件是更简洁的最终态
- 使用 ATTACH DATABASE — 增加查询复杂度，不如直接合并

**迁移策略**：
- 检测是否存在旧的独立 DB 文件（sessions.db, evolution.db, cron.db）
- 若存在，在启动时自动将表和数据迁移到 fastclaw.db
- 迁移成功后重命名旧文件为 `.bak`
- 新安装直接使用 fastclaw.db

### D3: 前端渐进式启动

**选择**：将 `mode` 状态从二态（connecting / ready）扩展为三态（shell / connecting / ready），在 Gateway 未就绪时就渲染完整的 UI 外壳。

**当前**：
```
mode=connecting → <Loading />（空白加载）
mode=ready     → 完整 UI
```

**改为**：
```
mode=shell      → 渲染 Sidebar + TitleBar + 骨架占位 (gateway 启动中)
mode=connecting → UI 已渲染，WebSocket 连接中（底部小状态条）
mode=ready      → 完全可交互
```

**具体设计**：
- `AppLayout` 在 `mode=shell` 时渲染固定 UI 结构（侧栏、标题栏、空聊天区域骨架）
- 从 localStorage 读取上次的 session 列表做占位显示（即使数据可能过期）
- WebSocket 连接成功后自动刷新真实数据
- 交互按钮在 `mode !== ready` 时 disabled 但可见

**理由**：用户感知的"启动速度"很大程度取决于是否看到了 UI 响应。骨架屏让用户感觉 app 在快速加载。

## Risks / Trade-offs

- **[DB 合并迁移]** 旧用户升级时需要迁移 3 个 DB → 1 个，迁移失败会丢数据
  → 迁移使用事务；失败时回退保持旧文件不动；保留 `.bak` 备份
- **[前端骨架数据过期]** localStorage 中缓存的 session 列表可能与后端不一致
  → WS 连接后立刻全量同步覆盖，过期数据只展示几秒
- **[watch channel 生命周期]** watch channel sender 必须在 Gateway task 中持有不被 drop
  → sender 存入 AppData，与 GatewayProcess 生命周期一致
