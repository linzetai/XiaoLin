## Context

FastClaw 的流式聊天热路径中，每个 token chunk 经历多次 JSON 序列化/反序列化。当前数据流：

```
LLM SSE bytes → parse StreamDelta → to_value(delta) → AgentEvent::ContentDelta
→ mpsc channel → to_string(delta) → format!("data: {}\n\n") → HTTP SSE / WS
```

此外，全局 gzip `CompressionLayer` 作用于包括 SSE 在内的所有路由，可能缓冲流数据。WebSocket 路径仍通过 `serde_json::to_value` 序列化 `ChatRequest` 和 `AgentConfig`，未使用已有的 `TypedTurnData` 机制。

上一轮 `optimize-chat-hot-path` 已优化了内存、缓存、token 估算和 EventLog 批写入。本轮聚焦网络出入口层的 CPU 和延迟。

## Goals / Non-Goals

**Goals:**
- 消除 SSE gzip 缓冲对首 token 延迟的影响
- 减少 ContentDelta 热路径上的 JSON 序列化次数（3 次 → ≤1 次）
- WebSocket UserTurn 提交与 HTTP 对齐，使用 TypedTurnData
- HTTP UserTurn 在 typed_data 存在时跳过冗余 messages 序列化

**Non-Goals:**
- 不改变 `ChatMessage.content` 的 `serde_json::Value` 类型（大 refactor）
- 不改变 LLM provider 的请求体构建方式（已是直接序列化）
- 不改变 EventLog 的序列化策略（已 batch 化）
- 不实现 LLM SSE 字节的完全透传（需统一多 provider 格式，架构改动过大）

## Decisions

### D1: SSE 路由排除 gzip — 使用 axum 路由层级隔离

**方案**: 将 `/api/v1/chat` 路由放入独立的 `Router`，不包含 `CompressionLayer`，其余路由保持 gzip。

**替代方案**: 按 `Content-Type: text/event-stream` 动态跳过压缩 → 需要自定义中间件，复杂度高。

**理由**: axum 路由可嵌套且各自有独立的 layer 栈，简单可靠。

### D2: ContentDelta 预序列化 — 在 runtime 侧直接持有 JSON 字节

**方案**: `AgentEvent::ContentDelta` 新增 `raw_bytes: Option<bytes::Bytes>` 字段。runtime 在 parse `StreamDelta` 后，将原始 SSE data 行保存为 `Bytes`，同时保留结构化 `delta: Value` 供需要语义访问的路径使用。gateway SSE 格式化时优先使用 `raw_bytes`，避免 `to_string` + `format!`。

**替代方案 A**: 完全用 `Bytes` 替代 `Value` → 破坏需要语义访问的下游（event_log、WS 路由、工具调度判断）。

**替代方案 B**: 在 gateway 侧用 `serde_json::to_writer` 写入复用 buffer → 仍需一次序列化，只是减少分配。

**理由**: 保持向后兼容，SSE 热路径零序列化，其他路径不受影响。

### D3: WS TypedTurnData — 复用现有 `TypedTurnData::wrap`

**方案**: WS `spawn_chat` 中，构建 `SessionOp::UserTurn` 时使用 `TypedTurnData::wrap(enriched_request, config)` 设置 `typed_data` 字段，与 HTTP `handle_stream` 对齐。

**理由**: 代码已有基础设施，只需 WS 侧调用即可。

### D4: HTTP UserTurn 跳过冗余 messages — 条件化 `to_value`

**方案**: 在 `handle_stream` 中，当 `typed_data` 已设置时，`SessionOp::UserTurn.messages` 传空 `Value::Array(vec![])`，session actor 侧从 `typed_data` 获取消息。

**约束**: 需确保 `SessionOp` 的 JSON 序列化（用于持久化/日志）不依赖 `messages` 字段的完整性，或有降级路径。

## Risks / Trade-offs

- **[Risk] `raw_bytes` 内存增长**: ContentDelta 同时持有 `delta` 和 `raw_bytes` → **Mitigation**: `raw_bytes` 是 `Bytes`（引用计数），生命周期短（传完即 drop），峰值内存增量 < 1KB/chunk
- **[Risk] WS 路径 `forward_event` 仍需 `delta` Value**: WS 不用 `raw_bytes` → **Mitigation**: 保留 `delta` 字段，WS 路径行为不变
- **[Risk] SSE 排除 gzip 增大带宽**: → **Mitigation**: SSE delta 本身很小（~100-500 bytes/chunk），gzip 收益微乎其微，延迟收益远大于带宽成本
- **[Risk] 空 messages 影响日志/调试**: → **Mitigation**: `typed_data` 存在时日志改为打印 `typed_data` 摘要（message count 等）
