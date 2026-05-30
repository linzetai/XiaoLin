## Context

当前 sub-agent 系统的执行模型是"LLM 手动编排"：

1. LLM 调用 `spawn_subagent` 获得 run_id
2. LLM 必须主动调用 `wait_agent` 阻塞等待，或调用 `subagent_get` 轮询
3. LLM 收到结果后继续工作

这个模型的问题是 LLM 需要消耗 tokens 来管理循环逻辑，且容易出现以下 failure modes：
- 忘记 wait/poll 导致 sub-agent 结果丢失
- 全部 wait_all 阻塞，无法对先完成的结果做中间决策
- Turn 在 sub-agent 仍活跃时意外结束

相关核心文件：
- `crates/fastclaw-agent/src/runtime/mod.rs` — agentic loop
- `crates/fastclaw-agent/src/session_bridge.rs` — turn 执行入口
- `crates/fastclaw-agent/src/subagent.rs` — spawn_subagent 等工具
- `crates/fastclaw-agent/src/subagent_manager.rs` — 生命周期管理
- `crates/fastclaw-agent/src/spawn_controller.rs` — 并发控制

## Goals / Non-Goals

**Goals:**
- Harness 自动管理 sub-agent 的等待和结果回注，LLM 只管 spawn 决策
- 每个 sub-agent 完成时自动 re-prompt 主 LLM，让它可以中间决策（spawn more / reason / wait）
- Turn 结束条件包含 "no active sub-agents" 守卫
- 短时间窗口内多个 completion 合并为一次 re-prompt（batch）
- 前端实时 sub-agent 状态监控面板
- 增强 prompt 让 LLM 更积极委派

**Non-Goals:**
- 用户直接 spawn sub-agent（仅主 agent 可以）
- Sub-agent 之间的直接通信（仍通过主 agent 中转）
- Sub-agent checkpoint/resume 机制（后续迭代）
- 修改 sub-agent 内部执行逻辑（子 agent 运行方式不变）

## Decisions

### D1: Reactive Loop 实现位置 — 在 `execute_unified` 外层包裹

**选择**: 在 `session_bridge.rs` 的 turn 执行逻辑外层新增 `reactive_loop` wrapper，而非修改 `execute_unified` 内部。

**理由**: `execute_unified` 负责单次 LLM 交互循环（消息 → LLM → tool calls → 结果），逻辑已经复杂。reactive loop 是"多次调用 execute_unified"的更高层语义，放在外层更清晰。

**替代方案**: 修改 `execute_unified` 内部循环。放弃原因：耦合太深，且 sub-agent 管理逻辑与 tool execution 逻辑正交。

### D2: Completion 通知格式 — 注入为 System Message

**选择**: Sub-agent 完成后，harness 将结果作为 `Role::System` 消息追加到 conversation messages，然后触发新的 LLM 调用。

**格式**:
```
[Sub-Agent Completed: {run_id}]
Type: {type} | Task: "{task}"
Status: {status} | Duration: {elapsed}s | Tool calls: {n}

Result:
{result_text (truncated to 2000 chars if needed)}

Remaining active: {count}
{remaining_list}

Instruction: Process this result. You may spawn additional tasks, reason about findings, or wait for remaining sub-agents.
```

**理由**: System message 不会被 LLM 视为用户对话，适合作为 harness 通知。且不需要引入新的 message role。

**替代方案**: 新增 `Role::Notification` 类型。放弃原因：需要修改 protocol 层且 LLM provider 可能不支持。

### D3: Batch Window — 2 秒合并窗口

**选择**: 当检测到 sub-agent 完成时，等待额外 2 秒，将窗口内所有 completion 合并为一次 re-prompt。

**理由**: 避免 3 个 sub-agent 在 1 秒内完成导致 3 次昂贵的 re-prompt。2 秒是 "快但不浪费" 的平衡点。

**配置**: 可通过 `SubAgentPolicy.batch_window_ms` 配置，默认 2000ms。

### D4: Turn 结束守卫 — execute_unified 返回后检查

**选择**: 每次 `execute_unified` 返回后（LLM 一轮结束），检查 `subagent_manager.list_active_runs(session_id).is_empty()`。如果不为空，进入等待而非结束 turn。

**特殊情况**:
- LLM 在一轮中 spawn 了新 sub-agent 但也产出了文本 → 文本先 stream 给前端，但 turn 不结束
- LLM 显式说"我不再等了"（无此能力，turn 总是等全部完成）

### D5: 工具集调整 — 保留但降级 wait_agent/subagent_get

**选择**: 不立即删除 `wait_agent` 和 `subagent_get`，而是：
- `wait_agent`: 从 prompt guidance 中移除推荐，但保留工具注册（向后兼容）
- `subagent_get`: 保留（LLM 偶尔需要手动查看特定 run 的详细状态）
- Prompt 改为引导 "spawn 后无需手动 wait，系统会自动通知你结果"

**理由**: 硬删除会破坏已有的自定义 prompt/workflow。soft deprecation 更安全。

### D6: 前端面板 — 聊天区右侧 auto-show panel

**选择**: 在 `MessageStream` 组件旁边新增 `SubAgentMonitor` 面板，仅当有活跃 sub-agent 时自动显示。

**布局**:
```
┌─ SessionList ─┐┌───── Chat Area ─────┐┌─ Monitor ─┐
│               ││                      ││           │
│  (sidebar)    ││  MessageStream       ││ SA status │
│               ││                      ││           │
│               ││  StreamFooter        ││           │
└───────────────┘└──────────────────────┘└───────────┘
```

**宽度**: 固定 280px，动画 slide-in/out。
**数据源**: 复用已有的 `SubAgentRunUI` store + SubAgent* WebSocket 事件。

### D7: LLM Re-prompt 时的 context 管理

**选择**: Re-prompt 时将 completion notification 追加到已有 messages 中（保持完整对话历史），而非单独发送。

**Token 控制**: 如果 result 文本超过 2000 chars，截断并提示 "Full result available via subagent_get(run_id)"。

**Ack 机制**: 如果 LLM 在 re-prompt 后只输出纯文本无 tool calls 且 still has active runs → 视为 "acknowledged, continue waiting"，不 stream 这段文本给用户（避免噪音）。可配置 `suppress_intermediate_ack: bool`。

## Risks / Trade-offs

**[Token 成本增加]** → 每次 re-prompt 都消耗 tokens。Mitigation: batch window 减少 re-prompt 次数；truncate result 控制注入大小；配置 `max_reprompts_per_turn` 上限（默认 10）。

**[无限循环风险]** → LLM 每次 re-prompt 都 spawn 新 sub-agent 导致永不结束。Mitigation: `max_depth` 已有限制 + 新增 `max_spawns_per_turn` 限制（默认 20）。

**[向后兼容]** → 已有依赖 `wait_agent` 流程的 custom prompt 会受影响。Mitigation: 工具保留但 soft-deprecate，新行为通过 config flag `reactive_loop.enabled` 开启。

**[中间 ack 噪音]** → LLM 可能频繁输出 "好的继续等待" 类噪音文本。Mitigation: `suppress_intermediate_ack` 配置 + 在 prompt 中引导 LLM 只在有实质决策时输出。

**[前端布局挤压]** → 监控面板占 280px 可能在小屏上挤压聊天区。Mitigation: 响应式设计，小屏时改为 overlay/drawer 模式。
