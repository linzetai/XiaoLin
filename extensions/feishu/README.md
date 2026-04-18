# fastclaw-feishu

飞书 / Lark 渠道扩展。

## 功能

- **Webhook 接入** — 事件回调验证（Challenge）与消息解析
- **WebSocket 长连接** — 飞书 WS 协议，实时接收事件
- **消息发送** — 文本、富文本、卡片消息的发送与回复
- **OAuth 客户端** — 自动管理 tenant_access_token
- **Protobuf 支持** — 飞书 WS 二进制事件解析
- **飞书工具** — 专属工具集（如卡片构建等）

## 关键导出

```rust
pub use channel::FeishuChannel;
pub use core::FeishuPlugin;
pub use core::FeishuClient;
```
