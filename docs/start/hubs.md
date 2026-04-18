---
title: 文档地图
summary: FastClaw 用户手册各章节说明与相对链接索引。
---

# 文档地图（Hubs）

本文列出用户手册主要分区及用途，便于按任务查找。内部 PRD、设计与评审材料见 `docs/prd/`、`docs/design/`、`docs/reports/`。

## 入门

| 文档 | 说明 |
|------|------|
| [快速开始](./getting-started.md) | 安装、启动网关、TUI 首次对话 |

## 概念与架构

| 文档 | 说明 |
|------|------|
| [系统架构](../concepts/architecture.md) | 组件划分、crate 结构、请求链路、设计原则 |
| [Agent 概念](../concepts/agents.md) | Agent 定义、配置、工具与记忆策略、多 Agent 路由 |
| [记忆模型](../concepts/memory.md) | 工作 / 情景 / 语义记忆、向量与图谱、Dreaming |

## 网关与集成

| 文档 | 说明 |
|------|------|
| [网关配置](../gateway/configuration.md) | 配置文件、JSON5、热重载、校验与回滚思路 |
| [配置字段参考](../gateway/configuration-reference.md) | `FastClawConfig` 及子结构完整说明 |
| [渠道总览](../channels/index.md) | 已支持渠道与扩展方式 |
| [飞书渠道](../channels/feishu.md) | 飞书机器人、字段与 OAuth 示例 |

## 工具与工作流

| 文档 | 说明 |
|------|------|
| [工具与插件](../tools/index.md) | 内置工具、WASM、MCP、权限与开发指引 |
| [DAG 工作流](../dag/index.md) | 节点类型、JSON 定义、检查点与表达式 |

## 智能体进阶能力

| 文档 | 说明 |
|------|------|
| [自我进化](../evolution/index.md) | 反馈、评估、蒸馏、技能自动沉淀 |
| [代码智能](../code/index.md) | Tree-sitter、调用图、测试运行器、补丁与重构 |
| [多 Agent 协作](../collab/index.md) | 委托、流水线、辩证、委员会与 CollabHub |

## 安全与运维

| 文档 | 说明 |
|------|------|
| [安全概览](../security/index.md) | 认证、限流、WASM 隔离、注入防护、HMAC、并发 |
| [CLI 参考](../cli/index.md) | `serve`、`gateway`、`mcp`、`doctor` 等 |
| [REST API](../reference/api.md) | Chat、DAG、Memory、Evolution、动态路由、指标与健康检查 |
| [常见问题](../help/faq.md) | FAQ、性能、调试、已知限制 |

## 内部资料（仓库内路径）

- **产品需求**：[`../prd/product-requirements.md`](../prd/product-requirements.md)
- **技术设计 / 调研**：[`../design/`](../design/) 下多份设计与 OpenClaw 对照材料
- **评审与验收**：[`../reports/`](../reports/)

## 预留章节目录

以下目录与 OpenClaw 风格信息架构对齐，后续可补充专题页：

- `docs/agents/` — Agent 运维与清单化配置（与 `concepts/agents` 互补）
- `docs/memory/` — 记忆子系统运维与调优
- `docs/studio/` — 可视化流程编辑器（Studio）
