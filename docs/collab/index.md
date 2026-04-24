---
title: 多 Agent 协作
summary: 委托、流水线、辩证、委员会模式与 CollabHub 能力概述。
---

# 多 Agent 协作

`fastclaw-collab` 在 **消息总线（MessageBus）** 之上实现多种编排模式，使多个 `agent_id` 能分工、对抗或合议，而无需把全部逻辑塞进单提示词。

## 委托模式（Delegation）

`DelegationRequest` 描述从 `from_agent` 到 `to_agent` 的任务：

- `task`：语义化任务名（如 `pipeline_stage`）。
- `context`：任意 JSON 负载。

### 推荐：SubAgentDelegation（新）

`SubAgentDelegation` 通过 `SubAgentManager` 执行委托，支持：

- **实时流式输出** — 通过 `delegate_with_stream()` 传入 `mpsc::Sender<StreamEvent>` 接收子 Agent 进度
- **类型化工具注册表** — 根据 `context.subagent_type` 自动筛选子 Agent 可用工具（explore 只读、shell 命令、browser 浏览器）
- **生命周期管理** — 并发控制、超时取消、状态持久化
- **策略控制** — 通过 `SubAgentPolicy` 配置深度限制、并行数、Token 预算

```rust
use fastclaw_collab::SubAgentDelegation;

let bridge = SubAgentDelegation::new(manager, tool_registry, policy);
let result = bridge.delegate(req, Duration::from_secs(60)).await?;
```

### 旧版：delegate_task（已弃用）

~~`delegate_task` 通过主题 `fastclaw.delegation` 发送请求并等待 `DelegationResult`（`success` + `output`）。~~

> **已弃用**：`delegate_task` 仅提供无流式的总线请求/应答。请迁移至 `SubAgentDelegation::delegate()`。

## 流水线模式（Pipeline）

`PipelineDefinition` 包含有序 **`stages`**，每阶段指定 `agent_id` 与可选 `transform` 任务名：

- 上一阶段输出的 JSON 作为下一阶段 `context`。
- 任一阶段 `success == false` 则整体失败。

适用于 **数据预处理 → 分析 → 格式化输出** 等线性工作流。

## 辩证模式（Dialectic）

两个（或多个）Agent 就同一命题 **轮流发言 / 反驳**，由编排器汇总收敛条件。实现见 `dialectic.rs`，适合 **方案评审、风险辩论**。

## 委员会模式（Committee）

`CommitteeConfig`：

- `expert_agent_ids`：并行或串行征求 **专家意见**。
- `lead_agent_id`：主理人 Agent 基于专家意见做 **综合结论**（`committee_synthesis` 任务）。

`parallel: true` 时专家阶段并发执行以降低尾延迟。

## CollabHub 能力

`CollabHub` 聚合 **消息总线**、会话上下文与协作策略入口，供网关在高级 API 或内部任务中调用。与 HTTP ` /api/v1/bus/*` 家族端点配合，可向外暴露 Agent 列表与 **send / request-reply** 语义。

## 相关文档

- [Agent 概念](../concepts/agents.md)
- [REST API：Bus](../reference/api.md)
- [安全：消息签名](../security/index.md)
