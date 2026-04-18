# fastclaw-session

会话持久化层，基于 SQLite WAL 模式存储会话、消息与元数据。

## 功能

- **会话 CRUD** — 创建、查询、删除会话
- **消息存储** — 按会话存储对话消息（含 tool_calls 和 tool 结果）
- **TTL 清理** — 过期会话自动回收
- **上下文压缩钩子** — 与 `fastclaw-context` 协作的压缩接口
- **会话级 `work_dir`** — 每个会话可关联独立工作目录

## 关键导出

```rust
pub use models::{Session, SessionMessage, SessionSummary, SessionCreateOutcome};
pub use store::SessionStore;
```
