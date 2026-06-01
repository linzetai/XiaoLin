## 1. SSE 路由排除 gzip 压缩

- [x] 1.1 在 `lib.rs` 中将 `/api/v1/chat` 路由拆分到独立 Router（不含 CompressionLayer）
- [x] 1.2 验证流式请求响应不含 `Content-Encoding: gzip`，非流式和其他路由正常压缩

## 2. ContentDelta 预序列化字节

- [x] 2.1 在 `AgentEvent::ContentDelta` 中新增 `raw_bytes: Option<bytes::Bytes>` 字段（serde skip）
- [x] 2.2 在 runtime `StreamDelta` 解析处，保存原始 SSE data 行为 `Bytes` 并填入 `raw_bytes`
- [x] 2.3 修改 HTTP SSE 格式化逻辑：优先使用 `raw_bytes` 构建输出，None 时降级 `to_string`
- [x] 2.4 编译验证，确保 WS 路径和 event_log 不受影响

## 3. HTTP UserTurn 跳过冗余 messages 序列化

- [x] 3.1 在 `handle_stream` 中，当 `typed_data` 已设置时，`messages` 字段传空 `Value::Array(vec![])`
- [x] 3.2 确认 session actor / bridge 侧 `typed_data` 优先逻辑正确处理空 messages

## 4. WebSocket UserTurn 对齐 TypedTurnData

- [x] 4.1 在 WS `spawn_chat` 中使用 `TypedTurnData::wrap` 设置 `typed_data`
- [x] 4.2 移除 `extra` 中 `_enriched_request` 和 `_agent_config` 的 `to_value` 序列化
- [x] 4.3 编译验证并测试 WS 聊天功能正常

## 5. 集成验证

- [x] 5.1 cargo check 全量编译通过
- [x] 5.2 启动 tauri dev，通过 HTTP 和 WS 各发一轮聊天，检查 perf 日志对比优化前后
