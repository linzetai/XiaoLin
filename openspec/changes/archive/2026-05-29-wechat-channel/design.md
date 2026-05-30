# WeChat Channel — Architecture Design

## Module Structure

```
extensions/wechat/
├── Cargo.toml
└── src/
    ├── lib.rs                  # pub mod 声明 + re-export
    ├── plugin.rs               # WechatPlugin: impl ChannelPlugin
    ├── config.rs               # WechatChannelConfig, AccountCredential
    ├── api/
    │   ├── mod.rs
    │   ├── client.rs           # WechatApiClient (HTTP JSON)
    │   └── types.rs            # WeixinMessage, MessageItem, CDNMedia, etc.
    ├── auth/
    │   ├── mod.rs
    │   ├── qr_login.rs         # QR 码登录流程 (start + poll)
    │   └── credential.rs       # Token 持久化 + 加载
    ├── monitor.rs              # Long-poll 循环 → InboundMessage
    ├── message.rs              # WeixinMessage ↔ InboundMessage/OutboundMessage 映射
    ├── media/
    │   ├── mod.rs
    │   ├── crypto.rs           # AES-128-ECB 加解密
    │   ├── upload.rs           # CDN 上传流程
    │   └── download.rs         # CDN 下载 + 解密
    └── typing.rs               # Typing indicator (getConfig + sendTyping)
```

## Architecture Overview

```
                          ┌───────────────────────────────────┐
                          │     WeChat Backend (腾讯)          │
                          │  ilinkai.weixin.qq.com             │
                          └──────────┬────────────────────────┘
                                     │  HTTP JSON
                                     │
    ┌────────────────────────────────┼────────────────────────────────┐
    │                    WechatPlugin                                  │
    │                                                                  │
    │  ┌──────────────┐   ┌──────────────┐   ┌──────────────────┐    │
    │  │ QR Login     │   │ Monitor      │   │ WechatApiClient  │    │
    │  │ (auth/)      │   │ (long-poll)  │   │ (api/client.rs)  │    │
    │  └──────┬───────┘   └──────┬───────┘   └──────┬───────────┘    │
    │         │                  │                    │                │
    │         │                  │ InboundMessage     │ OutboundMessage│
    │         │                  ▼                    ▲                │
    │         │           ┌──────────────┐     ┌─────┴──────────┐    │
    │         │           │ message.rs   │     │  message.rs    │    │
    │         │           │ weixin→fclaw │     │  fclaw→weixin  │    │
    │         │           └──────┬───────┘     └────────────────┘    │
    │         │                  │                                    │
    │         │                  │ inbound_tx.send()                  │
    └─────────┼──────────────────┼────────────────────────────────────┘
              │                  │
              │                  ▼
    ┌─────────┼──────────────────────────────────────────────────────┐
    │         │            Gateway Pipeline                          │
    │         │  spawn_inbound_dispatcher → handle_channel_message   │
    │         │  → session → agent pipeline → reply_message          │
    │         │                                                      │
    │         │  ┌──────────────────────────┐                        │
    │         └──┤ CLI / Tauri UI           │                        │
    │            │ channels login --channel  │                        │
    │            │ wechat                    │                        │
    │            └──────────────────────────┘                        │
    └────────────────────────────────────────────────────────────────┘
```

## Key Design Decisions

### D1: Long-poll 架构

WeChat 使用 `getUpdates` 长轮询而不是 WebSocket。实现 `ChannelPlugin::start()` 时启动一个后台 tokio task 运行 monitor 循环：

```
loop {
    resp = api.get_updates(cursor, timeout).await
    if aborted → break
    if error → retry with backoff
    for msg in resp.msgs {
        inbound_tx.send(convert_to_inbound(msg))
    }
    cursor = resp.get_updates_buf
    persist_cursor(cursor)
}
```

cursor（`get_updates_buf`）持久化到磁盘，重启后从上次位置恢复。

### D2: QR 码登录双通道

**CLI 通道**：
- 终端打印 QR（用 `qr2term` crate）
- 同时打印 URL 链接作为备选
- 轮询 `get_qrcode_status` 直到 confirmed
- 支持 `need_verifycode`（配对码）流程
- 支持 `scaned_but_redirect`（IDC 重定向）
- 支持 QR 过期自动刷新（最多 3 次）

**Tauri UI 通道**：
- HTTP API: `POST /api/v1/channels/wechat/login/start` → 返回 QR URL
- HTTP API: `GET /api/v1/channels/wechat/login/status/:session` → SSE 事件流
- 前端展示二维码图片，实时更新状态（等待扫码 → 已扫码 → 需要配对码 → 已确认）
- 完成后自动 reload channel

### D3: 消息格式映射

**Inbound（WeChat → FastClaw）**：

| WeixinMessage.item_list.type | InboundMessage.msg_type | InboundMessage.text |
|------------------------------|--------------------------|---------------------|
| 1 (TEXT)                     | "text"                  | item.text_item.text |
| 2 (IMAGE)                   | "image"                 | "[图片]"            |
| 3 (VOICE)                   | "voice"                 | voice.text 或 "[语音]" |
| 4 (FILE)                    | "file"                  | "[文件: {filename}]" |
| 5 (VIDEO)                   | "video"                 | "[视频]"            |

多 item 消息合并为一条 InboundMessage，extra 中携带原始 item_list 和 CDN 引用。

**Outbound（FastClaw → WeChat）**：

```rust
OutboundMessage { text, image_key, .. }
    → SendMessageReq { msg: WeixinMessage {
        to_user_id: target_id,
        context_token: cached_token,
        item_list: vec![MessageItem { type: 1, text_item: TextItem { text } }]
    }}
```

image/file 发送：先 getUploadUrl → AES 加密 → CDN PUT → 构造 CDNMedia → sendMessage

### D4: context_token 管理

微信每条 inbound 消息携带 `context_token`，回复时必须带上。

```rust
/// Per-account mapping: peer_id → latest context_token
struct ContextTokenCache {
    tokens: DashMap<(String, String), String>,  // (account_id, peer_id) → token
}
```

- 每收到一条消息，更新 cache
- 回复时从 cache 取 token
- 持久化到磁盘（JSON），restart 后恢复
- TTL 过期清理（比如 7 天未活跃的清除）

### D5: CDN 媒体加密

微信使用 AES-128-ECB 加密 CDN 传输：

**上传流程**：
1. 生成随机 AES-128 key
2. 计算原文件 MD5、大小
3. AES-128-ECB 加密文件（PKCS7 padding）
4. 计算密文大小
5. `getUploadUrl(filekey, media_type, rawsize, rawfilemd5, filesize, aeskey)` → `upload_param`
6. HTTP PUT 密文到 CDN URL
7. 构造 `CDNMedia { encrypt_query_param, aes_key }` 放入 MessageItem

**下载流程**：
1. 用 `encrypt_query_param` 构造 CDN 下载 URL
2. HTTP GET 下载密文
3. AES-128-ECB 解密（用 `aes_key`）
4. 去 PKCS7 padding

### D6: Typing Indicator

```
user sends message
    → getConfig(ilink_user_id, context_token) → typing_ticket
    → sendTyping(ilink_user_id, typing_ticket, status=1)  // 开始输入
    → ... agent processing ...
    → sendTyping(status=2)  // 取消输入
    → sendMessage(reply)
```

typing_ticket 按用户缓存，有 TTL（约 10 分钟），过期后重新 getConfig。

### D7: 多账号支持

```rust
struct WechatPlugin {
    accounts: DashMap<String, WechatAccount>,
    // account_id → { token, base_url, user_id, monitor_handle }
}
```

- 每个账号独立的 monitor 循环
- 每个账号独立的 context_token cache
- Session key: `wechat:{account_id}:{peer_id}`（对应 dmScope `per-account-channel-peer`）
- 账号增删通过 `start()/stop()` 或 CLI login 管理

### D8: 错误处理和断线重连

| 场景 | 策略 |
|------|------|
| getUpdates 网络超时 | 正常，立即重试 |
| getUpdates ret≠0 | 重试，3 次失败后 30s 退避 |
| errcode=-14 (session expired) | 暂停该账号，需要重新扫码 |
| sendMessage 失败 | 返回错误给 gateway，不重试 |
| CDN 上传失败 | 返回错误，不重试 |
| QR 过期 | 自动刷新（最多 3 次） |

### D9: 配置结构

```json
{
  "channels": {
    "wechat": {
      "enabled": true,
      "connectionMode": "longpoll",
      "accounts": {
        "default": {
          "token": "...",
          "baseUrl": "https://ilinkai.weixin.qq.com",
          "userId": "..."
        }
      },
      "defaultAccount": "default",
      "botAgent": "FastClaw/0.0.6",
      "typingEnabled": true,
      "longPollTimeoutMs": 35000
    }
  }
}
```

Credentials 独立存储在 `~/.fastclaw-dev/credentials/wechat-{account_id}.json`（不进 config 主文件）。

### D10: Gateway 注册

在 `build_channels()` 中：

```rust
if let Some(wechat_config) = config.channels.get("wechat") {
    if wechat_config.enabled {
        let wechat_plugin = WechatPlugin::new(wechat_config)?;
        wechat_plugin.start(inbound_tx.clone()).await?;
        channel_registry.register(Arc::new(wechat_plugin));
    }
}
```

在 `reload_channel()` 中支持 `"wechat"` 的热重载。

### D11: Tauri UI 登录 API

新增 HTTP endpoints：

```
POST /api/v1/channels/wechat/login/start
  → { session_key, qr_url, message }

GET  /api/v1/channels/wechat/login/status/:session_key
  → SSE stream: { status: "waiting"|"scanned"|"need_verifycode"|"confirmed"|"expired", ... }

POST /api/v1/channels/wechat/login/verify/:session_key
  → { code: "1234" }  // 提交配对码

GET  /api/v1/channels/wechat/accounts
  → [{ account_id, name, enabled, configured, last_activity }]

DELETE /api/v1/channels/wechat/accounts/:account_id
  → 注销该账号
```

## Dependencies

### New Rust crate dependencies

- `aes` + `ecb` — AES-128-ECB 加密/解密
- `qr2term` — CLI 终端 QR 码显示
- `md-5` — MD5 哈希（CDN 上传需要）
- `base64` (already in workspace)
- `reqwest` (already in workspace)
- `dashmap` (already in workspace)

### Existing infrastructure reused

- `ChannelPlugin` trait
- `InboundMessage` / `OutboundMessage`
- `ChannelRegistry`
- `handle_channel_message` pipeline
- Session management (dmScope)
- `inbound_tx` / `spawn_inbound_dispatcher`
