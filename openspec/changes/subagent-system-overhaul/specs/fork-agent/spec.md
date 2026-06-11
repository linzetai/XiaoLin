## ADDED Requirements

### Requirement: spawn_subagent supports inherit_context parameter

`spawn_subagent` 工具 SHALL 接受可选的 `inherit_context` 布尔参数。

#### Scenario: Spawn with context inheritance
- **WHEN** LLM 调用 `spawn_subagent` 且 `inherit_context: true`
- **THEN** 子 agent 的 initial messages 包含父级最近 N 条消息（filtered）+ task prompt

#### Scenario: Spawn without context inheritance (default)
- **WHEN** LLM 调用 `spawn_subagent` 且未指定 `inherit_context`（或为 false）
- **THEN** 子 agent 仅收到 system prompt + task prompt，不包含父级消息

### Requirement: Parent messages filtered before fork

继承的父级消息 SHALL 过滤掉不完整的 tool 调用和敏感内容。

#### Scenario: Incomplete tool calls removed
- **GIVEN** 父级消息中有 `tool_use` block 但缺少对应的 `tool_result`
- **WHEN** 构建 fork context
- **THEN** 该 `tool_use` block 及其所在消息被跳过（避免 API 格式错误）

#### Scenario: System messages excluded
- **GIVEN** 父级消息序列中包含 system role 消息
- **WHEN** 构建 fork context
- **THEN** system 消息不被包含（子 agent 有自己的 system prompt）

### Requirement: Fork context size is bounded

Fork 继承的消息数量 SHALL 有上限。

#### Scenario: Recent messages only
- **GIVEN** 父级有 100 条消息
- **WHEN** `inherit_context: true`
- **THEN** 最多取最近 20 条消息（可配置 via SubAgentDef.max_context_messages）

#### Scenario: Token budget for context
- **GIVEN** 最近 20 条消息总 token 数超过 8192
- **WHEN** 构建 fork context
- **THEN** 从最旧的消息开始裁剪，直到总量 <= 8192 token
