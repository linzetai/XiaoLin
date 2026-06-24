## ADDED Requirements

### Requirement: Structured re-prompt notification
reactive loop 的 completion-driven re-prompt（既有 R2）SHALL 以结构化数据回注 worker 结果，supersede「结果作为自由文本 system message 注入」的旧行为。注入位置保持在 messages 末尾（user context 层），MUST NOT 修改已缓存的 system prompt。

#### Scenario: Re-prompt injects structured worker result
- **WHEN** 一个 worker 完成触发 re-prompt
- **THEN** 主 LLM 收到结构化的 worker 结果（含 run_id/status/result/files），而非自由文本摘要

#### Scenario: Re-prompt preserves system prompt cache
- **WHEN** reactive loop 执行多次 re-prompt
- **THEN** 每次 re-prompt 的 Tier-1/Tier-2 system message byte-identical

### Requirement: Active runs injected via user context
主 agent 的 active sub-agent 状态（既有 R7 在前端，本要求针对 prompt 注入）SHALL 通过 user context 注入而非 system prompt，且 SHALL 携带进度信息（tool_calls_made、最新工具）。

#### Scenario: Active runs do not pollute system prompt
- **WHEN** 主 agent 有活跃 sub-agent
- **THEN** active runs 状态出现在最后一条 user message 的 `<system_context>` 中，不在 system role
