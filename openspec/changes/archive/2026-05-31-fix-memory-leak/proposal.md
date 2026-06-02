# fix-memory-leak

## 问题

XiaoLin 运行时内存占用达到 40GB 并持续上涨，导致电脑卡死。不确定是 dev 模式还是安装包触发。

## 根因分析

通过代码审查，识别出以下内存泄漏/无限积累点：

### 1. SessionManager 永不 GC（严重）

`SessionManager.sessions: RwLock<HashMap<SessionId, Arc<SessionHandle>>>` 只增不减。

- `get_or_create()` 每次对话创建 session 并存入 HashMap
- `gc()` 方法存在但**没有任何定时调度**
- Session actor 停止后，SessionHandle (含 async_channel、EventFanout、approval cache) 仍被 HashMap 持有
- 长期运行 → sessions 数量无限增长

### 2. LLM Streaming 无输出长度限制（严重）

`ws/chat.rs` 的 `spawn_chat` 中：

```rust
assistant_content.push_str(text);  // 无限积累直到 TurnEnd
```

如果 LLM streaming 出现异常（不发 stop signal），String 无限增长。每条 delta 还通过 `serde_json::to_value(event)` 序列化并写入 EventLog。

### 3. DashMap 无清理机制（中等）

- `chat_locks: DashMap<String, Arc<Semaphore>>` — 每个唯一 chat_id 永不移除
- `stream_event_tx: DashMap<String, Sender>` — stream context key 可能泄漏
- `chat_model_overrides: DashMap<String, String>` — 永不清理

### 4. EventFanout Subscriber 积累（低）

虽然 actor loop 有 `fanout.gc()`，但如果 actor 停止（不再循环），已关闭的 subscriber sender 仍占内存。

## 修复方案

### A. SessionManager 自动 GC（必须）

- 在 gateway 启动时 spawn 一个定时任务，每 60s 调用 `session_manager.gc()`
- 同时清理 `chat_locks`、`chat_model_overrides` 中对应已死 session 的条目

### B. LLM Streaming 输出限制（必须）

- 给 `assistant_content` 设置 max size（如 2MB）
- 超过限制时截断并触发 turn abort
- 给整个 turn 设置最大 duration（如 10min）

### C. DashMap TTL 清理（建议）

- 给 `chat_locks` 和 `chat_model_overrides` 加入清理逻辑
- 方案：在 SessionManager GC 周期内一并清理不再活跃的条目

### D. 内存监控（建议）

- 添加一个 `/health` endpoint 返回 RSS 内存信息
- 当 RSS > 配置阈值时输出 warning 日志
- 可选：在 Tauri App 托盘显示内存使用

## 影响范围

- `crates/xiaolin-gateway/src/lib.rs` — 启动 GC 定时任务
- `crates/xiaolin-session-actor/src/manager.rs` — GC 逻辑增强
- `crates/xiaolin-gateway/src/ws/chat.rs` — streaming 限制
- `crates/xiaolin-gateway/src/state/mod.rs` — DashMap 清理方法
