# fastclaw-context

六层上下文拼装引擎：滚动压缩、记忆自动捕获与上下文窗口裁剪。

## 功能

- **上下文引擎** — `ContextEngine` 按层级组装系统提示、记忆注入、工具描述、会话历史等
- **滚动压缩** — `ContextCompactor` 对超长会话进行摘要压缩
- **关键词拦截** — `MemoryKeywordInterceptor` 检测用户消息中的记忆触发词（中英文），自动存储为语义事实并注入系统提示
- **上下文窗口裁剪** — `ContextEngine::fit_to_context_window` 确保消息总 token 不超出模型上下文限制。预留输出空间后，依次尝试 `ImportanceBased` 压缩（保留近期消息与工具结果）和滑动窗口截断（丢弃最早的非系统消息），永不丢弃系统消息或当前用户轮
- **Token 估算** — `estimate_messages_tokens` 基于 `chars/4` 启发式方法，计入 per-message overhead (~4 tokens) 与 tool_call JSON，快速无外部依赖

## ContextHook 生命周期

| 阶段 | Hook | 描述 |
|------|------|------|
| `on_ingest` | `AgentMemoryIngestHook` | RAG 注入：基于用户消息检索相关记忆 |
| `on_ingest` | `MemoryKeywordInterceptor` | 关键词自动捕获：检测「记住」「remember」等触发词 |
| `on_assemble` | 内置 | 组装系统提示、人格、工具描述 |
| `on_compact` | `CompactionHook` | 超长上下文压缩 |
| `on_after_turn` | `MemoryConsolidationHook`* | 会话自动摘要：LLM 总结关键决策并存储 |

*`MemoryConsolidationHook` 实际位于 `fastclaw-gateway` 以避免循环依赖。

## 上下文窗口裁剪流程

1. 计算 `budget = context_window - reserved_output`（预留输出空间默认为 `min(max_tokens, context_window / 4)`）
2. 若 `estimate_messages_tokens(messages) <= budget`，直接返回（无修改）
3. 应用 `ImportanceBased` 压缩（保留近期 + 工具结果 + 系统消息）
4. 若仍超预算，回退至滑动窗口：从最早的非系统消息开始丢弃，并插入 `[Earlier conversation history was truncated to fit context window]` 提示

## 关键导出

```rust
pub use engine::ContextEngine;
pub use compressor::{ContextCompactor, estimate_messages_tokens};
pub use keyword_interceptor::MemoryKeywordInterceptor;
```
