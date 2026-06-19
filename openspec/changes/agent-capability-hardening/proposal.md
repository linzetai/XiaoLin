## Why

Agent runtime 存在两类安全和稳定性缺陷：Sub-agent 无法独立控制模型和 token 预算（`SubAgentDef.model` 字段已定义但未被执行路径尊重）、Sub-agent 结果返回上限硬编码不可配置。这些问题影响多 agent 协作的灵活性和资源控制能力。

注：经代码审查，MCP tools_ask 审批和 session 删除时文件系统清理均已实现（dispatcher 第 137-143 行的 `execute_mcp_with_approval` 路径，store.rs 第 1217-1232 行的 session 目录清理），不在本次范围内。

## What Changes

- Sub-agent 运行时尊重 `SubAgentDef.model` 字段，当 def 指定独立模型时覆盖继承的 main agent 模型配置
- Sub-agent 结果返回上限从硬编码 `MAX_RESULT_CHARS = 32768` 改为可通过 `SubAgentDef` 配置
- `SubAgentManager::cleanup_session()` 增加对 `session_event_senders` 的清理（目前遗漏）

## Capabilities

### New Capabilities
- `subagent-independent-config`: Sub-agent 独立的模型配置和结果返回上限控制

### Modified Capabilities

（无）

## Impact

- `crates/xiaolin-agent/src/subagent.rs` — 在构建 agent config 时应用 def.model 覆盖
- `crates/xiaolin-agent/src/subagent_manager.rs` — cleanup_session 修复、传递 max_result_chars
- `crates/xiaolin-agent/src/sidechain.rs` — truncate_result 参数化
- `crates/xiaolin-core/src/agent_config.rs` — SubAgentDef 新增 max_result_chars 字段
