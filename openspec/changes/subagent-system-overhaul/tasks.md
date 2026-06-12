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

## 2. Sidechain Transcript

- [ ] 2.1 创建 `crates/xiaolin-agent/src/sidechain.rs` 模块：`SidechainWriter` struct（path, BufWriter）
- [ ] 2.2 实现 `SidechainWriter::new(session_dir, run_id)` — 创建目录 + 写入 metadata header
- [ ] 2.3 实现 `SidechainWriter::append(message)` — 序列化为 JSON line 并 flush
- [ ] 2.4 实现 `SidechainReader::load(session_dir, run_id)` — 读取 JSONL 还原消息列表
- [ ] 2.5 在 `SubAgentManager::run_subagent()` 中创建 SidechainWriter，child event 持久化前 forward
- [ ] 2.6 实现 result extraction：子 agent 完成时取最后 assistant 消息（截断 4096 chars）
- [ ] 2.7 新增 `resume_subagent` 工具：读取 sidechain → 构建 initial messages → 继续执行
- [ ] 2.8 在 session 删除逻辑中添加 sidechains 目录清理

## 3. Fork Agent

- [ ] 3.1 在 `SubAgentTool::execute()` 中解析 `inherit_context` 参数
- [ ] 3.2 实现 `filter_parent_messages(session_store, max_messages, max_tokens)` 函数
- [ ] 3.3 过滤逻辑：移除 system messages、incomplete tool_calls，限制条数和 token 数
- [ ] 3.4 将 filtered messages 作为 child agent 的 initial context prefix
- [ ] 3.5 在 SubAgentDef 中添加 `max_context_messages` 可选字段（默认 20）

## 4. Message Queue + SendMessage

- [ ] 4.1 创建 `crates/xiaolin-agent/src/message_queue.rs`：定义 `Priority` enum 和 `MessageQueue` struct
- [ ] 4.2 实现 `MessageQueue::push(priority, source, message)` 和 `drain(max_priority) -> Vec<QueuedMessage>`
- [ ] 4.3 在 `AgentContext` 中添加 `message_queue: Option<Arc<MessageQueue>>` 字段
- [ ] 4.4 在 `execute_as_stream` 的 ToolRoundBoundary 处添加 drain + inject 逻辑
- [ ] 4.5 创建 `SendMessageTool` struct，实现 Tool trait（查找目标 run 的 queue → push）
- [ ] 4.6 在 SubAgentManager 中维护 `run_queues: DashMap<String, Arc<MessageQueue>>`
- [ ] 4.7 定义 `AgentStep::SteeringInjected` 变体 + 对应的 `AgentEvent::SteeringMessage`
- [ ] 4.8 在 gateway WebSocket handler 中支持前端 `steering_message` 命令 → push 到 queue

## 5. Permission Bubble

- [ ] 5.1 在 `xiaolin-core` 中定义 `PermissionMode` enum（AutoApprove, Bubble, Deny）
- [ ] 5.2 在 `SubAgentDef` 中添加 `permission_mode` 字段（默认 AutoApprove）
- [ ] 5.3 定义 `ApprovalStrategy::ParentApproval(oneshot::Sender<ApprovalResult>)` 变体
- [ ] 5.4 在 `SubAgentManager::run_subagent()` 中根据 permission_mode 构建对应的 ApprovalStrategy
- [ ] 5.5 定义 `AgentEvent::ApprovalBubble { run_id, tool_name, args_preview, respond_tx }` 变体
- [ ] 5.6 实现 30s timeout logic：tokio::select! approval_rx vs sleep(30s)
- [ ] 5.7 在 gateway WebSocket handler 中转发 approval_bubble → 前端
- [ ] 5.8 在 gateway 中实现 `approval_respond` 命令 → 通过 saved respond_tx 回复
- [ ] 5.9 管理 pending approvals map：`DashMap<request_id, oneshot::Sender<ApprovalResult>>`

## 6. Coordinator Mode

- [ ] 6.1 在 SubAgentDef 中添加 `mode` 字段（Normal / Coordinator）
- [ ] 6.2 实现 coordinator tool registry filter：仅允许 spawn_subagent, send_message, task_stop, subagent_list, subagent_get
- [ ] 6.3 创建 `TaskStopTool` struct（coordinator 主动结束编排）
- [ ] 6.4 在 coordinator 模式下 force worker spawn 为 background=true
- [ ] 6.5 worker 完成时将 CompletionSummary 格式化并 push 到 coordinator 的 MessageQueue
- [ ] 6.6 创建 `coordinator_system_prompt.txt` 默认编排指引
- [ ] 6.7 创建 builtin coordinator SubAgentDef（id="coordinator", mode=Coordinator）
- [ ] 6.8 集成测试：coordinator spawn 多个 worker → 收到 notifications → 综合输出

## 7. Markdown Agent Definitions

- [ ] 7.1 实现 `parse_agent_markdown(path) -> Result<SubAgentDef>` 函数（frontmatter YAML + body）
- [ ] 7.2 实现 `load_agents_from_dir(dir) -> Vec<SubAgentDef>` 函数
- [ ] 7.3 在 `SubAgentManager::new()` 中按优先级加载：builtin → `~/.xiaolin/agents/` → `{project}/.xiaolin/agents/`
- [ ] 7.4 实现 merge 逻辑：同 id 后者覆盖前者
- [ ] 7.5 添加 frontmatter schema 验证（required fields check + type validation）
- [ ] 7.6 处理无效文件：跳过 + warning 日志
- [ ] 7.7 实现 hot-reload：file watcher 监听 agents 目录变更 → 重新加载
- [ ] 7.8 更新 `ListAgentsTool` 输出包含 source 信息（builtin/user/project）

## 8. Frontend Interaction

- [ ] 8.1 在 stream-store 中添加 `notifications` 数组到 SubAgentRunUI
- [ ] 8.2 实现 `sub_agent_notification` handler 更新 store
- [ ] 8.3 在 SubAgentMonitor 中显示 notification feed
- [ ] 8.4 在 SubAgentCard 中添加 cancel 按钮（running 状态时显示）
- [ ] 8.5 在 SubAgentCard 展开态添加 steering 输入框（running 状态时显示）
- [ ] 8.6 实现 steering input → WebSocket `steering_message` 发送
- [ ] 8.7 创建 `ApprovalBubbleCard` 组件（tool_name, args_preview, Approve/Deny 按钮）
- [ ] 8.8 处理 `approval_bubble` WebSocket 事件 → 渲染 ApprovalBubbleCard
- [ ] 8.9 实现 Approve/Deny 按钮 → 发送 `approval_respond` + 更新卡片状态
- [ ] 8.10 处理 `approval_resolved` 事件（timeout/外部 resolve）→ 更新卡片状态
- [ ] 8.11 创建 CoordinatorPanel 组件（worker 列表 + 状态 + activity）
- [ ] 8.12 在 WorkspacePanel 中根据 coordinator run 存在与否显示/隐藏 Coordinator tab

## 9. 验证与清理

- [ ] 9.1 `cargo check` 全 workspace 通过
- [ ] 9.2 `cargo clippy -- -D warnings` 零警告
- [ ] 9.3 确认无 `#[allow(dead_code)]` 新增
- [ ] 9.4 `pnpm exec tsc --noEmit` 前端类型检查通过
- [ ] 9.5 现有 subagent 相关测试适配并通过
- [ ] 9.6 新增单元测试覆盖: MessageQueue, SidechainWriter/Reader, parse_agent_markdown, coordinator tool filter
