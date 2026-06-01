## ADDED Requirements

### Requirement: msg_cache 使用 Arc 共享消息历史
SessionStore 的 msg_cache SHALL 使用 `Arc<Vec<ChatMessage>>` 作为缓存值类型。`load_chat_messages` 在缓存命中时 SHALL 返回 `Arc::clone` 而非深拷贝整个 Vec。

#### Scenario: 缓存命中时零拷贝读取
- **WHEN** 调用 `load_chat_messages` 且 session_id 在 msg_cache 中存在
- **THEN** 返回 `Arc<Vec<ChatMessage>>` 的 clone（仅增加引用计数），不对消息内容进行深拷贝

#### Scenario: 缓存命中的内存开销
- **WHEN** 同一 session 的消息历史被 3 个并发调用方同时持有
- **THEN** 内存中仅存在 1 份消息数据，3 个 Arc 指针共享同一份底层数据

### Requirement: 写入时 copy-on-write
`append_message` 和 `append_messages` SHALL 使用 `Arc::make_mut` 实现写入。当缓存值仅被缓存自身持有时直接原地修改；当有其他持有者时先 clone 再修改。

#### Scenario: 独占写入（无外部持有者）
- **WHEN** 调用 `append_message` 且当前 Arc 的 strong count 为 1
- **THEN** 直接在现有 Vec 上 push，不产生额外分配

#### Scenario: 共享写入（有外部持有者）
- **WHEN** 调用 `append_message` 且当前 Arc 的 strong count > 1
- **THEN** 先 clone Vec 到新的 Arc，再 push 新消息；外部持有者看到的仍是旧版本

### Requirement: token 估算不触发 JSON 序列化
`estimate_single_message_tokens` SHALL 直接遍历 `serde_json::Value` 树计算字符数，不调用 `serde_json::to_string`。

#### Scenario: String 类型 content
- **WHEN** `ChatMessage.content` 为 `Value::String(s)`
- **THEN** 返回 `s.len()` 作为字符数，无堆分配

#### Scenario: Array 类型 content（multimodal）
- **WHEN** `ChatMessage.content` 为 `Value::Array` 包含多个 content part
- **THEN** 递归遍历数组中每个元素的文本字段，累加字符数，无堆分配

### Requirement: text_content 返回 Cow
`ChatMessage::text_content()` SHALL 返回 `Option<Cow<'_, str>>` 而非 `Option<String>`。

#### Scenario: 纯文本消息
- **WHEN** `content` 为 `Value::String(s)`
- **THEN** 返回 `Some(Cow::Borrowed(s.as_str()))`，零分配

#### Scenario: multimodal 消息
- **WHEN** `content` 为 `Value::Array` 且包含多个 text part
- **THEN** 返回 `Some(Cow::Owned(joined_text))`，仅在 join 时分配一次
