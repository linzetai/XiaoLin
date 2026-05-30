# WeChat Media Pipeline Spec

## 概述

微信的媒体传输使用 AES-128-ECB 加密的 CDN 系统。上传和下载都需要加解密。

## 媒体类型

| Type | 值 | 描述 | 缩略图 |
|------|-----|------|--------|
| IMAGE | 1 | 图片 (jpg/png/gif/webp) | 必须 |
| VIDEO | 2 | 视频 (mp4 等) | 必须 |
| FILE | 3 | 通用文件 | 不需要 |
| VOICE | 4 | 语音 (SILK 编码) | 不需要 |

## AES-128-ECB 加密

### 算法

- **算法**: AES-128-ECB
- **Key**: 随机 16 字节
- **Padding**: PKCS7
- **Key 传输**: Base64 编码

### Rust 实现

```rust
use aes::cipher::{BlockEncrypt, BlockDecrypt, KeyInit};
use aes::Aes128;

fn aes128_ecb_encrypt(plaintext: &[u8], key: &[u8; 16]) -> Vec<u8> {
    let cipher = Aes128::new(key.into());
    let padded = pkcs7_pad(plaintext, 16);
    let mut output = padded.clone();
    for chunk in output.chunks_exact_mut(16) {
        let block = aes::Block::from_mut_slice(chunk);
        cipher.encrypt_block(block);
    }
    output
}

fn aes128_ecb_decrypt(ciphertext: &[u8], key: &[u8; 16]) -> Result<Vec<u8>> {
    let cipher = Aes128::new(key.into());
    let mut output = ciphertext.to_vec();
    for chunk in output.chunks_exact_mut(16) {
        let block = aes::Block::from_mut_slice(chunk);
        cipher.decrypt_block(block);
    }
    pkcs7_unpad(&output)
}
```

### 密文大小计算

```
filesize = ceil(rawsize / 16) * 16
```

即 PKCS7 padding 后按 16 字节对齐。如果 `rawsize` 是 16 的倍数，密文比明文多 16 字节（一个完整的 padding block）。

## 上传流程

```
1. 读取文件
   │
2. 计算 rawsize, rawfilemd5 = hex(MD5(plaintext))
   │
3. 生成 16 字节随机 AES key
   │
4. AES-128-ECB 加密文件
   │
5. 计算 filesize = len(ciphertext)
   │
6. [IMAGE/VIDEO] 生成缩略图，重复 2-5 得到 thumb_*
   │
7. POST ilink/bot/getuploadurl
   │  body: {
   │    filekey: uuid,
   │    media_type: 1|2|3|4,
   │    to_user_id,
   │    rawsize, rawfilemd5, filesize,
   │    thumb_rawsize?, thumb_rawfilemd5?, thumb_filesize?,
   │    aeskey: base64(key),
   │    base_info
   │  }
   │
8. Response: { upload_param, thumb_upload_param, upload_full_url? }
   │
9. PUT ciphertext → CDN URL (upload_full_url or constructed from upload_param)
   │
10. [IMAGE/VIDEO] PUT thumb_ciphertext → CDN URL (from thumb_upload_param)
   │
11. 构造 CDNMedia {
       encrypt_query_param: upload_param,
       aes_key: base64(key)
    }
   │
12. 构造 MessageItem → sendMessage
```

### 缩略图生成

对于 IMAGE 和 VIDEO，需要生成缩略图：
- IMAGE: 缩小到 ≤120x120 像素
- VIDEO: 取第一帧，缩小到 ≤120x120

在 Rust 中可以用 `image` crate 处理图片缩略图。视频缩略图可以后续添加（MVP 阶段可以跳过视频缩略图）。

## 下载流程

```
1. 从 inbound WeixinMessage 提取 CDNMedia
   │
2. 确定 AES key:
   │  优先 image_item.aeskey (hex 格式)
   │  其次 cdn_media.aes_key (base64 格式)
   │
3. 构造下载 URL:
   │  优先 cdn_media.full_url
   │  其次从 encrypt_query_param 构造
   │
4. HTTP GET 下载密文
   │
5. AES-128-ECB 解密
   │
6. 去 PKCS7 padding
   │
7. 写入临时文件
```

### Key 格式注意

`image_item.aeskey` 是 hex 字符串（32 字符 = 16 字节）
`cdn_media.aes_key` 是 base64 字符串

下载时优先用 `image_item.aeskey`（openclaw-weixin 的做法）。

## Agent 接入

### Inbound 媒体

Agent 收到带媒体的消息时：
1. `InboundMessage.msg_type` = "image" / "voice" / "file" / "video"
2. `InboundMessage.text` = 描述性文本（如 "[图片]"、"[文件: report.pdf]"）
3. `InboundMessage.extra` 包含原始 CDN 引用

如果 agent 需要查看图片内容：
- 使用内置的 `download_media` 能力下载到临时目录
- 传给 vision model 分析

### Outbound 媒体

Agent 要发送媒体时：
1. `OutboundMessage.image_key` 设为文件路径（本地绝对路径或 URL）
2. Plugin 检测到 image_key 后走上传流程
3. 构造带 CDNMedia 的 MessageItem 发送

## 安全考虑

- AES key 不应持久化到磁盘（每次上传随机生成，用完即弃）
- 下载的 AES key 来自消息本身，仅在内存中使用
- 临时文件下载后应在处理完毕后清理
- CDN URL 和 upload_param 包含敏感信息，日志中应 redact
