## Context

XiaoLin 已有完整的 subagent 基础设施：

- **SubAgentTool** (`crates/xiaolin-agent/src/subagent.rs`): LLM 调用 `spawn_subagent` 工具生成子 agent
- **SubAgentManager** (`crates/xiaolin-agent/src/subagent_manager.rs`): 生命周期管理（spawn/cancel/track）
- **SpawnController** (`crates/xiaolin-agent/src/spawn_controller.rs`): RAII 并发控制 + RW 隔离
- **Reactive Loop** (`crates/xiaolin-agent/src/reactive_loop.rs`): 子 agent 完成后 batch 通知 + 重新触发父级
- **SubAgentDef** (`crates/xiaolin-core/src/agent_config.rs`): agent 类型定义（id/name/tools/system_prompt/background/concurrency_safe）
- **前端**: SubAgentCard + SubAgentMonitor + stream-store 实时状态

参考架构 claude-code 的核心设计：
- **query()**: 单一 async generator，递归调用自身实现 subagent
- **createSubagentContext()**: 克隆可变状态，UI 操作 no-op 化
- **recordSidechainTranscript()**: 独立持久化子 agent 对话
- **messageQueueManager**: 优先级消息队列，tool-round boundary 注入
- **permissionMode: 'bubble'**: 子 agent 权限请求在父级 UI 显示
- **Coordinator Mode**: 受限工具集 + SendMessage + async workers

## Goals / Non-Goals

**Goals:**
- 将 agent loop 统一为 Stream-based，使父子可组合
- 子 agent 对话完整持久化，支持 resume
- 实现 tool-round boundary 的消息注入机制
- 支持子 agent 权限请求 bubble 到父级
- 实现 Coordinator 编排模式
- 支持 Markdown frontmatter 定义自定义 agent
- 修复现有 Bug（notification 丢失、dead code）

**Non-Goals:**
- 不完全复制 claude-code 的 TypeScript async generator 模式（Rust 语言差异）
- 不实现 swarm/teammate 模式（超出当前范围）
- 不实现 MCP server per-agent 隔离（后续扩展）
- 不实现 agent memory 持久化（与 sidechain 不同，是跨 session 记忆）
- 不移除现有 SpawnController 的 RAII 设计（这是 XiaoLin 的优势）

## Decisions

### D1: Stream-based Loop 而非 Trait 抽象

**选择**: 将 `execute_unified` 内部改为产出 `Stream<Item=AgentStep>`，保留兼容 API。

**为什么不用 trait**: Claude-code 不是 trait 多态，而是一个递归函数。关键是可组合性（composability），不是多态性（polymorphism）。Rust 中 Stream 天然支持 composition：`stream.flat_map(|step| child_stream)`。

**为什么不用 async-stream crate**: 评估后倾向使用 `async-stream`（简洁）或 `futures::stream::unfold`（零依赖）。如果编译时间增加明显，退回手写 poll-based。

**兼容层**: 保留 `execute_unified` 签名，内部调用 `execute_as_stream()` 并 collect 为 `TurnSummary`。所有现有调用方无需修改。

### D2: Sidechain 存储在 session 目录下

**选择**: JSONL 文件存储在 `{session_dir}/sidechains/{run_id}.jsonl`。

**为什么不用 SQLite**: 子 agent 对话是追加写入（append-only），JSONL 更简单、支持 streaming 写入、方便人工查看。SQLite 适合结构化查询但子 agent 对话不需要复杂查询。

**替代方案**: 存入主 session 的 messages 表并标记 `is_sidechain`（类似 claude-code）。被否决因为 XiaoLin 的 session store 是文件系统 JSONL 而非 SQLite，在已有 JSONL 中混入 sidechain 会增加过滤复杂度。

### D3: 消息注入在 tool-round boundary

**选择**: 在所有 tool results 收集完毕、下一次 LLM 调用之前检查 MessageQueue。

**为什么这个时机**: 
1. LLM API 要求 tool_result 紧跟 tool_use，不能在中间插入 user message
2. 这是 claude-code 验证过的安全注入点
3. 保证消息语义正确（作为新 turn 的 user message 出现）

**Priority 系统**: `Now`（立即注入，本轮）> `Next`（下一个 tool-round boundary）> `Later`（Sleep/延迟后注入）。初期只实现 `Next`。

### D4: Permission Bubble 通过 oneshot channel

**选择**: Bubble 模式下，子 agent 的 `ApprovalStrategy` 设为 `ParentApproval(oneshot::Sender<ApprovalResult>)`。父级收到 `AgentEvent::ApprovalBubble` 后通过 channel 回复。

**为什么不共享 canUseTool 函数引用**: Rust 的所有权模型不允许跨 task 共享可变闭包。oneshot channel 是 Rust 中跨 task 通信的惯用模式。

**超时处理**: 如果父级 30s 未回复（如 UI 断连），默认 deny + 子 agent 收到 `ToolResult::err("approval timeout")`。

### D5: Coordinator 是 SubAgentDef 变体，非独立模式

**选择**: Coordinator 通过 `SubAgentDef { mode: "coordinator", ... }` 定义，而非全局 `ExecutionMode::Coordinator`。

**为什么**: 
1. Coordinator 本身可以被更高层 agent 生成（嵌套 coordinator）
2. 不需要全局模式切换，只需要特定 agent 定义的工具受限
3. 保持与 SubAgentDef 统一的加载/管理路径

**Coordinator 工具集**: `spawn_subagent`（受限为只能生成 worker）+ `send_message` + `task_stop` + `subagent_list` + `subagent_get`

### D6: Markdown Agent 加载路径与优先级

**选择**: 
- 项目级: `{project_root}/.xiaolin/agents/*.md`
- 用户级: `~/.xiaolin/agents/*.md`
- Builtin: 硬编码在 `agent_discovery.rs`

**优先级**: Builtin < User < Project（与 claude-code 一致）。同 id 的后者覆盖前者。

**Frontmatter 解析**: 使用 `serde_yaml` 解析 `---` 之间的 YAML。Markdown body 追加到 `system_prompt` 末尾（作为额外指令）。

### D7: AgentContext 合并 13+ 参数

**选择**: 将 `execute_unified` 的 13 个参数合并为 `AgentContext` struct：

```rust
pub struct AgentContext {
    pub config: AgentConfig,
    pub request: ChatRequest,
    pub tool_registry: Arc<ToolRegistry>,
    pub approval_strategy: ApprovalStrategy,
    pub llm_override: Option<Arc<dyn LlmProvider>>,
    pub orchestrator: Arc<ToolOrchestrator>,
    pub interaction_handle: Option<InteractionHandle>,
    pub message_queue: Option<Arc<MessageQueue>>,
    pub session_store: Option<Arc<SessionStore>>,
    pub sidechain_writer: Option<SidechainWriter>,
    // ... other optional contexts
}
```

**为什么**: 解决 clippy `too_many_arguments` warning；使 Stream API 签名清晰；支持后续扩展不需要改方法签名。

---

## 与 Claude Code 实现的验证记录

以下是对照 claude-code 源码后发现的关键差异和确认点：

### 确认一致的部分

| 领域 | Claude Code 实现 | 我们的设计 | 状态 |
|------|-----------------|-----------|------|
| Sidechain 存储 | 独立 `subagents/agent-{id}.jsonl` 文件 | `sidechains/{run_id}.jsonl` | 一致（命名差异可接受） |
| 消息队列优先级 | `now(0) > next(1) > later(2)` | 相同三级优先级 | 一致 |
| Agent frontmatter 必须字段 | `name` + `description` | 相同 | 一致 |
| Tool-round boundary 时机 | tools 完成后、下次 LLM 调用前 | 相同 | 一致 |
| Agent 定义优先级 | builtin < plugin < user < project | builtin < user < project | 一致（我们无 plugin 层） |

### 有意识的差异（设计决策）

| 领域 | Claude Code | 我们的选择 | 原因 |
|------|------------|-----------|------|
| Loop 模式 | `while(true)` + State 对象 + 内部 yield | `Stream<Item=AgentStep>` | Rust 无原生 async generator；Stream 支持 composition + drop 取消 |
| Permission bubble 通信 | 共享闭包 `canUseTool` + `shouldAvoidPermissionPrompts` flag | oneshot channel + `AgentEvent::ApprovalBubble` | Rust 所有权模型不允许跨 task 共享可变闭包 |
| SendMessage 目标 | agent name / "*" broadcast | run_id | XiaoLin 使用 run_id 作为 agent 实例标识，更精确 |
| Notification 格式 | XML envelope `<task-notification>` | JSON `CompletionSummary` | Rust serde 生态，JSON 更自然 |
| Markdown bubble mode | **不**允许在 markdown frontmatter 中设置 `bubble`（仅编程式设置） | 允许在 frontmatter 中设置 `permissionMode: bubble` | 我们认为用户应有这个控制权 |

### 需要注意的实现细节

1. **SendMessage 的实际机制**: Claude-code 中，对 in-process agents，SendMessage 写入 `task.pendingMessages`（per-task 状态），而非全局 `messageQueueManager`。消息通过 `drainPendingMessages` 在 tool-round boundary 注入。我们的设计（per-agent MessageQueue）与此一致。

2. **Notification 优先级**: Claude-code 的 task notification 使用 `later` 优先级（仅在 Sleep 后或显式触发时 drain）。我们的 worker→coordinator 通知使用 `Next` 优先级（每个 tool-round boundary 都 drain），因为 coordinator 需要及时知道 worker 完成。

3. **State 对象 vs Stream**: Claude-code 的 `State` 包含 `transition.reason`（跟踪为何继续循环），`turnCount`，`maxOutputTokensRecoveryCount` 等恢复状态。我们的 Stream 将这些作为内部 state（不 expose），但 `AgentStep::TurnEnd { reason }` 暴露终止原因。

4. **needsFollowUp 的简洁性**: Claude-code 仅用一个 `bool` 判断是否有 tool 调用。不信任 `stop_reason === 'tool_use'`。我们同样应在 stream 内部检测 tool_use 块存在来决定是否继续循环。
