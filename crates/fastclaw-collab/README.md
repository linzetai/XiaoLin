# fastclaw-collab

多 Agent 协作与 MCP 工具桥。

## 功能

- **委托（Delegation）** — Agent 间任务委托，支持两种方式：
  - `SubAgentDelegation`（推荐）— 基于 `SubAgentManager`，支持流式输出、类型化工具注册表、生命周期管理
  - `delegate_task`（已弃用）— 基于签名总线的简单请求/应答
- **MCP 服务端** — `McpServer` 将 FastClaw 工具暴露给外部 Agent 宿主（stdio 传输）
- **MCP 客户端** — 连接外部 MCP 工具服务器
- **工具桥接** — `McpTool` 将 MCP 远程工具注册为本地可调用工具

## 关键导出

```rust
pub use subagent_bridge::SubAgentDelegation;
pub use delegation::{DelegationRequest, DelegationResult};
pub use mcp::{McpServer, McpTool, create_fastclaw_mcp_server};
```

## Testing

```bash
cargo test -p fastclaw-collab
```

Coverage includes MCP JSON-RPC (initialize, ping, tools/list, tools/call), resources (list, read), prompts (list, get), capability advertisement, SSE transport, and delegation request/reply with timeout.
