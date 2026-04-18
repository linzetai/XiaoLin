---
title: 自我进化与技能
summary: 反馈闭环、策略评估、提示蒸馏、Hermes 式技能沉淀与轨迹流水线。
---

# 自我进化（Evolution）

## 总览

**Evolution** 子系统在长期运行中收集 **轨迹（trajectory）** 与用户 **反馈**，对 Agent 行为做离线或准实时 **评估**，并生成 **提示蒸馏** 与 **技能候选**，从而缩小「实际表现」与「期望策略」之间的差距，减少手工维护提示词的成本。

网关中的后台任务由 `FastClawConfig.evolution`（`skillExtractionIntervalSecs` / `skillMaintenanceIntervalSecs`）控制周期；HTTP 层暴露反馈提交、评估、蒸馏与候选接受/拒绝 API。

## 反馈收集

- 通过 `POST /api/v1/evolution/feedback` 写入结构化反馈（如对某轮回答点赞/点踩、标签、备注）。
- `GET /api/v1/evolution/feedback/:agent_id` 可按 Agent 聚合查询，用于评估任务输入。

## 策略评估

- `GET /api/v1/evolution/evaluate/:agent_id` 触发或查询评估结果（具体为同步摘要还是异步任务以实现为准）。
- 典型信号：工具误用率、用户纠正次数、任务完成率、延迟分位数。

## 提示蒸馏（Prompt distillation）

- `POST /api/v1/evolution/distill/:agent_id`：基于近期高置信轨迹与反馈，生成 **压缩后的系统提示候选**，减少 token 同时保留关键策略。
- 产出进入候选队列，经人工或自动策略 **接受** 后生效。

## 技能自动形成（Hermes 风格）

技能（Skills）以 Markdown / 元数据形式存放在技能目录；Evolution 周期性：

1. **扫描轨迹**：识别重复成功模式（相同工具链、相同领域问题）。
2. **抽取步骤**：LLM 或规则将模式泛化为可复用 **操作说明**。
3. **写入候选技能**：进入 `SkillStore`，附质量分与使用计数。
4. **维护任务**：晋升稳定技能、退役过期技能（`skillMaintenanceIntervalSecs`）。

## 轨迹 → 抽取 → 存储 → 注入

```text
在线对话 / 工具调用
        ↓
 TrajectoryStore（持久化步骤与结果）
        ↓
 抽取器（Evolution 任务）→ SkillStore / 候选提示
        ↓
 运行时注入（skills.promptMode: full | compact | lazy）
```

`skills.promptMode` 控制注入成本：`lazy` 模式以列表 + `read_skill` 工具按需加载，适合大量技能库。

## 相关文档

- [配置字段：EvolutionRuntimeConfig](../gateway/configuration-reference.md)
- [REST API：Evolution](../reference/api.md)
- [工具与插件](../tools/index.md)
