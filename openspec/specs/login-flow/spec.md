# WeChat QR Login Flow Spec

## 概述

微信 channel 使用 QR 码扫描方式认证，与飞书的 App ID/Secret 模式完全不同。需要支持两个入口：CLI 终端和 Tauri UI。

## CLI 登录流程

### 命令

```bash
xiaolin channels login --channel wechat [--account-id <id>]
```

### 流程

```
用户执行 CLI 命令
  │
  ├─ POST ilink/bot/get_bot_qrcode?bot_type=3
  │   body: { local_token_list: [existing tokens] }
  │
  ├─ 收到 { qrcode, qrcode_img_content }
  │
  ├─ 终端打印 QR 码 (qr2term)
  ├─ 打印备选 URL
  │
  ├─ 循环轮询 (最长 8 分钟)
  │   │
  │   ├─ GET ilink/bot/get_qrcode_status?qrcode=<qr>
  │   │
  │   ├─ status = "wait" → 继续等待
  │   ├─ status = "scaned" → 打印"正在验证"
  │   ├─ status = "need_verifycode"
  │   │   ├─ 从 stdin 读取配对码
  │   │   └─ 下次轮询携带 verify_code 参数
  │   ├─ status = "verify_code_blocked" → 刷新 QR（最多 3 次）
  │   ├─ status = "scaned_but_redirect"
  │   │   └─ 切换 polling host 到 redirect_host
  │   ├─ status = "expired" → 自动刷新 QR（最多 3 次）
  │   ├─ status = "binded_redirect" → 已绑定，成功
  │   └─ status = "confirmed"
  │       ├─ 提取 bot_token, ilink_bot_id, baseurl, ilink_user_id
  │       ├─ normalize account_id (去除特殊字符)
  │       ├─ save_credential(account_id, { token, base_url, user_id })
  │       └─ 触发 channel reload
  │
  └─ 打印结果
```

### 错误处理

- QR 过期：自动刷新，最多 3 次
- 配对码错误：提示重新输入
- 配对码多次错误 (verify_code_blocked)：刷新 QR 重来
- 网络错误：视为 "wait" 继续轮询
- 总超时 8 分钟：返回失败

## Tauri UI 登录流程

### API 设计

```
POST /api/v1/channels/wechat/login/start
  Request: { account_id?: string }
  Response: { session_key: string, qr_url: string, message: string }

GET  /api/v1/channels/wechat/login/status/:session_key
  Response: SSE stream
  Events:
    data: { "status": "waiting", "qr_url": "..." }
    data: { "status": "scanned" }
    data: { "status": "need_verifycode" }
    data: { "status": "confirmed", "account_id": "..." }
    data: { "status": "expired", "qr_url": "<new_url>" }  // auto-refresh
    data: { "status": "error", "message": "..." }
    data: { "status": "timeout" }

POST /api/v1/channels/wechat/login/verify/:session_key
  Request: { code: "1234" }
  Response: { ok: true }
```

### 前端交互

1. 用户点击"添加微信账号"
2. 调用 `POST /login/start` → 拿到 session_key + qr_url
3. 显示 QR 码图片（`<img src="{qr_url}">` 或渲染二维码）
4. 建立 SSE 连接 `GET /login/status/{session_key}`
5. 监听事件更新 UI：
   - `waiting` → 显示"请扫码"
   - `scanned` → 显示"已扫码，请在手机确认"
   - `need_verifycode` → 弹出数字输入框
   - `confirmed` → 显示"连接成功" → 自动刷新账号列表
   - `expired` → 自动更新 QR 图片（最多 3 次）
   - `error` / `timeout` → 显示错误 + 重试按钮

## Account ID 规范化

微信返回的 `ilink_bot_id` 格式如 `hex@im.bot`，需规范化为文件系统安全的 ID：

```
原始: a1b2c3d4@im.bot
规范化: a1b2c3d4-im-bot
```

规则：将 `@` 和 `.` 替换为 `-`。

## Token 刷新策略

微信 token 有有效期（具体时长取决于服务端）。当 `getUpdates` 返回 `errcode: -14` 时：

1. 暂停该账号的 monitor
2. 标记账号状态为 "session_expired"
3. 通知用户重新扫码（通过 WebSocket 推送或 UI 状态更新）
4. 用户扫码后自动恢复 monitor

## 多账号管理

每次 `login` 创建一个新的账号条目。账号之间完全隔离：

- 独立 token
- 独立 base_url（可能因 IDC 不同）
- 独立 monitor 循环
- 独立 context_token cache
- 独立 sync cursor (get_updates_buf)

Session key 格式：`wechat:{account_id}:{peer_id}`
