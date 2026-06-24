## ADDED Requirements

### Requirement: Hierarchical coordinator view in panel
floating sub-agent panel SHALL 在存在 coordinator 时以层级树形展示 coordinator 及其 worker，并在 coordinator header 显示聚合统计。

#### Scenario: Panel shows coordinator tree
- **WHEN** panel 中存在 coordinator 及其 worker
- **THEN** 以树形缩进展示层级，header 显示 worker 数/完成/运行中/失败/耗时

### Requirement: Enhanced steering in panel
floating sub-agent panel 的 steering 输入 SHALL 支持快捷操作、优先级切换和目标选择（coordinator 或指定 worker）。

#### Scenario: Panel steers selected target
- **WHEN** 用户在 panel 中选择某个活跃 worker 并发送 steering
- **THEN** 消息路由到该 worker，而非默认 coordinator

### Requirement: Markdown result in panel run items
panel 中 run item 的 result SHALL 在非失败状态下用 Markdown 渲染。

#### Scenario: Panel run result renders Markdown
- **WHEN** panel 中某 run item 展开且其 result 为 Markdown
- **THEN** result 以渲染后的 Markdown 展示
