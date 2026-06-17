## MODIFIED Requirements

### Requirement: Context collapse activation path
Context Collapse SHALL 可通过 `BehaviorConfig.enable_collapse` 配置启用（默认 false），启用时 autocompact SHALL 被抑制。

#### Scenario: Enable collapse via config
- **WHEN** `enable_collapse` 设为 true
- **THEN** `ContextPipeline` SHALL 调用 `CollapseEngine::collapse()` 进行读操作折叠
- **AND** `should_attempt_autocompact()` SHALL 返回 false

#### Scenario: Collapse integrates with unified_compact
- **WHEN** collapse 启用且 token 占用达到 async_threshold（默认 75%）
- **THEN** `unified_compact` SHALL 在 microcompact 之后、LLM autocompact 之前调用 `collapse()`
- **AND** 使用 `project()` 生成 LLM 可见的消息列表（collapsed round 替换为 summary）

### Requirement: CollapseSummarizer LLM bridge
系统 SHALL 提供 `CollapseSummarizer` 的生产实现，桥接到现有 LLM provider，用于生成 collapsed round 的摘要。

#### Scenario: Summarizer generates summary
- **WHEN** CollapseEngine 需要摘要一组 rounds
- **THEN** CollapseSummarizer SHALL 调用 LLM 生成结构化摘要
- **AND** 摘要包含关键文件路径、操作结果、错误信息

### Requirement: Stale detection content fallback
`FileStateCache::check_stale()` SHALL 在 mtime 变化时检查 `content_hash`，若 hash 匹配则返回 Fresh。

#### Scenario: mtime changed but content same
- **WHEN** 文件 mtime 大于缓存的 modified_at
- **AND** 文件内容 hash 与缓存的 content_hash 相同
- **THEN** `check_stale()` SHALL 返回 `StaleCheckResult::Fresh`

#### Scenario: mtime changed and content different
- **WHEN** 文件 mtime 大于缓存的 modified_at
- **AND** 文件内容 hash 与缓存的 content_hash 不同
- **THEN** `check_stale()` SHALL 返回 `StaleCheckResult::Stale`
