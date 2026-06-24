## Why

XiaoLin 的 subagent 系统功能完整，但存在三类相互关联的问题：1) **缓存正确性 Bug**——主 agent 的 system prompt 被每秒变化的 `elapsed_ms` 污染，导致有活跃 subagent 时 Tier-2（session-stable 层，约 5-8k tokens）每轮 100% cache miss，恰好发生在对话最长、token 最多的时刻；2) **能力发挥不充分**——worker 结果被压成文本摘要回传、主 agent 感知不到 worker 进展、缺少结构化协作；3) **UI 体验薄弱**——SubAgentCard 信息密度低、Result 不渲染 Markdown、多 subagent 场景缺乏层级和有效的 steering。这三类问题必须一并解决，否则 subagent 无法在系统中发挥最大能力。

## What Changes

### 缓存修复（最高优先级）
- **BREAKING（内部）** 将 `[Active Sub-Agents]` 状态（含 `elapsed_ms`）从 system prompt 移到 user context（`inject_user_context`），遵循 prompt-cache change 的 D3 零污染原则
- 从 delegation guidance 中剥离 active_runs，使 guidance 对同一 agent byte-stable（跨 session 命中 provider 自动前缀缓存）；guidance 因依赖 per-agent policy 不进全局 Tier-1（避免 D4b 污染 Bug）
- subagent 的 "Context from parent agent" 改为 user message 注入，不再污染可跨 subagent 共享的 Tier-2
- subagent 短生命周期 TTL 策略：使用 ephemeral 而非 1h，避免为 1-3 turn 的子对话支付 2x 计费

### 能力增强
- reactive loop 结果回传结构化（worker result 携带 run_id、修改文件列表、状态），而非纯文本摘要
- active_runs 注入升级：携带进度（toolCallsMade）和最新工具，让主 agent 感知 worker 进展
- 统一 `SubAgentType` enum 与 builtin def 两套类型体系的概念关系，文档化使用边界

### UI 体验升级
- SubAgentCard 折叠行增强：显示当前运行工具 + 实时计时 + 工具计数；展开/折叠平滑动画
- Result 区域用 Markdown 渲染替代纯 `<pre>`（失败状态保留 pre）
- CoordinatorPanel 层级化：coordinator → worker 树形缩进 + 聚合统计
- Steering 增强：快捷操作按钮、发送历史与状态反馈、优先级切换、steering 目标选择

## Capabilities

### New Capabilities
- `subagent-cache-hygiene`: subagent 场景下的 prompt cache 正确性保障——active_runs 状态零污染注入、delegation guidance 分层、parent context 移出 Tier-2、短生命周期 TTL 策略
- `subagent-structured-results`: worker 完成结果的结构化回传与主 agent 进展感知——结构化 completion notification、active_runs 进度注入、worker 中间状态可见
- `subagent-card-enhanced`: 消息流内 SubAgentCard 的可见性增强——实时工具显示、实时计时、工具计数、Markdown result 渲染、状态过渡动画
- `subagent-coordinator-hierarchy`: CoordinatorPanel 的层级化展示——coordinator/worker 树形结构、聚合统计、worker 排序
- `subagent-steering-enhanced`: Steering 交互增强——快捷操作、发送历史与反馈、优先级切换、目标选择

### Modified Capabilities
- `subagent-reactive-loop`: reactive loop 的 completion notification 从纯文本摘要改为结构化数据；active_runs 状态注入位置从 system prompt 改为 user context
- `floating-subagent-panel`: CoordinatorPanel 从扁平列表改为层级树形展示，新增聚合统计和增强 steering

## Impact

- **后端 crates/xiaolin-agent**：
  - `runtime/prompt_builder.rs`：`build_subagent_prompt_block` 拆分静态/动态部分，active_runs 状态剥离
  - `session_bridge.rs`：active_runs 改走 `inject_user_context`；reactive loop 结构化结果回传
  - `subagent_manager.rs`：parent context 注入方式改为 user message；run_subagent 结果结构化
  - `reactive_loop.rs`：`build_completion_notification` 输出结构化数据
- **后端 crates/xiaolin-protocol**：可能扩展 `SubAgentComplete` / `SubAgentNotification` 事件字段（结构化结果），跨 6 层同步（规则 #5/#6）
- **前端 crates/xiaolin-app/src**：
  - `components/message-stream/SubAgentCard.tsx`：折叠行增强 + Markdown result + 动画 + steering 增强
  - `components/shell/CoordinatorPanel.tsx`：层级化重构 + 聚合统计 + steering 增强
  - `lib/hooks/useElapsedTimer.ts`：新增共享计时 hook
  - `lib/stores/types.ts` / `stream-store.ts`：SubAgentRunUI 可能新增字段（进度、结构化结果）
  - `lib/transport.ts` / `api.ts`：steering 优先级/目标参数、结构化结果解析
  - `i18n/locales/{zh,en}/chat.json`：新增 steering 快捷操作等 key
- **关联 change**：缓存修复部分（`subagent-cache-hygiene`）与 `prompt-cache-maximize-hits` 的 §13 强相关，本 change 实现后应更新该 change 的 §13 状态
- **风险前置**：缓存修复需验证不改变模型行为（active_runs 在 user context 中用 `<system_context>` XML 标签标记）
