## 1. Watch Channel 替代双重轮询

- [x] 1.1 在 `AppData` 中添加 `watch::Sender<GatewayStartupState>` 和对应的 `watch::Receiver`
- [x] 1.2 修改 `lib.rs` setup 闭包：创建 watch channel，Gateway async task 在 ready/failed 时通过 sender 发送状态
- [x] 1.3 移除 `embedded.rs` 中的 `probe_gateway` 函数和 HTTP 轮询循环
- [x] 1.4 修改 `GatewayProcess::start`：接受 watch sender，启动后通过 sender 通知就绪状态
- [x] 1.5 修改 `commands/config.rs` 的 `get_gateway_info`：用 `watch_rx.changed().await` 替代 Mutex 轮询
- [x] 1.6 验证：启动后 Gateway 就绪通知零延迟到达前端

## 2. 合并 SQLite 为单一连接池

- [x] 2.1 在 `state/builder.rs` Phase 1 中创建单一 `xiaolin.db` 的 `SqlitePool`
- [x] 2.2 修改 `SessionStore::open` 支持接受已有 pool（新增 `from_pool`）
- [x] 2.3 修改 `EventLog::new` 支持复用已有 pool（EventLog::new 已接受 pool 参数）
- [x] 2.4 修改 Phase 2 的 evolution 相关 store（FeedbackStore、TrajectoryStore、SkillStore、PromptDistiller）使用共享 pool
- [x] 2.5 修改 Phase 5 的 CronJobStore 和 NotificationStore 使用共享 pool
- [x] 2.6 移除 `helpers::open_memory_pool_named` 对独立 DB 的调用
- [x] 2.7 实现旧数据库自动迁移逻辑：检测 sessions.db/evolution.db/cron.db → 迁移到 xiaolin.db → 重命名为 .bak
- [x] 2.8 验证：新安装只创建一个 xiaolin.db；旧安装升级后数据完整

## 3. 前端渐进式启动

- [x] 3.1 在 `store.ts` 的 `GatewayState` 中添加 `shell` 模式，初始状态设为 `shell`
- [x] 3.2 修改 `init()` 流程：先设 mode=shell，IPC 返回后设 mode=connecting，WS 连接后设 mode=ready
- [x] 3.3 在 `AppLayout.tsx` 中实现 shell 模式的骨架 UI（sidebar 骨架 + titlebar + chat 区域 skeleton）
- [x] 3.4 从 localStorage 读取上次 session 列表作为骨架占位数据
- [x] 3.5 在 WS 连接成功后的 `syncBackendData` 中将真实数据写入 localStorage 作为下次缓存
- [x] 3.6 在 shell/connecting 模式下禁用交互控件（input bar、导航按钮等），但保持可见
- [x] 3.7 验证：启动时立即看到 UI 骨架，Gateway 就绪后无缝切换到完整 UI
