---
title: DAG 工作流引擎
summary: DAG 概念、节点类型、JSON 定义、检查点、表达式与事件监控。
---

# DAG 工作流

## 什么是 DAG 工作流

**DAG（有向无环图）** 将多步自动化表示为 **节点（Node）** 与 **边（Edge）**：节点完成计算或副作用，边表达依赖与分支。FastClaw 的 `fastclaw-dag` crate 负责解析 JSON 定义、校验图结构、在网关中调度执行，并支持 **检查点** 以便失败恢复与人工审批暂停。

## 节点类型（NodeKind）

以下为 Serde 枚举 `snake_case` 序列化名，与 JSON 中 `kind` 字段对应：

| `kind` | 说明 |
|--------|------|
| `llm_call` | 调用 LLM；`config` 常见键：`prompt`、`model` |
| `tool_call` | 调用已注册工具；`tool_name`、`arguments` |
| `condition` | 条件分支；边使用 `label` 匹配分支结果 |
| `parallel` | 扇出并行 |
| `join` | 扇入等待全部上游 |
| `human_approval` | 暂停等待人工确认 |
| `code` | 内联/脚本代码节点 |
| `reflect` | 质量检查与带标签重试路由（`pass`、`retry` 等标签需与边一致） |
| `loop` | 子图循环；`loop_config.max_iterations`、可选 `condition_expr`；`body` 标签边定义循环体 |

节点还可选：`timeout_ms`、`retry_policy`、`failure_policy`（`abort` / `skip` / `continue`）。

## 创建流程（JSON）

```json
{
  "id": "demo-flow",
  "name": "Demo",
  "nodes": [
    { "id": "n1", "kind": "llm_call", "config": { "prompt": "Summarize: {{input}}" } },
    { "id": "n2", "kind": "tool_call", "config": { "tool_name": "web_search", "arguments": {} } }
  ],
  "edges": [
    { "from": "n1", "to": "n2" }
  ]
}
```

校验接口：`POST /api/v1/dag/validate`  
执行接口：`POST /api/v1/dag/execute`

## 检查点（Checkpoints）

示例配置（`config/default.json`）中的 `dag` 节可包含：

- `checkpointEnabled`：是否持久化中间状态  
- `checkpointDbPath`：SQLite 路径  
- `maxParallelNodes` / `defaultNodeTimeoutSecs`：并发与默认超时  

人工审批节点会将会话暂停在检查点，直到外部系统或 API 恢复执行。

## 表达式求值语法

- **条件节点**：`config.condition` 可为 JSONPath 或引擎支持的布尔表达式字符串（具体函数集以实现为准）。
- **循环节点**：`loop_config.condition_expr` 在每轮后求值，为真则继续。

建议在预发环境对复杂表达式编写 **单元级 DAG 片段** 做回归。

## 事件与监控

- 网关日志与 `fastclaw-observe` 指标中可追踪 DAG 提交率、节点耗时、失败重试次数。
- 将 `/metrics` 接入 Prometheus/Grafana 以配置告警阈值。

## 相关文档

- [REST API：DAG](../reference/api.md)
- [系统架构](../concepts/architecture.md)
