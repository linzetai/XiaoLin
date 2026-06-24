# Tasks

> 实施顺序遵循 design.md 的 Migration Plan：缓存修复（Phase 1）优先，因其影响成本和所有 subagent 场景。

## Phase 1 — 缓存修复（最高优先级，后端）

### 1.1 active_runs 状态零污染注入（D1）
- [x] 1.1.1 `prompt_builder.rs`：`build_subagent_prompt_block` 移除 active_runs 嵌入逻辑，`SubAgentPromptContext` 移除 `active_runs` 字段
- [x] 1.1.2 `prompt_builder.rs`：新增 `build_active_runs_context(active: &[ActiveRunSummary]) -> Option<String>`（纯状态文本，由 inject_user_context 包裹）
- [x] 1.1.3 `agent_context.rs`：`AgentContext` 新增 `active_runs_context: Option<String>`，3 个构造点全部补齐
- [x] 1.1.4 `runtime/mod.rs build_messages`：merge_leading_system_into_tier2 之后、model-switch reminder 之前调用 `inject_user_context`
- [x] 1.1.5 `session_bridge.rs`：首次 execute 计算 active_runs_context 并传入
- [x] 1.1.6 reactive loop：每次 re-prompt 用 `self.build_active_runs_context` 重算最新 elapsed 传入
- [x] 1.1.7 参数链路：`execute_unified_with_cost_store` 新增第 18 参数（最小签名变更，wrapper 传 None）
- [x] 1.1.8 单测：guidance 跨调用 byte-identical + 不含 active_runs（`guidance_excludes_active_runs_and_is_byte_stable`）
- [x] 1.1.9 单测：active_runs context 含进度 + 空集返回 None + elapsed 变化不触及 guidance

### 1.2 delegation guidance 确定性放置（D2）
- [x] 1.2.1 `prompt_builder.rs`：剥离 active_runs 后 guidance 对同一 policy byte-stable（单测覆盖）
- [ ] 1.2.2 检查 `available_agents` 来源（`mgr.agent_descriptions()`）顺序确定性（如来自 HashMap 需排序）— 待 Phase 5 回归核查
- [x] 1.2.3 guidance 整体**不**进 Tier-1（保持 append_prompt 落 Tier-2，避免 per-agent policy 污染）
- [x] 1.2.4 单测：同一 agent 两次 build guidance byte-identical
- [x] 1.2.5 仅在 `policy.enabled` 时注入 guidance（build_subagent_prompt_block 开头 early return）

### 1.3 subagent parent context 改 user message（D3）
- [x] 1.3.1 `subagent_manager.rs`：parent context 合并进 task user message（单条 user message）
- [x] 1.3.2 验证 `merge_leading_system_into_tier2` 只合并 System role，不再并入 parent context（review 确认）
- [ ] 1.3.3 单测：同类型 sub-agent Tier-2 byte-identical — 待 Phase 5（需更完整的 build_messages 集成测试夹具）

### 1.4 缓存验证
- [x] 1.4.1 `cargo test -p xiaolin-agent` 通过（新增 4 测试全过；5 个失败为 baseline 既有的 approval/hook/mutex 环境相关失败，与本改动无关）
- [ ] 1.4.2 手动测试：主 agent spawn subagent 后多轮对话，日志确认 Tier-2 cache hit 不掉落 — 待 Phase 5 E2E
- [ ] 1.4.3 更新 `prompt-cache-maximize-hits` §13 状态 — 待 Phase 5

### 1.5 review 修复（subagent review 发现）
- [x] 1.5.1 🟡 多模态 user message 保护：`append_text_to_chat_content` 改为数组时追加 text part，不再拍扁丢图（惠及所有 inject_user_context 调用方）
- [x] 1.5.2 🟡 task 截断：`build_active_runs_context` 按 UTF-8 安全截断 task 到 120 字符（规则 #1）

## Phase 2 — 能力增强（后端）

### 2.1 结构化 completion notification（D4）
- [x] 2.1.1 `reactive_loop.rs`：`build_completion_notification` 输出结构化文本（含 run_id/subagent_type/task/status/result/tool_call_count/elapsed_ms 字段）— 已有实现，本期复核确认字段完整、labeled
- [x] 2.1.2 评估 open question #2：`files_changed` — 决定**推迟**（需新增 CompletionSummary 字段 + 6 层前端同步 + file artifact 累积，成本高收益边际），现有 result_preview 已含变更摘要
- [x] 2.1.3 失败 worker 的 notification 包含错误原因和 status=failed（reactive_loop L44-45 error 分支 + 完成 channel error 字段）
- [x] 2.1.4 单测：`build_completion_notification_single` / `_all_done` 覆盖字段完整性（已有）

### 2.2 active_runs 进度注入（D5）
- [x] 2.2.1 `prompt_builder.rs`：`build_active_runs_context` 含 `tool_calls_made`（Phase 1 落地）
- [x] 2.2.2 `subagent_manager.rs`：`SubAgentRun` 新增运行时字段 `current_tool`（serde-skip、不持久化）；forwarder 在 `ToolExecuting` 自增 tool_calls_made + 设 current_tool，`ToolResult`/完成/失败 清空；`session_bridge.build_active_runs_context` 映射 current_tool 并从 created_at 派生 live elapsed_ms
- [x] 2.2.3 单测：`active_runs_context_shows_current_tool` 验证 `current: <tool>` 渲染（tool_calls_made/elapsed 已覆盖）

### 2.3 类型体系文档化（D 概念统一）
- [x] 2.3.1 `types.rs`：`current_tool` 字段文档化（live/运行时、不持久化、DB 用独立 row）；`prompt_builder.rs`：`ActiveRunSummary` 全字段文档化（live/ephemeral、不进 cache）
- [ ] 2.3.2 确认前端 `subagentType` 字符串与后端两套体系的映射一致 — 待 Phase 3/4 前端改造时核查

## Phase 3 — UI 基础（前端，可与后端并行）

### 3.1 共享计时 hook（D7）
- [x] 3.1.1 新增 `lib/hooks/useElapsedTimer.ts`：提取计时逻辑（isActive + baseMs → elapsed）+ 共享 `formatElapsed`
- [x] 3.1.2 `CoordinatorPanel.tsx` 的 `RunItem` 改用 `useElapsedTimer`（移除本地 setInterval + 本地 formatElapsed）
- [x] 3.1.3 `SubAgentCard.tsx` 接入 `useElapsedTimer`（折叠行实时计时）

### 3.2 SubAgentCard 折叠行增强（D7）
- [x] 3.2.1 `SubAgentCard.tsx`：折叠行改为两行布局（主信息行 + 辅助行）
- [x] 3.2.2 辅助行：运行中显示当前工具（复用 `StepIndicator` 的 `extractKeyInfo`：`name · key`）+ 实时计时
- [x] 3.2.3 辅助行：完成且有 result 时显示 result 首行摘要（`resultFirstLine` 去 Markdown 标记）
- [x] 3.2.4 折叠行始终显示工具计数（Lightning + thinking/count）+ 计时
- [x] 3.2.5 工具切换用 `key={auxText}` + `fade-in` animation 平滑过渡

### 3.3 SubAgentCard 动画（D7）
- [x] 3.3.1 展开/折叠改用 `gridTemplateRows: 0fr→1fr` 动画（复用 StepIndicator 模式）；折叠时 `inert` 移出 tab 序
- [x] 3.3.2 状态图标 ✓/✗ 用 `scale-spring` 弹入；背景色 transition；失败左红条（borderLeft）
- [x] 3.3.3 卡片首次出现用 `fade-slide-up`
- [x] 3.3.4 `prefers-reduced-motion` — index.css L1344 已有全局 reduce 规则覆盖所有 animation/transition

### 3.4 Result Markdown 渲染（D8）
- [x] 3.4.1 `SubAgentCard.tsx`：result 非失败用 `MarkdownContent`（lazy + Suspense），失败保留 `<pre>`（红色）
- [x] 3.4.2 `CoordinatorPanel.tsx` `RunItem`：result 同样 Markdown 渲染（失败 `<pre>`）
- [x] 3.4.3 新增 `.subagent-md` 紧凑 Markdown CSS（11px，缩小标题/列表/段落间距）
- [x] 3.4.4 streaming content（运行中）用 `StreamingMarkdown`，完成 result 用 `MarkdownContent`

### 3.5 Phase 3 验证
- [x] 3.5.1 `pnpm tsc --noEmit`（crates/xiaolin-app/）通过
- [x] 3.5.2 ReadLints 检查 SubAgentCard/CoordinatorPanel/useElapsedTimer/index.css 无 lint 错误
- [x] 3.5.3 检查 hooks 顺序（规则 #11）：所有 hooks 在 return 之前，无 early return
- [x] 3.5.4 检查 Zustand selector：SubAgentCard 用 props 无 selector；CoordinatorPanel selector 返回原始值/稳定引用，派生用 useMemo

### 3.6 review 修复（subagent review 发现，PASS 无🔴）
- [x] 3.6.1 🟡 嵌套 button：SubAgentCard 概要行 `<button>` 改 `<div role="button" tabIndex onKeyDown>`，消除非法 HTML + a11y 警告（取消按钮得以合法嵌套）
- [x] 3.6.2 🟡 折叠态常驻挂载重渲染：新增 `hasExpanded` state，首次展开后才挂载 Markdown body（折叠卡片保持轻量）
- [ ] 3.6.3 🟡 live timer 基准为挂载时刻（中途打开面板会低估耗时）→ 需后端补 `startedAtMs` 字段 + 6 层同步，作为已知精度限制推迟（Phase 4/后续评估）
- [x] 3.6.4 🟢 已知项记录：lazy MarkdownContent 两处独立（Vite 已合并 chunk）、JS slice UTF-16 边界（视觉乱码不 panic）— 不阻塞

## Phase 4 — UI 协作（前端，依赖 Phase 3）

### 4.1 CoordinatorPanel 层级化（D9）
- [x] 4.1.1 render 内分支：有 coordinator → 树形（header + WorkerRow），无 coordinator → 扁平 RunItem 列表
- [x] 4.1.2 worker 识别：`subagentType === "coordinator"` 为顶层，其余为 worker（方案 A，单 coordinator 主流程；depth 辅助留待方案 B）
- [x] 4.1.3 树形缩进连接线：`WorkerRow` 用 CSS border-left 竖干 + 横向 elbow（最后一个子节点竖干止于 elbow）
- [x] 4.1.4 coordinator header 聚合统计（worker 数/运行中/失败/完成 + 耗时，颜色区分）
- [x] 4.1.5 worker 排序 `sortWorkers`：运行中 > 失败 > 完成，再按 elapsed 降序
- [x] 4.1.6 复用 `RunItem`（summary row 组件）于树形与扁平两种布局

### 4.2 Steering 增强（D10）
- [x] 4.2.1 快捷操作按钮（聚焦文件/加速完成/跳过当前/停下解释），点击填入 input（不覆盖已有内容）
- [x] 4.2.2 发送状态反馈（CircleNotch spinner → ✓ → 恢复；失败红边 + 红色）
- [x] 4.2.3 前端维护 steering 历史（`SteeringEntry[]`，时间+目标+状态+内容，可折叠，上限 20）
- [x] 4.2.4 优先级切换按钮（普通/紧急 → normal/high，默认 high）
- [x] 4.2.5 steering 目标选择下拉（active runs，含 coordinator；单候选时隐藏）
- [x] 4.2.6 `transport.ts`/`api.ts`：`sendSteeringMessage(runId, message, priority?)` 已支持 priority + runId，无需改动
- [x] 4.2.7 i18n：`zh/en chat.json` 新增 coordinator 统计 + steering 目标/优先级/历史/快捷操作 keys

### 4.3 Phase 4 验证
- [x] 4.3.1 `pnpm tsc --noEmit` 通过 + ReadLints 零错误
- [ ] 4.3.2 E2E（Tauri MCP）：spawn 多 subagent，验证层级展示、result Markdown、steering 目标选择 — Phase 5 E2E 统一执行

### 4.4 review（self-review，subagent 额度受限改为人工审查，PASS 无🔴）
- [x] 4.4.1 hooks 顺序：SteeringBar/SubAgentsTabContent 所有 hooks 在 return 前；early return 后无 hook
- [x] 4.4.2 effectiveTarget 自愈：选中目标消失时回退 defaultTargetId；defaultTargetId 恒在 candidates 内（parent guard）
- [x] 4.4.3 🟢 修复：priority 按钮加 `aria-pressed`；post-send `setTimeout` 用 ref + useEffect cleanup 防 unmount 后 setState
- [x] 4.4.4 🟢 已知设计取舍：applyQuick 仅在 input 为空时填入（不覆盖用户输入）；树形 elbow 13px 为视觉近似

## Phase 5 — 整体回归
- [x] 5.1 `cargo clippy -p xiaolin-agent -p xiaolin-core` 通过；本改动涉及文件零 clippy 警告（残留 2 警告在 `token_budget.rs`/`xiaolin-tools-network`，均非本期改动文件，pre-existing）；`cargo test` 5 个失败为 baseline 既有（approval/hook/mutex 环境相关）
- [x] 5.2 `pnpm tsc --noEmit`（crates/xiaolin-app/）通过，零类型错误
- [ ] 5.3 E2E：完整 subagent 流程（spawn → 运行 → steering → 完成 → result 查看）真实 UI 验证 — 待 `cargo tauri dev` 起本地改动构建 + LLM 凭据后执行（当前仅安装版 `/usr/bin/xiaolin-app` 在跑，不含本期前端改动且 MCP bridge 未连）
- [ ] 5.4 缓存回归：对比修复前后，确认有活跃 subagent 时主 agent cache hit rate 提升 — 随 5.3 在 dev 日志中核查 Tier-2 cache hit
- [x] 5.5 发现 Bug 记录到 `docs/bugfix.md`（BUG-014/015/016 缓存污染），评估并已沿用现有 code-generation-quality 规则（无需新增）
