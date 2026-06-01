## ADDED Requirements

### Requirement: SSE 流式路由排除 gzip 压缩
流式聊天路由（`/api/v1/chat` 且 `stream=true`）SHALL NOT 经过 gzip `CompressionLayer`，以消除流缓冲导致的首 token 延迟增大。

#### Scenario: 流式请求不被压缩
- **WHEN** 客户端发送 `POST /api/v1/chat` 且 `stream=true`
- **THEN** SSE 响应 MUST NOT 包含 `Content-Encoding: gzip` 头，且每个 SSE chunk 立即发送

#### Scenario: 非流式请求仍被压缩
- **WHEN** 客户端发送 `POST /api/v1/chat` 且 `stream=false`
- **THEN** 响应 MAY 包含 `Content-Encoding: gzip`（若客户端支持）

#### Scenario: 其他 API 路由不受影响
- **WHEN** 客户端请求非 `/api/v1/chat` 的路由
- **THEN** gzip 压缩行为与当前一致
