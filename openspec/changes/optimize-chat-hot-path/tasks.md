## 1. Session Cache Arc 改造

- [x] 1.1 将 `SessionStore.msg_cache` 类型从 `RwLock<HashMap<String, Vec<ChatMessage>>>` 改为 `RwLock<HashMap<String, Arc<Vec<ChatMessage>>>>`
- [x] 1.2 修改 `load_chat_messages` 缓存命中路径：返回 `Arc::clone` 而非 `cached.clone()`；返回类型改为 `Arc<Vec<ChatMessage>>`
- [x] 1.3 修改 `append_message` / `append_messages`：使用 `Arc::make_mut` 获取可变引用后 push/extend
- [x] 1.4 修改 `replace_messages` / `invalidate_msg_cache`：适配 Arc 包装
- [x] 1.5 更新所有 `load_chat_messages` 调用方（session_bridge、chat_pipeline、ws/chat 等）适配新返回类型

## 2. Token 估算优化

- [x] 2.1 在 `fastclaw-context/src/compressor.rs` 中新增 `fn value_char_count(v: &serde_json::Value) -> usize`，递归遍历 Value 树累加字符数
- [x] 2.2 将 `estimate_single_message_tokens` 中的 `serde_json::to_string(c).map(|s| s.len())` 替换为 `value_char_count(c)`
- [x] 2.3 验证 token 估算结果与原实现的偏差在可接受范围内（±5%）

## 3. text_content 返回 Cow

- [x] 3.1 将 `ChatMessage::text_content()` 返回类型从 `Option<String>` 改为 `Option<Cow<'_, str>>`
- [x] 3.2 修改实现：`Value::String` 路径返回 `Cow::Borrowed`，`Value::Array` 路径返回 `Cow::Owned`
- [x] 3.3 更新所有调用方（约 15 处）适配 `Cow<str>` 返回值：prompt guard、tier router、Anthropic convert、runtime 等

## 4. Event Log 批量写入

- [x] 4.1 在 `EventLog` 中新增 `mpsc::Sender<EventEntry>` 和 batch writer task
- [x] 4.2 实现 writer task：50ms interval 或 buffer 满 64 条时 flush，使用单个 SQLite 事务批量 INSERT
- [x] 4.3 将 `EventLog::append` 从 async SQLite INSERT 改为 `try_send` 到 channel
- [x] 4.4 实现 shutdown flush：在 EventLog Drop 或 graceful shutdown 时 drain buffer
- [x] 4.5 更新 `ws/chat.rs` 和 `session_bridge.rs` 中的 event_log 调用方，移除 `.await`（改为非阻塞 send）

## 5. Stream Path 类型化传递

- [x] 5.1 修改 `SessionOp::UserTurn`：新增 `setup: Option<Arc<ChatSetup>>` 和 `agent_config: Option<Arc<AgentConfig>>` 字段
- [x] 5.2 修改 `handle_stream`（`routes/chat.rs`）：直接将 `Arc::new(setup)` 和 `Arc::new(agent_config)` 放入 UserTurn，移除 `serde_json::to_value` 调用
- [x] 5.3 修改 `session_bridge.rs` 的 UserTurn 处理：从 op 中直接取出 Arc 数据，移除 `serde_json::from_value` 调用
- [x] 5.4 确保 ws/chat.rs 的 WebSocket 路径同步适配（如果也走 SessionOp）

## 6. GC Key 修复

- [x] 6.1 将 `chat_locks` / `chat_cancels` 的 key 从 `chat_id` 统一为 `session_key`
- [x] 6.2 更新 `gc_stale_resources` 中的 retain 逻辑：使用 session_key 与 SessionManager 的 active 集合匹配
- [x] 6.3 更新 `chat_completions` / `handle_stream` 中 insert chat_locks 的 key 生成逻辑
- [x] 6.4 验证 IM 渠道（feishu/wechat）的 session 资源在 GC 后被正确回收
