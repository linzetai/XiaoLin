## Why

流式聊天热路径中，每个 token chunk 经历三重 JSON 序列化（LLM parse → `to_value` → SSE `to_string` + `format!`），SSE 响应被 gzip 压缩层缓冲增大首 token 延迟，WebSocket 路径未对齐 TypedTurnData 仍在全量序列化 `ChatRequest`。这些网络出入口层的冗余处理在高并发/长 session 场景下产生可观的 CPU 和延迟开销。

## What Changes

- 从 SSE 流式路由排除 gzip 压缩层，消除流缓冲导致的首 token 延迟
- 消除 ContentDelta 链路上的中间 `serde_json::Value` 层，实现 LLM chunk → SSE 字节的接近零拷贝转发
- WebSocket `UserTurn` 提交路径对齐 HTTP，使用 `TypedTurnData` 避免全量 JSON 序列化
- HTTP `UserTurn` 在 `typed_data` 存在时跳过冗余的 `to_value(messages)` 序列化

## Capabilities

### New Capabilities
- `sse-bypass-gzip`: SSE 流式响应绕过全局 gzip 压缩层，消除流缓冲延迟
- `delta-fast-path`: ContentDelta 事件使用预序列化字节直通，消除中间 Value 层
- `ws-typed-turn`: WebSocket 路径使用 TypedTurnData 提交 UserTurn，与 HTTP 对齐

### Modified Capabilities

## Impact

- `crates/fastclaw-gateway/src/lib.rs` — 压缩中间件配置
- `crates/fastclaw-gateway/src/routes/chat.rs` — SSE 事件格式化
- `crates/fastclaw-gateway/src/ws/chat.rs` — WS UserTurn 提交
- `crates/fastclaw-agent/src/runtime/mod.rs` — ContentDelta 事件构建
- `crates/fastclaw-agent/src/llm.rs` — StreamDelta 解析优化
- `crates/fastclaw-protocol/` — AgentEvent::ContentDelta 类型可能调整
