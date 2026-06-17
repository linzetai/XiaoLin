## MODIFIED Requirements

### Requirement: Tool exposure classification
ToolRegistry SHALL 支持三种暴露模式：Direct（每轮发送完整 schema）、Deferred（仅名字可见，需 tool_search 激活）、ChannelScoped（仅限特定 channel 可调用）。MCP 工具 SHALL 默认 Deferred。

#### Scenario: MCP tool default exposure
- **WHEN** McpToolBridge 被创建且未设 alwaysLoad
- **THEN** `exposure()` 返回 `ToolExposure::Deferred`

#### Scenario: MCP tool alwaysLoad exposure
- **WHEN** McpToolBridge 被创建且 `_meta.alwaysLoad` 为 true
- **THEN** `exposure()` 返回 `ToolExposure::Direct`

#### Scenario: Registry version bump on activation
- **WHEN** deferred 工具通过 `activate_deferred()` 激活
- **THEN** registry version SHALL 递增
- **AND** 下一轮 LLM 调用前 `tool_defs` SHALL 包含新激活的工具
