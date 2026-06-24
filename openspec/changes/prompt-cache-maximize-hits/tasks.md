## 1. 基线观测（已完成）

- [x] 1.1 CacheBreakDetector 传入真实 system hash 和 tools hash（llm_call.rs）
- [x] 1.2 添加 cache hit/miss tracing 日志（debug 级 hit 百分比，warn 级 break 原因）

## 2. System prefix 零污染：inject_system_block 改造

- [x] 2.1 新增 `inject_user_context(messages, block)` 函数：将 block 作为 `<system_context>` 注入最后一条 user message 而非 system role
- [x] 2.2 `turn_setup.rs`：git snapshot 改用 `inject_user_context` 替代 `inject_system_block`
- [x] 2.3 `turn_setup.rs`：project hints 改用 `inject_user_context`（或首轮冻结到 session cache）
- [x] 2.4 `turn_setup.rs`：task decomposer plan 改用 `inject_user_context`
- [x] 2.5 `mod.rs`：evolution skills 注入改为 `inject_user_context` 而非 append 到 messages[0]
- [x] 2.6 `mod.rs`：`inject_tool_recovery_guidance` 从 `insert(0)` 改为 `inject_user_context`（仅在 tool call 错误恢复时触发）
- [x] 2.7 `turn_setup.rs`：`magic_docs` 注入改为 `inject_user_context`（动态内容，依赖当前查询）
- [x] 2.8 `turn_setup.rs`：`token_budget` 提示改为 `inject_user_context`（每轮变化的数值）
- [x] 2.9 `ws/chat.rs`：`goal_instruction` 的 `messages.insert(0)` 改为 append 到 Tier-2 system messages 末尾（goal 在 session 内稳定）
- [x] 2.10 审查所有 `inject_system_block` 调用方，确认无遗漏（`rg "inject_system_block\|messages\.insert(0"` 零结果）

## 3. Gateway 注入顺序重构

- [x] 3.1 `chat_pipeline.rs`：`inject_skills_prompt` 从 `insert(0)` 改为 append（在 PromptEngine system messages 之后）
- [x] 3.2 `chat_pipeline.rs`：`inject_runtime_paths_prompt` 同理改为 append
- [x] 3.3 `chat_pipeline.rs`：`inject_browser_context_prompt` 改为 `inject_user_context`（每轮变的内容）
- [x] 3.4 `chat_pipeline.rs`：`inject_mcp_tools_prompt` 改为 append 到 Tier-2 区域
- [x] 3.5 去重 MCP instructions：PE `mcp_instructions_section` 与 gateway `inject_mcp_instructions_delta` 二选一，避免重复
- [x] 3.6 `chat_pipeline.rs`：slash skill activation prompt 从 `insert(0)` 改为 append（激活态在 session 内稳定）
- [x] 3.7 `chat_pipeline.rs`：slash command hints 从 `insert(0)` 改为 `inject_user_context`（动态提示，每轮不同）
- [x] 3.8 `chat_pipeline.rs`：dynamic role prompts（如 agent persona 切换）改为 Tier-2 append（session 内稳定）
- [x] 3.9 审查 `chat_pipeline.rs` 所有 `messages.insert(0` 调用，确认全部消除（`rg "messages\.insert\(0" crates/xiaolin-gateway` 零结果）

## 4. PromptEngine 分层重构

- [x] 4.1 拆分 `DYNAMIC_BOUNDARY` 为 `CACHE_TIER1_BOUNDARY` 和 `CACHE_TIER2_BOUNDARY`
- [x] 4.2 `frc_section` 从 dynamic_sections 移到 static_sections（纯模板，属于 Tier-1）
- [x] 4.3 明确 Tier-1 sections（纯模板）: intro, doing_tasks, actions, tone_and_style, output_efficiency, frc
- [x] 4.4 明确 Tier-2 sections（session-stable）: system, using_tools, session_guidance, environment, language, memory, mcp_instructions, token_budget
- [x] 4.5 `build_system_prompt` 输出顺序改为: Tier-1 + TIER1_BOUNDARY + Tier-2 + TIER2_BOUNDARY（Tier-2 之后无内容进入 system role）
- [x] 4.6 `build_messages` 识别双边界标记，拆成两个 System messages

## 5. MCP instructions 事件驱动 invalidation

- [x] 5.1 `mcp_instructions_section` 的 `cache_break` 改为 `false`
- [x] 5.2 在 MCP server connect/disconnect 事件处调用 `prompt_engine.invalidate_sections(&["mcp_instructions"])`
      （实现方式：`ToolRegistry` 新增 `mcp_instructions_version`，`set/remove_mcp_instructions` 仅在内容真正变化时 bump；
      `AgentRuntime::build_messages` 比对版本号做事件驱动失效，避免跨 crate 传递 prompt_engine 句柄）
- [x] 5.3 `code_context_section` 从 dynamic_sections 移除（Phase 1 已完成，内容经 `inject_user_context` 注入）

## 6. Session 级冻结与 per-session section 确定性（见 design.md D4b）

> **设计修正**：`cache_break` 仅控制内部 CPU memoize，与 provider 缓存无关；`prompt_engine` 是进程级全局单例。
> 因此 per-session 内容必须**每轮确定性重算**（`cache_break: true`），而非全局 memoize + 按 session 失效
> （后者在全局引擎上会污染其它 session，且重算已天然处理新鲜度）。

- [x] 6.2 ~~`tool_search` 激活后 `invalidate_sections`~~ **SUPERSEDED**：`session_guidance`/`using_tools`/`system` 改为每轮确定性重算（`cache_break: true`），tool_search 激活后下一轮自动反映，无需显式失效
- [x] 6.3 ~~`memory_store` 后 `invalidate_sections(&["memory"])`~~ **SUPERSEDED**：`memory_section` 改 `cache_break: true`，memory 更新下一轮自动反映
- [x] 6.4 ~~Plan mode 切换 `invalidate_sections(&["session_guidance"])`~~ **SUPERSEDED**：`session_guidance_section` 改 `cache_break: true`，plan-mode 切换下一轮自动反映
- [x] 6.6 （新增）`environment` / `language` / `token_budget` 同步改 `cache_break: true`，修复 per-request 内容（cwd 等）被全局 memoize 泄漏的预存 bug
- [x] 6.1 Skills injection per-session 冻结（含 `usage_counts` + `touched_paths` 漂移稳定）
      实现：`RuntimeState.skills_prompt_cache: DashMap<session_id, SkillsPromptCacheEntry>`，key=session_id，
      失效信号 = agent skill registry 的 `Arc::as_ptr`（reload 后变化）。命中即复用冻结 prompt，使 Tier-2 前缀字节稳定；
      容量上限 `SKILLS_PROMPT_CACHE_MAX=512` + 按 `last_access_ms` 淘汰（规则 #27）；get_mut 守卫不跨重算持有（规则 #45）
- [ ] 6.5 `session_start_date` per-session 冻结 → **移交 Phase 4 §9**（需 session-scoped ctx 状态；当前经 §6.6 重算已确定性到「天」粒度，仅跨午夜 session 会刷新一次，低优先级）

## 7. Anthropic 4-Tier cache_control 对接 — ⏭️ DEFERRED（用户决定暂不接入 Anthropic 显式缓存）

> **决定（2026-06-24）**：暂不实现 Anthropic 显式 `cache_control` 适配。
> 当前重点是 **provider 自动前缀缓存**（DeepSeek/OpenAI），其命中只依赖「发送字节稳定的前缀」，
> 而前缀稳定性已由 Phase 1（Tier 分层 + 零污染注入）+ Phase 2（§5/§6 确定性）保证。
> Anthropic 的 `ttl`/`scope` 是在此基础上的**额外增益**，待自动缓存验证完善后再行接入。
> 保留以下任务作为未来 backlog。

- [ ] 7.1 `llm.rs`：新增 `AnthropicSystemBlock` struct 支持 `cache_control` 字段（含 `scope` 和 `ttl`）
- [ ] 7.2 `llm.rs`：`AnthropicRequest.system` 从 `Option<String>` 改为 `Option<Vec<AnthropicSystemBlock>>`
- [ ] 7.3 `llm.rs`：`convert_messages` 识别 TIER1/TIER2 boundary，生成 2 个 system blocks（各带 cache_control）
- [ ] 7.4 `llm.rs`：Tier-1 block 添加 `cache_control: {type: "ephemeral", ttl: "1h"}`（scope:global 为 P2 可选，默认不启用）
- [ ] 7.5 `llm.rs`：Tier-2 block 添加 `cache_control: {type: "ephemeral", ttl: "1h"}`（无 scope — 含用户特定内容）
- [ ] 7.6 `llm.rs`：tools 数组最后一项添加 `cache_control: {type: "ephemeral", ttl: "1h"}`（BP#3）
- [ ] 7.7 `llm.rs`：messages 中倒数第二条非 user 消息添加 `cache_control: {type: "ephemeral"}`（BP#4 — history cache，无 1h TTL）
- [ ] 7.8 `llm_call.rs`：Anthropic provider 时 `has_cache_control` 改为 true
- [ ] 7.9 Fallback：对不支持 cache_control 的模型/proxy 退化为 plain String

> **注意**：虽然不接入 Anthropic cache_control，但 §4 定义的 `CACHE_TIER1_BOUNDARY` / `CACHE_TIER2_BOUNDARY`
> 标记**仍需保留**——它们对 provider 自动缓存同样有益（保证模板/会话稳定段在前、动态段在后），
> 且是未来接入 Anthropic 的前置条件。

## 8. Beta Header 与 Scope 支持（P2 实验性）— ⏭️ DEFERRED（随 §7 一并暂缓）

> ⚠️ `scope: 'global'` 功能为实验性，已知有 bug 且 3P proxy 不支持。随 §7 一并暂缓。

- [ ] 8.1 新增 `prompt-caching-scope-2026-01-05` beta header，当 provider 为 Anthropic firstParty 时添加
- [ ] 8.2 检测 API 是否支持 scope/ttl：首次请求如返回 400 则 fallback 并标记 session 内不再尝试
- [ ] 8.3 `scope: 'global'` 安全性检查：确认 Tier-1 内容不含用户敏感信息（如自定义 system_base_prompt 则降级为无 scope）
- [ ] 8.4 3P proxy 检测：若 base_url 非 api.anthropic.com 则跳过 scope/ttl/beta 字段
- [ ] 8.5 监控 Anthropic issue #49139（scope misroute bug）修复状态，修复后再考虑 promote 到 P1

## 9. Sticky-on Latch 机制

- [ ] 9.1 在 session state 中新增 `latched_values: HashMap<String, String>` 字段
- [ ] 9.2 影响 cache key 的参数首次发送时锁定：model_family, sandbox_mode 等
- [ ] 9.3 tool_search 启用/禁用状态在 session 内一旦启用即锁定（防止 tool 列表翻转）
- [ ] 9.4 `/clear` 或 `/compact` 时重置所有 latch

## 10. Tool Schema Session Memoize

- [ ] 10.1 新增 `ToolSchemaCache`：session 内缓存每个 tool 的序列化 JSON
- [ ] 10.2 首次序列化后缓存，后续请求复用相同字节（防止动态 prompt 变化导致 tool JSON 字节变化）
- [ ] 10.3 cache key = `blake3(tool.name + schema_json)`（确保 schema 实际变化时才 invalidate，而非依赖不稳定的 version_hash）
- [ ] 10.4 registry version 变化时清除整个 cache
- [ ] 10.5 MCP 工具处理：MCP server reconnect/schema_changed 事件触发对应 tool 的 cache invalidation（不整体清除）
- [ ] 10.6 动态 tool（如 browser_use 的 tab-specific 工具）排除在 cache 之外，每次重新序列化

## 11. DeepSeek 专项优化与验证

- [ ] 11.1 验证 `reasoning_content` 透传完整性：确保 assistant message 的 reasoning_content 原样保留在后续请求中（已实现，需单测覆盖）
- [x] 11.2 tool_defs 排序确定性：`filter_tool_definitions` 收口处调用 `sort_tool_definitions_by_name` 按 name 排序，
      消除 `HashMap.values()` 顺序非确定，保证跨轮/跨 session/跨进程 byte-identical（`demote_tools_for_plan_mode` 仅 retain 保序）
- [ ] 11.3 messages 历史只追加不修改：审查所有 `messages.retain/remove/swap/truncate` 调用，确认不会改变已发送消息的内容（compaction 除外）
- [ ] 11.4 compaction 后 baseline reset：compaction 改变了消息前缀，需要在 CacheBreakDetector 中 reset prevCacheReadTokens（避免误报）
- [ ] 11.5 新增集成测试：模拟 3 轮 DeepSeek 对话，验证 Turn 2/3 的 prompt_cache_hit_tokens > 0
- [ ] 11.6 验证 gateway `insert(0)` 消除后的效果：确认所有 7 个 `chat_pipeline.rs` 的 insert(0) 改为 append 后，DeepSeek cache hit 提升

## 11b. 历史消息不可变性（Cached History Immutability）

核心规则：被纳入 cache prefix 的历史消息禁止原地修改。

- [ ] 11b.1 `post_tool` microcompact/dedup：限制只修改"最新一轮"的 tool results（未被上一次 LLM call 缓存的部分）
- [ ] 11b.2 `unified_pre_query_compact`：如果需要修改已缓存区域的消息，在修改前调用 `cache_detector.notify_compaction()`（合法失效标记）
- [ ] 11b.3 `apply_message_budget`（context_budget.rs）：限制只压缩 BP#4 之后的 tool results
- [ ] 11b.4 `time_based_microcompact`：同 11b.1，只修改最新轮次的 tool content
- [ ] 11b.5 `strip_image_content` / `ensure_valid_assistant_messages`：这些在每次 LLM call 前执行，应保证幂等——如果上一轮已 strip，本轮不应再次修改（已 strip 的消息不变）
- [ ] 11b.6 引入 `cached_message_boundary` 标记：在 messages 中记录"到此为止的消息已被缓存"，compaction 操作必须检查此标记

## 12. 验证与回归

- [ ] 12.1 `cargo test -p xiaolin-agent` 全部通过
- [ ] 12.2 新增 unit test：验证 Anthropic 请求 JSON 含 4 个 cache_control breakpoint（含 scope/ttl 字段）
- [ ] 12.3 新增 unit test：验证 inject_user_context 不改变 system messages
- [ ] 12.4 新增 unit test：验证 Tier-1/Tier-2 sections 在 2 连续 build_system_prompt 间 byte-identical
- [ ] 12.5 新增 unit test：验证默认情况下 Tier-1 无 scope 字段；P2 启用后才有 `scope: "global"`
- [ ] 12.6 新增 unit test：验证 3P proxy 场景下 scope/ttl/beta header 均被跳过
- [ ] 12.7 新增 unit test：验证 Sticky-on Latch 后切换不改变实际发送的 cache key
- [ ] 12.8 新增 unit test：验证 ToolSchemaCache 在 registry version 不变时返回 byte-identical JSON
- [ ] 12.9 手动测试：Anthropic provider 多轮对话，日志确认 cache_read_tokens > 0 且 hit_pct > 90%
- [ ] 12.10 手动测试：跨 session（< 1h）验证 Tier-1 cache hit（cache_read_tokens 应含 Tier-1 部分）
- [ ] 12.11 手动测试：DeepSeek provider 多轮对话，验证 prompt_cache_hit_tokens / (hit + miss) > 90%
- [ ] 12.12 手动测试：DeepSeek 跨 session（< 数小时）验证 prompt_cache_hit_tokens > 0
- [ ] 12.13 对比基线：CostTracker cache hit rate 从 <5% → >90%（DeepSeek 和 Anthropic）

## 13. Subagent 缓存策略

- [ ] 13.1 分析 subagent 是否复用父 session 的 PromptEngine 实例（若独立实例，需单独冻结）
- [ ] 13.2 subagent 若使用相同 system prompt 模板 → 确保 Tier-1 memoize 可跨 session/subagent 共享
- [ ] 13.3 subagent 的 tool_defs 可能与父不同（如仅有 read-only tools）→ ToolSchemaCache 按 agent_id 隔离
- [ ] 13.4 subagent lifecycle 短（通常 1-3 turns）→ 评估是否需要 1h TTL（短生命周期用 ephemeral 即可）
- [ ] 13.5 确认 CacheBreakDetector 在 subagent 上下文中也正确追踪（独立计数器 vs 共享）

## 14. `cached_message_boundary` 精确语义定义

- [ ] 14.1 定义 boundary 语义：表示"本次 LLM API 调用之前的 message count"，在 API 返回成功后更新
- [ ] 14.2 boundary 仅在 LLM 实际返回 `cache_read_tokens > 0` 时前移（确认 provider 侧确实缓存了）
- [ ] 14.3 并发保护：boundary 用 `AtomicUsize` 或 session-level mutex，防止快速连续请求的 race
- [ ] 14.4 compaction 操作时 boundary 重置为 0（承认缓存已全部失效）
- [ ] 14.5 `/clear` 命令时 boundary 重置为 0
