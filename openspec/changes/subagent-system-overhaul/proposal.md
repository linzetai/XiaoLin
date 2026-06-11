## Why

XiaoLin 的 subagent 系统已具备基本能力（spawn/cancel/reactive loop/concurrency control），但与 claude-code 架构对比后发现核心差距：

1. **编排碎片化**：agent loop (`execute_unified`) 返回 `Result<TurnSummary>`，不可组合。子 agent 通过 `tokio::spawn` 脱离父级执行流，reactive loop 是外层独立循环。三条事件总线（AgentEvent mpsc、CompletionSummary broadcast、SlotEvent broadcast）增加理解复杂度。
2. **无上下文隔离/持久化**：子 agent 的对话历史仅通过 channel 转发，不做持久化。会话结束后子 agent 的完整推理过程丢失，无法 resume。
3. **无中途通信**：父级无法在子 agent 运行过程中注入消息（steering）。Coordinator 无法通过 `SendMessage` 续联 worker。
4. **权限模型单一**：所有子 agent 一律 `AutoApprove`，无法将危险操作的确认请求 bubble 到父级 UI。
5. **Agent 定义不一致**：硬编码的 `SubAgentType` 与 `SubAgentDef` 双轨并存，`MarkdownFile` source 已定义但未实现加载。
6. **前端 Bug**：`sub_agent_notification` event 已订阅但无 handler，notification 静默丢失。

参考 claude-code 的设计哲学：**一个递归函数、一套隔离上下文、工具轮边界注入、层级权限控制**，对 XiaoLin subagent 系统进行全面升级。

## What Changes

- **Stream-based Agent Loop**：将 `execute_unified` 重构为 `execute_as_stream() -> Stream<Item=AgentStep>`，使父子 agent 可组合（sync 子 agent = nested stream iteration）
- **Sidechain Transcript**：子 agent 对话持久化为 JSONL，支持 resume 和事后查看
- **Fork Agent**：可选继承父级上下文消息，子 agent 能理解前因后果
- **Message Queue + SendMessage**：在 tool-round boundary 注入消息，支持 Coordinator → Worker 续联
- **Permission Bubble**：子 agent 遇到需确认工具时，审批请求 bubble 到父级 UI
- **Coordinator Mode**：工具受限的编排 agent + async worker 模式
- **Markdown Agent Definitions**：frontmatter 定义自定义 agent 类型，加载 `.xiaolin/agents/*.md`
- **前端交互升级**：notification feed、steering input、approval bubble card、coordinator panel

## Capabilities

### New Capabilities
- `stream-agent-loop`: 统一 Stream-based agent loop，替代 imperative execute_unified
- `sidechain-transcript`: 子 agent 对话 JSONL 持久化 + resume
- `fork-agent`: 子 agent 继承父级上下文消息
- `message-queue-steering`: tool-round boundary 消息注入 + SendMessage 工具
- `permission-bubble`: 子 agent 权限请求 bubble 到父级
- `coordinator-mode`: Coordinator + Worker 编排模式
- `markdown-agent-defs`: Markdown frontmatter 自定义 agent 定义
- `frontend-interaction`: 前端 notification/steering/approval/coordinator UI

### Modified Capabilities
- `subagent-tool`: 参数扩展（inherit_context, permission_mode）
- `reactive-loop`: 迁移为 stream 内部逻辑（不再是外层独立循环）
- `spawn-controller`: 与 stream 集成，reservation 在 stream drop 时释放

## Impact

- **后端 crates**: `xiaolin-agent`（runtime loop 重构、新增 message_queue/sidechain/coordinator 模块）、`xiaolin-core`（SubAgentDef 扩展、新增 PermissionMode）、`xiaolin-protocol`（新增 AgentStep enum、ApprovalBubble event）
- **前端**: 新增 notification handler、steering input、approval card、coordinator panel
- **Protocol**: 新增 `approval_bubble`、`steering_message` WebSocket 事件
- **现有测试**: `SubAgentTool` 测试需适配新参数；reactive_loop 测试需重写为 stream-based
