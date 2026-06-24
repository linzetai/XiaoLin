## ADDED Requirements

### Requirement: Coordinator-worker hierarchical display
当存在 coordinator 类型的 run 时，CoordinatorPanel SHALL 以树形层级展示 coordinator 及其 worker（树形缩进连接线），而非扁平列表。

#### Scenario: Coordinator with workers shows tree
- **WHEN** 一个 session 有一个 coordinator run 和多个 worker run
- **THEN** coordinator 作为顶层，worker 以缩进树形（连接线）展示在其下

#### Scenario: No coordinator falls back to flat list
- **WHEN** 一个 session 只有独立 spawn 的 sub-agent，没有 coordinator
- **THEN** CoordinatorPanel 回退为扁平列表展示

### Requirement: Coordinator aggregate statistics
coordinator header SHALL 显示聚合统计：worker 总数、完成数、运行中数、失败数、总耗时。

#### Scenario: Header shows aggregate counts
- **WHEN** coordinator 有 3 个 worker（1 完成、1 运行中、1 失败）
- **THEN** header 显示 "3 worker · 1 完成 · 1 运行中 · 1 失败" 及耗时

### Requirement: Worker ordering by status
worker 列表 SHALL 按状态排序：运行中优先，其次失败，最后完成。

#### Scenario: Running workers appear first
- **WHEN** worker 列表包含运行中、失败、完成的 worker
- **THEN** 运行中的 worker 排在最前，失败的次之，完成的最后
