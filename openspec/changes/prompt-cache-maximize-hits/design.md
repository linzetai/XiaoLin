## Context

XiaoLin 的 LLM 调用链通过 `PromptEngine` 组装 system prompt，经 Gateway 注入 skills/MCP/paths 等内容后发送给 provider。当前架构有以下特点：

- **PromptEngine** 已实现静态/动态分区，用 `DYNAMIC_BOUNDARY` 标记分割，静态 section 被 memoize
- **CacheBreakDetector** 已实现但未接入真实数据（已修复，本次 change 的一部分）
- **CostTracker** 可追踪 `cache_read_tokens` / `cache_creation_tokens`
- **Anthropic provider** 将所有 System messages 拼为一个 `system: String`，无 `cache_control` block
- **Gateway 注入** 通过 `insert(0)` 将动态内容放在 prefix 最前面，破坏 prefix 稳定性
- **每轮动态内容**（git snapshot, code_context, evolution skills）在 system prefix 中间，导致前缀每轮不同

Provider 侧缓存机制：
- Anthropic：显式 `cache_control` + 自动 prefix（需要 byte-stable prefix）
  - 支持 `scope: 'global'`（跨用户/跨 session 共享，实验性，需 beta header）
  - 支持 `ttl: '1h'`（**已 GA，无需 beta header**，2x base input pricing）
  - 默认 TTL 为 5 分钟（ephemeral, 1.25x pricing）
- DeepSeek：自动 prefix cache（返回 `prompt_cache_hit_tokens`）
  - **纯自动，无需任何 API 标记**——只要 prefix byte-identical 就命中
  - TTL **远超 Anthropic**：几小时到几天（vs Anthropic 的 5min/1h）
  - 64-token 粒度，实际可靠命中需 ~1024 tokens
  - **公共前缀检测**：两次不同请求（如 A+B 和 A+C）后，系统自动发现公共前缀 A 并持久化
  - 多轮对话天然命中：第 N 轮请求的前缀 = 第 N-1 轮完整请求 → 自动 cache hit
  - **跨 session 天然支持**：同账号 + 同 prefix = 命中，TTL 覆盖数小时到数天
  - 请求边界持久化：每次请求在 user input 末尾 和 model output 末尾 各生成一个缓存前缀单元
- OpenAI：自动 prefix cache（GPT-4o+，静默的）

**Claude Code 架构参考**（2026-06 源码分析）：
- 使用 `SYSTEM_PROMPT_DYNAMIC_BOUNDARY` 分割 static（scope: global）和 dynamic（scope: null）
- static prefix 用 `scope: 'global'` + `ttl: '1h'` → 跨 session、跨用户命中
- User context（CLAUDE.md）作为 user message 注入而非 system prompt
- System context（git status）append 到 system prompt 尾部（boundary 之后）
- Beta headers 使用 "sticky-on latch" 防止 mid-session 翻转破坏 cache key
- Tool schemas session 内 memoize，防止 feature flag 翻转导致 schema 字节变化
- Messages 级只放 1 个 cache_control（最后一条消息）
- Cache break detection 精细到 per-tool hash diff

## Goals / Non-Goals

**Goals:**
- 稳态 turn（turn≥3, <5min gap）token 加权 cache hit rate 达到 **95–99%**（所有 provider）
- 全 session 平均（含首轮 creation + TTL 过期）达到 **90–95%**
- DeepSeek：利用长 TTL（数小时~数天）+ 公共前缀检测，达到**跨 session 95%+ 命中**
- Anthropic：利用 `ttl: '1h'`（GA），Tier-1/2/3 **跨 session 命中**（scope:global 为 P2 可选增强）
- 将 DeepSeek/OpenAI 的自动 prefix cache hit rate 提升到 95%+
- 在不改变用户体验的前提下减少 prompt token 成本 70%+
- 充分利用 Anthropic 4 个 breakpoint 实现分层缓存
- 提供可观测的 cache metrics 用于持续优化

**Non-Goals:**
- 不做 LLM response 缓存（不同于 prompt prefix 缓存）
- 不修改 OpenAI 的请求格式（OpenAI 无显式 cache API）
- 不做 Google/Gemini context caching（API 差异太大，单独 change）
- 不重构整个 PromptEngine 架构（只调整注入顺序和 provider 格式化）

## Decisions

### D1: 消息组装顺序从 "gateway first" 改为 "stable first"

**选择**：PromptEngine 的 static sections 始终在 messages[0]，gateway 注入 append 到后面

**替代方案考虑**：
- A) 维持 insert(0) 但冻结 gateway 注入 → 不可行，skills 依赖 touched_paths 是动态的
- B) 把所有 system 内容合并为一个 message → 失去按 block 标记 cache_control 的能力
- C) (**选择**) 改变注入顺序，stable 在前 → 简单直接，所有 provider 受益

**理由**：自动 prefix cache（DeepSeek/OpenAI）只看字节前缀，stable 内容在前 = 更多 bytes 命中。

### D2: Anthropic 4-Tier 分层缓存（充分利用 4 个 breakpoint）

**选择**：使用 Anthropic 的全部 4 个 cache breakpoint + `scope` + `ttl` 实现分层跨 session 缓存

**布局**：
```json
{
  "system": [
    {"type":"text", "text":"<TIER-1: 纯模板>", "cache_control":{"type":"ephemeral","scope":"global","ttl":"1h"}},
    {"type":"text", "text":"<TIER-2: session-stable>", "cache_control":{"type":"ephemeral","ttl":"1h"}}
  ],
  "tools": [..., {"name":"last_tool", "cache_control":{"type":"ephemeral","ttl":"1h"}}],
  "messages": [
    ...(history)...,
    {"role":"assistant/user", "content":[{..., "cache_control":{"type":"ephemeral"}}]},
    {"role":"user", "content":[{"type":"text","text":"<最新消息 + system_context>"}]}
  ]
}
```

**Scope 策略**（参考 Claude Code）：
- **Tier-1（纯模板）** → `scope: 'global'`：所有 XiaoLin 实例（同版本）共享此缓存，跨 session、跨用户
  - ⚠️ **实验性**：`scope: 'global'` 需要 beta header `prompt-caching-scope-2026-01-05`，目前仅 firstParty API 支持，且有已知 bug（Claude Code issue #49139：服务端可能将缓存放入 5min bucket 而非 1h bucket）
  - 3P proxy（LiteLLM/Vertex/Bedrock）全部报 400 错误
  - 实施优先级为 **P2 实验性**，不作为 99% 命中率的必要条件
- **Tier-2（session-stable）** → 无 scope（default, per-request namespace）：包含 cwd/model/memory 等用户特定内容
- **Tier-3（tools）** → 无 scope：工具集可能因用户配置不同
- **Tier-4（history）** → 无 scope：对话历史永远不同

**TTL 策略**：
- Tier-1~3：使用 `ttl: '1h'`（**已 GA**，无需 beta header，直接在 cache_control 中设置即可）
  - 费用：2x base input price（vs 1.25x for default 5min）
  - 约束：longer-TTL entries 必须出现在 shorter-TTL entries 之前
- Tier-4：使用默认 ephemeral（5min），因为对话历史只在同 session 内有价值
- 如果 provider 不支持（返回 400），fallback 到标准 ephemeral

**跨 Session 命中矩阵**：
| Tier | 同 session | 跨 session（< 1h） | 跨用户 |
|------|-----------|-------------------|--------|
| 1（ttl: 1h） | ✅ 100% | ✅ 100%（prefix 不变 + 1h TTL） | ❌（scope:global 实验性） |
| 2（ttl: 1h） | ✅ 100% | ✅ ~95%（memory 不变时） | ❌ |
| 3（ttl: 1h） | ✅ 100% | ✅ ~95%（同配置） | ❌ |
| 4（ephemeral） | ✅ 100% | ❌ 0% | ❌ |

**4 层缓存对应**：
- BP#1（system Tier-1）：纯模板，几乎永不失效（intro, doing_tasks, actions, tone, output_efficiency, frc）
- BP#2（system Tier-2）：session-stable 内容（system, using_tools, environment, language, session_guidance, memory, mcp_instructions, gateway injections）
- BP#3（tools 末项）：排序后的工具定义，仅在 tool_search 激活时变
- BP#4（messages 倒数第二条）：完整对话历史，仅最新 turn 不缓存

**替代方案**：
- A) 单一 breakpoint → 只缓存 ~2.5k tokens 模板，token 加权命中率仅 2-3%
- B) 两个 breakpoint（system + tools）→ 缓存 system + tools ~8k tokens，但不缓存 history（对长对话效果差）
- C) (**选择**) 四层分层 → 几乎所有 input tokens 都被缓存，仅最新 user message 是 uncached

**关键洞察**：BP#4 是 99% 的关键。在 20 轮对话中，history 可达 50-100k tokens，如果不缓存 history 则 token 加权命中率远低于 50%。

### D2b: DYNAMIC_BOUNDARY 拆分为 Tier-1/Tier-2 双边界

**选择**：现有的 `DYNAMIC_BOUNDARY` 拆成 `CACHE_TIER1_BOUNDARY` 和 `CACHE_TIER2_BOUNDARY`

- Tier-1 后边界：纯模板结束点
- Tier-2 后边界：session-stable 内容结束点（之后的内容不进 system role）

这让 Anthropic provider 的 `convert_messages` 可以精确识别两个 breakpoint 位置。

### D3: 每轮动态内容移出 system role（零污染原则）

**选择**：所有每轮必变的内容作为最后一条 user message 的 `<system_context>` attachment 注入，而非 system role message。同时**禁止 `inject_system_block` append 到 messages[0]**。

**必须移出 system role 的内容**：
- git snapshot（`turn_setup.rs:104`）
- code_context（`prompt_sections/dynamic.rs:457`）
- evolution skills（`mod.rs:1115`）
- project hints（`turn_setup.rs:97`）— 或首轮冻结到 Tier-2
- browser context（gateway `insert(0)`）
- task decomposer plan（`turn_setup.rs:138`）
- magic_docs（如有）

**理由**：
- 这些内容每轮必变，放在 system prefix 中会 100% 导致 BP#1 和 BP#2 cache miss
- 放在 user message 中不影响模型行为（模型同样能看到，用 XML 标签明确为系统上下文）
- `inject_system_block`（`mod.rs:1117-1133`）当前 append 到 `messages[0]` 是最大的缓存杀手

**风险**：部分模型对 system role vs user role 中的指令权重不同。通过保留 `<system_context>` XML 标签明确标记为系统级上下文来缓解。

### D4: mcp_instructions 改为事件驱动 invalidation

**选择**：去掉 `cache_break: true`，在 MCP server connect/disconnect 事件时调用 `invalidate_sections(&["mcp_instructions"])`

**理由**：MCP server 列表在 session 中 99% 不变，每轮 recompute 毫无意义。事件驱动 invalidate 既保证正确性又不浪费缓存。

### D5: Skills injection 冻结策略

**选择**：Gateway 层的 skills prompt 在 session 首轮计算后缓存，后续轮次仅在 skill registry 版本变化或 explicit invalidation 时重新计算

**替代方案**：
- A) 每轮都依据 touched_paths 重新计算 → 当前行为，导致几乎每轮都不同
- B) (**选择**) 首轮冻结 + 版本变化时刷新 → 稳定性极高，只在真正有变化时更新

### D5b: DeepSeek 高命中率策略

**目标**：利用 DeepSeek 的自动缓存特性（长 TTL + 公共前缀检测），实现稳态 turn ≥ 95% cache hit（同 session 内多轮对话）

> ⚠️ 注意：DeepSeek 的缓存是 best-effort 且不保证 100%。服务端可能因负载、冷启动等原因 miss。99% 是理论上限而非承诺。我们的目标是消除所有**客户端可控**的 cache bust 因素。

**关键约束**：
- DeepSeek 只看**从 token 0 开始的精确前缀**，中间任何字节变化 = 全部 miss
- 64-token 粒度，~1024 tokens 才开始可靠命中
- **多轮对话天然命中**：Turn N 的消息是 Turn N-1 的超集（追加 assistant 回复 + 新 user message）
- **公共前缀检测**：即使首次请求未命中，第二次相同前缀后系统会持久化公共部分

**必须保证的不变式（DeepSeek specific）**：
1. **System prompt + Tools 必须作为请求的最前部**，且 session 内 byte-identical
2. **禁止在 messages[0] 前面 insert 任何动态内容**（`insert(0)` 是 DeepSeek cache 的头号杀手）
3. **`reasoning_content` 必须原样回传**：DeepSeek 返回的 `reasoning_content` 字段在后续 turn 中必须作为 assistant message 的一部分传回，否则前缀不一致
4. **多轮消息追加而非重建**：每轮只 append 新消息，不修改/删除历史消息
5. **Tool 定义排序确定性**：tool_defs 按 name 排序后序列化，保证跨轮不变

**DeepSeek vs Anthropic 缓存对比**：
| 维度 | DeepSeek | Anthropic |
|------|----------|-----------|
| 控制方式 | 纯自动 | 显式 cache_control |
| TTL | 数小时~数天（best-effort） | 5min (default) / 1h (GA, 2x pricing) |
| 跨 session | ✅ 天然支持（长 TTL） | ✅ 1h TTL 实现（无需 beta header） |
| 跨用户 | ✅ 公共前缀自动共享 | ❌ scope:global 实验性/不稳定 |
| 最小缓存单位 | 64 tokens | ~1024 tokens |
| 公共前缀检测 | ✅ 自动 | ❌ 不支持 |
| 需要做的 | 保持 prefix byte-stable | 标记 cache_control breakpoints |
| 风险 | best-effort 不保证 100% | 4 个 breakpoint 限制 |

**DeepSeek 高命中率的实现路径**：
- 任务 2（inject_system_block 改造）完成后，system prefix 零污染 → Turn ≥ 2 自动命中
- 任务 3（Gateway 注入改 append）完成后，prefix 字节稳定 → Turn 1 后持久化
- 多轮对话：Turn N 的 prefix = Turn N-1 完整内容 → DeepSeek 请求边界持久化自动覆盖
- **DeepSeek 不需要任何额外的 API 标记或 beta header**——只要我们保证 prefix 稳定即可
- ⚠️ 即使客户端完美实现，DeepSeek 侧仍可能因冷缓存/服务器负载等原因 miss，这不属于可控范围

### D6: Sticky-on Latch 防止 mid-session cache bust（参考 Claude Code）

**选择**：影响 cache key 的动态参数（如 beta headers, model family, sandbox 模式）一旦首次发送即 "锁定" 整个 session，后续变化不改变已发送的值。

**理由**：Claude Code 实践表明，mid-session 切换（如 autoMode 激活、fast_mode 切换）导致 request header 变化，会完全 bust ~50-70K tokens 的缓存。锁定后保持 cache key 稳定。

**实现**：
- 在 session-level state 中保存 latched 值（`model_family_latched`, `sandbox_mode_latched` 等）
- 只在 session 开始时计算初始值
- `/clear` 或 `/compact` 时重置所有 latch

### D6b: 已缓存历史消息不可变原则（Cached History Immutability）

**选择**：引入 `cached_message_boundary` 标记——一旦某条消息被 LLM call 中作为 prefix 缓存过，后续 iteration 中**禁止原地修改**其内容。

**精确语义**：
- `cached_message_boundary: usize` = 上一次 LLM API 调用成功返回且 `cache_read_tokens > 0` 时的 `messages.len()`
- 只有在 API 返回确认缓存命中后才前移 boundary（不依赖推测）
- compaction 或 `/clear` 时重置为 0
- 用 `AtomicUsize` 或 session-level lock 保护，防止快速连续请求的 race condition

**关键约束**：
- `post_tool` microcompact/dedup 只允许修改最新一轮的 tool results（boundary 之后）
- `unified_pre_query_compact` 如需修改 boundary 之前的消息，视为合法 cache 失效 → 通知 CacheBreakDetector
- `apply_message_budget` 只压缩 boundary 之后的 tool content
- `strip_image_content` / `ensure_valid_assistant_messages` 必须幂等（已处理过的不再改）

**理由**：
- DeepSeek 从 token 0 精确匹配 prefix，中间任何一条消息 byte 变化 = 从该点起全部 miss
- Anthropic BP#4 放在倒数第二条消息 = 最新一条不缓存；已缓存的部分不应被修改
- Claude Code 的 `cached microcompact` 使用 `cache_edits` API（API 层面的增量编辑），而非直接修改消息内容

**boundary 更新规则**：
- 每次 LLM call 完成后，boundary 前移到 `messages.len() - 1`（最新 assistant response 之前的所有消息都被缓存了）
- compaction 清除 boundary（因为整表被替换了）

**影响最大的 3 个热路径**：
1. `post_tool.rs:95-96`：每 tool round 后 microcompact 改旧 tool content → 限制到 boundary 之后
2. `unified_pre_query_compact`：每 iteration 执行 → 标记为 expected cache break
3. `context_budget.rs:346,406`：budget 压缩 → 限制到 boundary 之后

### D7: Tool Schema Session-level Memoize（参考 Claude Code）

**选择**：工具 schema 在 session 内计算一次后缓存，后续请求复用相同字节。

**理由**：Claude Code 发现 GrowthBook feature flag 翻转导致 tool description 内容变化（如条件性文本 "when X is enabled, ..."），即使功能相同也会导致 tools JSON 字节变化 → cache break。

**实现**：
- `ToolSchemaCache`: HashMap<tool_name, (blake3_hash, serialized_schema_json)>
- cache key = `blake3(schema_json)` —— 确保 schema 内容实际变化才 invalidate（比 version_hash 更可靠）
- 首次序列化后缓存，后续直接使用缓存值
- 仅在 tool registry version 变化时清除
- **MCP 工具特殊处理**：MCP server reconnect/schema_changed 事件触发对应 tool 的精确 invalidation（不全量清除）
- **动态 tool 排除**：browser_use 的 tab-specific 工具等每轮可变的工具排除在 cache 之外

### D8: Beta Header `prompt-caching-scope-2026-01-05`（P2 实验性）

**选择**：当使用 Anthropic firstParty API 时，可选发送此 beta header，启用 `scope` 字段支持。

> ⚠️ 降级为 P2：已知 Anthropic 服务端 bug（issue #49139）可能将 global scope 缓存错误放入 5min bucket。3P proxy 一律返回 400。实际上 `ttl: '1h'` 已 GA 无需此 header，此 header 仅为 `scope` 字段所需。

**Eligibility 检查**（简化版）：
- 1h TTL：当前直连 Anthropic API 即可尝试（如 API 返回 400 则 fallback 到无 ttl）
- global scope：仅当 Tier-1 内容是纯模板（无用户敏感信息）时才使用

**Fallback 策略**：
- 如果 API 不支持 scope/ttl（返回 400），退化为标准 `{"type":"ephemeral"}` 并记录 warn
- 3P proxy（如 LiteLLM）通常不支持 beta 字段，需要检测并跳过

### D9: Subagent 缓存策略

**背景**：XiaoLin 支持子代理（subagent）模式，子代理有独立的对话线程但可能共享相同的 system prompt 模板。

**设计决策**：
1. **Tier-1 共享**：子代理若使用相同的 system prompt 模板版本，其 Tier-1 prefix 与父 session 天然 byte-identical，Anthropic 的 `ttl: '1h'` 可跨 session/subagent 命中
2. **Tier-2 独立**：子代理有独立的 cwd/memory/session_guidance → Tier-2 不可共享
3. **Tools 按 agent_id 隔离**：子代理通常只暴露 read-only tools → ToolSchemaCache 按 `(session_id, agent_id)` 隔离
4. **短生命周期优化**：子代理通常只有 1-3 turns，使用 ephemeral（5min）而非 1h TTL（避免为短命对话支付 2x pricing）
5. **CacheBreakDetector**：每个 subagent 独立计数器，不与父 session 混淆

**DeepSeek 场景**：子代理的 system prompt + tools 与父不同 → 无法共享 prefix cache，但子代理自身的多轮对话仍可命中。

## Risks / Trade-offs

- **[Risk] 模型行为差异** → user message 中的系统上下文可能权重稍低  
  → Mitigation: 使用 `<system_context>` XML 标签 + 保留核心指令在 system role 中

- **[Risk] Anthropic API 版本兼容性** → content block 格式需要 `anthropic-version: 2023-06-01+`  
  → Mitigation: 检测 API 版本或 model family，不支持时 fallback 到 String 格式

- **[Risk] Skills 首轮冻结可能遗漏后续新文件激活的条件 skill**  
  → Mitigation: 在 tool_search 激活新 skill 时同步触发 skills prompt invalidation

- **[Risk] CacheBreakDetector hash 使用 DefaultHasher 不跨进程稳定**  
  → Acceptable: detector 只做同进程内连续调用的对比，不持久化

- **[Trade-off] 多一次 system message 分割** → Anthropic provider 代码复杂度增加  
  → Acceptable: 收益（缓存节省）远大于维护成本

## Open Questions

1. ~~CacheBreakDetector 传入真实数据~~ → 已完成
2. ~~Anthropic 的 `cache_control` 是否需要 beta header~~ → 
   - `ttl: '1h'`：**不需要 beta header**，已 GA，直接在 `cache_control` 对象中使用
   - `scope: 'global'`：需要 beta header `prompt-caching-scope-2026-01-05`，但有已知 bug（may misroute to 5min bucket），且 3P proxy 不支持。降级为 P2 实验性
3. 是否需要为不同的 Anthropic 模型（Claude 3.5 vs Claude 4）使用不同的缓存策略？
4. DeepSeek 的自动缓存是否有最小 prefix 长度要求？（据悉约 1024 tokens）
5. `cached_message_boundary` 在并发场景（用户快速连续发送消息）下是否存在 race condition？→ 已在 D6b 中定义精确语义：boundary = 上次 API 确认缓存命中时的 message count，用 AtomicUsize 保护
6. Subagent（独立对话线程）是否可以继承父 session 的 Tier-1/Tier-2 缓存？→ 已在 D9 中定义策略：Tier-1 天然共享（same template），Tier-2/tools 独立
7. XiaoLin 当前是否有 3P proxy 场景？如有需检测并跳过 scope/ttl/beta 字段
8. `scope: 'global'` 的前提是 Tier-1 内容不含用户敏感信息（当前纯模板满足），需要确认 system_base_prompt 自定义场景是否安全
