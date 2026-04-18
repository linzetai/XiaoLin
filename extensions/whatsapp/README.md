# fastclaw-whatsapp

WhatsApp Cloud API 渠道扩展。

## 功能

- **Webhook 验证** — 订阅验证与 HMAC-SHA256 签名校验
- **消息接收** — Webhook 事件解析
- **消息发送** — Graph API 文本消息发送

## 关键导出

```rust
pub use plugin::{WhatsAppPlugin, WhatsAppPluginConfig};
```
