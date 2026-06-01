## Why

FastClaw 的聊天热路径存在多处冗余的序列化、深拷贝和同步 I/O，导致单次请求中消息数据被 JSON 序列化/反序列化/克隆 10+ 次。对于有上百条消息的长 session，每次请求浪费数 MB 的内存分配和可观的 CPU 时间。此外，WS 流式路径每个 delta 都写一次 SQLite，严重限制了吞吐量。这些问题在生产环境中随着会话增长和并发上升会显著影响用户体验（首 token 延迟、内存尖峰）。

## What Changes

- 将 `SessionStore.msg_cache` 的值类型从 `Vec<ChatMessage>` 改为 `Arc<Vec<ChatMessage>>`，缓存命中时返回 `Arc::clone` 而非深拷贝
- 重写 `estimate_single_message_tokens` 以直接遍历 `serde_json::Value` 树计算字符数，消除每次调用的 `to_string` 序列化
- 修复 `chat_locks` / `chat_cancels` 的 GC key 不匹配问题，确保 IM 渠道的 session 资源能被正确回收
- 实现 event_log 批量写入机制，将 per-delta INSERT 改为定时 batch flush
- 将 stream path 的 JSON 中转（`to_value` → `from_value`）替换为类型化 channel 直接传递 `Arc<ChatSetup>`
- 将 `text_content()` 返回类型从 `Option<String>` 改为 `Option<Cow<'_, str>>`，减少热路径上的 String 分配

## Capabilities

### New Capabilities
- `session-cache-arc`: SessionStore 消息缓存的 Arc 共享机制，消除读取时的深拷贝
- `event-log-batching`: Event log 批量写入，将高频 per-event INSERT 改为定时 batch flush
- `stream-typed-channel`: 流式路径的类型化 channel 传递，消除 JSON 双重序列化

### Modified Capabilities
- `subagent-reactive-loop`: GC key 修复影响 reactive loop 中 session 资源的清理逻辑

## Impact

- **核心 crate**: `fastclaw-session`（store.rs 缓存模型）、`fastclaw-core`（types.rs text_content 签名）、`fastclaw-context`（compressor.rs token 估算）、`fastclaw-gateway`（chat_pipeline.rs / routes/chat.rs / state/mod.rs）
- **API 兼容性**: `text_content()` 返回类型变更为 `Cow<str>`，所有调用方需适配（约 15 处）
- **行为变更**: event_log 写入从实时改为批量，极端情况下进程崩溃可能丢失最后一个 batch 窗口内的事件
- **依赖**: 无新增外部依赖
