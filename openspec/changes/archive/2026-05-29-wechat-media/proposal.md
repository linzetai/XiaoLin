## Why

微信 channel 目前仅支持纯文本消息的收发。当 agent 需要向用户发送图片（如生成的图表、截图）或文件（如代码补丁、导出文件）时，无法通过微信传递——只能发送纯文本或 URL 链接。这极大地限制了微信 channel 的实用性，尤其是在 agent 使用 `image_generate`、`screenshot`、文件编辑等工具后需要将结果直接推送给用户的场景。

openclaw-weixin 已有完整的媒体上传和发送实现（CDN + AES-128-ECB 加密），我们需要在 Rust 端实现同等能力。

## What Changes

- 新增 **CDN 媒体上传管线**：实现 `getUploadUrl` → AES-128-ECB 加密 → CDN PUT 上传 → 获取下载参数的完整流程
- 新增 **图片/文件/视频消息发送**：扩展 `sendMessage` 支持 `image_item`、`file_item`、`video_item` 类型的 `MessageItem`
- 扩展 **`OutboundMessage`**：增加 `attachments` 字段支持文件路径列表
- 扩展 **`outbound_to_weixin`**：根据文件 MIME 类型路由到对应的媒体消息格式
- 新增 **`WechatApiClient` 方法**：`get_upload_url`、`upload_to_cdn`
- 接收侧：解析 inbound 消息中的 `image_item` / `file_item` 等媒体字段，下载到本地后传递给 agent

## Capabilities

### New Capabilities
- `media-upload`: CDN 媒体上传管线（AES-128-ECB 加密、getUploadUrl、CDN PUT）
- `media-send`: 图片/文件/视频消息的构建与发送
- `media-receive`: 接收侧媒体解析与下载

### Modified Capabilities

## Impact

- `extensions/wechat/src/api/client.rs` — 新增 `get_upload_url`、`upload_to_cdn` 方法
- `extensions/wechat/src/api/types.rs` — 新增 `ImageItem`、`FileItem`、`VideoItem`、`CDNMedia` 等类型
- `extensions/wechat/src/message.rs` — 扩展 `outbound_to_weixin` 支持媒体附件
- `extensions/wechat/src/plugin.rs` — `send_message` / `reply_message` 支持带附件的消息
- `crates/xiaolin-core/src/channel.rs` — `OutboundMessage` 增加 `attachments` 字段
- 新增依赖：`aes`、`ecb`、`md-5` crate（AES-128-ECB 加密 + MD5 校验）
- `crates/xiaolin-gateway/src/routes/channel.rs` — agent 输出中的文件路径提取逻辑
