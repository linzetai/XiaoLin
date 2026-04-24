# fastclaw-dag

DAG（有向无环图）工作流引擎：定义、校验、执行与检查点。

## 功能

- **九类节点** — LLM、Tool、Condition、Parallel、Join、HumanApproval、Loop、Reflect、Code
- **表达式求值** — JSON Pointer、运算符、索引、`in`、`contains`
- **SQLite 检查点** — 执行状态持久化，支持中断恢复
- **超时/重试/失败策略** — 每个节点可独立配置
- **结构化执行事件** — 内部可观测性（`EventSink` trait，crate 内部使用）

## 关键导出

```rust
pub use definition::DagDefinition;
pub use graph::DagGraph;
pub use executor::DagExecutor;
pub use executor::ExecutionContext;
pub use checkpoint::SqliteCheckpointStore;
pub use expression::evaluate_condition;
```
