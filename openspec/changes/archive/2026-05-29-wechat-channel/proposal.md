# WeChat Channel for XiaoLin

## Problem

XiaoLin 目前只有飞书（Feishu）一个 IM channel 接入。微信作为中国最大的即时通讯平台，是用户日常使用最频繁的沟通工具。需要一个原生 WeChat channel 让 XiaoLin agent 能够通过微信与用户交互。

## Solution

基于腾讯官方 [openclaw-weixin](https://github.com/Tencent/openclaw-weixin) 的协议知识，用 **Native Rust** 实现一个完整的 WeChat ChannelPlugin，作为 `extensions/wechat/` crate。

### 为什么是 Native Rust 而不是 Process Plugin

1. openclaw-weixin 深度耦合 OpenClaw Plugin SDK（`openclaw/plugin-sdk/*`），无法直接复用代码
2. WeChat 后端 API 只有 5 个 HTTP JSON 端点，协议简单清晰，用 Rust 实现很直接
3. 避免外部 Node.js 进程依赖，保持 XiaoLin 单二进制分发
4. 原生支持流式回复和 typing indicator，无 IPC 延迟

### 核心能力

- **QR 码登录**：CLI 终端 + Tauri UI 双通道扫码登录
- **Long-poll 收消息**：getUpdates 长轮询，事件驱动推送到 gateway pipeline
- **富媒体消息**：text / image / voice / file / video 双向收发
- **CDN 媒体加密**：AES-128-ECB 加密传输（与微信后端兼容）
- **Typing indicator**：显示"正在输入"状态
- **多账号**：支持同时登录多个微信号，独立 session 隔离
- **会话上下文**：context_token 生命周期管理
- **断线重连**：自动重试 + 指数退避 + session 过期感知

## Scope

### In Scope

- `extensions/wechat/` Rust crate，实现 `ChannelPlugin` trait
- WeChat 后端 API client（getUpdates, sendMessage, getUploadUrl, getConfig, sendTyping, notifyStart, notifyStop）
- QR 码登录流程（CLI + Tauri UI webview）
- 消息双向映射（WeixinMessage ↔ InboundMessage/OutboundMessage）
- CDN 媒体上传/下载（AES-128-ECB）
- Long-poll monitor 循环
- Gateway 注册和热重载支持
- Credential 持久化（`~/.xiaolin-dev/credentials/wechat/`）
- 配置项（`channels.wechat` in default.json）

### Out of Scope

- 微信群聊（当前 API 仅支持 DM，chatTypes: ["direct"]）
- 企业微信（不同 API 体系）
- 微信支付等非消息能力
- 微信小程序消息

## Success Criteria

1. 能通过 CLI `xiaolin channels login --channel wechat` 扫码登录
2. 能通过 Tauri UI 界面扫码登录
3. 微信用户发送文字消息，XiaoLin agent 正确收到并回复
4. 微信用户发送图片/文件，agent 能识别并描述
5. Agent 可以主动发送图片/文件给微信用户
6. 多个微信号同时在线不冲突
7. 断线后自动重连

## References

- [openclaw-weixin GitHub](https://github.com/Tencent/openclaw-weixin)
- [WeChat Backend API Protocol](https://github.com/Tencent/openclaw-weixin/blob/main/README.md#backend-api-protocol)
- XiaoLin ChannelPlugin trait: `crates/xiaolin-core/src/channel.rs`
- Feishu reference implementation: `extensions/feishu/`
