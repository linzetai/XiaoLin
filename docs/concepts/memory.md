---
title: 记忆模型
summary: 三层记忆、自动捕获、重要性评分、LLM 巩固与 Dreaming 流水线概述。
---

# 记忆子系统概念

FastClaw 的认知记忆围绕 **工作记忆（Working）**、**情景记忆（Episodic）** 与 **语义记忆（Semantic）** 三层组织，配合嵌入向量、可选知识图谱、关键词自动捕获、重要性评分与 LLM 巩固，支撑长期上下文与检索增强生成（RAG）。

## 三层模型

1. **工作记忆（Working）**  
   当前对话窗口内的高频上下文，体量受会话压缩策略约束；用于「此刻正在做什么」的短期聚焦。

2. **情景记忆（Episodic）**  
   按时间线记录交互片段（episode），可附带摘要与嵌入，用于「上次做过什么」的追溯。

3. **语义记忆（Semantic）**  
   沉淀为可查询 **事实（facts）** 与关系，面向「稳定知识」而非逐字聊天记录。

三层在网关中与 Agent 绑定使用；API 层暴露 episodes / facts 的列表、搜索与删除等操作（见 [REST API](../reference/api.md)）。

## 自动捕获管线

记忆不再完全依赖 LLM 主动调用 `memory_store`，而是通过多层自动捕获机制保证关键信息不丢失：

### 关键词拦截（MemoryKeywordInterceptor）

在 `on_ingest` 阶段扫描用户消息，检测到记忆触发词时自动存储为语义事实：

| 语言 | 触发模式 |
|------|----------|
| 英文 | `remember that...`, `note this:...`, `keep in mind:...`, `don't forget:...`, `my preference is:...` |
| 中文 | `记住...`, `记一下...`, `别忘了...`, `我的偏好是...`, `以后注意...` |

捕获后注入系统提示 `[Auto-captured]`，让 LLM 知晓已记录并做出回应。

### 提示词强化（Prompt Reinforcement）

系统提示、工具使用指南、AGENTS.md 模板均内置显式记忆规则，提高 LLM 主动调用 `memory_store` 的频率。

### LLM 会话巩固（MemoryConsolidationHook）

在 `on_after_turn` 阶段，当对话满足最低消息数且重要性评分超过阈值时，使用 LLM 异步生成 2-3 句摘要并提取 FACT 三元组，非阻塞存储到情景与语义记忆。

### 自动记录（auto_record_episode）

每轮 Agent 回复后自动记录轻量情景，使用 `ImportanceScorer::score_single` 动态评分替代固定 0.5。

## 重要性评分（ImportanceScorer）

五维加权评估内容价值，决定是否存储、巩固优先级与遗忘策略：

| 信号 | 默认权重 | 描述 |
|------|----------|------|
| `weight_length` | 0.15 | 消息数量（封顶 20 条） |
| `weight_tool_calls` | 0.25 | 工具调用频次（封顶 10 次） |
| `weight_keywords` | 0.30 | 决策关键词（decided, chose, 记住, 决定 等） |
| `weight_depth` | 0.15 | 用户轮次深度（封顶 10 轮） |
| `weight_corrections` | 0.15 | 纠错标记（actually, wrong, 不对, 错了 等） |

评分低于 `min_threshold`（默认 0.3）的对话跳过巩固。权重可通过配置 `memory.importance` 覆盖。

## 工作记忆与 LRU

工作记忆在效果上类似 **带容量上限的缓存**：新 token 与工具结果进入窗口，触发阈值后与 `fastclaw-context` 协同做 **压缩/摘要**，避免无限增长。可将其理解为策略化的 **LRU / 滑动窗口** 语义（精确策略以运行时配置与上下文引擎为准）。

## 向量检索：SQLite + 可选 usearch

- 嵌入由 `memory.embedding` 配置：`provider` 为 `local`（默认，纯 Rust 本地模型）或 `remote`（OpenAI 兼容嵌入 API）。
- 向量索引路径等可在示例配置 `vectorIndexPath`、`vectorDimensions` 中与存储后端对齐；生产部署常用 **SQLite** 存元数据，**usearch**（若启用）加速近邻搜索。
- 亦可在配置中关闭向量（如 `none`），退化为关键词检索路径（以实际解析为准）。

## 语义图（petgraph）

语义层可将实体与关系组织为 **图结构**（实现上常用 `petgraph`），支持简单图遍历与影响分析类能力，并与向量检索互补：向量找相似片段，图找关联概念。

## Dreaming 流水线

`memory.dreamingIntervalSecs` 控制后台 **「做梦」周期**，包含三项增强功能：

1. **关系抽取** — 从情景摘要中提取实体关系（`is`、`uses`、`depends on`），存入语义图。
2. **事实抽取** — 识别偏好与选择模式（`prefers`、`chose`、`selected`），存为语义事实。
3. **嵌入回填** — 批量补全缺失嵌入向量的情景与事实，改善后续向量检索质量。
4. **重要性重评** — 对默认 0.5 分的情景使用 `ImportanceScorer` 重新评分。

间隔为 `0` 可关闭该周期任务。

示例（JSON5）：

```json5
{
  "memory": {
    "enabled": true,
    "dreamingIntervalSecs": 3600,
    "consolidationMinMessages": 6,
    "consolidationModel": "gpt-4o-mini",
    "importance": {
      "weightKeywords": 0.30,
      "weightToolCalls": 0.25,
      "minThreshold": 0.3
    },
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
- `consolidationModel` 建议配置为快速/廉价模型以减少巩固延迟。
- 记忆持久化于 SQLite，跨会话与进程重启均可保留。

更多实现细节见 [`../design/core-capabilities.md`](../design/core-capabilities.md) 与 [`../design/technical-design.md`](../design/technical-design.md)。
