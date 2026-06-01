## Context

FastClaw 的聊天请求热路径从 HTTP 入口到 LLM 响应涉及多个 crate 的协作：gateway（路由 + pipeline）→ session（消息持久化 + 缓存）→ context（上下文组装 + token 估算）→ agent（LLM 调用 + 工具执行）。当前实现存在以下开销模式：

1. **消息历史在每次请求中被深拷贝 5+ 次**：`load_chat_messages` 缓存命中时 clone 整个 Vec、`user_messages.clone()` ×2、`build_messages` 中 extend、LLM 序列化前的 slice 构建
2. **JSON 双重序列化桥接**：stream path 将 `ChatSetup` 序列化为 `serde_json::Value` 放入 `SessionOp::UserTurn` 的 extra map，session_bridge 再反序列化回来
3. **token 估算函数对每条消息做 `serde_json::to_string`**：被多处调用（pipeline、compressor、session_bridge）
4. **event_log 每个 WS delta 写一次 SQLite**：一次回复可能上百个 INSERT
5. **GC key 不匹配**：`chat_locks` 用 `chat_id` 做 key，GC 用 session actor ID 做 active 集合，IM 渠道的 session key 无法被 GC

## Goals / Non-Goals

**Goals:**
- 将单次聊天请求中消息历史的深拷贝次数从 5+ 降到 ≤2
- 消除 stream path 的 JSON 中转，改为类型化传递
- token 估算不再触发 JSON 序列化
- event_log 写入吞吐量提升 10x+（batch 化）
- 修复 GC key 不匹配导致的 DashMap 慢泄漏
- `text_content()` 热路径调用不再分配新 String

**Non-Goals:**
- 不重构 `ChatMessage.content` 从 `serde_json::Value` 到 typed enum（影响面过大，留作后续）
- 不修改 LLM provider 层的序列化（reqwest `.json()` 是必要的）
- 不引入 per-session 分片锁（P2 优化，不在本次范围内）
- 不改变 session actor 的架构模型

## Decisions

### D1: msg_cache 值类型改为 `Arc<Vec<ChatMessage>>`

**选择**: `Arc<Vec<ChatMessage>>` + `Arc::make_mut` on write

**替代方案**:
- `im::Vector<ChatMessage>`（persistent data structure）—— 引入新依赖，API 不熟悉
- `Arc<[ChatMessage]>`（immutable slice）—— 无法 push，append 时需重建

**理由**: `Arc<Vec>` 是最小侵入改动。读路径（高频）变为 `Arc::clone`（atomic inc）。写路径（相对低频）通过 `Arc::make_mut` 实现 copy-on-write——如果只有一个持有者则直接 mutate，否则 clone 后 mutate。对调用方影响最小：`load_chat_messages` 返回 `Arc<Vec<ChatMessage>>`，多数调用方只需 `&*arc` 即可访问 slice。

### D2: token 估算直接遍历 Value 树

**选择**: 递归遍历 `serde_json::Value` 累加字符数

**替代方案**:
- 在 `ChatMessage` 上缓存 `content_char_count: Option<usize>` —— 需要在每次 content 变更时失效，增加复杂度
- 使用 `serde_json::to_writer` 写入 `/dev/null` 计数 —— 仍有 IO trait 开销

**理由**: `Value` 的结构是已知的（String / Array / Object），递归遍历计算字符数是 O(n) 且零分配，比 `to_string`（分配完整 String buffer）快一个数量级。实现简单，约 15 行代码。

### D3: event_log 批量写入

**选择**: 内部 `mpsc` channel + 单 writer task，定时 flush（50ms 间隔或 buffer 满 64 条）

**替代方案**:
- SQLite WAL batch transaction（手动 `BEGIN`/`COMMIT`）—— 需要改 SQLite pool 交互模式
- 直接跳过 ContentDelta 不记录 —— 会丢失 replay 能力

**理由**: 独立 writer task 完全解耦了热路径和 DB I/O。50ms window 平衡了延迟和吞吐：最坏情况丢失 50ms 数据（进程 crash），但正常流程中 flush 是可靠的。单 writer 还避免了 SQLite pool 连接争用。

### D4: stream path 类型化传递

**选择**: `SessionOp::UserTurn` 直接携带 `Arc<ChatSetup>` + `Arc<AgentConfig>`

**替代方案**:
- 用 `oneshot<Arc<ChatSetup>>` 在 submit 时单独传入 —— 多一个 channel 管理
- 保留 `serde_json::Value` 但在 session_bridge 中用 `from_value` 的 zero-copy 优化 —— serde_json 的 `from_value` 仍需遍历和分配

**理由**: 类型化传递完全消除序列化/反序列化开销。`SessionOp` 的 `extra: HashMap<String, Value>` 本身就是一个过度通用的设计——特定 op 携带特定类型更清晰。需要修改 `SessionOp` enum 和 actor 处理逻辑。

### D5: text_content() → Cow<str>

**选择**: 返回 `Option<Cow<'_, str>>`

**替代方案**:
- 返回 `Option<&str>`（仅当 content 是 String 时可用）—— Array 类型无法返回引用
- 新增 `text_content_ref()` 不改原方法 —— 调用方需要逐个迁移，维护两套 API

**理由**: `Cow<str>` 对 `Value::String` 返回 `Borrowed`（零拷贝），对 `Value::Array` 返回 `Owned`（join 产生的新 String，与现在行为一致）。调用方通过 `&*cow` 或 `.as_ref()` 使用，改动量可控。

### D6: chat_locks GC key 对齐

**选择**: 统一使用 `session_key` 作为 DashMap key

**替代方案**:
- 为 channel sessions 维护独立的 GC 集合 —— 增加复杂度
- 在 GC 中额外查询 channel registry 获取活跃 session key —— 增加 GC 延迟

**理由**: `session_key` 已经是 session actor 的唯一标识，在 `get_or_create` 中使用。将 `chat_locks`/`chat_cancels` 的 key 统一为 `session_key` 是最干净的修复。

## Risks / Trade-offs

**[Risk] event_log batch 窗口内数据丢失** → 进程异常退出时最多丢失 50ms 窗口的事件。Mitigation: 在 graceful shutdown 中 flush 残余 buffer；对于崩溃场景，event_log 本身是调试/审计用途，丢失少量末尾事件可接受。

**[Risk] `Arc<Vec<ChatMessage>>` 引入引用计数开销** → 每次 clone 是 atomic increment（~1ns），远小于当前的深拷贝（~100μs for 200 条消息）。净收益巨大。

**[Risk] text_content() 签名变更影响面广** → 约 15 处调用方需要适配。Mitigation: 编译器会捕获所有不匹配；多数调用方只需加 `.as_ref()` 或 `&*`。

**[Risk] SessionOp 类型化改造可能影响 actor 扩展性** → 当前 `extra: HashMap<String, Value>` 的通用设计被特化。Mitigation: 仅对 `UserTurn` 做特化，其他 op 保持 extra map；或改为 enum variant 各自携带类型化数据。

**[Risk] GC key 变更可能导致旧格式 key 残留** → 部署后首次 GC 前存在的旧 key 不会被清除。Mitigation: 在启动时或首次 GC 时做一次全量清理。
