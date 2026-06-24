## Why

XiaoLin 已有 PromptEngine 静态/动态分区和 CacheBreakDetector 基建，但实际的 prompt cache 命中率极低（接近 0%）。核心原因：

1. **Gateway `insert(0)` 污染 prefix**：动态内容（skills、git snapshot、evolution skills）被注入到 prefix 最前面，导致稳定内容的字节前缀每轮都变
2. **Anthropic provider 未实现 `cache_control`**：无法利用显式分层缓存（4 个 breakpoint）
3. **历史消息被原地修改**：microcompact/dedup/budget 压缩改变已缓存的消息内容
4. **每轮动态内容在 system role 中间**：git snapshot、code_context 等导致前缀每轮不同

修复后预计：
- 稳态 turn（≥3）cache hit rate 从 <5% → **95-99%**（所有 provider）
- 节省 **70%+** prompt token 成本
- 降低首 token 延迟（缓存命中时 Anthropic 减少 ~1.5s）

## What Changes

### 消息组装重构
- 稳定内容（PromptEngine static sections）始终在 messages[0]
- Gateway 注入（skills、paths、MCP tools）从 `insert(0)` 改为 append
- 每轮必变内容（git snapshot、code_context、magic_docs、browser context）从 system prefix 移到最后一条 user message 的 `<system_context>` attachment
- 消除所有 `inject_system_block` 和 `messages.insert(0)` 调用

### Anthropic 4-Tier 分层缓存
- `system` 字段改为 `Vec<ContentBlock>`（每个 block 带独立 `cache_control`）
- 4 个 Breakpoint：Tier-1（纯模板, ttl:1h）→ Tier-2（session-stable, ttl:1h）→ Tier-3（tools 末项, ttl:1h）→ Tier-4（history 倒数第二条, ephemeral）
- `scope: 'global'` 为 P2 实验性功能（已知 bug + 3P 不兼容）

### DeepSeek 高命中率
- System prefix + tools byte-identical（排序确定性）
- 禁止 `insert(0)` 破坏前缀
- `reasoning_content` 原样回传
- 历史消息不可变原则（`cached_message_boundary`）

### Session 级缓存稳定性
- Sticky-on Latch：影响 cache key 的参数首次发送后锁定
- Tool Schema Session Memoize：工具 schema 序列化结果缓存，key=`blake3(schema_json)`
- Skills injection 首轮冻结 + 版本变化时刷新
- MCP instructions 事件驱动 invalidation（去掉 `cache_break: true`）

### 已缓存历史不可变
- 引入 `cached_message_boundary` 标记（基于 API 返回的 `cache_read_tokens > 0` 确认）
- microcompact/dedup/budget 压缩限制在 boundary 之后
- compaction 时合法重置 boundary

### CacheBreakDetector 基线（已完成）
- 填充真实 system_hash 和 tools_hash 数据
- 添加 cache hit/miss tracing 日志

## Capabilities

### New Capabilities
- `anthropic-4tier-cache`: Anthropic Messages API 4 层分级缓存，支持 scope/ttl/4 个 breakpoint
- `prefix-stable-assembly`: 消息组装保证稳定内容在 byte prefix 最前，动态内容不污染前缀
- `cached-history-immutability`: 已缓存的历史消息不可变保护机制
- `tool-schema-session-memoize`: 工具 schema 序列化结果 session 级缓存
- `sticky-on-latch`: 影响 cache key 的动态参数 session 级锁定
- `subagent-cache-strategy`: 子代理缓存隔离与共享策略

### Modified Capabilities
- `prompt-cache-optimization`: 从"只观测"升级为"观测 + 主动优化 + 分层缓存 + 不可变保护"
- `prompt-engine-sections`: `DYNAMIC_BOUNDARY` 拆分为 `CACHE_TIER1_BOUNDARY` + `CACHE_TIER2_BOUNDARY`
- `mcp-instructions`: 从每轮重新计算改为事件驱动 invalidation

## Impact

- **后端 crate**：
  - `xiaolin-agent`：llm.rs（Anthropic 4-Tier cache_control）、prompt_engine（双边界）、llm_call（boundary tracking）、mod.rs/turn_setup.rs（inject 改造）、post_tool.rs/unified_compact.rs/context_budget.rs（boundary 检查）
  - `xiaolin-gateway`：chat_pipeline.rs（7+ 个 insert(0) 改为 append）、ws/chat.rs（goal_instruction）
  - `xiaolin-context`：compressor.rs（幂等性保证）
- **API 兼容性**：对外 API 无变化，纯内部重构
- **Provider 影响**：
  - Anthropic：请求格式从 `system: String` → `system: Vec<ContentBlock>` with `cache_control`
  - DeepSeek/OpenAI：受益于 prefix 稳定性，无 API 格式变化
- **性能**：prompt token 成本 -70%，首 token 延迟 -1~2s（cache hit 时）
- **测试**：新增 ~15 个 unit tests + 4 个手动验证场景（见 tasks.md §12）
- **配置**：Sticky-on Latch 参数、ToolSchemaCache 容量、P2 scope:global feature flag
