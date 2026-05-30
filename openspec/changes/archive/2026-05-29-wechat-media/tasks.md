## 1. 基础类型与依赖

- [x] 1.1 在 `extensions/wechat/Cargo.toml` 中添加 `aes`、`cipher`、`md-5` crate 依赖
- [x] 1.2 在 `extensions/wechat/src/api/types.rs` 中新增 `ImageItem`、`FileItem`、`VideoItem`、`CDNMedia`、`GetUploadUrlReq`、`GetUploadUrlResp` 类型定义
- [x] 1.3 在 `extensions/wechat/src/api/types.rs` 中扩展 `MessageItem` 枚举支持 `image_item`、`file_item` 字段
- [x] 1.4 在 `crates/fastclaw-core/src/channel.rs` 中为 `OutboundMessage` 新增 `attachments: Vec<Attachment>` 字段和 `Attachment` 结构体
- [x] 1.5 在 `crates/fastclaw-core/src/channel.rs` 中为 `InboundMessage` 新增 `attachments: Vec<Attachment>` 字段

## 2. CDN 上传管线

- [x] 2.1 新建 `extensions/wechat/src/media/mod.rs` 模块入口（命名为 `media/` 而非 `cdn/`）
- [x] 2.2 实现 `extensions/wechat/src/media/crypto.rs`：AES-128-ECB PKCS7 加密 + 解密函数
- [x] 2.3 在 `extensions/wechat/src/api/client.rs` 中新增 `get_upload_url` 方法
- [x] 2.4 实现 `extensions/wechat/src/media/upload.rs`：完整上传管线 `upload_media` — 读取文件 → MD5 → 生成 AES key → getUploadUrl → 加密 → PUT CDN → 返回 `UploadedFileInfo`
- [x] 2.5 实现 CDN URL 解析逻辑：优先 `upload_full_url`，回退到 `cdn_base_url + upload_param`

## 3. 发送侧媒体支持

- [x] 3.1 实现 MIME 类型推断函数 `mime_from_extension(path) -> &str`
- [x] 3.2 实现 `build_image_item(cdn_media) -> MessageItem`
- [x] 3.3 实现 `build_file_item(cdn_media, file_name, raw_size, md5) -> MessageItem`
- [x] 3.4 新增 `outbound_to_weixin_with_media` 异步函数：遍历 `OutboundMessage.attachments`，对每个附件调用上传管线 + 构建对应的 MessageItem
- [x] 3.5 在 `WechatPlugin::send_message` 中根据 attachments 是否为空选择调用 `outbound_to_weixin` 或 `outbound_to_weixin_with_media`
- [x] 3.6 在 `WechatChannelConfig` 中新增 `cdn_base_url` 配置项

## 4. 接收侧媒体支持

- [x] 4.1 实现 `extensions/wechat/src/media/download.rs`：CDN 下载 + AES-128-ECB 解密 → 本地临时文件
- [x] 4.2 新增 `enrich_inbound_media` 异步函数：解析 `item_list` 中的 `image_item` / `file_item`，下载并填充 `InboundMessage.attachments`
- [x] 4.3 实现临时文件清理：在 `WechatPlugin::start` 中删除 `~/.fastclaw-dev/data/wechat-media/` 下超过 24 小时的文件

## 5. 集成与验证

- [x] 5.1 `cargo clippy -p fastclaw-wechat -p fastclaw-core -- -D warnings` 零警告
- [ ] 5.2 端到端测试：agent 生成图片 → 通过微信 channel 发送给用户
- [ ] 5.3 端到端测试：用户发送图片到微信 bot → agent 能识别并描述图片内容
