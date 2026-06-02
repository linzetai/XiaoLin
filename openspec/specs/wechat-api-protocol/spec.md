# WeChat Backend API Protocol Spec

基于 [openclaw-weixin](https://github.com/Tencent/openclaw-weixin) 提取的完整协议规格。

## Base URL

- QR 登录固定：`https://ilinkai.weixin.qq.com`
- 消息 API：登录成功后服务端返回 `baseurl`（可能因 IDC 重定向而不同）

## Common Headers

所有 POST 请求必须携带：

```
Content-Type: application/json
AuthorizationType: ilink_bot_token
Authorization: Bearer <token>
X-WECHAT-UIN: <base64(random_uint32_decimal)>
iLink-App-Id: <package.json 中的 ilink_appid>
iLink-App-ClientVersion: <major<<16 | minor<<8 | patch>
```

`X-WECHAT-UIN` 每次请求随机生成：`base64(to_string(random_u32))`。

## Endpoints

### POST ilink/bot/getupdates

长轮询，服务端 hold 直到有新消息或超时。

**Request:**
```json
{
  "get_updates_buf": "<cursor from previous response, empty string for first>",
  "base_info": { "channel_version": "2.0.x", "bot_agent": "XiaoLin/0.0.6" }
}
```

**Response:**
```json
{
  "ret": 0,
  "msgs": [WeixinMessage, ...],
  "get_updates_buf": "<new cursor>",
  "longpolling_timeout_ms": 35000
}
```

**Error codes:**
- `errcode: -14` → session expired, 需重新扫码
- `ret != 0` → API 错误

**Timeout 处理:** 客户端 `AbortController` 超时后返回空 `{ret:0, msgs:[]}` 继续重试。

### POST ilink/bot/sendmessage

**Request:**
```json
{
  "msg": {
    "to_user_id": "<target>",
    "context_token": "<from inbound message>",
    "item_list": [
      { "type": 1, "text_item": { "text": "Hello" } }
    ]
  },
  "base_info": { ... }
}
```

### POST ilink/bot/getuploadurl

获取 CDN 上传预签名参数。

**Request:**
```json
{
  "filekey": "<unique key>",
  "media_type": 1,
  "to_user_id": "<target>",
  "rawsize": 12345,
  "rawfilemd5": "<md5 hex>",
  "filesize": 12352,
  "thumb_rawsize": 1024,
  "thumb_rawfilemd5": "<md5 hex>",
  "thumb_filesize": 1040,
  "aeskey": "<base64 AES key>",
  "base_info": { ... }
}
```

`media_type`: 1=IMAGE, 2=VIDEO, 3=FILE, 4=VOICE
`filesize`: AES-128-ECB 加密后的密文大小（= ceil(rawsize/16)*16）

**Response:**
```json
{
  "upload_param": "<encrypted params for original>",
  "thumb_upload_param": "<encrypted params for thumbnail>",
  "upload_full_url": "<optional direct CDN URL>"
}
```

### POST ilink/bot/getconfig

获取账号配置（主要是 typing_ticket）。

**Request:**
```json
{
  "ilink_user_id": "<user id>",
  "context_token": "<optional>",
  "base_info": { ... }
}
```

**Response:**
```json
{
  "ret": 0,
  "typing_ticket": "<base64>"
}
```

### POST ilink/bot/sendtyping

**Request:**
```json
{
  "ilink_user_id": "<user>",
  "typing_ticket": "<from getConfig>",
  "status": 1,
  "base_info": { ... }
}
```

`status`: 1=typing, 2=cancel

### POST ilink/bot/msg/notifystart

通知服务端 channel 启动。

**Request:** `{ "base_info": { ... } }`
**Response:** `{ "ret": 0 }`

### POST ilink/bot/msg/notifystop

通知服务端 channel 停止。

**Request:** `{ "base_info": { ... } }`
**Response:** `{ "ret": 0 }`

## QR Login Protocol

### Step 1: Get QR Code

**POST** `ilink/bot/get_bot_qrcode?bot_type=3`

```json
{ "local_token_list": ["<existing tokens...>"] }
```

**Response:**
```json
{
  "qrcode": "<qr code identifier>",
  "qrcode_img_content": "<URL to QR image>"
}
```

### Step 2: Poll Status

**GET** `ilink/bot/get_qrcode_status?qrcode=<qr>&verify_code=<optional>`

Long-poll，35s timeout。

**Response:**
```json
{
  "status": "wait|scaned|confirmed|expired|need_verifycode|scaned_but_redirect|binded_redirect|verify_code_blocked",
  "bot_token": "<on confirmed>",
  "ilink_bot_id": "<on confirmed>",
  "baseurl": "<on confirmed>",
  "ilink_user_id": "<on confirmed>",
  "redirect_host": "<on scaned_but_redirect>"
}
```

**Status flow:**
```
wait → scaned → confirmed (success)
wait → expired (need refresh QR)
wait → scaned → need_verifycode → scaned (code correct) → confirmed
wait → scaned → need_verifycode → verify_code_blocked (too many failures)
wait → scaned → scaned_but_redirect (IDC redirect, switch polling host)
wait → scaned → binded_redirect (already bound, success without new token)
```

## Message Types

### WeixinMessage

```typescript
{
  seq?: number,
  message_id?: number,
  from_user_id?: string,     // sender
  to_user_id?: string,       // receiver (bot)
  create_time_ms?: number,
  session_id?: string,
  message_type?: number,     // 1=USER, 2=BOT
  message_state?: number,    // 0=NEW, 1=GENERATING, 2=FINISH
  item_list?: MessageItem[],
  context_token?: string     // MUST pass back when replying
}
```

### MessageItem

```typescript
{
  type?: number,       // 1=TEXT, 2=IMAGE, 3=VOICE, 4=FILE, 5=VIDEO
  text_item?: { text: string },
  image_item?: {
    media?: CDNMedia,
    thumb_media?: CDNMedia,
    aeskey?: string        // preferred hex AES key for decryption
  },
  voice_item?: {
    media?: CDNMedia,
    encode_type?: number,  // 6=SILK
    playtime?: number,     // ms
    text?: string          // voice-to-text
  },
  file_item?: {
    media?: CDNMedia,
    file_name?: string,
    md5?: string,
    len?: string           // string representation of size
  },
  video_item?: {
    media?: CDNMedia,
    thumb_media?: CDNMedia,
    video_size?: number,
    play_length?: number
  },
  ref_msg?: { message_item?: MessageItem, title?: string }
}
```

### CDNMedia

```typescript
{
  encrypt_query_param?: string,   // CDN download/upload params
  aes_key?: string,               // base64 AES-128 key
  encrypt_type?: number,          // 0=fileid only, 1=packed
  full_url?: string               // newer API: direct CDN URL
}
```

## CDN Upload Flow

1. Generate random 16-byte AES key
2. Calculate: `rawsize`, `rawfilemd5 = md5(plaintext)`, `filesize = ceil(rawsize/16)*16`
3. For IMAGE/VIDEO: same for thumbnail
4. Call `getUploadUrl` → get `upload_param`, `thumb_upload_param`, `upload_full_url`
5. AES-128-ECB encrypt file → PUT to CDN URL with `upload_param`
6. AES-128-ECB encrypt thumbnail → PUT to CDN URL with `thumb_upload_param`
7. Construct `CDNMedia { encrypt_query_param: upload_param, aes_key: base64(key) }`
8. Include in `MessageItem` and `sendMessage`

## CDN Download Flow

1. Use `full_url` (if present) or construct from `encrypt_query_param`
2. HTTP GET the encrypted file
3. AES-128-ECB decrypt using `aes_key` (base64-decode first)
4. Remove PKCS7 padding
