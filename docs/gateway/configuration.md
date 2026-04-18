---
title: 网关与主配置
summary: JSON5 主配置文件结构、OpenClaw 兼容路径、热重载与校验回滚行为。
---

# 配置文件结构

FastClaw 使用 **单文件主配置**（JSON5），与 Agent 拆分目录（可选）组合使用。主文件常见名：`config/default.json` 或 `~/.fastclaw/config/default.json`。

顶层键与业务模块对应，例如 `gateway`、`agents`、`channels`、`memory`、`security` 等（完整列表见 [配置字段参考](./configuration-reference.md)）。

### 最小可读示例

```json5
{
  gateway: { port: 18789, bind: "loopback" },
  agents: {
    list: [{ id: "main", default: true }]
  },
  models: {
    dashscope: {
      providerType: "openai_compatible",
      baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1",
      defaultModel: "qwen-plus"
    }
  },
  credentials: {
    dashscope: { apiKey: "YOUR_KEY" }
  }
}
```

## JSON5 支持

- 允许 **注释**（`//` 与 `/* */`）。
- 允许 **尾随逗号**、**无引号键名** 等 JSON5 便利语法。
- 解析器为 `json5`；非法语法会在启动时失败并记录路径。

## OpenClaw 兼容性

加载顺序（简化，以 `fastclaw_core::config::load_config` 为准）：

1. 当前工作目录下 `config/default.json`
2. `~/.fastclaw/config/default.json`（或 `--dev` / `--profile` 对应目录）
3. `~/.openclaw/openclaw.json`（迁移期回退）

键名采用 **camelCase**（Serde `rename_all = "camelCase"`），与 OpenClaw 文档中的常见字段一致；部分 `models` 子项仍接受 `providerType`、`defaultModel` 等别名。

## 热重载：文件监听与 SIGHUP

- **Agent 配置目录**：网关对 `paths.agentsDir`（默认如 `config/agents`）做递归监听；文件变更后触发 **重新加载**。
- **Unix `SIGHUP`**：向网关进程发送 `SIGHUP` 会走与文件监听相同的 **Agent 热重载** 路径，便于 systemd/k8s 侧触发刷新而无需重启进程。
- **插件目录**：可配置插件热更新监听（见示例配置中的 `plugins.hotReload`）。

主配置文件的持续热重载若未在部署中启用，请通过 **重启网关** 或运维流程更新全局项。

## 配置校验与原子回滚

- `fastclaw config check` 可对当前解析结果做校验。
- Agent 热重载路径会先 **解析并校验** 新 Agent 集合；若校验失败（例如重复 `agent_id`、模型字段为空），**不会替换** 当前内存中的路由表，即保持 **上一次成功加载** 的配置继续服务，从行为上等价于 **原子回滚**。

建议在 CI 中对 `config/default.json` 与 `config/agents/*.json` 运行校验后再发布。

## 相关文档

- [配置字段参考](./configuration-reference.md)
- [CLI：config 子命令](../cli/index.md)
- [安全概览](../security/index.md)
