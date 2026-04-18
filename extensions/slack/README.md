# fastclaw-slack

Slack Events API 渠道扩展。

## 功能

- **签名验证** — HMAC-SHA256 Webhook 签名校验
- **事件接收** — Events API 消息与事件解析
- **消息发送** — `chat.postMessage` / `chat.update`

## 关键导出

```rust
pub use plugin::{SlackPlugin, SlackPluginConfig};
```
