## Context

XiaoLin 的 Plan 模式体验在与 Codex CLI（78/100）和 Claude Code（80/100）对标后，发现核心差距集中在 5 个维度：Plan 产物呈现、审批门控、模式入口、视觉反馈、提示词质量。

当前架构中 Plan 通过 `write_file` 工具写入 `~/.xiaolin/plans/{slug}.md`，前端通过 `PlanFileUpdate` 信号事件触发 HTTP refetch 获取内容。LLM 流式输出工具参数时 content delta 被完全抑制（`llm_call.rs` 第 660 行 `if tool_call_accum.is_empty()` 守卫），用户在 Plan 写入期间只能等待。

已有基础设施：`StreamingToolExecutor` 支持 plan_file_path 感知、`SubAgentDelta` 提供了自定义流式事件的先例、`mode_attachments` 提供了 per-turn 提示词注入机制、`PlanApprovalCard` 和 `PlanPanel` UI 组件已存在。

## Goals / Non-Goals

**Goals:**
- Plan 内容在 LLM 生成工具参数时实时流式推送到前端（延迟 < 100ms vs 当前 2-5s）
- 审批门控提供 5 个选项（实现/清空上下文实现/拒绝反馈/继续规划/记住选择）
- UI 模式切换与 agent tool 入口注入等价上下文
- Plan 模式下消息流有明确视觉区分
- Plan 模式提示词引导模型执行三阶段工作流
- 审批后 PlanPanel 追踪实施进度（自动 checklist + 文件修改检测 + 完成检测）
- Session 刷新/重连后 Plan 状态自动恢复（元数据 hydrate + PlanPanel 恢复）
- 用户输入复杂任务时自动建议进入 Plan 模式（nudge + 快捷键）

**Non-Goals:**
- 不重构 Plan 产物模型（保持 write_file 到文件的方式，不改为 Codex 的 XML 标签模式）
- 不实现 Claude Code 的 Ultraplan（发到 Web 精修）
- 不实现 Plan 版本历史 diff（留给后续迭代）
- 不在此变更中集成 `permissions.rs` 到 Plan 模式（独立变更）

## Decisions

### D1: 流式方案选择 — 工具参数拦截（方案 B）而非 XML 标签解析（方案 A）

**选择**: 在 `llm_call.rs` 的工具调用处理循环中，通过 `PlanArgInterceptor` 从 `write_file`/`edit_file` 的参数流中实时提取 `content` 字段值，转发为 `AgentEvent::PlanDelta`。

**替代方案 A**: 改变模型输出方式，在 assistant text 中用 `<proposed_plan>` 标签包裹 plan 内容（Codex 做法）。这需要重写 Plan 模式的核心提示词策略，与现有 `PlanFileStore` 架构冲突，且模型不一定能可靠输出 XML 标签。

**替代方案 C**: write_file 执行完成后读取全量内容推送（写后推送）。延迟更高（等全部参数输出完毕），但实现简单。

**理由**: 方案 B 在保持现有架构完整性的前提下获得接近 Codex 的流式体验。工具参数的 JSON 流与 assistant text 流速度相当，用户体验几乎等价。方案 C 作为 Phase 1 的过渡方案，降低风险。

### D2: PlanArgInterceptor 的 JSON 解析策略

**选择**: 字符级流式 JSON 状态机，追踪 key/value 边界，在进入 `content` 字段值时开始转发 delta（含 JSON 字符串反转义）。

**替代方案**: 使用 `serde_json::StreamDeserializer` 或 `simd-json` 的流式解析。这些库不支持从部分 JSON 中提取单个字段值的增量。

**key 顺序问题**: OpenAI function calling 通常按 schema 定义顺序输出参数。XiaoLin 的 `write_file` schema 中 `file_path` 在 `content` 前，所以 path 通常先到。为安全起见，如果 `content` 先于 `path` 到达，buffer 前 200 字符的 content delta，path 到达后 flush 或 discard。

### D3: 渐进式实施 — Phase 1（写后推送）+ Phase 2（流式拦截）

**选择**: 分两个阶段实施。Phase 1 在 `PlanFileUpdate` 事件中增加 `content` 字段，write_file 完成后立即推送全量内容，省去 HTTP refetch。Phase 2 实施完整的 PlanArgInterceptor 流式方案。

**理由**: Phase 1 可在 1 天内完成，立即消除最显著的延迟感（HTTP refetch）。Phase 2 需要 3-4 天，但可以独立迭代。

### D4: 审批门控 — 参考 Claude Code 但不照搬

**选择**: 5 个选项设计

| 选项 | 行为 |
|------|------|
| 开始实现 (保持上下文) | 切换到 Agent 模式，发送 "实现计划" 引导消息 |
| 开始实现 (清空上下文) | 清空对话历史，以 "实现以下计划:\n\n{plan}" 作为新的用户消息开始 |
| 拒绝并反馈 | 保持 Plan 模式，用户反馈作为新消息发送，模型修改 plan |
| 继续规划 | 保持 Plan 模式，无额外消息 |
| 记住选择 | 后续 Plan 自动以选中的方式审批 |

**不做 Claude Code 的 Ultraplan 和权限模式选择**（前者需要 Web 端支持，后者 XiaoLin 暂无多级权限体系）。

### D5: 统一模式入口 — 合成用户消息注入

**选择**: 当用户通过 UI ModeSelector 切换到 Plan 模式时，gateway 的 `execution.set_mode` handler 向对话历史注入一条合成用户消息：`[系统: 用户已切换到规划模式]`，并立即触发 `mode_attachment` 注入。

**替代方案**: 强制 UI 切换走 `enter_plan_mode` 工具路径。太重——需要一次额外的 LLM 调用。

**理由**: 合成消息是最轻量的方式，确保模型在下一次回复时看到明确的模式切换信号。Claude Code 的 `prepareContextForPlanMode()` 采用类似策略。

### D6: todo_write 抑制

**选择**: Plan 模式下将 `todo_write` 加入 ToolProfile::plan_mode() 的 demote 列表（对模型不可见），避免两个规划渠道冲突。

**理由**: Plan 文件已经是结构化的规划产物，todo_write 在 Plan 模式下是冗余的。Codex 也在 Plan 模式下阻止了 `update_plan` 工具。

## Risks / Trade-offs

- **[JSON 解析复杂度]** → PlanArgInterceptor 需要处理 JSON 字符串转义、跨 chunk 的转义序列、Unicode 转义等边界情况。**缓解**: 充分的单元测试，覆盖所有转义场景；Phase 1 先用写后推送降低风险。
- **[path 顺序不确定]** → 少数模型可能先输出 content 再输出 path。**缓解**: 200 字符 buffer 窗口 + path 到达后 flush/discard。
- **[PlanFileStore::new(None) bug]** → 修复时需要确保所有调用点使用相同的 plans 目录配置。**缓解**: 单一 PlanFileStore 实例通过 TurnServices 传递。
- **[清空上下文实现]** → 需要 gateway 支持清空对话历史并以新消息开始新轮次，当前 session actor 可能不支持。**缓解**: 复用已有的 session 重建机制，或创建新 session 并复制 plan 内容。
- **[记住选择]** → 需要持久化用户偏好（per-session 或 per-workspace）。**缓解**: 使用 localStorage 存储，session 级别即可。
- **[提示词升级]** → 更复杂的提示词可能导致模型行为不一致。**缓解**: A/B 测试；保留旧版提示词作为 fallback。
- **[执行追踪路径匹配]** → plan Changes 中的文件路径可能与 agent 实际 write 的路径不完全一致（相对/绝对、前缀差异）。**缓解**: 尾部模糊匹配（取最后 2-3 路径段比较）。
- **[复杂度启发式误判]** → Nudge 可能在不需要 plan 的场景误触发，打扰用户。**缓解**: 多级频率控制 + dismiss 后不再触发 + 高阈值（需满足 2/4 条件）。
- **[模式推断歧义]** → 从最近消息推断 mode 可能不准确（如 agent 调了 enter_plan_mode 但后来又 exit 了）。**缓解**: Registry 优先；推断仅作最后 fallback；三条消息窗口取最近的 mode 工具。

### D7: Plan 色系统 — Teal 色系

**选择**: 使用 Teal（`#0D9488` light / `#2DD4BF` dark）作为 Plan 模式专用色系，通过 `--plan-tint-*` CSS custom properties 定义。

**替代方案 A**: 复用 `var(--tint)` 蓝色 + 透明度区分。无法在视觉上区分 Agent 和 Plan 模式。
**替代方案 B**: 使用紫色（当前 ModeSelector 的 `oklch(56% 0.18 310)`）。紫色与蓝色在某些显示器上难以区分。

**理由**:
- Claude Code 使用 `rgb(0,102,102)` (teal) 经大规模验证
- 与 Agent 蓝色色相差足够辨识但不突兀（同属冷色系）
- 形成三色体系：Agent (蓝) / Plan (青) / Goal (橙)
- teal 在 light/dark 主题下均有良好可见度和对比度

### D8: 在编辑器中打开 — Tauri Opener 插件

**选择**: PlanApprovalCard 提供「在编辑器中打开」辅助按钮，使用 Tauri v2 的 opener 插件（`@tauri-apps/plugin-opener`）打开 plan 文件。

**替代方案**: 使用 `shell_exec` 调用 `xdg-open` / `open`。需要运行时环境判断，且安全性不如 Tauri 内置插件。

**理由**: Tauri opener 是官方插件，跨平台支持好，安全性由 capability 配置控制。Claude Code 的 Ctrl+G 外部编辑功能证明此能力对 plan mode UX 有价值。

### D9: 审批卡 Plan 预览默认展开

**选择**: PlanApprovalCard 在有 plan 内容时，Markdown 预览默认展开（max-height 600px + overflow scroll），而非当前的折叠式。

**替代方案**: 保持折叠式。用户需要额外点击才能看到 plan，增加审批决策延迟。

**理由**: Claude Code 的 ExitPlanMode 对话框默认展示完整 plan（甚至有 fullscreen 模式）。GUI 应用有屏幕空间优势，默认展开更利于快速审阅。

### D10: Plan 执行追踪 — 自动 Checklist + 文件修改检测

**选择**: 审批后 PlanPanel 自动从 plan 文件 `## Changes` 章节解析步骤生成 checklist，agent 执行 write_file/edit_file 时自动路径匹配标记步骤完成。Compact 后注入 plan_file_reference，实施期每 5 轮注入 sparse reminder。

**替代方案 A**: 依赖 agent 手动调用 `update_plan` 工具标记进度（Codex 做法）。需要模型主动维护，可靠性低，且 plan 步骤与 update_plan 无结构化关联。

**替代方案 B**: 依赖 `TodoWrite` 工具创建独立 todo 列表（Claude Code 做法）。Plan 和 Todo 是两套系统，用户需要心智映射两者对应关系。

**替代方案 C**: 纯 UI 展示（只读 checklist），用户手动勾选。不利用 agent 工具执行信息，浪费了可自动化的机会。

**理由**:
- XiaoLin 的 GUI PlanPanel 天然支持持久化 checklist 展示（不会滚出屏幕）
- write_file/edit_file 的文件路径与 plan Changes 步骤有明确对应关系，自动匹配准确率高
- Codex 和 Claude Code 都没有做到「plan 步骤自动标记完成」，这是差异化能力
- Compact 后 plan 重注入是 Claude Code 的做法（plan_file_reference），直接对标
- sparse reminder 确保长实施过程中模型不偏离 plan

### D11: Plan 恢复策略 — get_plan_meta RPC + 推断模式

**选择**: 新增 `execution.get_plan_meta` RPC，前端在 session 激活 / 页面刷新 / WS 重连时主动查询 plan 元数据（文件路径、是否存在、执行模式）。模式推断优先使用 SessionModeRegistry，fallback 到从最近消息中 plan 相关工具调用推断。

**替代方案 A**: 在 SQLite 新增 `execution_mode` 列。需要 schema migration，增加复杂度，且 mode 变更频繁时写入开销大。

**替代方案 B**: 前端 localStorage 持久化 per-session executionMode。刷新后可恢复 UI，但与 backend 状态可能不一致（crash 后 backend 继续在 Plan 模式）。

**替代方案 C**: WS 重连时 backend 主动推送所有 session 的 mode/plan 状态。对有大量 session 的用户不友好（推送量大）。

**理由**:
- 按需查询（on-demand）比全量推送更高效，只对活跃 session 查询
- 推断逻辑兜底 gateway 重启（Registry 丢失但 plan 文件和消息还在）
- 不需要 DB schema 变更，降低迁移风险
- Claude Code 的 `recoverPlanFromMessages()` 验证了从消息推断 mode 的可行性

### D12: Plan Mode Reentry — 修改式引导而非重规划

**选择**: 当 session 重入 Plan 模式且 plan 文件已存在时，注入 `plan_mode_reentry` attachment，引导模型在已有方案基础上修改，而非从头规划。仅首次 LLM 调用注入，后续恢复标准 plan_full/plan_sparse 循环。

**替代方案**: 始终使用 plan_full attachment（不区分首次和重入）。模型可能重复生成已有内容，浪费 token 和时间。

**理由**: Claude Code 的 `plan_mode_reentry` attachment 证明了此模式的价值。重入场景（用户刷新后继续、审批拒绝后修改、长时间后回来继续）是高频场景，需要明确引导模型读取已有方案而非从零开始。

### D13: Plan Nudge — 多维度启发式 + 快捷键

**选择**: 三层 nudge 触发（关键词检测 + 复杂度启发式 + 首次使用教育），配合 Ctrl+Shift+P 快捷键。Nudge 以内联提示条形式显示在 Composer 内部，支持多级频率控制和 dismiss 持久化。

**替代方案 A**: 仅模型驱动（EnterPlanMode 工具描述中列条件，让模型自行决定）。Claude Code 的做法，但用户无法主动发现 Plan 模式的存在。

**替代方案 B**: 系统级 toast 通知。太打扰、与输入流脱节、用户可能忽略。

**替代方案 C**: Codex 做法（仅检测 "plan" 一词）。太简单，对中文用户无效，无法检测隐式复杂任务。

**理由**:
- GUI 环境允许更精细的内联 UI（比 TUI 底行文字强）
- 复杂度启发式是 Codex 和 Claude Code 都没做的差异化能力
- 中英文双语关键词适配国际化
- 频率控制避免反复打扰
- Ctrl+Shift+P 与 VS Code 的 Ctrl+Shift+P (Command Palette) 形成肌肉记忆迁移
