---
title: 记忆模型
summary: 三层记忆、工作区 LRU、向量检索、语义图与 Dreaming 流水线概述。
---

# 记忆子系统概念

FastClaw 的认知记忆围绕 **工作记忆（Working）**、**情景记忆（Episodic）** 与 **语义记忆（Semantic）** 三层组织，配合嵌入向量与可选知识图谱，支撑长期上下文与检索增强生成（RAG）。

## 三层模型

1. **工作记忆（Working）**  
   当前对话窗口内的高频上下文，体量受会话压缩策略约束；用于「此刻正在做什么」的短期聚焦。

2. **情景记忆（Episodic）**  
   按时间线记录交互片段（episode），可附带摘要与嵌入，用于「上次做过什么」的追溯。

3. **语义记忆（Semantic）**  
   沉淀为可查询 **事实（facts）** 与关系，面向「稳定知识」而非逐字聊天记录。

三层在网关中与 Agent 绑定使用；API 层暴露 episodes / facts 的列表、搜索与删除等操作（见 [REST API](../reference/api.md)）。

## 工作记忆与 LRU

工作记忆在效果上类似 **带容量上限的缓存**：新 token 与工具结果进入窗口，触发阈值后与 `fastclaw-context` 协同做 **压缩/摘要**，避免无限增长。可将其理解为策略化的 **LRU / 滑动窗口** 语义（精确策略以运行时配置与上下文引擎为准）。

## 向量检索：SQLite + 可选 usearch

- 嵌入由 `memory.embedding` 配置：`provider` 为 `local`（默认，纯 Rust 本地模型）或 `remote`（OpenAI 兼容嵌入 API）。
- 向量索引路径等可在示例配置 `vectorIndexPath`、`vectorDimensions` 中与存储后端对齐；生产部署常用 **SQLite** 存元数据，**usearch**（若启用）加速近邻搜索。
- 亦可在配置中关闭向量（如 `none`），退化为关键词检索路径（以实际解析为准）。

## 语义图（petgraph）

语义层可将实体与关系组织为 **图结构**（实现上常用 `petgraph`），支持简单图遍历与影响分析类能力，并与向量检索互补：向量找相似片段，图找关联概念。

## Dreaming 流水线

`memory.dreamingIntervalSecs` 控制后台 **「做梦」周期**：对近期情景做聚类、摘要或晋升到语义层，并执行遗忘曲线（若配置 `forgetting`）。间隔为 `0` 可关闭该周期任务。

示例（JSON5）：

```json5
{
  "memory": {
    "enabled": true,
    "dreamingIntervalSecs": 3600,
    "embedding": {
      "provider": "local",
      "model": "sentence-transformers/all-MiniLM-L6-v2"
    }
  }
}
```

## 运维提示

- 首次启用本地嵌入会下载模型到用户缓存目录，磁盘与冷启动时间会上升。
- 大规模部署建议监控嵌入队列延迟、索引大小与 SQLite WAL 增长。

更多实现细节见 [`../design/core-capabilities.md`](../design/core-capabilities.md) 与 [`../design/technical-design.md`](../design/technical-design.md)。
