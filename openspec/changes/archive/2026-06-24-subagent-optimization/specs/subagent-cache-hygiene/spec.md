## ADDED Requirements

### Requirement: Active sub-agent status zero-pollution injection
系统 SHALL NOT 将活跃 sub-agent 的动态状态（特别是 `elapsed_ms`）嵌入主 agent 的 system prompt。活跃状态 MUST 通过 user context 注入（最后一条 user message 的 `<system_context>` attachment）传递给主 agent。

#### Scenario: Active sub-agents do not break system prompt cache
- **WHEN** 主 agent 有一个或多个活跃 sub-agent，连续两个 turn 之间 session-stable 内容未变
- **THEN** 两个 turn 的 Tier-2 system message 内容 byte-identical（不因 `elapsed_ms` 变化而不同）

#### Scenario: Model still perceives active sub-agent status
- **WHEN** 主 agent 有活跃 sub-agent 且收到新的 user turn
- **THEN** active_runs 状态以 `<system_context>` 标签出现在最后一条 user message 中，主 agent 可读取

### Requirement: Delegation guidance byte-stability
delegation guidance（剥离 active_runs 后）SHALL 对同一 agent 配置在 session 内及跨 session byte-stable，使 provider 自动前缀缓存可命中。active_runs MUST 从 guidance block 中剥离（移交 user context 注入）。guidance 整体 MUST NOT 放入进程级全局 Tier-1 static section（避免 per-agent policy 跨 agent 污染）。

#### Scenario: Guidance byte-stable across turns for same agent
- **WHEN** 同一 agent 的 policy 配置不变，连续两个 turn
- **THEN** 两个 turn 的 delegation guidance 部分 byte-identical（不含 active_runs）

#### Scenario: Guidance byte-stable across sessions for same agent
- **WHEN** 同一 agent 在两个不同 session 启动，policy 配置相同
- **THEN** 两个 session 的 delegation guidance 部分 byte-identical

#### Scenario: active_runs removed from guidance block
- **WHEN** 主 agent 有活跃 sub-agent
- **THEN** delegation guidance block 中不含任何 active_runs 状态或 elapsed_ms

### Requirement: Sub-agent parent context as user message
sub-agent 接收的 parent context SHALL 作为 user message（或合并进首条 task user message）注入，MUST NOT 作为 System role message 注入。

#### Scenario: Parent context does not pollute sub-agent Tier-2
- **WHEN** 两个同类型 sub-agent 以不同 parent context 启动，但 cwd/memory/language 相同
- **THEN** 两个 sub-agent 的 Tier-2 system message byte-identical（parent context 不在其中）

### Requirement: Short-lived sub-agent TTL strategy
当未来接入 provider 显式 cache_control 时，sub-agent 的 LLM 调用 SHALL 使用 ephemeral（短 TTL）而非 1h TTL，以避免为 1-3 turn 的短生命周期对话支付 2x 计费。

#### Scenario: Sub-agent uses ephemeral TTL
- **WHEN** sub-agent 发起 LLM 调用且 provider 支持显式 cache_control
- **THEN** cache_control 使用 ephemeral（默认 5min）而非 ttl: 1h
