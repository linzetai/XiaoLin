---
title: 飞书（Lark）渠道
summary: 飞书机器人创建、FastClaw 配置字段、用户 OAuth 与常见问题。
---

# 飞书渠道接入

本文说明在飞书开放平台创建应用与机器人，并在 FastClaw 中完成配置与排障。实现细节以 `extensions/feishu` 为准。

## 在飞书控制台创建应用

1. 打开 [飞书开放平台](https://open.feishu.cn/)，创建 **企业自建应用**。
2. 在 **权限管理** 中按需开通：`im:message`、`im:message.group_at_msg`、机器人相关事件、多维表格/云文档等（仅当你需要扩展工具时）。
3. 在 **事件订阅** 中填写 FastClaw 网关公网地址，例如：  
   `https://<your-host>/webhook/feishu`  
   与控制台要求一致地完成 **Challenge 校验**（由网关 Webhook 处理）。
4. 发布版本并 **安装到企业**，获取 `App ID`、`App Secret`；在「事件与回调」页复制 **Verification Token**；若启用加密推送，记录 **Encrypt Key**。

## FastClaw 主配置示例

```json5
{
  channels: {
    feishu: {
      enabled: true,
      appId: "cli_xxxxxxxx",
      appSecret: "your_secret",
      verificationToken: "your_token",
      // encryptKey: "optional",
      connectionMode: "websocket",
      replyMode: "mention_only",
      domain: "https://open.feishu.cn"
    }
  },
  bindings: [
    { agentId: "main", match: { channel: "feishu" } }
  ]
}
```

### 常用字段

| 字段 | 说明 |
|------|------|
| `appId` / `appSecret` | 应用凭证 |
| `verificationToken` | 校验事件来源 |
| `encryptKey` | 消息体加密时必填 |
| `connectionMode` | 如 `websocket` 长连接接收事件 |
| `replyMode` | 如 `mention_only` 仅在群聊 @ 机器人时回复 |
| `domain` | OpenAPI 域名，国内一般为 `https://open.feishu.cn` |
| `userAccessToken` | 用户授权 token，用于代表用户调用任务/文档等 API |

## 用户 OAuth（高级工具）

访问 **用户维度** 接口（任务、日历、个人文档等）需要 OAuth 2.0 用户登录授权：

1. 在开放平台配置 **重定向 URL** 与安全域名。
2. 实现或对接授权流程，取得 **`user_access_token`**（及刷新机制）。
3. 将 token 写入配置 `channels.feishu.userAccessToken`，或通过后续版本的凭据托管方案注入。

> 请勿将长期令牌提交到 Git；生产环境建议使用密钥管理或运行时注入。

## 排障清单

| 现象 | 排查 |
|------|------|
| Challenge 失败 | 确认公网 URL、TLS 证书、网关日志；路径必须为 `/webhook/feishu`（若 `channel_id` 映射一致） |
| 收不到事件 | 检查应用是否已安装、事件订阅字段是否与文档一致 |
| 群聊无响应 | `replyMode` 是否为 `mention_only`；是否 @ 了机器人 |
| 401 / 403 on OpenAPI | `appSecret` 是否正确；用户接口是否缺少 `userAccessToken` |

扩展内置的 `feishu-troubleshoot` 等技能（若已挂载到 Agent）可辅助诊断。

## 相关文档

- [渠道总览](./index.md)
- [网关配置](../gateway/configuration.md)
- [安全概览](../security/index.md)
