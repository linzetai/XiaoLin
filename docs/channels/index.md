---
title: 渠道集成总览
summary: FastClaw 支持的七大渠道扩展、插件式架构与新增渠道指引。
---

# 渠道集成总览

## 已支持的七个渠道

以下扩展位于仓库 `extensions/` 目录，各自为独立 Cargo 包，由网关按需加载并与主配置 `channels` 节联动：

| 渠道 | 目录 | 典型场景 |
|------|------|----------|
| 飞书（Lark） | `extensions/feishu` | 企业 IM、机器人、多维表格与文档 |
| Slack | `extensions/slack` | 海外团队协作 |
| Microsoft Teams | `extensions/msteams` | 企业办公套件 |
| Discord | `extensions/discord` | 社区与开发者支持 |
| Telegram | `extensions/telegram` | 移动端与公开频道 |
| Matrix | `extensions/matrix` | 联邦去中心化通讯 |
| WhatsApp | `extensions/whatsapp` | 客户触达（视地区政策） |

具体启用方式：在主配置 `channels` 下增加对应键（如 `"feishu": { "enabled": true, ... }`），并在 `bindings` 中把入站路由到目标 Agent。

## 渠道插件架构

每个扩展通常实现：

- **Webhook / 长连接入站**：将厂商事件解析为 FastClaw 内部消息格式。
- **Outbound 发送**：将 Agent 回复映射为厂商消息类型（文本、卡片、富媒体）。
- **可选 WASM / 技能**：如飞书附带任务、文档、日历等工具技能包。

网关通过统一接口注册渠道，对外暴露 `GET /api/v1/channels` 与 `POST /webhook/:channel_id`。

## 如何新增一个渠道

1. **复制扩展骨架**：在 `extensions/` 下新建 crate，参考现有 `feishu` 或 `slack` 的 `lib.rs` 与 `Cargo.toml` 依赖。
2. **实现 Channel trait / 注册函数**（以当前网关扩展契约为准）：完成鉴权、事件解析、发送回执。
3. **声明配置 schema**：在 `ChannelConfig` 或扩展自有 schema 中增加所需字段，并文档化。
4. **绑定路由**：在主配置 `bindings` 中为该渠道增加 `match.channel`。
5. **联调**：使用 `fastclaw serve` + 厂商沙箱应用完成端到端测试。

详细示例请参考 [飞书渠道](./feishu.md)。

## 相关文档

- [飞书配置与排障](./feishu.md)
- [网关配置](../gateway/configuration.md)
- [安全：认证与 Webhook](../security/index.md)
