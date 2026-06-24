## Context

XiaoLin 的 subagent 系统由三层组成：后端运行时（`xiaolin-agent`：`SubAgentManager`、`reactive_loop`、`prompt_builder`）、协议层（`xiaolin-protocol`：6 种 `sub_agent_*` 事件）、前端 UI（`SubAgentCard` 内联卡片 + `CoordinatorPanel` 浮动面板）。

通过对调用链的完整追踪（spawn → run_subagent → reactive loop → 前端渲染），确认了以下**当前状态**：

**缓存相关（与 `prompt-cache-maximize-hits` change 强相关）**：
- `build_subagent_prompt_block`（`prompt_builder.rs:130-147`）把活跃 subagent 的 `elapsed_ms` 拼进 prompt block
- 该 block 经 `build_effective_prompt` 的 `append_prompt` 参数（`prompt_engine.rs:151-157`）推到 `system_text` 末尾
- `push_system_messages_from_prompt`（`mod.rs:109-138`）把末尾内容作为 `trailing` 合并进 **Tier-2 system message**
- session_bridge.rs:672-696 每个主 agent turn 重新计算 `subagent_prompt`（含最新 `elapsed_ms`）
- subagent 自身 execute 传 `subagent_prompt=None`（`subagent_manager.rs:1143`）✅ 子 agent 不受 active_runs 污染
- subagent 走同一个全局 `PromptEngine`，Tier-1 天然 byte-identical 跨父子共享 ✅
- subagent 的 "Context from parent agent" 作为 System message（`subagent_manager.rs:882-889`），经 `merge_leading_system_into_tier2` 合并进 Tier-2 ❌
- `CacheBreakDetector` 在 `TurnState` 中每 turn 新建（`turn_setup.rs:278`），父子不共享 ✅（design §13.5 的担忧不成立）

**能力相关**：
- reactive loop 用 `build_completion_notification` 把 worker 结果压成文本 system message（`session_bridge.rs:408`）
- `SubAgentRun`（`types.rs:439-462`）有 `depth` 但无 `parent_run_id`，前端无法精确构建层级树
- `SubAgentStart` 事件（`event.rs:343-350`）只传 `depth`，无父子关联

**UI 相关**：
- `SubAgentCard`（277 行）未复用 `StepIndicator` 的 `extractKeyInfo`、进度条、动画能力
- Result 用 `<pre>` 纯文本渲染，但项目已有成熟的 `MarkdownContent`（`react-markdown` + GFM + 代码高亮）
- 消息流卡片无实时计时（仅 `CoordinatorPanel` 的 `RunItem` 有，但两者计时逻辑未共享）
- `CoordinatorPanel` 是扁平列表，无 coordinator→worker 层级

**约束**：
- `AgentRuntime` 及其 `prompt_engine` 是进程级全局单例，被所有 session 共享
- 缓存修复必须遵循 `prompt-cache-maximize-hits` 的 D3（零污染）、D4b（per-session 确定性重算）原则
- 前端必须遵守 Zustand 5 + React 19 selector 陷阱、hooks 顺序、WS 6 层同步规则

## Goals / Non-Goals

**Goals:**
- 消除 subagent 场景下所有**客户端可控**的 prompt cache bust 因素，使有活跃 subagent 时主 agent 的 Tier-2 缓存保持 byte-stable
- 让主 agent 能感知 worker 进展并获得结构化结果，提升委派任务的完成质量
- 大幅提升 subagent 的可见性（实时工具/计时/进度）和结果可读性（Markdown 渲染）
- 多 subagent 场景下提供层级化视图和有效的 steering 控制

**Non-Goals:**
- 不实现 Anthropic 显式 `cache_control` 适配（随 `prompt-cache` change §7 一并 deferred）
- 不实现 worker↔worker 直接通信（保留为未来增强）
- 不实现 Pipeline DAG 时间线视图 / Thread 模式 / Agent Builder UI（P2-P3 范畴，本 change 不含）
- 不实现 task memory / 经验复用 / auto-routing（独立 change）
- 不重构 `SubAgentManager` 的生命周期管理核心逻辑

## Decisions

### D1: active_runs 状态从 system prompt 移到 user context（缓存核心修复）

**选择**：`build_subagent_prompt_block` 不再嵌入 active_runs 状态；active_runs 改为通过 `inject_user_context` 作为最后一条 user message 的 `<system_context>` attachment 注入。

**注入机制（关键实现细节）**：`inject_user_context(messages, block)` 操作的是 `build_messages` 内部已组装的 messages Vec（追加到最后一条 user message），**而非 session_bridge 层的 request**。因此 active_runs 需要一条独立于 `subagent_prompt` 的传递通道：
- 在 `AgentContext` 新增字段 `active_runs_context: Option<String>`（与现有 `subagent_prompt` 平级）
- `build_messages` 在 `messages.append(&mut conversation)` 之后、model-switch reminder 之前，调用 `inject_user_context(&mut messages, ctx.active_runs_context)`
- `session_bridge` 首次 execute 和 reactive loop 每次 re-prompt 都用 `mgr.active_runs(session_id)` 计算**最新** active_runs context 传入（reprompt 内可刷新 elapsed，因为走 user context 不破坏 system 缓存）

**替代方案**：
- A) 把 active_runs 排除在缓存边界外但仍在 system role → `push_system_messages_from_prompt` 的 trailing 仍并入 Tier-2，无效
- B) 给 active_runs 单独开 Tier-3 system block → 增加 provider 格式复杂度，且仍是 system role 动态内容
- C) session_bridge 直接 mutate `request.messages` → request 多处是 `&ChatRequest` 不可变引用，且会破坏 request 的语义纯净性
- D) (**选择**) AgentContext 新字段 + build_messages 内 inject_user_context → 完全遵循 D3 零污染，Tier-2 保持 byte-stable，复用现有注入基础设施

**理由**：`elapsed_ms` 每秒变化是确定性的缓存杀手。user context 注入是 prompt-cache change 已建立的标准模式（`inject_user_context` 已存在，签名 `(&mut Vec<ChatMessage>, &str)`）。模型对 `<system_context>` 标签内的内容同样能正确理解。

### D2: delegation guidance 确定性放置——剥离 active_runs，保持 Tier-2 byte-stable

**背景修正（review 发现）**：delegation guidance 依赖 **per-agent-config** 的 `policy`（`enabled`、`allowed_types`、`max_parallel`、`token_budget`、`available_agents`）。而 PromptEngine 的 Tier-1 static section 是**进程级全局 memoize**（`section_cache` 仅以 section 名为 key，被所有 session/agent 共享）。把 per-agent 内容放入 Tier-1 会触发 D4b 描述的污染 Bug——首个 agent 的 policy 值会泄漏给其它 agent。**因此 delegation guidance 整体不能进 Tier-1 全局 section。**

**选择**：`build_subagent_prompt_block` 仅做一处关键改动——**剥离 active_runs**（移交 D1）。其余内容（含静态模板 + per-agent 动态部分）继续作为 `append_prompt` 经 `build_effective_prompt` 注入，落在 Tier-2 区域。关键收益来自：剥离 active_runs 后，guidance block 对同一 agent 在 session 内/跨 session **byte-stable**（policy 不变 → 字节不变 → provider 自动前缀缓存命中）。

**替代方案**：
- A) 静态模板移入 Tier-1 全局 section → **不可行**，per-agent policy 污染（D4b Bug）
- B) 静态模板移入按 agent_id keyed 的缓存 → 收益有限（仅省 CPU memoize），且增加 per-agent 缓存基础设施复杂度，不值得
- C) (**选择**) 仅剥离 active_runs，guidance 整体留 Tier-2 确定性放置 → 改动最小，消除唯一的每轮变化因素（active_runs），同 agent 跨 session 仍命中

**理由**：缓存命中的核心是**字节稳定**，不是必须进 Tier-1。Tier-1 的额外价值是"跨不同 agent 共享"，但 delegation guidance 本就 per-agent 不同，跨 agent 共享无意义。剥离 active_runs（D1）后，Tier-2 对同一 agent 已 byte-stable，达成跨 session 命中目标。

**真正可进 Tier-1 的部分（可选优化，低优先级）**：若存在完全不依赖 policy 的通用 delegation 说明文本（如"委派是你的超能力"这类纯口号），可考虑抽出进 Tier-1；但收益微小（几百字节），本 change 不强制。

### D3: subagent parent context 改为 user message 注入

**选择**：`run_subagent`（`subagent_manager.rs:882-889`）的 "Context from parent agent" 不再作为 System role message，改为合并进 subagent 首条 user message（task），或作为独立 user message 排在 task 之前。

**替代方案**：
- A) 保留 System message 但放在 user task 之后 → 仍可能被 merge 逻辑影响，顺序脆弱
- B) (**选择**) 作为 user message → 不进 Tier-2，subagent 的 Tier-2 保持跨同类 subagent 可共享

**理由**：parent context 是 per-spawn 动态内容，放在 system role 会污染本可共享的 Tier-2（design §13.1 期望跨 subagent 命中 Tier-1+Tier-2）。作为 user message 注入符合 D3 零污染，且语义清晰（这是任务输入的一部分）。

### D4: 结构化 completion notification

**选择**：`reactive_loop::build_completion_notification` 的输出从纯文本摘要升级为结构化数据。worker 完成时携带 `{run_id, subagent_type, task, status, result, files_changed[], tool_calls_made, elapsed_ms}`，注入主 agent 时格式化为带明确字段的结构化文本（XML 或 Markdown 表格），而非自由文本。

**替代方案**：
- A) 保持纯文本摘要 → 主 agent 难以精确引用 worker 产出
- B) 作为独立的结构化 message 类型 → 需要协议层大改
- C) (**选择**) 结构化文本注入 → 主 agent 能可靠解析字段，改动可控

**理由**：主 agent 基于 worker 结果决策时，需要明确的"改了哪些文件、状态如何、关键产出是什么"。结构化格式（保留在文本注入层）兼顾可解析性和实现成本。

**缓存注意**：completion notification 走 user context（reactive loop 已是 append 到 messages 末尾），不影响 system prompt 缓存。

### D5: active_runs 进度注入升级

**选择**：D1 的 user context 注入中，active_runs 携带 `tool_calls_made` 和最新工具名（而非仅 task + elapsed），让主 agent 感知 worker 进展。

**理由**：主 agent 在 reactive loop 中决定"是否继续等待/spawn 更多/介入"时，进度信息（已执行多少工具、当前在做什么）比单纯的 elapsed 更有决策价值。数据已在 `SubAgentRun` 中（`tool_calls_made`）。

### D6: subagent 短生命周期 TTL 策略

**选择**：subagent 的 LLM 调用使用 ephemeral（5min）TTL，不使用 1h TTL。

**理由**：subagent 通常 1-3 turn，1h TTL 的 2x 计费无法被短对话摊销。落地 prompt-cache design §13.4。当前 Anthropic 显式 cache_control 已 deferred，此决策在未来接入时生效；现阶段作为约束记录。

### D7: SubAgentCard 增强（UI）

**选择**：
- 折叠行两行布局：主信息行（状态/类型/任务/计时/工具数）+ 辅助行（当前工具 或 result 首行摘要）
- 当前运行工具用 `extractKeyInfo`（复用 `StepIndicator` 已导出的逻辑）提取关键参数
- 提取共享 `useElapsedTimer` hook，`SubAgentCard` 和 `CoordinatorPanel.RunItem` 共用
- 展开/折叠用 `gridTemplateRows: 0fr→1fr` 动画（复用 `StepIndicator` 模式）
- 状态过渡：`scale-spring` ✓/✗ 图标 + 背景色过渡 + 失败左红条（复用已有 keyframes）
- 尊重 `prefers-reduced-motion`

**理由**：所有数据（`toolCalls`/`toolCallsMade`/`elapsedMs`/`result`）已在 `SubAgentRunUI` 中，WS 事件已实时更新 store，无需后端改动。复用现有动画 token 和组件逻辑，保证视觉一致性。

### D8: Result Markdown 渲染（UI）

**选择**：`SubAgentCard` 和 `CoordinatorPanel.RunItem` 的 result 区域用 `MarkdownContent`（lazy + Suspense）替代 `<pre>`；失败状态（`status === "failed"`）保留 `<pre>` 渲染错误信息；用紧凑 CSS（`text-[11px]`，缩小标题/列表间距）适配卡片空间。

**理由**：subagent result 通常是结构化 Markdown 报告（开发报告、审查报告）。项目已有成熟 `MarkdownContent`，直接复用。错误信息非 Markdown，保留 pre 更合适。

### D9: CoordinatorPanel 层级化（UI）

**选择**：
- 方案 A（前端推断，本 change 范围）：用 `subagentType === "coordinator"` 识别 coordinator，其余 active runs 作为 workers，树形缩进（`├──`/`└──` CSS 连接线）展示
- coordinator header 显示聚合统计（worker 数/完成/运行中/失败/耗时）
- worker 排序：运行中 > 失败 > 完成
- 无 coordinator 时回退扁平列表（复用增强后的 summary row）

**替代方案**：方案 B（后端加 `parent_run_id`，跨 6 层同步）作为未来精确化增强，本 change 不含——方案 A 已覆盖单 coordinator 主流程（90% 场景）。

**理由**：先用零后端改动的前端推断快速交付层级 UI；精确父子关系留待后续（多 coordinator 并行、深层嵌套场景）。

### D10: Steering 增强（UI）

**选择**：
- 快捷操作按钮（聚焦文件/加速完成/跳过当前/停下解释）→ 点击填入 input，用户可改后再发
- 发送状态反馈（发送中 spinner → ✓ → 恢复）+ 前端维护 steering 历史
- 优先级切换（普通/紧急，对应 `MessageQueue` 的 normal/high priority）
- coordinator 场景下 steering 目标选择（下拉选 active run）

**理由**：现有 steering 链路（`sendSteeringMessage` → WS `subagent.steer` → `MessageQueue.push`）已支持 priority 和按 run_id 路由，前端只需暴露这些已有能力。历史和反馈纯前端维护，无需后端改动。

## Risks / Trade-offs

- **[Risk] active_runs 移到 user context 后模型行为变化** → Mitigation: 用 `<system_context>` XML 标签明确标记为系统级上下文；保留核心 delegation 指令在 Tier-1 system role
- **[Risk] delegation guidance 拆分 Tier-1/Tier-2 引入 boundary 错误** → Mitigation: 单测验证拆分后两次 build_system_prompt byte-identical；验证 `policy` 配置在 Tier-1 的稳定性，不稳定则降级 Tier-2
- **[Risk] 结构化 completion notification 改变 reactive loop 行为** → Mitigation: 保持注入位置不变（messages 末尾），仅改格式；单测覆盖结构化解析
- **[Risk] 协议层扩展 SubAgentComplete 字段需 6 层同步** → Mitigation: 严格遵循规则 #5/#6 清单；优先复用现有字段（files_changed 可从 result 解析，避免新增字段）
- **[Risk] MarkdownContent lazy import 在紧凑卡片闪烁** → Mitigation: Suspense fallback 用极小占位符
- **[Risk] CoordinatorPanel 前端推断在多 coordinator 时误判** → Acceptable: 本 change 仅保证单 coordinator 准确，多 coordinator 留待方案 B
- **[Trade-off] 前端 steering 历史不持久化** → Acceptable: 历史仅 session 内有价值，刷新丢失可接受

## Migration Plan

分阶段实施，缓存修复优先（影响成本和所有场景）：

1. **Phase 1 — 缓存修复（D1/D2/D3）**：后端 prompt_builder + session_bridge + subagent_manager。这是核心，独立可验证（单测 + cache hit 日志）。
2. **Phase 2 — 能力增强（D4/D5/D6）**：reactive loop 结构化结果 + active_runs 进度注入。依赖 Phase 1 的 user context 注入基础设施。
3. **Phase 3 — UI 基础（D7/D8）**：SubAgentCard 增强 + Result 渲染 + 共享 timer hook。纯前端，可与后端并行。
4. **Phase 4 — UI 协作（D9/D10）**：CoordinatorPanel 层级化 + Steering 增强。依赖 Phase 3 的共享组件。

回滚策略：各 Phase 独立，缓存修复若导致模型行为问题，可单独回退 D1 的注入位置（恢复到 system prompt），其余不受影响。

## Open Questions

1. ~~delegation guidance 的 policy 配置是否进程级稳定？~~ → **已解决（review）**：policy 是 per-agent-config，故 guidance 整体不能进全局 Tier-1，改为 Tier-2 确定性放置 + 剥离 active_runs（见修正后的 D2）
2. 结构化 completion notification 的 `files_changed` 从哪获取——从 worker result 文本解析，还是 `SubAgentManager` 追踪 file artifact 事件？后者更准但需新增追踪（forwarder 已转发 `FileArtifact` 事件，可在此累积）
3. 是否需要在本 change 中同步更新 `prompt-cache-maximize-hits` 的 §13 任务状态，还是在本 change 完成后单独 PR 更新（倾向：本 change 完成后更新，见 task 1.4.3）
4. CoordinatorPanel 方案 A 的 worker 识别：除 `subagentType === "coordinator"` 外，是否需要结合 `depth` 辅助判断（避免把独立 spawn 的 subagent 误判为某 coordinator 的 worker）
5. active_runs_context 传到 AgentContext 的参数链路（`execute_unified_with_cost_store` 已有 17 个参数）——新增参数 vs 包装结构体，需在实现时权衡签名复杂度
