## 0. Bug Fixes（前置）— COMPLETED

- [x] 0.1 在 `useMessageStreamChat.ts` 中添加 `sub_agent_notification` case handler，将数据推入 stream-store
- [x] 0.2 审计 `subagent_prompt` 参数：确认有实际使用方（`build_subagent_prompt_block`），移除 `append_subagent_prompt_to_system` 死代码
- [x] 0.3 移除 `SubAgentManager.default_policy` 的 `#[allow(dead_code)]`，删除该未使用字段
- [x] 0.4 修复 `SubAgentTool` 中 `parent_tx` 为 None 时事件丢失：实现 session_event_senders + task-local 路由
- [x] 0.5 修复 `RuntimeTurnExecutor.subagent_manager` 初始化为 None（builder.rs 创建顺序问题）
- [x] 0.6 修复 `SUBAGENT_SESSION_ID` task-local 无法穿越 `tokio::spawn()` 边界

## 1. Stream-based Agent Loop — COMPLETED

### Phase 1a: 定义类型（AgentStep + AgentContext）— COMPLETED

- [x] 1a.1 新建 `crates/xiaolin-agent/src/runtime/agent_step.rs`，定义 `AgentStep` 枚举
- [x] 1a.2 新建 `crates/xiaolin-agent/src/runtime/agent_context.rs`，定义 `AgentContext` struct
- [x] 1a.3 在 `crates/xiaolin-agent/src/runtime/mod.rs` 中声明新模块并导出
- [x] 1a.4 `cargo clippy -- -D warnings` 零警告

### Phase 1b: 抽取阶段函数（重构，行为不变）— COMPLETED

- [x] 1b.1 新建 `turn_setup.rs` — setup_turn 抽取
- [x] 1b.2 新建 `iteration_check.rs` — pre_iteration_check 抽取
- [x] 1b.3 新建 `llm_call.rs` — LLM stream 准备 + 消费抽取
- [x] 1b.4 新建 `tool_round.rs` — 工具轮次 dispatch + 结果处理抽取
- [x] 1b.5 新建 `post_tool.rs` — 后处理抽取
- [x] 1b.6 重写主循环为 `turn_loop::run_turn_loop` 调用子函数
- [x] 1b.7 `cargo test` + `cargo clippy -- -D warnings` 全通过

### Phase 1c: 实现 execute_as_stream（核心变更）— COMPLETED

- [x] 1c.1 添加 `async-stream = "0.3"` 依赖
- [x] 1c.2 实现 `AgentRuntime::execute_as_stream` — 使用 `async_stream::stream!` 宏
- [x] 1c.3 Dual-Channel Architecture: step_tx (AgentStep) + event_tx (AgentEvent side-path)
- [x] 1c.4 Stream 取消语义：CancellationToken 集成

### Phase 1d: 兼容层桥接（外部 API 不变）— COMPLETED

- [x] 1d.1 重写 `execute_unified` → 构建 AgentContext → `run_stream_to_completion`
- [x] 1d.2 重写 `execute_stream` 同理
- [x] 1d.3 所有调用方零修改
- [x] 1d.4 旧 `execute_stream_inner` 已删除

### Phase 1e: 全量回归验证 + 清理 — COMPLETED

- [x] 1e.1 `cargo test -p xiaolin-agent` 789 tests pass
- [x] 1e.2 `cargo clippy --workspace -- -D warnings` 零警告
- [x] 1e.3 `#[allow(clippy::too_many_arguments)]` 保留于 `execute_unified`（兼容层，将随后续废弃一起移除）
- [x] 1e.4 旧 `execute_stream_inner` + `ExecutionParams` + `StreamParams` 全删除
- [x] 1e.5 E2E 回归：WebSocket 直连 gateway 验证完整事件流
  - 普通对话 ✅（TurnStart → context_usage_update → Delta* → TurnEnd）
  - 工具调用 ✅（通过 789 unit tests 覆盖）
- [x] 1e.6 最终 commit: `refactor(agent): eliminate from_agent_event bridge, yield AgentStep directly`

**关键成果**:
- 净减 ~200 行代码
- 消除 AgentEvent → AgentStep 转换桥（`from_agent_event` 删除）
- 主循环直接 yield 类型安全的 AgentStep
- 侧路径（ToolProgress, Approval, SubAgent*）通过 event_tx 直连 caller

## 2. Sidechain Transcript — COMPLETED

- [x] 2.1 创建 `crates/xiaolin-agent/src/sidechain.rs` 模块：`SidechainWriter` struct（path, BufWriter）
- [x] 2.2 实现 `SidechainWriter::new(session_dir, run_id)` — 创建目录 + 写入 metadata header
- [x] 2.3 实现 `SidechainWriter::append(message)` — 序列化为 JSON line 并 flush
- [x] 2.4 实现 `SidechainReader::load(session_dir, run_id)` — 读取 JSONL 还原消息列表
- [x] 2.5 在 `SubAgentManager::run_subagent()` 中创建 SidechainWriter，child event 持久化前 forward
- [x] 2.6 实现 result extraction：子 agent 完成时取最后 assistant 消息（截断 4096 chars）
- [x] 2.7 新增 `resume_subagent` 工具：读取 sidechain → 构建 initial messages → 继续执行
- [x] 2.8 在 session 删除逻辑中添加 sidechains 目录清理

**关键成果**:
- Sidechain 文件持久化到 `~/.xiaolin/sessions/{session_id}/sidechains/{run_id}.jsonl`
- 修复 `SUBAGENT_SESSION_ID` task-local 无法穿越 tokio::spawn 边界的 bug
- `resume_subagent` 工具可恢复上下文并续跑（BANANA 记忆测试通过）
- Session 删除时自动清理 sidechain 文件

## 3. Fork Agent — COMPLETED

- [x] 3.1 在 `SubAgentTool::execute()` 中解析 `inherit_context` 参数
- [x] 3.2 实现 `filter_parent_messages(messages, max_messages)` 函数
- [x] 3.3 过滤逻辑：移除 system messages、incomplete tool_calls，限制条数
- [x] 3.4 将 filtered messages 作为 child agent 的 initial context prefix
- [x] 3.5 在 SubAgentDef 中添加 `max_context_messages` 可选字段（默认 20）
- [x] 3.6 在 `parameters_schema()` 中暴露 `inherit_context` 参数给 LLM

**关键成果**:
- 子代理通过 `inherit_context: true` 继承父会话的过滤后上下文
- `filter_parent_messages` 移除 system 消息和不完整的 tool_calls，限制条数
- E2E 验证：子代理 msg_count 从 3（无继承）提升到 5（有继承），成功获取父会话信息

## 4. Message Queue + SendMessage — COMPLETED

- [x] 4.1 创建 `crates/xiaolin-agent/src/message_queue.rs`：定义 `Priority` enum 和 `MessageQueue` struct
- [x] 4.2 实现 `MessageQueue::push(priority, source, message)` 和 `drain(max_priority) -> Vec<QueuedMessage>`
- [x] 4.3 在 `AgentContext` 中添加 `message_queue: Option<Arc<MessageQueue>>` 字段
- [x] 4.4 在 `post_tool_processing` 的 ToolRoundBoundary 处添加 drain + inject 逻辑
- [x] 4.5 创建 `SendMessageTool` struct，实现 Tool trait（查找目标 run 的 queue → push）
- [x] 4.6 在 SubAgentManager 中维护 `run_queues: DashMap<String, Arc<MessageQueue>>`
- [x] 4.7 利用已有的 `AgentStep::SteeringInjected` 变体标记注入
- [x] 4.8 在 gateway WebSocket handler 中支持 `subagent.steer` / `steering_message` 命令
- [x] 4.9 在 `spawn()` 中创建 queue 并通过 `execute_unified_with_cost_store` 传递到 AgentContext
- [x] 4.10 E2E 验证：spawn sub-agent → steer ok=true → sub-agent completed

## 5. Permission Bubble — COMPLETED

- [x] 5.1 在 `xiaolin-core` 中定义 `PermissionMode` enum（AutoApprove, Bubble, Deny）
- [x] 5.2 在 `SubAgentDef` 中添加 `permission_mode` 字段（默认 AutoApprove）
- [x] 5.3 定义 `ApprovalStrategy::Bubble(Arc<BubbleApprovalPort>)` 变体 + `BubbleApprovalPort` struct
- [x] 5.4 在 `SubAgentManager::spawn()` 中根据 permission_mode 构建对应的 ApprovalStrategy
- [x] 5.5 复用 `AgentEvent::ApprovalRequired` — Bubble 时自动发送到 event stream
- [x] 5.6 实现 30s timeout logic：tokio::select! approval_rx vs sleep(30s)，超时自动 Denied
- [x] 5.7 在 gateway WS handler 中转发 approval_required → 前端（forward_event 自动处理）
- [x] 5.8 在 `resolve_approval` handler 中整合 BubbleApprovalPort 回退路径
- [x] 5.9 管理 pending approvals map：`BubbleApprovalPort { pending: DashMap<String, oneshot::Sender<ApprovalDecision>> }`

**关键成果**:
- `PermissionMode` 三态枚举控制子 agent 工具审批策略
- `BubbleApprovalPort` 通过 DashMap + oneshot channel 管理异步审批
- 30s 超时自动拒绝，防止 sub-agent 无限等待
- `resolve_approval` WS 命令同时支持 session 审批和 bubble 审批
- E2E 验证：resolve_approval 协议正常工作，approval.resolve 别名兼容

## 6. Coordinator Mode — COMPLETED

- [x] 6.1 在 SubAgentDef 中添加 `mode` 字段（`SubAgentMode::Normal` / `SubAgentMode::Coordinator`）
- [x] 6.2 实现 coordinator tool registry filter：通过 `SubAgentToolFilter.allowed` 限制为管理工具
- [x] 6.3 创建 `TaskStopTool` struct（coordinator 主动结束编排，返回 summary + status）
- [x] 6.4 在 coordinator 模式下 force worker spawn 为 background=true（`coordinator_mode` 字段）
- [x] 6.5 worker 完成时通过 `parent_queue` 将 CompletionSummary 推送到 coordinator 的 MessageQueue
- [x] 6.6 创建 `COORDINATOR_SYSTEM_PROMPT` 常量（编排指引 + 工作流规则）
- [x] 6.7 创建 builtin coordinator SubAgentDef（id="coordinator", mode=Coordinator, background=true）
- [x] 6.8 E2E 验证：task_stop 工具注册、coordinator 出现在 list_agents、resolve_approval 回归通过

**关键成果**:
- `SubAgentMode` 两态枚举（Normal / Coordinator）控制 agent 行为模式
- Coordinator 仅允许管理工具（spawn_subagent, send_message, task_stop 等）
- `TaskStopTool` 允许 coordinator 主动结束编排并提供最终 summary
- `coordinator_mode` 字段强制所有 worker spawn 为 background 模式
- `parent_queue` 参数使 worker 完成通知能推送到 coordinator 的 MessageQueue
- 5 个 builtin sub-agent defs（explore, code, shell, research, coordinator）

## 7. Markdown Agent Definitions — COMPLETED

- [x] 7.1 实现 `parse_agent_markdown(path) -> Result<SubAgentDef>` 函数（frontmatter YAML + body）
- [x] 7.2 实现 `load_agents_from_dir(dir) -> Vec<SubAgentDef>` 函数
- [x] 7.3 在 `SubAgentManager::new()` 中按优先级加载：builtin → `~/.xiaolin/agents/` → `{project}/.xiaolin/agents/`
- [x] 7.4 实现 merge 逻辑：同 id 后者覆盖前者
- [x] 7.5 添加 frontmatter schema 验证（required fields check + type validation）
- [x] 7.6 处理无效文件：跳过 + warning 日志
- [x] 7.7 实现 hot-reload：file watcher 监听 agents 目录变更 → 重新加载
- [x] 7.8 更新 `ListAgentsTool` 输出包含 source 信息（builtin/user/project）

**关键成果**:
- `agent_markdown` 模块：robust frontmatter 解析，支持 `deny_unknown_fields` schema 验证
- `merge_subagent_defs` 函数：多层级定义合并，后者按 id 覆盖前者
- `AgentDefWatcher`：基于 notify v6 的 hot-reload，300ms debounce
- `ListAgentsTool` 输出新增 `source` 字段（builtin/json:/markdown:）
- `SubAgentDef` 新增 `Default` impl，`parse_markdown_subagent_def` 委托到新模块
- 加载优先级：builtin → project JSON → project markdown → user markdown

## 8. Frontend Interaction — COMPLETED

- [x] 8.1 在 stream-store 中添加 `notifications` 数组到 SubAgentRunUI
- [x] 8.2 实现 `sub_agent_notification` handler 更新 store
- [x] 8.3 在 SubAgentMonitor 中显示 notification feed
- [x] 8.4 在 SubAgentCard 中添加 cancel 按钮（running 状态时显示）
- [x] 8.5 在 SubAgentCard 展开态添加 steering 输入框（running 状态时显示）
- [x] 8.6 实现 steering input → WebSocket `steering_message` 发送
- [x] 8.7 创建 `ApprovalBubbleCard` 组件（通过 pendingQuestion 机制实现）
- [x] 8.8 处理 `approval_required` WebSocket 事件 → 渲染审批卡片
- [x] 8.9 实现 Approve/Deny 按钮 → 发送 `resolveApproval` + 更新卡片状态
- [x] 8.10 处理 `approval_resolved` 事件 → 清除 pendingQuestion
- [x] 8.11 创建 CoordinatorPanel 组件（worker 列表 + 状态 + notifications + steering）
- [x] 8.12 在 WorkspacePanel 中根据 coordinator run 存在与否动态注册/注销 Coordinator tab

**关键成果**:
- SubAgentMonitor 展开态显示最近 5 条 notification（时间戳 + 消息）
- SubAgentCard 展开态新增 steering 输入框（Enter 发送，PaperPlaneRight 按钮）
- `sendSteeringMessage` transport 函数 → `subagent.steer` WebSocket 方法
- CoordinatorPanel 完整组件：协调器状态头、worker 列表、notification feed、steering 输入
- WorkspacePanel 动态 tab：检测到 coordinator run 时自动显示，消失时自动移除
- Approval 机制通过已有的 `setPendingQuestion` + `resolveApproval` 全链路实现

## 9. 验证与清理

- [ ] 9.1 `cargo check` 全 workspace 通过
- [ ] 9.2 `cargo clippy -- -D warnings` 零警告
- [ ] 9.3 确认无 `#[allow(dead_code)]` 新增
- [ ] 9.4 `pnpm exec tsc --noEmit` 前端类型检查通过
- [ ] 9.5 现有 subagent 相关测试适配并通过
- [ ] 9.6 新增单元测试覆盖: MessageQueue, SidechainWriter/Reader, parse_agent_markdown, coordinator tool filter
