## Context

当前 FastClaw 微信 channel 仅支持纯文本消息。微信后端 API 通过 CDN + AES-128-ECB 加密传输所有媒体文件。openclaw-weixin（TypeScript）已有完整实现，我们需要在 Rust 端实现等效能力。

核心挑战：
- 所有媒体必须先加密上传到微信 CDN，才能在消息中引用
- CDN 上传协议使用 AES-128-ECB 加密 + MD5 校验
- 不同媒体类型（图片/视频/文件）有不同的 `MessageItem` 结构
- 需要处理接收侧媒体的下载和解密

## Goals / Non-Goals

**Goals:**
- 支持发送图片（agent 生成的图表、截图等）
- 支持发送文件（代码文件、导出文件等）
- 支持接收图片/文件并传递给 agent
- 与 openclaw-weixin 的 CDN 协议完全兼容

**Non-Goals:**
- 视频发送（初期不实现，后续可增）
- 语音消息（SILK 编码复杂度高，暂不实现）
- 缩略图生成（初期使用 `no_need_thumb: true` 跳过）
- 富文本/卡片消息格式

## Decisions

### 1. 媒体上传管线架构

**决定**: 新增 `extensions/wechat/src/cdn/` 模块，包含 `aes_ecb.rs`（加密）、`upload.rs`（上传管线）。

**理由**: 将 CDN 相关逻辑独立成模块，与消息构建（`message.rs`）和 API 客户端（`api/client.rs`）解耦。openclaw-weixin 也采用相同的 `src/cdn/` 分离结构。

**替代方案**: 全部放在 `api/client.rs` 中 — 拒绝，因为 CDN 上传涉及加密、分步网络请求，逻辑复杂度高，混在 API client 中会降低可维护性。

### 2. AES-128-ECB 实现

**决定**: 使用 `aes` + `cipher` crate 实现 PKCS7 padding 的 AES-128-ECB 加密。

**理由**: `aes` 是 Rust 生态标准的 AES 实现（RustCrypto 项目），`cipher` 提供 BlockEncrypt trait。ECB 模式虽不安全（不应用于常规加密），但这里是微信 CDN 协议强制要求。

### 3. CDN 上传 URL 来源

**决定**: 优先使用 `getUploadUrl` 返回的 `upload_full_url`；若无则从 `upload_param` 提取并拼接 CDN base URL。

**理由**: 与 openclaw-weixin 的 `uploadBufferToCdn` 逻辑一致。CDN base URL 作为配置项存储在 `WechatChannelConfig` 中。

### 4. OutboundMessage 附件设计

**决定**: 在 `OutboundMessage` 中新增 `attachments: Vec<Attachment>` 字段，`Attachment` 包含 `file_path: String` 和 `mime_type: Option<String>`。

**理由**: 通用设计，不绑定微信特定的字段（如 `aes_key`）。各 channel plugin 在 `send_message` / `reply_message` 中根据自身需求处理附件。

**替代方案**: 直接在 OutboundMessage 中放 `image_key` 等微信特定字段 — 拒绝，破坏 channel 抽象层的通用性。

### 5. 接收侧媒体处理

**决定**: 接收到带 `image_item` / `file_item` 的消息时，使用 CDN URL + AES key 下载并解密到本地临时文件，将本地路径作为 `InboundMessage.attachments` 传递给 agent。

**理由**: agent 工具链（如 `read_file`）操作本地文件，不能直接使用加密的 CDN URL。下载到临时目录 `~/.fastclaw-dev/data/wechat-media/` 并设置 TTL 清理。

### 6. MIME 类型路由

**决定**: 根据文件扩展名推断 MIME 类型，路由到 `IMAGE`(type=2) 或 `FILE`(type=4)。

**理由**: 与 openclaw-weixin 的 `getMimeFromFilename` 一致。`image/*` → `image_item`，其他 → `file_item`。

## Risks / Trade-offs

- **[CDN 协议变更]** → 微信 CDN 上传协议可能变更。缓解：模块化设计，CDN 逻辑集中在 `cdn/` 模块中，易于适配。
- **[大文件内存占用]** → 当前实现将整个文件读入内存加密。缓解：初期限制文件大小（如 20MB），后续可实现分块加密上传。
- **[临时文件积累]** → 下载的媒体文件可能积累。缓解：启动时清理超过 24h 的临时文件。
- **[ECB 模式安全性]** → AES-ECB 存在已知弱点。缓解：这是微信 CDN 协议强制要求，非我们的选择。
