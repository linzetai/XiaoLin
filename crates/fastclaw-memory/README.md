# fastclaw-memory

三层记忆系统、重要性评分与做梦巩固管线。

## 架构

| 层 | 模块 | 描述 |
|----|------|------|
| 工作记忆 | `working` | LRU 缓存，当前会话上下文 |
| 情景记忆 | `episodic` | 向量检索，跨会话经验回忆 |
| 语义记忆 | `semantic` | petgraph 知识图谱，事实与关系 |
| 重要性评分 | `importance` | 多信号加权评分，决定是否存储与优先级 |

- **嵌入 Provider** — 本地 (`hypembed`) 或远程嵌入生成
- **做梦管线** — `DreamingPipeline` 在低负载时巩固短期记忆到长期存储，自动抽取事实、回填嵌入、重评分
- **重要性评分** — `ImportanceScorer` 基于消息长度、工具调用、决策关键词、对话深度、纠错标记五维加权评估内容价值

## Feature Flags

- `local-embedding`（默认） — 启用 `hypembed` 本地嵌入
- `usearch-backend` — 启用 `usearch` 高性能向量索引

## 关键导出

```rust
pub use working::WorkingMemory;
pub use episodic::EpisodicMemory;
pub use semantic::SemanticMemory;
pub use dreaming::DreamingPipeline;
pub use embedding::EmbeddingProvider;
pub use importance::ImportanceScorer;
```
