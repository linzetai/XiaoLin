---
title: Agent 概念
summary: Agent 是什么、JSON 配置结构、系统提示与工具/记忆策略、多 Agent 路由。
---

# Agent 是什么

在 FastClaw 中，**Agent** 表示一条可独立配置的对话智能体：**模型（provider + model）**、**系统提示（通常来自 agent 目录下的模板/文件）**、**工具白名单/黑名单**、**技能（Skills）** 与 **工作区路径** 等组合在一起，对外呈现为一个 `agent_id`（如 `main`、`reviewer`）。

运行时网关根据入站上下文选择唯一目标 Agent，再加载其会话与记忆策略。

## 配置结构（JSON）

顶层 `agents` 对象（`FastClawConfig::agents`）通常包含：

- **`defaults`**：所有 Agent 共享的默认值（可被单项覆盖）。
- **`list`**：多个 `AgentEntry`，每项至少包含 **`id`**。

示例（节选，与仓库 `config/default.json` 风格一致）：

```json
{
  "agents": {
    "defaults": {
      "model": "dashscope/qwen-plus",
      "workspace": "workspace"
    },
    "list": [
      {
        "id": "main",
        "name": "FastClaw 助手",
        "default": true,
        "workspace": "workspace",
        "tools": {
          "allow": ["web_search", "read_file"],
          "deny": []
        },
        "skills": ["feishu-channel-rules"]
      }
    ]
  }
}
```

### 常用字段说明

| 字段 | 说明 |
|------|------|
| `id` | 稳定标识，用于 API、绑定与总线消息 |
| `name` / `identity` | 展示名与头像等渠道侧身份 |
| `default` | 为 `true` 时表示默认 Agent（路由未命中时的回退之一，具体以路由实现为准） |
| `model` | 覆盖默认模型，格式多为 `providerKey/modelName`，与 `models` 中的键对应 |
| `workspace` | 代码与文件类工具的根路径提示 |
| `agentDir` | 若使用按目录拆分的 Agent 配置，指向该目录 |
| `tools.allow` / `tools.deny` | 工具权限；deny 优先于 allow |
| `tools.profile` | 可选工具配置档案名 |
| `groupChat` | 群聊中 `mention_patterns`、`require_mention` 等 |
| `skills` | 显式挂载的技能 ID 列表 |

更完整的类型定义见源码 `crates/fastclaw-core/src/config.rs` 中的 `AgentEntry`、`AgentToolsConfig`。

## 系统提示、工具与记忆策略

- **系统提示**：通常由 Agent 工厂按 `agent_id` 加载模板文件（具体路径依赖 `paths.agentsDir` 与 `agentDir`），并与全局策略、渠道规则拼接。
- **工具**：在注册表中的内置工具 + 已加载 WASM 插件能力 + MCP 暴露的工具；最终列表受 `tools.allow`/`deny` 与网关侧策略过滤。
- **记忆**：全局 `memory.enabled` 与嵌入配置决定是否写入/检索情景与语义记忆；Agent 级可有不同数据分片（实现细节见记忆模块）。

## 多 Agent 路由

**绑定（`bindings`）** 将入站条件映射到 `agent_id`：

```json
{
  "bindings": [
    { "agentId": "main", "match": { "channel": "feishu" } },
    {
      "agentId": "support",
      "match": {
        "channel": "slack",
        "peer": { "kind": "channel", "id": "C01234567" }
      }
    }
  ]
}
```

此外，网关提供 **动态路由 API**（`/api/v1/routes`）用于运行时增删改规则，与静态绑定互补。多 Agent **协作**（委托、流水线等）见 [多 Agent 协作](../collab/index.md)。

## 相关文档

- [网关配置](../gateway/configuration.md)
- [配置字段参考](../gateway/configuration-reference.md)
- [工具与插件](../tools/index.md)
- [记忆模型](./memory.md)
