# fastclaw-context

六层上下文拼装引擎：滚动压缩与记忆自动捕获。

## 功能

- **上下文引擎** — `ContextEngine` 按层级组装系统提示、记忆注入、工具描述、会话历史等
- **滚动压缩** — `ContextCompactor` 对超长会话进行摘要压缩
- **关键词拦截** — `MemoryKeywordInterceptor` 检测用户消息中的记忆触发词（中英文），自动存储为语义事实并注入系统提示

## ContextHook 生命周期

| 阶段 | Hook | 描述 |
|------|------|------|
| `on_ingest` | `AgentMemoryIngestHook` | RAG 注入：基于用户消息检索相关记忆 |
| `on_ingest` | `MemoryKeywordInterceptor` | 关键词自动捕获：检测「记住」「remember」等触发词 |
| `on_assemble` | 内置 | 组装系统提示、人格、工具描述 |
| `on_compact` | `CompactionHook` | 超长上下文压缩 |
| `on_after_turn` | `MemoryConsolidationHook`* | 会话自动摘要：LLM 总结关键决策并存储 |

*`MemoryConsolidationHook` 实际位于 `fastclaw-gateway` 以避免循环依赖。

## 关键导出

```rust
pub use engine::ContextEngine;
pub use compressor::ContextCompactor;
pub use keyword_interceptor::MemoryKeywordInterceptor;
```
