# fastclaw-agent

Agent 运行时核心：LLM 提供商抽象、内置工具集、执行循环与子 Agent 机制。

## 功能

- **LLM Provider** — 统一抽象层，支持 OpenAI、Anthropic、DeepSeek、Gemini、DashScope、Ollama
- **Agent Runtime** — 消息循环：系统提示 → LLM 调用 → tool_calls 执行 → 结果回灌
- **内置工具** — 文件读写（`read_file`、`write_file`、`edit_file`、`apply_patch`）、搜索（`search_in_files`）、代码智能（`workspace_symbols`、`go_to_definition`、`find_references`）、人机交互（`ask_question`）
- **LSP 会话管理** — `LspSessionManager` 管理 per-workspace LSP 进程（rust-analyzer 等），支持 JSON-RPC over stdio
- **子 Agent** — `SubAgentTool` 支持 Agent 间委托

## Feature Flags

- `browser` — 启用 `headless_chrome` 浏览器自动化能力

## 关键导出

```rust
pub use runtime::AgentRuntime;
pub use runtime::ExecutionResult;
pub use builtin_tools::register_builtin_tools_with_sandbox;
pub use subagent::SubAgentTool;
```
