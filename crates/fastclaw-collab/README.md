# fastclaw-collab

多 Agent 协作与 MCP 工具桥。

## 功能

- **委托（Delegation）** — Agent 间通过签名总线进行任务委托
- **MCP 服务端** — `McpServer` 将 FastClaw 工具暴露给外部 Agent 宿主（stdio 传输）
- **MCP 客户端** — 连接外部 MCP 工具服务器
- **工具桥接** — `McpTool` 将 MCP 远程工具注册为本地可调用工具

## 关键导出

```rust
pub use delegation::{DelegationRequest, DelegationResult};
pub use mcp::{McpServer, McpTool, create_fastclaw_mcp_server};
```
