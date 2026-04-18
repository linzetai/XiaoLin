# fastclaw-matrix

Matrix 渠道扩展。

## 功能

- **客户端同步** — Matrix client sync 长轮询
- **Appservice 模式** — Webhook 事件接收
- **消息发送** — `m.room.message` 发送，支持回复

## 关键导出

```rust
pub use plugin::{MatrixPlugin, MatrixPluginConfig};
```
