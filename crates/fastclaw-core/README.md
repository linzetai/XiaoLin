# fastclaw-core

FastClaw 的核心共享库，定义了跨 crate 通用的类型、配置、路由、工具注册与消息总线。

## 模块

| 模块 | 职责 |
|------|------|
| `config` | `FastClawConfig` 及 JSON5 配置加载/合并（含 `ModelProviderConfig.context_window`） |
| `config_access` | 配置 ACL：可读/可写键白名单、敏感值脱敏、安全读写 |
| `agent_config` | Agent 配置结构体与验证（含 `AgentModelConfig.context_window` 上下文窗口覆盖） |
| `routing` | 五级优先级路由与动态路由 API |
| `tool` | 工具定义（`ToolDefinition`）与注册表 |
| `bus` | HMAC 签名消息总线（重放防护、跳数限制） |
| `types` | 聊天请求/响应、流式事件（含 `AskQuestion`、`contextTokens` / `contextWindow` 用量）、`SlashIntent` |
| `paths` | 全局路径解析（config/db/plugins/skills/agents/logs 等目录） |
| `workspace` | 工作区引导、身份文件、`SYSTEM_BASE.md` 内嵌资源 |
| `skill` | 技能定义与 frontmatter（`enabled`、`tags`） |
| `channel` | 渠道抽象与 `ChannelPlugin` trait |
| `error` | 统一错误类型 `FastClawError` |

## 关键导出

```rust
pub use config::FastClawConfig;
pub use error::{FastClawError, FastClawResult};
pub use routing::Router;
pub use complexity::ComplexityTier;
```
