## 1. Crate 脚手架

- [x] 1.1 创建 `extensions/wechat/Cargo.toml`，声明 dependencies（xiaolin-core, reqwest, serde, tokio, dashmap, aes, ecb, md-5, base64, qr2term, thiserror, tracing）
- [x] 1.2 创建 `extensions/wechat/src/lib.rs`，声明 pub mod 结构
- [x] 1.3 在 workspace `Cargo.toml` 的 members 中添加 `extensions/wechat`
- [x] 1.4 在 `crates/xiaolin-gateway/Cargo.toml` 中添加 `xiaolin-wechat` 依赖
- [x] 1.5 `cargo check` 通过

## 2. API 类型定义

- [x] 2.1 创建 `api/types.rs`：定义 `BaseInfo`, `WeixinMessage`, `MessageItem`, `TextItem`, `ImageItem`, `VoiceItem`, `FileItem`, `VideoItem`, `CDNMedia`, `RefMessage`
- [x] 2.2 定义 `GetUpdatesReq`, `GetUpdatesResp`, `SendMessageReq`, `GetUploadUrlReq`, `GetUploadUrlResp`, `GetConfigResp`, `SendTypingReq`
- [x] 2.3 定义枚举常量：`MessageType` (USER=1, BOT=2), `MessageItemType` (TEXT=1, IMAGE=2, VOICE=3, FILE=4, VIDEO=5), `MessageState` (NEW=0, GENERATING=1, FINISH=2), `TypingStatus` (TYPING=1, CANCEL=2)
- [x] 2.4 定义 QR 登录类型：`QRCodeResponse`, `QRStatusResponse` (status: wait/scanned/confirmed/expired/need_verifycode/scaned_but_redirect/binded_redirect/verify_code_blocked)

## 3. API Client

- [x] 3.1 创建 `api/client.rs`：定义 `WechatApiClient` struct（base_url, token, reqwest::Client）
- [x] 3.2 实现 `build_headers()`：Content-Type, AuthorizationType, Authorization Bearer, X-WECHAT-UIN (base64 random u32), iLink-App-Id, iLink-App-ClientVersion
- [x] 3.3 实现 `build_base_info()`：channel_version, bot_agent（从 config 读取，默认 "XiaoLin"，sanitize UA 格式）
- [x] 3.4 实现 `get_updates(&self, cursor, timeout, cancel) -> Result<GetUpdatesResp>`：长轮询，支持 abort signal
- [x] 3.5 实现 `send_message(&self, msg: SendMessageReq) -> Result<()>`
- [x] 3.6 实现 `get_upload_url(&self, req: GetUploadUrlReq) -> Result<GetUploadUrlResp>`
- [x] 3.7 实现 `get_config(&self, user_id, context_token) -> Result<GetConfigResp>`
- [x] 3.8 实现 `send_typing(&self, req: SendTypingReq) -> Result<()>`
- [x] 3.9 实现 `notify_start(&self) -> Result<()>` 和 `notify_stop(&self) -> Result<()>`
- [x] 3.10 实现 `fetch_qr_code(&self, bot_type) -> Result<QRCodeResponse>` 和 `poll_qr_status(&self, qrcode, verify_code?) -> Result<QRStatusResponse>`

## 4. QR 码登录

- [x] 4.1 创建 `auth/qr_login.rs`：定义 `QrLoginSession` (session_key, qrcode, qr_url, status, started_at)
- [x] 4.2 实现 `start_login(api_client, account_id?) -> QrLoginSession`：请求 QR → 缓存 session
- [x] 4.3 实现 `poll_login(session, verify_code?) -> LoginPollResult`：轮询状态，处理 wait/scanned/confirmed/expired/need_verifycode/scaned_but_redirect/binded_redirect/verify_code_blocked
- [x] 4.4 实现 `wait_for_login(session, timeout) -> LoginResult`：循环 poll 直到 confirmed 或超时，支持 QR 过期自动刷新（最多 3 次）
- [x] 4.5 实现 CLI 展示：`display_qr_terminal(qr_url)` 使用 qr2term 打印二维码 + 备选 URL
- [x] 4.6 处理 IDC 重定向：`scaned_but_redirect` 时切换 polling host

## 5. Credential 持久化

- [x] 5.1 创建 `auth/credential.rs`：定义 `WechatCredential { token, base_url, user_id, cdn_base_url?, created_at }`
- [x] 5.2 实现 `save_credential(account_id, credential)`：写入 `~/.xiaolin-dev/credentials/wechat-{id}.json`
- [x] 5.3 实现 `load_credential(account_id) -> Option<WechatCredential>`
- [x] 5.4 实现 `list_credentials() -> Vec<(String, WechatCredential)>`
- [x] 5.5 实现 `delete_credential(account_id)`

## 6. 消息格式映射

- [x] 6.1 创建 `message.rs`：实现 `weixin_to_inbound(msg: WeixinMessage, channel_id, account_id) -> InboundMessage`
- [x] 6.2 TEXT 映射：`msg_type="text"`, `text=item.text_item.text`
- [x] 6.3 IMAGE/VOICE/FILE/VIDEO 映射：`msg_type="image"/"voice"/"file"/"video"`, 描述性 text, extra 中携带 CDN 引用
- [x] 6.4 多 item 消息合并：多个 item 合并为一条 InboundMessage，text 拼接，extra 携带完整 item_list
- [x] 6.5 实现 `outbound_to_weixin(msg: OutboundMessage, context_token) -> SendMessageReq`：text → TextItem
- [x] 6.6 处理 ref_msg（引用消息）映射

## 7. context_token 管理

- [x] 7.1 创建 context_token cache：`DashMap<(String, String), ContextEntry>` (account_id, peer_id) → { token, updated_at }
- [x] 7.2 实现 `update_token(account_id, peer_id, token)`
- [x] 7.3 实现 `get_token(account_id, peer_id) -> Option<String>`
- [ ] 7.4 实现持久化：save/load 到 `~/.xiaolin-dev/data/wechat-context-tokens.json`
- [ ] 7.5 实现 TTL 清理：定期清除 7 天未活跃的 token

## 8. CDN 媒体加密

- [x] 8.1 创建 `media/crypto.rs`：实现 `aes128_ecb_encrypt(plaintext: &[u8], key: &[u8]) -> Vec<u8>` (PKCS7 padding)
- [x] 8.2 实现 `aes128_ecb_decrypt(ciphertext: &[u8], key: &[u8]) -> Result<Vec<u8>>` (remove PKCS7 padding)
- [x] 8.3 单元测试：roundtrip encrypt/decrypt 验证

## 9. CDN 上传

- [x] 9.1 创建 `media/upload.rs`：实现 `upload_media(api_client, file_path, media_type, to_user_id) -> CDNMedia`
- [x] 9.2 流程：读文件 → 计算 MD5/size → 生成 AES key → 加密 → 计算密文 size → getUploadUrl → PUT CDN → 构造 CDNMedia
- [ ] 9.3 缩略图处理（IMAGE/VIDEO）：生成缩略图 → 同流程加密上传 → thumb_media
- [x] 9.4 构造 `MessageItem` with image_item/file_item/video_item

## 10. CDN 下载

- [x] 10.1 创建 `media/download.rs`：实现 `download_media(cdn_media: CDNMedia, dest_path) -> Result<PathBuf>`
- [x] 10.2 流程：构造 CDN URL → HTTP GET → AES 解密 → 写文件
- [x] 10.3 支持 full_url（新版 API 直接返回完整 URL）

## 11. Typing Indicator

- [x] 11.1 创建 `typing.rs`：`TypingManager` struct (api_client, ticket_cache: DashMap)
- [x] 11.2 实现 `start_typing(account_id, user_id, context_token)`：lazy getConfig → sendTyping(TYPING)
- [x] 11.3 实现 `stop_typing(account_id, user_id)`：sendTyping(CANCEL)
- [x] 11.4 ticket 缓存：按 (account_id, user_id) 缓存 typing_ticket，TTL 10 分钟

## 12. Monitor 长轮询循环

- [x] 12.1 创建 `monitor.rs`：`WechatMonitor` struct（api_client, account_id, inbound_tx, cancel_token）
- [x] 12.2 实现 `run(&self) -> Result<()>` 主循环：getUpdates → convert → inbound_tx.send
- [x] 12.3 cursor 持久化：`get_updates_buf` 写入 `~/.xiaolin-dev/data/wechat-sync-{account_id}.buf`
- [x] 12.4 错误处理：连续 3 次失败 → 30s 退避，errcode=-14 → session expired 暂停
- [x] 12.5 graceful shutdown：cancel_token 触发时停止长轮询
- [x] 12.6 notifyStart/notifyStop 生命周期通知

## 13. WechatPlugin 实现

- [x] 13.1 创建 `plugin.rs`：`WechatPlugin` struct（config, accounts: DashMap, context_tokens, typing_manager）
- [x] 13.2 实现 `ChannelPlugin::meta()` → ChannelMeta { id: "wechat", name: "WeChat", ... }
- [x] 13.3 实现 `ChannelPlugin::capabilities()` → { direct_message: true, media: true, streaming: false }
- [x] 13.4 实现 `ChannelPlugin::connection_mode()` → "longpoll"
- [x] 13.5 实现 `ChannelPlugin::start(inbound_tx)`：为每个已配置账号启动 WechatMonitor
- [x] 13.6 实现 `ChannelPlugin::stop()`：停止所有 monitor，发送 notifyStop
- [x] 13.7 实现 `ChannelPlugin::handle_webhook()`：返回 Ignored（WeChat 不用 webhook）
- [x] 13.8 实现 `ChannelPlugin::send_message(msg)`：outbound_to_weixin → api.send_message
- [x] 13.9 实现 `ChannelPlugin::reply_message(message_id, text)`：带 context_token 回复
- [x] 13.10 实现 `ChannelPlugin::probe()` → ping getConfig 检查连接状态

## 14. 配置与 Gateway 注册

- [x] 14.1 创建 `config.rs`：定义 `WechatChannelConfig` (enabled, accounts, defaultAccount, botAgent, typingEnabled, longPollTimeoutMs)
- [x] 14.2 在 `XiaoLinConfig` 的 channels 解析中支持 `"wechat"` key
- [x] 14.3 在 `build_channels()` 中注册 WechatPlugin
- [ ] 14.4 在 `reload_channel()` 中支持 `"wechat"` 热重载
- [ ] 14.5 在 `SUPPORTED_CHANNELS`（如有）中添加 "wechat"

## 15. Tauri UI 登录 API

- [x] 15.1 新增路由 `POST /api/v1/channels/wechat/login/start` → { session_key, qr_url, message }
- [x] 15.2 新增路由 `GET /api/v1/channels/wechat/login/status/:session_key` → SSE stream
- [x] 15.3 新增路由 `POST /api/v1/channels/wechat/login/verify/:session_key` → 提交配对码
- [x] 15.4 新增路由 `GET /api/v1/channels/wechat/accounts` → 列出所有账号
- [x] 15.5 新增路由 `DELETE /api/v1/channels/wechat/accounts/:account_id` → 注销账号
- [x] 15.6 在 `routes/mod.rs` 中注册以上路由

## 16. CLI 登录命令

- [x] 16.1 在 CLI 的 `channels login` 子命令中添加 `--channel wechat` 支持
- [x] 16.2 实现 CLI 登录流程：start_login → display_qr_terminal → wait_for_login → save_credential → restart hint
- [x] 16.3 实现 `channels list` 中显示 wechat 账号状态

## 17. 单元测试

- [x] 17.1 API types serde 测试：WeixinMessage JSON round-trip
- [x] 17.2 消息映射测试：TEXT / IMAGE / VOICE / FILE / VIDEO → InboundMessage
- [x] 17.3 outbound 映射测试：OutboundMessage → SendMessageReq
- [x] 17.4 AES-128-ECB 加解密 roundtrip 测试
- [ ] 17.5 context_token cache CRUD 测试
- [x] 17.6 bot_agent sanitize 测试
- [ ] 17.7 QR login 状态机测试（mock API responses）

## 18. 集成测试

- [ ] 18.1 Plugin 注册测试：WechatPlugin 正确注册到 ChannelRegistry
- [ ] 18.2 Monitor → InboundMessage 流转测试（mock HTTP server）
- [ ] 18.3 OutboundMessage → sendMessage API 调用测试
- [x] 18.4 `cargo clippy -- -D warnings` 零警告
- [x] 18.5 `cargo test -p xiaolin-wechat` 全部通过

## 19. 验证

- [x] 19.1 全量 `cargo test` 通过 (pre-existing failures in linux-sandbox & 2 LLM-dependent e2e tests excluded)
- [x] 19.2 `cargo clippy -- -D warnings` 零警告
- [x] 19.3 `npx tsc --noEmit` 通过
- [ ] 19.4 端到端：CLI 扫码登录 → 微信发消息 → agent 回复 → 微信收到回复
- [ ] 19.5 端到端：Tauri UI 扫码登录 → 同上验证
