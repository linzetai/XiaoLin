# xiaolin-feishu

飞书 / Lark 渠道扩展（内置 Rust 实现）。

## 快速开始

### 1. 配置凭证

在 `~/.xiaolin/default.json` 中添加：

```json
{
  "channels": {
    "feishu": {
      "appId": "cli_xxx",
      "appSecret": "your-app-secret",
      "verificationToken": "your-token",
      "connectionMode": "websocket",
      "replyMode": "mention_only",
      "domain": "https://open.feishu.cn"
    }
  }
}
```

### 2. 启动 XiaoLin

```bash
xiaolin serve
```

内置扩展会自动注册，无需额外配置文件。

## 配置字段

| 字段 | 说明 | 默认值 |
|------|------|--------|
| `appId` | 飞书应用 ID | (必填) |
| `appSecret` | 飞书应用密钥 | (必填) |
| `verificationToken` | Webhook 验证令牌 | (可选) |
| `encryptKey` | 事件加密密钥 | (可选) |
| `connectionMode` | `"websocket"` 或 `"webhook"` | `websocket` |
| `replyMode` | `"mention_only"` 或 `"always"` | `mention_only` |
| `domain` | API 域名 | `https://open.feishu.cn` |
| `userAccessToken` | 用户访问令牌（用于任务、文档等） | (可选) |

## 多账号支持

配置多个飞书机器人路由到不同 Agent：

```json
{
  "channels": {
    "feishu": {
      "domain": "https://open.feishu.cn",
      "accounts": {
        "bot-sales": {
          "appId": "cli_sales_xxx",
          "appSecret": "secret_sales"
        },
        "bot-support": {
          "appId": "cli_support_xxx",
          "appSecret": "secret_support"
        }
      },
      "defaultAccount": "bot-sales"
    }
  },
  "bindings": [
    {
      "agentId": "sales-agent",
      "match": { "channel": "feishu", "accountId": "bot-sales" }
    },
    {
      "agentId": "support-agent",
      "match": { "channel": "feishu", "accountId": "bot-support" }
    }
  ]
}
```

## 功能

- **Webhook 接入** — 事件回调验证（Challenge）与消息解析
- **WebSocket 长连接** — 飞书 WS 协议，实时接收事件
- **消息发送** — 文本、富文本、卡片、文件消息的发送与回复
- **流式输出** — Card Kit 2.0 流式卡片，支持 LLM 流式响应
- **回复容错** — 自动检测已撤回消息，fallback 为直发
- **OAuth 客户端** — 自动管理 tenant_access_token
- **Protobuf 支持** — 飞书 WS 二进制事件解析
- **飞书工具集** — IM、任务、多维表格、文档、知识库、云空间、权限、日历

## 提供的工具 (32)

### IM 核心

| 工具 | 说明 |
|------|------|
| `feishu_send_message` | 发送消息到群聊/私聊 |
| `feishu_reply_message` | 回复指定消息 |
| `feishu_get_chat_messages` | 获取群聊历史消息 |
| `feishu_send_image` | 发送图片 |
| `feishu_reply_image` | 回复图片 |

### IM 增强

| 工具 | 说明 |
|------|------|
| `feishu_send_rich_text` | 发送富文本 (post) 消息 |
| `feishu_send_file` | 上传并发送文件 |
| `feishu_edit_message` | 编辑已发送消息 |
| `feishu_get_message` | 获取单条消息详情 |
| `feishu_forward_message` | 转发消息 |
| `feishu_delete_message` | 撤回消息 |
| `feishu_reaction` | 表情回应 (add/remove/list) |
| `feishu_pin` | 置顶消息 (create/remove/list) |

### 多维表格

| 工具 | 说明 |
|------|------|
| `feishu_bitable_get_meta` | 获取多维表格元信息 |
| `feishu_bitable_list_fields` | 列出表字段 |
| `feishu_bitable_list_records` | 列出记录 |
| `feishu_bitable_get_record` | 获取单条记录 |
| `feishu_bitable_create_record` | 创建记录 |
| `feishu_bitable_update_record` | 更新记录 |
| `feishu_bitable_create_app` | 创建多维表格应用 |
| `feishu_bitable_create_field` | 创建字段 |

### 文档

| 工具 | 说明 |
|------|------|
| `feishu_doc_get_content` | 获取文档内容 (legacy) |
| `feishu_doc_create` | 创建文档 (legacy) |
| `feishu_doc` | 统一文档操作 (read/create/write/list_blocks/get_block/update_block/delete_block) |

### 知识库 / 云空间 / 权限 / 群聊

| 工具 | 说明 |
|------|------|
| `feishu_wiki` | 知识库操作 (spaces/nodes/get/search/create/move/rename) |
| `feishu_drive` | 云空间操作 (list/info/create_folder/move/delete/comments) |
| `feishu_perm` | 权限管理 (list/add/remove) |
| `feishu_chat` | 群聊管理 (info/members/member_info) |
| `feishu_app_scopes` | 查看应用已授权权限 |

### 其他

| 工具 | 说明 |
|------|------|
| `feishu_task_create` | 创建飞书任务 |
| `feishu_task_list` | 列出飞书任务 |
| `feishu_calendar_list_events` | 列出日历事件 |

## 关键导出

```rust
pub use crate::plugin::FeishuPlugin;
pub use crate::client::FeishuClient;
```
